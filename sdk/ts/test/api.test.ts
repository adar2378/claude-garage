import assert from "node:assert/strict";
import { afterEach, describe, it } from "node:test";

import { Api, GarageError, textWidget } from "../src/index.js";
import type { WidgetPut } from "../src/index.js";
import { FakeDaemon, type Reply, type Seen } from "./fake-daemon.js";

let daemon: FakeDaemon | undefined;

async function start(reply: (req: Seen) => Reply): Promise<Api> {
  daemon = await new FakeDaemon(reply).start();
  return new Api({ base: daemon.base, plugin: "usage-bar", token: "tok123" });
}

afterEach(async () => {
  await daemon?.stop();
  daemon = undefined;
});

const ok = (): Reply => [200, '{"ok":true}'];

describe("Api requests", () => {
  it("sends the plugin token as a bearer and JSON bodies as JSON", async () => {
    const api = await start(() => [200, '{"ok":true,"rev":7}']);
    const rev = await api.putWidget("bar", textWidget("hi"));
    assert.equal(rev, 7);
    const seen = daemon!.seen[0]!;
    assert.equal(seen.method, "PUT");
    assert.equal(seen.path, "/api/plugins/usage-bar/widgets/bar");
    assert.equal(seen.authorization, "Bearer tok123");
    assert.equal(seen.contentType, "application/json");
    assert.deepEqual(seen.body, {
      scopes: { "*": [{ segments: [{ text: "hi" }] }] },
    });
  });

  it("sends no content type and no body on a GET", async () => {
    const api = await start(() => [200, "[]"]);
    assert.deepEqual(await api.sessions(), []);
    const seen = daemon!.seen[0]!;
    assert.equal(seen.method, "GET");
    assert.equal(seen.path, "/api/sessions");
    assert.equal(seen.contentType, undefined);
    assert.equal(seen.body, undefined);
  });

  it("encodes ids in paths", async () => {
    const api = new Api({
      base: (daemon = await new FakeDaemon(ok).start()).base,
      plugin: "a b",
      token: "t",
    });
    await api.clearWidget("w/x");
    await api.closedChats("my ws");
    assert.equal(daemon.seen[0]!.path, "/api/plugins/a%20b/widgets/w%2Fx");
    assert.equal(daemon.seen[1]!.path, "/api/workspaces/my%20ws/closed");
  });

  it("tolerates a trailing slash on the base URL and an empty body", async () => {
    daemon = await new FakeDaemon(() => [200, ""]).start();
    const api = new Api({ base: `${daemon.base}/`, plugin: "p", token: "t" });
    assert.equal(await api.request("DELETE", "/api/x"), undefined);
    assert.equal(daemon.seen[0]!.path, "/api/x");
  });
});

describe("Api errors", () => {
  it("turns a non-2xx reply into a GarageError with status and body", async () => {
    const api = await start(() => [
      403,
      '{"error":"plugin usage-bar lacks permission ui.widgets"}',
    ]);
    await assert.rejects(
      () => api.putWidget("bar", textWidget("x")),
      (e: unknown) => {
        assert.ok(e instanceof GarageError);
        assert.equal(e.status, 403);
        assert.match(e.message, /lacks permission ui\.widgets/);
        assert.deepEqual(e.body, {
          error: "plugin usage-bar lacks permission ui.widgets",
        });
        return true;
      },
    );
  });

  it("keeps a non-JSON error body as text", async () => {
    const api = await start(() => [502, "bad gateway"]);
    await assert.rejects(
      () => api.health(),
      (e: unknown) =>
        e instanceof GarageError && e.status === 502 && e.body === "bad gateway",
    );
  });

  it("reports an unreachable daemon as status 0", async () => {
    const api = new Api({
      base: "http://127.0.0.1:1",
      plugin: "p",
      token: "t",
      timeoutMs: 2000,
    });
    await assert.rejects(
      () => api.health(),
      (e: unknown) =>
        e instanceof GarageError &&
        e.status === 0 &&
        /cannot reach the daemon/.test(e.message),
    );
  });
});

