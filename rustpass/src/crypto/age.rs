// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::str::{self, FromStr};

use age::armor::{ArmoredReader, ArmoredWriter, Format};
use age::secrecy::{ExposeSecret, SecretString};
use age::{Decryptor, Encryptor, IdentityFile, scrypt, ssh};
use tokio::fs;
use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};

/// A freshly generated native x25519 age identity plus its public recipient.
///
/// `identity` is wrapped in [`Zeroizing`] so the secret is wiped when the value
/// is dropped. The recipient is a public key, safe to store in plaintext.
pub struct AgeIdentity {
    /// The native x25519 secret identity (`AGE-SECRET-KEY-...`).
    pub identity: Zeroizing<String>,
    /// The matching public recipient (`age1...`).
    pub recipient: String,
}

/// Redacts `identity` — mirrors `rustpass::Secret` so `Debug` never leaks the
/// x25519 secret scalar (the derived `Debug` would print `Zeroizing<String>`).
impl fmt::Debug for AgeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgeIdentity")
            .field("identity", &"[REDACTED]")
            .field("recipient", &self.recipient)
            .finish()
    }
}

/// Generate a new native x25519 age identity and derive its public recipient.
///
/// This is gpm's in-app `age-keygen` equivalent — used by the create-store flow
/// to mint a brand-new identity on device. The identity is never written to disk
/// by this function; the caller persists it through the existing identity
/// storage (seal encryption; biometric-keystore gating on Android).
#[must_use]
pub fn generate_age_identity() -> AgeIdentity {
    let sk = age::x25519::Identity::generate();
    let recipient = sk.to_public().to_string();
    let identity = Zeroizing::new(sk.to_string().expose_secret().to_string());
    AgeIdentity {
        identity,
        recipient,
    }
}

/// Decrypt an `.age` file using the given identity bytes.
///
/// Returns the raw decrypted bytes. The caller is responsible for zeroizing
/// the identity after calling this function.
///
/// # Errors
///
/// Returns an error if the file cannot be read, the identity format is invalid,
/// or decryption fails.
pub async fn decrypt_file(
    file_path: &Path,
    identity_bytes: &[u8],
    passphrase: Option<&str>,
) -> Result<Vec<u8>, Error> {
    let encrypted = fs::read(file_path).await.map_err(|e| {
        Error::new(
            ErrorCode::IoError,
            format!("Failed to read entry file: {e}"),
        )
    })?;

    decrypt_bytes(&encrypted, identity_bytes, passphrase)
}

/// Decrypt age-encrypted bytes using the given identity.
///
/// Supports both native x25519 identities (`AGE-SECRET-KEY-...`) and SSH
/// private keys (OpenSSH or PEM format). Encrypted SSH keys require a
/// passphrase to be provided via the `passphrase` parameter.
///
/// # Errors
///
/// Returns an error if the identity format is invalid, contains no valid
/// identities, the encrypted data cannot be parsed, decryption fails, or
/// an encrypted SSH key is provided without a passphrase.
pub fn decrypt_bytes(
    encrypted: &[u8],
    identity_bytes: &[u8],
    passphrase: Option<&str>,
) -> Result<Vec<u8>, Error> {
    let identity_str = str::from_utf8(identity_bytes)
        .map_err(|_| Error::new(ErrorCode::InvalidIdentity, "Identity is not valid UTF-8"))?;
    let trimmed = identity_str.trim();

    // Intercept post-quantum identities before the underlying age parser
    // produces an opaque error — the Rust age crate (0.11.x) has no PQ support.
    if trimmed.starts_with("AGE-SECRET-KEY-PQ-1") {
        return Err(Error::new(
            ErrorCode::PostQuantumNotSupported,
            "Post-quantum (ML-KEM-768 / X-Wing) age keys aren't supported yet",
        ));
    }

    let identities: Vec<Box<dyn age::Identity>> = if trimmed.starts_with("AGE-SECRET-KEY-") {
        // x25519 path
        let identity_file = IdentityFile::from_buffer(identity_bytes).map_err(|_| {
            Error::new(
                ErrorCode::InvalidIdentity,
                "Identity is not valid AGE-SECRET-KEY-... format",
            )
        })?;
        identity_file.into_identities().map_err(|_| {
            Error::new(
                ErrorCode::InvalidIdentity,
                "Identity file contains no valid identities",
            )
        })?
    } else if trimmed.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
        || trimmed.starts_with("-----BEGIN RSA PRIVATE KEY-----")
    {
        // SSH path
        let buf = BufReader::new(trimmed.as_bytes());
        let ssh_identity =
            ssh::Identity::from_buffer(buf, passphrase.map(String::from)).map_err(|e| {
                Error::new(
                    ErrorCode::InvalidIdentity,
                    format!("Cannot parse SSH private key: {e}"),
                )
            })?;

        match ssh_identity {
            ssh::Identity::Unencrypted(_) => vec![Box::new(ssh_identity)],
            ssh::Identity::Encrypted(enc) => {
                // age's Identity trait returns None for Encrypted variants.
                // We must decrypt the SSH key ourselves, then use the UnencryptedKey.
                let Some(pw) = passphrase else {
                    return Err(Error::new(
                        ErrorCode::IdentityEncrypted,
                        "Encrypted SSH key requires a passphrase",
                    ));
                };
                let passphrase_str: SecretString = pw.to_string().into();
                let decrypted_key = enc.decrypt(passphrase_str).map_err(|e| {
                    Error::new(
                        ErrorCode::DecryptFailed,
                        format!("Failed to decrypt SSH key: {e}"),
                    )
                })?;
                let unencrypted = ssh::Identity::Unencrypted(decrypted_key);
                vec![Box::new(unencrypted)]
            }
            ssh::Identity::Unsupported(u) => {
                return Err(Error::new(
                    ErrorCode::InvalidIdentity,
                    format!("Unsupported SSH key type: {u:?}"),
                ));
            }
        }
    } else {
        return Err(Error::new(
            ErrorCode::InvalidIdentity,
            "Identity must be an age secret key (AGE-SECRET-KEY-...) or SSH private key",
        ));
    };

    if identities.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidIdentity,
            "No valid identities found",
        ));
    }

    // Build a decryptor from the age format (armored or binary)
    let Ok(decryptor) = Decryptor::new(encrypted) else {
        return Err(Error::new(
            ErrorCode::DecryptFailed,
            "Failed to parse encrypted data",
        ));
    };

    // Perform decryption
    let mut output = Vec::new();
    match decryptor.decrypt(identities.iter().map(AsRef::as_ref)) {
        Ok(mut reader) => {
            if reader.read_to_end(&mut output).is_err() {
                return Err(Error::new(
                    ErrorCode::DecryptFailed,
                    "Decryption failed — wrong identity or corrupted data",
                ));
            }
        }
        Err(_) => {
            return Err(Error::new(
                ErrorCode::DecryptFailed,
                "Decryption failed — wrong identity or corrupted data",
            ));
        }
    }

    Ok(output)
}

