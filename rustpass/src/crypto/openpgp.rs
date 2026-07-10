// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Low-level `OpenPGP` (rpgp) wrapper — the shared seam for both GPG
//! commit-signature verification (RFC 0009) and the future GPG crypto backend
//! (RFC 0036).
//!
//! Owns the `pgp` (rpgp 0.19) dependency so the rest of the crate never names
//! rpgp directly. Two groups of primitives live here:
//!
//! - **Verify** (live, called by the signing path): parse an armored public key,
//!   parse a detached signature, verify it over arbitrary bytes against a
//!   trusted key set, and report which trusted key (primary or subkey) verified.
//! - **Crypto** (`#[allow(dead_code)]`, awaiting the `GpgBackend` consumer):
//!   generate a keypair, encrypt to recipient subkeys, and decrypt with one key
//!   or a keyring. These are the load-bearing primitives the trait reshape will
//!   route through; proven against system `gpg` by the in-module interop tests.
//!
//! All functions are synchronous CPU work — callers (the sync signing path, or
//! a `spawn_blocking` from async Tauri commands / the future `GpgBackend`) own
//! the threading model. Callers running rpgp over attacker-controlled bytes
//! (commit payloads, ciphertext) should also wrap the call in `catch_unwind`
//! (see `signing::status_of_commit`): rpgp returns `Result` for parse/verify
//! failures, but a panic on a crafted packet would otherwise unwind through the
//! whole walk.

use std::fmt::Write;

use pgp::composed::{
    Deserializable, DetachedSignature, EncryptionCaps, KeyType, Message, MessageBuilder,
    SecretKeyParamsBuilder, SignedPublicKey, SignedSecretKey, SubkeyParamsBuilder,
};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::types::{Fingerprint, KeyDetails, KeyId, Password};
use rand::thread_rng;
use smallvec::smallvec;

use crate::error::{Error, ErrorCode};

/// The outcome of verifying a GPG detached signature against a trusted key set.
///
/// The signing layer maps this to [`crate::signing::CommitSigStatus`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GpgOutcome {
    /// Verified against a trusted primary key or one of its subkeys. Carries
    /// the PRIMARY key fingerprint (hex) — the trust anchor the user added —
    /// even when the signature was actually made by a subkey.
    Verified { primary_fp: String },
    /// The issuer was identified but is NOT in the trusted set, so NO
    /// cryptographic verification was performed (GPG signatures do not embed
    /// the public key, unlike SSH-sig). Carries the issuer fingerprint/key-id
    /// the signature claimed. Maps to `CommitSigStatus::UnverifiedSignature`,
    /// NOT `UntrustedKey` (which is SSH-only and crypto-verified).
    Unverified { issuer_fp: String },
    /// A trusted key matched the issuer but the cryptographic verify failed —
    /// the tampering / forgery signal. Maps to `CommitSigStatus::BadSignature`.
    BadSignature,
    /// Armor parsed but carried no usable issuer information, or the signature
    /// otherwise could not be classified. Fail-soft; maps to `Unknown`.
    Unknown,
}

/// Parse an armored `OpenPGP` public key and validate its self-signatures
/// (primary + subkey bindings).
///
/// The single parsing entry point used by both the trusted-key add flow and
/// per-pass trust-set construction. `verify_bindings` is what confirms a
/// signing subkey is genuinely bound to the primary (so a subkey match later
/// implies trust in the right primary).
///
/// # Errors
///
/// `SshKeyInvalid` (the crate's "key parse failed" bucket) if the armor is
/// unparseable or the self-signatures do not validate.
pub(crate) fn parse_armored_public_key(armored: &str) -> Result<SignedPublicKey, Error> {
    let (key, _headers) = SignedPublicKey::from_armor_single(armored.as_bytes()).map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Invalid GPG public key: {e}"),
        )
    })?;
    key.verify_bindings().map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("GPG public key self-signatures invalid: {e}"),
        )
    })?;
    Ok(key)
}

/// The primary key's fingerprint as canonical hex (rpgp's `Display`), e.g.
/// `7ABCD...`. This is the stable identity stored in `TrustedGpgKey::fingerprint`.
#[must_use]
pub(crate) fn primary_fingerprint(key: &SignedPublicKey) -> String {
    format!("{}", key.fingerprint())
}

/// Parse an armored detached PGP signature (the `gpgsig` content of a
/// GPG-signed git commit).
///
/// # Errors
///
/// `StoreError` if the armor is unparseable.
pub(crate) fn parse_detached_signature(armored: &str) -> Result<DetachedSignature, Error> {
    let (sig, _headers) = DetachedSignature::from_armor_single(armored.as_bytes())
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("Invalid GPG signature: {e}")))?;
    Ok(sig)
}

