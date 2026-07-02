// Powabetz key proxy — Cloudflare Worker.
//
// The desktop app calls THIS worker instead of the providers. The worker holds
// your real API keys as server-side secrets and injects them, so users only ever
// get a per-user access token — never your keys.
//
// Routes (the app builds these automatically in Server mode):
//   /af/<path>          -> https://v3.football.api-sports.io/<path>   (x-apisports-key)
//   /anthropic/<path>   -> https://api.anthropic.com/<path>           (x-api-key)
//   /xai/<path>         -> https://api.x.ai/<path>                    (Authorization: Bearer)
//   /deepseek/<path>    -> https://api.deepseek.com/<path>            (Authorization: Bearer)
//     (the app calls /deepseek/anthropic/v1/messages — DeepSeek's
//      Anthropic-compatible endpoint)
//
// Secrets (set with `wrangler secret put NAME`):
//   API_FOOTBALL_KEY, ANTHROPIC_KEY, XAI_KEY, OPENAI_KEY, DEEPSEEK_KEY
//   PROXY_TOKENS  -> comma-separated list of allowed user tokens, e.g. "alice-xyz,bob-abc"

const ROUTES = {
  "/af/": { base: "https://v3.football.api-sports.io/", header: "x-apisports-key", env: "API_FOOTBALL_KEY" },
  "/anthropic/": { base: "https://api.anthropic.com/", header: "x-api-key", env: "ANTHROPIC_KEY" },
  "/xai/": { base: "https://api.x.ai/", header: "Authorization", env: "XAI_KEY", bearer: true },
  "/openai/": { base: "https://api.openai.com/", header: "Authorization", env: "OPENAI_KEY", bearer: true },
  // DeepSeek's Anthropic-compatible endpoint authenticates like Anthropic
  // (x-api-key), not Bearer — the app only calls /deepseek/anthropic/… here.
  "/deepseek/": { base: "https://api.deepseek.com/", header: "x-api-key", env: "DEEPSEEK_KEY" },
};

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // Health check.
    if (url.pathname === "/" || url.pathname === "/health") {
      return new Response("powabetz proxy ok", { status: 200 });
    }

    // Auth: the app sends its per-user token in x-proxy-token.
    const token = request.headers.get("x-proxy-token") || "";
    const allowed = (env.PROXY_TOKENS || "").split(",").map((s) => s.trim()).filter(Boolean);
    if (allowed.length === 0) {
      return new Response("proxy not configured: set PROXY_TOKENS", { status: 500 });
    }
    if (!allowed.includes(token)) {
      return new Response("unauthorized", { status: 401 });
    }

    // Route to the right provider.
    const match = Object.entries(ROUTES).find(([prefix]) => url.pathname.startsWith(prefix));
    if (!match) return new Response("not found", { status: 404 });
    const [prefix, route] = match;

    const key = env[route.env];
    if (!key) return new Response(`proxy missing secret ${route.env}`, { status: 500 });

    const target = route.base + url.pathname.slice(prefix.length) + url.search;
    const headers = new Headers();
    headers.set(route.header, route.bearer ? `Bearer ${key}` : key);
    headers.set("content-type", request.headers.get("content-type") || "application/json");
    if (prefix === "/anthropic/") {
      headers.set("anthropic-version", request.headers.get("anthropic-version") || "2023-06-01");
    }

    const init = { method: request.method, headers };
    if (request.method !== "GET" && request.method !== "HEAD") {
      init.body = await request.arrayBuffer();
    }

    const resp = await fetch(target, init);
    const out = new Headers();
    out.set("content-type", resp.headers.get("content-type") || "application/json");
    return new Response(resp.body, { status: resp.status, headers: out });
  },
};
