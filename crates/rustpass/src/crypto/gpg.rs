// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The GPG/OpenPGP crypto backend.
//!
//! Stateless: like [`AgeBackend`](crate::crypto::AgeBackend) it is a unit struct
//! holding no fields. The operational identity (an S2K-unlocked armored secret
//! key) flows through `Store::cached_identity` as type-erased bytes; recipient
//! public keys are resolved on demand from the repo's `.public-keys/<id>`. Both
//! the at-rest own key and the recipient keyring live on disk (the own key
//! AEAD-sealed via the existing identity slot), never on this struct.
//!
//! Blocking rpgp work runs on a blocking thread and is wrapped in `catch_unwind`
//! ([`blocking`]): rpgp can panic on crafted packets, and a pulled `<name>.gpg`
//! or pasted identity is attacker-controlled — a panic must not unwind through
//! the async runtime. Sync CPU helpers (`identity_recipient`,
//! `identity_requires_passphrase`) isolate their parse the same way without
//! `spawn_blocking`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use async_trait::async_trait;
use tokio::task::spawn_blocking;
use zeroize::Zeroizing;

use crate::crypto::openpgp::{
    armor_secret_key, decrypt_with_unlocked_key, encrypt_to_selected_subkeys,
    parse_armored_public_key, parse_armored_secret_key, primary_fingerprint,
    secret_key_fingerprint, secret_key_is_encrypted, strip_passphrase,
};
use crate::crypto::{
    BackendKind, CryptoBackend, CryptoProfile, GPG_PUBLIC_KEYS_DIR, GPG_RECIPIENTS_FILE, SecretExt,
};
use crate::error::{Error, ErrorCode};
use crate::recipient::{KeyType, Recipient};
use crate::storage::{FilePresence, RepoFileView};

/// The GPG/OpenPGP crypto backend. Stateless unit struct — see the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct GpgBackend;

/// Run a CPU-bound, panic-isolating rpgp op on a blocking thread. A panic
/// (crafted input) becomes a `StoreError` instead of unwinding through the async
/// runtime; the inner `Result` is the op's own. Mirrors the signing path's
/// `catch_unwind` discipline.
async fn blocking<F, T>(f: F) -> Result<T, Error>
where
    F: FnOnce() -> Result<T, Error> + Send + 'static,
    T: Send + 'static,
{
    spawn_blocking(move || {
        catch_unwind(AssertUnwindSafe(f))
            .map_err(|_| {
                Error::new(
                    ErrorCode::StoreError,
                    "GPG crypto op panicked (crafted OpenPGP input?)",
                )
            })
            .and_then(|r| r)
    })
    .await
    .map_err(|e| Error::new(ErrorCode::StoreError, format!("blocking task join: {e}")))?
}

#[async_trait]
impl CryptoBackend for GpgBackend {
    fn profile(&self) -> CryptoProfile {
        CryptoProfile {
            backend_kind: BackendKind::Gpg,
            secret_extension: SecretExt::GPG,
            recipients_filename: GPG_RECIPIENTS_FILE,
            public_keys_dir: Some(GPG_PUBLIC_KEYS_DIR),
        }
    }

    /// Operational identity = the S2K-unlocked armored secret key. The at-rest
    /// bytes are parsed, the passphrase layer is stripped (`remove_password`),
    /// and the unlocked key is re-armored — the form cached in
    /// `Store::cached_identity` and consumed by [`Self::decrypt`] with no
    /// passphrase. `remove_password` is a no-op on an already-unprotected key,
    /// so this also covers that case; an S2K checksum failure (wrong passphrase)
    /// surfaces as `WrongPassphrase`.
    async fn unlock_identity(
        &self,
        at_rest: &[u8],
        passphrase: &str,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let at_rest = Zeroizing::new(at_rest.to_vec());
        let passphrase = Zeroizing::new(passphrase.to_string());
        let armor = blocking(move || {
            let mut sk = parse_armored_secret_key(&at_rest)?;
            strip_passphrase(&mut sk, passphrase.as_str())?;
            Ok(armor_secret_key(&sk)?.into_bytes())
        })
        .await?;
        Ok(Zeroizing::new(armor))
    }

