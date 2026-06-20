// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! At-rest encryption for local private files (`repo.json`, `identity`).
//!
//! AEAD-wraps blobs with a master key (AES-256-GCM) so that an attacker who
//! can *read* the app's private storage gets ciphertext, not the PAT / SSH key
//! / trust set, and so that silent tamper of those files is detected.
//!
//! The master key is **not** stored or generated here as a matter of
//! persistence — it is *injected* ([`AtRest::new`]) as plain bytes. In
//! production it comes (hardware-sealed) from the Android Keystore via the app
//! layer; on desktop and in tests it is `None`, which makes [`AtRest`] a
//! **passthrough** (no envelope, plaintext on disk). This keeps `rustpass`
//! free of any Android / Keystore dependency — it only ever sees a key as
//! bytes.
//!
//! ## Envelope
//!
//! `magic b"GPMATR1" | key_id: u8 | nonce: 12B | ciphertext ‖ tag`
//!
//! AES-GCM appends its 16-byte authentication tag to the ciphertext, so a
//! single AEAD operation delivers both confidentiality and integrity. The blob
//! name is bound as AAD, so a sealed blob cannot be swapped between slots
//! (e.g. an `identity` envelope replayed as `repo_config`).
//!
//! See `docs/SECURITY.md` for the threat-model scope: this defends a **read**
//! attacker and adds **integrity** to the two config files; it does **not**
//! defend a local **write** attacker (notably one who tampers the cloned
//! `repo/`).

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};

/// Envelope magic — distinguishes an at-rest blob from legacy plaintext.
const MAGIC: &[u8; 7] = b"GPMATR1";

/// Envelope key slot / version. Reserved for future key rotation; today only
/// `1` is ever produced or accepted.
const KEY_ID: u8 = 1;

/// GCM nonce length, in bytes (the standard 96-bit size).
const NONCE_LEN: usize = 12;

/// Length of the fixed header preceding the ciphertext: `magic | key_id`.
const HEADER_LEN: usize = MAGIC.len() + 1;

/// AES-256-GCM master-key length, in bytes.
pub const MASTER_KEY_LEN: usize = 32;

/// Generate a fresh random master key using the OS RNG.
///
/// The caller (app layer) seals this into the Android Keystore on Android;
/// on desktop there is no master key and at-rest encryption is a passthrough.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the OS RNG fails.
pub fn generate_master_key() -> Result<[u8; MASTER_KEY_LEN], Error> {
    let mut key = [0u8; MASTER_KEY_LEN];
    fill_random(&mut key)?;
    Ok(key)
}

/// Fill `out` with cryptographically-strong random bytes from the OS RNG.
fn fill_random(out: &mut [u8]) -> Result<(), Error> {
    getrandom::getrandom(out)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("OS RNG failed: {e}")))
}

/// Returns `true` if `raw` begins with the at-rest envelope magic — i.e. it is
/// a sealed blob rather than legacy plaintext.
#[must_use]
pub fn is_envelope(raw: &[u8]) -> bool {
    raw.starts_with(MAGIC)
}

/// At-rest AEAD wrapper around an optional master key.
///
/// Construct with [`AtRest::new`]; when the key is `None` (desktop / tests)
/// [`AtRest::seal`] and [`AtRest::unseal`] are passthroughs (no envelope), so
/// the rest of the codebase is uniform across platforms.
pub(crate) struct AtRest {
    /// The injected master key. `None` ⇒ passthrough (plaintext, no envelope).
    key: Option<Zeroizing<[u8; MASTER_KEY_LEN]>>,
}