/// Verify `sig` over `signed_data` against the trusted key set, returning the
/// classification the signing layer maps to `CommitSigStatus`.
///
/// Algorithm:
/// 1. Collect the issuer fingerprints + key-ids the signature claims
///    (self-reported subpackets — not authenticated, but a forged issuer fp
///    still has to pass crypto `verify` against the matched trusted key, so a
///    forgery surfaces as `BadSignature`, not `Verified`).
/// 2. Walk each trusted key's primary + subkeys. Where a (sub)key's identity
///    matches a claimed issuer, run the one crypto `verify`. Success →
///    `Verified` (primary fp). Failure is remembered as "claimed but failed."
/// 3. If any trusted (sub)key claimed the issuer but none verified →
///    `BadSignature` (tampering / forgery).
/// 4. If no trusted key claimed the issuer → `Unverified` (issuer known, no
///    crypto performed), or `Unknown` when the signature carried no issuer
///    subpackets at all (very old `GnuPG`); the latter falls back to a blind
///    try-verify against the trusted set so a key-id-only sig can still verify.
pub(crate) fn verify_detached(
    sig: &DetachedSignature,
    signed_data: &[u8],
    trusted: &[ParsedGpgKey],
) -> GpgOutcome {
    let issuer_fps: Vec<&Fingerprint> = sig.signature.issuer_fingerprint();
    let issuer_kids: Vec<&KeyId> = sig.signature.issuer_key_id();
    let has_issuer = !issuer_fps.is_empty() || !issuer_kids.is_empty();

    let mut claimed_but_failed = false;

    for pk in trusted {
        let key = &pk.key;
        let key_fp = key.fingerprint();
        let key_kid = key.legacy_key_id();
        // Primary key.
        if identity_matches(&key_fp, key_kid, &issuer_fps, &issuer_kids) {
            claimed_but_failed = true;
            if sig.verify(key, signed_data).is_ok() {
                return GpgOutcome::Verified {
                    primary_fp: pk.fingerprint.clone(),
                };
            }
        }
        // Subkeys — a commit is usually signed by a signing subkey, not the
        // primary. Only a subkey whose binding grants signing usage may
        // authenticate a commit as its primary: a signature from an
        // encryption-only subkey (whose private half may be held by someone
        // who shouldn't sign) must NOT be accepted as the primary's. Skipped
        // here, so it neither verifies nor counts as a failed claim. Mirrors
        // rpgp's own `sig.key_flags().sign()` signing-capable convention.
        for sub in &key.public_subkeys {
            if !sub.signatures.iter().any(|s| s.key_flags().sign()) {
                continue;
            }
            let sub_fp = sub.fingerprint();
            let sub_kid = sub.legacy_key_id();
            if identity_matches(&sub_fp, sub_kid, &issuer_fps, &issuer_kids) {
                claimed_but_failed = true;
                if sig.verify(sub, signed_data).is_ok() {
                    return GpgOutcome::Verified {
                        primary_fp: pk.fingerprint.clone(),
                    };
                }
            }
        }
    }

    if claimed_but_failed {
        return GpgOutcome::BadSignature;
    }

    // No trusted key claimed the issuer. If the signature carried no issuer
    // subpackets at all, fall back to a blind try-verify so a very old
    // (key-id-only) GnuPG signature can still match a trusted key.
    if !has_issuer {
        for pk in trusted {
            let key = &pk.key;
            if sig.verify(key, signed_data).is_ok() {
                return GpgOutcome::Verified {
                    primary_fp: pk.fingerprint.clone(),
                };
            }
            for sub in &key.public_subkeys {
                // Same signing-capable gate as the issuer-matched loop above.
                if !sub.signatures.iter().any(|s| s.key_flags().sign()) {
                    continue;
                }
                if sig.verify(sub, signed_data).is_ok() {
                    return GpgOutcome::Verified {
                        primary_fp: pk.fingerprint.clone(),
                    };
                }
            }
        }
        return GpgOutcome::Unknown;
    }

    // Issuer known but untrusted — prefer the fingerprint, fall back to the
    // 8-byte key-id rendered as stable uppercase hex (matching GnuPG's long
    // key-id display). `KeyId: AsRef<[u8]>` gives the raw bytes; hex-encoding
    // them is stable across rpgp versions, unlike `Debug`.
    let issuer = issuer_fps.first().map(|fp| format!("{fp}")).or_else(|| {
        issuer_kids.first().map(|kid| {
            let mut hex = String::with_capacity(kid.as_ref().len() * 2);
            for b in kid.as_ref() {
                // Writing to a String never fails.
                let _ = write!(hex, "{b:02X}");
            }
            hex
        })
    });
    match issuer {
        Some(fp) => GpgOutcome::Unverified { issuer_fp: fp },
        None => GpgOutcome::Unknown,
    }
}

/// Does a trusted (sub)key's identity match any issuer identifier the signature
/// claimed? Match by fingerprint (modern) OR legacy 8-byte key-id (old `GnuPG`).
fn identity_matches(
    key_fp: &Fingerprint,
    key_kid: KeyId,
    issuer_fps: &[&Fingerprint],
    issuer_kids: &[&KeyId],
) -> bool {
    issuer_fps.contains(&key_fp) || issuer_kids.contains(&&key_kid)
}

/// A trusted GPG public key pre-parsed for a verification pass, with its
/// primary fingerprint cached so the verifier reports the trust anchor without
/// re-deriving it per commit.
#[derive(Debug, Clone)]
pub(crate) struct ParsedGpgKey {
    /// Primary-key fingerprint (hex) — the identity stored in
    /// `TrustedGpgKey::fingerprint`.
    pub fingerprint: String,
    /// The parsed armored key (primary + subkeys, self-sigs already validated).
    pub key: SignedPublicKey,
}

/// Parse a trusted-key set leniently: unparseable entries are dropped (and
/// returned as a warning message) rather than failing the whole pass, so one
/// bad paste can't brick verification. Use this once per verification pass.
///
/// Returns `(keys, warnings)` where `warnings` carries the parse-error strings
/// for each dropped entry — the command layer surfaces those in the Settings UI
/// (so a previously-Verified key that later fails to parse can't silently
/// downgrade commits to `UnverifiedSignature`).
pub(crate) fn parse_trusted_keys<'a, I>(armored: I) -> (Vec<ParsedGpgKey>, Vec<String>)
where
    I: IntoIterator<Item = &'a str>,
{
    let mut keys = Vec::new();
    let mut warnings = Vec::new();
    for a in armored {
        match parse_armored_public_key(a) {
            Ok(key) => {
                let fingerprint = primary_fingerprint(&key);
                keys.push(ParsedGpgKey { fingerprint, key });
            }
            Err(e) => warnings.push(format!("{e}")),
        }
    }
    (keys, warnings)
}

// ======================================================================
// Crypto primitives (keygen / encrypt / decrypt)
//
// The load-bearing layer the future `GpgBackend` (RFC 0036 trait reshape)
// routes through. They have NO production caller yet — `Store` still holds an
// `AgeBackend` — so each is `#[allow(dead_code)]` until `GpgBackend` lands.
// Mirrors the free-function shape of `crypto::age`: synchronous, returning
// `Result`; the backend layer wraps these in `spawn_blocking` + `Zeroizing` +
// `catch_unwind`.
// ======================================================================

