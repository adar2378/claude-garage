//! Plugin panes (design section 6): a pane is one process started through
//! `SessionBackend::create`, so the TUI attaches to it with the same PTY code
//! as an agent tile.
//!
//! Backend id `plugin/<plugin>/<pane>[-<n>]`. tmux rewrites `.` and `:` in a
//! session name, so the design's dotted form would not survive a lookup. The
//! prefix is not `garage/`, so `list()` and `panes()` of both backends skip a
//! pane (it never reaches the session list, the poller or restore) while
//! `has`, `kill`, `create` and `attach_spec` work by id. The table of open
//! panes is what marks a backend session as a plugin pane.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use garage_protocol::plugins::PaneInstance;
use serde_json::Value;

use super::{Inner, PluginHost, PluginState};
use crate::backend::SessionBackend;
use crate::bus::DaemonEvent;

/// Backend id prefix of every plugin pane.
pub const PANE_PREFIX: &str = "plugin/";

#[derive(Debug, Clone, PartialEq)]
pub struct PaneRec {
    pub plugin: String,
    pub pane: String,
    pub session: String,
    pub title: String,
    pub placement: String,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub pct: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PaneError {
    UnknownPlugin,
    Disabled,
    UnknownPane,
    NoBackend,
    Backend(String),
}

impl std::fmt::Display for PaneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaneError::UnknownPlugin => write!(f, "no such plugin"),
            PaneError::Disabled => write!(f, "the plugin is disabled"),
            PaneError::UnknownPane => write!(f, "the plugin declares no such pane"),
            PaneError::NoBackend => write!(f, "no session backend"),
            PaneError::Backend(e) => write!(f, "{e}"),
        }
    }
}

/// `plugin/<plugin>/<pane>[-<n>]` into its plugin and pane ids.
pub fn parse_pane_session(id: &str) -> Option<(&str, &str)> {
    let rest = id.strip_prefix(PANE_PREFIX)?;
    let (plugin, pane) = rest.split_once('/')?;
    (!plugin.is_empty() && !pane.is_empty()).then_some((plugin, pane))
}

impl Inner {
    fn backend(&self) -> Option<Arc<dyn SessionBackend>> {
        self.backend.get().cloned()
    }

    pub(super) fn kill_panes_of(self: &Arc<Self>, plugin: &str) {
        let gone: Vec<PaneRec> = {
            let mut panes = self.panes.lock().unwrap();
            let (gone, keep) = std::mem::take(&mut *panes).into_iter().partition(|p: &PaneRec| p.plugin == plugin);
            *panes = keep;
            gone
        };
        let (Some(backend), Ok(_)) = (self.backend(), tokio::runtime::Handle::try_current()) else { return };
        let bus = self.bus.clone();
        tokio::spawn(async move {
            for p in gone {
                let _ = backend.kill(&p.session).await;
                bus.publish(DaemonEvent::PaneChanged { plugin: p.plugin, pane: p.pane, session: p.session, open: false });
            }
        });
    }
}

impl PluginHost {
    /// Give the host the backend panes run on. Set once, at wiring time.
    pub fn set_backend(&self, backend: Arc<dyn SessionBackend>) {
        let _ = self.0.backend.set(backend);
    }

    /// Open (or, for a singleton, find) a pane. `body` is the optional run
    /// context the TUI sends, passed on as `GARAGE_CONTEXT`.
    pub async fn open_pane(&self, plugin: &str, pane: &str, body: &Value) -> Result<(PaneRec, bool), PaneError> {
        let state = self.plugin(plugin).ok_or(PaneError::UnknownPlugin)?;
        if !state.active() {
            return Err(PaneError::Disabled);
        }
        let manifest = state.manifest().cloned().ok_or(PaneError::Disabled)?;
        let decl = manifest.panes.iter().find(|p| p.id == pane).ok_or(PaneError::UnknownPane)?;
        let backend = self.0.backend().ok_or(PaneError::NoBackend)?;

        if decl.singleton {
            let existing = self.0.panes.lock().unwrap().iter().find(|p| p.plugin == plugin && p.pane == pane).cloned();
            if let Some(rec) = existing {
                if backend.has(&rec.session).await.unwrap_or(false) {
                    return Ok((rec, true));
                }
                self.0.panes.lock().unwrap().retain(|p| p.session != rec.session);
            }
        }
        let n = self.0.pane_seq.fetch_add(1, Ordering::Relaxed);
        let session = if decl.singleton { format!("{PANE_PREFIX}{plugin}/{pane}") } else { format!("{PANE_PREFIX}{plugin}/{pane}-{n}") };
        // A leftover under the same id (daemon restarted, record lost) is replaced.
        if backend.has(&session).await.unwrap_or(false) {
            let _ = backend.kill(&session).await;
        }

        let req = super::context::CtxRequest::from_run_body("pane", &format!("{plugin}:{pane}"), body);
        let ctx = self.context(&state, req).await;
        let (env, _ctx_file) = self.0.run_env(&state, "pane", &ctx).map_err(PaneError::Backend)?;
        let argv = self.0.argv_for(&state, &decl.command);
        let Some((cmd, args)) = argv.split_first() else {
            return Err(PaneError::Backend("the pane has no command".into()));
        };
        backend.create(&session, &state.dir, cmd, args, &env).await.map_err(|e| PaneError::Backend(e.to_string()))?;

        let rec = PaneRec {
            plugin: plugin.to_owned(),
            pane: pane.to_owned(),
            session,
            title: decl.title.clone(),
            placement: match decl.placement {
                garage_plugin::manifest::PanePlacement::Tile => "tile",
                garage_plugin::manifest::PanePlacement::Overlay => "overlay",
                garage_plugin::manifest::PanePlacement::Popup => "popup",
            }
            .to_owned(),
            cols: decl.size.as_ref().map(|s| s.cols),
            rows: decl.size.as_ref().map(|s| s.rows),
            pct: decl.size.as_ref().is_some_and(|s| s.pct),
        };
        self.0.panes.lock().unwrap().push(rec.clone());
        self.0.bus.publish(DaemonEvent::PaneChanged {
            plugin: rec.plugin.clone(),
            pane: rec.pane.clone(),
            session: rec.session.clone(),
            open: true,
        });
        Ok((rec, false))
    }