describe("Api calls", () => {
  it("notice sends ttl_ms only when given", async () => {
    const api = await start(ok);
    await api.notice("done");
    await api.notice("done", 3000);
    assert.deepEqual(daemon!.seen[0]!.body, { text: "done" });
    assert.deepEqual(daemon!.seen[1]!.body, { text: "done", ttl_ms: 3000 });
    assert.equal(daemon!.seen[0]!.path, "/api/plugins/usage-bar/notice");
  });

  it("uiAction names the target, and args only when given", async () => {
    const api = await start(() => [202, '{"ok":true}']);
    await api.uiAction("session.focus", { session: "garage/ws/a" });
    await api.uiAction("usage-bar:details", { workspace: "ws" }, { n: 1 });
    assert.equal(daemon!.seen[0]!.path, "/api/plugins/ui/action");
    assert.deepEqual(daemon!.seen[0]!.body, {
      action: "session.focus",
      target: { session: "garage/ws/a" },
    });
    assert.deepEqual(daemon!.seen[1]!.body, {
      action: "usage-bar:details",
      target: { workspace: "ws" },
      args: { n: 1 },
    });
  });

  it("sessionInput sends if_status and surfaces the 409 refusal", async () => {
    const api = await start((req) =>
      (req.body as { if_status?: unknown }).if_status
        ? [409, '{"error":"garage/ws/a is needs-input; nothing was typed","status":"needs-input"}']
        : ok(),
    );
    await api.sessionInput("garage/ws/a", "continue");
    assert.deepEqual(daemon!.seen[0]!.body, { id: "garage/ws/a", text: "continue" });
    await assert.rejects(
      () => api.sessionInput("garage/ws/a", "yes", { ifStatusNot: ["needs-input"] }),
      (e: unknown) => e instanceof GarageError && e.status === 409,
    );
    assert.deepEqual(daemon!.seen[1]!.body, {
      id: "garage/ws/a",
      text: "yes",
      if_status: { not: ["needs-input"] },
    });
  });

  it("opens and closes a pane", async () => {
    const api = await start((req) =>
      req.method === "POST"
        ? [200, '{"ok":true,"session":"plugin/usage-bar/details","reused":false}']
        : [200, '{"ok":true,"closed":true}'],
    );
    const opened = await api.openPane("details");
    assert.equal(opened.session, "plugin/usage-bar/details");
    assert.equal(opened.reused, false);
    assert.equal((await api.closePane("details")).closed, true);
    assert.equal(daemon!.seen[0]!.path, "/api/plugins/usage-bar/panes/details/open");
    assert.deepEqual(daemon!.seen[0]!.body, {});
    assert.equal(daemon!.seen[1]!.method, "DELETE");
    assert.equal(daemon!.seen[1]!.path, "/api/plugins/usage-bar/panes/details");
  });

  it("reads features with the documented shape", async () => {
    const api = await start(() => [
      200,
      JSON.stringify({ api_versions: [1], garage: "0.7.0", features: { "api.v1.frozen": true } }),
    ]);
    const f = await api.features();
    assert.deepEqual(f.api_versions, [1]);
    assert.equal(f.features["api.v1.frozen"], true);
    assert.equal(daemon!.seen[0]!.path, "/api/plugins/features");
  });
});

describe("tokenless routes", () => {
  it("sends the plugin token on every read, even on routes that answer without one today", async () => {
    // /api/events, /api/sessions, /api/workspaces, /api/usage, /api/plugins/widgets
    // and /api/workspaces/{name}/closed accept a token now; tokenless access is
    // deprecated and may be removed, so the SDK never goes without.
    const api = await start(() => [200, "{}"]);
    await api.health();
    await api.features();
    await api.sessions();
    await api.workspaces();
    await api.usage();
    await api.closedChats("repo");
    await api.plugins();
    await api.widgets();
    await api.panes();
    await api.request("GET", "/api/events");
    const seen = daemon!.seen;
    assert.equal(seen.length, 10);
    for (const s of seen) {
      assert.equal(s.authorization, "Bearer tok123", `${s.method} ${s.path}`);
    }
    for (const path of [
      "/api/sessions",
      "/api/workspaces",
      "/api/usage",
      "/api/workspaces/repo/closed",
      "/api/plugins/widgets",
      "/api/events",
    ]) {
      assert.ok(seen.some((s) => s.path === path), path);
    }
  });
});

describe("errors", () => {
  it("exposes the stable code and the missing permission of an error body", async () => {
    const api = await start(() => [
      403,
      JSON.stringify({
        error: "plugin usage-bar lacks permission ui.widgets",
        code: "permission_denied",
        permission: "ui.widgets",
      }),
    ]);
    await assert.rejects(api.clearWidget("bar"), (e: unknown) => {
      assert.ok(e instanceof GarageError);
      assert.equal(e.status, 403);
      assert.equal(e.code, "permission_denied");
      assert.equal(e.permission, "ui.widgets");
      assert.match(e.message, /lacks permission/);
      return true;
    });
  });

  it("leaves code undefined for a body that is not an error object", async () => {
    const api = await start(() => [502, "bad gateway"]);
    await assert.rejects(api.sessions(), (e: unknown) => {
      assert.ok(e instanceof GarageError);
      assert.equal(e.code, undefined);
      assert.equal(e.permission, undefined);
      return true;
    });
  });
});

describe("Api.fromEnv", () => {
  it("reads the three variables garage sets", () => {
    const api = Api.fromEnv({
      GARAGE_API: "http://127.0.0.1:4747",
      GARAGE_PLUGIN_ID: "demo",
      GARAGE_PLUGIN_TOKEN: "t",
    });
    assert.equal(api.plugin, "demo");
  });

  it("names the missing variable", () => {
    assert.throws(
      () => Api.fromEnv({ GARAGE_API: "x", GARAGE_PLUGIN_ID: "demo" }),
      /GARAGE_PLUGIN_TOKEN is not set/,
    );
  });
});

describe("types", () => {
  it("accept the widget forms the docs show (compile-time check)", () => {
    const animated: WidgetPut = {
      scopes: {
        "*": [
          {
            priority: 5,
            placement: { kind: "filler", x: 20, caption: "hi" },
            frames: [[{ text: "a" }], [{ text: "b", style: { fg: "needs_input", bold: true } }]],
            frame_ms: 300,
            frame_x: [20, 21],
          },
        ],
      },
      ttl_secs: 15,
    };
    const gutter: WidgetPut = {
      scopes: { "garage/ws/a": [{ row: 120, kind: "turn", click: { action: "session.focus" } }] },
    };
    assert.equal(animated.ttl_secs, 15);
    assert.equal(gutter.scopes["garage/ws/a"]?.[0]?.row, 120);
  });
});
