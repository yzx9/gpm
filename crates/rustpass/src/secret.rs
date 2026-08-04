// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::fmt;

use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};

/// A decrypted secret — aligned with `gopass.Secret`.
///
/// In memory: first line = password, remainder = body (key-value pairs +
/// freeform notes). [`Secret::parse`] also reads the deprecated
/// `GOPASS-SECRET-1.0` format (which carries the password in a `Password:`
/// header) and normalizes it into this shape; see [`parse_legacy`].
///
/// Storage is **bytes** (`Zeroizing<Vec<u8>>`), not `String`. gopass's on-disk
/// format is byte-oriented (Go strings are byte slices) and a secret's body may
/// contain non-UTF-8 bytes; `String` storage would force `from_utf8_lossy` on
/// read, silently corrupting such a secret on the next edit-write. Instead the
/// bytes are preserved and UTF-8 is a layered view — [`Secret::password`] /
/// [`Secret::body`] return `&str` (empty when the bytes aren't valid UTF-8); use
/// [`Secret::password_bytes`] / [`Secret::body_bytes`] for byte-exact access.
/// A non-UTF-8 secret is edit-blocked upstream so its lossy view is display-only
/// and never written back. See ADR A005.
pub struct Secret {
    password: Zeroizing<Vec<u8>>,
    body: Zeroizing<Vec<u8>>,
    /// The parsed `Key: Value` attribute region (gopass AKV), as a derived view
    /// over [`Secret::body`]: every body line containing the gopass `": "`
    /// separator becomes one [`Attribute`], in order, duplicates preserved. In
    /// this phase `body()` still returns the raw blob (attributes inline); these
    /// accessors let the TOTP/attachment detectors stop re-scanning it. Phase 2b
    /// flips `body()` to free-text-only and makes `attributes` the source of truth.
    attributes: Vec<Attribute>,
}

/// One `Key: Value` line from a secret's attribute region (gopass AKV). Both
/// halves are decrypted content, so both are [`Zeroizing`]; the [`fmt::Debug`]
/// impl redacts them — never derive it, or a stray log line leaks the pair.
pub struct Attribute {
    key: Zeroizing<Vec<u8>>,
    value: Zeroizing<Vec<u8>>,
}

impl Attribute {
    /// The attribute key as raw bytes (byte-exact, never lossy).
    #[must_use]
    pub fn key(&self) -> &[u8] {
        self.key.as_slice()
    }
    /// The attribute value as raw bytes (byte-exact, never lossy).
    #[must_use]
    pub fn value(&self) -> &[u8] {
        self.value.as_slice()
    }
}

impl fmt::Debug for Attribute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attribute")
            .field("key", &"[REDACTED]")
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Custom `Debug` that redacts all fields — prevents accidental log leakage.
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("password", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .field(
                "attributes",
                &format!("{} [REDACTED]", self.attributes.len()),
            )
            .finish()
    }
}

impl Secret {
    /// Returns the password (first line) as a UTF-8 view.
    ///
    /// Returns an empty `&str` when the stored bytes aren't valid UTF-8 — use
    /// [`Secret::password_bytes`] for byte-exact access in that case.
    #[must_use]
    pub fn password(&self) -> &str {
        std::str::from_utf8(self.password.as_slice()).unwrap_or("")
    }

    /// The password as raw bytes (byte-exact, never lossy).
    #[must_use]
    pub fn password_bytes(&self) -> &[u8] {
        self.password.as_slice()
    }

    /// Returns the body (all content after the first line) as a UTF-8 view.
    ///
    /// In gopass AKV format this typically contains `key: value` metadata lines
    /// followed by optional freeform notes. Returns an empty `&str` when the
    /// stored bytes aren't valid UTF-8 — use [`Secret::body_bytes`] then.
    #[must_use]
    pub fn body(&self) -> &str {
        std::str::from_utf8(self.body.as_slice()).unwrap_or("")
    }

    /// The body as raw bytes (byte-exact, never lossy).
    #[must_use]
    pub fn body_bytes(&self) -> &[u8] {
        self.body.as_slice()
    }

