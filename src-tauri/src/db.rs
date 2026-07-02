//! SQLite layer: schema, cache rows, the daily request meter, AI result cache,
//! and saved tickets. All functions take a `&Connection` and never await.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{RequestMeter, SavedTicket};

// ---------- backup / restore / reset ----------

/// Tables included in an export (the meaningful state — not regenerable caches).
const EXPORT_TABLES: &[&str] = &[
    "settings",
    "saved_tickets",
    "placed_bets",
    "generated_tickets",
    "grok_log",
    "grok_usage",
    "ai_usage",
    "request_log",
    "league_picks",
    "player_features",
];

/// Tables wiped on a reset (everything except `settings`, so keys/config survive).
const RESET_TABLES: &[&str] = &[
    "cache",
    "request_log",
    "player_features",
    "ai_results",
    "saved_tickets",
    "league_picks",
    "ai_usage",
    "grok_log",
    "grok_usage",
    "placed_bets",
    "generated_tickets",
    "ingested",
];

fn json_to_sql(v: &serde_json::Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as V;
    match v {
        serde_json::Value::Null => V::Null,
        serde_json::Value::Bool(b) => V::Integer(*b as i64),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(V::Integer)
            .or_else(|| n.as_f64().map(V::Real))
            .unwrap_or(V::Null),
        serde_json::Value::String(s) => V::Text(s.clone()),
        other => V::Text(other.to_string()),
    }
}

