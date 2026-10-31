fn main() {
    if let Err(e) = garage_plugin_jump_palette::run() {
        eprintln!("jump-palette: {e}");
        std::process::exit(1);
    }
}
