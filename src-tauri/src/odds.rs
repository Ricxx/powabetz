//! Odds parsing. Pinnacle is the sharp book → de-vigged "true" probability; for
//! the price you'd take we LINE-SHOP across every other chosen book and keep the
//! BEST price (+ which book). +EV = best_odds * true_prob - 1.
//!
//! We parse far more than 1x2/O-U/BTTS now: team goals/corners O-U, anytime
//! scorer/assist/cards, and player threshold props (shots, SOT, fouls, tackles,
//! passes) — matched to candidates in features::attach_odds.

use std::collections::HashMap;

use serde_json::Value;

use crate::apifootball::response_array;

const PINNACLE: i64 = 4;

/// Parsed odds for one fixture.
#[derive(Default)]
pub struct FixtureOdds {
    /// "market|selection" → Pinnacle de-vigged true prob (1x2/dc/ou/btts/team O-U).
    pin: HashMap<String, f64>,
    /// "market|selection" → (best decimal odds across books, book name).
    book: HashMap<String, (f64, String)>,
    /// player-name(lower) → raw Pinnacle prob (scorer).
    pin_scorer: HashMap<String, f64>,
    /// player-name(lower) → (best odds, book) for anytime scorer.
    book_scorer: HashMap<String, (f64, String)>,
    /// player props keyed "tag|player(lower)|threshold" → (best odds, book).
    /// tag ∈ assist, card, sot, pshots, fouls, tackles, passes.
    book_prop: HashMap<String, (f64, String)>,
}

/// (pinnacle_true_prob, (best_odds, book_name)).
pub type Priced = (Option<f64>, Option<(f64, String)>);

impl FixtureOdds {
    pub fn get(&self, key: &str) -> Priced {
        (self.pin.get(key).copied(), self.book.get(key).cloned())
    }
    pub fn scorer(&self, name: &str) -> Priced {
        let n = fold(name);
        (lookup_name(&self.pin_scorer, &n), lookup_name_book(&self.book_scorer, &n))
    }
    /// A player threshold/anytime prop. No de-vig (props are usually single-sided),
    /// so only a book price is returned — EV falls back to our model probability.
    pub fn prop(&self, tag: &str, name: &str, thr: i64) -> Priced {
        let prefix = format!("{tag}|");
        let suffix = format!("|{thr}");
        let n = fold(name);
        let last = n.rsplit(' ').next().unwrap_or(&n);
        let found = self
            .book_prop
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix) && k.ends_with(&suffix))
            .find(|(k, _)| {
                let mid = &k[prefix.len()..k.len() - suffix.len()];
                mid.contains(&n) || mid.contains(last) || n.contains(mid)
            })
            .map(|(_, v)| v.clone());
        (None, found)
    }
}

fn lookup_name(map: &HashMap<String, f64>, name_lower: &str) -> Option<f64> {
    if let Some(v) = map.get(name_lower) {
        return Some(*v);
    }
    let last = name_lower.rsplit(' ').next().unwrap_or(name_lower);
    map.iter()
        .find(|(k, _)| k.contains(name_lower) || k.contains(last) || name_lower.contains(k.as_str()))
        .map(|(_, v)| *v)
}
fn lookup_name_book(map: &HashMap<String, (f64, String)>, name_lower: &str) -> Option<(f64, String)> {
    if let Some(v) = map.get(name_lower) {
        return Some(v.clone());
    }
    let last = name_lower.rsplit(' ').next().unwrap_or(name_lower);
    map.iter()
        .find(|(k, _)| k.contains(name_lower) || k.contains(last) || name_lower.contains(k.as_str()))
        .map(|(_, v)| v.clone())
}

