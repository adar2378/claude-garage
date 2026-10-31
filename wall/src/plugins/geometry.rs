//! Reporting the slot geometry to the daemon (`PUT /api/plugins/ui/slots`) so
//! plugin run contexts carry `ui.slots`: only when it changed, and at most
//! every [`MIN_GAP`].

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::slots::SlotGeometry;

pub const MIN_GAP: Duration = Duration::from_millis(200);

/// First wait after a failed report; it doubles per further failure up to [`MAX_BACKOFF`].
pub const FIRST_BACKOFF: Duration = Duration::from_secs(1);
pub const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct GeometryReporter {
    last: Option<Value>,
    last_at: Option<Instant>,
    /// Failed reports in a row, and the earliest instant to try again.
    failures: u32,
    retry_at: Option<Instant>,
}

/// The wire form: `to_json` with the nulls taken out (`filler: null` means
/// "no room", which the daemon's schema spells as an absent key).
pub fn wire(geometry: &SlotGeometry) -> Value {
    fn strip(v: Value) -> Value {
        match v {
            Value::Object(map) => Value::Object(map.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, strip(v))).collect()),
            other => other,
        }
    }
    strip(geometry.to_json())
}

impl GeometryReporter {
    /// The body to send now, if the geometry differs from the last one sent
    /// and the gap since that send has passed. A change inside the gap is
    /// not lost: the next frame asks again.
    pub fn due(&mut self, geometry: &SlotGeometry, now: Instant) -> Option<Value> {
        let body = wire(geometry);
        if self.last.as_ref() == Some(&body) {
            return None;
        }
        if self.retry_at.is_some_and(|t| now < t) {
            return None;
        }
        if self.last_at.is_some_and(|t| now.saturating_duration_since(t) < MIN_GAP) {
            return None;
        }
        self.last = Some(body.clone());
        self.last_at = Some(now);
        Some(body)
    }

    /// The daemon did not take it (the call failed): send again, after a
    /// wait that doubles with each failure in a row. Returns how many failed
    /// in a row, so the caller can tell the user once.
    pub fn failed(&mut self, now: Instant) -> u32 {
        self.last = None;
        self.failures += 1;
        let wait = FIRST_BACKOFF.saturating_mul(1 << (self.failures - 1).min(5)).min(MAX_BACKOFF);
        self.retry_at = Some(now + wait);
        self.failures
    }

    /// The daemon took it: the backoff starts over.
    pub fn reported(&mut self) {
        self.failures = 0;
        self.retry_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_protocol::widgets::validate_slot_geometry;

    fn geo(width: u16, filler: Option<std::ops::Range<u16>>) -> SlotGeometry {
        SlotGeometry { strip_width: width, strip_filler: filler, tile_title_max: 20, tile_gutter_max: 1, rail_width: 24 }
    }

    #[test]
    fn what_we_send_passes_the_daemons_schema_with_and_without_a_filler() {
        for g in [geo(80, Some(10..30)), geo(80, None)] {
            validate_slot_geometry(&wire(&g)).unwrap();
        }
        assert!(wire(&geo(80, None))["strip.right"].get("filler").is_none());
    }

    #[test]
    fn it_sends_on_change_only_and_not_faster_than_the_gap() {
        let mut r = GeometryReporter::default();
        let t0 = Instant::now();
        assert!(r.due(&geo(80, None), t0).is_some(), "first report");
        assert!(r.due(&geo(80, None), t0 + Duration::from_secs(5)).is_none(), "unchanged: never again");
        assert!(r.due(&geo(100, None), t0 + Duration::from_millis(50)).is_none(), "changed inside the gap: held back");
        assert!(r.due(&geo(100, None), t0 + Duration::from_millis(250)).is_some(), "and sent once the gap passed");
        assert!(r.due(&geo(100, None), t0 + Duration::from_secs(9)).is_none());
        assert_eq!(r.failed(t0 + Duration::from_secs(10)), 1);
        assert!(r.due(&geo(100, None), t0 + Duration::from_millis(10_500)).is_none(), "but not before the backoff has passed");
        assert!(r.due(&geo(100, None), t0 + Duration::from_millis(11_100)).is_some(), "a failed send is retried");
    }

    #[test]
    fn repeated_failures_back_off_exponentially_to_a_cap_and_a_success_resets_it() {
        let mut r = GeometryReporter::default();
        let g = geo(80, None);
        let mut now = Instant::now();
        assert!(r.due(&g, now).is_some());
        for wait in [1u64, 2, 4, 8, 16, 30, 30] {
            r.failed(now);
            let just_before = now + Duration::from_secs(wait) - Duration::from_millis(50);
            assert!(r.due(&g, just_before).is_none(), "still waiting {wait} s");
            now += Duration::from_secs(wait);
            assert!(r.due(&g, now).is_some(), "retried after {wait} s");
        }
        r.reported();
        assert!(r.due(&geo(81, None), now + Duration::from_millis(300)).is_some(), "after a success there is no wait");
    }
}
