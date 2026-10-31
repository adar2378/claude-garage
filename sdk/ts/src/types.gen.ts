/* Generated from schemas/garage-plugin-api.schema.json by scripts/gen-types.mjs.
 * Do not edit. Run `npm run gen` after the schema changes. */

/**
 * Event names a manifest `[[events]] on` may use.
 */
export type EventName =
  | "session.created"
  | "session.closed"
  | "session.turn"
  | "session.forgotten"
  | "session.status_changed"
  | "worktree.created"
  | "workspace.added"
  | "workspace.removed"
  | "daemon.started"
  | "timer";
/**
 * Session state. `Restorable` only appears in list entries built from
 * resume metadata; the status store itself never holds it.
 */
export type SessionStatus = "idle" | "working" | "needs-input" | "done" | "restorable";
/**
 * How a client attaches a terminal to a session: the seam between the TUI's
 * How the TUI attaches to a session. Deliberately data, not a command line:
 * the TUI builds the fixed command itself, so whatever answers on the daemon
 * port can never make it run an arbitrary program or dial an arbitrary host.
 */
export type AttachSpec =
  | {
      kind: "tmux";
      socket?: string | null;
    }
  | {
      id: string;
      kind: "host";
    };
/**
 * Where a widget draws. The scope key of its content depends on the slot.
 */
export type WidgetSlot =
  "tile.title.right" | "tile.gutter" | "rail.row" | "rail.extra_rows" | "rail.header" | "strip.right";
export type ContextSource = "statusline" | "transcript";
export type Placement =
  | {
      kind: "flow";
    }
  | {
      caption?: string | null;
      kind: "filler";
      x: number;
    };

/**
 * Root of the published schema: one field per public type, so every type is
 * a named definition (the SDK generators turn definitions into types).
 */
export interface ApiTypes {
  ack: Ack;
  close_pane_response: ClosePaneResponse;
  error: ErrorBody;
  event_name: EventName;
  event_payloads: EventPayloads;
  features_response: FeaturesResponse;
  health: Health;
  notice_request: NoticeRequest;
  open_pane_response: OpenPaneResponse;
  panes_response: PanesResponse;
  plugin_list: PluginList;
  put_widget_response: PutWidgetResponse;
  run_context: RunContext;
  session_input_request: SessionInputRequest;
  sessions: Session[];
  ui_action_frame: UiActionFrame;
  ui_action_request: UiActionRequest;
  usage: Usage;
  widget_item: WidgetItem;
  widget_put: WidgetPut;
  widgets_snapshot: WidgetsSnapshot;
  workspaces: Workspace[];
}
/**
 * `{"ok": true}` acknowledgements.
 */
export interface Ack {
  ok: boolean;
}
/**
 * Reply of `DELETE /api/plugins/{id}/panes/{pane}`.
 */
export interface ClosePaneResponse {
  closed: boolean;
  ok: boolean;
}
/**
 * Body of every non-2xx response: `{"error": "..."}`. Routes may add fields.
 */
export interface ErrorBody {
  error: string;
}
/**
 * The payload (`event.data`) of each event, by event name. `daemon.started`
 * and `timer` carry `{}`.
 */
export interface EventPayloads {
  "session.closed": SessionClosed;
  "session.created": SessionCreated;
  "session.forgotten": SessionForgotten;
  "session.status_changed": SessionStatusChanged;
  "session.turn": SessionTurn;
  "workspace.added": WorkspaceNamed;
  "workspace.removed": WorkspaceNamed;
  "worktree.created": WorktreeCreated;
}
export interface SessionClosed {
  dir: string;
  id: string;
  last_status: string | null;
  /**
   * `user`, `crash` or `daemon_restart`.
   */
  reason: string;
  workspace: string;
}
export interface SessionCreated {
  dir: string;
  id: string;
  workspace: string;
}
export interface SessionForgotten {
  id: string;
  workspace: string;
}
export interface SessionStatusChanged {
  from: SessionStatus;
  id: string;
  since: number | null;
  to: SessionStatus;
}
export interface SessionTurn {
  /**
   * Unix milliseconds.
   */
  at: number;
  history_limit: number | null;
  id: string;
  /**
   * Absolute scrollback row where the turn ended (0 is the oldest kept line).
   */
  row: number | null;
  transcript_offset: number | null;
  workspace: string;
}
export interface WorkspaceNamed {
  name: string;
}
export interface WorktreeCreated {
  branch: string;
  path: string;
  repo_dir: string;
  session: string;
  workspace: string;
}
/**
 * `GET /api/plugin/features`.
 */
