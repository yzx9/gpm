// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Divergence preview and "keep mine" resolution — the git halves of
//! `Store::resolve_sync_divergence`. Crypto-fused, intentionally: keep-mine
//! hands back ciphertext blobs (never plaintext) for `Store` to decrypt +
//! re-encrypt to the current recipients, and the authenticity checks interleave
//! with git object access mid-plan.
//!
//! Keep-mine boundary (carries ciphertext only — plaintext lives in `Store`
//! between decrypt and re-encrypt):
//!
//! ```text
//!   Store                            storage::git::divergence
//!   -----                            ----------------------
//!   resolve_keep_mine ──── plan ───▶ keep_local_plan       (fetch the reviewed
//!   ◀── KeepLocalPlan {               tip, verify its commit
//!         replays: [ciphertext          range, classify the local-
//!         blobs], deletes, … }          only .age set; return
//!   decrypt each blob (crypto)          CIPHERTEXT blobs + deletes)
//!   re-encrypt to CURRENT recipients
//!   ──── advance ─────────────────▶ keep_local_advance     (move HEAD to the
//!                                                          reviewed tip; no
//!   ──── finalize(ciphertexts, ───▶                         second fetch)
//!         deletes)                  keep_local_finalize    (write the re-encrypted
//!   ◀── new HEAD                                            blobs, apply deletes,
//!                                                           commit, push)
//! ```

use std::collections::HashMap;
use std::path::Component;
use std::path::Path;

use git2::Repository;

use crate::error::{Error, ErrorCode};
use crate::signing::{self, AuthenticityConfig, VerifyMode};
use crate::storage::{
    AuthenticityResult, GitAuth, KeepLocalOutcome, KeepLocalPlan, KeepLocalReplay, SyncDivergence,
    SyncResult,
};

use super::commit::push_current_branch;
use super::{transport, util};

/// Fetch the remote tip and compute the local-vs-remote divergence preview,
/// WITHOUT moving the working branch. Called after a push rejection (the write
/// path knows divergence is real) so the app can surface the resolution modal on
/// demand.
pub(super) fn preview_divergence(
    repo_path: &Path,
    auth: &GitAuth,
) -> Result<SyncDivergence, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    let pre_oid = repo
        .head()
        .ok()
        .and_then(|r| r.target())
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD to compute divergence"))?;
    let (_branch, temp_ref, fetched_oid) = transport::fetch_remote_into_temp(&repo, auth)?;
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };
    let div = divergence_info(&repo, pre_oid, fetched_oid)?;
    cleanup();
    Ok(div)
}

/// Build the divergence preview for a local-vs-remote split: ahead counts plus
/// the full set of local-side tracked-file changes an "adopt remote" would
/// discard/overwrite. Pure git tree diff — no decryption (so identical-plaintext
/// re-encryptions are over-reported as `modified` until a future enhancement).
pub(super) fn divergence_info(
    repo: &Repository,
    local_oid: git2::Oid,
    remote_oid: git2::Oid,
) -> Result<SyncDivergence, Error> {
    let base = repo.merge_base(local_oid, remote_oid)?;
    let local_ahead = count_ahead(repo, local_oid, base)?;
    let remote_ahead = count_ahead(repo, remote_oid, base)?;

    let local_tree = repo.find_commit(local_oid)?.tree()?;
    let remote_tree = repo.find_commit(remote_oid)?.tree()?;
    // diff_tree_to_tree(old=local, new=remote): old_file() is the local side.
    let diff = repo.diff_tree_to_tree(Some(&local_tree), Some(&remote_tree), None)?;

    let mut local_only = Vec::new();
    let mut modified = Vec::new();
    let mut other = Vec::new();
    for delta in diff.deltas() {
        match delta.status() {
            // Present locally, absent remotely → deleted by an adopt.
            git2::Delta::Deleted => {
                if let Some(p) = delta.old_file().path() {
                    classify_loss(p, &mut local_only, &mut other);
                }
            }
            // Present on both sides but differing (incl. rename/copy) → overwritten.
            git2::Delta::Modified | git2::Delta::Renamed | git2::Delta::Copied => {
                if let Some(p) = delta.old_file().path() {
                    classify_loss(p, &mut modified, &mut other);
                }
            }
            // Added remotely (absent locally): adopting remote gains it — not a loss.
            _ => {}
        }
    }

    Ok(SyncDivergence {
        local_ahead,
        remote_ahead,
        remote_tip: remote_oid.to_string(),
        local_only_entries: local_only,
        modified_entries: modified,
        other_changed_files: other,
    })
}

