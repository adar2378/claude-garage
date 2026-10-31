//! Draft and pasted-screenshot recovery, wall side (p25). Once a second the
//! prompt of every session tile is read off its live screen and kept on
//! disk; when a session comes back with an empty prompt but a kept draft,
//! the wall offers it (`u`) and types it back. See [`crate::drafts`] for what
//! is kept and how it is protected.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::drafts::{self, DraftStore, Prompt};
use crate::input::paste::wrap_bracketed_paste;
use crate::state::wall_state::WallSession;

use super::App;

/// The garage-layer key that restores an offered draft.
pub(super) const DRAFT_OFFER_KEY: &str = "u";
/// Read the prompts this often (ms).
const SCAN_EVERY_MS: i64 = 1_000;
/// Gap between the pieces of a restore (text, then each image path): Claude
/// Code needs a moment to take an image before the next paste.
const PIECE_GAP_MS: i64 = 700;
/// How long an offer stays takeable.
const OFFER_TTL_MS: i64 = 5 * 60_000;
/// How long the notice announcing an offer stays up.
const OFFER_NOTICE_MS: i64 = 12_000;
/// A fresh image paste is not judged by the prompt for this long: Claude
/// takes a moment to draw its `[Image #N]` placeholder.
const IMAGE_GRACE_MS: i64 = 5_000;

pub(super) struct Restore {
    pub(super) session: String,
    pub(super) pieces: VecDeque<String>,
    pub(super) due_ms: i64,
}

#[derive(Default)]
pub(super) struct DraftSync {
    store: Option<DraftStore>,
    last_scan_ms: i64,
    /// The text last written per session; a session is here once the wall
    /// has seen the user's own typing in it.
    seen: HashMap<String, String>,
    /// Sessions already offered a restore this run.
    offered: HashSet<String>,
    /// Open offers: (session, expires at ms).
    pub(super) offers: Vec<(String, i64)>,
    pub(super) restoring: Vec<Restore>,
    /// When an image was last added per session (ms).
    image_at: HashMap<String, i64>,
}

impl DraftSync {
    /// Turn recovery on, keeping files in `root`. Off until called, so
    /// nothing is written by tests or by an App that is never run.
    pub(super) fn enable(&mut self, root: std::path::PathBuf) {
        let store = DraftStore::new(root);
        store.prune_older_than(drafts::MAX_AGE);
        self.store = Some(store);
    }
}

impl App {
    /// Read prompts, make offers, type restores. True when a redraw is due.
    pub(super) fn draft_tick(&mut self, now: i64) -> bool {
        let mut dirty = self.pump_restores(now);
        if self.drafts.store.is_none() || now - self.drafts.last_scan_ms < SCAN_EVERY_MS {
            return dirty;
        }
        self.drafts.last_scan_ms = now;
        self.drafts.offers.retain(|(_, until)| *until > now);

        // Session tiles only: the gridded, live, attached ones. A plugin
        // pane is never in the grid, so nothing is ever kept for one.
        let ids: Vec<String> = self
            .store
            .state()
            .gridded_session_ids
            .iter()
            .filter(|id| {
                self.store
                    .state()
                    .session_by_id(Some(id))
                    .is_some_and(WallSession::live)
            })
            .cloned()
            .collect();
        for id in ids {
            let Some(prompt) = self.registry.with_live_screen(&id, drafts::read_prompt) else {
                continue;
            };
            dirty |= self.observe_prompt(&id, prompt, now);
        }
        dirty
    }

