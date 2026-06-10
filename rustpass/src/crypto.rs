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
/// Supports both native x25519 identities (`AGE-SECRET-KEY-...`) and SSH
/// private keys (OpenSSH or PEM format). Encrypted SSH keys are rejected
/// with an error.
///
/// # Errors
///
/// Returns an error if the identity format is invalid, contains no valid
/// identities, the encrypted data cannot be parsed, or decryption fails.
pub fn decrypt_bytes(encrypted: &[u8], identity_bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let identity_str = std::str::from_utf8(identity_bytes).map_err(|_| {
        Error::new(ErrorCode::InvalidIdentity, "Identity is not valid UTF-8")
    })?;
    let trimmed = identity_str.trim();

    let identities: Vec<Box<dyn age::Identity>> = if trimmed.starts_with("AGE-SECRET-KEY-") {
        // x25519 path
        let identity_file = age::IdentityFile::from_buffer(identity_bytes).map_err(|_| {
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
        let buf = std::io::BufReader::new(trimmed.as_bytes());
        let ssh_identity = age::ssh::Identity::from_buffer(buf, None).map_err(|e| {
            Error::new(
                ErrorCode::InvalidIdentity,
                format!("Cannot parse SSH private key: {e}"),
            )
        })?;

        match ssh_identity {
            age::ssh::Identity::Unencrypted(_) => vec![Box::new(ssh_identity)],
            age::ssh::Identity::Encrypted(_) => {
                return Err(Error::new(
                    ErrorCode::InvalidIdentity,
                    "Encrypted SSH keys are not yet supported as age identities",
                ));
            }
            age::ssh::Identity::Unsupported(u) => {
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

    /// Encrypt `plaintext` to the given SSH recipient string, returning ciphertext.
    fn encrypt_to_ssh(plaintext: &[u8], recipient_str: &str) -> Vec<u8> {
        let recipient: age::ssh::Recipient = recipient_str.parse().unwrap();
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

        let result = decrypt_bytes(&ciphertext, sk.as_bytes()).unwrap();
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

        let result = decrypt_bytes(&ciphertext, sk.as_bytes()).unwrap();
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
        let err = decrypt_bytes(&ciphertext, wrong_identity.as_bytes()).unwrap_err();
        assert_eq!(err.code, "DECRYPT_FAILED");
    }
}
