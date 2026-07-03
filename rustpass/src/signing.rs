// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Git commit signature extraction, classification, and SSH-sig verification.
//!
//! gpm is read-only and age-only — there is no GPG, no editing. The only
//! signing mechanism this module handles is **SSH-signed commits**
//! (`git commit -S` with `gpg.format = ssh`), verified against a user-managed
//! set of trusted signing public keys. GPG-signed commits classify as
//! [`UnsupportedFormat`](CommitSigStatus::UnsupportedFormat) (signed, but not
//! verifiable by gpm) rather than silently `Unsigned`.
//!
//! This module is pure: it reads from a [`git2::Repository`] and a trust set
//! and produces [`CommitSigStatus`] values. It does **not** know what "Enforce
//! blocks a pull" means as UI — that policy lives in [`crate::git`] and
//! [`crate::store`].
//!
//! Verification reuses the already-present `ssh-key` crate (gpm uses it for
//! SSH-identity key generation and SSH git auth). `ssh_key::PublicKey::verify`
//! over the `"git"` namespace is the load-bearing primitive — no new crypto
//! dependency is added. The feature set already enabled on `ssh-key`
//! (`["ed25519", "encryption", "rand_core", "std"]`) covers `SshSig` parse +
//! `PublicKey::verify` (both gated by `alloc`, implied by `std`).

use std::path::Path;

use git2::{Oid, Repository};
use serde::{Deserialize, Serialize};
use ssh_key::{HashAlg, PublicKey, SshSig};

use crate::error::{Error, ErrorCode};

// ---------------------------------------------------------------------------
// Status model
// ---------------------------------------------------------------------------

/// The verification outcome for a single commit — the vocabulary used by the
/// indicator badge, the popups, and the history screen.
///
/// Severity ordering (drives the indicator colour and Enforce blocking) is:
/// `Verified < UnsupportedFormat < Unsigned < UntrustedKey < BadSignature`,
/// with `Unknown` treated as a (fail-closed) soft issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommitSigStatus {
    /// Signed and the key is in the trusted set.
    Verified {
        /// Fingerprint (`SHA256:…`) of the trusted signer.
        signer_fp: String,
    },
    /// Signed by a key not in the trusted set (and not GPG/unknown-format).
    UntrustedKey {
        /// Fingerprint (`SHA256:…`) of the untrusted signer.
        signer_fp: String,
    },
    /// No `gpgsig` header at all.
    Unsigned,
    /// Header present, SSH armor parsed, but the cryptographic signature does
    /// not validate over the commit object. This is the tampering signal —
    /// treat as the most severe; never ignorable in Enforce.
    BadSignature,
    /// Signed with a format we don't verify (e.g. GPG/PGP armor). A soft
    /// warning: "signed, but not with an SSH key gpm can check".
    UnsupportedFormat {
        /// What format was detected (e.g. `"gpg"`).
        format: String,
    },
    /// Could not classify (corrupt object, read error, unparseable armor).
    /// Surfaced as an unknown problem.
    Unknown,
}

impl CommitSigStatus {
    /// The signer fingerprint, when the status carries one.
    #[must_use]
    pub fn signer_fp(&self) -> Option<&str> {
        match self {
            Self::Verified { signer_fp } | Self::UntrustedKey { signer_fp } => Some(signer_fp),
            _ => None,
        }
    }

    /// Is this a verification problem the user might want to act on?
    /// `Verified` is the only non-issue.
    #[must_use]
    pub fn is_issue(&self) -> bool {
        !matches!(self, Self::Verified { .. })
    }

    /// Can this be dismissed via an [`IgnoredIssue`]? `BadSignature` is never
    /// ignorable (letting a user dismiss a tampered commit in Enforce would gut
    /// the feature); everything else that `is_issue` is ignorable.
    #[must_use]
    pub fn is_ignorable(&self) -> bool {
        !matches!(self, Self::Verified { .. } | Self::BadSignature)
    }

    /// Severity rank: higher = more severe. `Verified` = 0.
    #[must_use]
    pub fn severity(&self) -> u8 {
        match self {
            Self::Verified { .. } => 0,
            Self::UnsupportedFormat { .. } => 1,
            Self::Unsigned => 2,
            Self::Unknown => 3,
            Self::UntrustedKey { .. } => 4,
            Self::BadSignature => 5,
        }
    }
}

// ---------------------------------------------------------------------------
// Authenticity config (persisted; no secrets — public trust anchors)
// ---------------------------------------------------------------------------

/// Tri-state per-repo verification mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VerifyMode {
    /// No verification (today's behaviour; the default).
    #[default]
    Off,
    /// Verify on every pull; pop a warning on mismatch. Pull always succeeds.
    Audit,
    /// Verify on every pull; a blocking issue aborts the pull.
    Enforce,
}

/// A trusted signing public key. Public data — no secret, no Keystore needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedKey {
    /// The OpenSSH public key string (`ssh-ed25519 AAAA… [comment]`).
    pub public_key: String,
    /// Stable identity: `SHA256:<base64>` fingerprint.
    pub fingerprint: String,
    /// User-given label, e.g. `"Alice — laptop"`.
    pub label: String,
    /// HEAD hash when the key was trusted (provenance).
    pub added_at_commit: String,
}

/// A user-dismissed commit issue. Scoped per-commit-hash + per-status — never
/// a blanket "ignore all unsigned commits".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredIssue {
    /// Full commit hash the issue was dismissed for.
    pub commit: String,
    /// The status that was dismissed.
    pub status: CommitSigStatus,
    /// HEAD hash at dismissal time (provenance).
    pub ignored_at_commit: String,
}

