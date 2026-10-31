//! The widget store (design section 5): content plugins push for their
//! declared widgets. In memory only. Replace-only (a PUT is the whole content),
//! equal pushes are no-ops, changes are coalesced into at most one
//! `WidgetsChanged` per plugin per 200 ms, and a `ttl_secs` widget expires.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use garage_protocol::widgets::{limits, WidgetItem, WidgetPut, WidgetSlot};

use crate::bus::{DaemonEvent, EventBus};

pub const COALESCE: Duration = Duration::from_millis(200);

/// Token bucket: `PUTS_PER_SEC` refill, `PUT_BURST` capacity.
#[derive(Debug)]
pub struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    pub fn new(now: Instant) -> Self {
        Self { tokens: f64::from(limits::PUT_BURST), at: now }
    }

    /// Take one token, or say how long until one exists.
    pub fn take(&mut self, now: Instant) -> Result<(), Duration> {
        let rate = f64::from(limits::PUTS_PER_SEC);
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * rate).min(f64::from(limits::PUT_BURST));
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - self.tokens) / rate))
        }
    }
}

/// One widget's stored content.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredWidget {
    pub slot: WidgetSlot,
    pub scopes: BTreeMap<String, Vec<WidgetItem>>,
    /// Expiry instant of the current content, if it has a ttl.
    pub deadline: Option<Instant>,
}

#[derive(Default)]
struct PluginWidgets {
    rev: u64,
    widgets: HashMap<String, StoredWidget>,
    bucket: Option<Bucket>,
    last_emit: Option<Instant>,
    emit_pending: bool,
}

/// What a push did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    Changed { rev: u64 },
    /// Same content as stored (a ttl heartbeat may still have been renewed).
    Unchanged { rev: u64 },
}

#[derive(Clone)]
pub struct WidgetStore {
    inner: Arc<Mutex<HashMap<String, PluginWidgets>>>,
    bus: EventBus,
    coalesce: Duration,
}

impl WidgetStore {
    pub fn new(bus: EventBus) -> Self {
        Self { inner: Arc::default(), bus, coalesce: COALESCE }
    }

    #[cfg(test)]
    pub fn with_coalesce(mut self, d: Duration) -> Self {
        self.coalesce = d;
        self
    }

    /// Rate limit one PUT attempt of `plugin`.
    pub fn acquire(&self, plugin: &str, now: Instant) -> Result<(), Duration> {
        let mut map = self.inner.lock().unwrap();
        map.entry(plugin.to_owned()).or_default().bucket.get_or_insert_with(|| Bucket::new(now)).take(now)
    }

    pub fn rev(&self, plugin: &str) -> u64 {
        self.inner.lock().unwrap().get(plugin).map_or(0, |p| p.rev)
    }

    /// Stored widgets of one plugin (clone), for the snapshot route.
    pub fn widgets(&self, plugin: &str) -> HashMap<String, StoredWidget> {
        self.inner.lock().unwrap().get(plugin).map(|p| p.widgets.clone()).unwrap_or_default()
    }

    pub fn put(&self, plugin: &str, widget: &str, slot: WidgetSlot, put: WidgetPut) -> PutOutcome {
        let ttl = put.ttl_secs.map(|s| Duration::from_secs(u64::from(s)));
        let deadline = ttl.map(|t| Instant::now() + t);
        let mut map = self.inner.lock().unwrap();
        let p = map.entry(plugin.to_owned()).or_default();
        let same = p.widgets.get(widget).is_some_and(|w| w.slot == slot && w.scopes == put.scopes);
        if same {
            // A heartbeat re-push renews the ttl without an event.
            if let (Some(w), Some(d)) = (p.widgets.get_mut(widget), deadline) {
                w.deadline = Some(d);
            }
            let rev = p.rev;
            drop(map);
            if let (Some(deadline), true) = (deadline, ttl.is_some()) {
                self.arm_expiry(plugin, widget, deadline);
            }
            return PutOutcome::Unchanged { rev };
        }
        p.widgets.insert(widget.to_owned(), StoredWidget { slot, scopes: put.scopes, deadline });
        p.rev += 1;
        let rev = p.rev;
        self.announce(&mut map, plugin);
        drop(map);
        if let Some(deadline) = deadline {
            self.arm_expiry(plugin, widget, deadline);
        }
        PutOutcome::Changed { rev }
    }

