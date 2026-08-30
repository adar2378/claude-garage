---
title: "Why Every Project Deserves a TUI"
date: 2026-01-27
description: "Dev tools don't have to be scattered scripts and forgotten commands. Build a TUI for your team."
author: "Norbert Kozsir"
tags: ["dev-tools", "tui", "dx", "flutter"]
---

I keep running into this problem: I come back to a project after a few days, sometimes a few weeks, and I'm staring at the codebase thinking "how do I even start this thing?"

Maybe there's a README with setup instructions. Maybe there's a `justfile` with some commands. But I still need to remember the order: run this first, then that, wait for this to finish, then start the other thing. And if the setup isn't documented well? I end up spelunking through shell history or asking teammates what the magic incantation is.

I've been there too many times. So here's how I think about solutions.

## Good: Documentation

The baseline. A README with setup instructions, maybe a `CONTRIBUTING.md` that explains how to run tests. It works, but it requires reading and remembering. Documentation also gets outdated. The project evolves, someone adds a new step, forgets to update the docs, and suddenly the setup instructions don't work anymore.

Still, having *something* written down is way better than having nothing.

## Better: Task runners

This is where things improve. Tools like `just` or `make` let me codify commands. Instead of remembering `flutter build apk --release --split-per-abi --target-platform android-arm64`, I just run `just build`.

I use `just` constantly. Write a `justfile`, and `just --list` shows everything:

```
Available recipes:
    build     # Build the release APK
    clean     # Clean build artifacts
    format    # Format all code
    test      # Run all tests
```

New teammate joins? "Run `just` to see what's available." That's huge for discovery.

## Even better: Interactive dashboards

Task runners solve the "what commands exist" problem. But they're non-interactive. Run a command, it exits. What about:

- Toggling feature flags without restarting the app
- Logging in as different test users with one keypress
- Triggering a specific push notification to your device
- Resetting onboarding state to test the first-run experience
- Switching between API environments (local, staging, prod)
- Clearing caches, seeding test data, or resetting database state

These are the things I actually do ten times a day during development. And they're annoying to do through code changes or CLI commands.

This is where I find TUI dashboards interesting. Something that treats developers a bit like customers. Shows what's available, what's running, what can be done next. A bit of handholding for when I come back after two weeks and my brain has completely context-switched away.

Tools like `lazygit`, `k9s`, and `btop` are popular for exactly this reason. Keyboard-first, fast, and I never have to leave my terminal.

## Why I like Dart for this

For Flutter projects, Dart is a natural choice. I can share models, API clients, whatever. No need to rewrite stuff in another language just for a dev tool.

I've been building [Nocterm](https://nocterm.dev), a TUI framework that uses Flutter-like APIs. If I know Flutter, I already know how to build with it.

```dart
class MyDashboard extends StatefulComponent {
  @override
  State<MyDashboard> createState() => _MyDashboardState();
}

class _MyDashboardState extends State<MyDashboard> {
  String _status = 'Ready';

  @override
  Component build(BuildContext context) {
    return Column(
      children: [
        Text('App Status: $_status'),
        Button(
          onPressed: () => setState(() => _status = 'Running'),
          child: Text('Start'),
        ),
      ],
    );
  }
}
```

`StatefulComponent`, `setState()`, `Column`, `Row`. It's all there. Hot reload works too, which makes iterating fast. And I can compile to a native executable that starts instantly.

## A practical example: the dev dashboard

Here's what I mean in practice. A dev dashboard that combines commands, logs, and service extensions in one place:

```dart
void main() async {
  await runApp(const DevDashboard());
}

class DevDashboard extends StatefulComponent {
  // ... state management

  @override
  Component build(BuildContext context) {
    return Focusable(
      onKeyEvent: _handleKeyEvent,
      child: Column(
        children: [
          _buildHeader(),
          _buildStatusBar(),  // Live metrics: users, requests/sec, memory
          _buildTabBar(),     // Commands | Logs | Service Extensions
          Expanded(child: _buildContent()),
          _buildFooter(),     // Keyboard shortcuts
        ],
      ),
    );
  }
}
```

The sidebar has categories like "App Control", "Environment", "Feature Flags", "Debug Tools". The main panel shows what's in each category. Some examples:

- **Environment** - Switch between local, staging, production APIs. See the current endpoint at a glance.
- **Feature Flags** - Toggle "Dark Mode", "New Checkout Flow", "Beta Features" on and off. No app restart needed.
- **Test Users** - Log in as "free_user", "premium_user", "admin", or "expired_trial" with one keypress.
- **Debug Tools** - Clear cache, reset onboarding, seed test data, trigger a test crash.

For Flutter apps, service extensions make this possible. I define them in my app:

```dart
// In the Flutter app
developer.registerExtension('ext.myapp.clearCache', (method, params) async {
  await CacheService.clear();
  return developer.ServiceExtensionResponse.result('{}');
});
```

And the TUI calls them via the VM service:

```dart
// In the TUI
Future<void> _callExtension(String name) async {
  final vmService = await vmServiceConnectUri('ws://127.0.0.1:5432/ws');
  final vm = await vmService.getVM();
  await vmService.callServiceExtension(name, isolateId: vm.isolates!.first.id);
}
```

If the TUI itself starts the Flutter app (spawning the process with the right flags and environment variables), it can grab the VM service URI directly from stdout. No copy-pasting URLs. Press `[R]` to run, the app starts, the TUI connects automatically, and service extensions are ready.

I've been thinking about packaging this into a reusable library. Something that handles the Flutter process lifecycle and VM service connection. Maybe for a future release.

Now the whole team has instant access to these tools. QA person needs to test the premium flow? They don't need to ask for test credentials or figure out how to toggle a feature flag in code. They open the dashboard, switch to "premium_user", enable "New Checkout Flow", and they're testing.

## Keep the CLI escape hatch

TUIs are great, but I still want traditional CLI support. CI/CD needs non-interactive commands. Sometimes I just want to run `myapp build --release` without navigating a UI.

The pattern I use:

```dart
void main(List<String> args) async {
  if (args.isNotEmpty) {
    // Non-interactive mode
    await runHeadless(args);
  } else {
    // TUI mode
    await runApp(const DevDashboard());
  }
}
```

TUI for daily development, raw CLI for scripts and automation.

## Let AI help

We're in the age of AI writing code, and TUIs are a good fit for that. The structure is declarative, patterns are repetitive, and if it's something Flutter-like, AI models already understand the concepts.

I find it works well to describe the dashboard I want, point at some docs, and let AI generate a draft. I'll tweak it, but I get something working faster than starting from scratch.

## Wrapping up

Dev tools should take mental load off, not add to it. A TUI that shows what's available, what's running, what you can do next. That's what makes coming back to a project after two weeks actually pleasant.

It doesn't need to replace the `justfile` or `Makefile`. I think of it as a layer on top. The TUI can call those same commands, but with context and visibility.

---

The example dev dashboard from this post is in the [Nocterm repo](https://github.com/Norbert515/nocterm) under `example/dev_dashboard_demo.dart` if you want to poke around.
