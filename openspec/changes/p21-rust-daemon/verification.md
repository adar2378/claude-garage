# p21 verification

## 5. tmux control mode

Measured with `cargo test -p garage-daemon control:: -- --nocapture` on an isolated `TestTmux` server (tmux 3.7b, macOS):

- Event latency, session change to `SessionsChanged` on the bus (poller tick is 2 s): create 89 ms, rename 113 ms, kill 185 ms.
- Control client death (`detach-client`): reconnected in 211 ms; the 2 s tick alone covered the gap.
- Server death then restart: client reconnects with backoff (250 ms doubling to 5 s) and events are fast again.
- Idle: the `tmux -C` client used 0:00.00 CPU over 20 s; no polling of its own. With no tmux server it retries `list-sessions` at most every 5 s and never starts a server.
- Pane-title changes produce no control-mode notification (checked by raw capture), so the 2 s tick is unchanged.
- Contract suite against the Rust daemon: 12/12.