/// Count commits reachable from `tip` but not from `base` (first-parent only).
fn count_ahead(repo: &Repository, tip: git2::Oid, base: git2::Oid) -> Result<usize, Error> {
    let mut walk = repo.revwalk()?;
    walk.push(tip)?;
    walk.hide(base)?;
    walk.simplify_first_parent()?;
    Ok(walk.filter_map(Result::ok).count())
}

/// Classify one local-side file loss for the divergence preview: `.age` files
/// become entry names (suffix stripped) and land in `secrets`; anything else
/// lands in `other` by path.
fn classify_loss(path: &Path, secrets: &mut Vec<String>, other: &mut Vec<String>) {
    let s = path.to_string_lossy().into_owned();
    if is_age_entry(path) {
        secrets.push(s.trim_end_matches(".age").to_string());
    } else {
        other.push(s);
    }
}

/// `.age`-entry changes on one side of a diff vs the base tree: paths the side
/// added/modified (with the side's blob, for replay) and paths it deleted. A
/// rename counts as delete(old) + add(new). Used for BOTH sides of a "keep mine"
/// plan — the local side yields what to replay; the remote side yields the
/// touched-path set for conflict detection (its blobs are unused).
struct AgeDiff {
    /// `(rel_path, blob_bytes)` the side has at `side_oid`.
    changed: Vec<(String, Vec<u8>)>,
    /// Worktree-relative paths the side deleted.
    deleted: Vec<String>,
}

/// Diff `base_tree` → `side_tree` and collect the `.age` changes on the side.
fn age_diff_side(
    repo: &Repository,
    base_tree: &git2::Tree<'_>,
    side_tree: &git2::Tree<'_>,
    side_oid: git2::Oid,
) -> Result<AgeDiff, Error> {
    let mut changed: Vec<(String, Vec<u8>)> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();
    for delta in repo
        .diff_tree_to_tree(Some(base_tree), Some(side_tree), None)?
        .deltas()
    {
        match delta.status() {
            git2::Delta::Added | git2::Delta::Modified | git2::Delta::Copied => {
                if let Some(p) = delta.new_file().path()
                    && is_age_entry(p)
                {
                    let rel = p.to_string_lossy().into_owned();
                    let blob = blob_at_commit(repo, side_oid, &rel).unwrap_or_default();
                    changed.push((rel, blob));
                }
            }
            git2::Delta::Deleted => {
                if let Some(p) = delta.old_file().path()
                    && is_age_entry(p)
                {
                    deleted.push(p.to_string_lossy().into_owned());
                }
            }
            // A rename is delete(old) + add(new).
            git2::Delta::Renamed => {
                if let Some(old) = delta.old_file().path()
                    && is_age_entry(old)
                {
                    deleted.push(old.to_string_lossy().into_owned());
                }
                if let Some(new) = delta.new_file().path()
                    && is_age_entry(new)
                {
                    let rel = new.to_string_lossy().into_owned();
                    let blob = blob_at_commit(repo, side_oid, &rel).unwrap_or_default();
                    changed.push((rel, blob));
                }
            }
            _ => {}
        }
    }
    Ok(AgeDiff { changed, deleted })
}

