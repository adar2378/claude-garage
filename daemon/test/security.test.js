// p17-tui-only — the daemon serves no web UI, so the default Origin
// allowlist is empty. The TUI, claude's hook posts and curl send no Origin
// header and must still pass; any browser Origin (including the daemon's
// own, which the removed served UI used) is refused on state-changing
// requests.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";

delete process.env.GARAGE_UI_ORIGINS;

let app;

before(async () => {
  const { default: Fastify } = await import("fastify");
  const { rejectForeignOrigins } = await import("../src/security.js");
  app = Fastify();
  // Same hook as daemon/src/index.js.
  app.addHook("onRequest", rejectForeignOrigins);
  app.post("/api/thing", async () => ({ ok: true }));
  app.get("/api/thing", async () => ({ ok: true }));
});

after(async () => {
  await app?.close();
});

test("a POST with no Origin header (the TUI) passes", async () => {
  const res = await app.inject({ method: "POST", url: "/api/thing" });
  assert.equal(res.statusCode, 200);
});

for (const origin of [
  "http://127.0.0.1:4747",
  "http://localhost:4747",
  "http://127.0.0.1:5173",
  "https://evil.example",
]) {
  test(`a POST from ${origin} is refused`, async () => {
    const res = await app.inject({
      method: "POST",
      url: "/api/thing",
      headers: { origin },
    });
    assert.equal(res.statusCode, 403);
  });
}

test("a GET with a foreign Origin is not blocked (reads are not gated)", async () => {
  const res = await app.inject({
    method: "GET",
    url: "/api/thing",
    headers: { origin: "https://evil.example" },
  });
  assert.equal(res.statusCode, 200);
});