    /// Whether both the password and body are valid UTF-8.
    ///
    /// `false` marks a secret that can't be safely round-tripped through a
    /// UTF-8 text editor — the UI edit-blocks it so its lossy view is never
    /// written back. (The legacy `GOPASS-SECRET-1.0` parser is text-based and
    /// lossy on read, so a non-UTF-8 *legacy* secret is already lossy in storage
    /// and reports `true` here; modern secrets — the realistic non-UTF-8 case —
    /// are byte-faithful and detected correctly.)
    #[must_use]
    pub fn is_utf8(&self) -> bool {
        std::str::from_utf8(self.password.as_slice()).is_ok()
            && std::str::from_utf8(self.body.as_slice()).is_ok()
    }

    /// Whether the password (first line) is valid UTF-8.
    ///
    /// Narrower than [`Secret::is_utf8`] (which requires both password and
    /// body): `copy_password` only ever places the password on the (UTF-8)
    /// clipboard, so a UTF-8 password with a non-UTF-8 body is still copyable.
    /// Editing round-trips the whole secret through a text editor, so the
    /// edit-block uses the stricter [`Secret::is_utf8`].
    #[must_use]
    pub fn password_is_utf8(&self) -> bool {
        std::str::from_utf8(self.password.as_slice()).is_ok()
    }

    /// The parsed attribute region (gopass AKV): every body line of the form
    /// `Key: Value`, in source order, duplicates preserved.
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    /// The first value for `key` (exact, case-sensitive key match) — gopass `Get`
    /// parity. gopass's TOTP reader uses exact lowercase keys (`totp`, `otpauth`).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.attributes
            .iter()
            .find(|a| a.key.as_slice() == key.as_bytes())
            .map(|a| a.value.as_slice())
    }

    /// The first value for `key`, matching the key case-insensitively — for the
    /// attachment consumer, where gopass's `isBase64Encoded` accepts both
    /// `Content-Transfer-Encoding` and `content-transfer-encoding`.
    #[must_use]
    pub fn get_ci(&self, key: &str) -> Option<&[u8]> {
        self.attributes
            .iter()
            .find(|a| a.key.as_slice().eq_ignore_ascii_case(key.as_bytes()))
            .map(|a| a.value.as_slice())
    }

    /// The first value for `key` (exact) as a UTF-8 view, or `None` when the key
    /// is absent or its value isn't valid UTF-8.
    #[must_use]
    pub fn attribute_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|v| std::str::from_utf8(v).ok())
    }

    /// Whether the secret signals a binary attachment: a
    /// `Content-Transfer-Encoding: base64` attribute (case-insensitive key and
    /// value, matching gopass's `isBase64Encoded`). A *presence* check, not a
    /// well-formedness one — a malformed payload still returns `true` so the
    /// decode error surfaces on export, not on the cheap UI probe.
    #[must_use]
    pub fn is_attachment(&self) -> bool {
        self.get_ci("Content-Transfer-Encoding").is_some_and(|v| {
            std::str::from_utf8(v).is_ok_and(|s| s.trim().eq_ignore_ascii_case("base64"))
        })
    }

    /// Serialize back to the modern on-disk plaintext: `password\n body`, or
    /// just `password` when the body is empty. Byte-exact inverse of
    /// [`Secret::parse`] for modern secrets (the only format gpm writes).
    #[must_use]
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let pw = self.password.as_slice();
        let bd = self.body.as_slice();
        let mut out = Vec::with_capacity(pw.len() + 1 + bd.len());
        out.extend_from_slice(pw);
        if !bd.is_empty() {
            out.push(b'\n');
            out.extend_from_slice(bd);
        }
        Zeroizing::new(out)
    }

    /// Parse decrypted bytes into a `Secret`.
    ///
    /// Recognizes two plaintext layouts:
    /// - **Modern** (what gpm writes): first line is the password, the rest is
    ///   the body.
    /// - **Legacy `GOPASS-SECRET-1.0`** (read-only compat for gopass secrets
    ///   written mid-2020–v1.13): the password lives in a `Password:` header;
    ///   see [`parse_legacy`].
    ///
    /// Trailing ASCII whitespace is stripped and CRLF is normalized to LF.
    /// Parsing is byte-oriented (no `from_utf8_lossy`), so non-UTF-8 modern
    /// secrets round-trip byte-exact via [`Secret::to_bytes`].
    ///
    /// # Errors
    ///
    /// Returns an error if the content is empty or contains only whitespace.
    pub fn parse(content: &[u8]) -> Result<Self, Error> {
        let normalized = normalize_bytes(content);

        if normalized.is_empty() {
            return Err(Error::new(
                ErrorCode::DecryptFailed,
                "Decrypted file is empty",
            ));
        }

        // The deprecated GOPASS-SECRET-1.0 format carries the password in a
        // `Password:` header rather than the first line; gopass still reads it,
        // so detect the magic and parse it. On a malformed header block gopass
        // falls back to its modern text parse (password = first line = the
        // magic); `parse_legacy` signals that by returning `None`, and we reuse
        // `modern_split_bytes` — the same path non-legacy secrets take.
        let first_line = normalized.split(|&b| b == b'\n').next().unwrap_or(&[]);
        let (password, body) = if first_line.trim_ascii() == LEGACY_MAGIC {
            // The MIME header state machine in `parse_legacy` is text-based, so
            // hand it a (lossy, for rare non-UTF-8 legacy) `&str` view and store
            // the resulting strings as bytes.
            let text = String::from_utf8_lossy(&normalized);
            match parse_legacy(&text) {
                Some((pw, bd)) => (
                    Zeroizing::new(pw.as_str().as_bytes().to_vec()),
                    Zeroizing::new(bd.as_str().as_bytes().to_vec()),
                ),
                None => modern_split_bytes(&normalized),
            }
        } else {
            modern_split_bytes(&normalized)
        };

        Ok(Self {
            attributes: parse_attributes(body.as_slice()),
            password,
            body,
        })
    }
}