/// Encrypt plaintext to one or more age recipients, returning binary ciphertext.
///
/// Each recipient string may be a native x25519 public key (`age1...`), an SSH
/// public key (`ssh-ed25519 ...` / `ssh-rsa ...`), or an age plugin recipient
/// (`age1<plugin>1...`, e.g. `age1yubikey1...` from age-plugin-yubikey) — exactly
/// as they appear in a gopass `.age-recipients` file. This
/// mirrors gopass's `age` crypto backend, which encrypts every secret to all
/// store recipients.
///
/// Plugin recipients are grouped by plugin name and wrapped one-per-plugin; the
/// age library then spawns `age-plugin-<name>` to wrap the file key. That only
/// works where the binary exists (desktop); on Android or without the binary it
/// fails with [`ErrorCode::PluginUnavailable`].
///
/// The output is unarmored (binary) age — the standard on-disk format for
/// gopass secrets and what [`decrypt_bytes`] expects.
///
/// # Errors
///
/// Returns `InvalidIdentity` if the recipient list is empty or any recipient
/// string cannot be parsed (unknown format, malformed SSH key). Returns
/// `PostQuantumNotSupported` for a post-quantum recipient. Returns
/// `PluginUnavailable` if a required `age-plugin-<name>` binary is missing.
/// Returns `DecryptFailed` if the age encryption step itself fails.
pub fn encrypt_to_recipients(plaintext: &[u8], recipients: &[String]) -> Result<Vec<u8>, Error> {
    if recipients.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidIdentity,
            "Cannot encrypt without at least one recipient",
        ));
    }

    // Native/SSH recipients become individual age Recipient trait objects. Plugin
    // recipients are grouped by plugin name and wrapped one-per-plugin, because
    // the age plugin protocol drives a single `age-plugin-<name>` subprocess for
    // all of that plugin's recipients at once. Post-quantum is rejected up front
    // so it keeps a distinct, accurate error (its `age1pq1` prefix would
    // otherwise parse as a plugin named `pq`).
    let mut parsed: Vec<Box<dyn age::Recipient>> = Vec::with_capacity(recipients.len());
    let mut plugin_groups: BTreeMap<String, Vec<age::plugin::Recipient>> = BTreeMap::new();

    for recipient in recipients {
        let trimmed = recipient.trim();
        if trimmed.starts_with("age1pq1") {
            return Err(Error::new(
                ErrorCode::PostQuantumNotSupported,
                "Post-quantum age recipients aren't supported yet",
            ));
        } else if let Ok(plugin_recipient) = age::plugin::Recipient::from_str(trimmed) {
            // A plugin recipient (`age1<plugin>1...`). Grouped by plugin name
            // because the protocol drives one `age-plugin-<name>` subprocess per
            // plugin. Native x25519 (`age1<data>`, bech32 HRP `age`) and SSH
            // keys both fail this parse and fall through to the native path.
            plugin_groups
                .entry(plugin_recipient.plugin().to_string())
                .or_default()
                .push(plugin_recipient);
        } else {
            parsed.push(parse_native_recipient(trimmed)?);
        }
    }

    // For each plugin, locate its binary (PATH lookup) and build the wrapper.
    // `MissingPlugin` surfaces here, before any file key is wrapped.
    for (plugin_name, group) in plugin_groups {
        let wrapper =
            age::plugin::RecipientPluginV1::new(&plugin_name, &group, &[], age::NoCallbacks)
                .map_err(map_encrypt_error)?;
        parsed.push(Box::new(wrapper));
    }

    let encryptor =
        Encryptor::with_recipients(parsed.iter().map(AsRef::as_ref)).map_err(map_encrypt_error)?;

    let mut ciphertext = Vec::new();
    let mut writer = encryptor.wrap_output(&mut ciphertext).map_err(|err| {
        Error::new(
            ErrorCode::DecryptFailed,
            format!("Encryption failed: {err}"),
        )
    })?;

    writer.write_all(plaintext).map_err(|err| {
        Error::new(
            ErrorCode::DecryptFailed,
            format!("Encryption write failed: {err}"),
        )
    })?;

    writer.finish().map_err(|err| {
        Error::new(
            ErrorCode::DecryptFailed,
            format!("Encryption finish failed: {err}"),
        )
    })?;

    Ok(ciphertext)
}

