/* Generated from schemas/garage-plugin-api.schema.json by scripts/gen-types.mjs.
 * Do not edit. Run `npm run gen` after the schema changes. */

export type Placement =
  | {
      kind: "flow";
    }
  | {
      kind: "filler";
      x: number;
      caption?: string | null;
    };
/**
 * Where a widget draws. The scope key of its content depends on the slot.
 */
export type WidgetSlot =
  "tile.title.right" | "tile.gutter" | "rail.row" | "rail.extra_rows" | "rail.header" | "strip.right";
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
      kind: "host";
      id: string;
    };
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
export type ContextSource = "statusline" | "transcript";

/**
 * Root of the published schema: one field per public type, so every type is
 * a named definition (the SDK generators turn definitions into types).
 */
export interface ApiTypes {
  widget_put: WidgetPut;
  widget_item: WidgetItem;
  put_widget_response: PutWidgetResponse;
  ui_action_request: UiActionRequest;
  ui_action_frame: UiActionFrame;
  notice_request: NoticeRequest;
  session_input_request: SessionInputRequest;
  open_pane_response: OpenPaneResponse;
  close_pane_response: ClosePaneResponse;
  plugin_list: PluginList;
  widgets_snapshot: WidgetsSnapshot;
  panes_response: PanesResponse;
  run_context: RunContext;
  event_name: EventName;
  event_payloads: EventPayloads;
  features_response: FeaturesResponse;
  health: Health;
  sessions: Session[];
  workspaces: Workspace[];
  usage: Usage;
  error: ErrorBody;
  ack: Ack;
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
 * One thing a widget shows. `segments` and `frames` are mutually exclusive;
 * the gutter fields (`row` / `kind`) apply to `tile.gutter`, and
 * `row_style` / `click` to `rail.extra_rows` rows (and gutter markers).
 */
export interface WidgetItem {
  priority?: number;
  placement?: Placement;
  segments?: Segment[] | null;
  frames?: Segment[][] | null;
  frame_ms?: number | null;
  /**
   * `strip.right` filler items only: the filler column of each frame, in
   * step with `frames` (same length), overriding `placement.x`. A walking
   * pet changes column from frame to frame; a plain `x` cannot say that.
   */
  frame_x?: number[] | null;
  row?: number | null;
  kind?: string | null;
  row_style?: Style | null;
  click?: Click | null;
  /**
   * `rail.extra_rows` only: the session this row is about (a closed chat),
   * which scopes its click so a core action such as `session.restore`
   * takes it as its target. Must be a session of the row's workspace.
   */
  session?: string | null;
}
export interface Segment {
  text: string;
  style?: Style | null;
  click?: Click | null;
}
/**
 * `fg`/`bg`: a theme token or `#rrggbb`.
 *
 * Build one with [`Style::new`] and the setters; fields may be added in
 * later v1 releases, so a struct literal is not available outside this crate.
 */
export interface Style {
  fg?: string | null;
  bg?: string | null;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  dim?: boolean;
}
/**
 * What a click on a segment (or a whole row) runs. `action` is a core id,
 * `plugin:action`, or a bare local id.
 */
export interface Click {
  action: string;
  args?: unknown;
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
/**
 * Where a `ui_action` request applies. Core actions take their session and
 * workspace from here and nowhere else.
 */
export interface UiTarget {
  session?: string | null;
  workspace?: string | null;
}
/**
 * The SSE `ui_action` frame: a plugin asked the TUI to run an action. There
 * is deliberately no `confirmed` field: whether a dangerous action runs is
 * the human's decision at the TUI, never the plugin's.
 */
export interface UiActionFrame {
  plugin: string;
  action: string;
  args?: {
    [k: string]: unknown;
  };
  target?: UiTarget;
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
 * `POST /api/sessions/input` (needs `sessions.input`, dangerous).
 */
export interface SessionInputRequest {
  /**
   * A `garage/<workspace>/<label>` session id.
   */
  id: string;
  /**
   * Typed into the session, then Enter.
   */
  text: string;
  if_status?: IfStatus | null;
}
/**
 * Refuse the input (409) when the session is in one of these statuses.
 */
export interface IfStatus {
  not: string[];
}
/**
 * Reply of `POST /api/plugins/{id}/panes/{pane}/open`.
 */
export interface OpenPaneResponse {
  ok: boolean;
  /**
   * The backend session id of the pane.
   */
  session: string;
  /**
   * The pane was already open (a singleton) and was reused.
   */
  reused: boolean;
}
/**
 * Reply of `DELETE /api/plugins/{id}/panes/{pane}`.
 */
export interface ClosePaneResponse {
  ok: boolean;
  closed: boolean;
}
/**
 * `GET /api/plugins`: the plugins in install order (which is also the
 * tiebreak for two plugins that suggest the same key).
 */
export interface PluginList {
  plugins: PluginSummary[];
}
export interface PluginSummary {
  id: string;
  name: string;
  enabled: boolean;
  /**
   * Enabled, approved, loaded: only an active plugin's actions, buttons
   * and widgets reach the screen.
   */
  active: boolean;
  version?: string | null;
  /**
   * Where it came from: `bundled`, `local` (a linked folder) or `github`.
   * Empty from a daemon that does not say.
   */
  source?: string;
  permissions?: string[];
  /**
   * The user's own key bindings (`action` local id to key).
   */
  bindings?: {
    [k: string]: string;
  };
  actions?: ActionSummary[];
  buttons?: ButtonSummary[];
  widgets?: WidgetSummary[];
  panes?: PaneSummary[];
}
export interface ActionSummary {
  /**
   * The full id, `plugin:action`.
   */
  id: string;
  title: string;
  default_key?: string | null;
  /**
   * `global`, `workspace` or `session`: what must be focused.
   */
  contexts?: string[];
  confirm?: boolean;
  /**
   * The local pane id when the action opens a pane instead of running a command.
   */
  pane?: string | null;
}
export interface ButtonSummary {
  slot: WidgetSlot;
  label: string;
  tooltip?: string | null;
  /**
   * A core id, `plugin:action`, or a bare local id.
   */
  on_click: string;
  args?: {
    [k: string]: unknown;
  };
  confirm?: boolean;
  priority?: number;
}
export interface WidgetSummary {
  id: string;
  slot: WidgetSlot;
}
export interface PaneSummary {
  id: string;
  title: string;
  /**
   * `tile`, `overlay` or `popup`.
   */
  placement: string;
  cols?: number | null;
  rows?: number | null;
  /**
   * `cols`/`rows` are percent of the screen.
   */
  pct?: boolean;
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
  slot: WidgetSlot;
  scopes: {
    [k: string]: WidgetItem[];
  };
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
  plugin: string;
  pane: string;
  title: string;
  placement: string;
  /**
   * The backend session id (`plugin/<plugin>/<pane>`); not a `garage/` id,
   * so it is never in the session list.
   */
  session: string;
  attach: AttachSpec;
  cols?: number | null;
  rows?: number | null;
  pct?: boolean;
}
/**
 * The JSON every action, event, pane and CLI run receives: in
 * `GARAGE_CONTEXT` (or the file `GARAGE_CONTEXT_FILE` names) and on stdin.
 * Keys are never absent; a missing value is `null`.
 */
export interface RunContext {
  api_version: number;
  plugin: ContextPlugin;
  trigger: ContextTrigger;
  focus: ContextFocus;
  selection: ContextSelection;
  /**
   * The focused session's worktree, if it has one.
   */
  worktree: ContextWorktree | null;
  /**
   * Set for event hook runs.
   */
  event: ContextEvent | null;
  ui: ContextUi;
  /**
   * How to run `claude-garage` again against this daemon.
   */
  garage?: ContextGarage | null;
}
export interface ContextPlugin {
  id: string;
  version: string;
  /**
   * The plugin's own writable folder.
   */
  state_dir: string;
}
export interface ContextTrigger {
  /**
   * `key`, `button`, `widget`, `palette`, `event`, `timer`, `pane` or `cli`.
   */
  kind: string;
  key: string | null;
  slot: string | null;
  action: string | null;
  /**
   * The `args` of the button or click; `{}` when none. For a `cli` run,
   * `{"argv": [...]}`.
   */
  args: {
    [k: string]: unknown;
  };
}
export interface ContextFocus {
  workspace: string | null;
  session: ContextSession | null;
  /**
   * CLI runs: the caller's working directory. `null` for every other run.
   */
  dir: string | null;
}
export interface ContextSession {
  id: string;
  dir: string | null;
  status: string | null;
  branch: string | null;
  worktree: ContextWorktree | null;
}
export interface ContextWorktree {
  path: string;
  branch: string;
  repo_dir: string;
}
export interface ContextSelection {
  session: string | null;
  index: number | null;
  view: string | null;
}
export interface ContextEvent {
  /**
   * One of the event names (see [`EventName`]).
   */
  name: string;
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
}
export interface ContextUi {
  /**
   * Slot geometry the wall reported (`width`, `max_cols`, `filler`), by
   * slot name. May be empty.
   */
  slots: {
    [k: string]: unknown;
  };
  /**
   * Whether a wall is in view right now (it reported itself visible within
   * the last 45 seconds). `false` when none did, or from a daemon that does
   * not say.
   */
  visible?: boolean;
  /**
   * The terminal the wall runs in, as the wall reported it: a macOS bundle
   * id (`com.mitchellh.ghostty`) or a `TERM_PROGRAM` name (`ghostty`).
   * `null` until a wall has said.
   */
  terminal?: string | null;
}
/**
 * How to run `claude-garage` again on this machine: what a plugin needs to
 * build a command that talks to this daemon (for example a notification
 * that runs `claude-garage alerts open ...` when clicked).
 */
export interface ContextGarage {
  /**
   * Absolute path of the `claude-garage` binary, when known.
   */
  bin: string | null;
  port: number;
  /**
   * `GARAGE_DIR`.
   */
  dir: string;
  /**
   * `GARAGE_TMUX_SOCKET`, when set.
   */
  tmux_socket: string | null;
}
/**
 * The payload (`event.data`) of each event, by event name. `daemon.started`
 * and `timer` carry `{}`.
 */
export interface EventPayloads {
  "session.created": SessionCreated;
  "session.closed": SessionClosed;
  "session.turn": SessionTurn;
  "session.forgotten": SessionForgotten;
  "session.status_changed": SessionStatusChanged;
  "worktree.created": WorktreeCreated;
  "workspace.added": WorkspaceNamed;
  "workspace.removed": WorkspaceNamed;
}
export interface SessionCreated {
  id: string;
  dir: string;
  workspace: string;
}
export interface SessionClosed {
  id: string;
  dir: string;
  workspace: string;
  /**
   * `user`, `crash` or `daemon_restart`.
   */
  reason: string;
  last_status: string | null;
}
export interface SessionTurn {
  id: string;
  workspace: string;
  /**
   * Unix milliseconds.
   */
  at: number;
  /**
   * Where the turn ended. Left out when nothing could be measured.
   */
  anchor?: TurnAnchor | null;
}
/**
 * The `anchor` of a `session.turn` event. Each value is `null` when it could
 * not be measured.
 */
export interface TurnAnchor {
  /**
   * The transcript's byte length when the turn ended.
   */
  transcript_offset: number | null;
  /**
   * Absolute scrollback row where the turn ended (0 is the oldest kept line).
   */
  row: number | null;
  /**
   * How many history lines the backend keeps.
   */
  history_limit: number | null;
}
export interface SessionForgotten {
  id: string;
  workspace: string;
}
export interface SessionStatusChanged {
  id: string;
  dir: string;
  workspace: string;
  from: SessionStatus;
  to: SessionStatus;
  since: number | null;
}
export interface WorktreeCreated {
  workspace: string;
  session: string;
  path: string;
  branch: string;
  repo_dir: string;
}
export interface WorkspaceNamed {
  name: string;
}
/**
 * `GET /api/plugins/features`.
 */
export interface FeaturesResponse {
  api_versions: number[];
  /**
   * The garage version.
   */
  garage: string;
  features: Features;
}
/**
 * The `features` object. New keys may appear in later garage versions.
 */
export interface Features {
  "api.v1.frozen": boolean;
  events: string[];
  permissions: string[];
  dangerous_permissions: string[];
  theme_tokens: string[];
  routes: RouteInfo[];
  slots: string[];
  "widgets.click_args": boolean;
  "widgets.filler": boolean;
  "widgets.animated": boolean;
  "widgets.frame_x": boolean;
  "widgets.slot_geometry": boolean;
  "ui.notice": boolean;
  "ui.visible"?: boolean;
  "ui.terminal"?: boolean;
  "context.garage"?: boolean;
  "cli.fallback": boolean;
  "panes.placements": string[];
  "sessions.input": boolean;
  "actions.core_invocable": string[];
  limits: FeatureLimits;
}
/**
 * One entry of `features.routes`.
 */
export interface RouteInfo {
  method: string;
  path: string;
  /**
   * `none` or a permission name.
   */
  needs: string;
  /**
   * `deprecated` when the route answers without a plugin token today but
   * takes one; plugins always send theirs. Absent otherwise.
   */
  tokenless?: string | null;
}
export interface FeatureLimits {
  widget_items: number;
  widget_segments: number;
  widget_cols: number;
  widget_body_bytes: number;
  widget_puts_per_sec: number;
  widget_ttl_secs: number;
  widget_frames: number;
  notice_chars: number;
}
/**
 * `GET /api/health`.
 */
export interface Health {
  status: string;
  version?: string | null;
  pid: number;
}
/**
 * One entry of `GET /api/sessions`: live, or restorable (`status` is
 * `restorable`, `restorable` is `true`, no `createdAt`).
 */
export interface Session {
  id: string;
  workspace: string;
  label: string;
  dir?: string | null;
  attached: boolean;
  createdAt?: number | null;
  status: SessionStatus;
  message?: string | null;
  since?: number | null;
  branch?: string | null;
  title?: string | null;
  context?: Context | null;
  restorable?: boolean | null;
}
/**
 * `context` of a session list entry. The percentage is clamped to 0-100 by
 * the daemon but may be fractional (statusline) or rounded (transcript).
 */
export interface Context {
  usedPercentage: number;
  source: ContextSource;
}
/**
 * One entry of `GET /api/workspaces`.
 */
export interface Workspace {
  name: string;
  dir: string;
  branch?: string | null;
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
  usedPercentage: number;
  resetsAt: unknown;
}
/**
 * Body of every non-2xx response: `{"error": "<message>", "code":
 * "<stable_snake_code>"}`, plus `"permission"` on a 403 for a missing
 * permission. `error` is for people and may change; `code` is the contract.
 * Routes may add other fields.
 */
export interface ErrorBody {
  error: string;
  code: string;
  /**
   * The permission the caller lacks (`code` is `permission_denied`).
   */
  permission?: string | null;
}
/**
 * `{"ok": true}` acknowledgements.
 */
export interface Ack {
  ok: boolean;
}
