// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Password generator (gopass `pkg/pwgen` analogue).
//!
//! Mirrors gopass's create-wizard generation: a `type: "password"` field is
//! generated from its own `charset` when one is set (e.g. a PIN over
//! `0123456789`), and otherwise from the selected [`GenerateMode`] (random,
//! memorable, xkcd). External delegation is deferred.

use std::sync::OnceLock;

use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};
use crate::rng::uniform_index;

/// Default generated length, in characters (gopass-aligned).
pub const DEFAULT_PASSWORD_LEN: usize = 24;
/// Hard ceiling on a generated length, defending the allocation and sampling
/// loop in [`sample_string`] against an absurd `min`/`max` arriving over IPC.
const MAX_PASSWORD_LEN: usize = 256;

/// Minimum total length a memorable password is built to.
const MEMORABLE_MIN_LEN: usize = 16;

/// Number of words in an xkcd-style passphrase (gopass default).
const XKCD_WORDS: usize = 4;

/// Max retries when a `strict` character-class validator rejects a candidate.
const MAX_STRICT_TRIES: usize = 64;

const RAW_DIGITS: &[u8] = b"0123456789";
const RAW_UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const RAW_LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
/// Curated, form-safe symbols (no quotes / backtick / backslash / angle
/// brackets / slash, which some sites reject or mishandle).
const RAW_SYMBOLS: &[u8] = b"!@#$%^&*-_=+?";
/// Visually-ambiguous characters excluded from the default alphabet (gopass `Ambiq`).
const AMBIGUOUS: &[u8] = b"0ODQ1IlB8G6S5Z2";

/// Generator method. Serialized lowercase over IPC to match the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenerateMode {
    /// Random characters over the (default or charset) alphabet.
    Random,
    /// Wordlist-based: `word + digit` repeated to a minimum length.
    Memorable,
    /// N-word passphrase joined by spaces (xkcd-936 style).
    Xkcd,
}

/// Parameters for [`generate_password`]. Mirrors the relevant fields of gopass's
/// create-wizard `Attribute`.
#[derive(Debug, Clone, Copy)]
pub struct GenerateOptions<'a> {
    /// Generator method (ignored when `charset` is set — the charset locks it).
    pub mode: GenerateMode,
    /// Per-field alphabet (gopass `charset`). When set, generation is
    /// charset-driven (e.g. `0123456789` for a PIN) and `mode` is ignored.
    pub charset: Option<&'a str>,
    /// Minimum length (gopass `min`).
    pub min_len: Option<usize>,
    /// Maximum length (gopass `max`).
    pub max_len: Option<usize>,
    /// Require every character class present in the alphabet to be represented
    /// (gopass `strict`).
    pub strict: bool,
}

/// Generate a password per `opts`, mirroring gopass's create-wizard semantics.
///
/// With `charset` set the result is random over that alphabet (the PIN path);
/// otherwise the selected [`GenerateMode`] drives it. The result is wrapped in
/// [`Zeroizing<String>`] so the in-process copy is wiped on drop.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the OS RNG fails, or if a charset is
/// supplied but empty.
pub fn generate_password(opts: &GenerateOptions<'_>) -> Result<Zeroizing<String>, Error> {
    match opts.charset {
        Some(cs) => generate_from_charset(cs, opts.min_len, opts.max_len, opts.strict),
        None => match opts.mode {
            GenerateMode::Random => {
                random_chars(default_alphabet(), DEFAULT_PASSWORD_LEN, opts.strict)
            }
            GenerateMode::Memorable => memorable_password(),
            GenerateMode::Xkcd => xkcd_password(),
        },
    }
}

/// Charset-driven generation (the PIN path): random over `charset`, length
/// clamped to `[min, max]`, `strict` enforces all classes present in the charset.
fn generate_from_charset(
    charset: &str,
    min_len: Option<usize>,
    max_len: Option<usize>,
    strict: bool,
) -> Result<Zeroizing<String>, Error> {
    let alphabet = charset.as_bytes();
    if alphabet.is_empty() {
        return Err(Error::new(
            ErrorCode::StoreError,
            "password charset is empty",
        ));
    }
    random_chars(alphabet, target_len(min_len, max_len), strict)
}

