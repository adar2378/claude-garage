import type {
  ClosePaneResponse,
  FeaturesResponse,
  Health,
  OpenPaneResponse,
  PanesResponse,
  PluginList,
  PutWidgetResponse,
  Session,
  Usage,
  Workspace,
  WidgetPut,
  WidgetsSnapshot,
} from "./types.gen.js";

/** The daemon answered with a non-2xx status, or could not be reached. */
export class GarageError extends Error {
  /** HTTP status; `0` when the daemon could not be reached. */
  readonly status: number;
  /** The reply body: parsed JSON when it was JSON, else the text. */
  readonly body: unknown;

  constructor(status: number, message: string, body: unknown) {
    super(message);
    this.name = "GarageError";
    this.status = status;
    this.body = body;
  }
}

export interface ApiOptions {
  /** Daemon base URL, for example `http://127.0.0.1:4747` (`GARAGE_API`). */
  base: string;
  /** Your plugin id (`GARAGE_PLUGIN_ID`). */
  plugin: string;
  /** Your plugin token (`GARAGE_PLUGIN_TOKEN`). */
  token: string;
  /** A `fetch` to use instead of the global one (tests). */
  fetch?: typeof fetch;
  /** Per-request timeout in milliseconds. Default 10000. */
  timeoutMs?: number;
}

/** What a UI action applies to. */
export type Target = { session: string } | { workspace: string };

/** The kind of widget text a push may carry. See the widgets doc. */
export type { WidgetPut };

/**
 * A client for the daemon's plugin API. Every call sends your plugin token.
 * A non-2xx reply rejects with {@link GarageError}.
 */
export class Api {
  readonly #base: string;
  readonly #plugin: string;
  readonly #token: string;
  readonly #fetch: typeof fetch;
  readonly #timeoutMs: number;

  constructor(options: ApiOptions) {
    this.#base = options.base.replace(/\/+$/, "");
    this.#plugin = options.plugin;
    this.#token = options.token;
    this.#fetch = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.#timeoutMs = options.timeoutMs ?? 10_000;
  }

  /** Build a client from the environment garage gives every plugin run. */
  static fromEnv(env: Record<string, string | undefined> = process.env): Api {
    const need = (name: string): string => {
      const v = env[name];
      if (!v) {
        throw new GarageError(
          0,
          `${name} is not set; run this through claude-garage`,
          null,
        );
      }
      return v;
    };
    return new Api({
      base: need("GARAGE_API"),
      plugin: need("GARAGE_PLUGIN_ID"),
      token: need("GARAGE_PLUGIN_TOKEN"),
    });
  }

  /** Your plugin id. */
  get plugin(): string {
    return this.#plugin;
  }

  /**
   * One call with the plugin token. The result is the parsed JSON body, or
   * `undefined` for an empty one. Use it for routes this class has no method for.
   */
  async request<T = unknown>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const headers: Record<string, string> = {
      authorization: `Bearer ${this.#token}`,
    };
    const init: RequestInit = {
      method,
      headers,
      signal: AbortSignal.timeout(this.#timeoutMs),
    };
    if (body !== undefined) {
      headers["content-type"] = "application/json";
      init.body = JSON.stringify(body);
    }
    let res: Response;
    try {
      res = await this.#fetch(`${this.#base}${path}`, init);
    } catch (e) {
      throw new GarageError(
        0,
        `cannot reach the daemon: ${e instanceof Error ? e.message : String(e)}`,
        null,
      );
    }
    const text = await res.text();
    let parsed: unknown = undefined;
    if (text !== "") {
      try {
        parsed = JSON.parse(text);
      } catch {
        parsed = text;
      }
    }
    if (!res.ok) {
      const detail =
        parsed !== null &&
        typeof parsed === "object" &&
        "error" in parsed &&
        typeof parsed.error === "string"
          ? parsed.error
          : text;
      throw new GarageError(
        res.status,
        `daemon answered ${res.status}: ${detail}`,
        parsed,
      );
    }
    return parsed as T;
  }

