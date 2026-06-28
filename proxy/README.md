# Powabetz key proxy

Lets other people use Powabetz **without ever seeing your API keys**. The app calls
your proxy; the proxy holds the real keys and injects them. You hand each user a
**token** (not a key) which you can revoke any time.

> ⚠️ You pay for everyone's usage. Hand out tokens only to people you trust, and
> keep an eye on your provider dashboards. (The app's own daily request meter still
> applies per install.)

---

## Option A — Cloudflare Worker (recommended, free)

1. Install the CLI and log in (one time):
   ```sh
   npm i -g wrangler
   wrangler login
   ```
2. From this `proxy/` folder, set your secrets (you'll be prompted to paste each):
   ```sh
   wrangler secret put API_FOOTBALL_KEY
   wrangler secret put ANTHROPIC_KEY
   wrangler secret put XAI_KEY
   wrangler secret put PROXY_TOKENS      # e.g.  alice-7f3a,bob-91kd,carol-22xt
   ```
   `PROXY_TOKENS` is a comma-separated list — invent one random token per user.
3. Deploy:
   ```sh
   wrangler deploy
   ```
   You'll get a URL like `https://powabetz-proxy.<you>.workers.dev`.

That URL is your **Proxy URL**. Each comma-separated value in `PROXY_TOKENS` is a
user **Access token**.

## Option B — Plain Node (any VPS, or local + a tunnel)

```sh
API_FOOTBALL_KEY=... ANTHROPIC_KEY=... XAI_KEY=... \
PROXY_TOKENS=alice-7f3a,bob-91kd PORT=8787 node server.js
```

To expose a local server to a friend, run a tunnel (e.g. `cloudflared tunnel --url
http://localhost:8787`) and give them the public URL it prints.

---

## How a user connects

In the app: **Settings → Server mode** →
- **Proxy URL**: the deploy URL above
- **Access token**: the token you assigned them

Save. From then on, every external call routes through your proxy — they need **no
keys of their own**. To cut someone off, remove their token from `PROXY_TOKENS` and
redeploy (`wrangler deploy`) / restart.

## Notes
- The proxy forwards `/af/*`, `/anthropic/*`, `/xai/*` to API-Football, Anthropic and
  x.ai respectively. The app builds those paths automatically.
- Health check: open the URL in a browser — it should say `powabetz proxy ok`.
