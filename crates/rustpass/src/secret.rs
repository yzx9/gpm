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
/// All fields use `Zeroizing<String>` so content is wiped on drop.
pub struct Secret {
    password: Zeroizing<String>,
    body: Zeroizing<String>,
}

/// Custom `Debug` that redacts all fields — prevents accidental log leakage.
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("password", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .finish()
    }
}

impl Secret {
    /// Returns the password (first line of the secret).
    #[must_use]
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Returns the body (all content after the first line).
    ///
    /// In gopass AKV format, this typically contains `key: value` metadata
    /// lines followed by optional freeform notes.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
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
    /// Trailing whitespace is stripped. CRLF is normalized to LF first.
    ///
    /// # Errors
    ///
    /// Returns an error if the content is empty or contains only whitespace.
    pub fn parse(content: &[u8]) -> Result<Self, Error> {
        let text = String::from_utf8_lossy(content);
        let text = text.trim_end();

        if text.is_empty() {
            return Err(Error::new(
                ErrorCode::DecryptFailed,
                "Decrypted file is empty",
            ));
        }

        // Normalize CRLF to LF for consistent parsing
        let normalized = text.replace("\r\n", "\n");
        let normalized = normalized.trim_end();

        // The deprecated GOPASS-SECRET-1.0 format carries the password in a
        // `Password:` header rather than the first line; gopass still reads it,
        // so detect the magic and parse it. On a malformed header block gopass
        // falls back to its modern text parse (password = first line = the
        // magic); `parse_legacy` signals that by returning `None`, and we reuse
        // `modern_split` — the same path non-legacy secrets take.
        let first_line = normalized.split('\n').next().unwrap_or("");
        let (password, body) = if first_line.trim() == "GOPASS-SECRET-1.0" {
            parse_legacy(normalized).unwrap_or_else(|| modern_split(normalized))
        } else {
            modern_split(normalized)
        };

        Ok(Self { password, body })
    }
}

/// Split `normalized` the modern way: first line is the password, everything
/// after the first `\n` is the body. Lossless inverse of the frontend's
/// `reassemble(pw, body)` (`${pw}\n${body}`). Also the gopass-parity fallback
/// for a malformed legacy header block — gopass re-parses the whole input as
/// its modern text format, so the magic line becomes the password.
fn modern_split(normalized: &str) -> (Zeroizing<String>, Zeroizing<String>) {
    if let Some(newline_pos) = normalized.find('\n') {
        (
            Zeroizing::new(normalized[..newline_pos].to_string()),
            Zeroizing::new(normalized[newline_pos + 1..].to_string()),
        )
    } else {
        (
            Zeroizing::new(normalized.to_string()),
            Zeroizing::new(String::new()),
        )
    }
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
/// [`modern_split`], matching gopass's cascade (`PermanentError` → `ParseAKV(in)`
/// → password = the magic line).
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
}
