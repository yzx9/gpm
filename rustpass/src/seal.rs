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
//! persistence — it is *injected* ([`Seal::new`]) as plain bytes. In
//! production it comes (hardware-sealed) from the Android Keystore via the app
//! layer; on desktop and in tests it is `None`, which makes [`Seal`] a
//! **passthrough** (no envelope, plaintext on disk). This keeps `rustpass`
//! free of any Android / Keystore dependency — it only ever sees a key as
//! bytes.
//!
//! ## Envelope
//!
//! `magic b"GPMSEL1" | key_id: u8 | nonce: 12B | ciphertext ‖ tag`
//!
//! Pre-rename builds wrote `GPMATR1` as the magic; those envelopes are still
//! **read** (see [`LEGACY_MAGIC`]) and proactively re-wrapped as `GPMSEL1` by
//! `Config::migrate_seal`. Both magics are 7 bytes, so the read path's magic
//! strip is uniform.
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

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};
use crate::rng::fill_random;

/// Envelope magic written by this build — distinguishes a sealed blob from
/// plaintext. New seals always emit `GPMSEL1`.
const MAGIC: &[u8; 7] = b"GPMSEL1";

/// Pre-rename envelope magic. Still **read** so envelopes written by older
/// builds stay decryptable; `Config::migrate_seal` proactively re-wraps them as
/// `GPMSEL1`. Kept the same 7-byte length as [`MAGIC`] so `unseal`'s
/// `split_at(MAGIC.len())` strip is correct for either header.
// TODO: v1.0.x — canonical removal list (other TODO: v1.0.x markers point
// here): LEGACY_MAGIC, the `|| starts_with(LEGACY_MAGIC)` in is_envelope,
// is_legacy_envelope, the legacy re-wrap branch in Config::wrap_if_needed,
// the post-unlock run_seal_migrate_once + its call in applock::app_unlock +
// the AppState.seal_migrate_state field, and the legacy-only tests in
// rustpass/src/config.rs and src-tauri/src/tests/seal_migrate.rs.
// PRECONDITION: remove ONLY in a release that first force-converts every
// remaining GPMATR1 envelope — mobile users skip versions, so dropping
// dual-read on a calendar version alone bricks their data.
const LEGACY_MAGIC: &[u8; 7] = b"GPMATR1";

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
/// on desktop there is no master key and seal encryption is a passthrough.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the OS RNG fails.
pub fn generate_master_key() -> Result<[u8; MASTER_KEY_LEN], Error> {
    let mut key = [0u8; MASTER_KEY_LEN];
    fill_random(&mut key)?;
    Ok(key)
}

/// Returns `true` if `raw` begins with a known envelope magic (current or
/// legacy) — i.e. it is a sealed blob rather than plaintext.
#[must_use]
pub fn is_envelope(raw: &[u8]) -> bool {
    raw.starts_with(MAGIC) || raw.starts_with(LEGACY_MAGIC)
}

/// Returns `true` if `raw` is a pre-rename `GPMATR1` envelope awaiting re-wrap
/// to `GPMSEL1`. Used only by `Config::migrate_seal`.
// TODO: v1.0.x — remove with LEGACY_MAGIC.
#[must_use]
pub(crate) fn is_legacy_envelope(raw: &[u8]) -> bool {
    raw.starts_with(LEGACY_MAGIC)
}

/// At-rest AEAD wrapper around an optional master key.
///
/// Construct with [`Seal::new`]; when the key is `None` (desktop / tests)
/// [`Seal::seal`] and [`Seal::unseal`] are passthroughs (no envelope), so
/// the rest of the codebase is uniform across platforms. The key is stored
/// behind a [`RwLock`] so it can be replaced at runtime via [`Seal::set_key`]
/// — used by the app-launch biometric lock to inject the key after the unlock
/// prompt and to wipe it (back to `None`) when the process is backgrounded, so
/// a locked app cannot read the store even from a memory snapshot.
pub(crate) struct Seal {
    /// The injected master key. `None` ⇒ passthrough (plaintext, no envelope) on
    /// desktop/tests; also the transient "key wiped" state while the app-launch
    /// biometric lock is engaged — envelopes then fail `SealKeyUnavailable`
    /// until the key is re-injected after the unlock prompt.
    key: RwLock<Option<Zeroizing<[u8; MASTER_KEY_LEN]>>>,
    /// Monotonic: latched `true` the first time a key is injected, never reset.
    /// Lets [`Seal::seal`] tell desktop/test passthrough (a key was never set,
    /// so no envelope exists on disk) apart from the app-lock "key wiped
    /// mid-session" state, and refuse to silently write plaintext where an
    /// envelope is expected. Atomic because it is read on the seal hot path
    /// without taking the key's [`RwLock`]; it only ever flips `false → true`,
    /// so any read is conservative.
    ever_keyed: AtomicBool,
}

impl std::fmt::Debug for Seal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted = self
            .key
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|_| "<redacted>"));
        f.debug_struct("Seal")
            .field("key", &redacted)
            .field("ever_keyed", &self.ever_keyed.load(Ordering::Relaxed))
            .finish()
    }
}