/// Default generation length clamped into `[min, max]`.
fn target_len(min_len: Option<usize>, max_len: Option<usize>) -> usize {
    let mut len = DEFAULT_PASSWORD_LEN;
    if let Some(mn) = min_len {
        len = len.max(mn);
    }
    if let Some(mx) = max_len {
        len = len.min(mx);
    }
    len.clamp(1, MAX_PASSWORD_LEN)
}

/// Build a `len`-character string by uniform sampling from `alphabet`. When
/// `strict`, retry until every class present in `alphabet` is represented.
fn random_chars(alphabet: &[u8], len: usize, strict: bool) -> Result<Zeroizing<String>, Error> {
    if strict {
        let classes = classes_in(alphabet);
        for _ in 0..MAX_STRICT_TRIES {
            let pw = sample_string(alphabet, len)?;
            if has_all_classes(pw.as_bytes(), &classes) {
                return Ok(Zeroizing::new(pw));
            }
        }
        return Err(Error::new(
            ErrorCode::StoreError,
            "could not satisfy strict character-class rules",
        ));
    }
    sample_string(alphabet, len).map(Zeroizing::new)
}

/// Sample a `len`-character ASCII string uniformly from `alphabet`.
fn sample_string(alphabet: &[u8], len: usize) -> Result<String, Error> {
    let n = alphabet.len();
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        let idx = uniform_index(n)?;
        let b = alphabet
            .get(idx)
            .copied()
            .expect("index in range by construction");
        out.push(char::from(b));
    }
    Ok(out)
}

/// Memorable password over the bundled wordlist: `word + digit` repeated until
/// at least [`MEMORABLE_MIN_LEN`] characters long (gopass `memorable.go` shape).
fn memorable_password() -> Result<Zeroizing<String>, Error> {
    memorable_password_with(MEMORABLE_MIN_LEN, wordlist())
}

/// Memorable password over a supplied wordlist (test seam).
fn memorable_password_with(min_len: usize, words: &[&str]) -> Result<Zeroizing<String>, Error> {
    let n_words = words.len();
    if n_words == 0 {
        return Err(Error::new(ErrorCode::StoreError, "wordlist is empty"));
    }
    let n_digits = RAW_DIGITS.len();
    let mut out = String::new();
    while out.len() < min_len {
        let wi = uniform_index(n_words)?;
        let di = uniform_index(n_digits)?;
        let word = words.get(wi).copied().expect("word index in range");
        let digit = char::from(RAW_DIGITS.get(di).copied().expect("digit index in range"));
        out.push_str(word);
        out.push(digit);
    }
    Ok(Zeroizing::new(out))
}

/// Xkcd-style passphrase over the bundled wordlist: [`XKCD_WORDS`] words joined
/// by single spaces.
fn xkcd_password() -> Result<Zeroizing<String>, Error> {
    xkcd_password_with(XKCD_WORDS, wordlist())
}

