// Powabetz floating research bar + SLIP ASSISTANT. Enabled from the extension
// popup; renders in a shadow root so page CSS can't touch it.
//
// Research bar: shows the ingest count, whether THIS page is already ingested
// (offering "Update"), and a one-click ingest.
//
// Slip assistant (🎯): pulls the latest built slate from the desktop app and
// floats it beside the page. On a Bet365 bet-builder page, clicking a leg
// FINDS that selection in the page (name + market text matching) and clicks
// it, adding it to the real bet slip — build in the app, place with taps.

(() => {
  const HOST_ID = "powabetz-bar-host";
  let state = { enabled: false, min: false, status: null };
  // Slip assistant state: open, latest build payload, ticket index, per-leg results.
  let slip = { open: false, data: null, idx: 0, res: {} };

  function extract() {
    const root =
      document.querySelector("main, article, [role=main], #content, .content, #main") || document.body;
    const content = ((root && root.innerText) || "").replace(/\n{3,}/g, "\n\n").trim();
    return { url: location.href, title: document.title, content, note: "" };
  }

  function send(msg) {
    return new Promise((resolve) => {
      try {
        chrome.runtime.sendMessage(msg, (resp) => resolve(chrome.runtime.lastError ? null : resp));
      } catch {
        resolve(null);
      }
    });
  }

  // ---------- leg placement (the whole point) ----------

  const norm = (s) =>
    (s || "")
      .normalize("NFD")
      .replace(/[̀-ͯ]/g, "") // strip accents: Mbappé → Mbappe
      .toLowerCase()
      .replace(/[^a-z0-9 ]+/g, " ")
      .replace(/\s+/g, " ")
      .trim();

  // Our market names → Bet365 bet-builder MARKET GROUP names (verified against
  // a real builder page dump). First entry = the exact group header to expand.
  const MARKET_TEXT = {
    "anytime scorer": ["player to score", "to score at any time", "goalscorer"],
    "multi scorer (2+)": ["player to score", "to score 2 or more"],
    "anytime assist": ["player to score or assist", "to assist"],
    "to score or assist": ["player to score or assist"],
    "shots on target": ["player shots on target"],
    "player shots": ["player shots"],
    "to be carded": ["player cards", "to be booked"],
    tackles: ["player tackles"],
    "passes completed": ["player passes"],
    "fouls committed": ["player fouls committed"],
    "fouls drawn": ["player to be fouled"],
    "goalkeeper saves": ["goalkeeper saves"],
    "match corners": ["corners"],
    "match shots on target": ["total shots on target"],
    "match shots": ["total shots"],
    "match cards": ["cards"],
    "team corners": ["corners"],
    "team total cards": ["cards"],
    "team shots": ["total shots"],
    "team offsides": ["total offsides"],
    btts: ["both teams to score"],
    "match result": ["result"],
    "over": ["total goals", "goals range"],
    "under": ["total goals", "goals range"],
  };

  function marketPhrases(market) {
    const m = norm(market);
    for (const k of Object.keys(MARKET_TEXT)) {
      if (m.includes(k) || k.includes(m)) return MARKET_TEXT[k];
    }
    return [m]; // unknown market — try its own words
  }

  // All visible elements whose text contains the needle (normalized).
  function findTextNodes(needle) {
    const out = [];
    const tw = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
      acceptNode: (n) =>
        n.nodeValue && n.nodeValue.trim().length >= 3 && norm(n.nodeValue).includes(needle)
          ? NodeFilter.FILTER_ACCEPT
          : NodeFilter.FILTER_SKIP,
    });
    let n;
    while ((n = tw.nextNode())) {
      const el = n.parentElement;
      if (!el || el.closest(`#${HOST_ID}`)) continue;
      const r = el.getBoundingClientRect?.();
      if (r && (r.width > 0 || r.height > 0)) out.push(el);
      if (out.length > 60) break; // enough candidates
    }
    return out;
  }

  // Does a nearby ancestor's (bounded) subtree text mention a market phrase?
  function marketContextScore(el, phrases) {
    let node = el;
    for (let up = 0; up < 10 && node; up++, node = node.parentElement) {
      const t = norm(node.innerText || "");
      if (!t) continue;
      if (phrases.some((p) => t.includes(norm(p)))) return 10 - up; // closer = better
      if (t.length > 4000) break; // subtree grew page-wide — stop climbing
    }
    return 0;
  }

  // Climb from the matched element to the thing that actually takes the click.
  function clickable(el) {
    let node = el;
    for (let up = 0; up < 8 && node; up++, node = node.parentElement) {
      const tag = node.tagName;
      if (tag === "BUTTON" || tag === "A" || node.getAttribute?.("role") === "button") return node;
      try {
        if (getComputedStyle(node).cursor === "pointer") return node;
      } catch {}
    }
    return el;
  }

  function realClick(el) {
    el.scrollIntoView({ block: "center", behavior: "instant" });
    const r = el.getBoundingClientRect();
    const opts = {
      bubbles: true,
      cancelable: true,
      view: window,
      clientX: r.left + r.width / 2,
      clientY: r.top + r.height / 2,
    };
    for (const type of ["pointerdown", "mousedown", "pointerup", "mouseup", "click"]) {
      el.dispatchEvent(type.startsWith("pointer") ? new PointerEvent(type, opts) : new MouseEvent(type, opts));
    }
  }

  // Bet365 lazy-renders group contents: rows only exist in the DOM once their
  // market group is EXPANDED. Find the group header by its (verified) name and
  // click it open when the selection isn't on the page yet.
  function expandGroup(phrases) {
    const headers = document.querySelectorAll(
      '.bbw-BetBuilderEmbeddedMarketGroupButton_TopRow, [class*="MarketGroupButton"]'
    );
    for (const h of headers) {
      const t = norm(h.textContent || "");
      if (t && phrases.some((p) => t.includes(norm(p)))) {
        realClick(h);
        return true;
      }
    }
    return false;
  }

  function searchSubject(subject) {
    let candidates = findTextNodes(subject);
    if (candidates.length === 0 && subject.includes(" ")) {
      // Fallback: surname only (pages often show "K. Mbappe" / just "Mbappe").
      const last = subject.split(" ").pop();
      if (last.length > 3) candidates = findTextNodes(last);
    }
    return candidates;
  }

  // Try to place one leg on the page. Returns "ok" | "loose" | "miss" | "manual".
  async function placeLeg(leg) {
    const phrases = marketPhrases(leg.market || "");
    // Player legs match on the player's name; team legs on the team name.
    const subject = norm(leg.selection || "");
    if (!subject || subject === "match" || subject === "both teams") return "manual";
    let candidates = searchSubject(subject);
    // Not rendered? Expand the market group and give it a beat to render.
    if (candidates.length === 0 && expandGroup(phrases)) {
      await new Promise((r) => setTimeout(r, 900));
      candidates = searchSubject(subject);
    }
    if (candidates.length === 0) return "miss";
    // Prefer candidates sitting inside the right market section.
    let best = null;
    let bestScore = -1;
    for (const el of candidates) {
      const score = marketContextScore(el, phrases);
      if (score > bestScore) {
        bestScore = score;
        best = el;
      }
    }
    if (!best) return "miss";
    realClick(clickable(best));
    return bestScore > 0 ? "ok" : "loose"; // loose = clicked, but market unconfirmed
  }

  // ---------- UI ----------

  function unmount() {
    document.getElementById(HOST_ID)?.remove();
  }

  function legLabel(l) {
    const line = l.line && l.line !== l.selection ? ` ${l.line}` : "";
    const odds = l.book_odds ? ` @${Number(l.book_odds).toFixed(2)}` : "";
    return { top: l.selection, sub: `${l.market}${line}${odds} · ${l.match || ""}` };
  }

  function render() {
    unmount();
    if (!state.enabled) return;
    const host = document.createElement("div");
    host.id = HOST_ID;
    const sh = host.attachShadow({ mode: "closed" });
    const st = state.status;
    const ingested = !!st?.ingested;
    const count = st ? `${st.count}${st.new_count ? ` (${st.new_count} new)` : ""}` : "…";

    const tickets = (slip.data && slip.data.result && slip.data.result.tickets) || [];
    const t = tickets[Math.min(slip.idx, Math.max(0, tickets.length - 1))];
    const ago = slip.data?.created_at
      ? `${Math.max(0, Math.round((Date.now() / 1000 - slip.data.created_at) / 60))}m ago`
      : "";

    sh.innerHTML = `
      <style>
        :host { all: initial; }
        .wrap { position: fixed; right: 0; top: 32%; z-index: 2147483647;
          font: 12px/1.3 ui-sans-serif, system-ui, sans-serif; color: #e2e8f0; }
        .bar { display: flex; align-items: center; gap: 8px; background: #0e1116ee;
          border: 1px solid #262c36; border-right: 0; border-radius: 12px 0 0 12px;
          padding: 8px 10px; box-shadow: 0 4px 18px rgba(0,0,0,.45); backdrop-filter: blur(4px); }
        .tab { display: flex; align-items: center; justify-content: center; width: 26px; height: 44px;
          background: #0e1116ee; border: 1px solid #262c36; border-right: 0;
          border-radius: 10px 0 0 10px; cursor: pointer; box-shadow: 0 4px 14px rgba(0,0,0,.4); }
        .count { color: #94a3b8; white-space: nowrap; }
        .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
        .ok { background: #38d39f; } .no { background: #64748b; }
        button.go { border: 0; border-radius: 8px; padding: 6px 10px; font-weight: 700; cursor: pointer;
          background: #38d39f; color: #0e1116; white-space: nowrap; }
        button.go.upd { background: #e8b53a; }
        button.go[disabled] { opacity: .55; cursor: default; }
        .chev, .slipbtn { background: none; border: 0; color: #94a3b8; cursor: pointer; font-size: 14px; padding: 2px; }
        .slipbtn.on { color: #38d39f; }
        .st { white-space: nowrap; color: ${ingested ? "#38d39f" : "#94a3b8"}; }
        .panel { margin-top: 8px; width: 280px; max-height: 60vh; overflow-y: auto; background: #0e1116f2;
          border: 1px solid #262c36; border-right: 0; border-radius: 12px 0 0 12px; padding: 10px;
          box-shadow: 0 4px 18px rgba(0,0,0,.45); }
        .phead { display: flex; align-items: center; justify-content: space-between; gap: 6px; margin-bottom: 6px; }
        .ptitle { font-weight: 700; font-size: 12px; color: #e2e8f0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .nav { display: flex; gap: 4px; align-items: center; color: #94a3b8; }
        .nav button { background: #171b22; border: 1px solid #262c36; color: #e2e8f0; border-radius: 6px;
          cursor: pointer; padding: 2px 8px; }
        .leg { display: flex; align-items: center; gap: 8px; background: #171b22; border: 1px solid #262c36;
          border-radius: 8px; padding: 6px 8px; margin-bottom: 5px; cursor: pointer; }
        .leg:hover { border-color: #38d39f88; }
        .leg .name { font-weight: 600; font-size: 12px; }
        .leg .sub { color: #94a3b8; font-size: 10px; }
        .leg .mark { margin-left: auto; font-size: 13px; flex: none; }
        .hint { color: #64748b; font-size: 10px; margin-top: 4px; }
        .meta { color: #64748b; font-size: 10px; }
        .allbtn { width: 100%; margin-top: 2px; border: 0; border-radius: 8px; padding: 7px; font-weight: 700;
          cursor: pointer; background: #38d39f; color: #0e1116; }
      </style>
      ${
        state.min
          ? `<div class="wrap"><div class="tab" title="Powabetz — expand">‹</div></div>`
          : `<div class="wrap">
            <div class="bar">
              <span class="dot ${ingested ? "ok" : "no"}"></span>
              <span class="st">${ingested ? "✓ ingested" : "not ingested"}</span>
              <span class="count">🧲 ${count}</span>
              <button class="go ${ingested ? "upd" : ""}">${ingested ? "Update" : "Ingest"}</button>
              <button class="slipbtn ${slip.open ? "on" : ""}" title="Slip assistant — your latest build; click a leg to place it on this page">🎯</button>
              <button class="chev" title="Minimize">›</button>
            </div>
            ${
              slip.open
                ? `<div class="panel">
                    ${
                      !slip.data
                        ? `<div class="meta">Loading latest build…</div>`
                        : slip.data.ok === false
                          ? `<div class="meta" style="color:#f87171">${slip.data.error || "Couldn't fetch the latest build."}</div>`
                        : tickets.length === 0
                          ? `<div class="meta">No builds yet — build tickets in the app first.</div>`
                          : `<div class="phead">
                              <span class="ptitle">${(t.title || t.type || "Ticket")}</span>
                              <span class="nav">
                                <button class="prev">‹</button>
                                <span>${slip.idx + 1}/${tickets.length}</span>
                                <button class="next">›</button>
                              </span>
                            </div>
                            <div class="meta">built ${ago} · ${(t.legs || []).length} legs${t.combined_odds ? ` · @${Number(t.combined_odds).toFixed(2)}` : ""}</div>
                            <div style="height:6px"></div>
                            ${(t.legs || [])
                              .map((l, i) => {
                                const lab = legLabel(l);
                                const r = slip.res[`${slip.idx}:${i}`];
                                const mark = r === "ok" ? "✅" : r === "loose" ? "🟡" : r === "miss" ? "❌" : r === "manual" ? "✋" : "▶";
                                return `<div class="leg" data-i="${i}">
                                  <div><div class="name">${lab.top}</div><div class="sub">${lab.sub}</div></div>
                                  <span class="mark">${mark}</span>
                                </div>`;
                              })
                              .join("")}
                            <button class="allbtn">▶ Place all legs</button>
                            <div class="hint">Open the game's BET BUILDER first and expand the market sections. ✅ placed · 🟡 clicked (verify on the slip) · ❌ not found on page · ✋ match-level leg — add by hand.</div>`
                    }
                  </div>`
                : ""
            }
          </div>`
      }
    `;
    document.documentElement.appendChild(host);

    if (state.min) {
      sh.querySelector(".tab").addEventListener("click", () => {
        state.min = false;
        chrome.storage.local.set({ barMin: false });
        render();
      });
      return;
    }
    sh.querySelector(".chev").addEventListener("click", () => {
      state.min = true;
      chrome.storage.local.set({ barMin: true });
      render();
    });
    const go = sh.querySelector(".go");
    go.addEventListener("click", async () => {
      go.disabled = true;
      go.textContent = "…";
      const resp = await send({ type: "pbz-ingest", payload: extract() });
      if (resp?.ok) {
        await refreshStatus();
      } else {
        go.textContent = "✗ retry";
        go.disabled = false;
      }
    });
    sh.querySelector(".slipbtn").addEventListener("click", async () => {
      slip.open = !slip.open;
      render();
      if (slip.open && !slip.data) {
        slip.data = (await send({ type: "pbz-ticket" })) || { result: { tickets: [] } };
        render();
      }
    });
    if (slip.open && slip.data) {
      sh.querySelector(".prev")?.addEventListener("click", () => {
        slip.idx = Math.max(0, slip.idx - 1);
        render();
      });
      sh.querySelector(".next")?.addEventListener("click", () => {
        slip.idx = Math.min(tickets.length - 1, slip.idx + 1);
        render();
      });
      const legs = (t && t.legs) || [];
      sh.querySelectorAll(".leg").forEach((el) => {
        el.addEventListener("click", async () => {
          const i = parseInt(el.getAttribute("data-i"), 10);
          slip.res[`${slip.idx}:${i}`] = await placeLeg(legs[i]);
          render();
        });
      });
      sh.querySelector(".allbtn")?.addEventListener("click", async () => {
        // Sequential with a gap so the builder can register each pick.
        for (let i = 0; i < legs.length; i++) {
          slip.res[`${slip.idx}:${i}`] = await placeLeg(legs[i]);
          render();
          await new Promise((r) => setTimeout(r, 700));
        }
      });
    }
  }

  async function refreshStatus() {
    state.status = await send({ type: "pbz-status", url: location.href });
    render();
  }

  chrome.storage.local.get(["barEnabled", "barMin"]).then(async (c) => {
    state.enabled = !!c.barEnabled;
    state.min = !!c.barMin;
    if (state.enabled) await refreshStatus();
  });

  chrome.storage.onChanged.addListener(async (ch, area) => {
    if (area !== "local") return;
    if ("barEnabled" in ch) {
      state.enabled = !!ch.barEnabled.newValue;
      if (state.enabled) await refreshStatus();
      else unmount();
    }
  });
})();