/// Generate a V4 `OpenPGP` keypair (Ed25519 primary + Curve25519 ECDH subkey),
/// optionally passphrase-protected, returning `(secret, public)`.
///
/// When a `passphrase` is given, the primary key + every subkey are S2K-locked
/// with rpgp's default V4 KDF — **iterated+salted CFB, AES256, 224 rounds**
/// (`S2kParams::new_default`, `pgp/src/types/s2k.rs`): exactly what
/// `gpg`/`gopass` produce, and readable by every `gpg` version. Argon2 is
/// V6-only and is intentionally NOT chosen — the gopass-compatibility hard
/// constraint favors universal readability over a stronger KDF.
///
/// The passphrase is applied by re-locking AFTER `generate()`, NOT via the
/// builder's `.passphrase()`: that setting is applied mid-generate but the
/// subsequent self-signing step (`sign`) unlocks the key in place, so the
/// returned key — and its armored form — would be UNPROTECTED. The explicit
/// `set_password` below is load-bearing: a future `GpgBackend` stores the armored
/// secret as the at-rest identity, which MUST be S2K-locked. Verified by
/// `passphrase_keypair_wrong_passphrase_fails` (a wrong passphrase only fails to
/// decrypt once this re-lock runs).
///
/// The public key joins the recipient pool / trust set.
///
/// # Errors
///
/// `StoreError` if rpgp's param build, key generation, or S2K locking fails
/// (programmer error in practice — never user-driven input).
#[allow(dead_code)]
pub(crate) fn generate_keypair(
    user_id: &str,
    passphrase: Option<&str>,
) -> Result<(SignedSecretKey, SignedPublicKey), Error> {
    let mut rng = thread_rng();
    let subkey = SubkeyParamsBuilder::default()
        .key_type(KeyType::ECDH(ECCCurve::Curve25519))
        .can_encrypt(EncryptionCaps::All)
        .build()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("subkey params: {e}")))?;
    let mut params = SecretKeyParamsBuilder::default();
    params
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(false)
        .can_sign(true)
        .primary_user_id(user_id.to_string())
        // `.passphrase()` is intentionally NOT set on the builder — rpgp applies
        // it mid-generate then the self-sign step unlocks in place, leaving an
        // unprotected key. We lock explicitly below instead.
        .preferred_symmetric_algorithms(smallvec![SymmetricKeyAlgorithm::AES256])
        .preferred_hash_algorithms(smallvec![HashAlgorithm::Sha256])
        .preferred_compression_algorithms(smallvec![])
        .subkeys(vec![subkey]);
    let mut sk = params
        .build()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("key params: {e}")))?
        .generate(&mut rng)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("key generation: {e}")))?;
    // Lock the primary + every subkey so the armored identity (what a future
    // `GpgBackend` stores at rest) is genuinely passphrase-protected. `set_password`
    // uses the default V4 iterated+salted S2K and requires the key to be unlocked
    // (Plain), which it is here post-`generate()`.
    if let Some(pw) = passphrase {
        let password: Password = pw.into();
        sk.primary_key
            .set_password(&mut rng, &password)
            .map_err(|e| Error::new(ErrorCode::StoreError, format!("lock primary key: {e}")))?;
        for sub in &mut sk.secret_subkeys {
            sub.key
                .set_password(&mut rng, &password)
                .map_err(|e| Error::new(ErrorCode::StoreError, format!("lock subkey: {e}")))?;
        }
    }
    let pk = sk.to_public_key();
    Ok((sk, pk))
}

/// Encrypt `plaintext` to every recipient's first subkey, returning binary
/// (unarmored) `OpenPGP` — the on-disk gopass `<name>.gpg` format.
///
/// `seipd_v1` with no compression call produces uncompressed SEIPD v1 (MDC),
/// matching gopass's `--compress-algo=none` output; one PKESK is emitted per
/// recipient. The first subkey is used as-is — gopass/gpg place the encryption
/// subkey first, so that holds for gopass stores; a future `GpgBackend` must
/// pre-select the encryption-capable subkey for keys with a different layout
/// (e.g. a signing subkey first), since this primitive does not validate key
/// flags.
///
/// # Errors
///
/// `InvalidIdentity` if `recipients` is empty or a recipient has no subkey;
/// `StoreError` if rpgp fails to encrypt or serialize.
#[allow(dead_code)]
pub(crate) fn encrypt_to_recipients(
    plaintext: &[u8],
    recipients: &[&SignedPublicKey],
) -> Result<Vec<u8>, Error> {
    // Fail fast: an empty recipient set would otherwise serialize a message with
    // no PKESK — unrecoverable ciphertext. The future `GpgBackend` also guards
    // this, but the primitive must not hand back data-loss bytes silently.
    if recipients.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidIdentity,
            "encrypt_to_recipients requires at least one recipient public key",
        ));
    }
    let mut rng = thread_rng();
    let mut builder = MessageBuilder::from_bytes("", plaintext.to_vec())
        .seipd_v1(&mut rng, SymmetricKeyAlgorithm::AES256);
    for r in recipients {
        let enc_subkey = r.public_subkeys.first().ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidIdentity,
                "recipient GPG key has no encryption subkey",
            )
        })?;
        builder
            .encrypt_to_key(&mut rng, enc_subkey)
            .map_err(|e| Error::new(ErrorCode::StoreError, format!("encrypt to subkey: {e}")))?;
    }
    builder
        .to_vec(&mut rng)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("serialize message: {e}")))
}

