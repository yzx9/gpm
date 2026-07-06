// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Low-level OpenPGP (rpgp) wrapper for GPG commit-signature verification.
//!
//! This is the shared OpenPGP seam: it owns the `pgp` (rpgp) dependency so the
//! rest of the crate never names rpgp directly, and the future GPG crypto
//! backend (RFC 0036) reuses these parsing primitives for its keyring.
//!
//! Scope is **verification only**: parse an armored public key, parse a detached
//! signature, verify the signature over arbitrary bytes against a trusted key
//! set, and report which trusted key (primary or subkey) verified it. It does
//! NOT encrypt, decrypt, sign, or manage secret material.
//!
//! All functions are synchronous CPU work — callers (the sync signing path, or
//! a `spawn_blocking` from async Tauri commands) own the threading model.
//! Callers running rpgp over attacker-controlled commit bytes should also wrap
//! the per-commit call in `catch_unwind` (see `signing::status_of_commit`):
//! rpgp returns `Result` for parse/verify failures, but a panic on a crafted
//! packet would otherwise unwind through the whole commit walk.

use std::fmt::Write;

use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use pgp::types::{Fingerprint, KeyDetails, KeyId};

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
    let issuer = issuer_fps
        .first()
        .map(|fp| format!("{fp}"))
        .or_else(|| {
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

#[cfg(test)]
mod tests {
    //! Real-world interop proofs (RFC 0009 D2). The fixtures here are committed
    //! bytes produced by the actual `gpg` CLI (Ed25519 primary + Ed25519 signing
    //! subkey, subkey-signed — `GnuPG`'s default), so these tests prove rpgp
    //! works on keys users actually have, NOT only on rpgp's own signatures.
    //! No `gpg` binary is needed at test time.

    use super::*;

    const GPG_PUBKEY: &str = include_str!("../../tests/fixtures/gpg/pubkey.asc");
    const GPG_SIG: &str = include_str!("../../tests/fixtures/gpg/payload.sig.asc");
    const GPG_PAYLOAD: &[u8] = include_bytes!("../../tests/fixtures/gpg/payload.txt");

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
}