export interface FeaturesResponse {
  api_versions: number[];
  features: Features;
  /**
   * The garage version.
   */
  garage: string;
}
/**
 * The `features` object. New keys may appear in later garage versions.
 */
export interface Features {
  "actions.core_invocable": string[];
  "api.v1.frozen": boolean;
  "cli.fallback": boolean;
  dangerous_permissions: string[];
  events: string[];
  limits: FeatureLimits;
  "panes.placements": string[];
  permissions: string[];
  routes: RouteInfo[];
  "sessions.input": boolean;
  slots: string[];
  theme_tokens: string[];
  "ui.notice": boolean;
  "widgets.animated": boolean;
  "widgets.click_args": boolean;
  "widgets.filler": boolean;
  "widgets.frame_x": boolean;
  "widgets.slot_geometry": boolean;
}
export interface FeatureLimits {
  notice_chars: number;
  widget_body_bytes: number;
  widget_cols: number;
  widget_frames: number;
  widget_items: number;
  widget_puts_per_sec: number;
  widget_segments: number;
  widget_ttl_secs: number;
}
/**
 * One entry of `features.routes`.
 */
export interface RouteInfo {
  method: string;
  /**
   * `none` or a permission name.
   */
  needs: string;
  path: string;
}
/**
 * `GET /api/health`.
 */
export interface Health {
  pid: number;
  status: string;
  version?: string | null;
}
/**
 * `POST /api/plugins/{id}/notice` (needs `ui.notice`).
 */
export interface NoticeRequest {
  /**
   * Cut to 120 characters.
   */
  text: string;
  /**
   * 1000..=10000, default 5000.
   */
  ttl_ms?: number | null;
}
/**
 * Reply of `POST /api/plugins/{id}/panes/{pane}/open`.
 */
export interface OpenPaneResponse {
  ok: boolean;
  /**
   * The pane was already open (a singleton) and was reused.
   */
  reused: boolean;
  /**
   * The backend session id of the pane.
   */
  session: string;
}
/**
 * `GET /api/plugins/panes`.
 */
export interface PanesResponse {
  panes: PaneInstance[];
}
/**
 * One open plugin pane, with how to attach to it.
 */
export interface PaneInstance {
  attach: AttachSpec;
  cols?: number | null;
  pane: string;
  pct?: boolean;
  placement: string;
  plugin: string;
  rows?: number | null;
  /**
   * The backend session id (`plugin/<plugin>/<pane>`); not a `garage/` id,
   * so it is never in the session list.
   */
  session: string;
  title: string;
}
/**
 * `GET /api/plugins`: the plugins in install order (which is also the
 * tiebreak for two plugins that suggest the same key).
 */
