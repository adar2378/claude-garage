//! Core wall actions as the plugin contract sees them (design section 7,
//! Gap E). The TUI owns the real table (`ActionMeta`); this copy exists so a
//! manifest can be validated, and a widget click checked, without the TUI. A
//! test in the wall keeps the two lists honest once group 1 lands.

/// What a plugin may do with a core action id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreAction {
    /// Needs the arm-then-confirm step when a plugin triggers it.
    pub dangerous: bool,
    pub plugin_invocable: bool,
}

/// Plugin-invocable ids. Exact ids only, no prefix families: a core id added
/// later is not reachable by plugins until it is listed here on purpose.
const INVOCABLE: &[&str] = &[
    "session.close",
    "session.delete",
    "session.focus",
    "session.maximize",
    "session.spawn",
    "session.spawn.worktree",
    "session.detach",
    "session.open_window",
    "session.restore",
    "session.restore_all",
    "session.restart",
    "workspace.focus",
    "workspace.restart",
    "workspace.remove",
    "workspace.remove.kill",
    "daemon.restart",
    "jump.blocked",
    "view.cycle",
    "view.focus",
    "hooks.install",
    "focus.cycle.prev",
    "focus.cycle.next",
    "overlay.open.queue",
    "overlay.open.help",
    "overlay.open.view_picker",
    "overlay.open.workspace_add",
];

const DANGEROUS: &[&str] = &[
    "session.close",
    "session.delete",
    "session.restart",
    "workspace.restart",
    "workspace.remove",
    "workspace.remove.kill",
    "daemon.restart",
    "session.restore_all",
    "hooks.install",
];

/// Moving focus or switching view: nothing is changed, closed or sent. The
/// `ui.navigate` permission is enough for these; everything else a plugin may
/// trigger needs `core.actions`.
const NAVIGATION: &[&str] = &[
    "jump.blocked",
    "session.focus",
    "workspace.focus",
    "view.cycle",
    "view.focus",
];

/// Real core ids a plugin must never trigger. `session.engage` is here
/// because it moves where the user types: only a human may change that.
const NEVER: &[&str] = &[
    "session.engage",
    "app.quit",
    "overlay.close",
    "overlay.select.next",
    "overlay.select.prev",
    "overlay.view_picker.pick",
    "chord.restart",
    "rail.toggle",
    "rail.narrow",
    "rail.widen",
];

/// `None` when `id` is not a core id at all.
pub fn lookup(id: &str) -> Option<CoreAction> {
    if INVOCABLE.contains(&id) {
        return Some(CoreAction {
            dangerous: DANGEROUS.contains(&id),
            plugin_invocable: true,
        });
    }
    NEVER.contains(&id).then_some(CoreAction {
        dangerous: false,
        plugin_invocable: false,
    })
}

/// Whether `id` is a plugin-invocable, non-dangerous navigation action (the
/// ones `ui.navigate` grants).
pub fn is_navigation(id: &str) -> bool {
    NAVIGATION.contains(&id) && lookup(id).is_some_and(|a| a.plugin_invocable && !a.dangerous)
}

/// The invocable ids, for `/api/plugins/features`.
pub fn invocable_ids() -> Vec<&'static str> {
    INVOCABLE.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_ids_are_invocable_and_never_ids_are_not() {
        for id in DANGEROUS {
            let a = lookup(id).unwrap_or_else(|| panic!("{id} missing"));
            assert!(a.dangerous && a.plugin_invocable, "{id}");
        }
        for id in NEVER {
            assert_eq!(
                lookup(id),
                Some(CoreAction {
                    dangerous: false,
                    plugin_invocable: false
                }),
                "{id}"
            );
        }
        assert_eq!(
            lookup("session.focus"),
            Some(CoreAction {
                dangerous: false,
                plugin_invocable: true
            })
        );
        // exact ids only: a name under a listed family is not invocable
        for id in ["overlay.open.queue", "focus.cycle.next"] {
            assert_eq!(lookup(id).map(|a| a.plugin_invocable), Some(true), "{id}");
        }
        for id in [
            "overlay.open.anything",
            "overlay.open.",
            "focus.cycle.sideways",
        ] {
            assert_eq!(lookup(id), None, "{id}");
        }
        assert_eq!(lookup("no.such.action"), None);
        // it is a real core id, so it is refused rather than "not a core action"
        assert_eq!(
            lookup("session.engage").map(|a| a.plugin_invocable),
            Some(false)
        );
        assert_eq!(lookup("usage-bar:details"), None);
    }

    #[test]
    fn navigation_is_a_small_safe_subset_of_the_invocable_ids() {
        for id in NAVIGATION {
            assert!(is_navigation(id), "{id}");
            assert!(!DANGEROUS.contains(id), "{id} must not be dangerous");
        }
        // Nothing that changes, sends or closes, nothing that moves where the
        // user types, and nothing unknown.
        for id in [
            "session.close",
            "session.restore",
            "session.spawn",
            "session.engage",
            "daemon.restart",
            "overlay.open.x",
            "overlay.open.help",
            "app.quit",
            "view.nothing",
            "x:y",
        ] {
            assert!(!is_navigation(id), "{id}");
        }
    }
}
