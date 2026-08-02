// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Per-revision content reads for the git backend — the content half of secret
//! revision history (RFC R027). [`blob_at_commit`] reads a secret's ciphertext
//! at a specific commit; it is shared by the keep-mine replay planner
//! ([`super::divergence::age_diff_side`]) and the per-secret revision view
//! ([`StorageBackend::blob_at_revision`](crate::storage::StorageBackend::blob_at_revision)).

use std::path::Path;

use git2::{ObjectType, Oid, Repository};

use crate::error::Error;

/// Read the blob content of `rel_path` at `commit_oid`, or `None` if the path
/// is absent from that commit's tree (e.g. the commit deleted the entry).
///
/// Pure read of an immutable content-addressed object: safe to run while a
/// background sync fast-forwards HEAD — the commit and its blob persist (gpm
/// never GCs), so a revision read can't race a ref move.
pub(crate) fn blob_at_commit(
    repo: &Repository,
    commit_oid: Oid,
    rel_path: &str,
) -> Option<Vec<u8>> {
    let commit = repo.find_commit(commit_oid).ok()?;
    let tree = commit.tree().ok()?;
    let entry = tree.get_path(Path::new(rel_path)).ok()?;
    // `TreeEntry::id()` is the oid of whatever sits at the path (blob, subtree,
    // or gitlink) — guard the kind explicitly. `find_blob` would already reject
    // a non-blob today, but the explicit check defends a future refactor to
    // `find_object` + cast and stops a malicious remote (in-model under sig-
    // verification Off) that plants a subtree at `<name>.age` from being read as
    // content. A non-blob here is treated as "no readable secret at this path".
    if entry.kind() != Some(ObjectType::Blob) {
        return None;
    }
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(blob.content().to_vec())
}

/// Discover the repo at `repo_path` and read `rel_path`'s blob at `commit_oid`,
/// or `None` if the path is absent (a delete-commit). `commit_oid` is a full
/// object id as returned by [`crate::signing::CommitSigInfo::hash`].
///
/// # Errors
///
/// Returns an error if the repo cannot be discovered or `commit_oid` is not a
/// valid full oid (it is not revparsed — the revision listing always supplies a
/// full hash).
pub(crate) fn blob_at_commit_at(
    repo_path: &Path,
    commit_oid: &str,
    rel_path: &str,
) -> Result<Option<Vec<u8>>, Error> {
    let repo = Repository::discover(repo_path)?;
    let oid = Oid::from_str(commit_oid)?;
    Ok(blob_at_commit(&repo, oid, rel_path))
}