    async fn list_recipients(&self, view: &dyn RepoFileView) -> Result<Vec<Recipient>, Error> {
        if let FilePresence::Present = view.file_liveness(GPG_RECIPIENTS_FILE).await? {
            let bytes = view.read(GPG_RECIPIENTS_FILE).await?;
            let content = str::from_utf8(&bytes).map_err(|e| {
                Error::new(
                    ErrorCode::StoreError,
                    format!(".gpg-id is not valid UTF-8: {e}"),
                )
            })?;
            Ok(parse_gpg_id(content))
        } else {
            Ok(Vec::new())
        }
    }

    async fn encrypt(
        &self,
        plaintext: &[u8],
        identity: &[u8],
        view: &dyn RepoFileView,
    ) -> Result<Vec<u8>, Error> {
        // Read each recipient's armored pubkey via the gopass token==filename
        // invariant (`.public-keys/<verbatim token>`); shared with
        // [`CryptoBackend::identity_is_recipient`].
        let armors = self.read_recipient_armors(view).await?;

        let plaintext = Zeroizing::new(plaintext.to_vec());
        let identity = Zeroizing::new(identity.to_vec());
        blocking(move || {
            // ensureOurKeyID: our own key must be a recipient so we can decrypt
            // what we write. Match by primary fingerprint — a gopass `.gpg-id`
            // token may be a long key id where our key reports its fingerprint,
            // so a naive string compare misfires on non-canonical stores.
            let our_pubkey = parse_armored_secret_key(&identity)?.to_public_key();
            let our_fingerprint = primary_fingerprint(&our_pubkey);

            let mut pubkeys: Vec<_> = Vec::with_capacity(armors.len() + 1);
            let mut seen_own = false;
            for armor in &armors {
                let pk = parse_armored_public_key(armor)?;
                if primary_fingerprint(&pk) == our_fingerprint {
                    seen_own = true;
                }
                pubkeys.push(pk);
            }
            if !seen_own {
                pubkeys.push(our_pubkey);
            }
            let refs: Vec<_> = pubkeys.iter().collect();
            encrypt_to_selected_subkeys(&plaintext, &refs)
        })
        .await
    }

    async fn decrypt(&self, ciphertext: &[u8], identity: &[u8]) -> Result<Vec<u8>, Error> {
        let ciphertext = ciphertext.to_vec();
        let identity = Zeroizing::new(identity.to_vec());
        blocking(move || {
            let sk = parse_armored_secret_key(&identity)?;
            decrypt_with_unlocked_key(&ciphertext, &sk)
        })
        .await
    }

    async fn validate_identity_passphrase(
        &self,
        identity_bytes: &[u8],
        passphrase: &str,
    ) -> Result<(), Error> {
        let bytes = Zeroizing::new(identity_bytes.to_vec());
        let passphrase = Zeroizing::new(passphrase.to_string());
        blocking(move || {
            let mut sk = parse_armored_secret_key(&bytes)?;
            strip_passphrase(&mut sk, passphrase.as_str())?; // throwaway; no serialize
            Ok(())
        })
        .await
    }

    /// The gopass recipient id: `0x` + the last 16 hex of the primary
    /// fingerprint (gopass's `Key.ID()`). The fingerprint is public-packet data,
    /// so no passphrase is needed — `_passphrase` is ignored.
    fn identity_recipient(
        &self,
        identity: &str,
        _passphrase: Option<&str>,
    ) -> Result<String, Error> {
        let bytes = identity.as_bytes();
        // Isolate the whole op — the parse + fingerprint are rpgp calls over
        // attacker-controllable identity bytes, not just the parse.
        catch_unwind(AssertUnwindSafe(|| {
            let fp = secret_key_fingerprint(bytes)?;
            if fp.len() < 25 {
                return Err(Error::new(
                    ErrorCode::InvalidIdentity,
                    "GPG key fingerprint too short (v4 40-hex fingerprint required)",
                ));
            }
            Ok(format!("0x{}", &fp[24..]))
        }))
        .map_err(|_| Error::new(ErrorCode::InvalidIdentity, "GPG secret key parse panicked"))?
    }

