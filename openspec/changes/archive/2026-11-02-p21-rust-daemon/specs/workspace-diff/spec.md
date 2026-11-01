## REMOVED Requirements

### Requirement: Diff endpoint returns changed-file list with per-file detail
**Reason**: The TUI has not called `GET /api/diff/:workspace` since 0.5.0, when diff review left with the web wall. The Rust daemon (p21) does not port it.
**Migration**: None needed; no client uses the route. The archived spec keeps the contract if review returns, likely as a plugin.

### Requirement: Diff content covers tracked changes and untracked files
**Reason**: The TUI has not called `GET /api/diff/:workspace` since 0.5.0, when diff review left with the web wall. The Rust daemon (p21) does not port it.
**Migration**: None needed; no client uses the route. The archived spec keeps the contract if review returns, likely as a plugin.

### Requirement: Diff response supports per-file hunk rendering
**Reason**: The TUI has not called `GET /api/diff/:workspace` since 0.5.0, when diff review left with the web wall. The Rust daemon (p21) does not port it.
**Migration**: None needed; no client uses the route. The archived spec keeps the contract if review returns, likely as a plugin.

### Requirement: Diff endpoint is strictly read-only
**Reason**: The TUI has not called `GET /api/diff/:workspace` since 0.5.0, when diff review left with the web wall. The Rust daemon (p21) does not port it.
**Migration**: None needed; no client uses the route. The archived spec keeps the contract if review returns, likely as a plugin.

### Requirement: Unknown workspace returns 404
**Reason**: The TUI has not called `GET /api/diff/:workspace` since 0.5.0, when diff review left with the web wall. The Rust daemon (p21) does not port it.
**Migration**: None needed; no client uses the route. The archived spec keeps the contract if review returns, likely as a plugin.

### Requirement: Non-git registered directory returns empty file list
**Reason**: The TUI has not called `GET /api/diff/:workspace` since 0.5.0, when diff review left with the web wall. The Rust daemon (p21) does not port it.
**Migration**: None needed; no client uses the route. The archived spec keeps the contract if review returns, likely as a plugin.

### Requirement: Oversized diffs are truncated, not failed
**Reason**: The TUI has not called `GET /api/diff/:workspace` since 0.5.0, when diff review left with the web wall. The Rust daemon (p21) does not port it.
**Migration**: None needed; no client uses the route. The archived spec keeps the contract if review returns, likely as a plugin.

### Requirement: Worktree diffs are fork-point diffs
**Reason**: The TUI has not called `GET /api/diff/:workspace` since 0.5.0, when diff review left with the web wall. The Rust daemon (p21) does not port it.
**Migration**: None needed; no client uses the route. The archived spec keeps the contract if review returns, likely as a plugin.
