//! The keymap layer (p22 tui-actions "Keymap layer"): one table of
//! `(layer, key) -> action id`. [`Keymap::defaults`] reproduces today's
//! bindings exactly; a user config will later layer overrides on top.
//!
//! Arming (`x x`, `X X`, `r r`, `X` then `K`) and the one-shot `r` prefix are
//! behaviour, not bindings: the router keeps that state and decides what an
//! action does on its first versus second press. The engaged layer is not in
//! the keymap — engaged input stays verbatim.
//!
//! [`Keymap::bind`] extends the table (the first binding for a key wins, so
//! it never displaces a default); [`Keymap::bind_override`] is for user
//! config: it replaces whatever the key was bound to, the later layer
//! winning.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::actions::{ids, ActionId};
use crate::state::wall_state::OverlayKind;

/// Which key table a key is looked up in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Garage,
    /// The key after the `r` prefix.
    RestartChord,
    Overlay(OverlayKind),
}

/// A resolved binding: the action, plus the `1`–`9` index for
/// `workspace.focus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub action: ActionId,
    pub index: Option<usize>,
}

#[derive(Clone, Debug)]
struct Binding {
    layer: Layer,
    key: String,
    action: ActionId,
    index: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// The shipped bindings (spec tui-key-routing, spec restart).
    pub fn defaults() -> Self {
        let mut km = Keymap { bindings: Vec::new() };
        for n in 1..=9usize {
            km.bind_indexed(Layer::Garage, &n.to_string(), ids::WORKSPACE_FOCUS, n - 1);
        }
        let garage: &[(&str, ActionId)] = &[
            ("[", ids::FOCUS_CYCLE_PREV),
            ("]", ids::FOCUS_CYCLE_NEXT),
            ("a", ids::JUMP_BLOCKED),
            ("A", ids::OVERLAY_OPEN_QUEUE),
            ("n", ids::SESSION_SPAWN),
            ("N", ids::SESSION_SPAWN_WORKTREE),
            ("m", ids::SESSION_MAXIMIZE),
            ("R", ids::SESSION_RESTORE_ALL),
            ("x", ids::SESSION_CLOSE),
            ("w", ids::OVERLAY_OPEN_WORKSPACE_ADD),
            ("X", ids::WORKSPACE_REMOVE),
            ("K", ids::WORKSPACE_REMOVE_KILL),
            ("d", ids::SESSION_DETACH),
            ("D", ids::OVERLAY_OPEN_VIEW_PICKER),
            ("\t", ids::VIEW_CYCLE),
            ("\n", ids::SESSION_ENGAGE),
            ("\r", ids::SESSION_ENGAGE),
            ("?", ids::OVERLAY_OPEN_HELP),
            ("I", ids::HOOKS_INSTALL),
            ("t", ids::SESSION_OPEN_WINDOW),
            ("P", ids::PET_CYCLE_SPECIES),
            ("r", ids::CHORD_RESTART),
            ("q", ids::APP_QUIT),
        ];
        for (key, action) in garage {
            km.bind(Layer::Garage, key, *action);
        }
        km.bind(Layer::RestartChord, "r", ids::SESSION_RESTART);
        km.bind(Layer::RestartChord, "a", ids::WORKSPACE_RESTART);
        km.bind(Layer::RestartChord, "d", ids::DAEMON_RESTART);

        for key in ["esc", "?", "q"] {
            km.bind(Layer::Overlay(OverlayKind::Help), key, ids::OVERLAY_CLOSE);
        }
        let queue = Layer::Overlay(OverlayKind::TriageQueue);
        km.bind(queue, "esc", ids::OVERLAY_CLOSE);
        km.bind(queue, "down", ids::OVERLAY_SELECT_NEXT);
        km.bind(queue, "j", ids::OVERLAY_SELECT_NEXT);
        km.bind(queue, "up", ids::OVERLAY_SELECT_PREV);
        km.bind(queue, "k", ids::OVERLAY_SELECT_PREV);
        km.bind(queue, "enter", ids::JUMP_BLOCKED);
        km
    }

