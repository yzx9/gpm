// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;

use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};

/// A decrypted secret — aligned with `gopass.Secret`.
///
/// In memory: first line = password; the remainder is split into a structured
/// `attributes` region (gopass AKV `Key: Value` pairs) and a free-text `body`
/// (gopass `Body()` — every line that is NOT a `Key: Value` pair). This is the
/// R069 phase-2b model: `attributes` is the source of truth and `body()` returns
/// free-text notes only. [`Secret::parse`] also reads the deprecated
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
///
/// A secret carrying a `---` document-marker line is parsed as **legacy YAML**
/// (A004): attributes are NOT split out of it, the whole post-password content
/// (marker included) is the opaque body, and [`Secret::is_yaml`] marks it so the
/// UI can edit-block it — gpm never writes a YAML secret back, which is what
/// makes the edit-time YAML corruption impossible.
pub struct Secret {
    password: Zeroizing<Vec<u8>>,
    /// Free-text notes only (gopass `Body()` parity): every line that contained
    /// the gopass `": "` separator has been lifted into `attributes`.
    body: Zeroizing<Vec<u8>>,
    /// The parsed `Key: Value` attribute region (gopass AKV), as the source of
    /// truth: ordered, duplicate-tolerant. Both halves are decrypted content, so
    /// both are [`Zeroizing`]; the [`fmt::Debug`] impl redacts them — never
    /// derive it, or a stray log line leaks the pair.
    attributes: Vec<Attribute>,
    /// Whether this secret was parsed via the legacy-YAML branch (A004). Not
    /// secret.
    yaml: bool,
}

/// One `Key: Value` line from a secret's attribute region (gopass AKV). Both
/// halves are decrypted content, so both are [`Zeroizing`]; the [`fmt::Debug`]
/// impl redacts them — never derive it, or a stray log line leaks the pair.
#[derive(PartialEq, Eq)]
pub struct Attribute {
    key: Zeroizing<Vec<u8>>,
    value: Zeroizing<Vec<u8>>,
}

impl Attribute {
    /// Construct a new attribute from byte parts (the edit/create assembler
    /// builds attributes this way before handing them to [`Secret::from_parts`]).
    #[must_use]
    pub fn new(key: Vec<u8>, value: Vec<u8>) -> Self {
        Self {
            key: Zeroizing::new(key),
            value: Zeroizing::new(value),
        }
    }
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
            .field("yaml", &self.yaml)
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

    /// Returns the free-text body (every post-password line that is NOT a
    /// `Key: Value` pair — gopass `Body()` parity) as a UTF-8 view.
    ///
    /// Returns an empty `&str` when the stored bytes aren't valid UTF-8 — use
    /// [`Secret::body_bytes`] then.
    #[must_use]
    pub fn body(&self) -> &str {
        std::str::from_utf8(self.body.as_slice()).unwrap_or("")
    }

    /// The body as raw bytes (byte-exact, never lossy).
    #[must_use]
    pub fn body_bytes(&self) -> &[u8] {
        self.body.as_slice()
    }

