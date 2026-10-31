import assert from "node:assert/strict";
import { createServer } from "node:http";
import { test } from "node:test";
import { daemonFromEnv, greeting, pushWidget } from "./plugin.js";

test("the greeting names the workspace or falls back", () => {
  assert.equal(greeting("app"), "hello, app");
  assert.equal(greeting(undefined), "hello, garage");
});

test("a missing environment is a clear error", () => {
  assert.throws(() => daemonFromEnv({}), /run this through claude-garage/);
});

test("pushWidget puts one text item on the widget with the plugin token", async () => {
  const seen: { method?: string; url?: string; auth?: string; body?: string }[] = [];
  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      seen.push({ method: req.method, url: req.url, auth: req.headers.authorization, body });
      res.end('{"ok":true}');
    });
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  const { port } = server.address() as { port: number };
  try {
    await pushWidget({ api: `http://127.0.0.1:${port}`, id: "@@ID@@", token: "tok" }, "status", "hi");
  } finally {
    server.close();
  }
  assert.equal(seen[0].method, "PUT");
  assert.equal(seen[0].url, "/api/plugins/@@ID@@/widgets/status");
  assert.equal(seen[0].auth, "Bearer tok");
  assert.equal(JSON.parse(seen[0].body!).scopes["*"][0].segments[0].text, "hi");
});