/// The deprecated gopass MIME magic line, as bytes.
const LEGACY_MAGIC: &[u8] = b"GOPASS-SECRET-1.0";

/// Split `normalized` the modern way: first line is the password, everything
/// after the first `\n` is the body. Byte-exact inverse of [`Secret::to_bytes`].
/// Also the gopass-parity fallback for a malformed legacy header block — gopass
/// re-parses the whole input as its modern text format, so the magic line
/// becomes the password.
fn modern_split_bytes(normalized: &[u8]) -> (Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>) {
    if let Some(newline_pos) = normalized.iter().position(|&b| b == b'\n') {
        // `newline_pos` is the '\n': split the password off, then skip the '\n'.
        let (pw, rest) = normalized.split_at(newline_pos);
        let body = rest.get(1..).unwrap_or(&[]);
        (Zeroizing::new(pw.to_vec()), Zeroizing::new(body.to_vec()))
    } else {
        (
            Zeroizing::new(normalized.to_vec()),
            Zeroizing::new(Vec::new()),
        )
    }
}

/// Normalize decrypted bytes for parsing: CRLF → LF, then trim trailing ASCII
/// whitespace. Byte-oriented, so it never needs the bytes to be valid UTF-8.
fn normalize_bytes(content: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(content.len());
    let mut iter = content.iter().copied().peekable();
    while let Some(b) = iter.next() {
        if b == b'\r' && iter.peek() == Some(&b'\n') {
            // Drop the '\r', consume the '\n', emit a single '\n'.
            iter.next();
            out.push(b'\n');
        } else {
            out.push(b);
        }
    }
    while out.last().is_some_and(|&b| b.is_ascii_whitespace()) {
        out.pop();
    }
    out
}

/// Parse the `Key: Value` attribute region out of a body blob: every line
/// containing the gopass `": "` separator becomes one [`Attribute`], in source
/// order, duplicates preserved. The key is the bytes before the first `": "`,
/// the value the bytes after it (untrimmed — gopass does not trim attribute
/// values, see gopass issue #2873). Byte-oriented, so it never needs the body to
/// be valid UTF-8 — a non-UTF-8 body still yields byte-exact attributes.
fn parse_attributes(body: &[u8]) -> Vec<Attribute> {
    body.split(|&b| b == b'\n')
        .filter_map(|line| {
            // gopass's kvSep is the literal ": " — find its first occurrence.
            let pos = line.windows(2).position(|w| w == b": ")?;
            // `pos` is the colon's index; the matching window ": " occupies
            // [pos, pos+2), so both split_at calls are in bounds.
            let (key, sep_and_value) = line.split_at(pos);
            let value = sep_and_value.split_at(2).1; // drop the ": "
            Some(Attribute {
                key: Zeroizing::new(key.to_vec()),
                value: Zeroizing::new(value.to_vec()),
            })
        })
        .collect()
}

