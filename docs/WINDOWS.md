# claude-garage on Windows

claude-garage runs natively on Windows 10/11 in Windows Terminal, PowerShell
or cmd. No WSL and no tmux. This page explains how it works, what was added,
and what is known to be missing.

> Status: working and used daily, but not yet exercised on macOS/Linux after
> these changes. See [Reviewer notes](#reviewer-notes) for exactly which
> shared code was touched.

## Install and run

You need:

- Windows 10 1809+ (ConPTY) or Windows 11
- Node.js 20 or newer
- [Claude Code](https://docs.claude.com/en/docs/claude-code) on your `PATH`
- Rust ([rustup.rs](https://rustup.rs)) with the MSVC build tools, for the
  first run only. The TUI builds itself once into `wall/dist`.

From a checkout:

```powershell
npm install
npm link          # optional: gives you the `claude-garage` command
claude-garage
```

### Two ways to start

| Command | What it does |
|---|---|
| `claude-garage` | Pick up where you left off. Restores every session that died with a reboot or crash. |
| `claude-garage .` or `claude-garage <folder>` | Work on that folder only. Adds it as a project if new, restores that folder's stopped sessions, or starts a fresh session if there is nothing to restore. Other projects' stopped sessions stay stopped. |
| `claude-garage install` | Same as pressing `I`: installs the statusline feed and hooks. |
| `claude-garage prune` | Remove projects that have no sessions and no closed chats. |
| `claude-garage restart` | Restart the background daemon (add `--sessions` for sessions). |

Set `GARAGE_NO_AUTORESTORE=1` to skip the automatic restore on a plain start.
Startup is quiet (no daemon notices); `GARAGE_VERBOSE=1` brings them back.

**Empty projects go away.** When you close a project's last chat and nothing
is left to reopen (no grey closed chat), the project is unregistered too.
Set `GARAGE_KEEP_EMPTY_PROJECTS=1` to keep them. Reopen a folder any time with
`claude-garage <folder>`.

## How it works

tmux owns the sessions on macOS/Linux. It has no native Windows build, so on
Windows each session is run by a small detached process, the **session host**:

```
garage-wall host   one per session: runs `claude` on a ConPTY, keeps a vt100
                   screen + 10k lines of scrollback, listens on 127.0.0.1
  └─ daemon        Node service: lists hosts from a registry, starts/kills them
       └─ TUI      attaches to a host over a loopback socket (like `tmux attach`)
```

- **Host** (`wall/src/host.rs`): spawned detached by the daemon, so it outlives
  both the daemon and the terminal window. It registers itself in
  `~/.garage/hosts/<id>.json` (port, token, pid). A connection must present the
  per-host random token. Wire protocol is documented at the top of `host.rs`.
- **Daemon backend** (`daemon/src/host-backend.js`): same function surface as
  `tmux.js` (list, create, kill, rename, respawn). `tmux.js` dispatches to it
  when `USE_HOST` is true (`win32`, or `GARAGE_BACKEND=host` on any OS).
- **TUI** (`wall/src/pty.rs`): `TileClient` attaches over the socket instead of
  `tmux attach`. Frozen scrollback is read from the host's own history.
- **`claude` resolution**: npm's `claude.cmd` shim is resolved to the real
  `claude.exe` so the pid the host owns is the pid `claude agents --json`
  reports. Falls back to `cmd /c`.
- **Detached children never flash a console window** (`windowsHide`, and
  `CREATE_NO_WINDOW` for `taskkill`).
- **Launcher**: `.exe` binary names, `netstat` instead of `lsof`, no tmux check.
  A running host locks its exe, so the launcher renames the old binary aside
  before copying a new one.

Quitting garage (`q`) leaves hosts and sessions running. A host dies with a
reboot or if you kill it. Sessions that died come back as `⟳ restorable`
(auto-restored on the next plain `claude-garage`).

## What was added

### Windows support

- Native session host and attach (above), including scrollback.
- `t` opens the focused session in its own terminal window
  (`garage-wall attach <id>`; `Ctrl+G` closes that window, the session stays).
- Windows folder picker for `Ctrl+O` in the add-project overlay.
- Windows toast notifications when a session needs input (macOS keeps its
  notifier).
- Context meter / statusline wrapper written as a Node script (no `sh`/`curl`
  on Windows). An existing statusline is chained and its output preserved.
- Windows paths: project names come from the last folder (`C:\a\my-app` →
  `my-app`), `~\` is expanded, `USERPROFILE` is used when `HOME` is unset.
- Keys through ConPTY: `Shift+Enter` → newline (Alt+Enter), `Ctrl+Enter` →
  send queued message now (`Ctrl+X Ctrl+S`, Claude Code's portable chord).
- `Ctrl+A` selects (and copies) the whole prompt, the Windows convention.
  Home still goes to start of line.
- Clipboard copy through the Win32 clipboard API.

### Features that are not Windows-specific

- **Closed chats stay in the sidebar.** `x x` keeps the conversation as a grey
  `◌` row under its project; click it to reopen (`claude --resume`). Live
  sessions are yellow, the focused one blue, needs-input stays amber.
  `H` lists them, `/` is a fuzzy jump palette over sessions, projects and
  closed chats. `⌫` forgets one.
- **Close / delete buttons** on every row and every tile corner. Closing needs
  two clicks (same guard as `x x`).
- **Quick send**: `s` types a prompt into the focused session without entering
  it; `S` sends it to every live session in the project.
- **Mouse selection** inside a tile with auto-copy on release. The wheel scrolls
  garage's history when the program did not ask for mouse input.
- **Usage bar**: `5h ▰▰▰▱▱▱ 47% (1h40m) · wk 12%` in the strip, green/yellow/red.
  Persisted in `~/.garage/usage.json`, fed by any authenticated statusline post
  (limits are account-wide), `~` marks a figure older than 3 minutes, and a
  window past its reset time is dropped.
- **Draft recovery**: the host saves the unsent prompt once a second
  (`~/.garage/hosts/<id>.draft`) and any image you paste (`Alt+V`/`Ctrl+V`) as
  a PNG. After a power cut, restore types the text back (never presses Enter)
  and re-attaches the screenshots by path.
- **Turn markers**: a blue bar down a tile's left edge marks the current turn,
  grey ticks mark where older turns began.
- **`n` goes where you picked.** A new session opens in the project you
  selected (key `1`-`9`, click, palette). Until you pick one, `n` opens in your
  home folder as its own project instead of whatever project is listed first.
  `N` in a folder that is not a git repo opens a normal session.
- **Recognisable projects and chats**: each project has a stable colour on its
  row and down the left edge of its chats; Claude Code's generic "Claude Code"
  title is ignored so chats show their label (`claude-1`) until they have a
  real summary.
- **Sidebar**: `b` hides/shows it, `<` `>` resize it, or drag its edge.
  Saved in `~/.garage/ui.json`.
- **Open on a folder**: `claude-garage .` (see above), and `claude-garage
  install` for the hooks/statusline.

### New keys

| Key | Action |
|---|---|
| `H` | recently closed chats |
| `/` | jump palette |
| `s` / `S` | send a prompt to the session / to all live sessions in the project |
| `b` | hide / show the sidebar |
| `<` `>` | narrow / widen the sidebar |
| `Ctrl+A` (typing, Windows) | select and copy the whole prompt |
| `Ctrl+Enter` (typing, Windows) | send queued message now |

## Files garage keeps

| Path | What |
|---|---|
| `~/.garage/state.json` | projects, resume info, closed chats |
| `~/.garage/hosts/` | live host registry, drafts, pasted images, host logs |
| `~/.garage/usage.json` | last known 5h / weekly usage |
| `~/.garage/ui.json` | sidebar width / hidden |
| `~/.garage/wall.json` | views and pet (unchanged) |

Drafts and pasted images are plain files, removed when you send the message
or close the session on purpose.

## Known gaps

- Hosts do not survive a reboot (same as tmux). Sessions come back through
  restore; a half-typed prompt comes back with them.
- Draft recovery saves once a second, so the last second of typing can be lost.
- The usage figures only update when a Claude session redraws its statusline.
- Mouse selection is limited to one tile; no auto-scroll while dragging.
- `Ctrl+Enter` is sent as the `Ctrl+X Ctrl+S` chord, so it needs a Claude
  Code with `chat:sendNow` (2.1.275+).
- Windows-only code paths have no CI yet.

## Tests

```powershell
cd wall; cargo test --lib --bins     # 528 tests
npm test                             # daemon tests
```

On Windows the daemon suite has 7 failures that are test-environment issues,
not regressions: 5 need tmux or Unix file permissions, 2 assert the POSIX
`sh` statusline wrapper format.

## Reviewer notes

- The macOS/Linux tmux path is untouched: every new branch is behind
  `USE_HOST` (daemon) or `use_host_backend()` (Rust).
- Shared (all platforms) changes worth a close look: rail colours and
  close/delete buttons (`ui/rail.rs`, `runtime.rs`), the usage bar
  (`ui/strip.rs`, `daemon/src/statusline.js`), closed chats
  (`daemon/src/sessions.js`, `daemon/src/registry.js`), `H` / `/` / `s` / `b`
  overlays and keys, turn markers (`ui/turns.rs`), mouse selection, launcher
  folder mode (`bin/garage.js`).
- A few existing tests were updated for intended behaviour changes (the strip
  usage chip is now a bar, modified-Enter on the Windows host).