    fn identity_requires_passphrase(&self, identity_bytes: &[u8]) -> bool {
        // Fail-open on a parse error / panic (mirrors age): the real check runs at
        // unlock, which rejects a bad key with a clear error. `secret_key_is_encrypted`
        // is rpgp introspection over attacker-controllable bytes, so it stays inside
        // the `catch_unwind`.
        matches!(
            catch_unwind(AssertUnwindSafe(|| {
                Ok::<bool, Error>(secret_key_is_encrypted(&parse_armored_secret_key(
                    identity_bytes,
                )?))
            })),
            Ok(Ok(true))
        )
    }

    async fn identity_is_recipient(
        &self,
        identity: &str,
        _passphrase: Option<&str>,
        view: &dyn RepoFileView,
    ) -> Result<bool, Error> {
        // Resolve each recipient's pubkey (hard-errors if the pool is incomplete
        // — a `.gpg-id` token with no `.public-keys/<token>` entry → `Err`, the
        // "repo looks incomplete" case, never a silent allow). Then match primary
        // fingerprints; no passphrase needed (public-packet data). A token may be
        // a long key id OR a full fingerprint, so the comparison is by fingerprint
        // — not a naive string compare against `identity_recipient`'s `0x`+16hex.
        let armors = self.read_recipient_armors(view).await?;
        let identity = Zeroizing::new(identity.as_bytes().to_vec());
        blocking(move || {
            let our_fp = secret_key_fingerprint(&identity)?;
            for armor in &armors {
                if primary_fingerprint(&parse_armored_public_key(armor)?) == our_fp {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .await
    }
}

impl GpgBackend {
    /// Read each `.gpg-id` recipient's armored pubkey via the gopass
    /// token==filename invariant (`.public-keys/<verbatim token>` — do NOT
    /// canonicalize the token; gopass guarantees it is the pubkey filename).
    /// Shared by [`CryptoBackend::encrypt`] (to build the recipient set) and
    /// [`CryptoBackend::identity_is_recipient`] (to resolve fingerprints). A token
    /// with no `.public-keys/<token>` entry is a hard error — the recipient pool is
    /// incomplete, not silently empty.
    async fn read_recipient_armors(&self, view: &dyn RepoFileView) -> Result<Vec<String>, Error> {
        let tokens: Vec<String> = self
            .list_recipients(view)
            .await?
            .into_iter()
            .map(|r| r.public_key)
            .collect();
        let mut armors: Vec<String> = Vec::with_capacity(tokens.len());
        for token in &tokens {
            let path = format!("{GPG_PUBLIC_KEYS_DIR}/{token}");
            let bytes = view.read(&path).await.map_err(|e| {
                if e.code == "ENTRY_NOT_FOUND" {
                    Error::new(
                        ErrorCode::InvalidIdentity,
                        format!(
                            "recipient {token} listed in .gpg-id has no .public-keys/{token} entry"
                        ),
                    )
                } else {
                    e
                }
            })?;
            let armor = str::from_utf8(&bytes).map_err(|e| {
                Error::new(
                    ErrorCode::InvalidIdentity,
                    format!("recipient {token} pubkey is not valid UTF-8: {e}"),
                )
            })?;
            armors.push(armor.to_string());
        }
        Ok(armors)
    }
}

/// Parse gopass's `.gpg-id`: one recipient id per line (`0x` + long key id or a
/// full fingerprint), `#` comments and blank lines skipped. Tokens are kept
/// verbatim — gopass guarantees the token is the `.public-keys/<token>` filename,
/// so resolution uses it as-is (no canonicalization).
fn parse_gpg_id(content: &str) -> Vec<Recipient> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            Some(Recipient {
                public_key: line.to_string(),
                comment: None,
                key_type: KeyType::Gpg,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! `GpgBackend` seam coverage. In-module because the keygen primitive is
    //! `pub(crate)`. Exercises every trait method against a real on-disk store
    //! layout (`.gpg-id` + `.public-keys/<id>`), the gopass-canonical recipient
    //! id form, fingerprint-based matching across id forms, and a system-`gpg`
    //! fixture for interop.

    use super::*;
    use crate::crypto::openpgp::{
        armor_public_key, armor_secret_key, generate_keypair, primary_fingerprint,
        secret_key_fingerprint,
    };
    use crate::recipient::KeyType;
    use crate::storage::{GitStorage, RepoFiles};

    const UID: &str = "gpm test <test@gpm.local>";
    const PASSPHRASE: &str = "test-passphrase";

    // Committed system-gpg fixtures (RSA-2048, --compress-algo=none) for interop.
    const FIXTURE_SECRET: &[u8] = include_bytes!("../../tests/fixtures/gpg/secret.asc");
    const FIXTURE_GPG_ENCRYPTED: &[u8] =
        include_bytes!("../../tests/fixtures/gpg/gpg-encrypted.gpg");
    const FIXTURE_PASSPHRASE: &str = "test-passphrase-fixture-only";
    const EXPECTED_PLAINTEXT: &[u8] = b"gpg-to-rpgp interop plaintext";

    /// A generated keypair materialized in the three forms the backend touches.
    struct Key {
        at_rest: String,     // S2K-locked armored secret (the on-disk identity)
        recipient: String,   // gopass Key.ID(): 0x + last 16 hex
        fingerprint: String, // full 40-hex primary fingerprint
        pubkey: String,      // armored public key (for `.public-keys/<id>`)
    }

    fn gen_key(passphrase: Option<&str>) -> Key {
        let (sk, pk) = generate_keypair(UID, passphrase).expect("keygen");
        let at_rest = armor_secret_key(&sk).expect("armor secret");
        let pubkey = armor_public_key(&pk).expect("armor public");
        let fingerprint = primary_fingerprint(&pk);
        let recipient = GpgBackend
            .identity_recipient(&at_rest, None)
            .expect("derive recipient");
        Key {
            at_rest,
            recipient,
            fingerprint,
            pubkey,
        }
    }

    /// Build a store dir with `.gpg-id` + `.public-keys/<recipient>` per key.
    fn gpg_store(keys: &[&Key]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let public_keys_dir = dir.path().join(GPG_PUBLIC_KEYS_DIR);
        std::fs::create_dir(&public_keys_dir).unwrap();
        let mut id = String::new();
        for k in keys {
            id.push_str(&k.recipient);
            id.push('\n');
            std::fs::write(public_keys_dir.join(&k.recipient), &k.pubkey).unwrap();
        }
        std::fs::write(dir.path().join(GPG_RECIPIENTS_FILE), id).unwrap();
        dir
    }

    #[tokio::test]
    async fn roundtrip_passphrase() {
        let me = gen_key(Some(PASSPHRASE));
        let dir = gpg_store(&[&me]);
        let backend = GpgBackend;
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        let unlocked = backend
            .unlock_identity(me.at_rest.as_bytes(), PASSPHRASE)
            .await
            .expect("unlock");
        let plaintext = b"gpg round trip";
        let ciphertext = backend
            .encrypt(plaintext, &unlocked, &view)
            .await
            .expect("encrypt");
        let decrypted = backend
            .decrypt(&ciphertext, &unlocked)
            .await
            .expect("decrypt");
        assert_eq!(decrypted.as_slice(), plaintext);
    }

    #[tokio::test]
    async fn roundtrip_no_passphrase() {
        let me = gen_key(None);
        let dir = gpg_store(&[&me]);
        let backend = GpgBackend;
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        // An unprotected key unlocks with any passphrase (remove_password no-op).
        let unlocked = backend
            .unlock_identity(me.at_rest.as_bytes(), "")
            .await
            .expect("unlock unprotected");
        let ciphertext = backend
            .encrypt(b"no-pw", &unlocked, &view)
            .await
            .expect("encrypt");
        let decrypted = backend
            .decrypt(&ciphertext, &unlocked)
            .await
            .expect("decrypt");
        assert_eq!(decrypted, b"no-pw");
    }

    #[tokio::test]
    async fn unlock_wrong_passphrase_is_wrong_passphrase() {
        let me = gen_key(Some(PASSPHRASE));
        let err = GpgBackend
            .unlock_identity(me.at_rest.as_bytes(), "wrong")
            .await
            .expect_err("wrong passphrase must fail");
        assert_eq!(err.code, "WRONG_PASSPHRASE");
    }

    #[tokio::test]
    async fn unlock_corrupt_armor_is_invalid_identity() {
        let err = GpgBackend
            .unlock_identity(b"not a pgp key block", PASSPHRASE)
            .await
            .expect_err("corrupt armor must fail");
        assert_eq!(err.code, "INVALID_IDENTITY");
    }

    /// `identity_recipient` yields gopass's `Key.ID()` = `0x` + last 16 hex.
    #[test]
    fn identity_recipient_is_gopass_key_id() {
        let me = gen_key(Some(PASSPHRASE));
        let expected = format!("0x{}", &me.fingerprint[24..]);
        assert_eq!(me.recipient, expected);
        assert!(
            me.recipient.starts_with("0x") && me.recipient.len() == 2 + 16,
            "gopass Key.ID() is 0x + 16 hex, got {}",
            me.recipient
        );
    }

    /// #41: the token derived from a real gpg-produced key must be UPPERCASE,
    /// matching gopass's `.gpg-id` token byte-for-byte (`0x` + last 16 of the
    /// primary fingerprint). gopass matches fingerprints case-sensitively, so a
    /// lowercase token (rpgp's `Fingerprint: Display`) wouldn't round-trip through
    /// gopass. Pinned against the fixture's authoritative `gpg --with-colons` fpr;
    /// host-only (no gpg/gpg-agent). RED before the fix, GREEN after.
    #[test]
    fn identity_recipient_matches_real_gpg_key_uppercase() {
        let armor = std::str::from_utf8(FIXTURE_SECRET).expect("fixture armor is utf-8");
        // Full primary fingerprint = gpg's uppercase fpr field.
        assert_eq!(
            secret_key_fingerprint(FIXTURE_SECRET).expect("derive fingerprint"),
            "45953F0FF50CD57C869CCB848C78A415A6EDA09F",
            "fingerprint must be uppercase hex matching gpg --with-colons",
        );
        // Recipient token = 0x + last 16 hex, uppercase.
        let recipient = GpgBackend
            .identity_recipient(armor, None)
            .expect("derive recipient");
        // gopass Key.ID() convention: lowercase `0x` prefix + UPPERCASE hex
        // digits — the digits are what gopass matches case-sensitively.
        assert_eq!(recipient, "0x8C78A415A6EDA09F");
    }

    /// .gpg-id parser keeps both id forms verbatim, skips comments/blanks.
    #[tokio::test]
    async fn list_recipients_parses_gpg_id_forms() {
        let dir = tempfile::tempdir().unwrap();
        let long_fp = "ABCD0123456789ABCDEF0123456789ABCDEF0123";
        std::fs::write(
            dir.path().join(GPG_RECIPIENTS_FILE),
            format!("# comment\n\n0x0123456789ABCDEF\n{long_fp}\n# tail\n"),
        )
        .unwrap();
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        let got = GpgBackend.list_recipients(&view).await.expect("parse");
        assert_eq!(
            got.len(),
            2,
            "two recipient lines, rest are comments/blanks"
        );
        assert_eq!(got.first().unwrap().public_key, "0x0123456789ABCDEF");
        assert_eq!(got.first().unwrap().key_type, KeyType::Gpg);
        assert_eq!(got.get(1).unwrap().public_key, long_fp);
    }

    #[tokio::test]
    async fn list_recipients_absent_index_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        let got = GpgBackend
            .list_recipients(&view)
            .await
            .expect("absent index = uninitialized store");
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn list_recipients_rejects_non_utf8_index() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(GPG_RECIPIENTS_FILE), b"0xabc\n\xff\xfe\n").unwrap();
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        let err = GpgBackend
            .list_recipients(&view)
            .await
            .expect_err("non-UTF-8 .gpg-id must be a hard error");
        assert_eq!(err.code, "STORE_ERROR");
    }

    /// Fingerprint matching: a store that lists our key by its FULL
    /// fingerprint (gopass's canonicalizeRecipient case-0 form) — not the
    /// `0x`+16hex `identity_recipient` produces — must still resolve and encrypt,
    /// because ensureOurKeyID matches by fingerprint, not token string.
    #[tokio::test]
    async fn encrypt_matches_recipient_by_fingerprint_not_token() {
        let me = gen_key(Some(PASSPHRASE));
        let dir = tempfile::tempdir().unwrap();
        let public_keys_dir = dir.path().join(GPG_PUBLIC_KEYS_DIR);
        std::fs::create_dir(&public_keys_dir).unwrap();
        // .gpg-id + .public-keys/<full-fp> use the full fingerprint as the token.
        std::fs::write(
            dir.path().join(GPG_RECIPIENTS_FILE),
            format!("{}\n", me.fingerprint),
        )
        .unwrap();
        std::fs::write(public_keys_dir.join(&me.fingerprint), &me.pubkey).unwrap();

        let backend = GpgBackend;
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        let unlocked = backend
            .unlock_identity(me.at_rest.as_bytes(), PASSPHRASE)
            .await
            .expect("unlock");
        let ciphertext = backend
            .encrypt(b"cross-form", &unlocked, &view)
            .await
            .expect("encrypt resolves a full-fp token");
        let decrypted = backend
            .decrypt(&ciphertext, &unlocked)
            .await
            .expect("decrypt");
        assert_eq!(decrypted, b"cross-form");
    }

    /// Membership is by primary fingerprint, so a key listed by its FULL
    /// fingerprint (gopass's canonicalizeRecipient case-0) — not the
    /// `identity_recipient` `0x`+16hex — is still recognized. Pins the
    /// `save_identity` gate's correctness for non-canonical stores.
    #[tokio::test]
    async fn gpg_identity_is_recipient_matches_full_fingerprint_token() {
        let me = gen_key(Some(PASSPHRASE));
        let dir = tempfile::tempdir().unwrap();
        let public_keys_dir = dir.path().join(GPG_PUBLIC_KEYS_DIR);
        std::fs::create_dir(&public_keys_dir).unwrap();
        // .gpg-id + .public-keys/<full-fp> use the full fingerprint as the token.
        std::fs::write(
            dir.path().join(GPG_RECIPIENTS_FILE),
            format!("{}\n", me.fingerprint),
        )
        .unwrap();
        std::fs::write(public_keys_dir.join(&me.fingerprint), &me.pubkey).unwrap();

        let backend = GpgBackend;
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        assert!(
            backend
                .identity_is_recipient(&me.at_rest, None, &view)
                .await
                .expect("member check"),
            "a key listed by full fingerprint is a recipient"
        );
    }

    #[tokio::test]
    async fn gpg_identity_is_recipient_true_for_a_listed_key() {
        let me = gen_key(Some(PASSPHRASE));
        let dir = gpg_store(&[&me]);
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        assert!(
            GpgBackend
                .identity_is_recipient(&me.at_rest, None, &view)
                .await
                .expect("member check")
        );
    }

    #[tokio::test]
    async fn gpg_identity_is_recipient_false_for_an_unlisted_key() {
        let me = gen_key(Some(PASSPHRASE));
        let other = gen_key(Some(PASSPHRASE));
        let dir = gpg_store(&[&other]); // only `other` is a recipient
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        assert!(
            !GpgBackend
                .identity_is_recipient(&me.at_rest, None, &view)
                .await
                .expect("member check"),
            "a key not in .gpg-id is not a recipient"
        );
    }

    /// An incomplete recipient pool (a `.gpg-id` token with no `.public-keys/`
    /// entry) is an `Err`, not a silent `Ok(false)` — the "repo looks incomplete"
    /// case the membership contract distinguishes from "definitely not a member".
    #[tokio::test]
    async fn gpg_identity_is_recipient_errors_when_pool_is_incomplete() {
        let me = gen_key(Some(PASSPHRASE));
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(GPG_PUBLIC_KEYS_DIR)).unwrap();
        // .gpg-id lists a token whose .public-keys/<token> is missing.
        std::fs::write(dir.path().join(GPG_RECIPIENTS_FILE), "0xDEADBEEFCAFEBABE\n").unwrap();

        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        let err = GpgBackend
            .identity_is_recipient(&me.at_rest, None, &view)
            .await
            .expect_err("incomplete pool must error, not silently allow");
        assert_eq!(err.code, "INVALID_IDENTITY");
    }

    /// ensureOurKeyID: our key absent from `.gpg-id` (only another key listed) —
    /// we must still decrypt what we write.
    #[tokio::test]
    async fn encrypt_ensures_our_recipient_when_absent() {
        let me = gen_key(Some(PASSPHRASE));
        let other = gen_key(Some(PASSPHRASE));
        let dir = gpg_store(&[&other]); // only `other` listed
        let backend = GpgBackend;
        let storage = GitStorage::new(dir.path());
        let view = RepoFiles::new(&storage);
        let unlocked = backend
            .unlock_identity(me.at_rest.as_bytes(), PASSPHRASE)
            .await
            .expect("unlock");
        let ciphertext = backend
            .encrypt(b"absent", &unlocked, &view)
            .await
            .expect("encrypt adds our key");
        let decrypted = backend
            .decrypt(&ciphertext, &unlocked)
            .await
            .expect("we can decrypt our own write");
        assert_eq!(decrypted, b"absent");
    }

    /// Attacker-controlled ciphertext must error (`DecryptFailed`), not panic —
    /// the `catch_unwind` contract on the decrypt path.
    #[tokio::test]
    async fn decrypt_attacker_bytes_does_not_panic() {
        let me = gen_key(Some(PASSPHRASE));
        let unlocked = GpgBackend
            .unlock_identity(me.at_rest.as_bytes(), PASSPHRASE)
            .await
            .unwrap();
        let err = GpgBackend
            .decrypt(b"not valid openpgp \xff\xfe\xfd", &unlocked)
            .await
            .expect_err("garbage must error");
        assert_eq!(err.code, "DECRYPT_FAILED");
    }

    /// `identity_requires_passphrase` reflects the S2K state.
    #[test]
    fn identity_requires_passphrase_reflects_s2k() {
        let locked = gen_key(Some(PASSPHRASE));
        let plain = gen_key(None);
        assert!(GpgBackend.identity_requires_passphrase(locked.at_rest.as_bytes()));
        assert!(!GpgBackend.identity_requires_passphrase(plain.at_rest.as_bytes()));
    }

    /// `validate_identity_passphrase` gates on the S2K passphrase.
    #[tokio::test]
    async fn validate_identity_passphrase_right_and_wrong() {
        let me = gen_key(Some(PASSPHRASE));
        GpgBackend
            .validate_identity_passphrase(me.at_rest.as_bytes(), PASSPHRASE)
            .await
            .expect("right passphrase validates");
        let err = GpgBackend
            .validate_identity_passphrase(me.at_rest.as_bytes(), "wrong")
            .await
            .expect_err("wrong passphrase rejected");
        assert_eq!(err.code, "WRONG_PASSPHRASE");
    }

    /// Interop: unlock a system-gpg RSA-2048 fixture key and decrypt real gpg
    /// ciphertext through `GpgBackend` — proves reading existing gopass stores.
    #[tokio::test]
    async fn decrypts_gpg_produced_ciphertext() {
        let unlocked = GpgBackend
            .unlock_identity(FIXTURE_SECRET, FIXTURE_PASSPHRASE)
            .await
            .expect("unlock gpg fixture key");
        let decrypted = GpgBackend
            .decrypt(FIXTURE_GPG_ENCRYPTED, &unlocked)
            .await
            .expect("decrypt gpg-produced ciphertext");
        assert_eq!(decrypted, EXPECTED_PLAINTEXT);
    }
}
