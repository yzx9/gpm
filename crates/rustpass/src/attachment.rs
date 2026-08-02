// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! gopass binary attachment detection + base64 decode — gopass
//! `internal/action/binary` analogue.
//!
//! A gopass binary attachment is layered on the everyday AKV text format: the
//! decrypted plaintext is an empty password line, two attribute lines
//! (`Content-Disposition: attachment; filename="…"` and
//! `Content-Transfer-Encoding: Base64`), and a single-line base64 body. age
//! encrypts the whole plaintext exactly as for any other secret; the base64 is
//! *encoding* (so binary can live inside a line-oriented text format), not
//! encryption. Detection keys off the `Content-Transfer-Encoding: Base64`
//! attribute, matching gopass's own `isBase64Encoded` (case-insensitive value).

use std::fmt;

use base64::Engine as _;
use serde::Serialize;
use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};

/// The AKV attribute separator gopass uses to split headers from body. The
/// base64 alphabet (`A-Za-z0-9+/=`) never contains it, so the attribute/payload
/// split is unambiguous for any real attachment.
const KV_SEP: &str = ": ";
/// The attachment signal attribute key.
const CTE_KEY: &str = "Content-Transfer-Encoding";
/// The attribute key carrying the suggested filename.
const CD_KEY: &str = "Content-Disposition";

/// A decoded gopass binary attachment. The bytes are wiped on drop through
/// [`Zeroizing`]; the filename is non-secret metadata. The [`Debug`](fmt::Debug)
/// impl is hand-rolled to redact the bytes — never derive it, or a stray log
/// line leaks the attachment into the disk-persisted log pipeline.
pub struct Attachment {
    filename: Option<String>,
    bytes: Zeroizing<Vec<u8>>,
}

impl Attachment {
    /// The attachment's suggested filename from `Content-Disposition`, if any.
    /// CTE-only detection means a real attachment may carry no filename.
    #[must_use]
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    /// The decoded binary payload.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for Attachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attachment")
            .field("filename", &self.filename)
            .field("bytes", &format!("{} bytes [REDACTED]", self.bytes.len()))
            .finish()
    }
}

/// Cheap attachment metadata for display, without decoding the payload. `size`
/// is the *decoded* length, computed from the base64 length formula (exact for
/// valid standard base64); [`extract`] is authoritative on export.
#[derive(Clone, Debug, Serialize)]
pub struct AttachmentMeta {
    filename: Option<String>,
    size: u64,
}

impl AttachmentMeta {
    /// The attachment's suggested filename from `Content-Disposition`, if any.
    #[must_use]
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }
    /// The decoded byte count (computed from the base64 length, no decode).
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }
}

/// Full attachment decode for export.
///
/// Returns `Ok(None)` when `body` holds no attachment, `Err(AttachmentInvalid)`
/// when one is detected but its base64 body is undecodable. The filename is
/// `None` when `Content-Disposition` is absent — CTE-only detection still
/// treats a lone `Content-Transfer-Encoding: Base64` as an attachment, as
/// gopass does.
///
/// # Errors
///
/// [`ErrorCode::AttachmentInvalid`] when the body is signalled as an attachment
/// but its payload is not valid standard base64.
pub fn extract(body: &str) -> Result<Option<Attachment>, Error> {
    let Some(payload) = attachment_payload(body) else {
        return Ok(None);
    };
    let filename = content_disposition_filename(body);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.as_bytes())
        .map_err(|_| {
            Error::new(
                ErrorCode::AttachmentInvalid,
                "attachment body is not valid base64",
            )
        })?;
    Ok(Some(Attachment {
        filename,
        bytes: Zeroizing::new(bytes),
    }))
}

/// Whether `body` signals a binary attachment — the UI probe, without decoding.
/// Matches gopass's `isBase64Encoded`: a `Content-Transfer-Encoding: base64`
/// attribute is present (case-insensitive value). Cheap; safe to call on every
/// decrypt. Note this is a *presence* check, not a well-formedness check: a
/// body with the attribute but a malformed payload still returns `true` (the
/// affordance stays visible and the decode error surfaces on export).
#[must_use]
pub fn has_attachment(body: &str) -> bool {
    is_attachment(body)
}

/// Cheap metadata (filename + decoded size) without decoding the payload.
/// Returns `None` when `body` holds no attachment. `size` matches
/// [`extract`]'s decoded length for valid base64 (pinned by tests).
#[must_use]
pub fn metadata(body: &str) -> Option<AttachmentMeta> {
    let payload = attachment_payload(body)?;
    Some(AttachmentMeta {
        filename: content_disposition_filename(body),
        size: decoded_len(&payload),
    })
}

// ---- shared detection / split (one source of truth) ----

/// True iff a `Content-Transfer-Encoding: base64` attribute is present.
fn is_attachment(body: &str) -> bool {
    kv_value(body, CTE_KEY)
        .map(str::trim)
        .is_some_and(|v| v.eq_ignore_ascii_case("base64"))
}

