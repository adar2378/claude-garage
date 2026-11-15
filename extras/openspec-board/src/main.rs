//! `openspec-board ui`: the overlay pane.

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let result = match cmd.as_str() {
        "ui" => garage_plugin_openspec_board::pane::run().map_err(|e| e.to_string()),
        other => Err(format!("unknown command {other:?} (ui)")),
    };
    if let Err(e) = result {
        eprintln!("openspec-board: {e}");
        std::process::exit(1);
    }
}