/// Persisted authenticity state. Stored as the `authenticity` field of
/// [`crate::config::RepoConfig`] (i.e. inside `repo.json`) — the public trust
/// set rides alongside the repo credentials. Omitted from serialization when
/// still default, so users who never enable authenticity see no change to
/// `repo.json`'s shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticityConfig {
    /// Current verification mode.
    #[serde(default)]
    pub mode: VerifyMode,
    /// Trusted signing public keys.
    #[serde(default)]
    pub trusted_keys: Vec<TrustedKey>,
    /// Dismissed commit issues.
    #[serde(default)]
    pub ignored: Vec<IgnoredIssue>,
}

impl AuthenticityConfig {
    /// Whether this config is the all-default (Off, no keys, no ignores).
    /// Used to skip-serialize the field in `RepoConfig` so users who never
    /// enable authenticity see no change to `repo.json`'s shape.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A commit's metadata + verification status (used by sync results & history).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitSigInfo {
    /// Full commit hash.
    pub hash: String,
    /// Short hash (first 7 chars).
    pub short_hash: String,
    /// Author name + email, e.g. `"Alice <alice@example.com>"`.
    pub author: String,
    /// ISO 8601 commit date (with the committer's timezone offset).
    pub date: String,
    /// First line of the commit message.
    pub subject: String,
    /// Verification status.
    pub status: CommitSigStatus,
    /// Whether this status matches a recorded [`IgnoredIssue`] (UI dims it).
    pub ignored: bool,
}

// ---------------------------------------------------------------------------
// Trust set (fingerprints only)
// ---------------------------------------------------------------------------

/// Extract the trusted-key fingerprints from an [`AuthenticityConfig`].
///
/// SSH signatures are **self-contained**: a parsed [`SshSig`] embeds the
/// signer's public key and is verified cryptographically against that embedded
/// key (via [`PublicKey::verify`]). Verification therefore does not need the
/// trusted public keys for the crypto — only their fingerprints, to decide
/// whether the signer's identity is trusted. Matching by fingerprint is
/// matching by key: the fingerprint is SHA-256 of the public-key blob, so two
/// different keys cannot share one.
pub(crate) fn trusted_fingerprints(config: &AuthenticityConfig) -> Vec<String> {
    config
        .trusted_keys
        .iter()
        .map(|k| k.fingerprint.clone())
        .collect()
}

/// Compute the `SHA256:<base64>` fingerprint of an OpenSSH public key string.
///
/// Used when adding a trusted key (validate + derive the stable identity +
/// dedupe) and when trusting a commit's signer.
///
/// # Errors
///
/// Returns [`ErrorCode::SshKeyInvalid`] if the string is not a parseable
/// OpenSSH public key.
pub fn fingerprint_of_public_key(public_key: &str) -> Result<String, Error> {
    let key = PublicKey::from_openssh(public_key).map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Invalid signing public key: {e}"),
        )
    })?;
    Ok(format!("{}", key.fingerprint(HashAlg::Sha256)))
}

