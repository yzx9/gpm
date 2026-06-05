use std::io::Read;
use std::path::Path;

use age::Decryptor;

use crate::error::{AppError, ErrorCode};

/// Decrypt an .age file using the given identity bytes.
/// Returns the raw decrypted bytes. The caller is responsible for zeroizing
/// the identity after calling this function.
pub fn decrypt_file(file_path: &Path, identity_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    let encrypted = std::fs::read(file_path).map_err(|e| {
        AppError::new(
            ErrorCode::IoError,
            format!("Failed to read entry file: {}", e),
        )
    })?;

    decrypt_bytes(&encrypted, identity_bytes)
}

/// Decrypt age-encrypted bytes using the given identity.
pub fn decrypt_bytes(encrypted: &[u8], identity_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    // Parse the identity from buffer — validates AGE-SECRET-KEY-... format
    let identity_file = age::IdentityFile::from_buffer(identity_bytes).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidIdentity,
            "Identity is not valid AGE-SECRET-KEY-... format",
        )
    })?;

    let identities = identity_file.into_identities().map_err(|_| {
        AppError::new(
            ErrorCode::InvalidIdentity,
            "Identity file contains no valid identities",
        )
    })?;

    if identities.is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidIdentity,
            "Identity file contains no valid identities",
        ));
    }

    // Build a decryptor from the age format (armored or binary)
    let decryptor = match Decryptor::new(encrypted) {
        Ok(d) => d,
        Err(_) => {
            return Err(AppError::new(
                ErrorCode::DecryptFailed,
                "Failed to parse encrypted data",
            ))
        }
    };

    // Perform decryption
    let mut output = Vec::new();
    match decryptor.decrypt(identities.iter().map(|i| i.as_ref() as &dyn age::Identity)) {
        Ok(mut reader) => {
            if reader.read_to_end(&mut output).is_err() {
                return Err(AppError::new(
                    ErrorCode::DecryptFailed,
                    "Decryption failed — wrong identity or corrupted data",
                ));
            }
        }
        Err(_) => {
            return Err(AppError::new(
                ErrorCode::DecryptFailed,
                "Decryption failed — wrong identity or corrupted data",
            ));
        }
    }

    Ok(output)
}
