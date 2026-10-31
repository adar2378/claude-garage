# Security model

**Plugins are not sandboxed.** A plugin is a program that runs as you. It can read, write and delete anything you can. It can use the network. It can read your SSH keys. Install a plugin only if you trust its author, the same way you would run any program from the internet.

garage cannot make an untrusted plugin safe. It can make four things less likely to go wrong by accident. This page says what those are, and what they are not.

## What garage does

**Before install**

- `plugin install` shows a preview first: the name, the source and commit, the permissions (dangerous ones marked), and **every command** the plugin will run, as an argument list. Nothing from the plugin has run yet. For a plugin that builds, the build commands are shown too.
- `--yes` skips the question, not the preview. A dangerous permission also needs `--allow-dangerous`, so a script cannot grant it by accident.
- An update asks again if commands, permissions or download addresses changed. It does not ask when only code changed.
- A downloaded binary needs a checksum file, and garage checks it.
- Symlinks that leave the checkout are refused.

**When it runs**

- No shell. Commands are argument lists, so text from the context cannot become a shell command.
- A cleared environment. A plugin gets only the variables listed in [api](api.md#environment). It does not see your API keys.
- A per-plugin token. The daemon checks the token and the [permission](permissions.md) on each call, and `403` names the permission that is missing.
- The daemon only listens on `127.0.0.1`, checks the `Host` header, and refuses state-changing requests that carry a browser `Origin`. A web page cannot change anything.
- Timeouts: a run that takes too long is killed with its whole process tree.
- Rate limits on widget pushes, notices and UI actions. Size limits on widget bodies. Text from plugins is cleaned of control and direction-changing characters so it cannot hide or reorder what the user reads.
- Event hooks that fail five times in a row are switched off.

**For the dangerous things**

- The [confirm rule](actions.md#the-confirm-rule): a plugin can ask to close or delete a session, but only the user can confirm.
- `sessions.input` has the `if_status` guard, and a plugin cannot move where the user types.
- Permissions that can hurt (`sessions.input`, `core.actions`) are marked and need an explicit extra flag.

## What garage does not do

- It does not stop a plugin from using a file, the network or another program. `permissions` is a declaration that garage shows to you and checks on its own API. It is **not** a boundary.
- The token is not a secret from other programs running as you. Your user can read the plugin's token file.
- Core routes outside the [plugin API](api.md#routes) (for example creating a session) are not gated per plugin. A malicious program running as you can call them without being a plugin at all.
- It does not sign plugins, review them or keep a list of approved ones. Finding a plugin by topic on GitHub is not a recommendation.
- It does not scan what a plugin does after install. A plugin you installed from a tag could change what it downloads later. The update prompt only covers what the manifest declares.

## Advice

For users:

- Read the preview. Look at the commands, not only the name.
- Prefer a pinned tag or commit: `plugin install owner/repo --ref v1.2.0`.
- Be careful with `sessions.input` and `core.actions`.
- `plugin disable <id>` stops a plugin at once. `plugin remove <id> --purge` deletes it and its data.

For authors:

- Ask for the least permission you need.
- Do not run `sh -c` with text you did not write. Pass arguments as an array.
- Do not print the token. Logs are readable by the user, and bug reports get pasted.
- Be honest in `description`: say what the plugin does with the network.
- Pin your dependencies and publish checksums for binaries (`[release]`).

## Reporting a problem

If you find a way for a plugin to do more than the rules above say (for example to confirm a dangerous action itself), report it to the garage maintainers as a security issue, not a public bug.