/// Read the blob content of `rel_path` at `commit_oid`, or `None` if the path
/// is absent from that commit's tree.
fn blob_at_commit(repo: &Repository, commit_oid: git2::Oid, rel_path: &str) -> Option<Vec<u8>> {
    let commit = repo.find_commit(commit_oid).ok()?;
    let tree = commit.tree().ok()?;
    let entry = tree.get_path(Path::new(rel_path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(blob.content().to_vec())
}

/// Whether `path` is an `.age` secret (case-insensitive suffix).
fn is_age_entry(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("age"))
}

/// Defense-in-depth: ensure a worktree-relative path from a git tree diff resolves
/// inside the repo — only `Normal`/`CurDir` components (rejects `..`, leading `/`,
/// Windows drive prefixes). Git rejects `..` in tree entries and gpm validates
/// secret names on write, so this is a backstop; [`keep_local_finalize`] replays
/// paths sourced from a (possibly remote) tree diff, so it asserts containment
/// before any filesystem write/delete, mirroring `Store::assert_within_repo`.
fn rel_within_repo(rel: &str) -> Result<(), Error> {
    let outside = Path::new(rel)
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
    if outside {
        return Err(Error::new(
            ErrorCode::EntryNotFound,
            "Entry path is outside repository",
        ));
    }
    Ok(())
}

/// If a `.age` entry was changed on BOTH sides (an irreconcilable same-secret
/// conflict), return the `PushRejected` error. A local replay collides with ANY
/// remote touch; a local delete collides only with a non-delete remote change
/// (both-deleted is agreement, not a conflict). The caller cleans up before
/// propagating the error.
fn keep_local_conflict(
    replays: &[KeepLocalReplay],
    deletes: &[String],
    remote_touched: &HashMap<String, bool>,
) -> Result<(), Error> {
    for r in replays {
        if remote_touched.contains_key(&r.rel_path) {
            return Err(Error::new(
                ErrorCode::PushRejected,
                format!(
                    "Can't keep mine: \"{}\" changed on both sides. Adopt the remote or cancel.",
                    r.rel_path.trim_end_matches(".age")
                ),
            ));
        }
    }
    for d in deletes {
        if matches!(remote_touched.get(d), Some(false)) {
            return Err(Error::new(
                ErrorCode::PushRejected,
                format!(
                    "Can't keep mine: \"{}\" was deleted locally but changed remotely. \
                     Adopt the remote or cancel.",
                    d.trim_end_matches(".age")
                ),
            ));
        }
    }
    Ok(())
}

/// Compute the "keep mine" plan: fetch the remote tip, refuse if it moved past
/// the reviewed `expected_remote_oid`, verify the remote-only range under the
/// authenticity policy (mirroring `adopt_remote` in [`super::pull`]), then compute
/// which local `.age` entries to replay (re-encrypt) and which to re-delete on the
/// tip. Does NOT move HEAD — the caller decrypts/re-encrypts, then
/// [`keep_local_advance`] + [`keep_local_finalize`] apply it.
///
/// Refuses ([`ErrorCode::PushRejected`]) when a `.age` entry was changed on BOTH
/// sides (an irreconcilable same-secret conflict) — the user must adopt the
/// remote or cancel; gpm never merges `.age` blobs.
///
/// Non-secret local changes (`.age-recipients`, templates) are NOT replayed:
/// "keep mine" adopts the remote's non-secret files verbatim and re-encrypts only
/// secrets onto them. gpm is single-identity today, so local recipient edits do
/// not arise; multi-recipient overwrite-safety is deferred (TODO).
pub(super) fn keep_local_plan(
    repo_path: &Path,
    auth: &GitAuth,
    policy: &AuthenticityConfig,
    expected_remote_oid: &str,
) -> Result<KeepLocalOutcome, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;

    let pre_oid = repo.head().ok().and_then(|r| r.target()).ok_or_else(|| {
        Error::new(
            ErrorCode::PullFfFailed,
            "No HEAD to compute a keep-mine plan",
        )
    })?;

    let (_branch, temp_ref, fetched_oid) = transport::fetch_remote_into_temp(&repo, auth)?;
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };

    // Stale-confirmation guard: keep exactly the tip the user reviewed.
    let expected = git2::Oid::from_str(expected_remote_oid)?;
    if fetched_oid != expected {
        cleanup();
        return Err(Error::new(
            ErrorCode::PullFfFailed,
            "Remote changed since you reviewed the divergence; pull again.",
        ));
    }

    let base = repo.merge_base(pre_oid, fetched_oid)?;
    let mode = policy.mode;

    // Authenticity: verify the remote-only range (base, fetched] — identical to
    // adopt_remote. A block under Enforce stops here with HEAD untouched.
    let (new_commits, open_issues, blocked) = if mode == VerifyMode::Off {
        (Vec::new(), Vec::new(), false)
    } else {
        let trusted = signing::TrustSet::from_config(policy);
        let nc = signing::verify_range(&repo, base, fetched_oid, &trusted, &policy.ignored)?;
        let oi: Vec<_> = nc
            .iter()
            .filter(|c| !c.ignored && c.status.is_issue())
            .cloned()
            .collect();
        let bl = mode == VerifyMode::Enforce && !oi.is_empty();
        (nc, oi, bl)
    };
    if blocked {
        cleanup();
        return Ok(KeepLocalOutcome::Blocked(SyncResult {
            changed: false,
            head: util::short_hash(&pre_oid),
            authenticity: AuthenticityResult {
                mode,
                new_commits,
                open_issues,
                blocked: true,
            },
        }));
    }

    let base_tree = repo.find_commit(base)?.tree()?;
    let local_tree = repo.find_commit(pre_oid)?.tree()?;
    let remote_tree = repo.find_commit(fetched_oid)?.tree()?;

    // Local changes vs base: entries to replay (added/modified) or re-delete.
    let local_diff = age_diff_side(&repo, &base_tree, &local_tree, pre_oid)?;
    let replays: Vec<KeepLocalReplay> = local_diff
        .changed
        .into_iter()
        .map(|(rel_path, blob)| KeepLocalReplay { rel_path, blob })
        .collect();
    let deletes = local_diff.deleted;

    // Remote changes vs base: every `.age` path the remote touched (value = was
    // it a deletion?), for same-secret conflict detection.
    let remote_diff = age_diff_side(&repo, &base_tree, &remote_tree, fetched_oid)?;
    let mut remote_touched: HashMap<String, bool> = HashMap::new();
    for (p, _) in remote_diff.changed {
        remote_touched.insert(p, false);
    }
    for p in remote_diff.deleted {
        remote_touched.insert(p, true);
    }

    // Refuse irreconcilable same-secret conflicts (both sides touched the same
    // `.age` entry). See [`keep_local_conflict`].
    if let Err(e) = keep_local_conflict(&replays, &deletes, &remote_touched) {
        cleanup();
        return Err(e);
    }

    cleanup();
    Ok(KeepLocalOutcome::Plan(KeepLocalPlan {
        fetched_oid: fetched_oid.to_string(),
        replays,
        deletes,
        authenticity: AuthenticityResult {
            mode,
            new_commits,
            open_issues,
            blocked: false,
        },
    }))
}

