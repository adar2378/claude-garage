use super::*;

fn text_of(buf: &Buffer, area: Rect) -> String {
    (area.top()..area.bottom())
        .map(|y| {
            (area.left()..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_reveal_runs_only_while_the_launcher_is_starting_the_daemon() {
    assert!(should_reveal(true, true, false));
    // Daemon already up (no env var, or it answered the first probe).
    assert!(!should_reveal(false, true, true));
    assert!(!should_reveal(true, true, true));
    // Plain `garage-wall` with no launcher hand-off never reveals.
    assert!(!should_reveal(false, true, false));
    // Motion off.
    assert!(!should_reveal(true, false, false));
}

#[test]
fn the_wordmark_block_draws_word_tagline_and_a_spinner_line() {
    let area = Rect::new(0, 0, 80, 20);
    let mut buf = Buffer::empty(area);
    let word = render_wordmark(&mut buf, area, 0, true);
    assert_eq!(word.height, 4, "big text at half height");
    assert!(
        (word.top()..word.bottom()).any(|y| (0..80).any(|x| buf[(x, y)].symbol() != " ")),
        "wordmark has ink"
    );
    let all = text_of(&buf, area);
    assert!(all.contains(TAGLINE));
    assert!(all.contains("starting daemon…"));
    assert!(all.contains(&spinner_symbol_for_test(0)));
}

fn spinner_symbol_for_test(now: i64) -> String {
    crate::ui::motion::spinner_symbol(now)
}

#[test]
fn a_narrow_terminal_falls_back_to_the_plain_word() {
    let area = Rect::new(0, 0, 24, 8);
    let mut buf = Buffer::empty(area);
    let word = render_wordmark(&mut buf, area, 0, false);
    assert_eq!(word.height, 1);
    let all = text_of(&buf, area);
    assert!(all.contains("garage"));
    assert!(all.contains("starting daemon…"));
    // Static marker, no throbber glyph.
    assert!(!all.contains(&spinner_symbol_for_test(0)));
}

#[test]
fn the_reveal_effect_ends_on_the_exact_plain_wordmark_and_never_uses_amber() {
    let area = Rect::new(0, 0, 80, 20);
    let mut plain = Buffer::empty(area);
    render_wordmark(&mut plain, area, 0, true);

    let mut effect = reveal_effect();
    let mut elapsed = 0;
    let mut changed_mid_way = false;
    while !effect.done() {
        let mut buf = Buffer::empty(area);
        let word = render_wordmark(&mut buf, area, 0, true);
        effect.process(tachyonfx::Duration::from_millis(33), &mut buf, word);
        changed_mid_way |= buf != plain;
        elapsed += 33;
        assert!(elapsed <= REVEAL_MS + 200, "reveal outlived its budget");
    }
    assert!(changed_mid_way, "the reveal actually animates");
    // The frame after completion is drawn without the effect.
    let mut settled = Buffer::empty(area);
    render_wordmark(&mut settled, area, 0, true);
    assert_eq!(settled, plain);
    for y in 0..area.height {
        for x in 0..area.width {
            assert_ne!(plain[(x, y)].fg, colors::AMBER);
        }
    }
}

#[test]
fn the_timeout_message_names_the_log_when_the_launcher_gave_one() {
    let with = timed_out_message("http://127.0.0.1:4747", Some("/h/.garage/daemon.log"));
    assert!(with.contains("/h/.garage/daemon.log") && with.contains("GARAGE_PORT"));
    assert!(!timed_out_message("http://127.0.0.1:4747", None).contains("see"));
}