    /// Replace the binding of `key` in `layer` (any index it carried is
    /// dropped), or add it when unbound. Later calls win.
    pub fn bind_override(&mut self, layer: Layer, key: &str, action: ActionId) {
        self.bindings.retain(|b| !(b.layer == layer && b.key == key));
        self.bind(layer, key, action);
    }

    /// Bind `key` in `layer` (the first binding for a key wins on lookup, so
    /// this extends the table rather than overriding a default — use
    /// [`Keymap::bind_override`] for that).
    pub fn bind(&mut self, layer: Layer, key: &str, action: ActionId) {
        self.bindings.push(Binding {
            layer,
            key: key.to_owned(),
            action,
            index: None,
        });
    }

    fn bind_indexed(&mut self, layer: Layer, key: &str, action: ActionId, index: usize) {
        self.bindings.push(Binding {
            layer,
            key: key.to_owned(),
            action,
            index: Some(index),
        });
    }

    /// The first binding for `key` in `layer`.
    pub fn resolve(&self, layer: Layer, key: &str) -> Option<Resolved> {
        self.bindings
            .iter()
            .find(|b| b.layer == layer && b.key == key)
            .map(|b| Resolved {
                action: b.action,
                index: b.index,
            })
    }

    /// Every bound action id, for reachability tests.
    pub fn bound_actions(&self) -> impl Iterator<Item = ActionId> + '_ {
        self.bindings.iter().map(|b| b.action)
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::defaults()
    }
}