/// Extract the signer fingerprint from a parsed SSH signature's embedded key.
fn ssh_sig_fingerprint(ssh_sig: &SshSig) -> String {
    let embedded: PublicKey = ssh_sig.public_key().clone().into();
    format!("{}", embedded.fingerprint(HashAlg::Sha256))
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Detected signature format by armor inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignatureKind {
    /// `-----BEGIN SSH SIGNATURE-----` (verifiable by gpm).
    Ssh,
    /// `-----BEGIN PGP SIGNATURE-----` / `-----BEGIN PGP MESSAGE-----`.
    Gpg,
    /// Some other/unrecognized armor.
    Other,
}

/// Classify a raw signature string by its armor prefix.
fn classify_signature(sig: &str) -> SignatureKind {
    let t = sig.trim();
    if t.contains("BEGIN SSH SIGNATURE") {
        SignatureKind::Ssh
    } else if t.contains("BEGIN PGP SIGNATURE") || t.contains("BEGIN PGP MESSAGE") {
        SignatureKind::Gpg
    } else {
        SignatureKind::Other
    }
}

// ---------------------------------------------------------------------------
// Extraction + verification
// ---------------------------------------------------------------------------

/// Extract the `gpgsig` signature and the signed commit data for a commit.
///
/// Returns `Ok(None)` when the commit is unsigned (no `gpgsig` header).
///
/// # Errors
///
/// Returns an error if the commit object cannot be read (other than
/// "not signed").
fn extract_signature(repo: &Repository, oid: Oid) -> Result<Option<(String, Vec<u8>)>, Error> {
    match repo.extract_signature(&oid, None) {
        Ok((sig_buf, data_buf)) => {
            let sig = String::from_utf8_lossy(&sig_buf).into_owned();
            Ok(Some((sig, data_buf.to_vec())))
        }
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(Error::new(
            ErrorCode::StoreError,
            format!("Failed to read commit signature: {e}"),
        )),
    }
}

/// Compute the verification status of a single commit against a set of
/// trusted-key fingerprints.
///
/// # Errors
///
/// Returns an error only if the commit object cannot be read.
pub fn status_of_commit(
    repo: &Repository,
    oid: Oid,
    trusted_fingerprints: &[String],
) -> Result<CommitSigStatus, Error> {
    let Some((sig, signed_data)) = extract_signature(repo, oid)? else {
        return Ok(CommitSigStatus::Unsigned);
    };

    match classify_signature(&sig) {
        SignatureKind::Ssh => {
            let ssh_sig: SshSig = match sig.parse() {
                Ok(s) => s,
                // Armor present but unparseable — corrupt object.
                Err(_) => return Ok(CommitSigStatus::Unknown),
            };
            let signer_fp = ssh_sig_fingerprint(&ssh_sig);
            let embedded: PublicKey = ssh_sig.public_key().clone().into();

            // Did the commit get tampered with? Verify the cryptographic
            // signature against the key embedded in the signature itself
            // (namespace "git"). PublicKey::verify checks the key matches the
            // one baked into the SshSig, the namespace, and the crypto.
            let crypto_ok = embedded.verify("git", &signed_data, &ssh_sig).is_ok();
            if !crypto_ok {
                return Ok(CommitSigStatus::BadSignature);
            }

            // Crypto is valid — the commit is legitimately signed by
            // `embedded`. Is that key's identity trusted?
            if trusted_fingerprints.iter().any(|fp| fp == &signer_fp) {
                Ok(CommitSigStatus::Verified { signer_fp })
            } else {
                Ok(CommitSigStatus::UntrustedKey { signer_fp })
            }
        }
        SignatureKind::Gpg => Ok(CommitSigStatus::UnsupportedFormat {
            format: "gpg".to_string(),
        }),
        // Header present but unrecognized armor — classify as unknown rather
        // than risking a false BadSignature on something we can't parse.
        SignatureKind::Other => Ok(CommitSigStatus::Unknown),
    }
}

/// Whether `status` matches a recorded [`IgnoredIssue`] for `commit_hash`.
fn is_ignored(commit_hash: &str, status: &CommitSigStatus, ignored: &[IgnoredIssue]) -> bool {
    ignored
        .iter()
        .any(|i| i.commit == commit_hash && &i.status == status)
}

/// Build a full [`CommitSigInfo`] for `oid` (metadata + status + ignored flag).
///
/// # Errors
///
/// Returns an error if the commit or its signature cannot be read.
pub fn commit_sig_info(
    repo: &Repository,
    oid: Oid,
    trusted_fingerprints: &[String],
    ignored: &[IgnoredIssue],
) -> Result<CommitSigInfo, Error> {
    let commit = repo.find_commit(oid)?;
    let hash = oid.to_string();
    let short_hash = short_hash(&hash);

    let author = format_signature(&commit.author());
    let committer_time = commit.committer().when();
    let date = format_iso8601(
        committer_time.seconds(),
        i64::from(committer_time.offset_minutes()),
    );
    let subject = commit
        .summary()
        .unwrap_or("")
        .trim()
        .lines()
        .next()
        .unwrap_or("")
        .to_string();

    let status = status_of_commit(repo, oid, trusted_fingerprints)?;
    let ignored_flag = is_ignored(&hash, &status, ignored);

    Ok(CommitSigInfo {
        hash,
        short_hash,
        author,
        date,
        subject,
        status,
        ignored: ignored_flag,
    })
}

/// Verify every commit in the half-open range `(from, to]` (newest first).
///
/// `to` must be a descendant of `from` (the caller guarantees fast-forward).
/// Commits reachable from `from` are excluded — they predate this pull and are
/// outside verification scope (see the plan's "Verification scope").
///
/// # Errors
///
/// Returns an error if the commit walk or any commit read fails.
pub fn verify_range(
    repo: &Repository,
    from: Oid,
    to: Oid,
    trusted_fingerprints: &[String],
    ignored: &[IgnoredIssue],
) -> Result<Vec<CommitSigInfo>, Error> {
    let mut walk = repo.revwalk()?;
    walk.push(to)?;
    // Exclude `from` and its ancestors — the range is (from, to].
    walk.hide(from)?;
    walk.simplify_first_parent()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL)?;

    let mut out = Vec::new();
    for oid in walk {
        let oid = oid?;
        out.push(commit_sig_info(repo, oid, trusted_fingerprints, ignored)?);
    }
    Ok(out)
}

/// List the `limit` most recent commits reachable from HEAD (newest first),
/// each annotated with its verification status. Used by the `/history` screen.
///
/// # Errors
///
/// Returns an error if HEAD cannot be resolved or the walk fails.
pub fn list_commit_signatures(
    repo: &Repository,
    limit: usize,
    trusted_fingerprints: &[String],
    ignored: &[IgnoredIssue],
) -> Result<Vec<CommitSigInfo>, Error> {
    let head = repo.head()?.peel_to_commit()?.id();

    let mut walk = repo.revwalk()?;
    walk.push(head)?;
    walk.simplify_first_parent()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL)?;

    let mut out = Vec::new();
    for oid in walk {
        if out.len() >= limit {
            break;
        }
        let oid = oid?;
        out.push(commit_sig_info(repo, oid, trusted_fingerprints, ignored)?);
    }
    Ok(out)
}

/// Convenience: status of the current HEAD commit.
///
/// Returns [`CommitSigStatus::Unsigned`] if HEAD has no signature.
///
/// # Errors
///
/// Returns an error if HEAD cannot be resolved or read.
pub fn head_status(
    repo: &Repository,
    trusted_fingerprints: &[String],
) -> Result<CommitSigStatus, Error> {
    let head = repo.head()?.peel_to_commit()?.id();
    status_of_commit(repo, head, trusted_fingerprints)
}

