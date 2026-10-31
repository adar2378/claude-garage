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

/// Plugin-invocable ids (exact).
const INVOCABLE: &[&str] = &[
    "session.close",
    "session.delete",
    "session.focus",
    "session.engage",
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
];

/// Plugin-invocable families (`focus.cycle.*`, `overlay.open.*`).
const INVOCABLE_PREFIXES: &[&str] = &["focus.cycle.", "overlay.open."];

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

/// Real core ids a plugin must never trigger.
const NEVER: &[&str] = &[
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
    let invocable = INVOCABLE.contains(&id)
        || INVOCABLE_PREFIXES
            .iter()
            .any(|p| id.starts_with(p) && id.len() > p.len());
    if invocable {
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

/// The invocable exact ids, for `/api/plugin/features`.
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
        assert_eq!(
            lookup("overlay.open.anything").map(|a| a.plugin_invocable),
            Some(true)
        );
        assert_eq!(lookup("overlay.open."), None);
        assert_eq!(lookup("no.such.action"), None);
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
            "app.quit",
            "view.nothing",
            "x:y",
        ] {
            assert!(!is_navigation(id), "{id}");
        }
    }
}