/// Encrypt `plaintext` to each recipient's encryption-capable subkey, returning
/// binary `OpenPGP` (the on-disk gopass `<name>.gpg` format). Unlike
/// [`encrypt_to_recipients`] (which takes the first subkey blindly), this selects
/// the subkey whose binding signature grants encryption — gpg's own rule, which
/// gopass delegates to (`gpg --encrypt` picks the encryption subkey by key flags).
/// Correct for imported keys whose first subkey is signing-only.
///
/// SEIPD v1 / AES256 / no compression, one PKESK per recipient — same wire shape
/// as [`encrypt_to_recipients`] and gopass's `--compress-algo=none` output.
///
/// # Errors
///
/// `InvalidIdentity` if `recipients` is empty, or a recipient has subkeys but
/// none encryption-capable (a "bad recipient" — surfaced so the caller doesn't
/// silently drop it, matching gopass's `badRecipients`); `StoreError` if rpgp
/// fails to encrypt or serialize.
pub(crate) fn encrypt_to_selected_subkeys(
    plaintext: &[u8],
    recipients: &[&SignedPublicKey],
) -> Result<Vec<u8>, Error> {
    if recipients.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidIdentity,
            "encrypt_to_selected_subkeys requires at least one recipient public key",
        ));
    }
    let mut rng = thread_rng();
    let mut builder = MessageBuilder::from_bytes("", plaintext.to_vec())
        .seipd_v1(&mut rng, SymmetricKeyAlgorithm::AES256);
    for r in recipients {
        // gpg's selection: the first subkey whose binding grants encryption
        // storage/comms. gopass/gpg keys always carry key-flags subpackets, so a
        // recipient with subkeys but no encryption-capable one is a bad recipient.
        let enc_subkey = r
            .public_subkeys
            .iter()
            .find(|sub| {
                sub.signatures
                    .iter()
                    .any(|s| s.key_flags().encrypt_storage() || s.key_flags().encrypt_comms())
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidIdentity,
                    "recipient GPG key has no encryption-capable subkey (bad recipient)",
                )
            })?;
        builder
            .encrypt_to_key(&mut rng, enc_subkey)
            .map_err(|e| Error::new(ErrorCode::StoreError, format!("encrypt to subkey: {e}")))?;
    }
    builder
        .to_vec(&mut rng)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("serialize message: {e}")))
}

/// Decrypt a single-recipient `OpenPGP` message with one secret key + passphrase,
/// returning the literal plaintext bytes. `passphrase` is `Password::empty()` for
/// an unprotected key. The single-key special case of
/// [`decrypt_message_with_keys`].
///
/// # Errors
///
/// `DecryptFailed` if the ciphertext is unparseable, the key/passphrase do not
/// match, or the data is corrupt.
pub(crate) fn decrypt_message(
    ciphertext: &[u8],
    passphrase: &Password,
    sk: &SignedSecretKey,
) -> Result<Vec<u8>, Error> {
    let mut decrypted = Message::from_bytes(ciphertext)
        .map_err(|e| Error::new(ErrorCode::DecryptFailed, format!("parse message: {e}")))?
        .decrypt(passphrase, sk)
        .map_err(|e| Error::new(ErrorCode::DecryptFailed, format!("decrypt: {e}")))?;
    decrypted
        .as_data_vec()
        .map_err(|e| Error::new(ErrorCode::DecryptFailed, format!("extract literal: {e}")))
}

/// Decrypt a multi-recipient `OpenPGP` message against a keyring — rpgp's
/// `Message::decrypt_with_keys` tries each (key, passphrase) pair internally.
/// This is the minimal keyring shape the future `GpgBackend` routes through.
///
/// Pass the full keyring and the full passphrase set: rpgp attempts every
/// key×password combination, so a passphrase-protected key needs its passphrase
/// present (an unprotected key pairs with `Password::empty()`).
///
/// # Errors
///
/// `DecryptFailed` if no (key, passphrase) combination decrypts the message, or
/// the ciphertext is unparseable.
#[allow(dead_code)]
pub(crate) fn decrypt_message_with_keys(
    ciphertext: &[u8],
    keys: &[&SignedSecretKey],
    passphrases: &[&Password],
) -> Result<Vec<u8>, Error> {
    let mut decrypted = Message::from_bytes(ciphertext)
        .map_err(|e| Error::new(ErrorCode::DecryptFailed, format!("parse message: {e}")))?
        .decrypt_with_keys(passphrases.to_vec(), keys.to_vec())
        .map_err(|e| Error::new(ErrorCode::DecryptFailed, format!("decrypt: {e}")))?;
    decrypted
        .as_data_vec()
        .map_err(|e| Error::new(ErrorCode::DecryptFailed, format!("extract literal: {e}")))
}

/// Strip the S2K passphrase layer from a secret key (primary + every subkey) in
/// place — the inverse of the `set_password` step in [`generate_keypair`]. A
/// `Plain` (unprotected) key is a no-op. This is the load-bearing op for the
/// GPG backend's `unlock_identity`/`validate_identity_passphrase`: rpgp's
/// `remove_password` consumes `Password` here so callers need not name the type.
///
/// # Errors
///
/// `WrongPassphrase` if `password` does not satisfy the S2K checksum on an
/// encrypted key (the only failure mode for an otherwise-parseable key).
pub(crate) fn strip_passphrase(sk: &mut SignedSecretKey, password: &str) -> Result<(), Error> {
    let pw: Password = password.into();
    sk.primary_key.remove_password(&pw).map_err(|e| {
        Error::new(
            ErrorCode::WrongPassphrase,
            format!("unlock primary key: {e}"),
        )
    })?;
    for sub in &mut sk.secret_subkeys {
        sub.key
            .remove_password(&pw)
            .map_err(|e| Error::new(ErrorCode::WrongPassphrase, format!("unlock subkey: {e}")))?;
    }
    Ok(())
}

/// Decrypt a single-recipient message with an already-unlocked (`Plain`) secret
/// key — `Password::empty()` because [`strip_passphrase`] already removed the S2K
/// layer. This is the GPG backend's `decrypt` path over the operational
/// (unlocked-armor) identity bytes.
///
/// # Errors
///
/// `DecryptFailed` if the ciphertext is unparseable, the key does not match, or
/// the data is corrupt.
pub(crate) fn decrypt_with_unlocked_key(
    ciphertext: &[u8],
    sk: &SignedSecretKey,
) -> Result<Vec<u8>, Error> {
    decrypt_message(ciphertext, &Password::empty(), sk)
}