/// Map an age [`EncryptError`] to a safe [`Error`].
///
/// The plugin-specific cases are surfaced explicitly: a missing binary is
/// [`ErrorCode::PluginUnavailable`] with platform-appropriate guidance (install
/// it on desktop; on Android, where the binary cannot run at all, say so
/// plainly instead of suggesting an impossible install), and any other
/// plugin-reported error is a fixed string (NOT the plugin's `CMD_ERROR` body,
/// which is plugin-controlled text and must not reach the `WebView`). Everything
/// else (age-internal I/O, incompatible recipients) collapses to `DecryptFailed`
/// with the age message — those carry no plugin-controlled or secret content.
fn map_encrypt_error(err: age::EncryptError) -> Error {
    match err {
        age::EncryptError::MissingPlugin { binary_name } => {
            // The message is tailored per build target so it never tells a user
            // to do something impossible on their platform. `cfg!` is evaluated
            // at compile time, so each build gets the string for its target.
            let message = if cfg!(target_os = "android") {
                format!(
                    "Encryption needs the age plugin '{binary_name}', which can't run on \
                     Android — age plugins are external binaries this device cannot launch. \
                     A store that uses this recipient can't be written from this device."
                )
            } else {
                format!(
                    "age plugin '{binary_name}' was not found in PATH. Install it and try \
                     again (for age-plugin-yubikey: `cargo install age-plugin-yubikey` or \
                     your package manager). Plugin encryption only works where the binary \
                     is installed."
                )
            };
            Error::new(ErrorCode::PluginUnavailable, message)
        }
        // A plugin that ran but reported an error: the error body comes from the
        // `age-plugin-<name>` subprocess's `CMD_ERROR` stanza, so it is
        // plugin-controlled text. Surface a fixed message rather than echoing it,
        // to honor the "no untrusted/secret content in error messages" invariant.
        age::EncryptError::Plugin(_) => Error::new(
            ErrorCode::DecryptFailed,
            "An age plugin reported an error while wrapping the file key",
        ),
        other => Error::new(
            ErrorCode::DecryptFailed,
            format!("Encryption failed: {other}"),
        ),
    }
}

/// Parse a **non-plugin** recipient string into an age `Recipient`.
///
/// Handles native x25519 (`age1...`) and SSH (`ssh-...`) recipients. Plugin
/// recipients (`age1<plugin>1...`) and post-quantum (`age1pq1...`) are handled
/// by the caller ([`encrypt_to_recipients`]); this must not be called with them.
/// This mirrors the classification [`recipient::parse_recipients`](crate::recipient)
/// uses, kept here so the crypto module is self-contained for encryption.
fn parse_native_recipient(trimmed: &str) -> Result<Box<dyn age::Recipient>, Error> {
    if trimmed.starts_with("age1") {
        let r = age::x25519::Recipient::from_str(trimmed).map_err(|_| {
            Error::new(
                ErrorCode::InvalidIdentity,
                "Cannot parse age recipient (age1...) key",
            )
        })?;
        Ok(Box::new(r))
    } else if trimmed.starts_with("ssh-") {
        let r: ssh::Recipient = trimmed.parse().map_err(|e| {
            Error::new(
                ErrorCode::InvalidIdentity,
                format!("Cannot parse SSH recipient key: {e:?}"),
            )
        })?;
        Ok(Box::new(r))
    } else {
        Err(Error::new(
            ErrorCode::InvalidIdentity,
            "Recipient must be an age public key (age1...), a plugin recipient \
             (age1<plugin>1...), or an SSH public key (ssh-...)",
        ))
    }
}

/// Encrypt identity bytes with a passphrase using age scrypt, producing armored output.
///
/// Returns an ASCII-armored age encrypted blob (`-----BEGIN AGE ENCRYPTED FILE-----`).
/// This format is interoperable with `age -d -i key.age` on the command line.
///
/// # Errors
///
/// Returns `IdentityNotEncrypted` if the passphrase is empty.
/// Returns `DecryptFailed` if encryption fails for any other reason.
pub fn encrypt_identity(passphrase: &str, identity: &[u8]) -> Result<Vec<u8>, Error> {
    if passphrase.is_empty() {
        return Err(Error::new(
            ErrorCode::IdentityNotEncrypted,
            "Passphrase must not be empty",
        ));
    }

    let secret: SecretString = passphrase.to_string().into();
    let encryptor = Encryptor::with_user_passphrase(secret);

    let mut encrypted = Vec::new();
    let armored =
        ArmoredWriter::wrap_output(&mut encrypted, Format::AsciiArmor).map_err(|err| {
            Error::new(
                ErrorCode::DecryptFailed,
                format!("Armor setup failed: {err}"),
            )
        })?;

    let mut writer = encryptor.wrap_output(armored).map_err(|err| {
        Error::new(
            ErrorCode::DecryptFailed,
            format!("Encryption failed: {err}"),
        )
    })?;

    writer.write_all(identity).map_err(|err| {
        Error::new(
            ErrorCode::DecryptFailed,
            format!("Encryption write failed: {err}"),
        )
    })?;

    let armored = writer.finish().map_err(|err| {
        Error::new(
            ErrorCode::DecryptFailed,
            format!("Encryption finish failed: {err}"),
        )
    })?;

    armored.finish().map_err(|err| {
        Error::new(
            ErrorCode::DecryptFailed,
            format!("Armor finish failed: {err}"),
        )
    })?;

    Ok(encrypted)
}