fn dump_table(conn: &Connection, table: &str) -> Result<Vec<serde_json::Value>, String> {
    use rusqlite::types::Value as V;
    let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).map_err(|e| e.to_string())?;
    let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let rows = stmt
        .query_map([], |row| {
            let mut obj = serde_json::Map::new();
            for (i, c) in cols.iter().enumerate() {
                let jv = match row.get::<_, V>(i)? {
                    V::Null => serde_json::Value::Null,
                    V::Integer(x) => serde_json::json!(x),
                    V::Real(x) => serde_json::json!(x),
                    V::Text(s) => serde_json::json!(s),
                    V::Blob(b) => serde_json::json!(b),
                };
                obj.insert(c.clone(), jv);
            }
            Ok(serde_json::Value::Object(obj))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn restore_table(conn: &Connection, table: &str, rows: &[serde_json::Value]) -> Result<(), String> {
    conn.execute(&format!("DELETE FROM {table}"), []).map_err(|e| e.to_string())?;
    for row in rows {
        let obj = match row.as_object() {
            Some(o) if !o.is_empty() => o,
            _ => continue,
        };
        let cols: Vec<&String> = obj.keys().collect();
        let collist = cols.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
        let placeholders = (1..=cols.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
        let sql = format!("INSERT INTO {table} ({collist}) VALUES ({placeholders})");
        let vals: Vec<rusqlite::types::Value> = cols.iter().map(|c| json_to_sql(&obj[*c])).collect();
        let p: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        conn.execute(&sql, p.as_slice()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- ingested web pages (from the browser extension) ----------

/// Insert (or refresh) an ingested page, deduped by URL. Resets it to "new" so it
/// re-processes if the content changed. Returns the row id.
pub fn ingest_add(
    conn: &Connection,
    now: i64,
    url: &str,
    url_hash: &str,
    title: &str,
    content: &str,
    note: &str,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO ingested (created_at, url, url_hash, title, content, note, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'new')
         ON CONFLICT(url_hash) DO UPDATE SET
            created_at=?1, title=?4, content=?5,
            note=CASE WHEN ?6 <> '' THEN ?6 ELSE note END,
            status='new'",
        params![now, url, url_hash, title, content, note],
    )
    .map_err(|e| e.to_string())?;
    conn.query_row("SELECT id FROM ingested WHERE url_hash = ?1", params![url_hash], |r| r.get(0))
        .map_err(|e| e.to_string())
}

pub struct IngestRow {
    pub id: i64,
    pub created_at: i64,
    pub url: String,
    pub title: String,
    pub content: String,
    pub note: String,
    pub status: String,
    pub fixture_label: Option<String>,
    pub fixture_id: Option<i64>,
    pub extracted_json: Option<String>,
    pub model: Option<String>,
    pub used: bool,
}

fn ingest_row(r: &rusqlite::Row) -> rusqlite::Result<IngestRow> {
    Ok(IngestRow {
        id: r.get(0)?,
        created_at: r.get(1)?,
        url: r.get(2)?,
        title: r.get(3)?,
        content: r.get(4)?,
        note: r.get(5)?,
        status: r.get(6)?,
        fixture_label: r.get(7)?,
        fixture_id: r.get(8)?,
        extracted_json: r.get(9)?,
        model: r.get(10)?,
        used: r.get::<_, i64>(11)? != 0,
    })
}

const INGEST_COLS: &str =
    "id, created_at, url, title, content, note, status, fixture_label, fixture_id, extracted_json, model, used";

pub fn ingest_list(conn: &Connection) -> Result<Vec<IngestRow>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {INGEST_COLS} FROM ingested ORDER BY created_at DESC"))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], ingest_row).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn ingest_get(conn: &Connection, id: i64) -> Result<Option<IngestRow>, String> {
    conn.query_row(&format!("SELECT {INGEST_COLS} FROM ingested WHERE id = ?1"), params![id], ingest_row)
        .optional()
        .map_err(|e| e.to_string())
}

pub fn ingest_set_processed(
    conn: &Connection,
    id: i64,
    fixture_label: &str,
    fixture_id: Option<i64>,
    extracted_json: &str,
    model: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE ingested SET status='processed', fixture_label=?2, fixture_id=?3, extracted_json=?4, model=?5 WHERE id=?1",
        params![id, fixture_label, fixture_id, extracted_json, model],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn ingest_set_note(conn: &Connection, id: i64, note: &str) -> Result<(), String> {
    conn.execute("UPDATE ingested SET note=?2 WHERE id=?1", params![id, note]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Manually (re)assign an ingested page to a fixture. Also patches the extracted
/// JSON's home/away/date so the build's matching + non-archived checks agree with
/// the user's assignment. `label` is "Home vs Away"; `date` is "YYYY-MM-DD" or "".
pub fn ingest_set_fixture(conn: &Connection, id: i64, label: &str, date: &str) -> Result<(), String> {
    // Patch extracted_json home/away/date if present so everything downstream lines up.
    let existing: Option<String> = conn
        .query_row("SELECT extracted_json FROM ingested WHERE id=?1", params![id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let patched: Option<String> = existing.and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok()).map(|mut v| {
        if let Some((h, a)) = label.split_once(" vs ") {
            v["home"] = serde_json::Value::String(h.trim().to_string());
            v["away"] = serde_json::Value::String(a.trim().to_string());
        }
        if !date.is_empty() {
            v["date"] = serde_json::Value::String(date.to_string());
        }
        v.to_string()
    });
    // A manual reassignment invalidates any AUTO-resolved fixture id — clear it
    // so the next build re-resolves against the new label.
    match patched {
        Some(p) => conn.execute(
            "UPDATE ingested SET fixture_label=?2, extracted_json=?3, fixture_id=NULL WHERE id=?1",
            params![id, label, p],
        ),
        None => conn.execute("UPDATE ingested SET fixture_label=?2, fixture_id=NULL WHERE id=?1", params![id, label]),
    }
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Resolve an ingested page to a REAL fixture (self-healing): once a build's
/// token-matching pairs the page with a selected fixture, store the canonical
/// id (label is left as the user/extraction set it — `ingest_set_fixture`
/// above is the manual reassignment path and owns the label).
pub fn ingest_resolve_fixture(conn: &Connection, id: i64, fixture_id: i64) -> Result<(), String> {
    conn.execute("UPDATE ingested SET fixture_id=?2 WHERE id=?1", params![id, fixture_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn ingest_mark_used(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("UPDATE ingested SET used=1 WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn ingest_delete(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM ingested WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Processed items whose extracted home/away match either of these team names.
pub fn ingest_for_fixture(conn: &Connection) -> Result<Vec<IngestRow>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {INGEST_COLS} FROM ingested WHERE status='processed'"))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], ingest_row).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Serialize the meaningful app state to a portable JSON backup string.
pub fn export_all(conn: &Connection) -> Result<String, String> {
    let mut tables = serde_json::Map::new();
    for t in EXPORT_TABLES {
        tables.insert(t.to_string(), serde_json::Value::Array(dump_table(conn, t)?));
    }
    let root = serde_json::json!({ "app": "powabet", "version": 1, "tables": tables });
    serde_json::to_string(&root).map_err(|e| e.to_string())
}

/// Replace current state with a backup made by `export_all`. Returns rows loaded.
pub fn import_all(conn: &Connection, json: &str) -> Result<usize, String> {
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("not a valid backup file: {e}"))?;
    if root.get("app").and_then(|a| a.as_str()) != Some("powabet") {
        return Err("this file isn't a powabet backup".to_string());
    }
    let tables = root.get("tables").and_then(|t| t.as_object()).ok_or("backup has no tables")?;
    conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let mut n = 0usize;
    for table in EXPORT_TABLES {
        if let Some(arr) = tables.get(*table).and_then(|v| v.as_array()) {
            if let Err(e) = restore_table(conn, table, arr) {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
            n += arr.len();
        }
    }
    conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    Ok(n)
}

/// Wipe all bets, picks, stats and caches back to a fresh state (keeps settings).
pub fn reset_all(conn: &Connection) -> Result<(), String> {
    for t in RESET_TABLES {
        conn.execute(&format!("DELETE FROM {t}"), []).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Default daily request budget. Overridable per plan in Settings.
pub const DEFAULT_DAILY_LIMIT: i64 = 7500;

pub fn open(path: &std::path::Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    // Two writers share this file (the app + the ingest server thread). WAL
    // lets a reader and a writer coexist; the busy timeout stops an extension
    // POST landing mid-build from failing with a raw "database is locked".
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "busy_timeout", 3000);
    init(&conn)?;
    Ok(conn)
}

fn init(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS cache (
            key        TEXT PRIMARY KEY,
            endpoint   TEXT NOT NULL,
            payload    TEXT NOT NULL,
            fetched_at INTEGER NOT NULL,
            ttl_secs   INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS request_log (
            day   TEXT PRIMARY KEY,
            count INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS player_features (
            fixture_id  INTEGER NOT NULL,
            player_id   INTEGER NOT NULL,
            json        TEXT NOT NULL,
            computed_at INTEGER NOT NULL,
            PRIMARY KEY (fixture_id, player_id)
        );
        CREATE TABLE IF NOT EXISTS ai_results (
            input_hash  TEXT PRIMARY KEY,
            output_json TEXT NOT NULL,
            model       TEXT NOT NULL,
            created_at  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS saved_tickets (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at     INTEGER NOT NULL,
            selection_json TEXT NOT NULL,
            result_json    TEXT NOT NULL,
            user_notes     TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS league_picks (
            league_id INTEGER PRIMARY KEY,
            count     INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS ai_usage (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at    INTEGER NOT NULL,
            model         TEXT NOT NULL,
            input_tokens  INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS grok_log (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at INTEGER NOT NULL,
            matches    TEXT NOT NULL,
            digest     TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS grok_usage (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at    INTEGER NOT NULL,
            input_tokens  INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            sources       INTEGER NOT NULL,
            cost_usd      REAL NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS placed_bets (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at       INTEGER NOT NULL,
            day              TEXT NOT NULL,
            ticket_json      TEXT NOT NULL,
            stake            REAL NOT NULL,
            status           TEXT NOT NULL,
            returns          REAL NOT NULL,
            leg_results_json TEXT NOT NULL,
            settled          INTEGER NOT NULL,
            grok_used        INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS generated_tickets (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at       INTEGER NOT NULL,
            day              TEXT NOT NULL,
            strategy         TEXT NOT NULL,
            grok_used        INTEGER NOT NULL,
            kind             TEXT NOT NULL DEFAULT 'Single',
            sig              TEXT NOT NULL,
            ticket_json      TEXT NOT NULL,
            combined_odds    REAL,
            settled          INTEGER NOT NULL DEFAULT 0,
            won              INTEGER,
            leg_results_json TEXT NOT NULL DEFAULT '[]',
            UNIQUE(day, strategy, grok_used, sig)
        );
        CREATE TABLE IF NOT EXISTS ingested (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at      INTEGER NOT NULL,
            url             TEXT NOT NULL,
            url_hash        TEXT NOT NULL UNIQUE,
            title           TEXT NOT NULL DEFAULT '',
            content         TEXT NOT NULL,
            note            TEXT NOT NULL DEFAULT '',
            status          TEXT NOT NULL DEFAULT 'new',
            fixture_label   TEXT,
            fixture_id      INTEGER,
            extracted_json  TEXT,
            model           TEXT,
            used            INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .map_err(|e| e.to_string())?;
    // Migrate older DBs that predate the grok_used column (ignore if it exists).
    let _ = conn.execute(
        "ALTER TABLE placed_bets ADD COLUMN grok_used INTEGER NOT NULL DEFAULT 0",
        [],
    );
    // Strategy the ticket came from: value | likely | board (migration).
    let _ = conn.execute(
        "ALTER TABLE placed_bets ADD COLUMN strategy TEXT NOT NULL DEFAULT 'value'",
        [],
    );
    // Actual x.ai-billed cost per Grok call (migration).
    let _ = conn.execute(
        "ALTER TABLE grok_usage ADD COLUMN cost_usd REAL NOT NULL DEFAULT 0",
        [],
    );
    // Ticket kind on the generated-tickets ledger (migration).
    let _ = conn.execute(
        "ALTER TABLE generated_tickets ADD COLUMN kind TEXT NOT NULL DEFAULT 'Single'",
        [],
    );
    // What each AI call was FOR (build/eval/plausibility/ingest/tactics) — migration.
    let _ = conn.execute(
        "ALTER TABLE ai_usage ADD COLUMN purpose TEXT NOT NULL DEFAULT 'build'",
        [],
    );
    // CLV (closing-line value) per placed bet (migration).
    let _ = conn.execute("ALTER TABLE placed_bets ADD COLUMN clv REAL", []);
    // All-void generated tickets (pushes) — excluded from hit/ROI (migration).
    let _ = conn.execute("ALTER TABLE generated_tickets ADD COLUMN voided INTEGER NOT NULL DEFAULT 0", []);
    // Whether ingested page data fed the build (A/B tracking, like grok_used).
    let _ = conn.execute("ALTER TABLE generated_tickets ADD COLUMN ingest_used INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE placed_bets ADD COLUMN ingest_used INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE placed_bets ADD COLUMN clv_json TEXT", []);
    // Calibration + settle scan the ledger by settled-state on every build/run —
    // index it so the ever-growing ledger stays cheap to scan.
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_gen_settled ON generated_tickets(settled)",
        [],
    );
    // Startup pruning — these tables grow without bound otherwise. Cache rows
    // expired >1 day are never read again (cache_get ignores expired rows);
    // ai_results transient keys (live tickets, plausibility) accrete forever.
    let now = chrono::Utc::now().timestamp();
    let _ = conn.execute("DELETE FROM cache WHERE fetched_at + ttl_secs < ?1 - 86400", params![now]);
    let _ = conn.execute("DELETE FROM ai_results WHERE created_at < ?1 - 60 * 86400", params![now]);
    Ok(())
}

// ---------- cache ----------

/// Returns the cached payload string if a row exists and is still fresh.
pub fn cache_get(conn: &Connection, key: &str, now: i64) -> Result<Option<String>, String> {
    let row: Option<(String, i64, i64)> = conn
        .query_row(
            "SELECT payload, fetched_at, ttl_secs FROM cache WHERE key = ?1",
            params![key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    Ok(match row {
        Some((payload, fetched_at, ttl)) if now - fetched_at < ttl => Some(payload),
        _ => None,
    })
}

pub fn cache_put(
    conn: &Connection,
    key: &str,
    endpoint: &str,
    payload: &str,
    now: i64,
    ttl: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO cache (key, endpoint, payload, fetched_at, ttl_secs)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![key, endpoint, payload, now, ttl],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- request meter ----------

pub fn meter(conn: &Connection, day: &str, limit: i64) -> Result<RequestMeter, String> {
    let count: i64 = conn
        .query_row(
            "SELECT count FROM request_log WHERE day = ?1",
            params![day],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(0);
    Ok(RequestMeter {
        day: day.to_string(),
        count,
        limit,
    })
}

pub fn meter_count(conn: &Connection, day: &str) -> Result<i64, String> {
    Ok(conn
        .query_row(
            "SELECT count FROM request_log WHERE day = ?1",
            params![day],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(0))
}

pub fn meter_increment(conn: &Connection, day: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO request_log (day, count) VALUES (?1, 1)
         ON CONFLICT(day) DO UPDATE SET count = count + 1",
        params![day],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- ai results ----------

pub fn ai_get(conn: &Connection, input_hash: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT output_json FROM ai_results WHERE input_hash = ?1",
        params![input_hash],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn ai_put(
    conn: &Connection,
    input_hash: &str,
    output_json: &str,
    model: &str,
    now: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO ai_results (input_hash, output_json, model, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![input_hash, output_json, model, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- saved tickets ----------

pub fn save_ticket(
    conn: &Connection,
    now: i64,
    selection_json: &str,
    result_json: &str,
    notes: &str,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO saved_tickets (created_at, selection_json, result_json, user_notes)
         VALUES (?1, ?2, ?3, ?4)",
        params![now, selection_json, result_json, notes],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

pub fn list_tickets(conn: &Connection) -> Result<Vec<SavedTicket>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, created_at, result_json, user_notes
             FROM saved_tickets ORDER BY created_at DESC LIMIT 100",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SavedTicket {
                id: r.get(0)?,
                created_at: r.get(1)?,
                result_json: r.get(2)?,
                user_notes: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

// ---------- AI token usage (for the cost estimator) ----------

pub fn usage_add(
    conn: &Connection,
    now: i64,
    model: &str,
    input: i64,
    output: i64,
    purpose: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO ai_usage (created_at, model, input_tokens, output_tokens, purpose)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![now, model, input, output, purpose],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Token usage grouped by (model, purpose): (model, purpose, input, output).
pub fn usage_by_purpose(conn: &Connection) -> Result<Vec<(String, String, i64, i64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT model, COALESCE(purpose,'build'), SUM(input_tokens), SUM(output_tokens)
             FROM ai_usage GROUP BY model, COALESCE(purpose,'build')",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

// ---------- grok newsfeed log ----------

pub fn grok_log_add(conn: &Connection, now: i64, matches: &str, digest: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO grok_log (created_at, matches, digest) VALUES (?1, ?2, ?3)",
        params![now, matches, digest],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn grok_log_list(conn: &Connection) -> Result<Vec<(i64, i64, String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, created_at, matches, digest FROM grok_log ORDER BY created_at DESC LIMIT 50")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

// ---------- grok usage ----------

pub fn grok_usage_add(
    conn: &Connection,
    now: i64,
    input: i64,
    output: i64,
    sources: i64,
    cost_usd: f64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO grok_usage (created_at, input_tokens, output_tokens, sources, cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![now, input, output, sources, cost_usd],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Actual x.ai-billed cost (USD) since a timestamp (use 0 for lifetime).
pub fn grok_cost_since(conn: &Connection, since: i64) -> Result<f64, String> {
    conn.query_row(
        "SELECT COALESCE(SUM(cost_usd),0) FROM grok_usage WHERE created_at >= ?1",
        params![since],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Usage (input, output tokens) by model since a timestamp, for cost windows.
pub fn usage_since(conn: &Connection, since: i64) -> Result<Vec<(String, i64, i64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT model, SUM(input_tokens), SUM(output_tokens)
             FROM ai_usage WHERE created_at >= ?1 GROUP BY model",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![since], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

// ---------- placed bets ----------

pub struct PlacedRow {
    pub id: i64,
    pub created_at: i64,
    pub day: String,
    pub ticket_json: String,
    pub stake: f64,
    pub status: String,
    pub returns: f64,
    pub leg_results_json: String,
    pub settled: bool,
    pub grok_used: bool,
    pub ingest_used: bool,
    pub strategy: String,
    /// Closing-line value: avg (placed_odds / closing_odds − 1) across priced
    /// legs. Positive = beat the close — the fastest-converging edge signal.
    pub clv: Option<f64>,
}

pub fn place_bet(
    conn: &Connection,
    created_at: i64,
    day: &str,
    ticket_json: &str,
    stake: f64,
    grok_used: bool,
    ingest_used: bool,
    strategy: &str,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO placed_bets (created_at, day, ticket_json, stake, status, returns, leg_results_json, settled, grok_used, ingest_used, strategy)
         VALUES (?1, ?2, ?3, ?4, 'open', 0, '[]', 0, ?5, ?6, ?7)",
        params![created_at, day, ticket_json, stake, grok_used as i64, ingest_used as i64, strategy],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

fn row_to_placed(r: &rusqlite::Row) -> rusqlite::Result<PlacedRow> {
    Ok(PlacedRow {
        id: r.get(0)?,
        created_at: r.get(1)?,
        day: r.get(2)?,
        ticket_json: r.get(3)?,
        stake: r.get(4)?,
        status: r.get(5)?,
        returns: r.get(6)?,
        leg_results_json: r.get(7)?,
        settled: r.get::<_, i64>(8)? != 0,
        grok_used: r.get::<_, i64>(9)? != 0,
        strategy: r.get(10)?,
        clv: r.get(11)?,
        ingest_used: r.get::<_, i64>(12)? != 0,
    })
}

const PLACED_COLS: &str =
    "id, created_at, day, ticket_json, stake, status, returns, leg_results_json, settled, grok_used, strategy, clv, ingest_used";

pub fn list_placed(conn: &Connection) -> Result<Vec<PlacedRow>, String> {
    let sql = format!("SELECT {PLACED_COLS} FROM placed_bets ORDER BY created_at DESC");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_placed).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn get_placed(conn: &Connection, id: i64) -> Result<Option<PlacedRow>, String> {
    let sql = format!("SELECT {PLACED_COLS} FROM placed_bets WHERE id = ?1");
    conn.query_row(&sql, params![id], row_to_placed)
        .optional()
        .map_err(|e| e.to_string())
}

pub fn update_settlement(
    conn: &Connection,
    id: i64,
    status: &str,
    returns: f64,
    leg_results_json: &str,
    settled: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE placed_bets SET status = ?2, returns = ?3, leg_results_json = ?4, settled = ?5 WHERE id = ?1",
        params![id, status, returns, leg_results_json, settled as i64],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_bet_clv(conn: &Connection, id: i64, clv: f64, detail_json: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE placed_bets SET clv = ?2, clv_json = ?3 WHERE id = ?1",
        params![id, clv, detail_json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Update the ticket's combined odds on an OPEN bet (the Tracker's "add odds to
/// settle" flow for all-green bets placed without a price).
pub fn set_bet_odds(conn: &Connection, id: i64, ticket_json: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE placed_bets SET ticket_json = ?2 WHERE id = ?1 AND settled = 0",
        params![id, ticket_json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- generated-tickets ledger (paper trading by strategy) ----------

pub struct GenRow {
    pub id: i64,
    pub ticket_json: String,
    pub combined_odds: Option<f64>,
}

/// Insert a generated ticket (ignored if the same one already exists for that
/// day + strategy + grok flag — so repeated builds don't double-count).
#[allow(clippy::too_many_arguments)]
pub fn gen_add(
    conn: &Connection,
    now: i64,
    day: &str,
    strategy: &str,
    grok_used: bool,
    ingest_used: bool,
    kind: &str,
    sig: &str,
    ticket_json: &str,
    combined_odds: Option<f64>,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO generated_tickets
         (created_at, day, strategy, grok_used, ingest_used, kind, sig, ticket_json, combined_odds)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![now, day, strategy, grok_used as i64, ingest_used as i64, kind, sig, ticket_json, combined_odds],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Paper-ledger A/B: with vs without ingested data (void-aware, windowed).
/// Returns (ingest_used, total, settled, won, priced_n, return_sum).
pub fn gen_ingest_split(conn: &Connection, since: i64) -> Result<Vec<(bool, i64, i64, i64, i64, f64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT ingest_used,
                    COUNT(*),
                    SUM(CASE WHEN settled=1 AND voided=0 THEN 1 ELSE 0 END),
                    SUM(CASE WHEN voided=0 THEN COALESCE(won,0) ELSE 0 END),
                    SUM(CASE WHEN settled=1 AND voided=0 AND combined_odds IS NOT NULL THEN 1 ELSE 0 END),
                    SUM(CASE WHEN settled=1 AND voided=0 AND combined_odds IS NOT NULL AND won=1 THEN combined_odds ELSE 0 END)
             FROM generated_tickets WHERE created_at >= ?1 GROUP BY ingest_used",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![since], |r| {
            Ok((
                r.get::<_, i64>(0)? != 0,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, f64>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Aggregate by ticket KIND (Single/SGP/SGP+): (kind, total, settled, won, priced_n, return_sum).
pub fn gen_report_by_kind(conn: &Connection) -> Result<Vec<(String, i64, i64, i64, i64, f64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, COUNT(*),
                    SUM(CASE WHEN settled=1 AND voided=0 THEN 1 ELSE 0 END),
                    SUM(CASE WHEN voided=0 THEN COALESCE(won,0) ELSE 0 END),
                    SUM(CASE WHEN settled=1 AND voided=0 AND combined_odds IS NOT NULL THEN 1 ELSE 0 END),
                    SUM(CASE WHEN settled=1 AND voided=0 AND combined_odds IS NOT NULL AND won=1 THEN combined_odds ELSE 0 END)
             FROM generated_tickets GROUP BY kind",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Settled generated tickets as (ticket_json, leg_results_json) — for per-leg /
/// per-market stats and feeding the calibration loop.
pub fn gen_settled(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT ticket_json, leg_results_json FROM generated_tickets WHERE settled = 1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn gen_unsettled(conn: &Connection) -> Result<Vec<GenRow>, String> {
    let mut stmt = conn
        .prepare("SELECT id, ticket_json, combined_odds FROM generated_tickets WHERE settled = 0")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(GenRow { id: r.get(0)?, ticket_json: r.get(1)?, combined_odds: r.get(2)? })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn gen_mark_settled(conn: &Connection, id: i64, won: bool, voided: bool, leg_results_json: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE generated_tickets SET settled = 1, won = ?2, voided = ?3, leg_results_json = ?4 WHERE id = ?1",
        params![id, won as i64, voided as i64, leg_results_json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Aggregate by strategy since `since` (unix ts; 0 = lifetime): (strategy,
/// grok=false, total, settled, won, priced_n, return_sum, voided). VOIDED
/// tickets (all-void = push, stake refunded) are excluded from settled/won/
/// ROI — counting a push as a loss dragged every hit-rate down.
pub fn gen_report(conn: &Connection, since: i64) -> Result<Vec<(String, bool, i64, i64, i64, i64, f64, i64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT strategy, 0,
                    COUNT(*),
                    SUM(CASE WHEN settled=1 AND voided=0 THEN 1 ELSE 0 END),
                    SUM(CASE WHEN voided=0 THEN COALESCE(won,0) ELSE 0 END),
                    SUM(CASE WHEN settled=1 AND voided=0 AND combined_odds IS NOT NULL THEN 1 ELSE 0 END),
                    SUM(CASE WHEN settled=1 AND voided=0 AND combined_odds IS NOT NULL AND won=1 THEN combined_odds ELSE 0 END),
                    SUM(CASE WHEN settled=1 AND voided=1 THEN 1 ELSE 0 END)
             FROM generated_tickets WHERE created_at >= ?1 GROUP BY strategy",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)? != 0,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, f64>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Settled non-void tickets with strategy + predicted combined_prob inputs —
/// for the per-strategy predicted-vs-actual comparison.
pub fn gen_settled_strat(conn: &Connection, since: i64) -> Result<Vec<(String, String, bool)>, String> {
    let mut stmt = conn
        .prepare("SELECT strategy, ticket_json, COALESCE(won,0) FROM generated_tickets WHERE settled=1 AND voided=0 AND created_at >= ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![since], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn delete_placed(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM placed_bets WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Lifetime usage grouped by model → (model, input_tokens, output_tokens).
pub fn usage_by_model(conn: &Connection) -> Result<Vec<(String, i64, i64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT model, SUM(input_tokens), SUM(output_tokens)
             FROM ai_usage GROUP BY model",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

// ---------- league pick counts (drive the league sort order) ----------

pub fn league_pick_counts(conn: &Connection) -> Result<HashMap<i64, i64>, String> {
    let mut stmt = conn
        .prepare("SELECT league_id, count FROM league_picks")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut map = HashMap::new();
    for row in rows {
        let (id, c) = row.map_err(|e| e.to_string())?;
        map.insert(id, c);
    }
    Ok(map)
}

pub fn bump_league(conn: &Connection, league_id: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO league_picks (league_id, count) VALUES (?1, 1)
         ON CONFLICT(league_id) DO UPDATE SET count = count + 1",
        params![league_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- settings (keys live in settings.json, not here; this is for future use) ----------

#[allow(dead_code)]
pub fn setting_get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

#[allow(dead_code)]
pub fn setting_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
