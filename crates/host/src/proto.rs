//! Wire protocol and client calls. Std only, so the TUI and the daemon can
//! talk to a host without pulling in the PTY stack's server half.
//!
//! All integers are little-endian. A client opens with a hello:
//! `"GH1"` + 32-char token + mode (`'A'` attach, `'Q'` control) + cols u16 +
//! rows u16. After that, client to host is a stream of frames
//! `[type u8][len u32][payload]`:
//!
//! - `0` input bytes (attach)
//! - `1` resize: cols u16, rows u16 (attach)
//! - `2` history size (control; reply `[len u32][i64]`)
//! - `3` formatted capture: top i64, cols u16, rows u16 (control; reply `[len u32][bytes]`,
//!   rows joined by `\n`, with escapes, for the TUI's frozen scrollback)
//! - `4` shutdown (control; reply `[len u32=1][1]`)
//! - `5` rename, payload = new id utf8 (control; reply `[len u32=1][1]`)
//! - `7` info (control; reply `[len u32][json]`, see [`Info`])
//! - `8` plain-text capture: last N lines u32 (control; reply `[len u32][utf8]`)
//!
//! An attach connection first receives a snapshot of the screen
//! (`state_formatted`), then the raw output stream; snapshot and registration
//! happen under one lock, so no byte is lost or duplicated.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8; 3] = b"GH1";
pub const TOKEN_LEN: usize = 32;
pub const MODE_ATTACH: u8 = b'A';
pub const MODE_CONTROL: u8 = b'Q';

pub const F_INPUT: u8 = 0;
pub const F_RESIZE: u8 = 1;
pub const F_HISTORY: u8 = 2;
pub const F_CAPTURE: u8 = 3;
pub const F_SHUTDOWN: u8 = 4;
pub const F_RENAME: u8 = 5;
pub const F_INFO: u8 = 7;
pub const F_TEXT: u8 = 8;

/// Largest frame either side accepts.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// What a live host reports about itself (control frame `7`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub id: String,
    pub pid: u32,
    pub child_pid: u32,
    pub dir: String,
    pub created_at: u64,
    pub title: String,
    /// Number of attach connections right now.
    pub attached: u32,
}

pub fn io_err(msg: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(msg.to_string())
}