impl Seal {
    /// Create a new wrapper. `None` selects passthrough mode.
    pub(crate) fn new(master_key: Option<[u8; MASTER_KEY_LEN]>) -> Self {
        Self {
            key: RwLock::new(master_key.map(Zeroizing::new)),
            ever_keyed: AtomicBool::new(master_key.is_some()),
        }
    }

    /// Replace the master key. `None` wipes it — passthrough on desktop/tests,
    /// and the "locked" state (envelopes then fail `SealKeyUnavailable`) for
    /// the app-launch biometric lock. Used to inject the key after the unlock
    /// prompt and to wipe it when the process is backgrounded. Latches
    /// `ever_keyed` on the first `Some`, so a later wipe cannot make [`seal`]
    /// forget that envelopes exist on disk.
    ///
    /// [`seal`]: Seal::seal
    pub(crate) fn set_key(&self, master_key: Option<[u8; MASTER_KEY_LEN]>) {
        if let Ok(mut guard) = self.key.write() {
            *guard = master_key.map(Zeroizing::new);
        }
        if master_key.is_some() {
            self.ever_keyed.store(true, Ordering::Release);
        }
    }

    /// Seal `plaintext` for the named slot into an seal envelope.
    ///
    /// In passthrough mode (`None` key, and no key was ever injected —
    /// desktop/tests) the plaintext is returned unchanged. If a key was injected
    /// once and is now `None` (app-lock wiped it), this refuses with
    /// [`ErrorCode::SealKeyUnavailable`] rather than silently downgrading an
    /// on-disk envelope to plaintext — the last line of defense behind the
    /// app-lock command gating.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::StoreError`] on AEAD failure (should not happen
    /// with a valid 32-byte key and a fresh nonce) or if the key lock is
    /// poisoned.
    pub(crate) fn seal(&self, name: &str, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let guard = self
            .key
            .read()
            .map_err(|_| Error::new(ErrorCode::StoreError, "seal key lock poisoned"))?;
        let Some(key) = guard.as_ref() else {
            if self.ever_keyed.load(Ordering::Acquire) {
                // A key was injected once but is wiped now (app-lock engaged) —
                // refuse to write plaintext where an envelope is expected.
                return Err(Error::new(
                    ErrorCode::SealKeyUnavailable,
                    "At-rest master key is wiped — unlock the app before writing",
                ));
            }
            // Desktop/test passthrough: no key was ever set, so no envelope
            // exists on disk; plaintext as-is.
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

    /// Unseal a seal envelope (or pass through legacy plaintext).
    ///
    /// - No envelope magic ⇒ treated as legacy plaintext and returned as-is
    ///   (this is the migration / passthrough path).
    /// - Envelope with a key present ⇒ AEAD-open with `name` as AAD.
    /// - Envelope with **no** key present ⇒ [`ErrorCode::SealKeyUnavailable`]
    ///   (a sealed file exists but the Keystore key is gone → re-setup).
    /// - AEAD tag mismatch / truncated / wrong version ⇒
    ///   [`ErrorCode::SealTampered`].
    pub(crate) fn unseal(&self, name: &str, raw: &[u8]) -> Result<Vec<u8>, Error> {
        // Legacy plaintext (passthrough or pre-migration): return as-is.
        if !is_envelope(raw) {
            return Ok(raw.to_vec());
        }

        // It is an envelope — we must have the key to open it.
        let guard = self
            .key
            .read()
            .map_err(|_| Error::new(ErrorCode::StoreError, "seal key lock poisoned"))?;
        let Some(key) = guard.as_ref() else {
            return Err(Error::new(
                ErrorCode::SealKeyUnavailable,
                "At-rest data is encrypted — unlock the app to read it.",
            ));
        };

        if raw.len() < HEADER_LEN + NONCE_LEN {
            return Err(Error::new(
                ErrorCode::SealTampered,
                "Truncated seal envelope",
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
                ErrorCode::SealTampered,
                format!("Unsupported seal key id: {key_id}"),
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
                    ErrorCode::SealTampered,
                    "At-rest data is tampered or corrupt",
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> Seal {
        Seal::new(Some(generate_master_key().unwrap()))
    }

    #[test]
    fn passthrough_when_no_key_roundtrips_plaintext() {
        let ar = Seal::new(None);
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
        // An enabled Seal must still read a pre-migration plaintext file.
        let ar = enabled();
        let pt = b"legacy plaintext identity";
        let back = ar.unseal("identity", pt).unwrap();
        assert_eq!(back, pt);
    }

    #[test]
    fn unseal_envelope_without_key_is_key_unavailable() {
        // Seal with a key, then try to open with no key → re-setup required.
        let sealed = enabled().seal("repo_config", b"secret").unwrap();
        let no_key = Seal::new(None);
        let err = no_key.unseal("repo_config", &sealed).unwrap_err();
        assert_eq!(err.code, "SEAL_KEY_UNAVAILABLE");
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
        assert_eq!(err.code, "SEAL_TAMPERED");
    }

    #[test]
    fn wrong_aad_slot_is_rejected() {
        // A blob sealed for "identity" must not open as "repo_config" (anti-swap).
        let ar = enabled();
        let sealed = ar.seal("identity", b"secret").unwrap();
        let err = ar.unseal("repo_config", &sealed).unwrap_err();
        assert_eq!(err.code, "SEAL_TAMPERED");
    }

    #[test]
    fn truncated_envelope_is_tampered() {
        let ar = enabled();
        let mut sealed = ar.seal("repo_config", b"x").unwrap();
        sealed.truncate(HEADER_LEN + NONCE_LEN); // drop all ciphertext + tag
        let err = ar.unseal("repo_config", &sealed).unwrap_err();
        assert_eq!(err.code, "SEAL_TAMPERED");
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
        let other = Seal::new(Some(generate_master_key().unwrap()));
        let err = other.unseal("repo_config", &sealed).unwrap_err();
        assert_eq!(err.code, "SEAL_TAMPERED");
    }

    #[test]
    fn set_key_inject_wipe_and_reinject_roundtrip() {
        // The app-launch biometric lock model: the store is built without the
        // key, the key is injected after the unlock prompt, wiped when the
        // process is backgrounded, then re-injected on the next unlock.
        let key = generate_master_key().unwrap();
        let ar = Seal::new(None);

        // No key ⇒ passthrough (no envelope), mirroring desktop / pre-unlock.
        assert!(!is_envelope(&ar.seal("repo_config", b"x").unwrap()));

        // Inject after unlock ⇒ seals real envelopes that round-trip.
        ar.set_key(Some(key));
        let sealed = ar.seal("repo_config", b"secret").unwrap();
        assert!(is_envelope(&sealed));
        assert_eq!(ar.unseal("repo_config", &sealed).unwrap(), b"secret");

        // Wipe on background ⇒ the store becomes unreadable (the lock state):
        // an existing envelope can no longer be opened, the signal the app-lock
        // overlay relies on.
        ar.set_key(None);
        let err = ar.unseal("repo_config", &sealed).unwrap_err();
        assert_eq!(err.code, "SEAL_KEY_UNAVAILABLE");

        // Re-inject the SAME key on the next unlock ⇒ envelopes open again.
        ar.set_key(Some(key));
        assert_eq!(ar.unseal("repo_config", &sealed).unwrap(), b"secret");
    }

    #[test]
    fn set_key_replaces_a_prior_key() {
        // Toggling the app lock migrates the master key between the auth-free
        // and biometric-gated Keystore stores; the same blob is re-sealed under
        // a different key, so replacing the in-memory key must open the new
        // envelopes and reject the old.
        let first = generate_master_key().unwrap();
        let second = generate_master_key().unwrap();
        let ar = Seal::new(Some(first));
        let sealed_first = ar.seal("identity", b"x").unwrap();

        ar.set_key(Some(second));
        let sealed_second = ar.seal("identity", b"x").unwrap();
        // Old envelope no longer opens under the new key.
        assert_eq!(
            ar.unseal("identity", &sealed_first).unwrap_err().code,
            "SEAL_TAMPERED"
        );
        // New envelope opens.
        assert_eq!(ar.unseal("identity", &sealed_second).unwrap(), b"x");
    }

    #[test]
    fn seal_refuses_passthrough_after_key_was_wiped() {
        // Defense-in-depth: once a key has been injected, wiping it (app-lock
        // background) must NOT let a stray write silently downgrade an envelope
        // to plaintext. seal is the last line behind the app-lock command gate.
        let ar = Seal::new(Some(generate_master_key().unwrap()));
        ar.set_key(None);
        let err = ar.seal("repo_config", b"x").unwrap_err();
        assert_eq!(err.code, "SEAL_KEY_UNAVAILABLE");
    }

    #[test]
    fn seal_passthrough_when_never_keyed() {
        // Desktop/tests never inject a key, so passthrough stays available —
        // the ever_keyed latch must not break the no-encryption platform path.
        let ar = Seal::new(None);
        assert!(!is_envelope(&ar.seal("repo_config", b"x").unwrap()));
    }

    #[test]
    fn seal_refuses_passthrough_after_key_swap_then_wipe() {
        // The app-lock migration path: swap the master key (enable/disable
        // moves it between Keystore stores — same blob, but set_key is called),
        // then wipe it on background. The ever_keyed latch must hold across the
        // swap, so seal still refuses to write plaintext where an envelope is
        // expected — the defense the migration relies on.
        let a = generate_master_key().unwrap();
        let b = generate_master_key().unwrap();
        let ar = Seal::new(Some(a));
        ar.set_key(Some(b)); // swap (migration)
        ar.set_key(None); // wipe (background)
        let err = ar.seal("repo_config", b"x").unwrap_err();
        assert_eq!(err.code, "SEAL_KEY_UNAVAILABLE");
    }
}