export interface PluginList {
  plugins: PluginSummary[];
}
export interface PluginSummary {
  actions?: ActionSummary[];
  /**
   * Enabled, approved, loaded: only an active plugin's actions, buttons
   * and widgets reach the screen.
   */
  active: boolean;
  /**
   * The user's own key bindings (`action` local id to key).
   */
  bindings?: {
    [k: string]: string;
  };
  buttons?: ButtonSummary[];
  enabled: boolean;
  id: string;
  name: string;
  panes?: PaneSummary[];
  permissions?: string[];
  version?: string | null;
  widgets?: WidgetSummary[];
}
export interface ActionSummary {
  confirm?: boolean;
  /**
   * `global`, `workspace` or `session`: what must be focused.
   */
  contexts?: string[];
  default_key?: string | null;
  /**
   * The full id, `plugin:action`.
   */
  id: string;
  /**
   * The local pane id when the action opens a pane instead of running a command.
   */
  pane?: string | null;
  title: string;
}
export interface ButtonSummary {
  args?: {
    [k: string]: unknown;
  };
  confirm?: boolean;
  label: string;
  /**
   * A core id, `plugin:action`, or a bare local id.
   */
  on_click: string;
  priority?: number;
  slot: WidgetSlot;
  tooltip?: string | null;
}
export interface PaneSummary {
  cols?: number | null;
  id: string;
  /**
   * `cols`/`rows` are percent of the screen.
   */
  pct?: boolean;
  /**
   * `tile`, `overlay` or `popup`.
   */
  placement: string;
  rows?: number | null;
  title: string;
}
export interface WidgetSummary {
  id: string;
  slot: WidgetSlot;
}
/**
 * Reply of `PUT /api/plugins/{id}/widgets/{widget}`.
 */
export interface PutWidgetResponse {
  ok: boolean;
  /**
   * Grows on every change of the stored content.
   */
  rev: number;
}
/**
 * The JSON every action, event, pane and CLI run receives: in
 * `GARAGE_CONTEXT` (or the file `GARAGE_CONTEXT_FILE` names) and on stdin.
 * Keys are never absent; a missing value is `null`.
 */
export interface RunContext {
  api_version: number;
  /**
   * Set for event hook runs.
   */
  event: ContextEvent | null;
  focus: ContextFocus;
  plugin: ContextPlugin;
  selection: ContextSelection;
  trigger: ContextTrigger;
  ui: ContextUi;
  /**
   * The focused session's worktree, if it has one.
   */
  worktree: ContextWorktree | null;
}
export interface ContextEvent {
  /**
   * Unix milliseconds.
   */
  at: number;
  /**
   * The payload; its shape depends on `name` (see [`EventPayloads`]).
   */
  data: {
    [k: string]: unknown;
  };
  /**
   * One of the event names (see [`EventName`]).
   */
  name: string;
}
export interface ContextFocus {
  /**
   * CLI runs only: the caller's working directory.
   */
  dir?: string | null;
  session: ContextSession | null;
  workspace: string | null;
}
export interface ContextSession {
  branch: string | null;
  dir: string | null;
  id: string;
  status: string | null;
  worktree: ContextWorktree | null;
}
export interface ContextWorktree {
  branch: string;
  path: string;
  repo_dir: string;
}
export interface ContextPlugin {
  id: string;
  /**
   * The plugin's own writable folder.
   */
  state_dir: string;
  version: string;
}
export interface ContextSelection {
  index: number | null;
  session: string | null;
  view: string | null;
}
export interface ContextTrigger {
  action: string | null;
  /**
   * The `args` of the button or click; `{}` when none. For a `cli` run,
   * `{"argv": [...]}`.
   */
  args: {
    [k: string]: unknown;
  };
  key: string | null;
  /**
   * `key`, `button`, `widget`, `palette`, `event`, `timer`, `pane` or `cli`.
   */
  kind: string;
  slot: string | null;
}
export interface ContextUi {
  /**
   * Slot geometry the wall reported (`width`, `max_cols`, `filler`), by
   * slot name. May be empty.
   */
  slots: {
    [k: string]: unknown;
  };
}
/**
 * `POST /api/sessions/input` (needs `sessions.input`, dangerous).
 */
export interface SessionInputRequest {
  /**
   * A `garage/<workspace>/<label>` session id.
   */
  id: string;
  if_status?: IfStatus | null;
  /**
   * Typed into the session, then Enter.
   */
  text: string;
}
/**
 * Refuse the input (409) when the session is in one of these statuses.
 */
export interface IfStatus {
  not: string[];
}
/**
 * One entry of `GET /api/sessions`: live, or restorable (`status` is
 * `restorable`, `restorable` is `true`, no `createdAt`).
 */
