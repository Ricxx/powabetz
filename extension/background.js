// Powabetz Ingest — service worker. Right-click any page → send its visible text
// to the local Powabetz app. Because it runs in YOUR browser, there's no
// bot-detection / Cloudflare wall — the page is already rendered.

const MENU = [
  { id: "ingest", title: "Add to Powabetz" },
  { id: "ingest-note", title: "Ingest with note…" },
];

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    MENU.forEach((m) =>
      chrome.contextMenus.create({ id: m.id, title: m.title, contexts: ["page", "selection", "link"] })
    );
  });
});

async function grab(tabId, askNote) {
  const [res] = await chrome.scripting.executeScript({
    target: { tabId },
    func: (ask) => {
      const note = ask ? window.prompt("Note for Haiku — what should it extract?", "") || "" : "";
      // Prefer the main content region so we skip nav/header/footer token-busters.
      const root =
        document.querySelector("main, article, [role=main], #content, .content, #main") || document.body;
      const content = ((root && root.innerText) || "").replace(/\n{3,}/g, "\n\n").trim();
      return { url: location.href, title: document.title, content, note };
    },
    args: [askNote],
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

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (!tab?.id) return;
  try {
    const data = await grab(tab.id, info.menuItemId === "ingest-note");
    const ok = await send(data);
    badge(ok ? "✓" : "!", ok);
  } catch (e) {
    badge("!", false);
  }
});