    /// Whether the password, every attribute key/value, and the body are all
    /// valid UTF-8.
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
            && self.attributes.iter().all(|a| {
                std::str::from_utf8(a.key.as_slice()).is_ok()
                    && std::str::from_utf8(a.value.as_slice()).is_ok()
            })
            && std::str::from_utf8(self.body.as_slice()).is_ok()
    }

    /// Whether the password (first line) is valid UTF-8.
    ///
    /// Narrower than [`Secret::is_utf8`] (which requires the password,
    /// attributes, and body to all be UTF-8): `copy_password` only ever places
    /// the password on the (UTF-8) clipboard, so a UTF-8 password with a
    /// non-UTF-8 body is still copyable. Editing round-trips the whole secret
    /// through a text editor, so the edit-block uses the stricter
    /// [`Secret::is_utf8`].
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

    /// Serialize back to the modern on-disk plaintext in canonical order:
    /// `password`, then each attribute as `key: value`, then the free-text body.
    /// Byte-exact inverse of [`Secret::parse`] **for the AKV/MIME-normalized
    /// paths** (the common case); interleaved secrets canonicalize on
    /// round-trip, which stays gopass-readable (gopass's reader is
    /// position-agnostic over `": "`). YAML secrets are **read-only** (A004) —
    /// they never reach this method through gpm's write path, and no
    /// round-trip is promised for them (a bare `---` doc re-parsed through
    /// [`Secret::from_parts`] would not reproduce the original bytes).
    #[must_use]
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let pw = self.password.as_slice();
        let has_content = !self.attributes.is_empty() || !self.body.is_empty();
        let mut out =
            Vec::with_capacity(pw.len() + self.attributes.len() * 32 + self.body.len() + 16);
        out.extend_from_slice(pw);
        if !has_content {
            return Zeroizing::new(out); // password-only, no trailing newline
        }
        out.push(b'\n');
        for (i, a) in self.attributes.iter().enumerate() {
            if i > 0 {
                out.push(b'\n');
            }
            out.extend_from_slice(a.key.as_slice());
            out.extend_from_slice(b": ");
            out.extend_from_slice(a.value.as_slice());
        }
        if !self.body.is_empty() {
            if !self.attributes.is_empty() {
                out.push(b'\n');
            }
            out.extend_from_slice(self.body.as_slice());
        }
        Zeroizing::new(out)
    }

    /// Build a `Secret` from its structured parts — the single-source assembler
    /// for the edit/create write path ([`Secret::to_bytes`] then serializes it).
    ///
    /// Validates that the password contains no newline, no attribute key
    /// contains the gopass `": "` separator or a newline, and no value contains
    /// a newline — otherwise the reassembled plaintext would re-parse to a
    /// different structure (silent corruption).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::SecretInvalid`] when the password contains a newline, an
    /// attribute key contains `": "` or a newline, or an attribute value contains
    /// a newline.
    pub fn from_parts(
        password: Vec<u8>,
        attributes: Vec<Attribute>,
        body: Vec<u8>,
    ) -> Result<Self, Error> {
        // A newline breaks the first-line invariant; `": "` in a password is fine.
        if password.as_slice().contains(&b'\n') {
            return Err(Error::new(
                ErrorCode::SecretInvalid,
                "password contains a newline",
            ));
        }
        for a in &attributes {
            if a.key.as_slice().contains(&b'\n')
                || a.value.as_slice().contains(&b'\n')
                || a.key.as_slice().windows(2).any(|w| w == b": ")
            {
                return Err(Error::new(
                    ErrorCode::SecretInvalid,
                    "attribute key contains \": \" or a newline, or value contains a newline",
                ));
            }
        }
        Ok(Self {
            password: Zeroizing::new(password),
            attributes,
            body: Zeroizing::new(body),
            yaml: false,
        })
    }

    /// Parse decrypted bytes into a `Secret`.
    ///
    /// Recognizes two plaintext layouts:
    /// - **Modern** (what gpm writes): first line is the password; the rest is
    ///   partitioned into attributes (`Key: Value` lines) and free-text body.
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
        // the modern path — the same path non-legacy secrets take.
        let first_line = normalized.split(|&b| b == b'\n').next().unwrap_or(&[]);
        // The MIME header state machine in `parse_legacy` is text-based, so hand
        // it a (lossy, for rare non-UTF-8 legacy) `&str` view. On a malformed
        // header block it returns None and we fall back to the modern split
        // (gopass's cascade: PermanentError → ParseAKV → password = the magic).
        let legacy = if first_line.trim_ascii() == LEGACY_MAGIC {
            let text = String::from_utf8_lossy(&normalized);
            parse_legacy(&text).map(|(pw, header_attrs, post_body)| {
                // The post-header body is split with the same position-agnostic
                // rule as a modern body (gopass `Body()` drops any ": " line).
                let (body_attrs, free_body) = split_attrs(&post_body);
                let mut attrs = header_attrs;
                attrs.extend(body_attrs);
                (pw, attrs, free_body)
            })
        } else {
            None
        };
        let (password, attributes, body, yaml) = match legacy {
            // MIME never carries a YAML block; it is fully handled above.
            Some(parts) => {
                let (pw, attrs, body) = parts;
                (pw, attrs, body, false)
            }
            // ORDER INVARIANT (A004): when the MIME magic matched, YAML is
            // never attempted — gopass's cascade short-circuits a malformed
            // MIME header block (PermanentError) straight to ParseAKV
            // (`secparse/parse.go`), skipping ParseYAML entirely, even if the
            // body carries a `---` line.
            None if first_line.trim_ascii() == LEGACY_MAGIC => {
                let (pw, attrs, body) = modern_split(&normalized);
                (pw, attrs, body, false)
            }
            None if is_bare_yaml_doc(first_line) || contains_yaml_marker(&normalized) => {
                let (pw, body) = yaml_split(&normalized);
                (pw, Vec::new(), body, true)
            }
            None => {
                let (pw, attrs, body) = modern_split(&normalized);
                (pw, attrs, body, false)
            }
        };

        Ok(Self {
            password,
            attributes,
            body: Zeroizing::new(body),
            yaml,
        })
    }

    /// Whether this secret was parsed via the legacy-YAML branch (A004): a
    /// `---` document-marker line is present, so the secret is read-only
    /// (gpm never writes it back) and its fields are not exposed as AKV
    /// attributes.
    #[must_use]
    pub fn is_yaml(&self) -> bool {
        self.yaml
    }
}