impl std::fmt::Debug for AtRest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtRest")
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl AtRest {
    /// Create a new wrapper. `None` selects passthrough mode.
    pub(crate) fn new(master_key: Option<[u8; MASTER_KEY_LEN]>) -> Self {
        Self {
            key: master_key.map(Zeroizing::new),
        }
    }

    /// Seal `plaintext` for the named slot into an at-rest envelope.
    ///
    /// In passthrough mode (`None` key) the plaintext is returned unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::StoreError`] on AEAD failure (should not happen
    /// with a valid 32-byte key and a fresh nonce).
    pub(crate) fn seal(&self, name: &str, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let Some(key) = self.key.as_ref() else {
            // Passthrough: no envelope, plaintext as-is.
            return Ok(plaintext.to_vec());
        };

        let cipher = Aes256Gcm::new_from_slice(&**key).expect("master key is 32 bytes");

        let mut nonce_bytes = [0u8; NONCE_LEN];
        fill_random(&mut nonce_bytes)?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ct = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|e| Error::new(ErrorCode::StoreError, format!("AEAD encrypt failed: {e}")))?;

        let mut out = Vec::with_capacity(HEADER_LEN + NONCE_LEN + ct.len());
        out.extend_from_slice(MAGIC);
        out.push(KEY_ID);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Unseal an at-rest envelope (or pass through legacy plaintext).
    ///
    /// - No envelope magic ⇒ treated as legacy plaintext and returned as-is
    ///   (this is the migration / passthrough path).
    /// - Envelope with a key present ⇒ AEAD-open with `name` as AAD.
    /// - Envelope with **no** key present ⇒ [`ErrorCode::AtRestKeyUnavailable`]
    ///   (a sealed file exists but the Keystore key is gone → re-setup).
    /// - AEAD tag mismatch / truncated / wrong version ⇒
    ///   [`ErrorCode::AtRestTampered`].
    pub(crate) fn unseal(&self, name: &str, raw: &[u8]) -> Result<Vec<u8>, Error> {
        // Legacy plaintext (passthrough or pre-migration): return as-is.
        if !is_envelope(raw) {
            return Ok(raw.to_vec());
        }

        // It is an envelope — we must have the key to open it.
        let Some(key) = self.key.as_ref() else {
            return Err(Error::new(
                ErrorCode::AtRestKeyUnavailable,
                "At-rest data is encrypted but the master key is unavailable — re-setup required",
            ));
        };

        if raw.len() < HEADER_LEN + NONCE_LEN {
            return Err(Error::new(
                ErrorCode::AtRestTampered,
                "Truncated at-rest envelope",
            ));
        }

        // Envelope: magic(MAGIC.len()) | key_id(1) | nonce(NONCE_LEN) | ct‖tag.
        // The length check above guarantees every split is in bounds; split_at is
        // used instead of indexing so there is no panicking slice for clippy.
        let (_magic, rest) = raw.split_at(MAGIC.len());
        let (key_id_slice, rest) = rest.split_at(1);
        let (nonce_bytes, ct) = rest.split_at(NONCE_LEN);
        let key_id = key_id_slice
            .first()
            .copied()
            .expect("split_at(1) yields exactly one byte");

        if key_id != KEY_ID {
            return Err(Error::new(
                ErrorCode::AtRestTampered,
                format!("Unsupported at-rest key id: {key_id}"),
            ));
        }

        let nonce = Nonce::from_slice(nonce_bytes);

        let cipher = Aes256Gcm::new_from_slice(&**key).expect("master key is 32 bytes");
        cipher
            .decrypt(
                nonce,
                Payload {
                    msg: ct,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| {
                Error::new(
                    ErrorCode::AtRestTampered,
                    "At-rest data is tampered or corrupt",
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> AtRest {
        AtRest::new(Some(generate_master_key().unwrap()))
    }

    #[test]
    fn passthrough_when_no_key_roundtrips_plaintext() {
        let ar = AtRest::new(None);
        let pt = b"plain repo.json contents";
        let sealed = ar.seal("repo_config", pt).unwrap();
        assert_eq!(sealed, pt, "passthrough seal must be identity");
        assert!(!is_envelope(&sealed));
        let back = ar.unseal("repo_config", &sealed).unwrap();
        assert_eq!(back, pt);
    }

    #[test]
    fn enabled_seal_is_envelope_and_roundtrips() {
        let ar = enabled();
        let pt = r#"{"url":"https://x/repo","pat":"secret-token"}"#;
        let sealed = ar.seal("repo_config", pt.as_bytes()).unwrap();
        assert!(is_envelope(&sealed), "sealed blob must carry the magic");
        assert_ne!(&sealed[..], pt.as_bytes());
        assert!(sealed.len() > pt.len(), "envelope adds header+nonce+tag");

        let back = ar.unseal("repo_config", &sealed).unwrap();
        assert_eq!(back, pt.as_bytes());
    }

    #[test]
    fn unseal_legacy_plaintext_passes_through_even_when_enabled() {
        // An enabled AtRest must still read a pre-migration plaintext file.
        let ar = enabled();
        let pt = b"legacy plaintext identity";
        let back = ar.unseal("identity", pt).unwrap();
        assert_eq!(back, pt);
    }

    #[test]
    fn unseal_envelope_without_key_is_key_unavailable() {
        // Seal with a key, then try to open with no key → re-setup required.
        let sealed = enabled().seal("repo_config", b"secret").unwrap();
        let no_key = AtRest::new(None);
        let err = no_key.unseal("repo_config", &sealed).unwrap_err();
        assert_eq!(err.code, "AT_REST_KEY_UNAVAILABLE");
    }

    #[test]
    fn tampered_ciphertext_is_detected() {
        let ar = enabled();
        let sealed = ar.seal("repo_config", b"secret").unwrap();
        let mut tampered = sealed.clone();
        // Flip a byte deep in the ciphertext (past header + nonce).
        if let Some(b) = tampered.last_mut() {
            *b ^= 0xff;
        }
        let err = ar.unseal("repo_config", &tampered).unwrap_err();
        assert_eq!(err.code, "AT_REST_TAMPERED");
    }

    #[test]
    fn wrong_aad_slot_is_rejected() {
        // A blob sealed for "identity" must not open as "repo_config" (anti-swap).
        let ar = enabled();
        let sealed = ar.seal("identity", b"secret").unwrap();
        let err = ar.unseal("repo_config", &sealed).unwrap_err();
        assert_eq!(err.code, "AT_REST_TAMPERED");
    }

    #[test]
    fn truncated_envelope_is_tampered() {
        let ar = enabled();
        let mut sealed = ar.seal("repo_config", b"x").unwrap();
        sealed.truncate(HEADER_LEN + NONCE_LEN); // drop all ciphertext + tag
        let err = ar.unseal("repo_config", &sealed).unwrap_err();
        assert_eq!(err.code, "AT_REST_TAMPERED");
    }

    #[test]
    fn distinct_seals_have_distinct_nonces() {
        // Random nonces must not repeat across seals of the same plaintext.
        let ar = enabled();
        let a = ar.seal("repo_config", b"same").unwrap();
        let b = ar.seal("repo_config", b"same").unwrap();
        assert_ne!(a, b, "seals of identical plaintext must differ");
        // Both still decrypt cleanly.
        assert_eq!(ar.unseal("repo_config", &a).unwrap(), b"same");
        assert_eq!(ar.unseal("repo_config", &b).unwrap(), b"same");
    }

    #[test]
    fn seal_then_migrate_idempotent() {
        // Sealing already-sealed bytes is not something migrate does, but seal
        // of arbitrary plaintext must be stable enough to round-trip.
        let ar = enabled();
        let sealed = ar.seal("repo_config", b"x").unwrap();
        // Re-sealing the *plaintext* again yields a different envelope (new
        // nonce) but both unseal to the same value.
        let sealed2 = ar.seal("repo_config", b"x").unwrap();
        assert_ne!(sealed, sealed2);
        assert_eq!(ar.unseal("repo_config", &sealed).unwrap(), b"x");
        assert_eq!(ar.unseal("repo_config", &sealed2).unwrap(), b"x");
    }

    #[test]
    fn wrong_key_cannot_open_envelope() {
        let sealed = enabled().seal("repo_config", b"secret").unwrap();
        let other = AtRest::new(Some(generate_master_key().unwrap()));
        let err = other.unseal("repo_config", &sealed).unwrap_err();
        assert_eq!(err.code, "AT_REST_TAMPERED");
    }
}