/// The base64 payload of an attachment, or `None` when the body is not an
/// attachment. Mirrors gopass `Body()`: every line containing the `": "`
/// attribute separator is dropped; remaining non-empty lines are the payload,
/// with all ASCII whitespace stripped so wrapped (76-char) base64 — which
/// gopass's `StdEncoding` tolerates — decodes too.
fn attachment_payload(body: &str) -> Option<String> {
    if !is_attachment(body) {
        return None;
    }
    let mut payload = String::new();
    for line in body.lines() {
        if line.contains(KV_SEP) || line.trim().is_empty() {
            continue;
        }
        payload.push_str(line);
    }
    payload.retain(|c| !c.is_ascii_whitespace());
    Some(payload)
}

/// The trimmed value of the first `Key: Value` line for `key`, or `None`.
///
/// The key match is case-insensitive: gopass's `isBase64Encoded` accepts both
/// `Content-Transfer-Encoding` and `content-transfer-encoding` (its own docs
/// show the lowercase form), so a lowercase-keyed attachment must still be
/// detected — otherwise its base64 body would slip past detection and reach the
/// `WebView`.
fn kv_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    for line in body.lines() {
        let Some((k, v)) = line.split_once(KV_SEP) else {
            continue;
        };
        if k.eq_ignore_ascii_case(key) {
            return Some(v);
        }
    }
    None
}

/// The `filename="…"` (or bare `filename=…`) value from a `Content-Disposition`
/// attribute, or `None` when absent/unparseable.
fn content_disposition_filename(body: &str) -> Option<String> {
    parse_filename(kv_value(body, CD_KEY)?)
}

/// Decode the size of a base64 payload without decoding the bytes:
/// `(len / 4) * 3 − trailing '=' padding`. Exact for valid standard base64.
/// Uses saturating subtraction so a malformed payload (e.g. `"==="`) can't
/// underflow — `metadata()` runs on the read/probe path before `extract`
/// validates the body, so an underflow would panic (debug) or report a huge
/// bogus size (release).
fn decoded_len(payload: &str) -> u64 {
    let padding = payload.bytes().filter(|b| *b == b'=').count() as u64;
    let groups = payload.len() as u64 / 4;
    (groups * 3).saturating_sub(padding)
}