/// The deprecated gopass MIME magic line, as bytes.
const LEGACY_MAGIC: &[u8] = b"GOPASS-SECRET-1.0";

/// Output of [`parse_legacy`]: `(password, header attributes, post-header body)`.
type LegacyParts = (Zeroizing<Vec<u8>>, Vec<Attribute>, Vec<u8>);

/// Split `normalized` into the password (first line) and the remainder (after
/// the first `\n`). The password is `Zeroizing`; the remainder is plain bytes
/// handed to [`split_attrs`].
fn split_first_line(normalized: &[u8]) -> (Zeroizing<Vec<u8>>, Vec<u8>) {
    if let Some(newline_pos) = normalized.iter().position(|&b| b == b'\n') {
        let (pw, rest) = normalized.split_at(newline_pos);
        // `rest` starts at the '\n' (index 0); skip it.
        let after = rest.get(1..).unwrap_or(&[]);
        (Zeroizing::new(pw.to_vec()), after.to_vec())
    } else {
        (Zeroizing::new(normalized.to_vec()), Vec::new())
    }
}

/// The modern AKV split: first line is the password, the rest is partitioned
/// into attributes (`Key: Value` lines) and free-text body. Also the
/// gopass-parity fallback for a malformed legacy header block.
fn modern_split(normalized: &[u8]) -> (Zeroizing<Vec<u8>>, Vec<Attribute>, Vec<u8>) {
    let (pw, rest) = split_first_line(normalized);
    let (attrs, body) = split_attrs(&rest);
    (pw, attrs, body)
}

/// The gopass YAML document marker, as bytes.
const YAML_MARKER: &[u8] = b"---";

/// Whether the first line of a secret is a bare YAML document opener — the
/// line, trimmed, IS the marker. gopass's `ParseYAML` reads the first line,
/// `TrimSpace`s it, and treats `line == "---"` as "no password, the whole
/// secret is one YAML document" (`pkg/gopass/secrets/yaml.go`) — so `---` and
/// `--- ` both open a bare doc, while `---hunter2` is a password.
fn is_bare_yaml_doc(first_line: &[u8]) -> bool {
    first_line.trim_ascii() == YAML_MARKER
}

/// Whether one (CR-stripped) line is a YAML document-marker line: it starts
/// with `---` (gopass's `Peek(3)` token) but NOT with `----`. gopass's peek
/// matches PEM armor (`-----BEGIN …`) and `----` rules too, but its YAML
/// *decode* then fails and the cascade falls back to AKV — so gopass's
/// effective classification of armor is editable AKV, and excluding `----`+
/// here mirrors the outcome rather than the token (pinned against the real
/// binary in `gopass_interop_age.rs`). A `--- `-style marker (trailing space)
/// matches, like gopass's token check.
fn is_marker_line(line: &[u8]) -> bool {
    line.starts_with(YAML_MARKER) && !line.starts_with(b"----")
}

/// Whether the (already-normalized) plaintext carries a YAML document-marker
/// line after its password line. gopass consumes the first line as the
/// password (bare-doc check first) and only then peeks for the marker
/// (`pkg/gopass/secrets/yaml.go` parseBody), so the password line itself never
/// counts — a password starting `---` stays AKV here exactly as in gopass.
///
/// Parser-free (A004: no YAML decode on the read path), so a marker-bearing
/// body that is not valid YAML is still treated as YAML (read-only) —
/// over-blocking a possible edit is safe where corrupting one is not. The
/// write-path rule is the broader [`is_yaml_secret_content`].
fn contains_yaml_marker(content: &[u8]) -> bool {
    // Skip the first line (the password); a marker anywhere after it routes
    // the secret to the YAML branch.
    content
        .split(|&b| b == b'\n')
        .skip(1)
        .any(|line| is_marker_line(line.strip_suffix(b"\r").unwrap_or(line)))
}

