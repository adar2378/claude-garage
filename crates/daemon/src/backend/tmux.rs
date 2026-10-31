//! Port of `daemon/src/tmux.js`: the `SessionBackend` implementation over tmux.
//! 
//! Always passes `-L <socket>` when `GARAGE_TMUX_SOCKET` is set (see `Config`).
