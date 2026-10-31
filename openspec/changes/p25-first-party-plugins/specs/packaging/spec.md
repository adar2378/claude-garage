## ADDED Requirements

### Requirement: Quiet startup
The launcher SHALL print only errors and essential prompts on a normal start, and SHALL print full startup detail when `GARAGE_VERBOSE=1` is set.

#### Scenario: Quiet by default
- **WHEN** the user runs `claude-garage` with a healthy daemon
- **THEN** no startup log lines are printed before the wall opens
