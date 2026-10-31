//! usage-bar: the account's 5-hour and weekly usage as a dim `5h 42% · wk 10%`
//! chip at the right of the status strip.
//!
//! The figures come from `GET /api/usage` (the daemon keeps the latest
//! statusline report in memory). The plugin also writes them to its own
//! storage directory, so after a daemon restart the chip is drawn again at
//! once instead of staying empty until Claude Code next reports.

use garage_plugin_sdk::{
    Api, Error, Segment, Style, Value, WidgetItem, WidgetPut, json, read_state_in, write_state_in,
};
use serde::{Deserialize, Serialize};

pub const WIDGET: &str = "usage";
const STORE: &str = "usage.json";
/// Where the chip sat among `strip.right` items when it was a core provider.
const PRIORITY: i32 = 50;
/// How long a figure with no reset time stays drawn after it was last seen to
/// change: with nothing to say when it stops being true, it is only trusted
/// for a while. (15 minutes.)
pub const NO_RESET_TTL_SECS: i64 = 15 * 60;

/// One rate-limit window as the plugin keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// 0..=100, rounded the way the chip always showed it.
    pub pct: u32,
    /// When the window resets, whole seconds since the epoch, when known.
    pub resets_at: Option<i64>,
    /// When this exact figure was first seen, whole seconds since the epoch.
    /// Set by [`choose`]; what a window with no reset time expires from.
    #[serde(default)]
    pub seen_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

fn window(v: Option<&Value>) -> Option<Window> {
    let v = v.filter(|v| !v.is_null())?;
    let pct = v.get("usedPercentage")?.as_f64()?.clamp(0.0, 100.0).round() as u32;
    Some(Window {
        pct,
        resets_at: v.get("resetsAt").and_then(epoch_secs),
        seen_at: None,
    })
}

impl Usage {
    /// Parse a `GET /api/usage` body (`{fiveHour, sevenDay}`, each a window or
    /// null). Never fails: anything unreadable counts as "no figure".
    pub fn from_api(body: &Value) -> Usage {
        Usage {
            five_hour: window(body.get("fiveHour")),
            seven_day: window(body.get("sevenDay")),
        }
    }

    /// Drop the windows that have already reset, and those with no reset time
    /// that were last seen to change more than [`NO_RESET_TTL_SECS`] ago: a
    /// figure that cannot say when it stops being true is not kept forever.
    pub fn without_expired(mut self, now: i64) -> Usage {
        let live = |w: &Option<Window>| {
            w.as_ref().is_none_or(|w| match w.resets_at {
                Some(t) => t > now,
                None => w.seen_at.is_some_and(|s| now - s < NO_RESET_TTL_SECS),
            })
        };
        if !live(&self.five_hour) {
            self.five_hour = None;
        }
        if !live(&self.seven_day) {
            self.seven_day = None;
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        self.five_hour.is_none() && self.seven_day.is_none()
    }

    /// `5h N% · wk M%`; a missing window is left out, none at all is `None`.
    pub fn chip(&self) -> Option<String> {
        let five = self.five_hour.as_ref().map(|w| format!("5h {}%", w.pct));
        let week = self.seven_day.as_ref().map(|w| format!("wk {}%", w.pct));
        match (five, week) {
            (None, None) => None,
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (Some(a), Some(b)) => Some(format!("{a} · {b}")),
        }
    }

    /// The widget content: one dim flow item, or `None` to clear the widget.
    pub fn widget(&self) -> Option<WidgetPut> {
        let chip = self.chip()?;
        let item = WidgetItem {
            priority: PRIORITY,
            segments: Some(vec![Segment {
                text: format!(" {chip} "),
                style: Some(Style {
                    fg: Some("dim".into()),
                    ..Style::default()
                }),
                click: None,
            }]),
            ..WidgetItem::default()
        };
        Some(WidgetPut {
            scopes: [("*".to_owned(), vec![item])].into(),
            ttl_secs: None,
        })
    }
}

/// `Z` or `+hh:mm` RFC 3339 timestamps and plain numbers (seconds, or
/// milliseconds when too large to be seconds) as epoch seconds.
pub fn epoch_secs(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => {
            let n = n.as_f64()?;
            Some(if n > 1e11 {
                (n / 1000.0) as i64
            } else {
                n as i64
            })
        }
        Value::String(s) => rfc3339(s),
        _ => None,
    }
}