/// Decrypt an age-encrypted identity using a passphrase.
///
/// Accepts both armored (`-----BEGIN AGE ENCRYPTED FILE-----`) and binary
/// age encrypted data. Returns the raw plaintext identity bytes.
///
/// # Errors
///
/// Returns `IdentityNotEncrypted` if the passphrase is empty.
/// Returns `WrongPassphrase` if the passphrase is incorrect.
/// Returns `DecryptFailed` if the encrypted data is corrupted or cannot be parsed.
pub fn decrypt_identity(passphrase: &str, encrypted: &[u8]) -> Result<Vec<u8>, Error> {
    if passphrase.is_empty() {
        return Err(Error::new(
            ErrorCode::IdentityNotEncrypted,
            "Passphrase must not be empty",
        ));
    }

    // Detect armored format and dearmor if needed
    let encrypted_data = if encrypted.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----") {
        let reader = ArmoredReader::new(encrypted);
        let mut buf = Vec::new();
        let mut dearmored = reader;
        dearmored.read_to_end(&mut buf).map_err(|err| {
            Error::new(
                ErrorCode::DecryptFailed,
                format!("Failed to dearmor: {err}"),
            )
        })?;
        buf
    } else {
        encrypted.to_vec()
    };

    let secret: SecretString = passphrase.to_string().into();
    let scrypt_identity = scrypt::Identity::new(secret);

    let Ok(decryptor) = Decryptor::new(encrypted_data.as_slice()) else {
        return Err(Error::new(
            ErrorCode::DecryptFailed,
            "Failed to parse encrypted identity data",
        ));
    };

    let mut output = Vec::new();
    let identities: Vec<Box<dyn age::Identity>> = vec![Box::new(scrypt_identity)];
    match decryptor.decrypt(identities.iter().map(AsRef::as_ref)) {
        Ok(mut reader) => {
            if reader.read_to_end(&mut output).is_err() {
                return Err(Error::new(
                    ErrorCode::WrongPassphrase,
                    "Wrong passphrase or corrupted identity data",
                ));
            }
        }
        Err(_) => {
            return Err(Error::new(
                ErrorCode::WrongPassphrase,
                "Wrong passphrase or corrupted identity data",
            ));
        }
    }

    Ok(output)
}

/// Validate a passphrase against an SSH private key without producing output.
///
/// Parses the key and, if it is passphrase-encrypted, attempts to decrypt it
/// with `passphrase`. Used by the biometric enable flow to reject a wrong SSH
/// passphrase before sealing it. Unencrypted keys succeed with any passphrase.
///
/// # Errors
///
/// Returns `WrongPassphrase` if the SSH key is encrypted and `passphrase` is
/// incorrect. Returns `InvalidIdentity` if the key cannot be parsed.
pub fn validate_ssh_key_passphrase(identity_bytes: &[u8], passphrase: &str) -> Result<(), Error> {
    let identity_str = str::from_utf8(identity_bytes)
        .map_err(|_| Error::new(ErrorCode::InvalidIdentity, "Identity is not valid UTF-8"))?;
    let buf = BufReader::new(identity_str.trim().as_bytes());
    let ssh_identity =
        ssh::Identity::from_buffer(buf, Some(passphrase.to_string())).map_err(|e| {
            Error::new(
                ErrorCode::InvalidIdentity,
                format!("Cannot parse SSH private key: {e}"),
            )
        })?;

    match ssh_identity {
        ssh::Identity::Encrypted(enc) => {
            let secret: SecretString = passphrase.to_string().into();
            enc.decrypt(secret).map_err(|_| {
                Error::new(ErrorCode::WrongPassphrase, "Wrong passphrase for SSH key")
            })?;
            Ok(())
        }
        ssh::Identity::Unencrypted(_) => Ok(()),
        ssh::Identity::Unsupported(u) => Err(Error::new(
            ErrorCode::InvalidIdentity,
            format!("Unsupported SSH key type: {u:?}"),
        )),
    }
}