pub fn write_frame(w: &mut impl Write, ty: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(5 + payload.len());
    buf.push(ty);
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

pub fn read_frame(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_FRAME {
        return Err(io_err("frame too large"));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

pub fn read_reply(r: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io_err("reply too large"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn write_reply(w: &mut impl Write, payload: &[u8]) -> std::io::Result<()> {
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// The hello bytes for `token`, `mode` and a size.
pub fn hello(token: &str, mode: u8, cols: u16, rows: u16) -> Vec<u8> {
    let mut h = Vec::with_capacity(3 + TOKEN_LEN + 1 + 4);
    h.extend_from_slice(MAGIC);
    h.extend_from_slice(token.as_bytes());
    h.push(mode);
    h.extend_from_slice(&cols.to_le_bytes());
    h.extend_from_slice(&rows.to_le_bytes());
    h
}

/// `addr` as a loopback socket address, else an error. Hosts only ever listen
/// on loopback, so anything else is a bad or hostile registry/spec value and
/// must never be dialed (the hello carries the token).
pub fn parse_loopback(addr: &str) -> std::io::Result<SocketAddr> {
    let parsed: SocketAddr = addr.parse().map_err(|_| io_err(format!("bad host address {addr:?}")))?;
    if !parsed.ip().is_loopback() {
        return Err(io_err(format!("refusing non-loopback host address {addr:?}")));
    }
    Ok(parsed)
}

/// Connect to `addr` (a loopback address, `127.0.0.1:<port>`) and complete the
/// hello. A wrong token is only noticed by the host closing the connection.
pub fn connect(addr: &str, token: &str, mode: u8, cols: u16, rows: u16) -> std::io::Result<TcpStream> {
    if token.len() != TOKEN_LEN {
        return Err(io_err("bad host token"));
    }
    let addr = parse_loopback(addr)?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    stream.set_nodelay(true).ok();
    stream.write_all(&hello(token, mode, cols, rows))?;
    Ok(stream)
}

pub fn connect_attach(addr: &str, token: &str, cols: u16, rows: u16) -> std::io::Result<TcpStream> {
    connect(addr, token, MODE_ATTACH, cols, rows)
}

/// One control round trip: send `ty`/`payload`, read the reply.
fn control(addr: &str, token: &str, ty: u8, payload: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut s = connect(addr, token, MODE_CONTROL, 0, 0)?;
    s.set_read_timeout(Some(REPLY_TIMEOUT))?;
    s.set_write_timeout(Some(REPLY_TIMEOUT))?;
    write_frame(&mut s, ty, payload)?;
    read_reply(&mut s)
}

/// Doubles as the liveness probe: `Err` means no host answers there.
pub fn info(addr: &str, token: &str) -> std::io::Result<Info> {
    let reply = control(addr, token, F_INFO, &[])?;
    serde_json::from_slice(&reply).map_err(io_err)
}

/// Absolute history size (lines scrolled off the top).
pub fn history_size(addr: &str, token: &str) -> Option<i64> {
    let reply = control(addr, token, F_HISTORY, &[]).ok()?;
    Some(i64::from_le_bytes(reply.try_into().ok()?))
}

/// `rows` formatted lines (with escapes, joined by `\n`) from absolute line `top_abs`.
pub fn capture_formatted(addr: &str, token: &str, top_abs: i64, cols: u16, rows: u16) -> Option<Vec<u8>> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&top_abs.to_le_bytes());
    p.extend_from_slice(&cols.to_le_bytes());
    p.extend_from_slice(&rows.to_le_bytes());
    control(addr, token, F_CAPTURE, &p).ok()
}

/// The last `lines` lines of scrollback plus screen, as plain text.
pub fn capture_text(addr: &str, token: &str, lines: u32) -> std::io::Result<String> {
    let reply = control(addr, token, F_TEXT, &lines.to_le_bytes())?;
    Ok(String::from_utf8_lossy(&reply).into_owned())
}

/// Type `text` into the host's command as keystrokes, then press Enter (what
/// `tmux send-keys -l` plus `Enter` does). Rides the attach connection's input
/// frame with a 0x0 hello size, which the host reads as "do not resize". The
/// connection is closed the orderly way: the write side is shut, then the
/// host's snapshot is drained until it hangs up, so the input frames are never
/// discarded by a reset.
pub fn send_text(addr: &str, token: &str, text: &str) -> std::io::Result<()> {
    let mut s = connect(addr, token, MODE_ATTACH, 0, 0)?;
    s.set_read_timeout(Some(REPLY_TIMEOUT))?;
    s.set_write_timeout(Some(REPLY_TIMEOUT))?;
    write_frame(&mut s, F_INPUT, text.as_bytes())?;
    // A separate frame after a beat, so a multi-line paste does not swallow it.
    std::thread::sleep(Duration::from_millis(50));
    write_frame(&mut s, F_INPUT, b"\r")?;
    s.shutdown(std::net::Shutdown::Write)?;
    let mut sink = [0u8; 4096];
    let deadline = std::time::Instant::now() + REPLY_TIMEOUT;
    while std::time::Instant::now() < deadline && matches!(s.read(&mut sink), Ok(n) if n > 0) {}
    Ok(())
}

/// Ask the host to kill its command and exit.
pub fn shutdown(addr: &str, token: &str) -> std::io::Result<()> {
    control(addr, token, F_SHUTDOWN, &[]).map(|_| ())
}

pub fn rename(addr: &str, token: &str, new_id: &str) -> std::io::Result<()> {
    control(addr, token, F_RENAME, new_id.as_bytes()).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_text_types_the_text_then_enter_without_resizing() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let host = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut hello = [0u8; 40];
            s.read_exact(&mut hello).unwrap();
            let mut frames = Vec::new();
            while let Ok((ty, payload)) = read_frame(&mut s) {
                frames.push((ty, payload));
            }
            (hello, frames)
        });
        send_text(&addr, &"t".repeat(TOKEN_LEN), "echo hi").unwrap();
        let (hello, frames) = host.join().unwrap();
        assert_eq!(hello[3 + TOKEN_LEN], MODE_ATTACH);
        assert_eq!(&hello[36..40], &[0, 0, 0, 0], "size 0x0: the host must not resize its PTY");
        assert_eq!(frames, vec![(F_INPUT, b"echo hi".to_vec()), (F_INPUT, b"\r".to_vec())]);
    }

    #[test]
    fn frames_roundtrip_and_oversize_is_refused() {
        let mut buf = Vec::new();
        write_frame(&mut buf, F_RESIZE, &[1, 2, 3, 4]).unwrap();
        assert_eq!(buf, [1, 4, 0, 0, 0, 1, 2, 3, 4]);
        assert_eq!(read_frame(&mut buf.as_slice()).unwrap(), (F_RESIZE, vec![1, 2, 3, 4]));

        let huge = [0u8, 0xff, 0xff, 0xff, 0xff];
        assert!(read_frame(&mut huge.as_slice()).is_err());
        assert!(read_reply(&mut [0xff, 0xff, 0xff, 0xff].as_slice()).is_err());
        assert!(read_frame(&mut [0u8, 9].as_slice()).is_err(), "truncated header");
    }

    #[test]
    fn replies_roundtrip() {
        let mut buf = Vec::new();
        write_reply(&mut buf, b"hello").unwrap();
        assert_eq!(buf, [5, 0, 0, 0, b'h', b'e', b'l', b'l', b'o']);
        assert_eq!(read_reply(&mut buf.as_slice()).unwrap(), b"hello");
    }

    #[test]
    fn hello_layout_is_magic_token_mode_size() {
        let token = "a".repeat(TOKEN_LEN);
        let h = hello(&token, MODE_ATTACH, 0x0102, 0x0304);
        assert_eq!(&h[..3], b"GH1");
        assert_eq!(&h[3..35], token.as_bytes());
        assert_eq!(h[35], b'A');
        assert_eq!(&h[36..], [0x02, 0x01, 0x04, 0x03]);
        assert_eq!(h.len(), 40);
    }

    #[test]
    fn connect_rejects_a_malformed_token_or_address_without_dialing() {
        assert!(connect("127.0.0.1:1", "short", MODE_CONTROL, 0, 0).is_err());
        assert!(connect("not an address", &"a".repeat(TOKEN_LEN), MODE_CONTROL, 0, 0).is_err());
        assert!(connect("", &"a".repeat(TOKEN_LEN), MODE_CONTROL, 0, 0).is_err());
    }

    #[test]
    fn only_loopback_addresses_are_ever_dialed() {
        for ok in ["127.0.0.1:4000", "[::1]:4000", "127.5.5.5:1"] {
            assert!(parse_loopback(ok).is_ok(), "{ok}");
        }
        for bad in ["10.0.0.1:80", "8.8.8.8:53", "0.0.0.0:80", "[::2]:1", "evil.example:80", "localhost:80", "127.0.0.1", "[fe80::1]:1"] {
            assert!(parse_loopback(bad).is_err(), "{bad}");
        }
        // The check happens before any dial: a public address fails at once.
        let e = connect("8.8.8.8:53", &"a".repeat(TOKEN_LEN), MODE_CONTROL, 0, 0).unwrap_err();
        assert!(e.to_string().contains("non-loopback"), "{e}");
    }
}