/// True iff the secret key (primary or any subkey) is S2K-encrypted at rest —
/// i.e. unlocking needs a passphrase. Used by the GPG backend's
/// `identity_requires_passphrase`.
#[must_use]
pub(crate) fn secret_key_is_encrypted(sk: &SignedSecretKey) -> bool {
    sk.primary_key.secret_params().is_encrypted()
        || sk
            .secret_subkeys
            .iter()
            .any(|sub| sub.key.secret_params().is_encrypted())
}

/// Armor a secret key for at-rest storage (the gopass identity format).
/// Round-trips with [`parse_armored_secret_key`].
///
/// # Errors
///
/// `StoreError` if rpgp fails to serialize.
#[allow(clippy::default_trait_access)]
pub(crate) fn armor_secret_key(sk: &SignedSecretKey) -> Result<String, Error> {
    // `ArmorOptions` isn't re-exported by pgp 0.19 (`mod message` is private), so
    // `Default::default()` is the only external way to construct it — matches
    // pgp's own internal `to_armored_string(Default::default())` usage.
    sk.to_armored_string(Default::default())
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("armor secret key: {e}")))
}

/// Armor a public key — the gopass `.public-keys/<id>` blob format. Round-trips
/// with [`parse_armored_public_key`].
///
/// # Errors
///
/// `StoreError` if rpgp fails to serialize.
#[allow(dead_code, clippy::default_trait_access)]
pub(crate) fn armor_public_key(pk: &SignedPublicKey) -> Result<String, Error> {
    pk.to_armored_string(Default::default())
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("armor public key: {e}")))
}

/// Parse an armored secret key produced by [`armor_secret_key`] or by
/// `gpg --armor --export-secret-key`. Self-signatures are NOT re-validated here
/// — the key is trusted as our own identity, not as an external signer. A future
/// `GpgBackend` import path that accepts attacker-controlled armor (paste-in /
/// sync-received identity) MUST call `verify_bindings` separately before
/// trusting the key, mirroring [`parse_armored_public_key`].
///
/// # Errors
///
/// `InvalidIdentity` if the armor is unparseable.
pub(crate) fn parse_armored_secret_key(armored: &[u8]) -> Result<SignedSecretKey, Error> {
    let (sk, _headers) = SignedSecretKey::from_armor_single(armored).map_err(|e| {
        Error::new(
            ErrorCode::InvalidIdentity,
            format!("invalid GPG secret key: {e}"),
        )
    })?;
    Ok(sk)
}

#[cfg(test)]
mod tests {
    //! Real-world interop proofs for both rpgp touchpoints.
    //!
    //! - **Verify** (RFC 0009): committed `gpg`-produced Ed25519 key + detached
    //!   signature prove rpgp verifies the keys/sigs users actually have.
    //! - **Crypto** (RFC 0036 spike): rpgp↔gpg encrypt/decrypt interop (RSA-2048
    //!   fixture + rpgp-generated Curve25519), passphrase-keygen round-trips,
    //!   and the multi-recipient keyring shape. No `gpg` binary is needed except
    //!   the `#[ignore]` reverse-interop tests (they spawn `gpg` + `gpg-agent`).

    use std::io::Write as _;
    use std::process::{Command, Stdio};

    use pgp::composed::Esk;

    use super::*;

    // --- verify fixtures (RFC 0009): committed gpg-produced Ed25519 key/sig ---
    const GPG_PUBKEY: &str = include_str!("../../tests/fixtures/gpg/pubkey.asc");
    const GPG_SIG: &str = include_str!("../../tests/fixtures/gpg/payload.sig.asc");
    const GPG_PAYLOAD: &[u8] = include_bytes!("../../tests/fixtures/gpg/payload.txt");

    // --- crypto fixtures (RFC 0036): committed gpg 2.4 RSA-2048 keypair +
    //     ciphertext (--compress-algo=none, passphrase-protected). TEST-ONLY. ---
    const FIXTURE_SECRET: &[u8] = include_bytes!("../../tests/fixtures/gpg/secret.asc");
    const FIXTURE_PUBLIC: &[u8] = include_bytes!("../../tests/fixtures/gpg/public.asc");
    const FIXTURE_GPG_ENCRYPTED: &[u8] =
        include_bytes!("../../tests/fixtures/gpg/gpg-encrypted.gpg");
    const FIXTURE_PASSPHRASE: &str = "test-passphrase-fixture-only";
    const EXPECTED_PLAINTEXT: &[u8] = b"gpg-to-rpgp interop plaintext";

    const SPIKE_UID: &str = "gpm spike <spike@gpm.local>";
    const SPIKE_PASSPHRASE: &str = "spike-passphrase";

    // ===================== verify tests (RFC 0009) =====================

    /// A `GnuPG`-produced Ed25519 armored pubkey parses and its self-signatures
    /// validate — the floor-level proof that `add_trusted_gpg_key` accepts a
    /// real key instead of rejecting it with `SshKeyInvalid`.
    #[test]
    fn real_gnupg_pubkey_parses() {
        let key = parse_armored_public_key(GPG_PUBKEY).expect("real GnuPG pubkey must parse");
        // The primary fingerprint is the stable identity stored in
        // TrustedGpgKey; it must be non-empty hex.
        let fp = primary_fingerprint(&key);
        assert!(!fp.is_empty(), "primary fingerprint must be non-empty");
    }

    /// A real `gpg --detach-sign --armor` signature — made by the signing
    /// SUBKEY (`GnuPG`'s default) — verifies against the trusted primary via
    /// rpgp. This is the load-bearing interop case: subkey signs, user trusts
    /// the primary, the binding must hold and the crypto must pass.
    #[test]
    fn real_gnupg_subkey_signature_verifies() {
        let key = parse_armored_public_key(GPG_PUBKEY).expect("pubkey parses");
        let sig = parse_detached_signature(GPG_SIG).expect("real GnuPG detached sig must parse");
        let trusted = vec![ParsedGpgKey {
            fingerprint: primary_fingerprint(&key),
            key,
        }];
        match verify_detached(&sig, GPG_PAYLOAD, &trusted) {
            GpgOutcome::Verified { primary_fp } => {
                assert!(
                    !primary_fp.is_empty(),
                    "verified outcome must carry the primary fingerprint"
                );
            }
            other => panic!("real GnuPG subkey signature must verify, got {other:?}"),
        }
    }

