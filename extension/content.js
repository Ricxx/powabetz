// Powabetz floating research bar. Enabled from the extension popup; renders in
// a shadow root so page CSS can't touch it. Shows the ingest count, whether
// THIS page is already ingested (offering "Update"), and a one-click ingest —
// built for rapid multi-page research. Minimizes to a chevron pull-tab; when
// disabled in the popup it unmounts everywhere, instantly.

(() => {
  const HOST_ID = "powabetz-bar-host";
  let state = { enabled: false, min: false, status: null };

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

  function unmount() {
    document.getElementById(HOST_ID)?.remove();
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

    sh.innerHTML = `
      <style>
        :host { all: initial; }
        .wrap { position: fixed; right: 0; top: 40%; z-index: 2147483647;
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
        .chev { background: none; border: 0; color: #94a3b8; cursor: pointer; font-size: 14px; padding: 2px; }
        .st { white-space: nowrap; color: ${ingested ? "#38d39f" : "#94a3b8"}; }
      </style>
      ${
        state.min
          ? `<div class="wrap"><div class="tab" title="Powabetz — expand">‹</div></div>`
          : `<div class="wrap"><div class="bar">
              <span class="dot ${ingested ? "ok" : "no"}"></span>
              <span class="st">${ingested ? "✓ ingested" : "not ingested"}</span>
              <span class="count">🧲 ${count}</span>
              <button class="go ${ingested ? "upd" : ""}">${ingested ? "Update" : "Ingest"}</button>
              <button class="chev" title="Minimize">›</button>
            </div></div>`
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
