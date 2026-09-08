// Germal Cat dashboard — Bun static server + proxy to the Rust daemon.
// The daemon holds the decryption key; this process never sees the password
// beyond forwarding the unlock request.

const DAEMON = process.env.GERMALCAT_DAEMON ?? "http://127.0.0.1:8420";
const PORT = Number(process.env.PORT ?? 6969);

const html = await Bun.file(new URL("./index.html", import.meta.url)).text();

Bun.serve({
  port: PORT,
  async fetch(req) {
    const url = new URL(req.url);

    if (url.pathname.startsWith("/api/")) {
      const res = await fetch(DAEMON + url.pathname + url.search, {
        method: req.method,
        headers: { "content-type": "application/json" },
        body: req.method === "GET" || req.method === "HEAD" ? undefined : await req.text(),
      }).catch(() => null);
      if (!res) return Response.json({ error: "daemon unreachable" }, { status: 502 });
      return new Response(res.body, { status: res.status, headers: { "content-type": res.headers.get("content-type") ?? "application/json" } });
    }

    return new Response(html, { headers: { "content-type": "text/html" } });
  },
});

console.log(`Germal Cat dashboard → http://localhost:${PORT}  (daemon: ${DAEMON})`);