fn rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', 't', ' '])?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    if d.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    // Split the clock from the zone: `Z`, `+hh:mm` or `-hh:mm`.
    let zone_at = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let (clock, zone) = rest.split_at(zone_at);
    let mut c = clock.split(':');
    let (h, mi): (i64, i64) = (c.next()?.parse().ok()?, c.next()?.parse().ok()?);
    let sec: f64 = c.next().unwrap_or("0").parse().ok()?;
    let offset = match zone {
        "" | "Z" | "z" => 0,
        z => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let (oh, om) = z[1..].split_once(':')?;
            sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60)
        }
    };
    // Days from 1970-01-01 (Howard Hinnant's civil-date algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec as i64 - offset)
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// What to draw: the daemon's figures when it has any, else the last stored
/// ones that have not reset yet. Returns the usage to show and whether it
/// came fresh from the daemon (and so should be stored).
pub fn choose(fresh: Usage, stored: Option<Usage>, now: i64) -> (Usage, bool) {
    // A figure the daemon repeats unchanged keeps the time it was first seen,
    // so one without a reset time ages out even while the daemon keeps saying it.
    let stamp = |f: Option<Window>, was: Option<&Window>| {
        f.map(|mut w| {
            w.seen_at = match was {
                Some(o) if o.pct == w.pct && o.resets_at == w.resets_at && o.seen_at.is_some() => {
                    o.seen_at
                }
                _ => Some(now),
            };
            w
        })
    };
    let was = stored.clone().unwrap_or_default();
    let fresh = Usage {
        five_hour: stamp(fresh.five_hour, was.five_hour.as_ref()),
        seven_day: stamp(fresh.seven_day, was.seven_day.as_ref()),
    }
    .without_expired(now);
    if !fresh.is_empty() {
        // The daemon may know one window only: keep the other from storage.
        let kept = stored.unwrap_or_default().without_expired(now);
        return (
            Usage {
                five_hour: fresh.five_hour.or(kept.five_hour),
                seven_day: fresh.seven_day.or(kept.seven_day),
            },
            true,
        );
    }
    (stored.unwrap_or_default().without_expired(now), false)
}

/// One run: read, choose, store, draw. A daemon that cannot be reached for
/// `/api/usage` still gets the stored figures drawn.
pub fn refresh(api: &Api, dir: &std::path::Path, now: i64) -> Result<(), Error> {
    let fresh = api
        .get("/api/usage")
        .map(|b| Usage::from_api(&b))
        .unwrap_or_default();
    let (usage, store) = choose(fresh, read_state_in::<Usage>(dir, STORE), now);
    if store {
        write_state_in(dir, STORE, &usage)?;
    }
    match usage.widget() {
        Some(put) => api.put_widget(WIDGET, &put).map(|_| ()),
        None => api.clear_widget(WIDGET),
    }
}

