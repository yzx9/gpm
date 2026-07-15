// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier-Identifier: Apache-2.0

//! TOTP (RFC 6238) generation — gopass `pkg/otp` analogue.
//!
//! Reads a TOTP seed stored in a gopass secret's body and produces the current
//! one-time code. The seed is conventionally an `otpauth://totp/...` URI or a
//! bare base32 secret under a `totp:` key — the format gopass, Bitwarden, and
//! authenticator apps exchange. Code generation is delegated to the audited
//! [`totp_rs`] crate; this module owns only the gopass body extraction plus the
//! validation the crate does not perform (see [`extract`]).

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use totp_rs::{Algorithm, Secret, TOTP};
use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};

/// A parsed TOTP configuration, ready to mint codes. Wraps a [`totp_rs::TOTP`];
/// the inner seed is wiped on drop through totp-rs's `zeroize` feature. The
/// [`Debug`](fmt::Debug) impl is hand-rolled to redact the seed — never derive
/// it, or a stray log line leaks the seed into the disk-persisted log pipeline.
pub struct Otp(TOTP);

impl fmt::Debug for Otp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Otp")
            .field("secret", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// Extract a TOTP configuration from a gopass secret body, in gopass's
/// priority order (first match wins):
///
/// 1. an `otpauth:` key whose value is a full `otpauth://totp/...` URI;
/// 2. a body line beginning with `otpauth://`;
/// 3. a `totp:` key whose value is a bare base32 secret (all-default params).
///
/// Returns `Ok(None)` when the body holds no TOTP seed (not an error). HOTP
/// seeds (`otpauth://hotp/...`, `hotp:` keys) are not TOTP and surface as an
/// error or `Ok(None)` rather than a silently miscomputed code.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] when a candidate line is present but
/// malformed, references an unsupported OTP type / encoder / algorithm, or
/// carries an out-of-range parameter. Messages never contain the seed or URI.
pub fn extract(body: &str) -> Result<Option<Otp>, Error> {
    // 1 & 2: an otpauth:// URI — as a key value, then as a standalone body line.
    if let Some(uri) = kv_value(body, "otpauth")
        .map(str::trim)
        .or_else(|| first_otpauth_line(body))
    {
        return Ok(Some(from_uri(uri)?));
    }
    // 3: a bare base32 secret under `totp:`. (`hotp:` is HOTP — not matched.)
    if let Some(secret) = kv_value(body, "totp").map(str::trim) {
        if secret.is_empty() {
            return Err(Error::new(ErrorCode::StoreError, "TOTP seed is empty"));
        }
        return Ok(Some(from_bare_secret(secret)?));
    }
    Ok(None)
}

/// Produce the current one-time code at `now`. `now` is a parameter (not read
/// from the system clock internally) so RFC 6238 vectors can drive it
/// deterministically in tests.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] only if `now` precedes the Unix epoch.
pub fn generate_at(otp: &Otp, now: SystemTime) -> Result<Zeroizing<String>, Error> {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            Error::new(
                ErrorCode::StoreError,
                "system clock precedes the Unix epoch",
            )
        })?
        .as_secs();
    Ok(Zeroizing::new(otp.0.generate(secs)))
}

/// Build an [`Otp`] from a full `otpauth://` URI.
///
/// `totp-rs` parses the URI and validates `digits` (6..=8), `algorithm`
/// (SHA1/256/512), and secret size (≥ 128 bits). It does **not** validate the
/// OTP type segment, the `encoder` query param, or the time step — so we guard
/// those ourselves: a HOTP URI, a Steam Guard encoder, or a zero period are
/// rejected here rather than silently producing a code that never matches the
/// server.
fn from_uri(uri: &str) -> Result<Otp, Error> {
    if type_segment(uri).is_some_and(|t| !t.eq_ignore_ascii_case("totp")) {
        return Err(Error::new(
            ErrorCode::StoreError,
            "only TOTP seeds are supported (HOTP is not)",
        ));
    }
    if has_query_param(uri, "encoder") {
        return Err(Error::new(
            ErrorCode::StoreError,
            "Steam Guard / non-default OTP encoders are not supported",
        ));
    }
    let totp = TOTP::from_url(uri).map_err(parse_err)?;
    // totp-rs does not validate step; its `generate` divides by it.
    if totp.step == 0 {
        return Err(Error::new(
            ErrorCode::StoreError,
            "TOTP period must be at least 1 second",
        ));
    }
    Ok(Otp(totp))
}

