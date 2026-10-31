/** The text the widget shows; `who` is the focused workspace, if any. */
export function greeting(who: string | undefined): string {
  return `hello, ${who ?? "garage"}`;
}

/** The body of `PUT /api/plugins/<id>/widgets/<widget>`: one text item for every scope. */
export function widgetBody(text: string) {
  return { scopes: { "*": [{ segments: [{ text }] }] } };
}

export interface Daemon {
  api: string;
  id: string;
  token: string;
}

/** Read the daemon address and this plugin's token from the environment. */
export function daemonFromEnv(env: NodeJS.ProcessEnv): Daemon {
  const { GARAGE_API: api, GARAGE_PLUGIN_ID: id, GARAGE_PLUGIN_TOKEN: token } = env;
  if (!api || !id || !token) {
    throw new Error("GARAGE_API, GARAGE_PLUGIN_ID and GARAGE_PLUGIN_TOKEN are not set; run this through claude-garage");
  }
  return { api: api.replace(/\/+$/, ""), id, token };
}

export async function pushWidget(d: Daemon, widget: string, text: string): Promise<void> {
  const res = await fetch(`${d.api}/api/plugins/${d.id}/widgets/${widget}`, {
    method: "PUT",
    headers: { authorization: `Bearer ${d.token}`, "content-type": "application/json" },
    body: JSON.stringify(widgetBody(text)),
  });
  if (!res.ok) {
    throw new Error(`daemon answered ${res.status}: ${await res.text()}`);
  }
}
