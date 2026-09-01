// pit-pet: an anthropomorphized attention badge. The pet's mood is pure
// derived state — the same signals the header badge and connection chip
// already consume — rendered as a small ASCII creature on the footer key
// strip. One glance at the pet = one glance at the whole wall.
//
// Every state is expressed per species (the animation IS the
// personality): the cat saunters, barely bounces, sometimes ignores a
// stroll, and celebrates by kneading; the duck never bounces — it
// waddles, alerts with a single motionless stare, and celebrates with
// exactly one flap; the pup runs fastest, bounces big, and celebrates
// with zoomies. Validated in the design-mockup artifact before landing.
//
// The TUI port of this pet lives in `wall/src/ui/pet.rs` (spec
// tui-pit-pet) — same species, names, personalities, and mood rules,
// re-cut for one strip row; edit the two together.

// The family, by name (named by the user, 2026-07-19).
export const PET_NAMES = {
  cat: "Arthur",
  duck: "Papito",
  pup: "Segan",
};

export const PET_OPTIONS = [
  { value: "off", label: "no pet", description: "the strip stays empty" },
  { value: "cat", label: "Arthur the shop cat", description: "aloof; saunters over when you're needed, kneads when all clear" },
  { value: "duck", label: "Papito the rubber duck", description: "deadpan; waddles, and alerts with a single motionless stare" },
  { value: "pup", label: "Segan the pit pup", description: "eager; sprints to problems, zoomies when the wall is clear" },
];

// Movement/cadence per species. speed = px per 300ms tick; frameEvery =
// ticks per animation frame; aloof = chance to ignore an idle stroll.
export const PET_PROFILES = {
  cat: { speed: 10, frameEvery: 2, alertBounce: "bounce-small", waddle: false, deadpan: false, aloof: 0.4, zoomies: false, celebrateMs: 3000 },
  duck: { speed: 6, frameEvery: 3, alertBounce: null, waddle: true, deadpan: true, aloof: 0, zoomies: false, celebrateMs: 1100 },
  pup: { speed: 20, frameEvery: 1, alertBounce: "bounce-big", waddle: false, deadpan: false, aloof: 0, zoomies: true, celebrateMs: 3400 },
};

// Frame sets per species per mood (arrays of multi-line frames).
export const PET_FRAMES = {
  cat: {
    sleep: [
      [" /\\_/\\      ", "( -.- )  z  ", " (\")_(\")    "],
      [" /\\_/\\      ", "( -.- )    z", " (\")_(\")    "],
    ],
    watch: [
      [" /\\_/\\ ", "( o.o )", " > ^ < "],
      [" /\\_/\\ ", "( o.o )", "  > ^ <"],
    ],
    alert: [
      [" /\\_/\\ ", "( O_O )", " /| |\\ "],
      [" /\\_/\\ ", "( O_O )", " /|_|\\ "],
    ],
    box: [
      ["┌─────┐", "│ -.- │", "└─────┘"],
      ["┌─────┐", "│ o.o │", "└─────┘"],
    ],
    // cats do not jump for joy in front of you — they knead.
    celebrate: [
      [" /\\_/\\ ", "( ^.^ )", " >^ ^< "],
      [" /\\_/\\ ", "( ^.^ )", " >^^ < "],
    ],
  },
  duck: {
    sleep: [
      ["   __       ", " <(- )___  z", "  `-----'   "],
      ["   __       ", " <(- )___   ", "  `-----'  z"],
    ],
    watch: [
      ["   __     ", " <(o )___ ", "  `-----' "],
      ["   __     ", " <(- )___ ", "  `-----' "],
    ],
    // a single motionless stare. that IS the animation.
    alert: [["   __     ", " <(O )___ ", "  `-----' "]],
    box: [["┌─────┐", "│ (o )>│", "└─────┘"]],
    celebrate: [
      [" \\ __ /   ", " <(o )___ ", "  `-----' "],
      ["  \\__/    ", " <(o )___ ", "  `-----' "],
    ],
  },
  pup: {
    sleep: [
      ["  ∧_∧    ", " (-ᴥ-)  z", "  |u u|   "],
      ["  ∧_∧    ", " (-ᴥ-)   ", "  |u u|  z"],
    ],
    watch: [
      ["  ∧_∧  ", " (·ᴥ·)  ", "  |u u|"],
      ["  ∧_∧  ", " (·ᴥ·)/ ", "  |u u|"],
    ],
    alert: [
      ["  ∧ ∧  ", " (°ᴥ°)  ", "  |U U|"],
      ["  ∧ ∧  ", " (°ᴥ°)  ", " /|U U|"],
    ],
    box: [["┌─────┐", "│ ·ᴥ· │", "└─────┘"]],
    celebrate: [
      ["\\ ∧_∧ /", " (^ᴥ^)  ", "  |u u|"],
      [" \\∧_∧/ ", " (^ᴥ^)  ", "  |u u|"],
    ],
  },
};

/**
 * Mood from wall state: daemon unreachable > nothing to guard > blocked
 * sessions > agents running > quiet.
 */
export function petMood({ sessions, connState }) {
  if (connState !== "live") return "box";
  if (sessions.length === 0) return "sleep";
  if (sessions.some((s) => s.status === "needs-input")) return "alert";
  if (sessions.some((s) => s.status === "working")) return "watch";
  return "sleep";
}