    /// Tampering the signed payload flips the outcome to `BadSignature` — the
    /// tampering catch holds on real `GnuPG` signatures, not only rpgp ones.
    #[test]
    fn real_gnupg_signature_tampered_is_bad() {
        let key = parse_armored_public_key(GPG_PUBKEY).expect("pubkey parses");
        let sig = parse_detached_signature(GPG_SIG).expect("sig parses");
        let trusted = vec![ParsedGpgKey {
            fingerprint: primary_fingerprint(&key),
            key,
        }];
        // Flip one byte of a non-empty payload (clippy-safe vs indexing).
        assert!(!GPG_PAYLOAD.is_empty());
        let mut tampered = GPG_PAYLOAD.to_vec();
        if let Some(b) = tampered.first_mut() {
            *b ^= 0xff;
        }
        let outcome = verify_detached(&sig, &tampered, &trusted);
        assert!(
            matches!(outcome, GpgOutcome::BadSignature),
            "tampered real GnuPG signature must be BadSignature, got {outcome:?}"
        );
    }

    /// A real `GnuPG` signature by a key NOT in the trust set surfaces as
    /// `Unverified` (issuer known via the signature's issuer subpacket, but no
    /// crypto performed) — the `UnverifiedSignature` status source.
    #[test]
    fn real_gnupg_signature_untrusted_is_unverified() {
        let sig = parse_detached_signature(GPG_SIG).expect("sig parses");
        let outcome = verify_detached(&sig, GPG_PAYLOAD, &[]);
        match outcome {
            GpgOutcome::Unverified { issuer_fp } => assert!(
                !issuer_fp.is_empty(),
                "issuer fingerprint must be surfaced for an untrusted GPG signature"
            ),
            other => panic!("untrusted real GnuPG sig must be Unverified, got {other:?}"),
        }
    }

    // ===================== crypto tests (RFC 0036) =====================

    #[test]
    fn self_roundtrip_no_passphrase() {
        const DATA: &[u8] = b"hello gopass from rpgp";
        let (sk, pk) = generate_keypair(SPIKE_UID, None).expect("keygen");
        let encrypted = encrypt_to_recipients(DATA, &[&pk]).expect("encrypt");
        let decrypted = decrypt_message(&encrypted, &Password::empty(), &sk).expect("decrypt");
        assert_eq!(decrypted, DATA);
    }

    #[test]
    fn multi_recipient_each_can_decrypt() {
        const DATA: &[u8] = b"shared secret for three recipients";
        let (sk_a, pk_a) = generate_keypair(SPIKE_UID, None).expect("keygen a");
        let (sk_b, pk_b) = generate_keypair(SPIKE_UID, None).expect("keygen b");
        let (sk_c, pk_c) = generate_keypair(SPIKE_UID, None).expect("keygen c");

        // Encrypt once to all three; each subkey gets its own PKESK.
        let encrypted = encrypt_to_recipients(DATA, &[&pk_a, &pk_b, &pk_c]).expect("encrypt");

        for (i, sk) in [sk_a, sk_b, sk_c].iter().enumerate() {
            let decrypted = decrypt_message(&encrypted, &Password::empty(), sk)
                .unwrap_or_else(|e| panic!("recipient {i} failed: {e:?}"));
            assert_eq!(decrypted, DATA);
        }
    }

    /// Packet-shape contract: one PKESK per recipient, no compression. SEIPD v1
    /// is asserted transitively by the interop tests (gpg produces v1; if rpgp
    /// emitted v2, `gpg_*_decrypts` would fail).
    #[test]
    fn packet_shape_uncompressed_one_pkesk_per_recipient() {
        const DATA: &[u8] = b"shape check";
        let (sk_a, pk_a) = generate_keypair(SPIKE_UID, None).expect("keygen a");
        let (_sk_b, pk_b) = generate_keypair(SPIKE_UID, None).expect("keygen b");

        let encrypted = encrypt_to_recipients(DATA, &[&pk_a, &pk_b]).expect("encrypt");

        // One PKESK per recipient (2 recipients -> 2 ESK entries).
        let msg = Message::from_bytes(&encrypted[..]).expect("parse");
        let Message::Encrypted { esk, .. } = msg else {
            panic!("expected encrypted message");
        };
        let pkesk_count = esk
            .iter()
            .filter(|e| matches!(e, Esk::PublicKeyEncryptedSessionKey(_)))
            .count();
        assert_eq!(pkesk_count, 2, "one PKESK per recipient");

        // No compressed-data packet: the decrypted payload is a Literal, not
        // Compressed (no `.compression()` was called on the builder).
        let msg = Message::from_bytes(&encrypted[..]).expect("parse again");
        let decrypted = msg
            .decrypt(&Password::empty(), &sk_a)
            .expect("decrypt with the matching key");
        assert!(
            !decrypted.is_compressed(),
            "gopass emits compress-algo=none; rpgp output must not be compressed"
        );
    }

    /// THE forward-interop test: rpgp decrypts a ciphertext + unlocks a
    /// passphrase key, both produced by system gpg (RSA-2048, gopass's default).
    /// Proves gpm can read existing gopass stores. Runs without a runtime gpg
    /// (the fixture is committed).
    #[test]
    fn gpg_encrypts_rpgp_decrypts() {
        let sk = parse_armored_secret_key(FIXTURE_SECRET).expect("parse gpg armored secret key");
        let pw: Password = FIXTURE_PASSPHRASE.into();
        let decrypted = decrypt_message(FIXTURE_GPG_ENCRYPTED, &pw, &sk)
            .expect("rpgp decrypts gpg-produced ciphertext with the passphrase");
        assert_eq!(decrypted, EXPECTED_PLAINTEXT);
    }

