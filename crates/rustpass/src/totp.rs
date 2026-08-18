// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

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

use totp_rs::{Algorithm, Builder, Totp};
use zeroize::Zeroizing;

use crate::Secret;
use crate::error::{Error, ErrorCode};

/// A parsed TOTP configuration, ready to mint codes. Wraps a [`totp_rs::Totp`];
/// the inner seed is wiped on drop through totp-rs's `zeroize` feature. The
/// [`Debug`](fmt::Debug) impl is hand-rolled to redact the seed — never derive
/// it, or a stray log line leaks the seed into the disk-persisted log pipeline.
pub struct Otp(Totp);

impl fmt::Debug for Otp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Otp")
            .field("secret", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// Extract a TOTP configuration from a gopass secret, in gopass's priority
/// order (first match wins):
///
/// 1. an `otpauth:` key whose value is a full `otpauth://totp/...` URI;
/// 2. a body line beginning with `otpauth://`;
/// 3. a `totp:` key whose value is a bare base32 secret (all-default params).
///
/// The `otpauth`/`totp` keys are matched exact-case (gopass's `pkg/otp` uses
/// exact lowercase `Get`), via [`Secret::attribute_str`]. Returns `Ok(None)`
/// when the body holds no TOTP seed (not an error). HOTP seeds
/// (`otpauth://hotp/...`, `hotp:` keys) are not TOTP and surface as an error or
/// `Ok(None)` rather than a silently miscomputed code.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] when a candidate line is present but
/// malformed, references an unsupported OTP type / encoder / algorithm, or
/// carries an out-of-range parameter. Messages never contain the seed or URI.
pub fn extract(secret: &Secret) -> Result<Option<Otp>, Error> {
    // A legacy-YAML secret (A004) has no attributes — its `k: v` lines stay in
    // the opaque body — so scan the body lines for the same `otpauth:`/`totp:`
    // candidates the attribute lookups would have found. Line-level byte
    // matching only; the YAML block itself is never parsed.
    let (otpauth, totp): (Option<String>, Option<String>) = if secret.is_yaml() {
        (
            yaml_body_value(secret.body_bytes(), "otpauth"),
            yaml_body_value(secret.body_bytes(), "totp"),
        )
    } else {
        (
            secret
                .attribute_str("otpauth")
                .map(str::trim)
                .map(str::to_string),
            secret
                .attribute_str("totp")
                .map(str::trim)
                .map(str::to_string),
        )
    };
    // 1 & 2: an otpauth:// URI — as a key value, then as a standalone body line.
    if let Some(uri) =
        otpauth.or_else(|| first_otpauth_line(secret.body_bytes()).map(str::to_string))
    {
        return Ok(Some(from_uri(uri.as_str())?));
    }
    // 3: a bare base32 secret under `totp:`. (`hotp:` is HOTP — not matched.)
    if let Some(secret_str) = totp {
        if secret_str.is_empty() {
            return Err(Error::new(ErrorCode::StoreError, "TOTP seed is empty"));
        }
        return Ok(Some(from_bare_secret(secret_str.as_str())?));
    }
    Ok(None)
}

/// Whether `secret` carries a TOTP seed — the UI's "does this entry have a 2FA
/// code?" probe, without minting one. Returns `true` whenever [`extract`] finds
/// a seed (`Ok(Some)`) **or** hits a candidate that failed to parse (`Err`): a
/// malformed `otpauth:`/`totp:` line still signals 2FA intent, so the affordance
/// stays visible and the parse error surfaces when the user actually requests a
/// code. Returns `false` only on a clean `Ok(None)` — no seed at all.
#[must_use]
pub fn has_totp(secret: &Secret) -> bool {
    !matches!(extract(secret), Ok(None))
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
    // `generate` returns a stack-allocated `Token` that wipes itself on drop
    // (zeroize feature); the `Zeroizing` guards the formatted `String` copy.
    Ok(Zeroizing::new(otp.0.generate(secs).to_string()))
}

/// Build an [`Otp`] from a full `otpauth://` URI.
///
/// `totp-rs` parses the URI and validates `digits` (6..=8), `algorithm`
/// (SHA1/256/512), secret size (≥ 128 bits), and a non-zero time step. It does
/// **not** validate the OTP type segment or the `encoder` query param — so we
/// guard those ourselves: a HOTP URI or a Steam Guard encoder is rejected here
/// rather than silently producing a code that never matches the server.
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
    Ok(Otp(Totp::from_url(uri).map_err(parse_err)?))
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
    let secret = totp_rs::Secret::try_from_base32(normalized).map_err(parse_err)?;
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_secret(secret)
        .with_digits(6)
        .with_skew(0)
        .with_step_duration(30)
        .with_account_name("gpm")
        .build()
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

/// The first body line (trimmed) that begins with `otpauth://`.
fn first_otpauth_line(body: &[u8]) -> Option<&str> {
    body.split(|&b| b == b'\n').find_map(|line| {
        let s = std::str::from_utf8(line.trim_ascii()).ok()?;
        s.starts_with("otpauth://").then_some(s)
    })
}

/// The value of the first TOP-LEVEL `key: value` body line. Only used for
/// legacy-YAML secrets, whose `k: v` lines live in the body instead of the
/// attribute region — this restores the TOTP detection those secrets had
/// before the YAML branch stopped attribute splitting. Two constraints keep
/// it at parity with both the old attribute split and gopass `Get`: the key
/// sits at column 0 (an indented key belongs to a nested mapping gopass's
/// top-level lookup does not read), and a space follows the colon (so a bare
/// `otpauth://…` line is left for [`first_otpauth_line`] instead of being
/// mangled by the scheme's own colon).
fn yaml_body_value(body: &[u8], key: &str) -> Option<String> {
    let prefix = format!("{key}: ");
    body.split(|&b| b == b'\n').find_map(|line| {
        let s = std::str::from_utf8(line).ok()?;
        if s.starts_with([' ', '\t']) {
            return None;
        }
        s.strip_prefix(prefix.as_str())
            .map(str::trim)
            .map(str::to_string)
    })
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
    use std::time::Duration;

    use super::*;

    /// Parse a secret plaintext the way the read path does, for detector tests.
    fn sec(body: &str) -> Secret {
        Secret::parse(body.as_bytes()).unwrap()
    }

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
            let totp = Builder::new()
                .with_algorithm(*alg)
                .with_secret(seed.clone())
                .with_digits(8)
                .with_skew(0)
                .with_step_duration(30)
                .with_account_name("t")
                .build()
                .unwrap();
            let otp = Otp(totp);
            let got = generate_at(&otp, UNIX_EPOCH + Duration::from_secs(*t)).unwrap();
            assert_eq!(&*got, *expected, "RFC 6238 {alg:?} @ t={t}");
        }
    }

    #[test]
    fn generate_at_errors_before_epoch() {
        let secret =
            totp_rs::Secret::try_from_base32("KRSXG5CTMVRXEZLUKN2XAZLSKNSWG4TFOQ").unwrap();
        let otp = Otp(Builder::new()
            .with_algorithm(Algorithm::SHA1)
            .with_secret(secret)
            .with_skew(0)
            .with_account_name("t")
            .build()
            .unwrap());
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
            extract(&sec(&format!(
                "pw\notpauth: otpauth://totp/Ex:a?secret={SECRET}"
            )))
            .unwrap()
            .is_some()
        );
        assert!(
            extract(&sec(&format!("pw\notpauth://totp/Ex:a?secret={SECRET}")))
                .unwrap()
                .is_some()
        );
        assert!(
            extract(&sec(&format!("pw\ntotp: {SECRET}")))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn extract_bare_secret_matches_direct_construction() {
        // Proves the `totp:` path wires the secret correctly, without hand-computing
        // a code: extract must agree with a directly-built TOTP for the same secret.
        let body = format!("pw\ntotp: {SECRET}");
        let extracted = extract(&sec(&body)).unwrap().unwrap();
        let direct = Otp(Builder::new()
            .with_algorithm(Algorithm::SHA1)
            .with_secret(totp_rs::Secret::try_from_base32(SECRET).unwrap())
            .with_digits(6)
            .with_skew(0)
            .with_step_duration(30)
            .with_account_name("gpm")
            .build()
            .unwrap());
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
        let otp = extract(&sec(&body))
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
        let otp = extract(&sec(&body)).unwrap().unwrap();
        let code = generate_at(&otp, UNIX_EPOCH + Duration::from_mins(1)).unwrap();
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
    }

    // ---- rejection: never a silent wrong code ----

    #[test]
    fn extract_rejects_hotp_uri_and_treats_hotp_kv_as_none() {
        assert!(
            extract(&sec(&format!(
                "pw\notpauth://hotp/A:x?secret={SECRET}&counter=1"
            )))
            .is_err()
        );
        assert!(
            extract(&sec(&format!("pw\nhotp: {SECRET}")))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn extract_rejects_steam_encoder() {
        assert!(
            extract(&sec(&format!(
                "pw\notpauth://totp/A:x?secret={SECRET}&encoder=steam"
            )))
            .is_err()
        );
    }

    #[test]
    fn extract_rejects_zero_period() {
        // totp-rs 6 rejects a zero step at build time (InvalidStepZero); pinned
        // here so a future upstream relaxation can't reintroduce a divide-by-zero.
        assert!(
            extract(&sec(&format!(
                "pw\notpauth://totp/A:x?secret={SECRET}&period=0"
            )))
            .is_err()
        );
    }

    #[test]
    fn extract_rejects_bad_digits_via_totp_rs() {
        assert!(
            extract(&sec(&format!(
                "pw\notpauth://totp/A:x?secret={SECRET}&digits=4"
            )))
            .is_err()
        );
        assert!(
            extract(&sec(&format!(
                "pw\notpauth://totp/A:x?secret={SECRET}&digits=99"
            )))
            .is_err()
        );
    }

    #[test]
    fn extract_rejects_unsupported_algorithm_via_totp_rs() {
        assert!(
            extract(&sec(&format!(
                "pw\notpauth://totp/A:x?secret={SECRET}&algorithm=MD5"
            )))
            .is_err()
        );
    }

    #[test]
    fn extract_rejects_short_secret_via_totp_rs() {
        // < 16 bytes (128 bits) → totp-rs SecretTooShort error. This is the documented
        // gopass divergence: gopass accepts these, gpm does not. The canonical toy
        // secret JBSWY3DPEHPK3PXP (10 bytes) is rejected for the same reason.
        assert!(extract(&sec("pw\ntotp: ABCD")).is_err());
        assert!(extract(&sec("pw\ntotp: JBSWY3DPEHPK3PXP")).is_err());
    }

    #[test]
    fn extract_none_when_no_seed() {
        assert!(
            extract(&sec("pw\nusername: alice\nurl: example.com"))
                .unwrap()
                .is_none()
        );
        assert!(extract(&sec("just a password")).unwrap().is_none());
        // An empty body (password-only secret) carries no seed.
        assert!(extract(&sec("pw")).unwrap().is_none());
    }

    #[test]
    fn has_totp_tracks_extract_presence() {
        // Clean seed → true (otpauth, bare, all match extract's Some branch).
        assert!(has_totp(&sec(&format!(
            "pw\notpauth: otpauth://totp/Ex:a?secret={SECRET}"
        ))));
        assert!(has_totp(&sec(&format!(
            "pw\notpauth://totp/Ex:a?secret={SECRET}"
        ))));
        assert!(has_totp(&sec(&format!("pw\ntotp: {SECRET}"))));
        // No seed anywhere → false.
        assert!(!has_totp(&sec("pw\nusername: alice\nurl: example.com")));
        assert!(!has_totp(&sec("just a password")));
        assert!(!has_totp(&sec("pw")));
    }

    #[test]
    fn has_totp_true_for_malformed_seed() {
        // A present-but-broken candidate (HOTP URI) is an `Err`, not `Ok(None)`:
        // the entry signals 2FA intent, so the affordance stays visible and the
        // error surfaces on a real copy attempt.
        assert!(has_totp(&sec(&format!(
            "pw\notpauth://hotp/A:x?secret={SECRET}&counter=1"
        ))));
        assert!(has_totp(&sec("pw\ntotp: ABCD")));
    }

    #[test]
    fn otp_debug_redacts_secret() {
        let otp = extract(&sec(&format!("pw\ntotp: {SECRET}")))
            .unwrap()
            .unwrap();
        let s = format!("{otp:?}");
        assert!(s.contains("[REDACTED]"));
        assert!(!s.contains(SECRET));
    }

    // ---- TOTP inside a legacy-YAML block (A004 keeps detection working) ----

    /// Generate at a fixed timestamp so two secrets parsed from different
    /// shapes can be compared on the code they produce, not on Otp identity.
    fn code_now(secret: &Secret) -> String {
        let otp = extract(secret).unwrap().unwrap();
        generate_at(&otp, SystemTime::UNIX_EPOCH)
            .unwrap()
            .to_string()
    }

    #[test]
    fn yaml_secret_totp_bare_seed_is_extracted() {
        // Pre-A004 the `totp:` line was (accidentally) lifted into attributes;
        // the YAML branch keeps it in the body, so the body scan must find it
        // and produce the same code as the AKV shape.
        let yaml = sec(&format!("pw\n---\ntotp: {SECRET}"));
        assert!(yaml.is_yaml());
        let akv = sec(&format!("pw\ntotp: {SECRET}"));
        assert!(!akv.is_yaml());
        assert_eq!(code_now(&yaml), code_now(&akv));
        assert!(has_totp(&yaml));
    }

    #[test]
    fn yaml_secret_otpauth_uri_is_extracted() {
        let uri = format!("otpauth://totp/x?secret={SECRET}");
        let yaml = sec(&format!("pw\n---\notpauth: {uri}"));
        assert!(yaml.is_yaml());
        let akv = sec(&format!("pw\notpauth: {uri}"));
        assert!(!akv.is_yaml());
        assert_eq!(code_now(&yaml), code_now(&akv));
        assert!(has_totp(&yaml));
    }

    #[test]
    fn yaml_secret_without_seed_has_no_totp() {
        // A plain YAML block carries no 2FA intent — no candidates, no probe.
        let secret = sec("pw\n---\nusername: alice");
        assert!(secret.is_yaml());
        assert!(matches!(extract(&secret), Ok(None)));
        assert!(!has_totp(&secret));
    }

    #[test]
    fn yaml_secret_malformed_seed_still_signals_intent() {
        // HOTP inside a YAML block: `Err`, so the affordance stays visible —
        // same contract as the AKV path. (An `otpauth:`-prefixed value that is
        // itself a hotp URI errors in from_uri.)
        let secret = sec(&format!(
            "pw\n---\notpauth: otpauth://hotp/x?secret={SECRET}&counter=1"
        ));
        assert!(secret.is_yaml());
        assert!(has_totp(&secret));
    }

    #[test]
    fn yaml_secret_bare_otpauth_line_still_works() {
        // A bare `otpauth://…` line in a YAML body must fall through to the
        // whole-line scan (`otpauth: ` with its scheme colon is NOT a `k: v`
        // separator) — pre-A004 the AKV split found this line and produced a
        // code; mangling it to `//totp/…` would regress that to a parse error.
        let uri = format!("otpauth://totp/x?secret={SECRET}");
        let yaml = sec(&format!("pw\n---\nusername: alice\n{uri}"));
        assert!(yaml.is_yaml());
        let akv = sec(&format!("pw\nusername: alice\n{uri}"));
        assert!(!akv.is_yaml());
        assert_eq!(code_now(&yaml), code_now(&akv));
    }

    #[test]
    fn yaml_secret_indented_totp_key_not_matched() {
        // An indented `totp:` is a NESTED mapping key gopass's top-level
        // `Get("totp")` does not read — and pre-A004 the attribute split
        // (key = bytes before `": "`, untrimmed) did not match it either. It
        // must not surface a nested (possibly different service's) seed.
        let secret = sec(&format!("pw\n---\nlogin:\n  totp: {SECRET}"));
        assert!(secret.is_yaml());
        assert!(matches!(extract(&secret), Ok(None)));
        assert!(!has_totp(&secret));
    }
}
