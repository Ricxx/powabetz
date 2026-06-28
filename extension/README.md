# Powabetz Ingest extension

Send any web page's content to your Powabetz desktop app — works on Cloudflare-
protected sites (Forebet etc.) because the page renders in *your* real browser.

## Install (Chrome / Edge / Brave)
1. Open `chrome://extensions`, turn on **Developer mode** (top-right).
2. Click **Load unpacked** and pick this `extension/` folder.
3. Open the Powabetz app → **🧲 Ingest** screen → copy the **Endpoint** + **Token**.
4. Click the extension icon, paste both, **Save settings**.

## Use
- On any fixture page, **right-click → Add to Powabetz** (or **Ingest with note…**
  to tell Haiku what to pull, e.g. "only the predicted scoreline + corners").
- Or click the extension icon → **Ingest this page**.
- The page appears in the app's **Ingest** screen. Hit **Process with Haiku** to
  structure it and tag the fixture; it then feeds your builds.

The app must be running (its local ingest server listens on 127.0.0.1). Pages are
cached by URL, so re-ingesting the same page won't duplicate it.
