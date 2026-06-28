// Powabetz key proxy — plain Node (no dependencies, Node 18+).
// Same behaviour as worker.js, for running on a VPS or locally behind a tunnel.
//
//   API_FOOTBALL_KEY=... ANTHROPIC_KEY=... XAI_KEY=... \
//   PROXY_TOKENS=alice-xyz,bob-abc  PORT=8787  node server.js

import http from "node:http";

const ROUTES = {
  "/af/": { base: "https://v3.football.api-sports.io/", header: "x-apisports-key", env: "API_FOOTBALL_KEY" },
  "/anthropic/": { base: "https://api.anthropic.com/", header: "x-api-key", env: "ANTHROPIC_KEY" },
  "/xai/": { base: "https://api.x.ai/", header: "authorization", env: "XAI_KEY", bearer: true },
  "/openai/": { base: "https://api.openai.com/", header: "authorization", env: "OPENAI_KEY", bearer: true },
};

const TOKENS = (process.env.PROXY_TOKENS || "").split(",").map((s) => s.trim()).filter(Boolean);
const PORT = Number(process.env.PORT || 8787);

function readBody(req) {
  return new Promise((resolve) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks)));
  });
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://localhost");
  if (url.pathname === "/" || url.pathname === "/health") {
    res.writeHead(200).end("powabetz proxy ok");
    return;
  }
  if (TOKENS.length === 0) {
    res.writeHead(500).end("proxy not configured: set PROXY_TOKENS");
    return;
  }
  if (!TOKENS.includes(req.headers["x-proxy-token"] || "")) {
    res.writeHead(401).end("unauthorized");
    return;
  }
  const entry = Object.entries(ROUTES).find(([p]) => url.pathname.startsWith(p));
  if (!entry) {
    res.writeHead(404).end("not found");
    return;
  }
  const [prefix, route] = entry;
  const key = process.env[route.env];
  if (!key) {
    res.writeHead(500).end(`proxy missing secret ${route.env}`);
    return;
  }
  const target = route.base + url.pathname.slice(prefix.length) + url.search;
  const headers = {
    [route.header]: route.bearer ? `Bearer ${key}` : key,
    "content-type": req.headers["content-type"] || "application/json",
  };
  if (prefix === "/anthropic/") headers["anthropic-version"] = req.headers["anthropic-version"] || "2023-06-01";

  const init = { method: req.method, headers };
  if (req.method !== "GET" && req.method !== "HEAD") init.body = await readBody(req);

  try {
    const r = await fetch(target, init);
    const buf = Buffer.from(await r.arrayBuffer());
    res.writeHead(r.status, { "content-type": r.headers.get("content-type") || "application/json" });
    res.end(buf);
  } catch (e) {
    res.writeHead(502).end(String(e));
  }
});

server.listen(PORT, () => console.log(`powabetz proxy on :${PORT}`));
