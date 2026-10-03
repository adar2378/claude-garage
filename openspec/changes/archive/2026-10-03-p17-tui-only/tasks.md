# Tasks: p17-tui-only

## 1. Daemon: worktree fixes (`daemon/src/`)

- [x] 1.1 `worktrees.js`: on failed `merge --no-ff`, run `git -C repoDir merge --abort` (ignore its error), then 409 with stderr
- [x] 1.2 `sessions.js`: add `target` (current branch of `repoDir`, or `null`) to the worktree record in both DELETE paths (live and `?meta=1`)
- [x] 1.3 Tests against scratch git repos (scratchpad, never `~/.garage`): conflict leaves no merge in progress; dirty worktree refuses merge; `target` reported

## 2. TUI: `I` installs hooks too (`wall/src/`)

- [x] 2.1 `api/client.rs`: `install_hooks()` → `POST /api/hooks/install`, model with `installed` / `alreadyInstalled`
- [x] 2.2 `runtime.rs`: install effect calls statusline then hooks; one settled event; combined notice (success / already installed / which one failed); rename busy guard to `installing`
- [x] 2.3 Update `STATUSLINE_HINT` (`ui/strip.rs`) and the `I` row in `help.rs` to say hooks + context meter
- [x] 2.4 Unit tests for the combined notice text (both ok, hooks already installed, one failed)

## 3. TUI: worktree finish overlay (`wall/src/`)

- [x] 3.1 `api/client.rs`: worktree record model with `target`; `finish_worktree(record, action)` → `POST /api/worktrees/finish`, error message from body
- [x] 3.2 `state/wall_state.rs` + `store.rs`: `OverlayKind::WorktreeFinish` holding the record, busy flag, inline error, armed discard
- [x] 3.3 `runtime.rs`: on close with `Some(worktree)` (both live and meta-only) open the overlay instead of the "web wall" notice; key handling `m` / `d d` / `k` / `Esc`; `WorktreeFinishSettled` closes or shows error
- [x] 3.4 New `ui/worktree_finish.rs`: render branch, target, choices, armed prompt, busy and error lines; draw it in the overlay pass; mouse hit targets if other overlays have them
- [x] 3.5 Unit tests: key routing, armed discard, error keeps overlay open, keep sends nothing
- [x] 3.6 Remove web-wall wording from TUI strings and comments (`runtime.rs`, `ui/pet.rs`, `client.rs`, `workspace_form.rs`, `wall_state.rs`)

## 4. Phase A verification

- [x] 4.1 `npm test`, `cd wall && cargo test --lib --bins`, `cargo clippy` green
- [x] 4.2 Screenshot the overlay and the `I` notice for review
- [x] 4.3 Live (main session only, ask the user first): spawn a worktree session with `N`, make a commit, `x x` → merge; repeat → discard; repeat with dirty tree → error inline → keep. Record in `verification.md`

## 5. Remove the web wall

- [x] 5.1 Delete `ui/`; drop the `ui` workspace, `ui/dist` from `files`, `prepack`, `ui/test` from `npm test`, the `dev` script and `concurrently`
- [x] 5.2 Check `grep -rn "node-pty\|ws\b\|@fastify/static" daemon/src`; delete `term.js` and its registration; drop `ws`, `node-pty`, `@fastify/static`, the `postinstall` chmod; refresh `package-lock.json`
- [x] 5.3 `daemon/src/index.js`: remove static serving and `GARAGE_SERVE_UI`; `security.js`: remove `:5173` and own-origin UI entries, confirm Origin-less requests still pass
- [x] 5.4 `notify.js`: drop `-open <UI_URL>` from `terminal-notifier`
- [x] 5.5 `bin/garage.js`: bare command runs `tuiMain`; `tui` alias; delete web `main()` path and browser `open`; stop passing `GARAGE_SERVE_UI` in `startDetachedDaemon`; update header comments and usage text
- [x] 5.6 `package.json` description and keywords: drop browser / xterm wording

## 6. Docs and release

- [x] 6.1 README: `I` row says hooks + context meter; install command becomes `npx claude-garage`; mention worktree finish under keys
- [x] 6.2 CHANGELOG 0.5.0 entry marking the breaking change; bump version
- [x] 6.3 Update `RELEASE-CHECKLIST.md` (no web wall, no ui prepack) and `lefthook.yml` if it references the UI

## 7. Phase B verification

- [x] 7.1 `npm test`, `cargo test --lib --bins`, `cargo clippy` green; `npm pack --dry-run` lists no `ui/` files
- [x] 7.2 Fresh install from the packed tarball into a scratch dir: `npx claude-garage` opens the TUI, `GET /` serves no HTML, `/api/health` is 200
- [x] 7.3 Record results in `verification.md`