/// True iff `identity_bytes` is an SSH private key whose body is
/// passphrase-encrypted.
///
/// Returns `false` for non-SSH bytes, invalid UTF-8, unencrypted SSH keys, or
/// unsupported SSH key types. This is the crypto backend's answer to "does this
/// identity need a passphrase?" — it keeps the age `ssh` types out of
/// [`crate::store`], which has no other reason to touch the age library.
///
/// The caller ([`crate::store::Store::is_identity_encrypted`]) has already
/// classified the identity as an SSH variant, so a `false` here means
/// specifically "unencrypted SSH key", not "not an SSH key".
#[must_use]
pub fn is_ssh_identity_encrypted(identity_bytes: &[u8]) -> bool {
    let Ok(text) = str::from_utf8(identity_bytes) else {
        return false;
    };
    let buf = BufReader::new(text.trim().as_bytes());
    matches!(
        ssh::Identity::from_buffer(buf, None),
        Ok(ssh::Identity::Encrypted(_))
    )
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::str::FromStr;

    use bech32::ToBase32;

    use super::*;

    /// Generate a random x25519 keypair, returning `(identity, recipient)` strings.
    fn generate_keypair() -> (String, String) {
        let generated = generate_age_identity();
        (generated.identity.as_str().to_owned(), generated.recipient)
    }

    /// Encrypt `plaintext` to the given recipient string, returning ciphertext.
    fn encrypt(plaintext: &[u8], recipient_str: &str) -> Vec<u8> {
        let recipient = age::x25519::Recipient::from_str(recipient_str).unwrap();
        let recipients: Vec<Box<dyn age::Recipient>> = vec![Box::new(recipient)];
        let encryptor = Encryptor::with_recipients(recipients.iter().map(AsRef::as_ref)).unwrap();
        let mut encrypted = Vec::new();
        let mut writer = encryptor.wrap_output(&mut encrypted).unwrap();
        writer.write_all(plaintext).unwrap();
        writer.finish().unwrap();
        encrypted
    }

    /// Encrypt `plaintext` to the given SSH recipient string, returning ciphertext.
    fn encrypt_to_ssh(plaintext: &[u8], recipient_str: &str) -> Vec<u8> {
        let recipient: ssh::Recipient = recipient_str.parse().unwrap();
        let recipients: Vec<Box<dyn age::Recipient>> = vec![Box::new(recipient)];
        let encryptor = Encryptor::with_recipients(recipients.iter().map(AsRef::as_ref)).unwrap();
        let mut encrypted = Vec::new();
        let mut writer = encryptor.wrap_output(&mut encrypted).unwrap();
        writer.write_all(plaintext).unwrap();
        writer.finish().unwrap();
        encrypted
    }

    #[test]
    fn generate_age_identity_produces_valid_self_round_trip_key() {
        let generated = generate_age_identity();

        // Identity is a native x25519 secret; recipient its public key.
        assert!(
            generated.identity.starts_with("AGE-SECRET-KEY-1"),
            "identity must be a native age secret key"
        );
        assert!(
            generated.recipient.starts_with("age1"),
            "recipient must be an age public key"
        );
        assert!(
            !generated.recipient.starts_with("age1pq1"),
            "recipient must not be post-quantum"
        );

        // Round-trip: encrypt to the recipient, decrypt with the identity.
        let plaintext = b"generated-identity-round-trip";
        let ciphertext = encrypt(plaintext, &generated.recipient);
        let decrypted =
            decrypt_bytes(&ciphertext, generated.identity.as_bytes(), None).expect("self decrypt");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn debug_redacts_identity() {
        let generated = generate_age_identity();
        let debug_output = format!("{generated:?}");
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug should redact the identity, got: {debug_output}"
        );
        assert!(
            !debug_output.contains("AGE-SECRET-KEY-"),
            "Debug must not contain the secret identity, got: {debug_output}"
        );
        // The recipient is public — safe to surface.
        assert!(
            debug_output.contains(&generated.recipient),
            "Debug should still show the recipient, got: {debug_output}"
        );
    }

    #[tokio::test]
    async fn decrypt_file_reads_and_decrypts() {
        let (identity, recipient) = generate_keypair();
        let plaintext = b"hunter2\nusername: bob";

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("entry.age");
        let ciphertext = encrypt(plaintext, &recipient);
        std::fs::write(&file_path, &ciphertext).unwrap();

        let result = decrypt_file(&file_path, identity.as_bytes(), None)
            .await
            .unwrap();
        assert_eq!(result, plaintext);

        let bytes_result = decrypt_bytes(&ciphertext, identity.as_bytes(), None).unwrap();
        assert_eq!(result, bytes_result);
    }

    #[tokio::test]
    async fn decrypt_file_missing_file() {
        let (identity, _recipient) = generate_keypair();
        let missing = std::path::PathBuf::from("/nonexistent/path/no-such-file.age");

        let err = decrypt_file(&missing, identity.as_bytes(), None)
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "IO_ERROR",
            "expected IO_ERROR for missing file, got: {err}"
        );
    }

    #[test]
    fn decrypt_bytes_with_ssh_ed25519() {
        let sk = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML
agAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ
AAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz
1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=
-----END OPENSSH PRIVATE KEY-----";
        let pk = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHsKLqeplhpW+uObz5dvMgjz1OxfM/XXUB+VHtZ6isGN";

        let plaintext = b"secret-password\nnotes: ssh encrypted";
        let ciphertext = encrypt_to_ssh(plaintext, pk);

        let result = decrypt_bytes(&ciphertext, sk.as_bytes(), None).unwrap();
        assert_eq!(result, plaintext);
    }

    #[test]
    fn decrypt_bytes_with_ssh_rsa() {
        let sk = "-----BEGIN RSA PRIVATE KEY-----
MIIEogIBAAKCAQEAxO5yF0xjbmkQTfbaCP8DQC7kHnPJr5bdIie6Nzmg9lL6Chye
0vK5iJ+BYkA1Hnf1WnNzoVIm3otZPkwZptertkY95JYFmTiA4IvHeL1yiOTd2AYc
a947EPpM9XPomeM/7U7c99OvuCuOl1YlTFsMsoPY/NiZ+NZjgMvb3XgyH0OXy3mh
qp+SsJU+tRjZGfqM1iv2TZUCJTQnKF8YSVCyLPV67XM1slQQHmtZ5Q6NFhzg3j8a
CY5rDR66UF5+Zn/TvN8bNdKn01I50VLePI0ZnnRcuLXK2t0Bpkk0NymZ3vsF10m9
HCKVyxr2Y0Ejx4BtYXOK97gaYks73rBi7+/VywIDAQABAoIBADGsf8TWtOH9yGoS
ES9hu90ttsbjqAUNhdv+r18Mv0hC5+UzEPDe3uPScB1rWrrDwXS+WHVhtoI+HhWz
tmi6UArbLvOA0Aq1EPUS7Q7Mop5bNIYwDG09EiMXL+BeC1b91nsygFRW5iULf502
0pOvB8XjshEdRcFZuqGbSmtTzTjLLxYS/aboBtZLHrH4cRlFMpHWCSuJng8Psahp
SnJbkjL7fHG81dlH+M3qm5EwdDJ1UmNkBfoSfGRs2pupk2cSJaL+SPkvNX+6Xyoy
yvfnbJzKUTcV6rf+0S0P0yrWK3zRK9maPJ1N60lFui9LvFsunCLkSAluGKiMwEjb
fm40F4kCgYEA+QzIeIGMwnaOQdAW4oc7hX5MgRPXJ836iALy56BCkZpZMjZ+VKpk
8P4E1HrEywpgqHMox08hfCTGX3Ph6fFIlS1/mkLojcgkrqmg1IrRvh8vvaZqzaAf
GKEhxxRta9Pvm44E2nUY97iCKzE3Vfh+FIyQLRuc+0COu49Me4HPtBUCgYEAym1T
vNZKPfC/eTMh+MbWMsQArOePdoHQyRC38zeWrLaDFOUVzwzEvCQ0IzSs0PnLWkZ4
xx60wBg5ZdU4iH4cnOYgjavQrbRFrCmZ1KDUm2+NAMw3avcLQqu41jqzyAlkktUL
fZzyqHIBmKYLqut5GslkGnQVg6hB4psutHhiel8CgYA3yy9WH9/C6QBxqgaWdSlW
fLby69j1p+WKdu6oCXUgXW3CHActPIckniPC3kYcHpUM58+o5wdfYnW2iKWB3XYf
RXQiwP6MVNwy7PmE5Byc9Sui1xdyPX75648/pEnnMDGrraNUtYsEZCd1Oa9l6SeF
vv/Fuzvt5caUKkQ+HxTDCQKBgFhqUiXr7zeIvQkiFVeE+a/ovmbHKXlYkCoSPFZm
VFCR00VAHjt2V0PaCE/MRSNtx61hlIVcWxSAQCnDbNLpSnQZa+SVRCtqzve4n/Eo
YlSV75+GkzoMN4XiXXRs5XOc7qnXlhJCiBac3Segdv4rpZTWm/uV8oOz7TseDtNS
tai/AoGAC0CiIJAzmmXscXNS/stLrL9bb3Yb+VZi9zN7Cb/w7B0IJ35N5UOFmKWA
QIGpMU4gh6p52S1eLttpIf2+39rEDzo8pY6BVmEp3fKN3jWmGS4mJQ31tWefupC+
fGNu+wyKxPnSU3svsuvrOdwwDKvfqCNyYK878qKAAaBqbGT1NJ8=
-----END RSA PRIVATE KEY-----";
        let pk = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDE7nIXTGNuaRBN9toI/wNALuQec8mvlt0iJ7o3OaD2UvoKHJ7S8rmIn4FiQDUed/Vac3OhUibei1k+TBmm16u2Rj3klgWZOIDgi8d4vXKI5N3YBhxr3jsQ+kz1c+iZ4z/tTtz306+4K46XViVMWwyyg9j82Jn41mOAy9vdeDIfQ5fLeaGqn5KwlT61GNkZ+ozWK/ZNlQIlNCcoXxhJULIs9XrtczWyVBAea1nlDo0WHODePxoJjmsNHrpQXn5mf9O83xs10qfTUjnRUt48jRmedFy4tcra3QGmSTQ3KZne+wXXSb0cIpXLGvZjQSPHgG1hc4r3uBpiSzvesGLv79XL";

        let plaintext = b"secret-password\nnotes: rsa encrypted";
        let ciphertext = encrypt_to_ssh(plaintext, pk);

        let result = decrypt_bytes(&ciphertext, sk.as_bytes(), None).unwrap();
        assert_eq!(result, plaintext);
    }

    #[test]
    fn decrypt_bytes_wrong_ssh_key_fails() {
        let pk = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHsKLqeplhpW+uObz5dvMgjz1OxfM/XXUB+VHtZ6isGN";
        let plaintext = b"secret";

        // Use the correct key to encrypt
        let ciphertext = encrypt_to_ssh(plaintext, pk);

        // Use a different (wrong) SSH key to try to decrypt
        let (wrong_identity, _) = generate_keypair();
        let err = decrypt_bytes(&ciphertext, wrong_identity.as_bytes(), None).unwrap_err();
        assert_eq!(err.code, "DECRYPT_FAILED");
    }

    #[test]
    fn decrypt_bytes_rejects_post_quantum_identity() {
        let identity = b"AGE-SECRET-KEY-PQ-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ";
        let err = decrypt_bytes(b"some data", identity, None).unwrap_err();
        assert_eq!(
            err.code, "POST_QUANTUM_NOT_SUPPORTED",
            "PQ identity must be rejected as unsupported before age parsing"
        );
    }

    // ── Plugin recipient (age-plugin-yubikey) tests ──────────────────────
    //
    // These don't have a YubiKey or the `age-plugin-yubikey` binary available,
    // so they can't do a real encrypt round-trip. They *do* prove the two
    // things that matter for correctness here: a plugin recipient is recognized
    // and routed (not misparsed as native x25519, which used to break every
    // write in a store sharing a yubikey recipient), and a missing plugin binary
    // surfaces as the dedicated `PluginUnavailable` error.

    /// Build a valid `age1yubikey1...` recipient encoding (bech32 with the
    /// `age1yubikey` HRP). The data is dummy bytes — `Recipient::from_str` only
    /// validates the HRP/plugin name, not the payload.
    fn yubikey_recipient() -> String {
        bech32::encode(
            "age1yubikey",
            [0u8; 32].to_base32(),
            bech32::Variant::Bech32,
        )
        .expect("bech32 encode of a yubikey recipient")
    }

    #[test]
    fn plugin_recipient_is_recognized_not_misparsed_as_x25519() {
        // The encoding must round-trip through the age plugin parser (pure
        // bech32 — no binary spawned). If this regressed, encrypt_to_recipients
        // would treat it as a native age1 recipient and fail with InvalidIdentity.
        let recipient = yubikey_recipient();
        assert!(recipient.starts_with("age1yubikey1"));
        assert!(
            crate::recipient::is_plugin_recipient(&recipient),
            "constructed yubikey recipient must classify as a plugin recipient"
        );
        assert!(
            age::plugin::Recipient::from_str(&recipient).is_ok(),
            "age must parse the constructed yubikey recipient"
        );
    }

    #[test]
    fn encrypt_to_plugin_recipient_reports_missing_binary() {
        // age-plugin-yubikey is not installed in the test environment, so
        // wrapping to a yubikey recipient cannot proceed. The error must be the
        // dedicated PluginUnavailable (with install guidance), not a generic
        // decrypt/parse failure.
        let recipient = yubikey_recipient();
        let err = encrypt_to_recipients(b"secret", &[recipient]).unwrap_err();
        assert_eq!(
            err.code, "PLUGIN_UNAVAILABLE",
            "missing age-plugin-yubikey binary must surface as PLUGIN_UNAVAILABLE, got: {err}"
        );
    }

    #[test]
    fn encrypt_mixed_native_and_plugin_reports_missing_binary() {
        // A native recipient parses fine; the plugin recipient then fails at
        // binary lookup. Confirms native parsing still happens and the plugin
        // failure is what surfaces.
        let (identity, recipient) = generate_keypair();
        let yubikey = yubikey_recipient();
        let recipients = vec![recipient, yubikey];
        let _ = identity; // unused; we only exercise the encrypt path
        let err = encrypt_to_recipients(b"secret", &recipients).unwrap_err();
        assert_eq!(err.code, "PLUGIN_UNAVAILABLE");
    }

    #[test]
    fn encrypt_groups_multiple_same_plugin_recipients() {
        // Two recipients of the same plugin must be grouped into a single
        // age-plugin-<name> lookup (one subprocess), not one per recipient. With
        // no binary installed the grouped lookup still surfaces a single
        // PLUGIN_UNAVAILABLE — this guards against a regression that spawned N
        // subprocesses (the BTreeMap grouping is the one non-obvious bit of the
        // encrypt path).
        let yk = yubikey_recipient();
        let err = encrypt_to_recipients(b"secret", &[yk.clone(), yk]).unwrap_err();
        assert_eq!(err.code, "PLUGIN_UNAVAILABLE");
    }

    #[test]
    fn encrypt_rejects_post_quantum_recipient_with_dedicated_error() {
        // PQ must keep its own error even though `age1pq1` would otherwise parse
        // as a plugin named `pq`.
        let (_identity, native_recipient) = generate_keypair();
        let err = encrypt_to_recipients(
            b"secret",
            &[
                native_recipient,
                "age1pq1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq".to_string(),
            ],
        )
        .unwrap_err();
        assert_eq!(err.code, "POST_QUANTUM_NOT_SUPPORTED");
    }

    // ── Identity encryption tests ─────────────────────────────────────────

    #[tokio::test]
    async fn encrypt_decrypt_identity_roundtrip() {
        // Serialized: concurrent age-scrypt round-trips intermittently fail
        // with WRONG_PASSPHRASE (a data-race/UB fingerprint, not a codegen
        // miscompilation; root cause unconfirmed). See crate::test_crypto_gate.
        let _crypto = crate::test_crypto_gate::crypto_permit().await;
        let (identity, _recipient) = generate_keypair();
        let passphrase = "correct-horse-battery-staple";

        let encrypted = encrypt_identity(passphrase, identity.as_bytes()).unwrap();
        assert!(
            encrypted.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"),
            "encrypted output should be armored"
        );

        let decrypted = decrypt_identity(passphrase, &encrypted).unwrap();
        assert_eq!(decrypted, identity.as_bytes());
    }

    #[test]
    fn encrypt_identity_rejects_empty_passphrase() {
        let (identity, _recipient) = generate_keypair();
        let err = encrypt_identity("", identity.as_bytes()).unwrap_err();
        assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
    }

    #[test]
    fn decrypt_identity_rejects_empty_passphrase() {
        let encrypted = b"some encrypted data";
        let err = decrypt_identity("", encrypted).unwrap_err();
        assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
    }

    #[tokio::test]
    async fn decrypt_identity_wrong_passphrase() {
        // Serialized: age-scrypt round-trip — see crate::test_crypto_gate.
        let _crypto = crate::test_crypto_gate::crypto_permit().await;
        let (identity, _recipient) = generate_keypair();
        let encrypted = encrypt_identity("correct-passphrase", identity.as_bytes()).unwrap();

        let err = decrypt_identity("wrong-passphrase", &encrypted).unwrap_err();
        assert_eq!(err.code, "WRONG_PASSPHRASE");
    }

    #[test]
    fn decrypt_identity_corrupted_data() {
        let err = decrypt_identity("some-passphrase", b"not-valid-encrypted-data").unwrap_err();
        assert_eq!(err.code, "DECRYPT_FAILED");
    }

    // ── Encrypted SSH key tests ──────────────────────────────────────────

    #[test]
    fn decrypt_bytes_with_encrypted_ssh_key_and_passphrase() {
        let sk = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let pk = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08lcpk06Ast8Z7z7CjjvwJHMnKMjH7";

        let plaintext = b"secret-password\nnotes: encrypted SSH key";
        let ciphertext = encrypt_to_ssh(plaintext, pk);

        let result = decrypt_bytes(&ciphertext, sk.as_bytes(), Some("test-passphrase")).unwrap();
        assert_eq!(result, plaintext);
    }

    #[test]
    fn decrypt_bytes_encrypted_ssh_key_wrong_passphrase() {
        let sk = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let pk = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08lcpk06Ast8Z7z7CjjvwJHMnKMjH7";

        let plaintext = b"secret";
        let ciphertext = encrypt_to_ssh(plaintext, pk);

        let err = decrypt_bytes(&ciphertext, sk.as_bytes(), Some("wrong-passphrase")).unwrap_err();
        assert_eq!(
            err.code, "DECRYPT_FAILED",
            "expected DECRYPT_FAILED for wrong passphrase, got: {err}"
        );
    }

    #[test]
    fn decrypt_bytes_encrypted_ssh_key_no_passphrase() {
        let sk = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let pk = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08lcpk06Ast8Z7z7CjjvwJHMnKMjH7";

        let plaintext = b"secret";
        let ciphertext = encrypt_to_ssh(plaintext, pk);

        let err = decrypt_bytes(&ciphertext, sk.as_bytes(), None).unwrap_err();
        assert_eq!(
            err.code, "IDENTITY_ENCRYPTED",
            "expected IDENTITY_ENCRYPTED for missing passphrase, got: {err}"
        );
    }

    // ── is_ssh_identity_encrypted ───────────────────────────────────────

    /// Unencrypted OpenSSH ed25519 key (aes256-cbc bcrypt KDF absent — `none`).
    const UNENCRYPTED_ED25519_PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\n\
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n\
QyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\n\
agAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBj\n\
AAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n\
1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n\
-----END OPENSSH PRIVATE KEY-----";

    /// Encrypted OpenSSH ed25519 key (aes256-cbc bcrypt KDF present).
    const ENCRYPTED_ED25519_PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\n\
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\n\
c7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\n\
lcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n\
2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\n\
AAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\n\
IJjptSbFpDh+zfEg==\n\
-----END OPENSSH PRIVATE KEY-----";

    #[test]
    fn is_ssh_identity_encrypted_true_for_encrypted_key() {
        assert!(
            is_ssh_identity_encrypted(ENCRYPTED_ED25519_PEM.as_bytes()),
            "a bcrypt-KDF-encrypted SSH key must read as encrypted"
        );
    }

    #[test]
    fn is_ssh_identity_encrypted_false_for_unencrypted_key() {
        assert!(
            !is_ssh_identity_encrypted(UNENCRYPTED_ED25519_PEM.as_bytes()),
            "an unencrypted SSH key must NOT read as encrypted"
        );
    }

    #[test]
    fn is_ssh_identity_encrypted_false_for_non_ssh_and_invalid_utf8() {
        // Non-SSH bytes — not an encrypted SSH key.
        assert!(!is_ssh_identity_encrypted(b"AGE-SECRET-KEY-1..."));
        assert!(!is_ssh_identity_encrypted(b"just some password text"));
        assert!(!is_ssh_identity_encrypted(b""));
        // Invalid UTF-8 — must return false, not panic.
        assert!(!is_ssh_identity_encrypted(&[0xff, 0xfe, 0xfd]));
    }
}
