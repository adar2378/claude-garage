//! Plugin API v1 marks some read routes "accepts a token now; tokenless access
//! is deprecated and may be removed". An SDK must therefore send the plugin
//! token on every call, reads included, so a later garage that requires it
//! breaks no plugin.

#![cfg(feature = "testing")]

use garage_plugin_sdk::testing::FakeDaemon;

#[test]
fn every_read_call_carries_the_plugin_token() {
    let fake = FakeDaemon::start(|_| (200, "[]".to_owned()));
    let api = fake.api("hello");
    // answers that do not parse as the typed shapes are fine: only the request matters
    let _ = api.health();
    let _ = api.features();
    let _ = api.sessions();
    let _ = api.workspaces();
    let _ = api.usage();
    let _ = api.closed_chats("repo");
    let _ = api.plugins();
    let _ = api.widgets();
    let _ = api.panes();
    let _ = api.get("/api/events");

    let seen = fake.requests();
    assert_eq!(seen.len(), 10, "{:?}", fake.log());
    for r in &seen {
        assert_eq!(
            r.authorization.as_deref(),
            Some("Bearer tok"),
            "{} {} went out without the plugin token",
            r.method,
            r.path
        );
    }
    let paths: Vec<&str> = seen.iter().map(|r| r.path.as_str()).collect();
    for tokenless_today in [
        "/api/sessions",
        "/api/workspaces",
        "/api/usage",
        "/api/workspaces/repo/closed",
        "/api/plugins/widgets",
        "/api/events",
    ] {
        assert!(paths.contains(&tokenless_today), "{tokenless_today}");
    }
}