/// The overlay-layer key string: `esc` / `enter` / `up` / `down` regardless
/// of modifiers (as the overlay handlers always matched them), and a plain
/// (non-Ctrl/Alt) character as itself.
pub fn overlay_key_string(key: &KeyEvent) -> Option<String> {
    match key.code {
        KeyCode::Esc => Some("esc".to_owned()),
        KeyCode::Enter => Some("enter".to_owned()),
        KeyCode::Up => Some("up".to_owned()),
        KeyCode::Down => Some("down".to_owned()),
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(c.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn garage(key: &str) -> Option<(&'static str, Option<usize>)> {
        Keymap::defaults()
            .resolve(Layer::Garage, key)
            .map(|r| (r.action.as_str(), r.index))
    }

    #[test]
    fn every_default_garage_binding_resolves_to_its_id() {
        let table: &[(&str, &str)] = &[
            ("[", "focus.cycle.prev"),
            ("]", "focus.cycle.next"),
            ("a", "jump.blocked"),
            ("A", "overlay.open.queue"),
            ("n", "session.spawn"),
            ("N", "session.spawn.worktree"),
            ("m", "session.maximize"),
            ("R", "session.restore_all"),
            ("x", "session.close"),
            ("w", "overlay.open.workspace_add"),
            ("X", "workspace.remove"),
            ("K", "workspace.remove.kill"),
            ("d", "session.detach"),
            ("D", "overlay.open.view_picker"),
            ("\t", "view.cycle"),
            ("\n", "session.engage"),
            ("\r", "session.engage"),
            ("?", "overlay.open.help"),
            ("I", "hooks.install"),
            ("t", "session.open_window"),
            ("P", "pet.cycle_species"),
            ("r", "chord.restart"),
            ("q", "app.quit"),
        ];
        for (key, id) in table {
            assert_eq!(garage(key), Some((*id, None)), "key {key:?}");
        }
        for n in 1..=9usize {
            assert_eq!(
                garage(&n.to_string()),
                Some(("workspace.focus", Some(n - 1)))
            );
        }
    }

    #[test]
    fn every_action_id_is_reachable_from_a_key_or_a_click() {
        // Ids that only a click produces (see `hit_targets::action_at`).
        let click_only = [
            ids::SESSION_FOCUS,
            ids::SESSION_RESTORE,
            ids::PET_PET,
            ids::VIEW_FOCUS,
            ids::OVERLAY_VIEW_PICKER_PICK,
            // No key by design: only a plugin's click target reaches it
            // (p24), always through the confirm rule.
            ids::SESSION_DELETE,
        ];
        let bound: Vec<ActionId> = Keymap::defaults().bound_actions().collect();
        for id in ids::ALL {
            assert!(
                bound.contains(id) || click_only.contains(id),
                "{} has neither a key nor a click",
                id.as_str()
            );
        }
    }

    #[test]
    fn unbound_garage_keys_stay_unbound() {
        for key in ["0", "z", " ", "i", "k", "j", "p", "Q"] {
            assert_eq!(garage(key), None, "key {key:?}");
        }
    }

    #[test]
    fn an_override_replaces_a_default_and_bind_does_not() {
        let mut km = Keymap::defaults();
        km.bind(Layer::Garage, "q", ids::SESSION_MAXIMIZE);
        assert_eq!(km.resolve(Layer::Garage, "q").unwrap().action, ids::APP_QUIT, "bind extends");
        km.bind_override(Layer::Garage, "q", ids::SESSION_MAXIMIZE);
        assert_eq!(km.resolve(Layer::Garage, "q").unwrap().action, ids::SESSION_MAXIMIZE);
        km.bind_override(Layer::Garage, "q", ids::OVERLAY_OPEN_HELP);
        assert_eq!(km.resolve(Layer::Garage, "q").unwrap().action, ids::OVERLAY_OPEN_HELP, "later wins");
        // An indexed default loses its index when replaced; other layers are untouched.
        km.bind_override(Layer::Garage, "1", ids::SESSION_MAXIMIZE);
        assert_eq!(km.resolve(Layer::Garage, "1").unwrap().index, None);
        assert_eq!(km.resolve(Layer::Overlay(OverlayKind::Help), "q").unwrap().action, ids::OVERLAY_CLOSE);
    }

    #[test]
    fn restart_chord_second_keys() {
        let km = Keymap::defaults();
        let id = |k| km.resolve(Layer::RestartChord, k).map(|r| r.action.as_str());
        assert_eq!(id("r"), Some("session.restart"));
        assert_eq!(id("a"), Some("workspace.restart"));
        assert_eq!(id("d"), Some("daemon.restart"));
        assert_eq!(id("x"), None, "a mistyped chord key resolves to nothing");
    }

    #[test]
    fn overlay_keys_resolve_per_overlay() {
        let km = Keymap::defaults();
        let id = |kind, k| km.resolve(Layer::Overlay(kind), k).map(|r| r.action.as_str());
        for k in ["esc", "?", "q"] {
            assert_eq!(id(OverlayKind::Help, k), Some("overlay.close"));
        }
        assert_eq!(id(OverlayKind::Help, "j"), None);
        assert_eq!(id(OverlayKind::TriageQueue, "enter"), Some("jump.blocked"));
        assert_eq!(id(OverlayKind::TriageQueue, "j"), Some("overlay.select.next"));
        assert_eq!(id(OverlayKind::TriageQueue, "down"), Some("overlay.select.next"));
        assert_eq!(id(OverlayKind::TriageQueue, "k"), Some("overlay.select.prev"));
        assert_eq!(id(OverlayKind::TriageQueue, "up"), Some("overlay.select.prev"));
        assert_eq!(id(OverlayKind::TriageQueue, "esc"), Some("overlay.close"));
        assert_eq!(id(OverlayKind::TriageQueue, "?"), None);
    }

    #[test]
    fn overlay_key_strings_ignore_modifiers_on_named_keys_only() {
        let k = |code, mods| overlay_key_string(&KeyEvent::new(code, mods));
        assert_eq!(k(KeyCode::Esc, KeyModifiers::CONTROL).as_deref(), Some("esc"));
        assert_eq!(k(KeyCode::Char('j'), KeyModifiers::NONE).as_deref(), Some("j"));
        assert_eq!(k(KeyCode::Char('J'), KeyModifiers::SHIFT).as_deref(), Some("J"));
        assert_eq!(k(KeyCode::Char('j'), KeyModifiers::CONTROL), None);
    }
}