export interface Session {
  attached: boolean;
  branch?: string | null;
  context?: Context | null;
  createdAt?: number | null;
  dir?: string | null;
  id: string;
  label: string;
  message?: string | null;
  restorable?: boolean | null;
  since?: number | null;
  status: SessionStatus;
  title?: string | null;
  workspace: string;
}
/**
 * `context` of a session list entry. The percentage is clamped to 0-100 by
 * the daemon but may be fractional (statusline) or rounded (transcript).
 */
export interface Context {
  source: ContextSource;
  usedPercentage: number;
}
/**
 * The SSE `ui_action` frame: a plugin asked the TUI to run an action. There
 * is deliberately no `confirmed` field: whether a dangerous action runs is
 * the human's decision at the TUI, never the plugin's.
 */
export interface UiActionFrame {
  action: string;
  args?: {
    [k: string]: unknown;
  };
  plugin: string;
  target?: UiTarget;
}
/**
 * Where a `ui_action` request applies. Core actions take their session and
 * workspace from here and nowhere else.
 */
export interface UiTarget {
  session?: string | null;
  workspace?: string | null;
}
/**
 * `POST /api/plugins/ui/action` (needs `ui.invoke`).
 */
export interface UiActionRequest {
  /**
   * A core id, `plugin:action`, or a bare local action id.
   */
  action: string;
  args?: unknown;
  target?: UiTarget;
}
export interface Usage {
  fiveHour?: UsageWindow | null;
  sevenDay?: UsageWindow | null;
}
/**
 * One rate-limit window of `GET /api/usage`. `resetsAt` is whatever scalar
 * Claude Code's statusline carried (an ISO string in practice), or null.
 */
export interface UsageWindow {
  resetsAt: unknown;
  usedPercentage: number;
}
/**
 * One thing a widget shows. `segments` and `frames` are mutually exclusive;
 * the gutter fields (`row` / `kind`) apply to `tile.gutter`, and
 * `row_style` / `click` to `rail.extra_rows` rows (and gutter markers).
 */
export interface WidgetItem {
  click?: Click | null;
  frame_ms?: number | null;
  /**
   * `strip.right` filler items only: the filler column of each frame, in
   * step with `frames` (same length), overriding `placement.x`. A walking
   * pet changes column from frame to frame; a plain `x` cannot say that.
   */
  frame_x?: number[] | null;
  frames?: Segment[][] | null;
  kind?: string | null;
  placement?: Placement;
  priority?: number;
  row?: number | null;
  row_style?: Style | null;
  segments?: Segment[] | null;
  /**
   * `rail.extra_rows` only: the session this row is about (a closed chat),
   * which scopes its click so a core action such as `session.restore`
   * takes it as its target. Must be a session of the row's workspace.
   */
  session?: string | null;
}
/**
 * What a click on a segment (or a whole row) runs. `action` is a core id,
 * `plugin:action`, or a bare local id.
 */
export interface Click {
  action: string;
  args?: unknown;
}
export interface Segment {
  click?: Click | null;
  style?: Style | null;
  text: string;
}
/**
 * `fg`/`bg`: a theme token or `#rrggbb`.
 */
export interface Style {
  bg?: string | null;
  bold?: boolean;
  dim?: boolean;
  fg?: string | null;
  italic?: boolean;
  underline?: boolean;
}
/**
 * Body of `PUT /api/plugins/{id}/widgets/{widget}`.
 */
export interface WidgetPut {
  scopes: {
    [k: string]: WidgetItem[];
  };
  ttl_secs?: number | null;
}
/**
 * `GET /api/plugins/widgets[?plugin=]`.
 */
export interface WidgetsSnapshot {
  plugins: {
    [k: string]: PluginWidgets;
  };
}
export interface PluginWidgets {
  rev: number;
  widgets: {
    [k: string]: WidgetContent;
  };
}
/**
 * One stored widget as `GET /api/plugins/widgets` returns it.
 */
export interface WidgetContent {
  scopes: {
    [k: string]: WidgetItem[];
  };
  slot: WidgetSlot;
}
/**
 * One entry of `GET /api/workspaces`.
 */
export interface Workspace {
  branch?: string | null;
  dir: string;
  name: string;
}