    /// Clear one widget. `true` when it held content.
    pub fn clear_widget(&self, plugin: &str, widget: &str) -> bool {
        let mut map = self.inner.lock().unwrap();
        let Some(p) = map.get_mut(plugin) else { return false };
        if p.widgets.remove(widget).is_none() {
            return false;
        }
        p.rev += 1;
        self.announce(&mut map, plugin);
        true
    }

    /// Drop everything a plugin showed (disable, remove).
    pub fn clear_plugin(&self, plugin: &str) -> bool {
        let mut map = self.inner.lock().unwrap();
        let Some(p) = map.get_mut(plugin) else { return false };
        if p.widgets.is_empty() {
            return false;
        }
        p.widgets.clear();
        p.rev += 1;
        self.announce(&mut map, plugin);
        true
    }

    /// Forget a plugin's widgets that no longer match its manifest (a reload
    /// removed or re-slotted them).
    pub fn retain_declared(&self, plugin: &str, declared: &HashMap<String, WidgetSlot>) {
        let mut map = self.inner.lock().unwrap();
        let Some(p) = map.get_mut(plugin) else { return };
        let before = p.widgets.len();
        p.widgets.retain(|id, w| declared.get(id) == Some(&w.slot));
        if p.widgets.len() != before {
            p.rev += 1;
            self.announce(&mut map, plugin);
        }
    }

