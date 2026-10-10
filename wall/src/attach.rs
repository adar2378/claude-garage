//! `garage-wall attach <session-id>` — a plain terminal client for one native
//! session host (the Windows counterpart of `tmux attach`). `t` in the wall
//! opens this in a new OS terminal window.
//!
//! Host output is the session's raw terminal stream (a screen snapshot first,
//! then live bytes), so it is written to stdout verbatim; keys come from
//! crossterm and go through the same encoder the wall uses. `Ctrl+G`
//! detaches (closes this window, the session keeps running).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};

use crate::host;
use crate::input::encode::encode_key;

pub fn run_attach(id: &str) -> i32 {
    let (cols, rows) = size().unwrap_or((120, 40));
    let mut stream = match host::connect_attach(id, cols, rows) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("garage attach: {e}");
            return 1;
        }
    };
    let mut reader = match stream.try_clone() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("garage attach: {e}");
            return 1;
        }
    };

    enable_vt_output();
    if enable_raw_mode().is_err() {
        eprintln!("garage attach: could not enter raw mode");
        return 1;
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);

    let gone = Arc::new(AtomicBool::new(false));
    {
        let gone = gone.clone();
        std::thread::spawn(move || {
            let mut out = std::io::stdout();
            let mut buf = [0u8; 65536];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = out.write_all(&buf[..n]);
                        let _ = out.flush();
                    }
                }
            }
            gone.store(true, Ordering::SeqCst);
        });
    }

    while !gone.load(Ordering::SeqCst) {
        if !event::poll(Duration::from_millis(100)).unwrap_or(false) {
            continue;
        }
        match event::read() {
            Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                if let Some(bytes) = encode_key(&key) {
                    if host::write_frame(&mut stream, 0, &bytes).is_err() {
                        break;
                    }
                }
            }
            Ok(Event::Paste(text)) => {
                let wrapped = format!("\x1b[200~{text}\x1b[201~");
                if host::write_frame(&mut stream, 0, wrapped.as_bytes()).is_err() {
                    break;
                }
            }
            Ok(Event::Resize(c, r)) => {
                let mut p = Vec::with_capacity(4);
                p.extend_from_slice(&c.to_le_bytes());
                p.extend_from_slice(&r.to_le_bytes());
                let _ = host::write_frame(&mut stream, 1, &p);
            }
            _ => {}
        }
    }

    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
    let _ = disable_raw_mode();
    let _ = stream.shutdown(std::net::Shutdown::Both);
    println!();
    0
}

/// Make sure the console interprets the escape sequences the host streams.
fn enable_vt_output() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{
            GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            STD_OUTPUT_HANDLE,
        };
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0u32;
        if GetConsoleMode(h, &mut mode) != 0 {
            SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}
