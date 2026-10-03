## ADDED Requirements

### Requirement: `I` installs hooks and the statusline together
Pressing `I` in the TUI SHALL call `POST /api/statusline/install` and then `POST /api/hooks/install`, and SHALL show one strip notice summarizing both results. A result reporting `alreadyInstalled` SHALL be described as already installed, not as an error. If one install fails, the notice SHALL name which one failed and its error, and the other result SHALL still be reported. While the installs are in flight, further `I` presses SHALL be ignored.

#### Scenario: Fresh install of both
- **WHEN** neither hooks nor the statusline are installed and the user presses `I`
- **THEN** both are installed and the strip shows a single success notice covering hooks and statusline

#### Scenario: Hooks already installed
- **WHEN** hooks are already present in `~/.claude/settings.json` and the user presses `I`
- **THEN** the notice reports hooks as already installed and the statusline result as usual

#### Scenario: Corrupt settings file
- **WHEN** `~/.claude/settings.json` is not valid JSON and the user presses `I`
- **THEN** the notice names the failure and its error, and the file is left unchanged

### Requirement: Install hint mentions hooks
The TUI's existing one-time install hint SHALL describe `I` as installing hooks and the context meter, using the existing trigger (no session has statusline-sourced context after 30s).

#### Scenario: Hint text
- **WHEN** the install hint is shown
- **THEN** its text says `I` installs hooks (instant status) and the context meter