/// The write-path rule (A004): whether this content would parse as a
/// legacy-YAML secret — a bare `---` first line OR a document-marker line
/// after it. Every write path funnels through [`crate::Store::set`], which
/// refuses such content, so gpm never persists a secret it would immediately
/// show read-only. Accepts RAW (not yet CRLF-normalized) input.
#[must_use]
pub fn is_yaml_secret_content(content: &[u8]) -> bool {
    let mut lines = content.split(|&b| b == b'\n');
    let first = lines.next().unwrap_or(&[]);
    is_bare_yaml_doc(first.strip_suffix(b"\r").unwrap_or(first))
        || lines.any(|line| is_marker_line(line.strip_suffix(b"\r").unwrap_or(line)))
}

/// The legacy-YAML split (A004): the password is the first line — empty when
/// the first line is itself the `---` marker (a bare YAML document, mirroring
/// gopass `ParseYAML`, which `TrimSpace`s the line before the bare-doc
/// comparison so `--- ` counts too) — and everything after it (marker
/// included) is the opaque body. No attributes: gpm does not parse the YAML
/// block, and never writes the secret back, so the edit-time corruption is
/// impossible.
fn yaml_split(normalized: &[u8]) -> (Zeroizing<Vec<u8>>, Vec<u8>) {
    let (pw, rest) = split_first_line(normalized);
    if is_bare_yaml_doc(pw.as_slice()) {
        // Bare YAML document: no password line before the marker; the marker
        // line itself stays in the body.
        let mut body = pw.as_slice().to_vec();
        if !rest.is_empty() {
            body.push(b'\n');
            body.extend_from_slice(&rest);
        }
        (Zeroizing::new(Vec::new()), body)
    } else {
        (pw, rest)
    }
}

/// Partition the post-password bytes into (attributes, free-text body), gopass
/// AKV parity: each line containing the `": "` separator becomes one
/// [`Attribute`] (key = bytes before the first `": "`, value = bytes after,
/// untrimmed — gopass does not trim attribute values, see gopass issue #2873);
/// every other line is free-text body. Attribute order and duplicate keys are
/// preserved. Byte-oriented, so a non-UTF-8 body still yields byte-exact
/// attributes. The free-text body is the non-`": "` lines joined by `\n` (no
/// trailing newline).
fn split_attrs(rest: &[u8]) -> (Vec<Attribute>, Vec<u8>) {
    let mut attrs = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    let mut first_body_line = true;
    for line in rest.split(|&b| b == b'\n') {
        if let Some(pos) = line.windows(2).position(|w| w == b": ") {
            let (key, sep_and_value) = line.split_at(pos);
            let value = sep_and_value.split_at(2).1; // drop the ": "
            attrs.push(Attribute::new(key.to_vec(), value.to_vec()));
        } else {
            if !first_body_line {
                body.push(b'\n');
            }
            body.extend_from_slice(line);
            first_body_line = false;
        }
    }
    (attrs, body)
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

/// Parse the deprecated `GOPASS-SECRET-1.0` format — read-only compatibility
/// with gopass secrets written between mid-2020 and v1.13 (Jan 2021). gpm never
/// writes this format; an edit through gpm normalizes to the modern text format.
///
/// `normalized` is the full plaintext after CRLF→LF + trailing trim, whose first
/// line (trimmed) is the magic `GOPASS-SECRET-1.0`; the caller discriminates and
/// commits to legacy.
///
/// Returns `Some(password, header_attrs, post-header-body)` on a well-formed
/// legacy parse, or `None` when the header block is malformed (a header line
/// with no colon, or a continuation line with no preceding header) — the caller
/// then falls back to the modern split, matching gopass's cascade
/// (`PermanentError` → `ParseAKV(in)` → password = the magic line).
///
/// gopass parity (verified against `pkg/gopass/secrets/secparse`):
/// - The `Password:` header is extracted only when its first value is non-empty;
///   an empty-value `Password:` is kept as an `Attribute { key: "password", "" }`
///   (gopass gates both `Get` and `Del` on `sv != ""`).
/// - Remaining header keys are lowercased (matching gopass's `strings.ToLower`)
///   and kept as structured `Attribute`s in source order — no longer flattened
///   into the body.
fn parse_legacy(normalized: &str) -> Option<LegacyParts> {
    // Skip the magic first line.
    let after_magic = match normalized.find('\n') {
        Some(i) => &normalized[i + 1..],
        None => "", // magic-only file
    };
    let lines: Vec<&str> = after_magic.split('\n').collect();

    let mut password: Option<String> = None;
    let mut password_seen = false; // has the first `Password:` header been flushed?
    let mut header_attrs: Vec<Attribute> = Vec::new();
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
            &mut header_attrs,
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
        &mut header_attrs,
    );

    let body_text = lines.get(body_start_idx..).unwrap_or_default().join("\n");
    // `trim_end` is load-bearing: the normalized input already had trailing
    // whitespace stripped, but the header block may end with a blank-line
    // terminator whose join leaves a trailing newline.
    let body_bytes = body_text.trim_end().as_bytes().to_vec();

    Some((
        Zeroizing::new(password.unwrap_or_default().into_bytes()),
        header_attrs,
        body_bytes,
    ))
}

