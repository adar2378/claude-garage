// Workspace-add helpers (tui/lib/state/workspace_form.dart): `~` expansion
// and the web-UI-parity name derivation (basename → slug → -2/-3 collision
// suffix; mirrors ui/src/components/AddWorkspaceForm.jsx deriveName).
import 'package:garage_tui/state/workspace_form.dart';
import 'package:test/test.dart';

void main() {
  group('expandTilde', () {
    const home = '/Users/me';

    test('bare ~ expands to home', () {
      expect(expandTilde('~', home), home);
    });

    test('~/path expands', () {
      expect(expandTilde('~/dev/proj', home), '/Users/me/dev/proj');
    });

    test('absolute paths pass through', () {
      expect(expandTilde('/opt/x', home), '/opt/x');
    });

    test('~otheruser is left alone (unknown-user shell fallback)', () {
      expect(expandTilde('~bob/x', home), '~bob/x');
    });

    test('surrounding whitespace is trimmed', () {
      expect(expandTilde('  ~/x  ', home), '/Users/me/x');
    });

    test('a mid-path tilde is not expanded', () {
      expect(expandTilde('/a/~/b', home), '/a/~/b');
    });
  });

  group('deriveWorkspaceName', () {
    test('basename, lowercased', () {
      expect(deriveWorkspaceName('/Users/me/dev/MyProj', const []), 'myproj');
    });

    test('non-alphanumeric runs collapse to a single dash', () {
      expect(deriveWorkspaceName('/x/My Cool_App!!', const []), 'my-cool-app');
    });

    test('leading/trailing dashes are trimmed', () {
      expect(deriveWorkspaceName('/x/--weird--', const []), 'weird');
    });

    test('trailing slash still finds the basename', () {
      expect(deriveWorkspaceName('/a/b/proj/', const []), 'proj');
    });

    test('an unsluggable basename falls back to "workspace"', () {
      expect(deriveWorkspaceName('/x/###', const []), 'workspace');
    });

    test('empty dir falls back to "workspace"', () {
      expect(deriveWorkspaceName('', const []), 'workspace');
    });

    test('collision appends -2, then -3, …', () {
      expect(deriveWorkspaceName('/x/proj', const ['proj']), 'proj-2');
      expect(
          deriveWorkspaceName('/x/proj', const ['proj', 'proj-2']), 'proj-3');
    });

    test('no collision means no suffix even when -2 exists', () {
      expect(deriveWorkspaceName('/x/proj', const ['proj-2']), 'proj');
    });
  });
}
