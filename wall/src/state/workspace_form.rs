//! Pure helpers for the `w` add-workspace overlay (port of
//! `tui/lib/state/workspace_form.dart` — spec tui-key-routing "p8.1 session
//! lifecycle bindings"): `~` expansion for the typed path and the
//! workspace-name derivation ported from the web UI
//! (`ui/src/components/AddWorkspaceForm.jsx` deriveName — basename →
//! lowercase → non-[a-z0-9] runs collapsed to `-` → trimmed; `-2`, `-3`, …
//! on collision). Pure so both unit-test without IO; the daemon stays the
//! source of truth (a wrong collision guess surfaces as its 409).

use std::collections::HashSet;

/// Expand a leading `~` / `~/…` to `home`. Only the bare current-user forms
/// are expanded (`~other` is left alone — same as most shells' fallback
/// when the user is unknown). Trims surrounding whitespace.
pub fn expand_tilde(path: &str, home: &str) -> String {
    let trimmed = path.trim();
    if trimmed == "~" {
        return home.to_owned();
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return format!("{home}/{rest}");
    }
    trimmed.to_owned()
}

/// Derive a workspace name from `dir`'s basename, suffixing `-2`, `-3`, …
/// against `existing` (case: the web UI's deriveName, byte-for-byte
/// semantics).
pub fn derive_workspace_name<S: AsRef<str>>(dir: &str, existing: &[S]) -> String {
    let names: HashSet<&str> = existing.iter().map(AsRef::as_ref).collect();
    let base = dir
        .split('/')
        .rfind(|s| !s.is_empty())
        .unwrap_or("workspace");

    // lowercase → non-[a-z0-9] runs collapsed to '-' → trimmed of dashes.
    let mut slug = String::with_capacity(base.len());
    let mut pending_dash = false;
    for c in base.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(c);
        } else {
            pending_dash = true;
        }
    }
    if slug.is_empty() {
        slug = "workspace".to_owned();
    }

    if !names.contains(slug.as_str()) {
        return slug;
    }
    let mut suffix = 2u64;
    while names.contains(format!("{slug}-{suffix}").as_str()) {
        suffix += 1;
    }
    format!("{slug}-{suffix}")
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/workspace_form_test.dart`.
    use super::*;

    const HOME: &str = "/Users/me";
    const NONE: &[&str] = &[];

    // ── expandTilde ─────────────────────────────────────────────────────

    #[test]
    fn bare_tilde_expands_to_home() {
        assert_eq!(expand_tilde("~", HOME), HOME);
    }

    #[test]
    fn tilde_slash_path_expands() {
        assert_eq!(expand_tilde("~/dev/proj", HOME), "/Users/me/dev/proj");
    }

    #[test]
    fn absolute_paths_pass_through() {
        assert_eq!(expand_tilde("/opt/x", HOME), "/opt/x");
    }

    #[test]
    fn tilde_otheruser_is_left_alone_unknown_user_shell_fallback() {
        assert_eq!(expand_tilde("~bob/x", HOME), "~bob/x");
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(expand_tilde("  ~/x  ", HOME), "/Users/me/x");
    }

    #[test]
    fn a_mid_path_tilde_is_not_expanded() {
        assert_eq!(expand_tilde("/a/~/b", HOME), "/a/~/b");
    }

    // ── deriveWorkspaceName ─────────────────────────────────────────────

    #[test]
    fn basename_lowercased() {
        assert_eq!(derive_workspace_name("/Users/me/dev/MyProj", NONE), "myproj");
    }

    #[test]
    fn non_alphanumeric_runs_collapse_to_a_single_dash() {
        assert_eq!(derive_workspace_name("/x/My Cool_App!!", NONE), "my-cool-app");
    }

    #[test]
    fn leading_trailing_dashes_are_trimmed() {
        assert_eq!(derive_workspace_name("/x/--weird--", NONE), "weird");
    }

    #[test]
    fn trailing_slash_still_finds_the_basename() {
        assert_eq!(derive_workspace_name("/a/b/proj/", NONE), "proj");
    }

    #[test]
    fn an_unsluggable_basename_falls_back_to_workspace() {
        assert_eq!(derive_workspace_name("/x/###", NONE), "workspace");
    }

    #[test]
    fn empty_dir_falls_back_to_workspace() {
        assert_eq!(derive_workspace_name("", NONE), "workspace");
    }

    #[test]
    fn collision_appends_2_then_3() {
        assert_eq!(derive_workspace_name("/x/proj", &["proj"]), "proj-2");
        assert_eq!(derive_workspace_name("/x/proj", &["proj", "proj-2"]), "proj-3");
    }

    #[test]
    fn no_collision_means_no_suffix_even_when_2_exists() {
        assert_eq!(derive_workspace_name("/x/proj", &["proj-2"]), "proj");
    }
}