/// Lowercase + strip common Latin accents, so "Touré" matches the book's "Toure".
pub fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' | 'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' | 'Ø' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
            'ñ' | 'Ñ' => 'n',
            'ç' | 'Ç' => 'c',
            'ý' | 'ÿ' | 'Ý' => 'y',
            'š' | 'Š' | 'ş' | 'Ş' => 's',
            'ž' | 'Ž' => 'z',
            'č' | 'Č' | 'ć' | 'Ć' => 'c',
            'đ' | 'Đ' => 'd',
            'ř' | 'Ř' => 'r',
            'ł' | 'Ł' => 'l',
            'ğ' | 'Ğ' => 'g',
            other => other.to_ascii_lowercase(),
        })
        .collect::<String>()
        .to_lowercase()
}

fn round4(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}
fn parse_odd(v: &Value) -> Option<f64> {
    v.as_str().and_then(|s| s.trim().parse::<f64>().ok()).or_else(|| v.as_f64())
}

/// Map a bet (id, name) to our internal market tag, or None to ignore.
fn bet_tag(id: i64, name: &str) -> Option<&'static str> {
    match id {
        1 => return Some("1x2"),
        5 => return Some("ou"),
        8 => return Some("btts"),
        12 => return Some("dc"),
        16 => return Some("tgoals_home"),
        17 => return Some("tgoals_away"),
        57 => return Some("corners_home"),
        58 => return Some("corners_away"),
        92 => return Some("scorer"),
        212 => return Some("assist"),
        102 | 251 => return Some("card"),
        242 | 264 => return Some("sot"),
        240 | 241 | 265 => return Some("pshots"),
        266 => return Some("fouls"),
        272 | 278 => return Some("tackles"),
        273 | 279 => return Some("passes"),
        _ => {}
    }
    let n = name.to_lowercase();
    if n.contains("anytime") && n.contains("scorer") {
        Some("scorer")
    } else {
        None
    }
}

/// One row of odds: (book_id, book_name, tag, value_lower, odd).
fn collect_all(json: &Value) -> Vec<(i64, String, &'static str, String, f64)> {
    let mut rows = Vec::new();
    let resp = response_array(json);
    let entry = match resp.first() {
        Some(e) => e,
        None => return rows,
    };
    let bookmakers = match entry.get("bookmakers").and_then(|b| b.as_array()) {
        Some(b) => b,
        None => return rows,
    };
    for bm in bookmakers {
        let bid = bm.get("id").and_then(|v| v.as_i64()).unwrap_or(-1);
        let bname = bm.get("name").and_then(|v| v.as_str()).unwrap_or("book").to_string();
        let bets = match bm.get("bets").and_then(|b| b.as_array()) {
            Some(b) => b,
            None => continue,
        };
        for bet in bets {
            let id = bet.get("id").and_then(|v| v.as_i64()).unwrap_or(-1);
            let name = bet.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let tag = match bet_tag(id, name) {
                Some(t) => t,
                None => continue,
            };
            if let Some(values) = bet.get("values").and_then(|v| v.as_array()) {
                for val in values {
                    let label = val.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(odd) = val.get("odd").and_then(parse_odd) {
                        rows.push((bid, bname.clone(), tag, fold(label), odd));
                    }
                }
            }
        }
    }
    rows
}

/// POWER-method de-vig: find k with Σ(1/oᵢ)ᵏ = 1, then trueᵢ = (1/oᵢ)ᵏ. Corrects
/// the favourite-longshot bias of naive proportional de-vig.
fn devig(odds: &[f64]) -> Vec<f64> {
    let r: Vec<f64> = odds.iter().map(|o| if *o > 1.0 { 1.0 / o } else { 0.0 }).collect();
    let sum: f64 = r.iter().sum();
    if sum <= 0.0 {
        return vec![0.0; odds.len()];
    }
    let proportional = || -> Vec<f64> { r.iter().map(|x| x / sum).collect() };
    if r.iter().any(|x| *x <= 0.0) {
        return proportional();
    }
    let f = |k: f64| -> f64 { r.iter().map(|x| x.powf(k)).sum::<f64>() - 1.0 };
    let (mut lo, mut hi) = (0.2_f64, 10.0_f64);
    let (flo, fhi) = (f(lo), f(hi));
    if flo == 0.0 {
        return r.iter().map(|x| x.powf(lo)).collect();
    }
    if flo.signum() == fhi.signum() {
        return proportional();
    }
    let mut flo_s = flo.signum();
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        let fm = f(mid);
        if fm.abs() < 1e-10 {
            lo = mid;
            hi = mid;
            break;
        }
        if fm.signum() == flo_s {
            lo = mid;
            flo_s = fm.signum();
        } else {
            hi = mid;
        }
    }
    let k = 0.5 * (lo + hi);
    r.iter().map(|x| x.powf(k)).collect()
}