/// Build an [`Otp`] from a bare base32 secret (the `totp:` key path) using
/// gopass defaults: SHA1, 6 digits, 30-second period. Whitespace, base32
/// padding (`=`), and case are normalized first — gopass accepts all three.
fn from_bare_secret(secret: &str) -> Result<Otp, Error> {
    let normalized: String = secret
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '=')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if normalized.is_empty() {
        return Err(Error::new(
            ErrorCode::StoreError,
            "TOTP seed has no base32 characters",
        ));
    }
    let bytes = Secret::Encoded(normalized).to_bytes().map_err(parse_err)?;
    TOTP::new(Algorithm::SHA1, 6, 0, 30, bytes, None, "gpm".to_string())
        .map(Otp)
        .map_err(parse_err)
}

/// The OTP type segment of an `otpauth://` URI (`totp`, `hotp`, …), or `None`
/// when the URI does not start with `otpauth://`.
fn type_segment(uri: &str) -> Option<&str> {
    let rest = uri.strip_prefix("otpauth://")?;
    Some(rest.split('/').next().unwrap_or(rest))
}

/// Whether an `otpauth://` URI's query string carries the named parameter.
fn has_query_param(uri: &str, name: &str) -> bool {
    let Some(query) = uri.split_once('?').map(|(_, q)| q) else {
        return false;
    };
    let needle = format!("{name}=");
    query
        .split('&')
        .any(|pair| pair.starts_with(needle.as_str()))
}

/// The trimmed value of the first `key: value` line for `key`, or `None`.
fn kv_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}: ");
    body.lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
}

/// The first body line (trimmed) that begins with `otpauth://`.
fn first_otpauth_line(body: &str) -> Option<&str> {
    body.lines()
        .map(str::trim)
        .find(|line| line.starts_with("otpauth://"))
}