/// Commit the pending header (`cur_key`/`cur_val`) into either the password slot
/// or the header-attribute list. Mirrors gopass's `Password` handling: extraction
/// (and dropping) is gated on the FIRST `Password:` header having a non-empty
/// value.
fn flush(
    cur_key: &mut Option<String>,
    cur_val: &mut String,
    password: &mut Option<String>,
    password_seen: &mut bool,
    header_attrs: &mut Vec<Attribute>,
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
            // header (gopass: hdr.Get + hdr.Del, both gated on `sv != ""`).
            *password = Some(value.to_string());
        } else if password.is_some() {
            // A previous Password was extracted → drop this one too.
        } else {
            // First Password was empty → gopass leaves all Password headers in
            // place; keep this one as a structured attribute.
            header_attrs.push(Attribute::new(key.into_bytes(), value.as_bytes().to_vec()));
        }
    } else {
        header_attrs.push(Attribute::new(key.into_bytes(), value.as_bytes().to_vec()));
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
        assert!(secret.attributes().is_empty());
    }

    #[test]
    fn parse_password_and_body() {
        let content = b"hunter2\nusername: alice\nurl: example.com";
        let secret = Secret::parse(content).unwrap();
        assert_eq!(secret.password(), "hunter2");
        assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
        assert_eq!(secret.get("url"), Some(b"example.com".as_slice()));
        // No free-text line → empty body.
        assert_eq!(secret.body(), "");
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
        assert_eq!(secret.attribute_str("用户"), Some("张三"));
        assert_eq!(secret.attribute_str("网址"), Some("example.com"));
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
        assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
        assert_eq!(secret.body(), "free text");
    }

    #[test]
    fn parse_legacy_multiple_attributes_preserve_order() {
        let secret = Secret::parse(
            b"GOPASS-SECRET-1.0\nPassword: p\nusername: alice\nurl: example.com\ntype: ssh",
        )
        .unwrap();
        assert_eq!(secret.password(), "p");
        let pairs: Vec<(&[u8], &[u8])> = secret
            .attributes()
            .iter()
            .map(|a| (a.key(), a.value()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                (b"username".as_slice(), b"alice".as_slice()),
                (b"url".as_slice(), b"example.com".as_slice()),
                (b"type".as_slice(), b"ssh".as_slice()),
            ]
        );
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_no_password_header() {
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nusername: alice\n\nnotes").unwrap();
        assert_eq!(secret.password(), "");
        assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
        assert_eq!(secret.body(), "notes");
    }

    #[test]
    fn parse_legacy_folded_continuation() {
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nnote: line one\n  line two\n\nbody")
                .unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(secret.attribute_str("note"), Some("line one line two"));
        assert_eq!(secret.body(), "body");
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
        assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_crlf_in_header_block() {
        let secret = Secret::parse(
            b"GOPASS-SECRET-1.0\r\nPassword: hunter2\r\nusername: alice\r\n\r\nbody\r\n",
        )
        .unwrap();
        assert_eq!(secret.password(), "hunter2");
        assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
        assert_eq!(secret.body(), "body");
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
        assert_eq!(
            secret.get("url"),
            Some(b"https://example.com:8080/path".as_slice())
        );
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_empty_password_value_kept_in_body() {
        // gopass parity: an empty-value `Password:` header is NOT extracted
        // (the `sv != ""` guard) and stays as a structured `password` attribute.
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword:\nFoo: Bar").unwrap();
        assert_eq!(secret.password(), "");
        assert_eq!(secret.get("password"), Some(b"".as_slice()));
        assert_eq!(secret.get("foo"), Some(b"Bar".as_slice()));
        assert_eq!(secret.body(), "");
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
        assert_eq!(secret.get("totp"), Some(b"ABCD".as_slice()));
        assert_eq!(secret.get("note"), Some(b"x".as_slice()));
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_magic_line_with_surrounding_whitespace() {
        let secret = Secret::parse(b"  GOPASS-SECRET-1.0  \nPassword: p\n").unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_no_colon_header_falls_back_to_modern() {
        // gopass parity: a no-colon line in the header block is malformed →
        // gopass falls back to ParseAKV(in) → password = the magic line. The
        // modern split then treats `Password: p` as an attribute.
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nthis has no colon\nmore body").unwrap();
        assert_eq!(secret.password(), "GOPASS-SECRET-1.0");
        // Modern fallback preserves key case (only parse_legacy lowercases MIME
        // headers), so the attribute is key "Password", matched case-insensitively.
        assert_eq!(secret.get_ci("password"), Some(b"p".as_slice()));
        assert_eq!(secret.body(), "this has no colon\nmore body");
    }

    #[test]
    fn parse_legacy_orphan_fold_falls_back_to_modern() {
        // gopass parity: a continuation line with no preceding header is
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
        assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_magic_literal_first_line_routes_to_legacy() {
        // gpm inherits gopass's footgun: a secret whose first line IS the magic
        // literal is treated as legacy (gopass uses the identical discriminator),
        // even if a user meant it as a modern password. With no `Password:`
        // header the password slot is empty and the rest becomes attributes.
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nusername: alice").unwrap();
        assert_eq!(secret.password(), "");
        assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_legacy_first_password_empty_then_non_empty_all_kept() {
        // gopass parity: the FIRST `Password:` header decides. Its value is
        // empty, so nothing is extracted and EVERY `Password:` header stays as a
        // structured attribute (gopass's `hdr.Del` is gated on `sv != ""`).
        let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword:\nPassword: second\n").unwrap();
        assert_eq!(secret.password(), "");
        let pw_values: Vec<&[u8]> = secret
            .attributes()
            .iter()
            .filter(|a| a.key() == b"password")
            .map(Attribute::value)
            .collect();
        assert_eq!(pw_values, vec![b"".as_slice(), b"second".as_slice()]);
    }

    #[test]
    fn parse_legacy_post_header_kv_line_becomes_attribute() {
        // R069 phase-2b parity: a `Key: Value` line in the post-header body is
        // promoted to an attribute (gopass `Body()` drops it from free text).
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\n\nfree\nnote: in body").unwrap();
        assert_eq!(secret.password(), "p");
        assert_eq!(secret.get("note"), Some(b"in body".as_slice()));
        assert_eq!(secret.body(), "free");
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
        assert_eq!(twice.attributes(), once.attributes());
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

    // ---- attribute region (R069 phase 2a → 2b source of truth) ----

    #[test]
    fn attributes_parsed_from_body() {
        let secret = Secret::parse(b"pw\nuser: alice\nurl: https://example.com\nnotes").unwrap();
        // `get` is exact-case (gopass `Get` parity).
        assert_eq!(secret.get("user"), Some(b"alice".as_slice()));
        assert_eq!(secret.attribute_str("url"), Some("https://example.com"));
        // A free-text line (no ": ") is not an attribute — it is the body.
        assert_eq!(secret.get("notes"), None);
        assert_eq!(secret.body(), "notes");
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
        // The legacy parser lowercases the CTE key, so a case-sensitive
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
        // And is_utf8() flags it so the edit-block holds.
        assert!(!secret.is_utf8());
    }

    #[test]
    fn non_utf8_attribute_key_edit_blocked() {
        // A non-UTF-8 attribute KEY must trip is_utf8() too (not just values),
        // or it would be lossy-serialized to the frontend and saved back corrupt.
        let secret = Secret::parse(b"pw\n\xff\xfe: value").unwrap();
        assert!(secret.attributes().iter().any(|a| a.key() == b"\xff\xfe"));
        assert!(!secret.is_utf8());
    }

    // ---- from_parts assembler (R069 phase 2b write path) ----

    #[test]
    fn from_parts_assembles_all_layouts() {
        // password-only.
        let s = Secret::from_parts(b"pw".to_vec(), vec![], vec![]).unwrap();
        assert_eq!(s.to_bytes().as_slice(), b"pw");
        // + attributes.
        let s = Secret::from_parts(
            b"pw".to_vec(),
            vec![
                Attribute::new(b"user".to_vec(), b"alice".to_vec()),
                Attribute::new(b"url".to_vec(), b"example.com".to_vec()),
            ],
            vec![],
        )
        .unwrap();
        assert_eq!(
            s.to_bytes().as_slice(),
            b"pw\nuser: alice\nurl: example.com"
        );
        // + body.
        let s = Secret::from_parts(b"pw".to_vec(), vec![], b"notes".to_vec()).unwrap();
        assert_eq!(s.to_bytes().as_slice(), b"pw\nnotes");
        // + both.
        let s = Secret::from_parts(
            b"pw".to_vec(),
            vec![Attribute::new(b"user".to_vec(), b"alice".to_vec())],
            b"notes".to_vec(),
        )
        .unwrap();
        assert_eq!(s.to_bytes().as_slice(), b"pw\nuser: alice\nnotes");
        // empty password + content (attachment-style).
        let s = Secret::from_parts(
            vec![],
            vec![Attribute::new(
                b"Content-Transfer-Encoding".to_vec(),
                b"Base64".to_vec(),
            )],
            b"QUJD".to_vec(),
        )
        .unwrap();
        assert_eq!(
            s.to_bytes().as_slice(),
            b"\nContent-Transfer-Encoding: Base64\nQUJD"
        );
    }

    #[test]
    fn from_parts_rejects_invalid_keys_and_values() {
        // key with the ": " separator.
        let err = Secret::from_parts(
            b"pw".to_vec(),
            vec![Attribute::new(b"user: x".to_vec(), b"v".to_vec())],
            vec![],
        )
        .unwrap_err();
        assert_eq!(err.code, "SECRET_INVALID");
        // key with a newline.
        let err = Secret::from_parts(
            b"pw".to_vec(),
            vec![Attribute::new(b"bad\nkey".to_vec(), b"v".to_vec())],
            vec![],
        )
        .unwrap_err();
        assert_eq!(err.code, "SECRET_INVALID");
        // value with a newline.
        let err = Secret::from_parts(
            b"pw".to_vec(),
            vec![Attribute::new(b"k".to_vec(), b"bad\nvalue".to_vec())],
            vec![],
        )
        .unwrap_err();
        assert_eq!(err.code, "SECRET_INVALID");
        // password with a newline breaks the first-line invariant.
        let err = Secret::from_parts(b"bad\npw".to_vec(), vec![], vec![]).unwrap_err();
        assert_eq!(err.code, "SECRET_INVALID");
        // a key with a lone colon (no space) is fine — it is not the separator.
        let s = Secret::from_parts(
            b"pw".to_vec(),
            vec![Attribute::new(b"a:b".to_vec(), b"v".to_vec())],
            vec![],
        )
        .unwrap();
        assert_eq!(s.to_bytes().as_slice(), b"pw\na:b: v");
    }

    // ---- legacy YAML read-only branch (A004) ----

    #[test]
    fn parse_yaml_marker_line_marks_secret_readonly() {
        // A `---` line routes the whole secret to the YAML branch: no attribute
        // splitting (the block's `k: v` lines stay in the opaque body), password
        // = first line, marker kept in the body.
        let secret = Secret::parse(b"password\n---\notp: bar").unwrap();
        assert!(secret.is_yaml());
        assert_eq!(secret.password(), "password");
        assert!(secret.attributes().is_empty());
        assert_eq!(secret.body_bytes(), b"---\notp: bar");
    }

    #[test]
    fn parse_yaml_bare_doc_first_line_marker_has_empty_password() {
        // gopass ParseYAML: a first line that IS the marker is a bare YAML
        // document — no password line precedes it. NOTE: this parse-level
        // branch is reached only when some LATER line also carries a marker
        // (a single `---` line is just a password-bearing secret whose body
        // is empty); the bare-doc-yaml shape gopass writes always has YAML
        // content after the marker.
        let secret = Secret::parse(b"---\nusername: alice\nurl: example.com").unwrap();
        assert!(secret.is_yaml());
        assert_eq!(secret.password(), "");
        assert_eq!(
            secret.body_bytes(),
            b"---\nusername: alice\nurl: example.com"
        );
    }

    #[test]
    fn parse_yaml_bare_doc_single_marker_only() {
        // Marker-only file: still YAML, empty password, body = the marker.
        let secret = Secret::parse(b"---").unwrap();
        assert!(secret.is_yaml());
        assert_eq!(secret.password(), "");
        assert_eq!(secret.body_bytes(), b"---");
    }

    #[test]
    fn parse_without_marker_stays_akv() {
        // Pure AKV (with attributes and body) — unchanged behavior, is_yaml
        // false, attributes split as before.
        let secret = Secret::parse(b"pw\nmy notes\nuser: alice").unwrap();
        assert!(!secret.is_yaml());
        assert_eq!(secret.get("user"), Some(b"alice".as_slice()));
        assert_eq!(secret.body(), "my notes");
    }

    #[test]
    fn parse_pem_armor_body_is_editable_akv() {
        // gopass's Peek(3) token matches PEM armor (`-----BEGIN …`), but its
        // YAML decode then fails and the cascade falls back to AKV — gopass's
        // effective classification of armor is editable AKV. gpm matches the
        // outcome, not the token: armor lines start `----` and are excluded
        // from the marker, so an armored key body stays a normal editable
        // secret (and the write-path guard does not refuse key material).
        let secret = Secret::parse(
            b"pw\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----",
        )
        .unwrap();
        assert!(!secret.is_yaml());
        assert_eq!(secret.password(), "pw");
        // The armor has no `": "` lines, so it is all free-text body.
        assert!(secret.attributes().is_empty());
        assert!(secret.body_bytes().starts_with(b"-----BEGIN"));
    }

    #[test]
    fn parse_password_starting_with_marker_stays_akv() {
        // gopass consumes the first line as the password before its marker
        // peek, so a password that itself starts `---` is never a YAML
        // marker — the secret stays AKV (gopass ParseYAML would read the
        // password line, then find no marker in an empty body).
        let secret = Secret::parse(b"---hunter2\nuser: alice").unwrap();
        assert!(!secret.is_yaml());
        assert_eq!(secret.password(), "---hunter2");
        assert_eq!(secret.get("user"), Some(b"alice".as_slice()));
    }

    #[test]
    fn parse_bare_doc_trailing_space_marker_is_bare() {
        // gopass TrimSpaces the first line before the bare-doc comparison, so
        // `--- ` (trailing space) is a bare YAML document with an empty
        // password — not a secret whose password is the junk string "--- ".
        let secret = Secret::parse(b"--- \nusername: alice").unwrap();
        assert!(secret.is_yaml());
        assert_eq!(secret.password(), "");
        assert_eq!(secret.body_bytes(), b"--- \nusername: alice");
    }

    #[test]
    fn parse_yaml_crlf_normalized_before_marker_match() {
        // CRLF endings normalize to LF before detection, so a `---\r\n` line
        // matches the marker (gopass reads CRLF YAML the same way).
        let secret = Secret::parse(b"password\r\n---\r\notp: bar\r\n").unwrap();
        assert!(secret.is_yaml());
        assert_eq!(secret.body_bytes(), b"---\notp: bar");
    }

    #[test]
    fn parse_yaml_trailing_space_marker_still_matches() {
        // gopass Peek(3) only checks the first three bytes: `--- ` (trailing
        // space) matches its token, so gpm matches it too — conservative
        // read-only beats a possible corrupting edit.
        let secret = Secret::parse(b"password\n--- \notp: bar").unwrap();
        assert!(secret.is_yaml());
    }

    #[test]
    fn parse_mime_with_yaml_marker_line_stays_akv() {
        // ORDER INVARIANT: the `---` line (no colon) makes the MIME header
        // block malformed; gopass's cascade then short-circuits PermanentError
        // straight to ParseAKV — skipping ParseYAML — so gpm must not take the
        // YAML branch either. Password = the magic line (the modern fallback),
        // `username: alice` an attribute, `---` body text.
        let secret =
            Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\n---\nusername: alice").unwrap();
        assert!(!secret.is_yaml());
        assert_eq!(secret.password(), "GOPASS-SECRET-1.0");
        assert_eq!(secret.get_ci("password"), Some(b"p".as_slice()));
        assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
        assert_eq!(secret.body(), "---");
    }

    #[test]
    fn from_parts_is_never_yaml() {
        // The assembler builds AKV secrets only; a YAML secret never reaches
        // the write path.
        let s = Secret::from_parts(b"pw".to_vec(), vec![], b"notes".to_vec()).unwrap();
        assert!(!s.is_yaml());
    }

    #[test]
    fn contains_yaml_marker_direct() {
        // The write-path guard reuses the same rule; pin its semantics.
        assert!(contains_yaml_marker(b"pw\n---\nk: v"));
        assert!(contains_yaml_marker(b"pw\n--- \nk: v"));
        assert!(contains_yaml_marker(b"x\ny\n---\nlate marker"));
        // first line skipped:
        assert!(!contains_yaml_marker(b"---\nk: v"));
        assert!(!contains_yaml_marker(b"---"));
        assert!(!contains_yaml_marker(b"---hunter2\nuser: a"));
        // armor and 4-dash rules are not markers:
        assert!(!contains_yaml_marker(b"pw\n-----BEGIN X-----\n"));
        assert!(!contains_yaml_marker(b"pw\n----\nk: v"));
        assert!(!contains_yaml_marker(b"pw\nnotes\nk: v"));
        assert!(!contains_yaml_marker(b""));
    }
}
