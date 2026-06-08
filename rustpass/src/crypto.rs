// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::io::Read;
use std::path::Path;

use age::Decryptor;

use crate::error::{Error, ErrorCode};

/// Decrypt an `.age` file using the given identity bytes.
///
/// Returns the raw decrypted bytes. The caller is responsible for zeroizing
/// the identity after calling this function.
///
/// # Errors
///
/// Returns an error if the file cannot be read, the identity format is invalid,
/// or decryption fails.
pub fn decrypt_file(file_path: &Path, identity_bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let encrypted = std::fs::read(file_path).map_err(|e| {
        Error::new(
            ErrorCode::IoError,
            format!("Failed to read entry file: {e}"),
        )
    })?;

    decrypt_bytes(&encrypted, identity_bytes)
}

/// Decrypt age-encrypted bytes using the given identity.
///
/// # Errors
///
/// Returns an error if the identity format is invalid, contains no valid
/// identities, the encrypted data cannot be parsed, or decryption fails.
pub fn decrypt_bytes(encrypted: &[u8], identity_bytes: &[u8]) -> Result<Vec<u8>, Error> {
    // Parse the identity from buffer — validates AGE-SECRET-KEY-... format
    let identity_file = age::IdentityFile::from_buffer(identity_bytes).map_err(|_| {
        Error::new(
            ErrorCode::InvalidIdentity,
            "Identity is not valid AGE-SECRET-KEY-... format",
        )
    })?;

    let identities = identity_file.into_identities().map_err(|_| {
        Error::new(
            ErrorCode::InvalidIdentity,
            "Identity file contains no valid identities",
        )
    })?;

    if identities.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidIdentity,
            "Identity file contains no valid identities",
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

#[cfg(test)]
mod tests {
    use super::*;
    use age::secrecy::ExposeSecret;
    use age::x25519::Identity;
    use std::io::Write;

    /// Generate a random x25519 keypair, returning `(identity, recipient)` strings.
    fn generate_keypair() -> (String, String) {
        let sk = Identity::generate();
        let pk = sk.to_public();
        let identity = sk.to_string().expose_secret().to_string();
        let recipient = pk.to_string();
        (identity, recipient)
    }

    /// Encrypt `plaintext` to the given recipient string, returning ciphertext.
    fn encrypt(plaintext: &[u8], recipient_str: &str) -> Vec<u8> {
        use std::str::FromStr;

        let recipient = age::x25519::Recipient::from_str(recipient_str).unwrap();
        let recipients: Vec<Box<dyn age::Recipient>> = vec![Box::new(recipient)];
        let encryptor =
            age::Encryptor::with_recipients(recipients.iter().map(AsRef::as_ref)).unwrap();
        let mut encrypted = Vec::new();
        let mut writer = encryptor.wrap_output(&mut encrypted).unwrap();
        writer.write_all(plaintext).unwrap();
        writer.finish().unwrap();
        encrypted
    }

    #[test]
    fn decrypt_file_reads_and_decrypts() {
        let (identity, recipient) = generate_keypair();
        let plaintext = b"hunter2\nusername: bob";

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("entry.age");
        let ciphertext = encrypt(plaintext, &recipient);
        std::fs::write(&file_path, &ciphertext).unwrap();

        let result = decrypt_file(&file_path, identity.as_bytes()).unwrap();
        assert_eq!(result, plaintext);

        let bytes_result = decrypt_bytes(&ciphertext, identity.as_bytes()).unwrap();
        assert_eq!(result, bytes_result);
    }

    #[test]
    fn decrypt_file_missing_file() {
        let (identity, _recipient) = generate_keypair();
        let missing = std::path::PathBuf::from("/nonexistent/path/no-such-file.age");

        let err = decrypt_file(&missing, identity.as_bytes()).unwrap_err();
        assert_eq!(
            err.code, "IO_ERROR",
            "expected IO_ERROR for missing file, got: {err}"
        );
    }
}
