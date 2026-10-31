//! garage-wall entrypoint: deliberately thin. The sequence lives in
//! `garage_wall::app`, shared with the `claude-garage` binary.

use std::process::ExitCode;

use garage_wall::app::{self, Launch};

fn main() -> ExitCode {
    ExitCode::from(u8::try_from(app::run(Launch::from_env())).unwrap_or(1))
}