/// The signer's public key embedded in HEAD's SSH signature, as an OpenSSH
/// string — for the "trust this signer" TOFU flow.
///
/// Returns `Ok(None)` when HEAD is unsigned or signed with a non-SSH format
/// (there is no SSH key to trust), or when the signature cannot be parsed.
///
/// # Errors
///
/// Returns an error if HEAD cannot be resolved or its object cannot be read.
/// The signer's public key embedded in a commit's SSH signature, as an OpenSSH
/// string — for the "trust this signer" TOFU flow.
///
/// Returns `Ok(None)` when the commit is unsigned or signed with a non-SSH
/// format (there is no SSH key to trust), or when the signature cannot be
/// parsed.
///
/// # Errors
///
/// Returns an error if the commit object cannot be read.
pub fn signer_public_key(repo: &Repository, oid: Oid) -> Result<Option<String>, Error> {
    let Some((sig, _signed_data)) = extract_signature(repo, oid)? else {
        return Ok(None);
    };
    if classify_signature(&sig) != SignatureKind::Ssh {
        return Ok(None);
    }
    let ssh_sig: SshSig = match sig.parse() {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    let embedded: PublicKey = ssh_sig.public_key().clone().into();
    let openssh = embedded.to_openssh().map_err(|e| {
        Error::new(
            ErrorCode::StoreError,
            format!("Failed to serialize signer key: {e}"),
        )
    })?;
    Ok(Some(openssh))
}

/// The signer's public key of HEAD's SSH signature — convenience wrapper over
/// [`signer_public_key`].
///
/// # Errors
///
/// Returns an error if HEAD cannot be resolved or read.
pub fn head_signer_public_key(repo: &Repository) -> Result<Option<String>, Error> {
    let head = repo.head()?.peel_to_commit()?.id();
    signer_public_key(repo, head)
}

// ---------------------------------------------------------------------------
// Path-based wrappers (keep `git2` out of `store`)
// ---------------------------------------------------------------------------
//
// `store`'s authenticity methods used to open the repo themselves
// (`git2::Repository::discover` + `git2::Oid::from_str`) only to hand a
// `&Repository` to the functions above. These `_at` wrappers take a repo
// *path* and do the discover + hash-parse internally, so `store` no longer
// names `git2` for authenticity. The `&Repository` versions above stay —
// `git` calls them directly (it already has a `Repository` open).

/// Discover the repo at `repo_path` and return HEAD's verification status.
///
/// # Errors
///
/// Returns an error if the repo cannot be discovered or HEAD cannot be read.
pub fn head_status_at(
    repo_path: &Path,
    trusted_fingerprints: &[String],
) -> Result<CommitSigStatus, Error> {
    let repo = Repository::discover(repo_path)?;
    head_status(&repo, trusted_fingerprints)
}

/// Discover the repo at `repo_path` and return HEAD signer's public key.
///
/// # Errors
///
/// Returns an error if the repo cannot be discovered or HEAD cannot be read.
pub fn head_signer_public_key_at(repo_path: &Path) -> Result<Option<String>, Error> {
    let repo = Repository::discover(repo_path)?;
    head_signer_public_key(&repo)
}

/// Discover the repo at `repo_path` and return the signer key of the commit
/// named by `hash` (full or short — resolved via `revparse_single`).
///
/// # Errors
///
/// Returns an error if the repo cannot be discovered or `hash` cannot be
/// resolved to a commit.
pub fn signer_public_key_at(repo_path: &Path, hash: &str) -> Result<Option<String>, Error> {
    let repo = Repository::discover(repo_path)?;
    let oid = repo.revparse_single(hash)?.id();
    signer_public_key(&repo, oid)
}

/// Discover the repo at `repo_path` and return the verification status of the
/// commit named by `hash` (full hash — resolved via `Oid::from_str`, matching
/// the previous store-side call).
///
/// # Errors
///
/// Returns an error if the repo cannot be discovered or `hash` is not a valid
/// full OID.
pub fn status_of_commit_at(
    repo_path: &Path,
    hash: &str,
    trusted_fingerprints: &[String],
) -> Result<CommitSigStatus, Error> {
    let repo = Repository::discover(repo_path)?;
    let oid = Oid::from_str(hash)?;
    status_of_commit(&repo, oid, trusted_fingerprints)
}

/// Discover the repo at `repo_path` and verify the half-open range `(from, to]`
/// (full hashes).
///
/// # Errors
///
/// Returns an error if the repo cannot be discovered, the walk fails, or
/// `from`/`to` are not valid full OIDs.
pub fn verify_range_at(
    repo_path: &Path,
    from: &str,
    to: &str,
    trusted_fingerprints: &[String],
    ignored: &[IgnoredIssue],
) -> Result<Vec<CommitSigInfo>, Error> {
    let repo = Repository::discover(repo_path)?;
    let from = Oid::from_str(from)?;
    let to = Oid::from_str(to)?;
    verify_range(&repo, from, to, trusted_fingerprints, ignored)
}

/// Discover the repo at `repo_path` and return the `limit` most recent commits
/// with per-commit verification status.
///
/// # Errors
///
/// Returns an error if the repo cannot be discovered or HEAD cannot be read.
pub fn list_commit_signatures_at(
    repo_path: &Path,
    limit: usize,
    trusted_fingerprints: &[String],
    ignored: &[IgnoredIssue],
) -> Result<Vec<CommitSigInfo>, Error> {
    let repo = Repository::discover(repo_path)?;
    list_commit_signatures(&repo, limit, trusted_fingerprints, ignored)
}

/// Discover the repo at `repo_path` and return metadata + status for a single
/// commit named by `hash` (full or short — resolved via `revparse_single`).
///
/// # Errors
///
/// Returns an error if the repo cannot be discovered or `hash` cannot be
/// resolved to a commit.
pub fn commit_sig_info_at(
    repo_path: &Path,
    hash: &str,
    trusted_fingerprints: &[String],
    ignored: &[IgnoredIssue],
) -> Result<CommitSigInfo, Error> {
    let repo = Repository::discover(repo_path)?;
    let oid = repo.revparse_single(hash)?.id();
    commit_sig_info(&repo, oid, trusted_fingerprints, ignored)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Short hash = first 7 chars of the full hex hash.
fn short_hash(full: &str) -> String {
    if full.len() >= 7 {
        full[..7].to_string()
    } else {
        full.to_string()
    }
}

/// Format a `git2::Signature` as `"Name <email>"`.
fn format_signature(sig: &git2::Signature<'_>) -> String {
    let name = sig.name().unwrap_or("");
    let email = sig.email().unwrap_or("");
    if email.is_empty() {
        name.to_string()
    } else {
        format!("{name} <{email}>")
    }
}

/// Format a Unix timestamp + UTC-offset-minutes as ISO 8601
/// (`YYYY-MM-DDTHH:MM:SS±HH:MM`).
///
/// Uses Howard Hinnant's `civil_from_days` algorithm — no external date crate.
/// `offset_minutes` is the committer's UTC offset (e.g. `-480` = UTC-8).
fn format_iso8601(seconds: i64, offset_minutes: i64) -> String {
    // Shift into the committer's local wall-clock time.
    let local = seconds + offset_minutes * 60;
    let days = local.div_euclid(86_400);
    let secs_of_day = local.rem_euclid(86_400);

    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    let sign = if offset_minutes >= 0 { '+' } else { '-' };
    let abs_off = offset_minutes.unsigned_abs();
    let off_hour = abs_off / 60;
    let off_min = abs_off % 60;

    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{sign}{off_hour:02}:{off_min:02}"
    )
}

/// Convert days-since-Unix-epoch to `(year, month, day)` (proleptic Gregorian).
/// Algorithm by Howard Hinnant.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh_key::{Algorithm, LineEnding, PrivateKey, rand_core::OsRng};

    // ── git fixture helpers ───────────────────────────────────────────────

    /// Shared test signature.
    fn test_signature() -> git2::Signature<'static> {
        git2::Signature::new("Alice", "alice@example.com", &git2::Time::new(0, 0))
            .expect("failed to create signature")
    }

    /// Create an empty initial commit (unsigned) on the default branch.
    fn create_empty_commit(repo: &Repository, sig: &git2::Signature<'_>) -> Oid {
        let mut index = repo.index().expect("failed to get index");
        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_id).expect("failed to find tree");
        let parents: &[&git2::Commit<'_>] = &[];
        repo.commit(Some("HEAD"), sig, sig, "initial commit", &tree, parents)
            .expect("failed to create commit")
    }

    /// Create a child commit of `parent` (unsigned) with message `msg`.
    fn create_child_commit(
        repo: &Repository,
        sig: &git2::Signature<'_>,
        parent: Oid,
        msg: &str,
    ) -> Oid {
        let parent_commit = repo.find_commit(parent).expect("parent commit");
        let tree = parent_commit.tree().expect("parent tree");
        repo.commit(Some("HEAD"), sig, sig, msg, &tree, &[&parent_commit])
            .expect("failed to create child commit")
    }

    /// Build a signed child commit of `parent`, signed with `privkey` in the
    /// `"git"` namespace (matching `ssh-keygen -Y sign`).
    ///
    /// Returns the new signed commit's Oid.
    fn create_signed_child(
        repo: &Repository,
        sig: &git2::Signature<'_>,
        parent: Oid,
        msg: &str,
        privkey: &PrivateKey,
    ) -> Oid {
        let parent_commit = repo.find_commit(parent).expect("parent commit");
        let tree = parent_commit.tree().expect("parent tree");

        // Build the unsigned commit content in a buffer (what gets signed).
        let buffer = repo
            .commit_create_buffer(sig, sig, msg, &tree, &[&parent_commit])
            .expect("commit_create_buffer");

        // Sign the buffer in the "git" namespace (sha512, git's default).
        let ssh_sig = privkey
            .sign("git", HashAlg::Sha512, &buffer)
            .expect("ssh-key sign");
        let armor = ssh_sig.to_pem(LineEnding::LF).expect("ssh-sig to_pem");

        // Attach the signature → signed commit object.
        let content = std::str::from_utf8(&buffer).expect("buffer utf8");
        repo.commit_signed(content, &armor, None)
            .expect("commit_signed")
    }

    /// Init a repo and create an initial unsigned commit on the default branch.
    fn repo_with_initial_commit() -> (tempfile::TempDir, Repository, Oid) {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init");
        // Silence libgit2's "default identity unknown" for the empty repo.
        repo.signature().ok();
        let sig = test_signature();
        let head = create_empty_commit(&repo, &sig);
        (dir, repo, head)
    }

    /// Generate an ed25519 keypair for signing in tests.
    fn test_signing_keypair() -> PrivateKey {
        PrivateKey::random(&mut OsRng, Algorithm::Ed25519).expect("keygen")
    }

    /// Drop-in for the removed `parse_trusted_keys`: map trusted keys to their
    /// fingerprints (the only thing verification needs).
    fn fps(keys: &[TrustedKey]) -> Vec<String> {
        keys.iter().map(|k| k.fingerprint.clone()).collect()
    }

    // ── status model unit tests ───────────────────────────────────────────

    #[test]
    fn severity_orders_correctly() {
        assert_eq!(
            CommitSigStatus::Verified {
                signer_fp: String::new()
            }
            .severity(),
            0
        );
        assert_eq!(
            CommitSigStatus::UnsupportedFormat {
                format: String::new()
            }
            .severity(),
            1
        );
        assert_eq!(CommitSigStatus::Unsigned.severity(), 2);
        assert_eq!(CommitSigStatus::Unknown.severity(), 3);
        assert_eq!(
            CommitSigStatus::UntrustedKey {
                signer_fp: String::new()
            }
            .severity(),
            4
        );
        assert_eq!(CommitSigStatus::BadSignature.severity(), 5);
    }

    #[test]
    fn verified_is_not_an_issue() {
        assert!(
            !CommitSigStatus::Verified {
                signer_fp: String::new()
            }
            .is_issue()
        );
    }

    #[test]
    fn bad_signature_is_not_ignorable() {
        // BadSignature must never be dismissable, even though it is an issue.
        assert!(CommitSigStatus::BadSignature.is_issue());
        assert!(!CommitSigStatus::BadSignature.is_ignorable());
    }

    #[test]
    fn soft_issues_are_ignorable() {
        assert!(CommitSigStatus::Unsigned.is_ignorable());
        assert!(CommitSigStatus::Unknown.is_ignorable());
        assert!(
            CommitSigStatus::UntrustedKey {
                signer_fp: String::new()
            }
            .is_ignorable()
        );
        assert!(
            CommitSigStatus::UnsupportedFormat {
                format: String::new()
            }
            .is_ignorable()
        );
    }

    #[test]
    fn signer_fp_carried_by_signed_statuses() {
        assert_eq!(
            CommitSigStatus::Verified {
                signer_fp: "SHA256:x".into()
            }
            .signer_fp(),
            Some("SHA256:x")
        );
        assert_eq!(
            CommitSigStatus::UntrustedKey {
                signer_fp: "SHA256:y".into()
            }
            .signer_fp(),
            Some("SHA256:y")
        );
        assert_eq!(CommitSigStatus::Unsigned.signer_fp(), None);
        assert_eq!(CommitSigStatus::BadSignature.signer_fp(), None);
    }

    #[test]
    fn status_roundtrips_through_serde() {
        let cases = [
            CommitSigStatus::Verified {
                signer_fp: "SHA256:abc".into(),
            },
            CommitSigStatus::UntrustedKey {
                signer_fp: "SHA256:def".into(),
            },
            CommitSigStatus::Unsigned,
            CommitSigStatus::BadSignature,
            CommitSigStatus::UnsupportedFormat {
                format: "gpg".into(),
            },
            CommitSigStatus::Unknown,
        ];
        for status in cases {
            let json = serde_json::to_string(&status).expect("serialize");
            let back: CommitSigStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(status, back, "roundtrip failed for {status:?}: {json}");
        }
    }

    #[test]
    fn verify_mode_default_is_off() {
        assert_eq!(VerifyMode::default(), VerifyMode::Off);
    }

    #[test]
    fn verify_mode_serde_lowercase() {
        assert_eq!(
            serde_json::to_string(&VerifyMode::Audit).unwrap(),
            "\"audit\""
        );
        assert_eq!(
            serde_json::to_string(&VerifyMode::Enforce).unwrap(),
            "\"enforce\""
        );
        assert_eq!(serde_json::to_string(&VerifyMode::Off).unwrap(), "\"off\"");
    }

    #[test]
    fn authenticity_config_default_is_off_empty() {
        let cfg = AuthenticityConfig::default();
        assert_eq!(cfg.mode, VerifyMode::Off);
        assert!(cfg.trusted_keys.is_empty());
        assert!(cfg.ignored.is_empty());
    }

    #[test]
    fn authenticity_config_backward_compat_missing_fields() {
        // An old/partial signing.json should still parse to a valid config.
        let json = "{}";
        let cfg: AuthenticityConfig = serde_json::from_str(json).expect("parse empty");
        assert_eq!(cfg.mode, VerifyMode::Off);
        assert!(cfg.trusted_keys.is_empty());
    }

    // ── classification ────────────────────────────────────────────────────

    #[test]
    fn classify_ssh_signature() {
        let armor = "-----BEGIN SSH SIGNATURE-----\nAAAA\n-----END SSH SIGNATURE-----";
        assert_eq!(classify_signature(armor), SignatureKind::Ssh);
    }

    #[test]
    fn classify_gpg_signature() {
        let armor = "-----BEGIN PGP SIGNATURE-----\n\niQ\n-----END PGP SIGNATURE-----";
        assert_eq!(classify_signature(armor), SignatureKind::Gpg);
    }

    #[test]
    fn classify_gpg_message() {
        let armor = "-----BEGIN PGP MESSAGE-----\ndata-----END PGP MESSAGE-----";
        assert_eq!(classify_signature(armor), SignatureKind::Gpg);
    }

    #[test]
    fn classify_unknown_armor() {
        assert_eq!(classify_signature("not a signature"), SignatureKind::Other);
    }

    // ── fingerprint ───────────────────────────────────────────────────────

    #[test]
    fn fingerprint_of_public_key_works() {
        let pair = test_signing_keypair();
        let pub_str = pair.public_key().to_openssh().expect("pubkey string");
        let fp = fingerprint_of_public_key(&pub_str).expect("fingerprint");
        assert!(
            fp.starts_with("SHA256:"),
            "fingerprint should be SHA256-prefixed: {fp}"
        );
    }

    #[test]
    fn fingerprint_of_public_key_rejects_garbage() {
        assert!(fingerprint_of_public_key("not a key").is_err());
    }

    #[test]
    fn fingerprint_matches_ssh_key_format() {
        let pair = test_signing_keypair();
        let pub_str = pair.public_key().to_openssh().expect("pubkey string");
        let ours = fingerprint_of_public_key(&pub_str).unwrap();
        let theirs = format!("{}", pair.public_key().fingerprint(HashAlg::Sha256));
        assert_eq!(ours, theirs);
    }

    // ── status_of_commit (the core) ───────────────────────────────────────

    #[test]
    fn unsigned_commit_is_unsigned() {
        let (_dir, repo, head) = repo_with_initial_commit();
        let trusted = fps(&[]);
        let status = status_of_commit(&repo, head, &trusted).expect("status");
        assert_eq!(status, CommitSigStatus::Unsigned);
    }

    #[test]
    fn signed_by_trusted_key_is_verified() {
        let (_dir, repo, head) = repo_with_initial_commit();
        let privkey = test_signing_keypair();
        let sig = test_signature();

        let signed_oid = create_signed_child(&repo, &sig, head, "trusted commit", &privkey);

        let pub_str = privkey.public_key().to_openssh().unwrap();
        let fp = fingerprint_of_public_key(&pub_str).unwrap();
        let trusted_key = TrustedKey {
            public_key: pub_str,
            fingerprint: fp.clone(),
            label: "test".into(),
            added_at_commit: String::new(),
        };
        let trusted = fps(&[trusted_key]);

        let status = status_of_commit(&repo, signed_oid, &trusted).expect("status");
        assert_eq!(
            status,
            CommitSigStatus::Verified { signer_fp: fp },
            "commit signed by a trusted key must verify"
        );
    }

    #[test]
    fn signed_by_untrusted_key_is_untrusted() {
        let (_dir, repo, head) = repo_with_initial_commit();
        let signer = test_signing_keypair();
        let sig = test_signature();

        let signed_oid = create_signed_child(&repo, &sig, head, "untrusted commit", &signer);

        // A *different* key is trusted.
        let other = test_signing_keypair();
        let other_pub = other.public_key().to_openssh().unwrap();
        let other_fp = fingerprint_of_public_key(&other_pub).unwrap();
        let trusted = fps(&[TrustedKey {
            public_key: other_pub,
            fingerprint: other_fp,
            label: "other".into(),
            added_at_commit: String::new(),
        }]);

        let status = status_of_commit(&repo, signed_oid, &trusted).expect("status");
        let signer_fp = format!("{}", signer.public_key().fingerprint(HashAlg::Sha256));
        assert_eq!(
            status,
            CommitSigStatus::UntrustedKey { signer_fp },
            "commit signed by an untrusted key must be UntrustedKey, not Verified"
        );
        assert!(status.is_issue());
    }

    #[test]
    fn tampered_signed_commit_is_bad_signature() {
        // Re-create the signed commit but then rewrite its object so the
        // signed data no longer matches the signature → BadSignature.
        let (dir, repo, head) = repo_with_initial_commit();
        let signer = test_signing_keypair();
        let sig = test_signature();
        let signed_oid = create_signed_child(&repo, &sig, head, "signed commit", &signer);

        // Tamper: write a brand-new commit object that reuses the original's
        // signature but over a *different* message. We rebuild via the buffer
        // path so the signature attaches to mismatched content.
        let parent = repo.find_commit(head).unwrap();
        let tree = parent.tree().unwrap();
        let buffer = repo
            .commit_create_buffer(&sig, &sig, "DIFFERENT message", &tree, &[&parent])
            .unwrap();
        // Reuse the *original* signature armor (valid for the old content).
        let original_sig_armor = {
            let (s, _) = repo.extract_signature(&signed_oid, None).unwrap();
            String::from_utf8_lossy(&s).into_owned()
        };
        let tampered = repo
            .commit_signed(
                std::str::from_utf8(&buffer).unwrap(),
                &original_sig_armor,
                None,
            )
            .unwrap();
        // Silence unused-binding lint for `dir` (kept to hold the tempdir).
        let _ = &dir;

        let trusted = fps(&[TrustedKey {
            public_key: signer.public_key().to_openssh().unwrap(),
            fingerprint: fingerprint_of_public_key(&signer.public_key().to_openssh().unwrap())
                .unwrap(),
            label: "signer".into(),
            added_at_commit: String::new(),
        }]);

        let status = status_of_commit(&repo, tampered, &trusted).expect("status");
        assert_eq!(
            status,
            CommitSigStatus::BadSignature,
            "a commit whose signed data was altered must classify as BadSignature"
        );
        assert!(
            !status.is_ignorable(),
            "BadSignature must never be ignorable"
        );
    }

    // ── verify_range ──────────────────────────────────────────────────────

    #[test]
    fn verify_range_walks_new_commits() {
        let (_dir, repo, head) = repo_with_initial_commit();
        let sig = test_signature();
        let child = create_child_commit(&repo, &sig, head, "second");
        let grandchild = create_child_commit(&repo, &sig, child, "third");

        let trusted = fps(&[]);
        let range = verify_range(&repo, head, grandchild, &trusted, &[]).expect("range");
        // (head, grandchild] = {child, grandchild}, newest first.
        assert_eq!(range.len(), 2, "range should contain the two new commits");
        let newest = range.first().expect("newest");
        let oldest = range.get(1).expect("oldest");
        assert_eq!(newest.short_hash, grandchild.to_string()[..7]);
        assert_eq!(oldest.short_hash, child.to_string()[..7]);
        assert_eq!(newest.status, CommitSigStatus::Unsigned);
        assert_eq!(oldest.status, CommitSigStatus::Unsigned);
    }

    #[test]
    fn verify_range_excludes_ancestor_at_from() {
        let (_dir, repo, head) = repo_with_initial_commit();
        let sig = test_signature();
        let child = create_child_commit(&repo, &sig, head, "second");
        // Range (child, child] must be empty.
        let trusted = fps(&[]);
        let range = verify_range(&repo, child, child, &trusted, &[]).expect("range");
        assert!(range.is_empty(), "a degenerate range must be empty");
    }

    #[test]
    fn verify_range_respects_ignore_list() {
        let (_dir, repo, head) = repo_with_initial_commit();
        let sig = test_signature();
        let child = create_child_commit(&repo, &sig, head, "second");
        let child_hash = child.to_string();

        let ignored = vec![IgnoredIssue {
            commit: child_hash,
            status: CommitSigStatus::Unsigned,
            ignored_at_commit: String::new(),
        }];
        let trusted = fps(&[]);
        let range = verify_range(&repo, head, child, &trusted, &ignored).expect("range");
        assert!(
            range.first().expect("entry").ignored,
            "the ignored commit should be flagged ignored"
        );
    }

    // ── list_commit_signatures + head_status ──────────────────────────────

    #[test]
    fn list_commit_signatures_from_head() {
        let (_dir, repo, head) = repo_with_initial_commit();
        let sig = test_signature();
        let child = create_child_commit(&repo, &sig, head, "second");
        let _ = child;

        let trusted = fps(&[]);
        let list = list_commit_signatures(&repo, 50, &trusted, &[]).expect("list");
        assert_eq!(list.len(), 2, "should list both commits");
        assert_eq!(list.first().expect("first").subject, "second");
        assert_eq!(list.get(1).expect("second").subject, "initial commit");
    }

    #[test]
    fn list_commit_signatures_respects_limit() {
        let (_dir, repo, head) = repo_with_initial_commit();
        let sig = test_signature();
        let mut prev = head;
        for i in 0..5 {
            prev = create_child_commit(&repo, &sig, prev, &format!("c{i}"));
        }
        let trusted = fps(&[]);
        let list = list_commit_signatures(&repo, 3, &trusted, &[]).expect("list");
        assert_eq!(list.len(), 3, "limit should cap the list length");
    }

    #[test]
    fn head_status_reflects_head() {
        let (_dir, repo, _head) = repo_with_initial_commit();
        let trusted = fps(&[]);
        let status = head_status(&repo, &trusted).expect("head status");
        assert_eq!(status, CommitSigStatus::Unsigned);
    }

    #[test]
    fn commit_sig_info_populates_metadata() {
        let (_dir, repo, head) = repo_with_initial_commit();
        let trusted = fps(&[]);
        let info = commit_sig_info(&repo, head, &trusted, &[]).expect("info");
        assert_eq!(info.short_hash.len(), 7);
        assert_eq!(info.subject, "initial commit");
        assert!(info.author.contains("Alice"));
        assert!(
            info.date.contains('T'),
            "date should be ISO 8601: {}",
            info.date
        );
        assert_eq!(info.status, CommitSigStatus::Unsigned);
        assert!(!info.ignored);
    }

    // ── format helpers ────────────────────────────────────────────────────

    #[test]
    fn short_hash_truncates_to_seven() {
        assert_eq!(short_hash("abcdef1234567890"), "abcdef1");
    }

    #[test]
    fn short_hash_short_input() {
        assert_eq!(short_hash("abc"), "abc");
    }

    #[test]
    fn iso8601_epoch_utc() {
        // Unix epoch at UTC.
        let s = format_iso8601(0, 0);
        assert_eq!(s, "1970-01-01T00:00:00+00:00");
    }

    #[test]
    fn iso8601_known_instant() {
        // 2000-01-01T00:00:00Z = 946_684_800.
        let s = format_iso8601(946_684_800, 0);
        assert_eq!(s, "2000-01-01T00:00:00+00:00");
    }

    #[test]
    fn iso8601_with_offset() {
        // Same instant, UTC-8 (-480 min): 1999-12-31T16:00:00-08:00.
        let s = format_iso8601(946_684_800, -480);
        assert_eq!(s, "1999-12-31T16:00:00-08:00");
    }

    #[test]
    fn iso8601_positive_offset() {
        // 946_684_800 at UTC+5:30 (+330 min): 2000-01-01T05:30:00+05:30.
        let s = format_iso8601(946_684_800, 330);
        assert_eq!(s, "2000-01-01T05:30:00+05:30");
    }

    #[test]
    fn civil_from_days_known() {
        // 1970-01-01 is day 0.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-01-01 is day 10957.
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        // 2026-06-14 is day 20618.
        assert_eq!(civil_from_days(20_618), (2026, 6, 14));
    }
}