/// Advance the branch + worktree to the reviewed remote tip (`fetched_oid`),
/// WITHOUT refetching. The fetched commit is still in the object DB (the plan
/// only deleted its temp ref), so this reuses the exact tip the authenticity
/// check ran against — no TOCTOU under Enforce.
pub(super) fn keep_local_advance(repo_path: &Path, fetched_oid: &str) -> Result<(), Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    let branch_name = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot advance"))?
        .to_string();
    let target = git2::Oid::from_str(fetched_oid)?;
    util::advance_branch(&repo, &branch_name, target)
}

/// Apply a "keep mine" plan onto the (already-advanced) remote tip: write the
/// re-encrypted `entries`, apply the local `deletes`, commit on HEAD, and push
/// (now a fast-forward — our commit sits on the reviewed remote tip). Returns the
/// new HEAD short hash. Crypto is done by the caller; this is pure git + IO.
pub(super) fn keep_local_finalize(
    repo_path: &Path,
    auth: &GitAuth,
    entries: &[(String, Vec<u8>)],
    deletes: &[String],
    name: Option<&str>,
    email: Option<&str>,
) -> Result<String, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;

    let mut index = repo.index()?;
    for (rel, ciphertext) in entries {
        rel_within_repo(rel)?;
        let file_path = repo_path.join(rel);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file_path, ciphertext)?;
        index.add_path(Path::new(rel)).map_err(|e| {
            Error::new(ErrorCode::StoreError, format!("Failed to stage {rel}: {e}"))
        })?;
    }
    for rel in deletes {
        rel_within_repo(rel)?;
        let file_path = repo_path.join(rel);
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
        }
        // Tolerate an already-gone index entry: the remote may have deleted it
        // too (both-deleted agreement). remove_path errors on an untracked path.
        let _ = index.remove_path(Path::new(rel));
    }
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let head_oid = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD commit to build on"))?;
    let parent = repo.find_commit(head_oid)?;
    let sig = util::gpm_signature(name, email)?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "Keep local changes (re-encrypted onto remote)",
        &tree,
        &[&parent],
    )?;

    push_current_branch(&repo, auth)?;

    let head = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD after keep-mine commit"))?;
    Ok(util::short_hash(&head))
}
