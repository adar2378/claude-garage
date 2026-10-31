# Tasks: p25-first-party-plugins

## 1. Core data
- [x] 1.1 Daemon keeps closed-session records (id, workspace, title, closed at) with an API to list them (records also keep the worktree, so a restored worktree chat can still be finished)

## 2. First-party plugins (`plugins/`)
- [ ] 2.1 closed-chats (rail widget rows + `H` recent overlay)
- [ ] 2.2 jump-palette (`/` overlay pane)
- [ ] 2.3 quick-send (`s` / `S`)
- [ ] 2.4 tile-buttons (close / delete)
- [ ] 2.5 usage-bar (5h / weekly, persisted)
- [ ] 2.6 turn-markers
- [ ] 2.7 prune-empty-projects (`session.closed` hook)
- [ ] 2.8 cli-extras (`<folder>`, `install`, `prune`)
- [ ] 2.9 pit-pet (moved out of `ui/pet.rs`, same behaviour)

## 3. Core items
- [ ] 3.1 Draft and pasted-screenshot recovery
- [ ] 3.2 Mouse selection and copy
- [ ] 3.3 Sidebar hide and resize
- [ ] 3.4 Quiet startup (`GARAGE_VERBOSE=1` for full output)

## 4. API freeze
- [ ] 4.1 List every plugin API gap found in group 2 and close it in the plugin host
- [ ] 4.2 Freeze plugin API v1 and document it

## 5. Verification
- [ ] 5.1 Unit and contract tests per plugin
- [ ] 5.2 Live: each plugin on macOS and Linux; pit pet behaves as before
- [ ] 5.3 Record results in `verification.md`
