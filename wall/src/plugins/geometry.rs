//! Reporting the slot geometry to the daemon (`PUT /api/plugins/ui/slots`) so
//! plugin run contexts carry `ui.slots`: only when it changed, and at most
//! every [`MIN_GAP`].

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::slots::SlotGeometry;

pub const MIN_GAP: Duration = Duration::from_millis(200);

#[derive(Default)]
pub struct GeometryReporter {
    last: Option<Value>,
    last_at: Option<Instant>,
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
        if self.last_at.is_some_and(|t| now.saturating_duration_since(t) < MIN_GAP) {
            return None;
        }
        self.last = Some(body.clone());
        self.last_at = Some(now);
        Some(body)
    }

    /// The daemon did not take it (the call failed): send again next time.
    pub fn failed(&mut self) {
        self.last = None;
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
        r.failed();
        assert!(r.due(&geo(100, None), t0 + Duration::from_secs(10)).is_some(), "a failed send is retried");
    }
}