/// Parse the deprecated `GOPASS-SECRET-1.0` format — read-only compatibility
/// with gopass secrets written between mid-2020 and v1.13 (Jan 2021). gpm never
/// writes this format; an edit through gpm normalizes to the modern text format
/// (the frontend reassembles `${pw}\n${body}` and Rust encrypts verbatim).
///
/// `normalized` is the full plaintext after CRLF→LF + trailing trim, whose first
/// line (trimmed) is the magic `GOPASS-SECRET-1.0`; the caller discriminates and
/// commits to legacy.
///
/// Returns `Some(password, body)` on a well-formed legacy parse, or `None` when
/// the header block is malformed (a header line with no colon, or a continuation
/// line with no preceding header) — the caller then falls back to
/// [`modern_split_bytes`], matching gopass's cascade (`PermanentError` →
/// `ParseAKV(in)` → password = the magic line).
///
/// gopass parity (verified against `pkg/gopass/secrets/secparse`):
/// - The `Password:` header is extracted only when its first value is non-empty;
///   an empty-value `Password:` is left in the rendered body as `password:`
///   (gopass gates both `Get` and `Del` on `sv != ""`).
/// - Remaining header keys are lowercased (matching gopass's `strings.ToLower`)
///   and rendered in source order.
///
/// ```text
/// after_magic lines
///    │
///    ▼
///  ┌────────────────────── in header region? ──────────────────────┐
///  │ blank line ──► body = rest; STOP                              │
///  │ ws-led, cur_key set ──► fold (append, single-space)           │
///  │ ws-led, no cur_key  ──► None  (orphan → modern_split fallback)│
///  │ has ':' ──► flush pending; new header (first-colon, lc key)   │
///  │ else    ──► None  (no-colon → modern_split fallback)          │
///  │ EOF     ──► flush last; body = ""                             │
///  └───────────────────────────────────────────────────────────────┘
///    │ flush: v = val.trim_end()
///    │   key=="password", first value non-empty → pw=v, drop ALL password
///    │   key=="password", first value empty     → keep ALL as "password:"
///    │   else                                    → rendered.push("key: v")
///    ▼
///  body = rendered.join("\n") + body_text  →  Some((password, body))
/// ```
fn parse_legacy(normalized: &str) -> Option<(Zeroizing<String>, Zeroizing<String>)> {
    // Skip the magic first line.
    let after_magic = match normalized.find('\n') {
        Some(i) => &normalized[i + 1..],
        None => "", // magic-only file
    };
    let lines: Vec<&str> = after_magic.split('\n').collect();

    let mut password: Option<String> = None;
    let mut password_seen = false; // has the first `Password:` header been flushed?
    let mut rendered: Vec<String> = Vec::new();
    let mut cur_key: Option<String> = None;
    let mut cur_val = String::new();
    let mut body_start_idx = lines.len(); // default: no body (headers ran to EOF)

    for (idx, line) in lines.iter().enumerate() {
        if line.is_empty() {
            // Blank line ends the header block; the body follows.
            body_start_idx = idx + 1;
            break;
        }

        let starts_ws = line.starts_with(' ') || line.starts_with('\t');
        if starts_ws {
            // An orphan continuation (no preceding header) is malformed; `?`
            // turns a missing current header into None (the gopass-parity
            // modern-split fallback).
            cur_key.as_ref()?;
            // Folded continuation: append to the current header value.
            cur_val.push(' ');
            cur_val.push_str(line.trim_start_matches([' ', '\t']));
            continue;
        }

        // A header line with no colon → malformed. `?` propagates `None`, which
        // the caller turns into the gopass-parity modern-split fallback.
        let colon = line.find(':')?;

        // New header. Commit the pending one, then start fresh.
        flush(
            &mut cur_key,
            &mut cur_val,
            &mut password,
            &mut password_seen,
            &mut rendered,
        );
        let (key_raw, val_raw) = line.split_at(colon);
        cur_key = Some(key_raw.to_ascii_lowercase());
        cur_val = val_raw[1..].trim_start().to_string();
    }

    // Commit the final pending header (EOF or blank-line terminator).
    flush(
        &mut cur_key,
        &mut cur_val,
        &mut password,
        &mut password_seen,
        &mut rendered,
    );

    let body_text = lines.get(body_start_idx..).unwrap_or_default().join("\n");
    let body_text = body_text.trim_end();

    let body = if rendered.is_empty() {
        body_text.to_string()
    } else if body_text.is_empty() {
        rendered.join("\n")
    } else {
        format!("{}\n{}", rendered.join("\n"), body_text)
    };

    Some((
        Zeroizing::new(password.unwrap_or_default()),
        // `trim_end` is load-bearing: a final empty-value header (e.g. an empty
        // `Password:`) renders as "key: " with a trailing space.
        Zeroizing::new(body.trim_end().to_string()),
    ))
}