  #own(path: string): string {
    return `/api/plugins/${encodeURIComponent(this.#plugin)}${path}`;
  }

  // ---- reading ----

  /** `GET /api/plugin/features`: API versions and optional capabilities. */
  features(): Promise<FeaturesResponse> {
    return this.request("GET", "/api/plugin/features");
  }

  /** `GET /api/health`. */
  health(): Promise<Health> {
    return this.request("GET", "/api/health");
  }

  /** `GET /api/sessions`: live sessions, and restorable ones. */
  sessions(): Promise<Session[]> {
    return this.request("GET", "/api/sessions");
  }

  /** `GET /api/workspaces`. */
  workspaces(): Promise<Workspace[]> {
    return this.request("GET", "/api/workspaces");
  }

  /** `GET /api/usage`: the 5 hour and weekly rate-limit windows. */
  usage(): Promise<Usage> {
    return this.request("GET", "/api/usage");
  }

  /**
   * `GET /api/workspaces/{name}/closed`: the closed chats kept for a
   * workspace. Needs `sessions.read`.
   */
  closedChats(workspace: string): Promise<unknown[]> {
    return this.request(
      "GET",
      `/api/workspaces/${encodeURIComponent(workspace)}/closed`,
    );
  }

  /** `GET /api/plugins`: installed plugins with their actions and panes. */
  plugins(): Promise<PluginList> {
    return this.request("GET", "/api/plugins");
  }

  /** `GET /api/plugins/widgets`: what every plugin currently shows. */
  widgets(): Promise<WidgetsSnapshot> {
    return this.request("GET", "/api/plugins/widgets");
  }

  /** `GET /api/plugins/panes`: the open plugin panes. */
  panes(): Promise<PanesResponse> {
    return this.request("GET", "/api/plugins/panes");
  }

  // ---- acting ----

  /**
   * `PUT /api/plugins/{me}/widgets/{widget}`: replace what a widget shows.
   * The widget must be declared in the manifest. Needs `ui.widgets`.
   * Resolves to the new `rev`.
   */
  async putWidget(widget: string, put: WidgetPut): Promise<number> {
    const res = await this.request<PutWidgetResponse>(
      "PUT",
      this.#own(`/widgets/${encodeURIComponent(widget)}`),
      put,
    );
    return res.rev;
  }

  /** `DELETE /api/plugins/{me}/widgets/{widget}`. Needs `ui.widgets`. */
  async clearWidget(widget: string): Promise<void> {
    await this.request(
      "DELETE",
      this.#own(`/widgets/${encodeURIComponent(widget)}`),
    );
  }

  /**
   * `POST /api/plugins/{me}/notice`: a short toast for the user. Needs
   * `ui.notice`. One a second; bursts of three.
   */
  async notice(text: string, ttlMs?: number): Promise<void> {
    await this.request("POST", this.#own("/notice"), {
      text,
      ...(ttlMs === undefined ? {} : { ttl_ms: ttlMs }),
    });
  }

  /**
   * `POST /api/plugins/ui/action`: ask the wall to run `action` (a core id,
   * `plugin:action`, or one of your own action ids) on `target`. Needs
   * `ui.invoke`, plus `ui.navigate` or `core.actions` for a core id. The
   * user confirms dangerous actions at the wall; you cannot.
   */
  async uiAction(
    action: string,
    target: Target,
    args?: unknown,
  ): Promise<void> {
    await this.request("POST", "/api/plugins/ui/action", {
      action,
      target,
      ...(args === undefined ? {} : { args }),
    });
  }

  /**
   * `POST /api/sessions/input`: type `text` into a session and press Enter.
   * Needs `sessions.input` (dangerous). Pass `ifStatusNot: ["needs-input"]`
   * so a prompt waiting on the user is never answered by accident: the
   * daemon then rejects with status 409 and types nothing.
   */
  async sessionInput(
    id: string,
    text: string,
    options: { ifStatusNot?: string[] } = {},
  ): Promise<void> {
    await this.request("POST", "/api/sessions/input", {
      id,
      text,
      ...(options.ifStatusNot === undefined
        ? {}
        : { if_status: { not: options.ifStatusNot } }),
    });
  }

  /** `POST /api/plugins/{me}/panes/{pane}/open`. Needs `ui.panes`. */
  openPane(pane: string, args?: unknown): Promise<OpenPaneResponse> {
    return this.request(
      "POST",
      this.#own(`/panes/${encodeURIComponent(pane)}/open`),
      args === undefined ? {} : args,
    );
  }

  /** `DELETE /api/plugins/{me}/panes/{pane}`. Needs `ui.panes`. */
  closePane(pane: string): Promise<ClosePaneResponse> {
    return this.request(
      "DELETE",
      this.#own(`/panes/${encodeURIComponent(pane)}`),
    );
  }
}

/** A widget with one plain-text item for every scope (`*`). */
export function textWidget(text: string): WidgetPut {
  return { scopes: { "*": [{ segments: [{ text }] }] } };
}
