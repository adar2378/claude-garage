//! Proof that a local listener is the real daemon before a client hands it
//! the UI token. Anything can bind a free loopback port, and the token in
//! `<GARAGE_DIR>/ui-token` is a bearer secret, so a client first sends a
//! random nonce (no credentials) to `POST /api/plugins/ui/challenge`. The
//! daemon answers `{"proof": HMAC-SHA256(ui_token, "garage-ui-challenge:v1:daemon->client:<port>:<nonce>")}`
//! with the port it is bound to, and the client checks that against its own
//! copy of the token and the port it connected to. Only a holder of the token
//! can produce a matching proof, and binding the port means a proof relayed
//! from a daemon on another port does not verify.

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const CHALLENGE_PATH: &str = "/api/plugins/ui/challenge";
const DOMAIN: &str = "garage-ui-challenge:v1:daemon->client:";
const BLOCK: usize = 64;

/// HMAC-SHA256 (RFC 2104), lowercase hex.
pub fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    hex(&outer.finalize())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 32 random bytes as hex.
pub fn new_nonce() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| format!("no randomness available: {e}"))?;
    Ok(hex(&b))
}

/// A nonce the daemon will answer: 16 to 128 lowercase hex digits.
pub fn nonce_ok(n: &str) -> bool {
    (16..=128).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The answer of the daemon bound to `port` for `nonce`.
pub fn proof_for(token: &str, port: u16, nonce: &str) -> String {
    hmac_sha256_hex(token.trim().as_bytes(), format!("{DOMAIN}{port}:{nonce}").as_bytes())
}

/// Constant-time equality for equal-purpose hex strings.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Does the response body of the challenge route prove the peer on `port`
/// holds `token`? The body must be exactly one JSON object with a string `proof`.
pub fn verify_response(token: &str, port: u16, nonce: &str, body: &str) -> bool {
    if token.trim().is_empty() {
        return false;
    }
    let Ok(Value::Object(o)) = serde_json::from_str::<Value>(body) else { return false };
    o.get("proof").and_then(Value::as_str).is_some_and(|p| same(p, &proof_for(token, port, nonce)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_the_rfc_4231_vectors_including_a_key_longer_than_a_block() {
        assert_eq!(
            hmac_sha256_hex(&[0x0b; 20], b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hmac_sha256_hex(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First"),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn only_the_token_holder_produces_a_proof_that_verifies_and_the_body_is_parsed_strictly() {
        let n = new_nonce().unwrap();
        assert!(nonce_ok(&n) && n.len() == 64 && n != new_nonce().unwrap());
        let good = format!(r#"{{"proof":"{}"}}"#, proof_for("secret\n", 4747, &n));
        assert!(verify_response("secret", 4747, &n, &good), "surrounding whitespace in the token file is ignored");
        assert!(!verify_response("other", 4747, &n, &good));
        assert!(!verify_response("secret", 4747, &new_nonce().unwrap(), &good), "a proof for another nonce is a replay");
        assert!(!verify_response("secret", 4748, &n, &good), "a proof relayed from a daemon on another port is refused");
        assert!(!verify_response("", 4747, &n, r#"{"proof":""}"#));
        for bad in ["", "not json", "[]", r#"{"proof":1}"#, r#"{"ok":true}"#, &format!("<html>{good}</html>"), &format!("{good} trailing")] {
            assert!(!verify_response("secret", 4747, &n, bad), "{bad:?}");
        }
        assert!(!nonce_ok("short") && !nonce_ok(&"G".repeat(32)) && !nonce_ok(&"a".repeat(129)));
    }
}