/// Map any `totp-rs` parse error to a safe [`Error`]. The detail is discarded:
/// an error `Display` could echo input, and `Error.message` crosses IPC and is
/// logged to disk, so we never forward the underlying error text.
fn parse_err<E>(_e: E) -> Error {
    Error::new(
        ErrorCode::StoreError,
        "TOTP seed could not be parsed (bad otpauth URI, digits, algorithm, or secret)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ---- generation: RFC 6238 Appendix B vectors (direct construction) ----

    #[test]
    fn generate_at_matches_rfc6238_vectors() {
        // Seed = ASCII "12345678901234567890" extended to the algorithm's block
        // size (20/32/64 bytes). 8 digits, 30s period. Asserts generate_at wires
        // totp-rs correctly against the canonical reference codes.
        let sha1_seed = b"12345678901234567890".to_vec();
        let sha256_seed = format!("{}{}", "1234567890".repeat(3), "12").into_bytes();
        let sha512_seed = format!("{}{}", "1234567890".repeat(6), "1234").into_bytes();
        let cases: &[(Algorithm, Vec<u8>, u64, &str)] = &[
            (Algorithm::SHA1, sha1_seed.clone(), 59, "94287082"),
            (
                Algorithm::SHA1,
                sha1_seed.clone(),
                1_111_111_109,
                "07081804",
            ),
            (Algorithm::SHA1, sha1_seed, 1_234_567_890, "89005924"),
            (Algorithm::SHA256, sha256_seed.clone(), 59, "46119246"),
            (Algorithm::SHA256, sha256_seed, 1_111_111_109, "68084774"),
            (Algorithm::SHA512, sha512_seed.clone(), 59, "90693936"),
            (Algorithm::SHA512, sha512_seed, 1_111_111_111, "99943326"),
        ];
        for (alg, seed, t, expected) in cases {
            let totp = TOTP::new(*alg, 8, 0, 30, seed.clone(), None, "t".to_string()).unwrap();
            let otp = Otp(totp);
            let got = generate_at(&otp, UNIX_EPOCH + Duration::from_secs(*t)).unwrap();
            assert_eq!(&*got, *expected, "RFC 6238 {alg:?} @ t={t}");
        }
    }

    #[test]
    fn generate_at_errors_before_epoch() {
        let bytes = Secret::Encoded("KRSXG5CTMVRXEZLUKN2XAZLSKNSWG4TFOQ".to_string())
            .to_bytes()
            .unwrap();
        let otp = Otp(TOTP::new(Algorithm::SHA1, 6, 0, 30, bytes, None, "t".to_string()).unwrap());
        assert!(generate_at(&otp, SystemTime::UNIX_EPOCH - Duration::from_secs(1)).is_err());
    }

    // ---- extraction: gopass priority order ----

    /// A real-world-size (20-byte / 160-bit) base32 secret that totp-rs accepts.
    /// The canonical toy secret `JBSWY3DPEHPK3PXP` is only 10 bytes and is
    /// rejected by totp-rs's ≥128-bit floor — see `extract_rejects_short_secret`.
    const SECRET: &str = "KRSXG5CTMVRXEZLUKN2XAZLSKNSWG4TFOQ";

    #[test]
    fn extract_priority_otpauth_kv_then_body_line_then_totp_kv() {
        assert!(
            extract(&format!("pw\notpauth: otpauth://totp/Ex:a?secret={SECRET}"))
                .unwrap()
                .is_some()
        );
        assert!(
            extract(&format!("pw\notpauth://totp/Ex:a?secret={SECRET}"))
                .unwrap()
                .is_some()
        );
        assert!(extract(&format!("pw\ntotp: {SECRET}")).unwrap().is_some());
    }

    #[test]
    fn extract_bare_secret_matches_direct_construction() {
        // Proves the `totp:` path wires the secret correctly, without hand-computing
        // a code: extract must agree with a directly-built TOTP for the same secret.
        let body = format!("pw\ntotp: {SECRET}");
        let extracted = extract(&body).unwrap().unwrap();
        let bytes = Secret::Encoded(SECRET.to_string()).to_bytes().unwrap();
        let direct =
            Otp(TOTP::new(Algorithm::SHA1, 6, 0, 30, bytes, None, "gpm".to_string()).unwrap());
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(
            &*generate_at(&extracted, now).unwrap(),
            &*generate_at(&direct, now).unwrap()
        );
    }

    #[test]
    fn extract_normalizes_lowercase_whitespace_and_padding() {
        let lower = SECRET.to_ascii_lowercase();
        let body = format!("pw\ntotp: {lower} ==");
        let otp = extract(&body)
            .unwrap()
            .expect("lowercase/whitespace/padded secret should extract");
        let code = generate_at(&otp, UNIX_EPOCH + Duration::from_secs(1_700_000_000)).unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
    }

    #[test]
    fn extract_custom_params_round_trip() {
        // digits=8, period=60, SHA256 → a valid 8-digit code.
        let body =
            format!("pw\notpauth://totp/Ex:a?secret={SECRET}&algorithm=SHA256&digits=8&period=60");
        let otp = extract(&body).unwrap().unwrap();
        let code = generate_at(&otp, UNIX_EPOCH + Duration::from_mins(1)).unwrap();
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
    }

    // ---- rejection: never a silent wrong code ----

    #[test]
    fn extract_rejects_hotp_uri_and_treats_hotp_kv_as_none() {
        assert!(extract(&format!("pw\notpauth://hotp/A:x?secret={SECRET}&counter=1")).is_err());
        assert!(extract(&format!("pw\nhotp: {SECRET}")).unwrap().is_none());
    }

    #[test]
    fn extract_rejects_steam_encoder() {
        assert!(
            extract(&format!(
                "pw\notpauth://totp/A:x?secret={SECRET}&encoder=steam"
            ))
            .is_err()
        );
    }

    #[test]
    fn extract_rejects_zero_period() {
        // totp-rs accepts period=0 from the URI; we reject before generate divides by zero.
        assert!(extract(&format!("pw\notpauth://totp/A:x?secret={SECRET}&period=0")).is_err());
    }

    #[test]
    fn extract_rejects_bad_digits_via_totp_rs() {
        assert!(extract(&format!("pw\notpauth://totp/A:x?secret={SECRET}&digits=4")).is_err());
        assert!(extract(&format!("pw\notpauth://totp/A:x?secret={SECRET}&digits=99")).is_err());
    }

    #[test]
    fn extract_rejects_unsupported_algorithm_via_totp_rs() {
        assert!(
            extract(&format!(
                "pw\notpauth://totp/A:x?secret={SECRET}&algorithm=MD5"
            ))
            .is_err()
        );
    }

    #[test]
    fn extract_rejects_short_secret_via_totp_rs() {
        // < 16 bytes (128 bits) → totp-rs SecretSize error. This is the documented
        // gopass divergence: gopass accepts these, gpm does not. The canonical toy
        // secret JBSWY3DPEHPK3PXP (10 bytes) is rejected for the same reason.
        assert!(extract("pw\ntotp: ABCD").is_err());
        assert!(extract("pw\ntotp: JBSWY3DPEHPK3PXP").is_err());
    }

    #[test]
    fn extract_none_when_no_seed() {
        assert!(
            extract("pw\nusername: alice\nurl: example.com")
                .unwrap()
                .is_none()
        );
        assert!(extract("just a password").unwrap().is_none());
        assert!(extract("").unwrap().is_none());
    }

    #[test]
    fn otp_debug_redacts_secret() {
        let otp = extract(&format!("pw\ntotp: {SECRET}")).unwrap().unwrap();
        let s = format!("{otp:?}");
        assert!(s.contains("[REDACTED]"));
        assert!(!s.contains(SECRET));
    }
}