    /// Close every instance of one pane. Returns how many were closed.
    pub async fn close_pane(&self, plugin: &str, pane: &str) -> Result<usize, PaneError> {
        let backend = self.0.backend().ok_or(PaneError::NoBackend)?;
        let gone: Vec<PaneRec> = {
            let mut panes = self.0.panes.lock().unwrap();
            let (gone, keep) = std::mem::take(&mut *panes).into_iter().partition(|p: &PaneRec| p.plugin == plugin && p.pane == pane);
            *panes = keep;
            gone
        };
        for p in &gone {
            let _ = backend.kill(&p.session).await;
            self.0.bus.publish(DaemonEvent::PaneChanged { plugin: p.plugin.clone(), pane: p.pane.clone(), session: p.session.clone(), open: false });
        }
        Ok(gone.len())
    }

    /// The panes that are still alive, with how to attach. A pane whose
    /// process ended is dropped from the table here and announced as closed.
    pub async fn live_panes(&self) -> Vec<PaneInstance> {
        let Some(backend) = self.0.backend() else { return Vec::new() };
        let recs = self.0.panes.lock().unwrap().clone();
        let mut out = Vec::new();
        for rec in recs {
            if backend.has(&rec.session).await.unwrap_or(false) {
                out.push(PaneInstance {
                    attach: backend.attach_spec(&rec.session),
                    plugin: rec.plugin,
                    pane: rec.pane,
                    title: rec.title,
                    placement: rec.placement,
                    session: rec.session,
                    cols: rec.cols,
                    rows: rec.rows,
                    pct: rec.pct,
                });
            } else {
                self.0.panes.lock().unwrap().retain(|p| p.session != rec.session);
                self.0.bus.publish(DaemonEvent::PaneChanged { plugin: rec.plugin, pane: rec.pane, session: rec.session, open: false });
            }
        }
        out
    }

    /// At daemon start: panes that outlived the daemon are adopted when their
    /// plugin is active and still declares them, and killed otherwise.
    pub async fn adopt_panes(&self) {
        let Some(backend) = self.0.backend() else { return };
        let Ok(sessions) = backend.plugin_pane_sessions().await else { return };
        for session in sessions {
            let Some((plugin, pane)) = parse_pane_session(&session) else { continue };
            let base = pane.rsplit_once('-').filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit())).map_or(pane, |(b, _)| b);
            let decl = self.active(plugin).and_then(|s: PluginState| s.manifest().cloned()).and_then(|m| {
                m.panes.iter().find(|p| p.id == pane || (p.id == base && !p.singleton)).cloned()
            });
            match decl {
                Some(decl) => {
                    let placement = match decl.placement {
                        garage_plugin::manifest::PanePlacement::Tile => "tile",
                        garage_plugin::manifest::PanePlacement::Overlay => "overlay",
                        garage_plugin::manifest::PanePlacement::Popup => "popup",
                    };
                    self.0.panes.lock().unwrap().push(PaneRec {
                        plugin: plugin.to_owned(),
                        pane: decl.id.clone(),
                        session,
                        title: decl.title.clone(),
                        placement: placement.to_owned(),
                        cols: decl.size.as_ref().map(|s| s.cols),
                        rows: decl.size.as_ref().map(|s| s.rows),
                        pct: decl.size.as_ref().is_some_and(|s| s.pct),
                    });
                }
                None => {
                    let _ = backend.kill(&session).await;
                }
            }
        }
    }
}
