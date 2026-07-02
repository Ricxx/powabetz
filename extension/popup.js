const $ = (id) => document.getElementById(id);

chrome.storage.local.get(["endpoint", "token", "barEnabled"]).then((c) => {
  $("endpoint").value = c.endpoint || "http://127.0.0.1:8765/ingest";
  $("token").value = c.token || "";
  $("bar").checked = !!c.barEnabled;
});

// The floating bar toggles LIVE on every open tab (content scripts listen to
// storage changes) — no reloads needed; off removes it everywhere.
$("bar").onchange = () => chrome.storage.local.set({ barEnabled: $("bar").checked });

$("save").onclick = async () => {
  await chrome.storage.local.set({
    endpoint: $("endpoint").value.trim(),
    token: $("token").value.trim(),
  });
  $("status").textContent = "Saved.";
};

$("ingest").onclick = async () => {
  $("status").textContent = "Sending…";
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    const [res] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => {
        const root =
          document.querySelector("main, article, [role=main], #content, .content, #main") || document.body;
        const content = ((root && root.innerText) || "").replace(/\n{3,}/g, "\n\n").trim();
        return { url: location.href, title: document.title, content, note: "" };
      },
    });
    const r = await fetch($("endpoint").value.trim(), {
      method: "POST",
      headers: { "content-type": "application/json", "x-ingest-token": $("token").value.trim() },
      body: JSON.stringify(res.result),
    });
    $("status").textContent = r.ok ? "Ingested ✓ — see the app's Ingest screen." : `Failed (${r.status})`;
  } catch (e) {
    $("status").textContent = "Error: " + e;
  }
};
