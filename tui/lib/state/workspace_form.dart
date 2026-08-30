/// Pure helpers for the `w` add-workspace overlay (spec tui-key-routing
/// "p8.1 session lifecycle bindings"): `~` expansion for the typed path and
/// the workspace-name derivation ported from the web UI
/// (`ui/src/components/AddWorkspaceForm.jsx` deriveName — basename →
/// lowercase → non-[a-z0-9] runs collapsed to `-` → trimmed; `-2`, `-3`, …
/// on collision). Pure so both unit-test without IO; the daemon stays the
/// source of truth (a wrong collision guess surfaces as its 409).
library;

/// Expand a leading `~` / `~/…` to [home]. Only the bare current-user forms
/// are expanded (`~other` is left alone — same as most shells' fallback
/// when the user is unknown). Trims surrounding whitespace.
String expandTilde(String path, String home) {
  final trimmed = path.trim();
  if (trimmed == '~') return home;
  if (trimmed.startsWith('~/')) return home + trimmed.substring(1);
  return trimmed;
}

/// Derive a workspace name from [dir]'s basename, suffixing `-2`, `-3`, …
/// against [existing] (case: the web UI's deriveName, byte-for-byte
/// semantics).
String deriveWorkspaceName(String dir, Iterable<String> existing) {
  final names = existing.toSet();
  final segments = dir.split('/').where((s) => s.isNotEmpty);
  final base = segments.isEmpty ? 'workspace' : segments.last;
  var slug = base
      .toLowerCase()
      .replaceAll(RegExp('[^a-z0-9]+'), '-')
      .replaceAll(RegExp(r'^-+|-+$'), '');
  if (slug.isEmpty) slug = 'workspace';
  if (!names.contains(slug)) return slug;
  var suffix = 2;
  while (names.contains('$slug-$suffix')) {
    suffix++;
  }
  return '$slug-$suffix';
}