    /// Publish now if the last frame was long enough ago, otherwise schedule
    /// one trailing frame: at most one `WidgetsChanged` per plugin per window.
    fn announce(&self, map: &mut HashMap<String, PluginWidgets>, plugin: &str) {
        let p = map.get_mut(plugin).expect("announced plugin exists");
        let now = Instant::now();
        let wait = match p.last_emit {
            Some(at) if now.duration_since(at) < self.coalesce => Some(self.coalesce - now.duration_since(at)),
            _ => None,
        };
        match wait {
            None => {
                p.last_emit = Some(now);
                self.bus.publish(DaemonEvent::WidgetsChanged { plugin: plugin.to_owned(), rev: p.rev });
            }
            Some(_) if p.emit_pending => {} // the trailing frame will carry the newest rev
            Some(wait) => {
                p.emit_pending = true;
                let (this, plugin) = (self.clone(), plugin.to_owned());
                tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    let mut map = this.inner.lock().unwrap();
                    if let Some(p) = map.get_mut(&plugin) {
                        p.emit_pending = false;
                        p.last_emit = Some(Instant::now());
                        this.bus.publish(DaemonEvent::WidgetsChanged { plugin: plugin.clone(), rev: p.rev });
                    }
                });
            }
        }
    }

    fn arm_expiry(&self, plugin: &str, widget: &str, deadline: Instant) {
        let (this, plugin, widget) = (self.clone(), plugin.to_owned(), widget.to_owned());
        tokio::spawn(async move {
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            let mut map = this.inner.lock().unwrap();
            let Some(p) = map.get_mut(&plugin) else { return };
            // Only the push that armed this timer may expire it; a renewed or
            // replaced widget has a later deadline.
            if p.widgets.get(&widget).and_then(|w| w.deadline) != Some(deadline) {
                return;
            }
            p.widgets.remove(&widget);
            p.rev += 1;
            this.announce(&mut map, &plugin);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_protocol::widgets::Segment;

    fn put(text: &str, ttl: Option<u32>) -> WidgetPut {
        let item = WidgetItem {
            segments: Some(vec![Segment { text: text.into(), style: None, click: None }]),
            ..Default::default()
        };
        WidgetPut { scopes: BTreeMap::from([("*".to_owned(), vec![item])]), ttl_secs: ttl }
    }

    fn store() -> (WidgetStore, tokio::sync::broadcast::Receiver<DaemonEvent>) {
        let bus = EventBus::default();
        let rx = bus.subscribe();
        (WidgetStore::new(bus), rx)
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<DaemonEvent>) -> Vec<(String, u64)> {
        let mut out = vec![];
        while let Ok(DaemonEvent::WidgetsChanged { plugin, rev }) = rx.try_recv() {
            out.push((plugin, rev));
        }
        out
    }

    #[test]
    fn the_bucket_allows_a_burst_of_20_then_refills_10_per_second() {
        let t0 = Instant::now();
        let mut b = Bucket::new(t0);
        for i in 0..20 {
            assert!(b.take(t0).is_ok(), "burst token {i}");
        }
        let wait = b.take(t0).unwrap_err();
        assert!(wait > Duration::from_millis(90) && wait <= Duration::from_millis(100), "{wait:?}");
        assert!(b.take(t0 + Duration::from_millis(100)).is_ok(), "one token after 100 ms");
        assert!(b.take(t0 + Duration::from_millis(100)).is_err());
        // a long pause never banks more than the burst
        let later = t0 + Duration::from_secs(60);
        for _ in 0..20 {
            assert!(b.take(later).is_ok());
        }
        assert!(b.take(later).is_err());
    }

    #[tokio::test]
    async fn a_put_replaces_content_and_bumps_the_rev_but_equal_content_is_a_no_op() {
        let (s, mut rx) = store();
        let slot = WidgetSlot::StripRight;
        assert_eq!(s.put("p", "bar", slot, put("a", None)), PutOutcome::Changed { rev: 1 });
        assert_eq!(s.put("p", "bar", slot, put("a", None)), PutOutcome::Unchanged { rev: 1 });
        assert_eq!(s.put("p", "bar", slot, put("b", None)), PutOutcome::Changed { rev: 2 });
        assert_eq!(s.widgets("p")["bar"].scopes["*"][0].segments.as_ref().unwrap()[0].text, "b");
        assert_eq!(s.rev("p"), 2);
        assert_eq!(s.rev("other"), 0, "revs are per plugin");
        // first change publishes at once; the second (inside 200 ms) trails
        assert_eq!(drain(&mut rx), [("p".to_string(), 1)]);
    }

    #[tokio::test]
    async fn changes_inside_the_window_coalesce_into_one_trailing_frame_with_the_newest_rev() {
        let (s, mut rx) = store();
        let s = s.with_coalesce(Duration::from_millis(120));
        for i in 0..6 {
            s.put("p", "bar", WidgetSlot::StripRight, put(&format!("v{i}"), None));
        }
        assert_eq!(drain(&mut rx), [("p".to_string(), 1)], "leading frame only, so far");
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(drain(&mut rx), [("p".to_string(), 6)], "one trailing frame carrying rev 6");
        // a quiet period later, the next change publishes immediately again
        s.put("p", "bar", WidgetSlot::StripRight, put("later", None));
        assert_eq!(drain(&mut rx), [("p".to_string(), 7)]);
    }

    #[tokio::test]
    async fn clear_widget_and_clear_plugin_bump_the_rev_only_when_something_was_removed() {
        let (s, _rx) = store();
        s.put("p", "a", WidgetSlot::StripRight, put("x", None));
        s.put("p", "b", WidgetSlot::StripRight, put("y", None));
        assert!(!s.clear_widget("p", "zzz"));
        assert!(s.clear_widget("p", "a"));
        let rev = s.rev("p");
        assert!(s.clear_plugin("p"));
        assert_eq!(s.rev("p"), rev + 1);
        assert!(!s.clear_plugin("p"), "nothing left to clear");
        assert!(!s.clear_plugin("never-seen"));
    }

    #[tokio::test]
    async fn a_ttl_widget_expires_but_a_heartbeat_push_renews_it_without_an_event() {
        let (s, mut rx) = store();
        let s = s.with_coalesce(Duration::from_millis(10));
        s.put("p", "bar", WidgetSlot::StripRight, put("x", Some(1)));
        tokio::time::sleep(Duration::from_millis(700)).await;
        // renew at 0.7 s: same content, so no new rev, but the clock restarts
        assert_eq!(s.put("p", "bar", WidgetSlot::StripRight, put("x", Some(1))), PutOutcome::Unchanged { rev: 1 });
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(s.widgets("p").contains_key("bar"), "alive at 1.4 s because it was renewed");
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(!s.widgets("p").contains_key("bar"), "expired at ~1.7 s");
        assert_eq!(s.rev("p"), 2, "expiry is a change");
        let frames = drain(&mut rx);
        assert_eq!(frames.last(), Some(&("p".to_string(), 2)), "{frames:?}");
    }

    #[tokio::test]
    async fn retain_declared_drops_widgets_a_reload_removed_or_moved() {
        let (s, _rx) = store();
        s.put("p", "keep", WidgetSlot::StripRight, put("x", None));
        s.put("p", "gone", WidgetSlot::StripRight, put("x", None));
        s.put("p", "moved", WidgetSlot::StripRight, put("x", None));
        let declared = HashMap::from([("keep".to_owned(), WidgetSlot::StripRight), ("moved".to_owned(), WidgetSlot::RailRow)]);
        s.retain_declared("p", &declared);
        assert_eq!(s.widgets("p").keys().cloned().collect::<Vec<_>>(), ["keep"]);
    }
}