/// The wire body, for tests and the wall's golden file.
pub fn wire(usage: &Usage) -> Value {
    usage.widget().map_or(Value::Null, |w| {
        serde_json::to_value(w).unwrap_or(json!(null))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::fake::FakeDaemon;

    fn api(d: &FakeDaemon) -> Api {
        Api::new(&d.base, "usage-bar", "tok")
    }

    fn usd(five: Option<(u32, Option<i64>)>, week: Option<(u32, Option<i64>)>) -> Usage {
        let w = |x: Option<(u32, Option<i64>)>| {
            x.map(|(pct, resets_at)| Window {
                pct,
                resets_at,
                seen_at: Some(0),
            })
        };
        Usage {
            five_hour: w(five),
            seven_day: w(week),
        }
    }

    #[test]
    fn the_chip_reads_exactly_as_the_core_chip_did() {
        for (u, want) in [
            (
                usd(Some((42, None)), Some((10, None))),
                Some("5h 42% · wk 10%"),
            ),
            (usd(Some((42, None)), None), Some("5h 42%")),
            (usd(None, Some((10, None))), Some("wk 10%")),
            (usd(None, None), None),
        ] {
            assert_eq!(u.chip().as_deref(), want);
        }
        // A null window is omitted, a fractional one rounds (23.5 -> 24), >100 clamps.
        let u = Usage::from_api(&json!({
            "fiveHour": {"usedPercentage": 23.5, "resetsAt": "2026-08-30T12:00:00Z"},
            "sevenDay": {"usedPercentage": 400, "resetsAt": null}
        }));
        assert_eq!(u.chip().as_deref(), Some("5h 24% · wk 100%"));
        assert_eq!(
            Usage::from_api(&json!({"fiveHour": null, "sevenDay": null})),
            Usage::default()
        );
        assert_eq!(Usage::from_api(&json!("garbage")), Usage::default());
    }

    #[test]
    fn the_widget_is_one_dim_padded_flow_item_at_the_old_priority() {
        let wire = wire(&usd(Some((98, None)), None));
        assert_eq!(
            wire,
            json!({"scopes": {"*": [{"priority": 50, "placement": {"kind": "flow"},
                "segments": [{"text": " 5h 98% ", "style": {"fg": "dim"}}]}]}})
        );
        assert_eq!(super::wire(&Usage::default()), Value::Null);
    }

    /// The wall renders these same files in its strip tests: the widget body this
    /// plugin pushes and the one the wall's snapshots assume are one thing.
    #[test]
    fn the_wire_body_is_the_golden_file_the_wall_renders() {
        for (file, usage) in [
            (
                include_str!("../golden/both.json"),
                usd(Some((24, None)), Some((61, None))),
            ),
            (
                include_str!("../golden/five-only.json"),
                usd(Some((98, None)), None),
            ),
        ] {
            let golden: Value = serde_json::from_str(file).unwrap();
            assert_eq!(wire(&usage), golden);
        }
    }

    #[test]
    fn reset_times_parse_from_iso_strings_and_numbers() {
        // 2026-08-30T12:00:00Z = 1788091200.
        for v in [
            json!("2026-08-30T12:00:00Z"),
            json!("2026-08-30T14:00:00+02:00"),
            json!("2026-08-30T07:00:00-05:00"),
            json!(1_788_091_200),
            json!(1_788_091_200_000_i64),
        ] {
            assert_eq!(epoch_secs(&v), Some(1_788_091_200), "{v}");
        }
        assert_eq!(
            epoch_secs(&json!("2024-02-29T00:00:00Z")),
            Some(1_709_164_800),
            "leap day"
        );
        for bad in [
            json!("soon"),
            json!("2026-13-01T00:00:00Z"),
            json!(null),
            json!(true),
        ] {
            assert_eq!(epoch_secs(&bad), None, "{bad}");
        }
    }

    #[test]
    fn stored_figures_stand_in_until_their_window_resets() {
        let stored = usd(Some((40, Some(2_000))), Some((70, Some(9_000))));
        // The daemon restarted and has nothing yet: draw what we stored.
        let (u, fresh) = choose(Usage::default(), Some(stored.clone()), 1_000);
        assert_eq!(
            (u.chip().as_deref(), fresh),
            (Some("5h 40% · wk 70%"), false)
        );
        // The 5h window reset meanwhile: only the weekly figure is still true.
        let (u, _) = choose(Usage::default(), Some(stored.clone()), 3_000);
        assert_eq!(u.chip().as_deref(), Some("wk 70%"));
        // Everything reset: nothing to draw.
        assert!(
            choose(Usage::default(), Some(stored.clone()), 10_000)
                .0
                .is_empty()
        );
        // A window with no known reset time is kept for 15 minutes from when it was seen.
        let no_reset = || Some(usd(Some((5, None)), None)); // seen at 0
        let shown = |now| choose(Usage::default(), no_reset(), now).0.chip();
        assert_eq!(shown(NO_RESET_TTL_SECS - 1).as_deref(), Some("5h 5%"));
        assert_eq!(shown(NO_RESET_TTL_SECS), None, "then it is history");
        // An old file from before `seen_at` existed has no age, so it is not trusted.
        let legacy: Usage =
            serde_json::from_str(r#"{"five_hour":{"pct":5,"resets_at":null},"seven_day":null}"#)
                .unwrap();
        assert!(choose(Usage::default(), Some(legacy), 1).0.is_empty());
        // Fresh figures win per window and are flagged for storing; the other window is kept.
        let (u, fresh) = choose(usd(Some((41, Some(2_500))), None), Some(stored), 1_000);
        assert_eq!(
            (u.chip().as_deref(), fresh),
            (Some("5h 41% · wk 70%"), true)
        );
    }

    #[test]
    fn a_figure_without_a_reset_time_ages_out_even_while_the_daemon_keeps_repeating_it() {
        let said = || usd(Some((30, None)), None);
        let seen = |at| Usage {
            five_hour: said().five_hour.map(|w| Window {
                seen_at: Some(at),
                ..w
            }),
            seven_day: None,
        };
        // First sight at t=1000: stamped and shown.
        let (u, store) = choose(said_unstamped(), None, 1000);
        assert_eq!((u.chip().as_deref(), store), (Some("5h 30%"), true));
        assert_eq!(u.five_hour.as_ref().unwrap().seen_at, Some(1000));
        // The same figure again 10 minutes later keeps its original stamp.
        let (u, _) = choose(said_unstamped(), Some(u), 1600);
        assert_eq!(u.five_hour.as_ref().unwrap().seen_at, Some(1000));
        // 15 minutes after first sight it is gone, though the daemon still says it.
        let (u, store) = choose(said_unstamped(), Some(seen(1000)), 1000 + NO_RESET_TTL_SECS);
        assert!(u.is_empty() && !store);
        // A changed figure is new news and starts its own 15 minutes.
        let changed = Usage {
            five_hour: Some(Window {
                pct: 31,
                resets_at: None,
                seen_at: None,
            }),
            seven_day: None,
        };
        let (u, store) = choose(changed, Some(seen(1000)), 1000 + NO_RESET_TTL_SECS);
        assert_eq!(u.five_hour.unwrap().seen_at, Some(1000 + NO_RESET_TTL_SECS));
        assert!(store);
    }

    fn said_unstamped() -> Usage {
        Usage::from_api(&json!({"fiveHour": {"usedPercentage": 30, "resetsAt": null}}))
    }

    #[test]
    fn a_run_stores_what_the_daemon_said_and_draws_it() {
        let d = FakeDaemon::start();
        d.route("GET", "/api/usage", 200, r#"{"fiveHour":{"usedPercentage":42,"resetsAt":"2999-01-01T00:00:00Z"},"sevenDay":null}"#);
        let dir = tempfile_dir("run-stores");
        refresh(&api(&d), &dir, 1_000).unwrap();
        let puts = d.bodies("PUT", "/api/plugins/usage-bar/widgets/usage");
        assert_eq!(puts.len(), 1);
        assert_eq!(puts[0]["scopes"]["*"][0]["segments"][0]["text"], " 5h 42% ");
        let stored: Usage = read_state_in(&dir, STORE).unwrap();
        assert_eq!(stored.five_hour.unwrap().pct, 42);
    }

    #[test]
    fn after_a_daemon_restart_the_stored_figures_are_drawn_again() {
        let d = FakeDaemon::start(); // /api/usage answers the default {"ok":true}: no figures
        let dir = tempfile_dir("restart");
        write_state_in(
            &dir,
            STORE,
            &usd(Some((42, Some(i64::MAX / 2))), Some((7, None))), // seen at 0
        )
        .unwrap();
        refresh(&api(&d), &dir, 800).unwrap();
        let puts = d.bodies("PUT", "/api/plugins/usage-bar/widgets/usage");
        assert_eq!(
            puts[0]["scopes"]["*"][0]["segments"][0]["text"],
            " 5h 42% · wk 7% "
        );
        // Much later the weekly figure, which never had a reset time, is dropped.
        refresh(&api(&d), &dir, NO_RESET_TTL_SECS).unwrap();
        let puts = d.bodies("PUT", "/api/plugins/usage-bar/widgets/usage");
        assert_eq!(puts[1]["scopes"]["*"][0]["segments"][0]["text"], " 5h 42% ");
    }

    #[test]
    fn with_nothing_known_the_widget_is_cleared_not_left_stale() {
        let d = FakeDaemon::start();
        d.route(
            "GET",
            "/api/usage",
            200,
            r#"{"fiveHour":null,"sevenDay":null}"#,
        );
        refresh(&api(&d), &tempfile_dir("empty"), 1_000).unwrap();
        let seen = d.seen();
        assert!(
            seen.iter()
                .any(|s| s.method == "DELETE" && s.path == "/api/plugins/usage-bar/widgets/usage"),
            "{seen:?}"
        );
        assert!(
            d.bodies("PUT", "/api/plugins/usage-bar/widgets/usage")
                .is_empty()
        );
    }

    fn tempfile_dir(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("usage-bar-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }
}
