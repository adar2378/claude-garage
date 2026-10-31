//! `quick-send one` and `quick-send all`: the popups behind `s` and `S`.

fn main() {
    let all = match std::env::args().nth(1).as_deref() {
        Some("one") => false,
        Some("all") => true,
        other => {
            eprintln!("quick-send: unknown command {other:?} (one | all)");
            std::process::exit(2);
        }
    };
    if let Err(e) = garage_plugin_quick_send::run(all) {
        eprintln!("quick-send: {e}");
        std::process::exit(1);
    }
}