    #[test]
    fn passphrase_keypair_decrypts_with_right_passphrase() {
        const DATA: &[u8] = b"passphrase-protected round trip";
        let passphrase = SPIKE_PASSPHRASE;
        let pw: Password = passphrase.into();
        let (sk, pk) = generate_keypair(SPIKE_UID, Some(passphrase)).expect("keygen");
        let encrypted = encrypt_to_recipients(DATA, &[&pk]).expect("encrypt");
        let decrypted = decrypt_message(&encrypted, &pw, &sk).expect("right passphrase decrypts");
        assert_eq!(decrypted, DATA);
    }

    #[test]
    fn passphrase_keypair_wrong_passphrase_fails() {
        const DATA: &[u8] = b"passphrase-protected round trip";
        let passphrase = SPIKE_PASSPHRASE;
        let (sk, pk) = generate_keypair(SPIKE_UID, Some(passphrase)).expect("keygen");
        let encrypted = encrypt_to_recipients(DATA, &[&pk]).expect("encrypt");
        // A freshly generated key stays unlocked in memory (the passphrase only
        // gates serialization), so round-trip through armor to get an S2K-locked
        // key — the form that must reject a wrong passphrase, and the form a
        // future `GpgBackend` stores at rest.
        let armored = armor_secret_key(&sk).expect("armor");
        let locked = parse_armored_secret_key(armored.as_bytes()).expect("parse armored");
        let wrong: Password = "wrong".into();
        let err = decrypt_message(&encrypted, &wrong, &locked)
            .expect_err("wrong passphrase must fail on a locked key");
        // Pin the failure mode the test exists to prove: S2K-rejection surfaces
        // as DecryptFailed, not a different bucket (e.g. InvalidIdentity).
        assert_eq!(
            err.code, "DECRYPT_FAILED",
            "wrong passphrase must surface as DecryptFailed"
        );
    }

    #[test]
    fn armored_secret_key_roundtrips_and_decrypts() {
        const DATA: &[u8] = b"armor round-trip";
        let passphrase = SPIKE_PASSPHRASE;
        let pw: Password = passphrase.into();
        let (sk, pk) = generate_keypair(SPIKE_UID, Some(passphrase)).expect("keygen");
        let encrypted = encrypt_to_recipients(DATA, &[&pk]).expect("encrypt");

        // Serialize → parse the secret key; the parsed key must still decrypt.
        let armored = armor_secret_key(&sk).expect("armor");
        let parsed = parse_armored_secret_key(armored.as_bytes()).expect("parse armored");
        let decrypted = decrypt_message(&encrypted, &pw, &parsed).expect("parsed key decrypts");
        assert_eq!(decrypted, DATA);
    }

    /// Gate-zero for the MIN unlock design. Proves the exact chain a future
    /// `GpgBackend::unlock_identity` + `decrypt` depends on:
    ///   at-rest S2K-locked armor
    ///     -> `remove_password` (primary + every subkey)
    ///     -> `armor_secret_key`   (the operational bytes cached in
    ///                              `Store::cached_identity`)
    ///     -> `parse_armored_secret_key` (re-parse on the decrypt path)
    ///     -> `decrypt_message_with_keys(.., Password::empty())`
    /// Every other spike test decrypts *with* the passphrase; this one proves
    /// the unlocked-then-emptied path that the bytes-through-cached_identity
    /// model relies on. Round-trip bar only — S2K salts make byte-equality
    /// meaningless (see the `scrypt-at-rest-not-byte-identical` learning).
    #[test]
    fn remove_password_unlock_chain_round_trips() {
        const DATA: &[u8] = b"unlocked-armor chain";
        let passphrase = SPIKE_PASSPHRASE;
        let pw: Password = passphrase.into();
        let (sk, pk) = generate_keypair(SPIKE_UID, Some(passphrase)).expect("keygen");
        let encrypted = encrypt_to_recipients(DATA, &[&pk]).expect("encrypt");

        // at-rest form: S2K-locked armor (what gets AEAD-sealed on disk).
        let at_rest = armor_secret_key(&sk).expect("armor at-rest");
        let mut locked = parse_armored_secret_key(at_rest.as_bytes()).expect("parse at-rest");

        // unlock_identity: strip the S2K layer from the primary + every subkey.
        locked
            .primary_key
            .remove_password(&pw)
            .expect("remove_password primary");
        for sub in &mut locked.secret_subkeys {
            sub.key
                .remove_password(&pw)
                .expect("remove_password subkey");
        }

        // The operational bytes cached in `Store::cached_identity`: the unlocked
        // key re-armored (no passphrase layer left).
        let unlocked_armor = armor_secret_key(&locked).expect("re-armor unlocked");
        let operational =
            parse_armored_secret_key(unlocked_armor.as_bytes()).expect("re-parse unlocked armor");

        // decrypt: the operational (unlocked) key + Password::empty().
        let decrypted =
            decrypt_message_with_keys(&encrypted, &[&operational], &[&Password::empty()])
                .expect("unlocked armor decrypts with empty passphrase");
        assert_eq!(decrypted, DATA);
    }

    #[test]
    fn decrypt_message_with_keys_two_recipients() {
        const DATA: &[u8] = b"two-recipient keyring decrypt";
        // Two recipients: an rpgp-generated key (no passphrase) + the gpg
        // fixture key (passphrase-protected). A message encrypted to both
        // decrypts via the keyring helper with either secret key.
        let (sk_rpgp, pk_rpgp) = generate_keypair(SPIKE_UID, None).expect("rpgp keygen");
        let (pk_fixture, _headers) =
            SignedPublicKey::from_armor_single(FIXTURE_PUBLIC).expect("parse fixture public key");
        let sk_fixture = parse_armored_secret_key(FIXTURE_SECRET).expect("parse fixture secret");
        let fixture_pw: Password = FIXTURE_PASSPHRASE.into();

        let encrypted = encrypt_to_recipients(DATA, &[&pk_rpgp, &pk_fixture]).expect("encrypt");

        let empty = Password::empty();
        let dec_rpgp = decrypt_message_with_keys(&encrypted, &[&sk_rpgp], &[&empty])
            .expect("rpgp key decrypts via keyring");
        assert_eq!(dec_rpgp, DATA);

        let dec_fixture = decrypt_message_with_keys(&encrypted, &[&sk_fixture], &[&fixture_pw])
            .expect("fixture key decrypts via keyring");
        assert_eq!(dec_fixture, DATA);
    }