/// Commit the pending header (`cur_key`/`cur_val`) into either the password slot
/// or the rendered body. Mirrors gopass's `Password` handling: extraction (and
/// dropping from the render) is gated on the FIRST `Password:` header having a
/// non-empty value.
fn flush(
    cur_key: &mut Option<String>,
    cur_val: &mut String,
    password: &mut Option<String>,
    password_seen: &mut bool,
    rendered: &mut Vec<String>,
) {
    let Some(key) = cur_key.take() else {
        cur_val.clear();
        return;
    };
    let value = cur_val.trim_end();
    if key.eq_ignore_ascii_case("password") {
        let first = !*password_seen;
        *password_seen = true;
        if first && !value.is_empty() {
            // First Password is non-empty → extract it and drop every Password
            // header from the render (gopass: hdr.Get + hdr.Del, both gated on
            // `sv != ""`).
            *password = Some(value.to_string());
        } else if password.is_some() {
            // A previous Password was extracted → drop this one too.
        } else {
            // First Password was empty (or a later one when it was) → gopass
            // leaves all Password headers in place; render this one.
            rendered.push(format!("{key}: {value}"));
        }
    } else {
        rendered.push(format!("{key}: {value}"));
    }
    cur_val.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_password_only() {
        let secret = Secret::parse(b"hunter2").unwrap();
        assert_eq!(secret.password(), "hunter2");
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_password_and_body() {
        let content = b"hunter2\nusername: alice\nurl: example.com";
        let secret = Secret::parse(content).unwrap();
        assert_eq!(secret.password(), "hunter2");
        assert!(secret.body().contains("username: alice"));
        assert!(secret.body().contains("url: example.com"));
    }

    #[test]
    fn parse_empty_content_errors() {
        let result = Secret::parse(b"");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "DECRYPT_FAILED");
    }

    #[test]
    fn parse_whitespace_only_errors() {
        let result = Secret::parse(b"  \n  \n");
        assert!(result.is_err());
    }

    #[test]
    fn parse_trailing_newlines_stripped() {
        let secret = Secret::parse(b"pw\nnotes\n").unwrap();
        assert_eq!(secret.password(), "pw");
        assert_eq!(secret.body(), "notes");
    }

    #[test]
    fn parse_crlf_line_endings() {
        let secret = Secret::parse(b"pw\r\nnotes\r\nmore notes\r\n").unwrap();
        assert_eq!(secret.password(), "pw");
        assert_eq!(secret.body(), "notes\nmore notes");
    }

    #[test]
    fn debug_redacts_password() {
        let secret = Secret::parse(b"hunter2\nnotes").unwrap();
        let debug_output = format!("{secret:?}");
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug output should contain [REDACTED], got: {debug_output}"
        );
        assert!(
            !debug_output.contains("hunter2"),
            "Debug output must not contain the actual password, got: {debug_output}"
        );
    }

    #[test]
    fn parse_unicode_content() {
        let secret = Secret::parse("密码123\n用户: 张三\n网址: example.com".as_bytes()).unwrap();
        assert_eq!(secret.password(), "密码123");
        assert!(secret.body().contains("用户: 张三"));
    }

    #[test]
    fn parse_multiline_body() {
        let secret = Secret::parse(b"pw\nline1\nline2\nline3").unwrap();
        assert_eq!(secret.password(), "pw");
        assert_eq!(secret.body(), "line1\nline2\nline3");
    }

    // ---- deprecated GOPASS-SECRET-1.0 format (read-only compat) ----

    #[test]
    fn parse_legacy_basic() {
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: hunter2\nusername: alice\n\nfree text")
                .unwrap();
        assert_eq!(secret.password(), "hunter2");
        assert_eq!(secret.body(), "username: alice\nfree text");
    }

    #[test]
    fn parse_legacy_multiple_attributes_preserve_order() {
        let secret = Secret::parse(
            b"GOPASS-SECRET-1.0\nPassword: p\nusername: alice\nurl: example.com\ntype: ssh",
        )
        .unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(
            secret.body(),
            "username: alice\nurl: example.com\ntype: ssh"
        );
    }

    #[test]
    fn parse_legacy_no_password_header() {
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nusername: alice\n\nnotes").unwrap();
        assert_eq!(secret.password(), "");
        assert_eq!(secret.body(), "username: alice\nnotes");
    }

    #[test]
    fn parse_legacy_folded_continuation() {
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nnote: line one\n  line two\n\nbody")
                .unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(secret.body(), "note: line one line two\nbody");
    }

    #[test]
    fn parse_legacy_magic_only_no_newline() {
        let secret = Secret::parse(b"GOPASS-SECRET-1.0").unwrap();
        assert_eq!(secret.password(), "");
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_no_body_eof_terminates_headers() {
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nusername: alice").unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(secret.body(), "username: alice");
    }

    #[test]
    fn parse_legacy_crlf_in_header_block() {
        let secret = Secret::parse(
            b"GOPASS-SECRET-1.0\r\nPassword: hunter2\r\nusername: alice\r\n\r\nbody\r\n",
        )
        .unwrap();
        assert_eq!(secret.password(), "hunter2");
        assert_eq!(secret.body(), "username: alice\nbody");
    }

    #[test]
    fn parse_legacy_password_key_case_insensitive() {
        for key in ["password", "PASSWORD", "PaSsWoRd"] {
            let input = format!("GOPASS-SECRET-1.0\n{key}: x\n");
            let secret = Secret::parse(input.as_bytes()).unwrap();
            assert_eq!(secret.password(), "x", "Password key {key}");
            assert_eq!(secret.body(), "", "Password key {key}");
        }
    }

    #[test]
    fn parse_legacy_first_password_wins_others_dropped() {
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: first\nPassword: second\n").unwrap();
        assert_eq!(secret.password(), "first");
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_header_value_with_colon_url() {
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nurl: https://example.com:8080/path")
                .unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(secret.body(), "url: https://example.com:8080/path");
    }

    #[test]
    fn parse_legacy_empty_password_value_kept_in_body() {
        // gopass parity (D5): an empty-value `Password:` header is NOT extracted
        // (the `sv != ""` guard) and stays in the rendered body as `password:`.
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword:\nFoo: Bar").unwrap();
        assert_eq!(secret.password(), "");
        assert_eq!(secret.body(), "password: \nfoo: Bar");
    }

    #[test]
    fn parse_legacy_multiple_blank_lines_body_keeps_leading_newline() {
        // gopass parity: the body is everything after the FIRST blank line, so a
        // second blank line survives as a leading "\n" (gopass io.Copy).
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\n\n\nbody").unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(secret.body(), "\nbody");
    }

    #[test]
    fn parse_legacy_renders_lowercased_keys() {
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nTotp: ABCD\nNote: x\n").unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(secret.body(), "totp: ABCD\nnote: x");
    }

    #[test]
    fn parse_legacy_magic_line_with_surrounding_whitespace() {
        let secret = Secret::parse(b"  GOPASS-SECRET-1.0  \nPassword: p\n").unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_no_colon_header_falls_back_to_modern() {
        // gopass parity (D6): a no-colon line in the header block is malformed →
        // gopass falls back to ParseAKV(in) → password = the magic line.
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nthis has no colon\nmore body").unwrap();
        assert_eq!(secret.password(), "GOPASS-SECRET-1.0");
        assert_eq!(secret.body(), "Password: p\nthis has no colon\nmore body");
    }

    #[test]
    fn parse_legacy_orphan_fold_falls_back_to_modern() {
        // gopass parity (D6): a continuation line with no preceding header is
        // malformed → fallback → password = the magic line.
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\n  orphan fold\nmore body").unwrap();
        assert_eq!(secret.password(), "GOPASS-SECRET-1.0");
        assert_eq!(secret.body(), "  orphan fold\nmore body");
    }

    #[test]
    fn parse_real_password_not_treated_as_legacy() {
        // Regression guard: a modern secret whose first line is a real password
        // must bypass the legacy branch.
        let secret = Secret::parse(b"hunter2\nusername: alice").unwrap();
        assert_eq!(secret.password(), "hunter2");
        assert_eq!(secret.body(), "username: alice");
    }

    #[test]
    fn parse_legacy_magic_literal_first_line_routes_to_legacy() {
        // gpm inherits gopass's footgun: a secret whose first line IS the magic
        // literal is treated as legacy (gopass uses the identical discriminator),
        // even if a user meant it as a modern password. With no `Password:`
        // header the password slot is empty and the rest renders as the body.
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nusername: alice").unwrap();
        assert_eq!(secret.password(), "");
        assert_eq!(secret.body(), "username: alice");
    }

    #[test]
    fn parse_legacy_first_password_empty_then_non_empty_all_rendered() {
        // gopass parity: the FIRST `Password:` header decides. Its value is
        // empty, so nothing is extracted and EVERY `Password:` header stays in
        // the rendered body (gopass's `hdr.Del` is gated on `sv != ""`).
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword:\nPassword: second\n").unwrap();
        assert_eq!(secret.password(), "");
        assert_eq!(secret.body(), "password: \npassword: second");
    }

    // ---- bytes-native (R069 phase 1) ----

    #[test]
    fn to_bytes_password_only_has_no_newline() {
        let secret = Secret::parse(b"hunter2").unwrap();
        assert_eq!(secret.to_bytes().as_slice(), b"hunter2");
    }

    #[test]
    fn to_bytes_round_trips_modern() {
        // parse → to_bytes → parse must yield the same password/body bytes.
        let original = b"pw\nusername: alice\nurl: example.com\nnotes";
        let once = Secret::parse(original).unwrap();
        let bytes = once.to_bytes();
        let twice = Secret::parse(&bytes).unwrap();
        assert_eq!(twice.password_bytes(), once.password_bytes());
        assert_eq!(twice.body_bytes(), once.body_bytes());
        // And to_bytes is stable (idempotent on the already-normalized form).
        assert_eq!(twice.to_bytes().as_slice(), bytes.as_slice());
    }

    #[test]
    fn to_bytes_round_trips_attachment_layout() {
        // An attachment's plaintext is an empty password line + attribute lines
        // + a base64 body; to_bytes must reproduce it byte-for-byte.
        let original = b"\nContent-Disposition: attachment; filename=\"x.bin\"\nContent-Transfer-Encoding: Base64\nQUJD";
        let secret = Secret::parse(original).unwrap();
        assert_eq!(secret.password_bytes(), b"");
        assert_eq!(secret.to_bytes().as_slice(), original);
    }

    #[test]
    fn non_utf8_body_preserved_and_detected() {
        // The headline phase-1 test: non-UTF-8 body bytes survive parse + to_bytes
        // bit-identical (no from_utf8_lossy corruption), and is_utf8() flags it so
        // the UI can edit-block it. body() returns "" (the lossy view) for display.
        let original = b"pw\n\xff\xfe garbage \x80";
        let secret = Secret::parse(original).unwrap();
        assert_eq!(secret.body_bytes(), &original[3..]);
        assert!(!secret.is_utf8());
        assert_eq!(secret.body(), "");
        // Round-trips byte-identical through to_bytes.
        assert_eq!(secret.to_bytes().as_slice(), original);
    }

    #[test]
    fn non_utf8_password_detected() {
        let secret = Secret::parse(b"\xff\xfe\nbody").unwrap();
        assert_eq!(secret.password_bytes(), b"\xff\xfe");
        assert!(!secret.is_utf8());
        assert_eq!(secret.password(), "");
    }

    #[test]
    fn password_is_utf8_distinguishes_password_from_body() {
        // A UTF-8 password with a non-UTF-8 body: copy touches the password
        // only, so password_is_utf8() is true (is_utf8() is still false → the
        // edit-block holds, but the copy path is not blocked).
        let secret = Secret::parse(b"hunter2\n\xff\xfe body garbage \x80").unwrap();
        assert!(secret.password_is_utf8());
        assert!(!secret.is_utf8());
        // A non-UTF-8 password: copy is blocked too.
        let secret = Secret::parse(b"\xff\xfe\nbody").unwrap();
        assert!(!secret.password_is_utf8());
    }

    // ---- attribute region (R069 phase 2a) ----

    #[test]
    fn attributes_parsed_from_body() {
        let secret = Secret::parse(b"pw\nuser: alice\nurl: https://example.com\nnotes").unwrap();
        // `get` is exact-case (gopass `Get` parity).
        assert_eq!(secret.get("user"), Some(b"alice".as_slice()));
        assert_eq!(secret.attribute_str("url"), Some("https://example.com"));
        // A free-text line (no ": ") is not an attribute.
        assert_eq!(secret.get("notes"), None);
        // body() still carries the attribute lines (phase-2a compat shim).
        assert!(secret.body().contains("user: alice"));
    }

    #[test]
    fn get_is_case_sensitive_get_ci_is_not() {
        // The case-sensitivity split that grounds the detectors: TOTP reads
        // exact lowercase (`get`); attachments read case-insensitively (`get_ci`,
        // gopass binary.go tries both key casings).
        let secret = Secret::parse(b"pw\nuser: alice").unwrap();
        assert_eq!(secret.get("user"), Some(b"alice".as_slice()));
        assert_eq!(secret.get("USER"), None);
        assert_eq!(secret.get_ci("USER"), Some(b"alice".as_slice()));
        assert_eq!(secret.get_ci("UsEr"), Some(b"alice".as_slice()));
    }

    #[test]
    fn is_attachment_detects_cte_both_key_cases() {
        // R066/T5: the legacy parser lowercases the CTE key, so a case-sensitive
        // lookup would miss it and the base64 body would leak to the WebView.
        for body in [
            "\nContent-Transfer-Encoding: base64\nQUJD",
            "\ncontent-transfer-encoding: base64\nQUJD",
            "\nCONTENT-TRANSFER-ENCODING: Base64\nQUJD",
        ] {
            let secret = Secret::parse(body.as_bytes()).unwrap();
            assert!(secret.is_attachment(), "should detect: {body}");
        }
        // Not base64, or no CTE at all → not an attachment.
        assert!(
            !Secret::parse(b"pw\nContent-Transfer-Encoding: 8bit\nQUJD")
                .unwrap()
                .is_attachment()
        );
        assert!(!Secret::parse(b"pw\nuser: alice").unwrap().is_attachment());
    }

    #[test]
    fn attributes_preserve_duplicates_and_order() {
        // gopass allows duplicate keys (the legacy format leans on it). All are
        // kept in source order; `get` returns the first.
        let secret = Secret::parse(b"pw\nnote: one\nnote: two\nx: y").unwrap();
        let notes: Vec<&[u8]> = secret
            .attributes()
            .iter()
            .filter(|a| a.key() == "note".as_bytes())
            .map(Attribute::value)
            .collect();
        assert_eq!(notes, vec![b"one".as_slice(), b"two".as_slice()]);
        assert_eq!(secret.get("note"), Some(b"one".as_slice()));
    }

    #[test]
    fn attribute_str_none_for_non_utf8_value() {
        // A non-UTF-8 attribute value: the byte accessor sees it, the str view is None.
        let secret = Secret::parse(b"pw\nk: \xff\xfe").unwrap();
        assert_eq!(secret.get("k"), Some(b"\xff\xfe".as_slice()));
        assert_eq!(secret.attribute_str("k"), None);
    }
}
