//! A fake `gh` for tests.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub fn cache_key(endpoint: &str) -> String {
    endpoint
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// A fake `gh` in a temp dir. It logs every call (arguments) to `log`
/// and answers from files named after the endpoint: `resp.<key>` (with
/// `exit.<key>` for its exit status), or `r304.<key>` (exit status 1)
/// when the call carries `If-None-Match`; `fail` makes every call fail
/// with the text in it on stderr and no status line.
pub struct FakeGh {
    pub dir: tempfile::TempDir,
}

impl FakeGh {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("gh");
        std::fs::write(
            &p,
            r#"#!/bin/sh
d=$(dirname "$0")
echo "$*" >> "$d/log"
for a; do last=$a; done
key=$(printf %s "$last" | tr -c 'A-Za-z0-9' '_')
if [ -f "$d/fail" ]; then cat "$d/fail" >&2; exit 4; fi
if [ -f "$d/sleep" ]; then sleep 5; fi
case "$*" in *If-None-Match*) if [ -f "$d/r304.$key" ]; then cat "$d/r304.$key"; exit 1; fi;; esac
if [ -f "$d/resp.$key" ]; then cat "$d/resp.$key"; exit $(cat "$d/exit.$key" 2>/dev/null || echo 0); fi
echo "no canned response for $last" >&2
exit 1
"#,
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        FakeGh { dir }
    }

    pub fn program(&self) -> PathBuf {
        self.dir.path().join("gh")
    }

    /// A `200` with an ETag and extra headers.
    pub fn ok(&self, endpoint: &str, etag: &str, extra: &str, body: &str) {
        let text = format!(
            "HTTP/2.0 200 OK\r\nEtag: {etag}\r\n{extra}Content-Type: application/json\r\n\r\n{body}"
        );
        std::fs::write(
            self.dir
                .path()
                .join(format!("resp.{}", cache_key(endpoint))),
            text,
        )
        .unwrap();
    }

    pub fn raw(&self, endpoint: &str, text: &str, exit: i32) {
        let k = cache_key(endpoint);
        std::fs::write(self.dir.path().join(format!("resp.{k}")), text).unwrap();
        std::fs::write(self.dir.path().join(format!("exit.{k}")), exit.to_string()).unwrap();
    }

    pub fn not_modified(&self, endpoint: &str, extra: &str) {
        let text = format!("HTTP/2.0 304 Not Modified\r\n{extra}\r\n");
        std::fs::write(
            self.dir
                .path()
                .join(format!("r304.{}", cache_key(endpoint))),
            text,
        )
        .unwrap();
    }

    pub fn log(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}