/// O/U tags that carry "Over X.5 / Under X.5" lines.
const OU_TAGS: [&str; 5] = ["ou", "tgoals_home", "tgoals_away", "corners_home", "corners_away"];
/// Player threshold prop tags (value like "Name - 1+" or "Name Over 0.5").
const PROP_TAGS: [&str; 5] = ["sot", "pshots", "fouls", "tackles", "passes"];

pub fn parse_fixture_odds(json: &Value, allowed: &[String]) -> FixtureOdds {
    let mut out = FixtureOdds::default();
    let rows = collect_all(json);
    let allow = |name: &str| {
        allowed.is_empty() || allowed.iter().any(|a| name.to_lowercase().contains(&a.to_lowercase()))
    };

    // ---- best (max) book price per key across the chosen books ----
    for (bid, bname, tag, value, odd) in &rows {
        if *bid == PINNACLE || !allow(bname) {
            continue;
        }
        let best = |map: &mut HashMap<String, (f64, String)>, key: String, odd: f64, bname: &str| {
            let e = map.entry(key).or_insert((0.0, String::new()));
            if odd > e.0 {
                *e = (odd, bname.to_string());
            }
        };
        match *tag {
            "scorer" => best(&mut out.book_scorer, value.clone(), *odd, bname),
            "assist" | "card" => best(&mut out.book_prop, format!("{tag}|{value}|1"), *odd, bname),
            t if PROP_TAGS.contains(&t) => {
                if let Some((name, thr)) = parse_prop(value) {
                    best(&mut out.book_prop, format!("{t}|{name}|{thr}"), *odd, bname);
                }
            }
            t if OU_TAGS.contains(&t) => {
                if let Some((side, line)) = ou_parse(value) {
                    best(&mut out.book, format!("{t}|{side}|{line}"), *odd, bname);
                }
            }
            "1x2" => {
                if let Some(s) = onex2_sel(value) {
                    best(&mut out.book, format!("1x2|{s}"), *odd, bname);
                }
            }
            "btts" => {
                let s = if value.starts_with("yes") { "yes" } else if value.starts_with("no") { "no" } else { "" };
                if !s.is_empty() {
                    best(&mut out.book, format!("btts|{s}"), *odd, bname);
                }
            }
            "dc" => {
                if let Some(s) = dc_sel(value) {
                    best(&mut out.book, format!("dc|{s}"), *odd, bname);
                }
            }
            _ => {}
        }
    }

    // ---- Pinnacle de-vig → true probabilities ----
    let pin_get = |tag: &str, sel: &str| -> Option<f64> {
        rows.iter()
            .find(|(bid, _, t, v, _)| *bid == PINNACLE && *t == tag && v.contains(sel))
            .map(|(_, _, _, _, o)| *o)
    };

    if let (Some(h), Some(d), Some(a)) = (pin_get("1x2", "home"), pin_get("1x2", "draw"), pin_get("1x2", "away")) {
        let p = devig(&[h, d, a]);
        out.pin.insert("1x2|home".into(), round4(p[0]));
        out.pin.insert("1x2|draw".into(), round4(p[1]));
        out.pin.insert("1x2|away".into(), round4(p[2]));
        out.pin.insert("dc|homedraw".into(), round4(p[0] + p[1]));
        out.pin.insert("dc|awaydraw".into(), round4(p[1] + p[2]));
        out.pin.insert("dc|homeaway".into(), round4(p[0] + p[2]));
    }

    // Every O/U-style market: de-vig each line's over/under pair at Pinnacle.
    for tag in OU_TAGS {
        let mut lines: HashMap<String, (Option<f64>, Option<f64>)> = HashMap::new();
        for (bid, _, t, value, odd) in &rows {
            if *bid != PINNACLE || *t != tag {
                continue;
            }
            if let Some((side, line)) = ou_parse(value) {
                let e = lines.entry(line).or_insert((None, None));
                if side == "over" {
                    e.0 = Some(*odd);
                } else {
                    e.1 = Some(*odd);
                }
            }
        }
        for (line, (o, u)) in lines {
            if let (Some(o), Some(u)) = (o, u) {
                let p = devig(&[o, u]);
                out.pin.insert(format!("{tag}|over|{line}"), round4(p[0]));
                out.pin.insert(format!("{tag}|under|{line}"), round4(p[1]));
            }
        }
    }

    if let (Some(y), Some(n)) = (pin_get("btts", "yes"), pin_get("btts", "no")) {
        let p = devig(&[y, n]);
        out.pin.insert("btts|yes".into(), round4(p[0]));
        out.pin.insert("btts|no".into(), round4(p[1]));
    }

    for (bid, _, tag, value, odd) in &rows {
        if *bid == PINNACLE && *tag == "scorer" && *odd > 1.0 {
            out.pin_scorer.insert(value.clone(), round4(1.0 / odd));
        }
    }

    out
}

