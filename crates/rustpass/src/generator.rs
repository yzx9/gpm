// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

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
/// create-wizard `Attribute`, extended with the standalone generator's minimum
/// class counts and ambiguous-character switch.
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
    /// Minimum number of digits in a random/charset password. `0` disables the
    /// requirement. Unlike `strict`, this is a count, not a presence check.
    pub min_digits: usize,
    /// Minimum number of symbols in a random/charset password. `0` disables.
    pub min_symbols: usize,
    /// Remove visually ambiguous characters (`0ODQ1IlB8G6S5Z2`) from an explicit
    /// `charset`. The default random alphabet already omits them.
    pub avoid_ambiguous: bool,
}

/// Generate a password per `opts`, mirroring gopass's create-wizard semantics.
///
/// With `charset` set the result is random over that alphabet (the PIN path);
/// otherwise the selected [`GenerateMode`] drives it. For the charset-less modes,
/// `min_len`/`max_len` shape the length: `Random` lands at [`target_len`] (`min ==
/// max` pins an exact length; `None` keeps [`DEFAULT_PASSWORD_LEN`]), and
/// `Memorable` builds to at least `min_len` ([`MEMORABLE_MIN_LEN`] by default). The
/// result is wrapped in [`Zeroizing<String>`] so the in-process copy is wiped on
/// drop.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the OS RNG fails, or if a charset is
/// supplied but empty.
pub fn generate_password(opts: &GenerateOptions<'_>) -> Result<Zeroizing<String>, Error> {
    match opts.charset {
        Some(cs) => generate_from_charset(
            cs,
            opts.min_len,
            opts.max_len,
            opts.strict,
            opts.min_digits,
            opts.min_symbols,
            opts.avoid_ambiguous,
        ),
        None => match opts.mode {
            GenerateMode::Random => random_chars(
                default_alphabet(),
                target_len(opts.min_len, opts.max_len),
                opts.strict,
                opts.min_digits,
                opts.min_symbols,
            ),
            GenerateMode::Memorable => memorable_password_with(
                // Clamp like the random path's `target_len`: memorable honors
                // `min_len` directly, and the renderer's `max="256"` is advisory,
                // not a trust boundary. Without this, `min_len: 0` yields an empty
                // password and a huge `min_len` is an unbounded allocation.
                opts.min_len
                    .unwrap_or(MEMORABLE_MIN_LEN)
                    .clamp(1, MAX_PASSWORD_LEN),
                wordlist(),
            ),
            GenerateMode::Xkcd => xkcd_password(),
        },
    }
}