    // ===================== error-path tests (RFC 0036) =====================

    #[test]
    fn encrypt_to_recipients_rejects_empty_recipient_set() {
        // Fail-fast: an empty recipient set must NOT serialize a no-PKESK
        // message (unrecoverable ciphertext). InvalidIdentity, not a panic.
        let err = encrypt_to_recipients(b"x", &[]).expect_err("empty recipients");
        assert_eq!(err.code, "INVALID_IDENTITY");
    }

    #[test]
    fn decrypt_message_rejects_non_openpgp_bytes() {
        // Garbage ciphertext fails at the parse step, surfacing as DecryptFailed
        // rather than panicking or returning empty bytes.
        let (sk, _pk) = generate_keypair(SPIKE_UID, None).expect("keygen");
        let err = decrypt_message(b"not an openpgp message", &Password::empty(), &sk)
            .expect_err("garbage ciphertext must error");
        assert_eq!(err.code, "DECRYPT_FAILED");
    }

    #[test]
    fn parse_armored_secret_key_rejects_malformed_armor() {
        // A truncated/invalid private-key block is the corrupt-identity-file path
        // a user would actually hit; it must error, not accept garbage.
        let malformed =
            b"-----BEGIN PGP PRIVATE KEY BLOCK-----\nnot a real key\n-----END PGP PRIVATE KEY BLOCK-----\n";
        assert!(parse_armored_secret_key(malformed).is_err());
    }

    /// Reverse interop: rpgp encrypts to the gpg fixture's public key and system
    /// `gpg --decrypt` reads it with the matching secret key + passphrase. Proves
    /// rpgp's ciphertext output is desktop-gopass-readable.
    ///
    /// `#[ignore]`: spawns `gpg`, which needs `gpg-agent` (an `AF_UNIX` socket the
    /// sandbox blocks). Run with sandbox disabled:
    ///   `cargo test -p rustpass crypto::openpgp::tests::rpgp_encrypts_gpg_decrypts -- --ignored`
    #[test]
    #[ignore = "needs system gpg + sandbox disabled (gpg-agent socket)"]
    fn rpgp_encrypts_gpg_decrypts() {
        let (pk, _headers) = SignedPublicKey::from_armor_single(FIXTURE_PUBLIC)
            .expect("parse gpg armored public key");
        let ciphertext =
            encrypt_to_recipients(EXPECTED_PLAINTEXT, &[&pk]).expect("rpgp encrypts to gpg key");

        let home = tempfile::tempdir().expect("tmp GNUPGHOME");
        // Import the fixture secret key into the throwaway keyring.
        let mut import = Command::new("gpg")
            .env("GNUPGHOME", home.path())
            .args(["--batch", "--import"])
            .stdin(Stdio::piped())
            .spawn()
            .expect("spawn gpg import");
        import
            .stdin
            .take()
            .expect("gpg stdin")
            .write_all(FIXTURE_SECRET)
            .expect("pipe fixture secret");
        let status = import.wait().expect("gpg import wait");
        assert!(status.success(), "gpg import failed");

        let ct_path = home.path().join("rpgp.gpg");
        std::fs::write(&ct_path, &ciphertext).expect("write ciphertext");
        let out = Command::new("gpg")
            .env("GNUPGHOME", home.path())
            .args([
                "--batch",
                "--yes",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                FIXTURE_PASSPHRASE,
                "--decrypt",
            ])
            .arg(&ct_path)
            .output()
            .expect("spawn gpg decrypt");
        assert!(
            out.status.success(),
            "gpg decrypt failed: {}",
            String::from_utf8_lossy(&out.stderr),
        );
        assert_eq!(out.stdout, EXPECTED_PLAINTEXT);
    }

    /// Reverse interop, keygen variant: rpgp generates a passphrase-protected key
    /// (iterated+salted V4 S2K — the gpg/gopass default), and system `gpg
    /// --decrypt` reads a message encrypted to it. Proves gpm-produced keys are
    /// desktop-gopass-readable.
    ///
    /// Same `#[ignore]` sandbox constraint as `rpgp_encrypts_gpg_decrypts`.
    #[test]
    #[ignore = "needs system gpg + sandbox disabled (gpg-agent socket)"]
    fn rpgp_passphrase_key_gpg_decrypts() {
        const DATA: &[u8] = b"rpgp-keygen-gpg-decrypt interop";
        let passphrase = SPIKE_PASSPHRASE;
        let (sk, pk) = generate_keypair(SPIKE_UID, Some(passphrase)).expect("keygen");
        let ciphertext = encrypt_to_recipients(DATA, &[&pk]).expect("encrypt");
        let armored_secret = armor_secret_key(&sk).expect("armor secret key");

        let home = tempfile::tempdir().expect("tmp GNUPGHOME");
        let mut import = Command::new("gpg")
            .env("GNUPGHOME", home.path())
            .args(["--batch", "--import"])
            .stdin(Stdio::piped())
            .spawn()
            .expect("spawn gpg import");
        import
            .stdin
            .take()
            .expect("gpg stdin")
            .write_all(armored_secret.as_bytes())
            .expect("pipe rpgp secret");
        let status = import.wait().expect("gpg import wait");
        assert!(status.success(), "gpg import failed");

        let ct_path = home.path().join("rpgp.gpg");
        std::fs::write(&ct_path, &ciphertext).expect("write ciphertext");
        let out = Command::new("gpg")
            .env("GNUPGHOME", home.path())
            .args([
                "--batch",
                "--yes",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                passphrase,
                "--decrypt",
            ])
            .arg(&ct_path)
            .output()
            .expect("spawn gpg decrypt");
        assert!(
            out.status.success(),
            "gpg decrypt of rpgp-generated key failed: {}",
            String::from_utf8_lossy(&out.stderr),
        );
        assert_eq!(out.stdout, DATA);
    }
}