/// "Name - 1+" or "Name Over 0.5" → (name, threshold). Over X.5 → ceil = X+1.
fn parse_prop(value: &str) -> Option<(String, i64)> {
    if let Some(idx) = value.rfind(" - ") {
        let name = value[..idx].trim().to_string();
        let thr: i64 = value[idx + 3..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?;
        return Some((name, thr.max(1)));
    }
    if let Some(idx) = value.find(" over ") {
        let name = value[..idx].trim().to_string();
        let num: f64 = value[idx + 6..].chars().filter(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse().ok()?;
        return Some((name, num.ceil() as i64));
    }
    None
}

fn onex2_sel(label: &str) -> Option<&'static str> {
    if label.contains("home") || label == "1" {
        Some("home")
    } else if label.contains("draw") || label == "x" {
        Some("draw")
    } else if label.contains("away") || label == "2" {
        Some("away")
    } else {
        None
    }
}
/// "over 2.5" / "Under 1.5" → ("over"|"under", "2.5"). Half lines only.
fn ou_parse(label: &str) -> Option<(&'static str, String)> {
    let l = label.trim().to_lowercase();
    let side = if l.starts_with("over") {
        "over"
    } else if l.starts_with("under") {
        "under"
    } else {
        return None;
    };
    let num: String = l.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    let val: f64 = num.parse().ok()?;
    if (val * 2.0).fract().abs() > 1e-9 {
        return None;
    }
    Some((side, format!("{val:.1}")))
}
fn dc_sel(label: &str) -> Option<&'static str> {
    let l = label.replace(' ', "");
    if (l.contains("home") && l.contains("draw")) || l == "1x" {
        Some("homedraw")
    } else if (l.contains("draw") && l.contains("away")) || l == "x2" {
        Some("awaydraw")
    } else if (l.contains("home") && l.contains("away")) || l == "12" {
        Some("homeaway")
    } else {
        None
    }
}

/// One-line predictions summary (weak signal context for the model).
pub fn predictions_summary(json: &Value, fixture_label: &str) -> Option<String> {
    let entry = response_array(json);
    let p = entry.first()?.get("predictions")?;
    let pct = p.get("percent");
    let h = pct.and_then(|x| x.get("home")).and_then(|v| v.as_str()).unwrap_or("?");
    let d = pct.and_then(|x| x.get("draw")).and_then(|v| v.as_str()).unwrap_or("?");
    let a = pct.and_then(|x| x.get("away")).and_then(|v| v.as_str()).unwrap_or("?");
    let advice = p.get("advice").and_then(|v| v.as_str()).unwrap_or("");
    Some(format!("{fixture_label}: win% H/D/A={h}/{d}/{a}; advice=\"{advice}\""))
}