/// Charset-driven generation (the PIN path): random over `charset`, length
/// clamped to `[min, max]`, `strict` enforces all classes present in the charset.
#[allow(clippy::too_many_arguments)]
fn generate_from_charset(
    charset: &str,
    min_len: Option<usize>,
    max_len: Option<usize>,
    strict: bool,
    min_digits: usize,
    min_symbols: usize,
    avoid_ambiguous: bool,
) -> Result<Zeroizing<String>, Error> {
    let alphabet: Vec<u8> = charset
        .bytes()
        .filter(|&b| !avoid_ambiguous || !AMBIGUOUS.contains(&b))
        .collect();
    if alphabet.is_empty() {
        return Err(Error::new(
            ErrorCode::StoreError,
            if charset.is_empty() {
                "password charset is empty"
            } else {
                "password charset is empty after removing ambiguous characters"
            },
        ));
    }
    random_chars(
        &alphabet,
        target_len(min_len, max_len),
        strict,
        min_digits,
        min_symbols,
    )
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

/// Build a `len`-character string by uniform sampling from `alphabet`, with
/// minimum per-class counts. Required characters are seeded first, the rest are
/// sampled uniformly, then the whole result is shuffled so required classes
/// aren't clustered at the front. `strict` promotes every class present in
/// `alphabet` to a minimum of one.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] when a requirement cannot be met (an empty
/// alphabet, a quota for an absent class, or quotas longer than `len`).
fn random_chars(
    alphabet: &[u8],
    len: usize,
    strict: bool,
    min_digits: usize,
    min_symbols: usize,
) -> Result<Zeroizing<String>, Error> {
    if alphabet.is_empty() {
        return Err(Error::new(
            ErrorCode::StoreError,
            "password charset is empty",
        ));
    }
    let len = len.clamp(1, MAX_PASSWORD_LEN);
    let digit_pool = class_pool(alphabet, Class::Digit);
    let symbol_pool = class_pool(alphabet, Class::Symbol);
    if min_digits > 0 && digit_pool.is_empty() {
        return Err(Error::new(
            ErrorCode::StoreError,
            "password length requires digits but the charset has none",
        ));
    }
    if min_symbols > 0 && symbol_pool.is_empty() {
        return Err(Error::new(
            ErrorCode::StoreError,
            "password length requires symbols but the charset has none",
        ));
    }

    // Required count per class. Quotas are saturating-summed so an absurd
    // client-supplied count can't overflow before the length check rejects it.
    let mut required: Vec<(Class, usize)> = Vec::new();
    for class in classes_in(alphabet) {
        let count = match class {
            Class::Digit => min_digits,
            Class::Symbol => min_symbols,
            Class::Upper | Class::Lower => 0,
        };
        let count = if strict { count.max(1) } else { count };
        if count > 0 {
            required.push((class, count));
        }
    }
    let required_len = required
        .iter()
        .fold(0_usize, |sum, (_, count)| sum.saturating_add(*count));
    if required_len > len {
        return Err(Error::new(
            ErrorCode::StoreError,
            "password length is too short for the requested character requirements",
        ));
    }

    let mut out: Vec<u8> = Vec::with_capacity(len);
    for (class, count) in required {
        let pool = class_pool(alphabet, class);
        for _ in 0..count {
            let idx = uniform_index(pool.len())?;
            out.push(*pool.get(idx).expect("index in range by construction"));
        }
    }
    while out.len() < len {
        let idx = uniform_index(alphabet.len())?;
        out.push(*alphabet.get(idx).expect("index in range by construction"));
    }
    shuffle(&mut out)?;

    // Every alphabet byte is ASCII in practice; keep the conversion total so a
    // future non-ASCII charset can't panic here.
    Ok(Zeroizing::new(String::from_utf8_lossy(&out).into_owned()))
}

/// All bytes of `alphabet` in `class`.
fn class_pool(alphabet: &[u8], class: Class) -> Vec<u8> {
    alphabet
        .iter()
        .copied()
        .filter(|&b| class_of(b) == class)
        .collect()
}

/// In-place Fisher-Yates shuffle over the OS CSPRNG.
fn shuffle(items: &mut [u8]) -> Result<(), Error> {
    for i in (1..items.len()).rev() {
        let j = uniform_index(i + 1)?;
        items.swap(i, j);
    }
    Ok(())
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
        include_str!("../data/wordlist/english.txt")
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
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: false,
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
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: false,
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
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: false,
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
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: false,
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
    fn random_honors_explicit_length() {
        // min == max pins the exact length (target_len starts at 24 and only raises).
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: None,
            min_len: Some(8),
            max_len: Some(8),
            strict: false,
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: false,
        };
        for _ in 0..16 {
            let pw = generate_password(&opts).unwrap();
            let s = &*pw;
            assert_eq!(s.len(), 8, "wrong length: {s}");
        }
    }

    #[test]
    fn random_honors_explicit_long_length() {
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: None,
            min_len: Some(100),
            max_len: Some(100),
            strict: false,
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: false,
        };
        let pw = generate_password(&opts).unwrap();
        let s = &*pw;
        assert_eq!(s.len(), 100, "wrong length: {s}");
    }

    #[test]
    fn explicit_charset_honors_min_digits_and_symbols() {
        let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*";
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some(alphabet),
            min_len: Some(32),
            max_len: Some(32),
            strict: false,
            min_digits: 4,
            min_symbols: 3,
            avoid_ambiguous: false,
        };
        for _ in 0..64 {
            let pw = generate_password(&opts).unwrap();
            let s = &*pw;
            assert_eq!(s.len(), 32);
            assert!(
                s.bytes().filter(u8::is_ascii_digit).count() >= 4,
                "too few digits: {s}"
            );
            assert!(
                s.bytes().filter(|b| class_of(*b) == Class::Symbol).count() >= 3,
                "too few symbols: {s}"
            );
            assert!(
                s.bytes().all(|b| alphabet.as_bytes().contains(&b)),
                "char outside charset: {s}"
            );
        }
    }

    #[test]
    fn strict_minimums_fit_a_short_exact_length() {
        // Every class seeded, quotas filled, then shuffled into exactly 4 chars.
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some("0aA!"),
            min_len: Some(4),
            max_len: Some(4),
            strict: true,
            min_digits: 1,
            min_symbols: 1,
            avoid_ambiguous: false,
        };
        for _ in 0..16 {
            let pw = generate_password(&opts).unwrap();
            let s = &*pw;
            assert_eq!(s.len(), 4);
            assert!(s.bytes().any(|b| b.is_ascii_digit()), "no digit: {s}");
            assert!(s.bytes().any(|b| b.is_ascii_uppercase()), "no upper: {s}");
            assert!(s.bytes().any(|b| b.is_ascii_lowercase()), "no lower: {s}");
            assert!(
                s.bytes().any(|b| class_of(b) == Class::Symbol),
                "no symbol: {s}"
            );
        }
    }

    #[test]
    fn avoid_ambiguous_filters_the_requested_charset() {
        // Filtering leaves only "A"; every generated char must be "A".
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some("A0"),
            min_len: Some(8),
            max_len: Some(8),
            strict: false,
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: true,
        };
        for _ in 0..16 {
            let pw = generate_password(&opts).unwrap();
            assert_eq!(pw.as_str(), "A".repeat(8).as_str());
        }
    }

    #[test]
    fn impossible_class_quotas_are_an_error() {
        // Sum of minimums exceeds the length.
        let too_many = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some("0a"),
            min_len: Some(3),
            max_len: Some(3),
            strict: false,
            min_digits: 2,
            min_symbols: 2,
            avoid_ambiguous: false,
        };
        assert!(generate_password(&too_many).is_err());

        // Requested digits/symbols but the selected charset has none.
        let no_digits = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some("abc"),
            min_len: Some(8),
            max_len: Some(8),
            strict: false,
            min_digits: 1,
            min_symbols: 0,
            avoid_ambiguous: false,
        };
        assert!(generate_password(&no_digits).is_err());

        let no_symbols = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some("abc"),
            min_len: Some(8),
            max_len: Some(8),
            strict: false,
            min_digits: 0,
            min_symbols: 1,
            avoid_ambiguous: false,
        };
        assert!(generate_password(&no_symbols).is_err());
    }

    #[test]
    fn avoid_ambiguous_can_empty_the_charset() {
        let opts = GenerateOptions {
            mode: GenerateMode::Random,
            charset: Some("0O"),
            min_len: Some(8),
            max_len: Some(8),
            strict: false,
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: true,
        };
        assert!(generate_password(&opts).is_err());
    }

    #[test]
    fn memorable_honors_min_len() {
        let opts = GenerateOptions {
            mode: GenerateMode::Memorable,
            charset: None,
            min_len: Some(40),
            max_len: None,
            strict: false,
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: false,
        };
        for _ in 0..16 {
            let pw = generate_password(&opts).unwrap();
            let s = &*pw;
            assert!(s.len() >= 40, "memorable below min: {s}");
        }
    }

    #[test]
    fn memorable_default_when_min_len_none() {
        // Regression: a null min_len still yields the built-in default floor.
        let opts = GenerateOptions {
            mode: GenerateMode::Memorable,
            charset: None,
            min_len: None,
            max_len: None,
            strict: false,
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: false,
        };
        let pw = generate_password(&opts).unwrap();
        let s = &*pw;
        assert!(s.len() >= MEMORABLE_MIN_LEN, "below default min: {s}");
    }

    #[test]
    fn memorable_clamps_an_extreme_min_len() {
        // 0 must not yield an empty password; usize::MAX must not hang/OOM.
        // Memorable honors `min_len` directly, so it must be capped like the
        // random path's `target_len`.
        for min in [0_usize, usize::MAX] {
            let opts = GenerateOptions {
                mode: GenerateMode::Memorable,
                charset: None,
                min_len: Some(min),
                max_len: None,
                strict: false,
                min_digits: 0,
                min_symbols: 0,
                avoid_ambiguous: false,
            };
            let pw = generate_password(&opts).unwrap();
            let s = &*pw;
            assert!(!s.is_empty(), "memorable empty for min={min}");
            // Capped at MAX_PASSWORD_LEN, plus at most one trailing word+digit.
            assert!(
                s.len() <= MAX_PASSWORD_LEN + 16,
                "memorable unbounded for min={min}: got {}",
                s.len()
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
                min_digits: 0,
                min_symbols: 0,
                avoid_ambiguous: false,
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
            min_digits: 0,
            min_symbols: 0,
            avoid_ambiguous: false,
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