/// Xkcd-style passphrase over a supplied wordlist (test seam).
fn xkcd_password_with(n_words: usize, words: &[&str]) -> Result<Zeroizing<String>, Error> {
    let n = words.len();
    if n == 0 {
        return Err(Error::new(ErrorCode::StoreError, "wordlist is empty"));
    }
    let mut chosen: Vec<&str> = Vec::with_capacity(n_words);
    for _ in 0..n_words {
        let i = uniform_index(n)?;
        chosen.push(words.get(i).copied().expect("word index in range"));
    }
    Ok(Zeroizing::new(chosen.join(" ")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Digit,
    Upper,
    Lower,
    Symbol,
}

fn class_of(b: u8) -> Class {
    if b.is_ascii_digit() {
        Class::Digit
    } else if b.is_ascii_uppercase() {
        Class::Upper
    } else if b.is_ascii_lowercase() {
        Class::Lower
    } else {
        Class::Symbol
    }
}

/// Character classes present in `alphabet`, in first-seen order.
fn classes_in(alphabet: &[u8]) -> Vec<Class> {
    let mut v: Vec<Class> = Vec::new();
    for &b in alphabet {
        let c = class_of(b);
        if !v.contains(&c) {
            v.push(c);
        }
    }
    v
}

/// True if `pw` contains at least one byte of every class in `required`.
fn has_all_classes(pw: &[u8], required: &[Class]) -> bool {
    required
        .iter()
        .all(|&c| pw.iter().any(|&b| class_of(b) == c))
}

/// Default alphabet: digits + upper + lower + curated symbols, with visually
/// ambiguous characters removed. Built once and cached.
fn default_alphabet() -> &'static [u8] {
    static ALPHA: OnceLock<Box<[u8]>> = OnceLock::new();
    ALPHA.get_or_init(|| {
        let mut v = Vec::new();
        v.extend_from_slice(RAW_DIGITS);
        v.extend_from_slice(RAW_UPPER);
        v.extend_from_slice(RAW_LOWER);
        v.extend_from_slice(RAW_SYMBOLS);
        v.retain(|&c| !AMBIGUOUS.contains(&c));
        v.into_boxed_slice()
    })
}

/// Bundled BIP39 English wordlist (2048 words), parsed once and cached.
fn wordlist() -> &'static [&'static str] {
    static WORDS: OnceLock<Box<[&'static str]>> = OnceLock::new();
    WORDS.get_or_init(|| {
        include_str!("wordlist/english.txt")
            .split_ascii_whitespace()
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_mode_serde_is_lowercase() {
        assert_eq!(
            serde_json::to_string(&GenerateMode::Random).unwrap(),
            "\"random\""
        );
        assert_eq!(
            serde_json::to_string(&GenerateMode::Memorable).unwrap(),
            "\"memorable\""
        );
        assert_eq!(
            serde_json::to_string(&GenerateMode::Xkcd).unwrap(),
            "\"xkcd\""
        );
        for s in ["random", "memorable", "xkcd"] {
            let m: GenerateMode = serde_json::from_str(&format!("\"{s}\"")).unwrap();
            assert_eq!(serde_json::to_string(&m).unwrap(), format!("\"{s}\""));
        }
    }

    #[test]
    fn charset_pin_is_all_digits() {
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some("0123456789"),
            min_len: Some(1),
            max_len: Some(64),
            strict: false,
        };
        for _ in 0..16 {
            let pw = generate_password(&opts).unwrap();
            let s = &*pw;
            assert!(!s.is_empty(), "PIN should not be empty");
            assert!(s.len() <= 64, "PIN exceeds max: {s}");
            assert!(
                s.bytes().all(|b| b.is_ascii_digit()),
                "PIN has non-digits: {s}"
            );
        }
    }

    #[test]
    fn charset_empty_is_an_error() {
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some(""),
            min_len: None,
            max_len: None,
            strict: false,
        };
        assert!(generate_password(&opts).is_err());
    }

    #[test]
    fn random_default_length_and_alphabet() {
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: None,
            min_len: None,
            max_len: None,
            strict: false,
        };
        let alpha = default_alphabet();
        for _ in 0..32 {
            let pw = generate_password(&opts).unwrap();
            let s = &*pw;
            assert_eq!(s.len(), DEFAULT_PASSWORD_LEN, "wrong length: {s}");
            assert!(
                s.bytes().all(|b| alpha.contains(&b)),
                "char outside default alphabet: {s}"
            );
            assert!(
                !s.bytes().any(|b| AMBIGUOUS.contains(&b)),
                "ambiguous char present: {s}"
            );
        }
    }

    #[test]
    fn random_strict_has_every_class() {
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: None,
            min_len: None,
            max_len: None,
            strict: true,
        };
        let pw = generate_password(&opts).unwrap();
        let bytes = pw.as_bytes();
        assert!(bytes.iter().any(u8::is_ascii_digit), "no digit");
        assert!(bytes.iter().any(u8::is_ascii_uppercase), "no upper");
        assert!(bytes.iter().any(u8::is_ascii_lowercase), "no lower");
        assert!(
            bytes.iter().any(|b| class_of(*b) == Class::Symbol),
            "no symbol"
        );
    }

    #[test]
    fn memorable_uses_known_words_and_digits() {
        let words = ["aa", "bb", "cc"];
        let pw = memorable_password_with(MEMORABLE_MIN_LEN, &words).unwrap();
        let s = &*pw;
        assert!(s.len() >= MEMORABLE_MIN_LEN, "too short: {s}");
        // Structure: alternating known 2-letter words and single digits.
        let mut chars = s.chars().peekable();
        while chars.peek().is_some() {
            let w: String = (0..2).filter_map(|_| chars.next()).collect();
            assert!(
                words.contains(&w.as_str()),
                "unknown word chunk {w:?} in {s}"
            );
            let d = chars.next();
            assert!(
                d.is_some_and(|c| c.is_ascii_digit()),
                "missing digit in {s}"
            );
        }
    }

    #[test]
    fn xkcd_is_four_known_words() {
        let words = ["alpha", "beta", "gamma"];
        let pw = xkcd_password_with(XKCD_WORDS, &words).unwrap();
        let s = &*pw;
        let parts: Vec<&str> = s.split(' ').collect();
        assert_eq!(parts.len(), XKCD_WORDS, "wrong word count: {s}");
        assert!(
            parts.iter().all(|p| words.contains(p)),
            "unknown word in {s}"
        );
    }

    #[test]
    fn xkcd_from_real_wordlist_is_known_words() {
        let pw = xkcd_password().unwrap();
        let s = &*pw;
        let words = wordlist();
        for part in s.split(' ') {
            assert!(words.contains(&part), "word not in list: {part}");
        }
    }

    #[test]
    fn every_mode_returns_nonempty() {
        for mode in [
            GenerateMode::Random,
            GenerateMode::Memorable,
            GenerateMode::Xkcd,
        ] {
            let opts = GenerateOptions {
                mode,
                charset: None,
                min_len: None,
                max_len: None,
                strict: false,
            };
            let pw = generate_password(&opts).unwrap();
            assert!(!pw.is_empty(), "{mode:?} returned empty");
        }
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn uniform_index_is_unbiased_enough() {
        // Modulo-bias guard: a million draws over n=7 should land within ±5%
        // of even for each value. Catches a broken sampler, not a CSPRNG proof.
        let n = 7_usize;
        let mut counts = vec![0u32; n];
        for _ in 0..1_000_000 {
            let idx = uniform_index(n).unwrap();
            if let Some(slot) = counts.get_mut(idx) {
                *slot += 1;
            }
        }
        let expected = 1_000_000.0 / n as f64;
        for (i, &c) in counts.iter().enumerate() {
            let ratio = f64::from(c) / expected;
            assert!(
                (0.95..=1.05).contains(&ratio),
                "index {i} out of ±5%: ratio {ratio}"
            );
        }
    }

    #[test]
    fn strict_fails_when_classes_exceed_length() {
        // 4 classes (digit/lower/upper/symbol) but length clamped to 1 → impossible.
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some("0aA!"),
            min_len: Some(1),
            max_len: Some(1),
            strict: true,
        };
        let err = generate_password(&opts).unwrap_err();
        assert_eq!(err.code, "STORE_ERROR");
    }

    #[test]
    fn memorable_and_xkcd_reject_empty_wordlist() {
        assert!(memorable_password_with(MEMORABLE_MIN_LEN, &[]).is_err());
        assert!(xkcd_password_with(XKCD_WORDS, &[]).is_err());
    }

    #[test]
    fn target_len_clamps_into_min_max_with_cap() {
        assert_eq!(target_len(None, None), DEFAULT_PASSWORD_LEN);
        assert_eq!(target_len(Some(40), None), 40); // min raises
        assert_eq!(target_len(None, Some(8)), 8); // max lowers
        assert_eq!(target_len(Some(10), Some(2)), 2); // min>max → max wins
        assert_eq!(target_len(None, Some(0)), 1); // floor at 1
        // An absurd min over IPC is capped, never reaching the allocator.
        assert_eq!(target_len(Some(usize::MAX), None), MAX_PASSWORD_LEN);
    }
}
