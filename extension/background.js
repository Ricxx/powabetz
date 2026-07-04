// Powabetz Ingest — service worker. Right-click any page → send its visible text
// to the local Powabetz app. Because it runs in YOUR browser, there's no
// bot-detection / Cloudflare wall — the page is already rendered.

// ONE menu item, one click — notes are added in the app before processing.
const MENU = [{ id: "ingest", title: "Add to Powabetz" }];

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    MENU.forEach((m) =>
      chrome.contextMenus.create({ id: m.id, title: m.title, contexts: ["page", "selection", "link"] })
    );
  });
});

async function grab(tabId) {
  const [res] = await chrome.scripting.executeScript({
    target: { tabId },
    func: () => {
      // Prefer the main content region so we skip nav/header/footer token-busters.
      const root =
        document.querySelector("main, article, [role=main], #content, .content, #main") || document.body;
      const content = ((root && root.innerText) || "").replace(/\n{3,}/g, "\n\n").trim();
      return { url: location.href, title: document.title, content, note: "" };
    },
  });
  return res.result;
}

async function send(payload) {
  const cfg = await chrome.storage.local.get(["endpoint", "token"]);
  const endpoint = cfg.endpoint || "http://127.0.0.1:8765/ingest";
  const r = await fetch(endpoint, {
    method: "POST",
    headers: { "content-type": "application/json", "x-ingest-token": cfg.token || "" },
    body: JSON.stringify(payload),
  });
  return r.ok;
}

function badge(text, ok) {
  chrome.action.setBadgeText({ text });
  chrome.action.setBadgeBackgroundColor({ color: ok ? "#16a34a" : "#dc2626" });
  setTimeout(() => chrome.action.setBadgeText({ text: "" }), 2500);
}

// Floating-bar messaging: the content script can't reach localhost from an
// https page (mixed content), so the service worker does the fetches.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  (async () => {
    const cfg = await chrome.storage.local.get(["endpoint", "token"]);
    const endpoint = cfg.endpoint || "http://127.0.0.1:8765/ingest";
    const base = endpoint.replace(/\/ingest\/?$/, "");
    try {
      if (msg?.type === "pbz-status") {
        const r = await fetch(`${base}/status?url=${encodeURIComponent(msg.url || "")}`, {
          headers: { "x-ingest-token": cfg.token || "" },
        });
        sendResponse(r.ok ? await r.json() : null);
        return;
      }
      if (msg?.type === "pbz-ticket") {
        // Latest built slate from the app - for the on-page slip assistant.
        try {
          const r = await fetch(`${base}/ticket`, {
            headers: { "x-ingest-token": cfg.token || "" },
          });
          if (r.status === 401) {
            sendResponse({ ok: false, error: "Unauthorized — the token in the extension popup doesn't match the app (Settings → ingest token)." });
          } else if (!r.ok) {
            sendResponse({ ok: false, error: `App answered HTTP ${r.status} — is the app up to date? (rebuild + restart)` });
          } else {
            sendResponse(await r.json());
          }
        } catch {
          sendResponse({ ok: false, error: "Can't reach the app on 127.0.0.1:8765 — is Powabetz running?" });
        }
        return;
      }
      if (msg?.type === "pbz-ingest") {
        const ok = await send(msg.payload);
        badge(ok ? "✓" : "!", ok);
        sendResponse({ ok });
        return;
      }
    } catch {
      sendResponse(null);
    }
  })();
  return true; // async sendResponse
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (!tab?.id) return;
  try {
    const data = await grab(tab.id);
    const ok = await send(data);
    badge(ok ? "✓" : "!", ok);
  } catch (e) {
    badge("!", false);
  }
});