    pub(super) fn observe_prompt(&mut self, id: &str, prompt: Prompt, now: i64) -> bool {
        let Some(store) = self.drafts.store.clone() else {
            return false;
        };
        match prompt {
            Prompt::Unknown => false,
            Prompt::Text(text) => {
                // The user is typing (or the restore landed): no offer needed.
                self.drafts.offers.retain(|(s, _)| s != id);
                if self.drafts.seen.get(id) != Some(&text) && store.save_text(id, &text).is_ok() {
                    self.drafts.seen.insert(id.to_owned(), text);
                }
                false
            }
            Prompt::Empty => {
                let restoring = self.drafts.restoring.iter().any(|r| r.session == id);
                let offered_now = self.drafts.offers.iter().any(|(s, _)| s == id);
                if restoring || offered_now {
                    return false;
                }
                if self.drafts.seen.contains_key(id) {
                    // Typed, now empty: it was sent (or deliberately wiped).
                    let fresh_image = self
                        .drafts
                        .image_at
                        .get(id)
                        .is_some_and(|at| now - at < IMAGE_GRACE_MS);
                    if !fresh_image {
                        store.clear(id);
                        self.drafts.seen.remove(id);
                    }
                    return false;
                }
                if self.drafts.image_at.contains_key(id) {
                    return false; // an image pasted this run: ours, not a leftover
                }
                if store.has_anything(id) && self.drafts.offered.insert(id.to_owned()) {
                    self.drafts.offers.push((id.to_owned(), now + OFFER_TTL_MS));
                    let name = self.session_name(id);
                    self.router.notices.show(
                        format!(
                            "unsent draft kept for {name} - press {DRAFT_OFFER_KEY} to restore it"
                        ),
                        OFFER_NOTICE_MS,
                        now,
                    );
                    return true;
                }
                false
            }
        }
    }

    fn session_name(&self, id: &str) -> String {
        self.store.state().session_by_id(Some(id)).map_or_else(
            || id.to_owned(),
            |s| s.title.clone().unwrap_or_else(|| s.label.clone()),
        )
    }

    /// `u` in the garage layer: type the offered draft back (the focused
    /// session's offer first). True when the key was an answer to an offer.
    pub(super) fn take_draft_offer(&mut self, key: &str, now: i64) -> bool {
        if key != DRAFT_OFFER_KEY || self.drafts.offers.is_empty() {
            return false;
        }
        let focused = self.store.state().focused_session_id.clone();
        let at = self
            .drafts
            .offers
            .iter()
            .position(|(s, _)| Some(s) == focused.as_ref())
            .unwrap_or(0);
        let (session, _) = self.drafts.offers.remove(at);
        let Some(store) = self.drafts.store.clone() else {
            return false;
        };
        let mut pieces = VecDeque::new();
        if let Some(text) = store.load_text(&session) {
            let text = drafts::strip_image_placeholders(&text);
            if !text.is_empty() {
                pieces.push_back(text);
            }
        }
        for img in store.images(&session) {
            pieces.push_back(img.to_string_lossy().into_owned());
        }
        if pieces.is_empty() {
            return true;
        }
        let name = self.session_name(&session);
        self.router
            .notices
            .show(format!("restoring the draft for {name}"), 2500, now);
        self.drafts.restoring.push(Restore {
            session,
            pieces,
            due_ms: now,
        });
        true
    }

    /// Type the next piece of each restore that is due.
    fn pump_restores(&mut self, now: i64) -> bool {
        let mut wrote = false;
        let mut restoring = std::mem::take(&mut self.drafts.restoring);
        for r in &mut restoring {
            if r.due_ms > now {
                continue;
            }
            if let Some(piece) = r.pieces.pop_front() {
                self.registry.set_frozen(&r.session, None);
                self.registry
                    .write(&r.session, &wrap_bracketed_paste(&piece));
                r.due_ms = now + PIECE_GAP_MS;
                wrote = true;
            }
        }
        restoring.retain(|r| !r.pieces.is_empty());
        self.drafts.restoring = restoring;
        wrote
    }

    /// A bracketed paste went to `session`'s prompt: keep any image files it
    /// named, next to the draft.
    pub(super) fn note_paste_for_drafts(&mut self, session: &str, text: &str, now: i64) {
        let Some(store) = self.drafts.store.clone() else {
            return;
        };
        for path in drafts::pasted_image_paths(text) {
            if let Ok(Some(_)) = store.add_image(session, &path) {
                self.drafts.image_at.insert(session.to_owned(), now);
                // Judge an empty prompt as "was sent", not "never typed".
                self.drafts.seen.entry(session.to_owned()).or_default();
            }
        }
    }

    /// The user closed or deleted the session: nothing to recover.
    pub(super) fn forget_drafts(&mut self, session: &str) {
        if let Some(store) = &self.drafts.store {
            store.clear(session);
        }
        self.drafts.seen.remove(session);
        self.drafts.offers.retain(|(s, _)| s != session);
        self.drafts.restoring.retain(|r| r.session != session);
        self.drafts.image_at.remove(session);
    }
}