/// Parse `filename="…"` (quoted, gopass's form) or a bare `filename=token`
/// fallback out of a `Content-Disposition` value. `None` if no filename param.
fn parse_filename(cd_value: &str) -> Option<String> {
    for param in cd_value.split(';') {
        let param = param.trim();
        if let Some(rest) = param.strip_prefix("filename=") {
            if let Some(inner) = rest.strip_prefix('"') {
                // quoted: up to the closing quote
                return Some(inner.split('"').next().unwrap_or(inner).to_string());
            }
            // bare token: the remainder of this param
            return Some(rest.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    /// Build a gopass attachment body with the given base64 payload.
    fn attachment_body(b64: &str) -> String {
        format!(
            "\nContent-Disposition: attachment; filename=\"photo.png\"\nContent-Transfer-Encoding: Base64\n{b64}"
        )
    }

    // ---- detection ----

    #[test]
    fn detects_cte_base64_case_insensitively() {
        for value in ["Base64", "base64", "BASE64"] {
            let body = format!("\nContent-Transfer-Encoding: {value}\nQUJD");
            assert!(is_attachment(&body), "CTE={value} should detect");
            assert!(has_attachment(&body));
        }
    }

    #[test]
    fn detects_lowercase_cte_key_like_gopass() {
        // gopass's isBase64Encoded accepts the lowercase key too (its docs show
        // it); gpm must as well, or the base64 body slips past detection and
        // reaches the WebView.
        let body = "\ncontent-disposition: attachment; filename=\"x.bin\"\ncontent-transfer-encoding: base64\nQUJD";
        assert!(has_attachment(body));
        let meta = metadata(body).expect("lowercase-keyed attachment detected");
        assert_eq!(meta.filename(), Some("x.bin"));
        assert_eq!(meta.size(), 3);
    }

    #[test]
    fn metadata_size_does_not_underflow_on_malformed_payload() {
        // A CTE-signalled body whose payload is just padding must not underflow
        // decoded_len (panic in debug, huge bogus size in release).
        let body = "\nContent-Transfer-Encoding: Base64\n===";
        let meta = metadata(body).expect("still an attachment (CTE present)");
        assert_eq!(meta.size(), 0);
        // extract rejects the malformed payload.
        assert!(extract(body).is_err());
    }

    #[test]
    fn ignores_unrelated_or_absent_cte() {
        // No CTE at all.
        assert!(!has_attachment("pw\nusername: alice"));
        // CTE present but not base64.
        assert!(!has_attachment("pw\nContent-Transfer-Encoding: 8bit\nQUJD"));
        // A body line that merely contains the substring (not a `Key: ` attribute).
        assert!(!has_attachment(
            "pw\nnotes: see Content-Transfer-Encoding: Base64 docs"
        ));
    }

    // ---- decode round-trip (pins the from_utf8_lossy lossless assumption) ----

    #[test]
    fn extract_round_trips_all_byte_values_through_secret_parse() {
        // Every byte 0x00..=0xFF must survive Secret::parse (from_utf8_lossy on the
        // ASCII base64 is lossless) → attachment::extract bit-identical.
        let original: Vec<u8> = (0u8..=255).collect();
        let b64 = STANDARD.encode(&original);
        let plaintext = format!(
            "\nContent-Disposition: attachment; filename=\"x.bin\"\nContent-Transfer-Encoding: Base64\n{b64}\n"
        );
        let secret = crate::Secret::parse(plaintext.as_bytes()).unwrap();
        let att = extract(secret.body())
            .unwrap()
            .expect("should detect attachment");
        assert_eq!(att.bytes(), &original[..]);
        assert_eq!(att.filename(), Some("x.bin"));
    }

    #[test]
    fn extract_decodes_wrapped_multiline_base64() {
        // gopass's StdEncoding tolerates newlines; gopass-produced single-line is
        // the norm, but a wrapped (76-char) body from another tool must still decode.
        let original = vec![0xABu8; 200];
        let b64 = STANDARD.encode(&original);
        let wrapped: String = b64
            .as_bytes()
            .chunks(76)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let body = format!("\nContent-Transfer-Encoding: Base64\n{wrapped}");
        let att = extract(&body).unwrap().expect("attachment");
        assert_eq!(att.bytes(), &original[..]);
    }

    #[test]
    fn extract_errors_on_malformed_payload() {
        let body = attachment_body("!!!not base64!!!");
        let err = extract(&body).unwrap_err();
        assert_eq!(err.code, "ATTACHMENT_INVALID");
    }

    #[test]
    fn extract_none_when_not_an_attachment() {
        assert!(extract("pw\nusername: alice").unwrap().is_none());
        assert!(extract("").unwrap().is_none());
    }

    // ---- metadata size correctness (pins the formula against the real decode) ----

    #[test]
    fn metadata_size_matches_decoded_len_across_sizes() {
        for n in [1usize, 2, 3, 4, 100, 255, 1024] {
            let original = vec![0x42u8; n];
            let b64 = STANDARD.encode(&original);
            let body = attachment_body(&b64);
            let meta = metadata(&body).expect("meta for attachment");
            let ext = extract(&body).unwrap().expect("extract");
            assert_eq!(
                meta.size(),
                ext.bytes().len() as u64,
                "size mismatch at n={n}"
            );
            assert_eq!(meta.filename(), Some("photo.png"));
        }
    }

    #[test]
    fn metadata_none_when_not_an_attachment() {
        assert!(metadata("pw\nusername: alice").is_none());
    }

    // ---- filename parsing ----

    #[test]
    fn parses_quoted_filename_gopass_form() {
        let body = attachment_body("QUJD");
        assert_eq!(
            extract(&body).unwrap().unwrap().filename(),
            Some("photo.png")
        );
    }

    #[test]
    fn parses_bare_token_filename() {
        let body = "\nContent-Disposition: attachment; filename=report.pdf\nContent-Transfer-Encoding: Base64\nQUJD";
        assert_eq!(
            extract(body).unwrap().unwrap().filename(),
            Some("report.pdf")
        );
    }

    #[test]
    fn filename_none_when_content_disposition_absent() {
        // CTE-only: still an attachment, but no filename.
        let body = "\nContent-Transfer-Encoding: Base64\nQUJD";
        let att = extract(body).unwrap().expect("attachment");
        assert_eq!(att.filename(), None);
        assert_eq!(att.bytes(), b"ABC");
    }

    // ---- presence tracking ----

    #[test]
    fn has_attachment_tracks_cte_presence() {
        // CTE present (even with a malformed payload) → true; the decode error
        // surfaces only on export, mirroring totp::has_totp's present-but-broken rule.
        assert!(has_attachment(&attachment_body("!!!bad!!!")));
        assert!(has_attachment(&attachment_body("QUJD")));
        // No CTE → false.
        assert!(!has_attachment("pw\nusername: alice"));
    }

    // ---- redaction ----

    #[test]
    fn attachment_debug_redacts_bytes() {
        let att = extract(&attachment_body("QUJD"))
            .unwrap()
            .expect("attachment");
        let s = format!("{att:?}");
        assert!(s.contains("[REDACTED]"));
        assert!(!s.contains("QUJD"));
        // filename is non-secret metadata, so it may appear.
        assert!(s.contains("photo.png"));
    }
}
