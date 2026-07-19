# App.jsx wiring — p4-layout-focus-workspace-ux (chrome half)

Everything below is additive to the existing `App.jsx`. No renames of
existing state/props — `groups` stays `groups`, `focusedSessionId` stays
`focusedSessionId`, so the keybinding handler, `focusedGroup` lookup, etc.
need zero changes beyond what's shown here.

## 1. Imports

```diff
-import { buildGroups } from "./lib/groups.js";
+import { buildGroupTree } from "./lib/groups.js";
+import SettingsPopover from "./components/SettingsPopover.jsx";
+import { useSettings } from "./lib/settings.js";
```

## 2. Swap `buildGroups` for `buildGroupTree`, keep the `groups` variable name

`buildGroupTree` returns `{ tree, flattenedGroups }` — `flattenedGroups` is
the exact array `buildGroups` used to return (needs-you-first, now also
nesting-aware and pre-order-flattened per design D-nesting), so aliasing it
to `groups` means every existing consumer (rail, grid, the `1`-`9`
keybinding's `groups[idx]`, `jumpToNeedsInput`, `cycleFocusedCell`,
`focusedGroup`) keeps working unmodified — they all index into the same
array, which is the load-bearing part of D-nesting ("what you see is what
you press").

```diff
-  const groups = useMemo(() => buildGroups(workspaces, sessions), [workspaces, sessions]);
+  const { flattenedGroups: groups } = useMemo(
+    () => buildGroupTree(workspaces, sessions),
+    [workspaces, sessions]
+  );
```

Each entry in `groups` now also carries `depth` (0 = top-level) and
`children` — `WorkspaceRail` uses `depth` for indentation; everything else
can ignore the extra fields.

## 3. Settings hook + gear button in the header

```diff
+  const [settings] = useSettings();
```

In the header, next to the existing "+ add workspace" button:

```diff
         <div className="relative ml-auto">
+          <SettingsPopover />
           <button
             type="button"
             onClick={() => setShowAddWorkspace((v) => !v)}
```
(`SettingsPopover` owns its own trigger + popover + open state, so this is
the only line needed — no App-owned toggle state, unlike
`AddWorkspaceForm`. Wrap the two in a `flex items-center gap-2` div if you
want visual spacing between the gear and "+ add workspace"; not required.)

## 4. Apply `focus-dim` on `<main>`

```diff
-    <main className="flex h-screen flex-col bg-garage-bg font-mono text-sm text-garage-ink">
+    <main
+      className={`flex h-screen flex-col bg-garage-bg font-mono text-sm text-garage-ink ${
+        settings.focusDim ? "focus-dim" : ""
+      }`}
+    >
```

That's the entire dimming trigger — `index.css` already defines what
`.focus-dim [data-dim-zone]` does; nothing else in `App.jsx` needs to know
about opacity.

## 5. `WorkspaceRail` — new optional prop

```diff
         <WorkspaceRail
           groups={groups}
           focusedWorkspace={focusedWorkspace}
           focusedSessionId={focusedSessionId}
           onSelectWorkspace={selectWorkspace}
           onSelectSession={selectSession}
           onSessionCreated={refreshSessions}
           onOpenRoot={openWorkspaceRoot}
           onBlurChrome={blurActiveTerminal}
           onSessionsRestored={refreshSessions}
+          onWorkspaceRenamed={() => {
+            refreshWorkspaces();
+            refreshSessions();
+          }}
         />
```

Why both refetches: a rename changes the registry key *and* every live
`garage/<old>/<label>` session id under it (the daemon's `PATCH` response
carries `renamedSessions`, but the simplest correct thing here is just to
refetch — `focusedWorkspace`/`focusedSessionId` self-correct via the
existing "keep focus valid as groups change" effects already in `App.jsx`,
since the old name/ids will no longer be present in the refetched data and
those effects already re-pick a valid focus).

## 6. `AddWorkspaceForm` — new prop for name-collision suffixing

```diff
             <AddWorkspaceForm
               onClose={() => setShowAddWorkspace(false)}
               onCreated={() => {
                 refreshWorkspaces();
                 setShowAddWorkspace(false);
               }}
+              existingNames={workspaces.map((w) => w.name)}
             />
```

Purely a client-side hint for deriving `-2`/`-3` suffixes on the
picker-derived name; the daemon's 409 is still the source of truth if the
guess is stale.

## 7. `TerminalGrid` zone-tagging convention (for the grid-cell owner, not applied here)

`TerminalGrid.jsx` is owned by the parallel agent, so this is a
description, not a diff. The CSS contract in `ui/src/index.css` is generic
— any element gets dimmed if it's `data-dim-zone` and doesn't carry
`dim-exempt`/`dim-focused`:

- Each grid cell's outer wrapper div (the one currently keyed by `s.id`,
  with the `border-garage-amber`/`border-garage-line` focus ring) gets
  `data-dim-zone`.
- Add `dim-exempt` when that cell's session `status === "needs-input"`.
- Add `dim-focused` when `s.id === focusedSessionId` — **note**:
  `focusedSessionId` already *is* the D-dim focus-target resolution.
  `TerminalGrid`'s own `onFocus={() => onFocusCell(s.id)}` means DOM focus
  on a terminal already wins and updates `focusedSessionId` before render,
  so "a terminal has DOM focus → that cell" and "else → the app-focused
  cell" (design D-dim) collapse into the single existing prop — no new
  state needed anywhere for this.
- The grid container itself is not a zone (same rule as the rail).
- Help overlay / review mode are full-page overlays with an opaque
  backdrop, so per design D-dim they don't need any special-case dimming
  logic — whatever's behind them can stay dimmed or not, it's covered
  either way.

## Net effect

- No existing prop/state is renamed or removed.
- `groups` keeps its shape as "the array the rail renders and the `1`-`9`
  keys index into" — just now nesting-aware.
- Total diff is small: 3 new imports, 1 hook call, 1 computation swap
  (destructure), 1 header line, 1 className template, 2 new props on
  existing components.
