// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! The sync "read-in" surface: fetch + classify the local-vs-remote relation,
//! verify the new commit range under the authenticity policy, and conditionally
//! fast-forward. `adopt_remote` resolves a divergence by hard-advancing to the
//! reviewed remote tip.

use std::path::Path;

use git2::{FetchOptions, RemoteCallbacks, Repository};

use crate::error::{Error, ErrorCode};
use crate::signing::{self, AuthenticityConfig, VerifyMode};
use crate::storage::{
    AuthenticityResult, CancelToken, GitAuth, ProgressSender, SyncOutcome, SyncResult,
};

use super::{divergence, transport, util};

/// Pull (fetch + fast-forward only merge) from origin.
///
/// Applies repository-authenticity verification according to `policy`:
/// - **[`VerifyMode::Off`]** — today's behaviour: in-place fetch + checkout.
///   Byte-for-byte equivalent to the pre-authenticity pull.
/// - **[`VerifyMode::Audit`]/[`VerifyMode::Enforce`]** — fetch the current
///   branch into a temp ref, verify every commit in `(old HEAD, new HEAD]`,
///   then (Audit) always advance + check out reporting open issues, or
///   (Enforce) refuse to advance when a non-ignored blocking issue remains —
///   HEAD and the working tree stay put.
///
/// Returns a [`SyncOutcome`]: [`SyncOutcome::FastForwarded`] for a normal
/// pull, or [`SyncOutcome::Diverged`] when the branches have diverged — the
/// caller surfaces this for resolution instead of erroring.
///
/// # Errors
///
/// Returns an error if the repository cannot be found, the remote is
/// unreachable, or the branches have diverged (non-fast-forward).
pub(super) fn pull_repo(
    repo_path: &Path,
    auth: &GitAuth,
    policy: &AuthenticityConfig,
    cancel: Option<&CancelToken>,
    progress: Option<&ProgressSender>,
) -> Result<SyncOutcome, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    transport::ensure_https_ca_for_origin(&repo)?;

    // No `origin` → a local-only store (e.g. created with no remote). Pull is a
    // no-op: there is nothing to fetch. This mirrors the push no-op so the
    // gopass-style pre-write sync (`Store::set` → `sync`) never errors on a
    // local-only store. See `push_current_branch` for the matching case.
    let Ok(mut remote) = repo.find_remote("origin") else {
        let head = repo
            .head()
            .ok()
            .and_then(|r| r.target())
            .map_or_else(String::new, |oid| util::short_hash(&oid));
        return Ok(SyncOutcome::FastForwarded(SyncResult {
            changed: false,
            head,
            authenticity: empty_authenticity(policy.mode),
        }));
    };

    let callbacks = transport::build_remote_callbacks(auth, cancel, progress);

    // Off mode: temp-ref fetch + fast-forward (divergence surfaced, not errored).
    if policy.mode == VerifyMode::Off {
        return transport::cancelled_or(pull_off(&repo, &mut remote, callbacks), cancel);
    }

    // Audit / Enforce: verify-before-checkout.
    transport::cancelled_or(pull_verified(&repo, &mut remote, callbacks, policy), cancel)
}

/// An empty authenticity result for a given mode (Off pull, no-op pull).
fn empty_authenticity(mode: VerifyMode) -> AuthenticityResult {
    AuthenticityResult {
        mode,
        new_commits: Vec::new(),
        open_issues: Vec::new(),
        blocked: false,
    }
}

/// How the fetched remote tip relates to the local HEAD. Used by the pull paths
/// (and the sync-divergence preview) to tell the three benign cases apart from a
/// true split, so a strictly-local-ahead repo (unpushed commit, remote unchanged)
/// is a no-op pull rather than a spurious divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchClass {
    /// `fetched == local HEAD` — nothing changed either way.
    Equal,
    /// Remote is strictly ahead of local — a fast-forward pull applies it.
    RemoteAhead,
    /// Local is strictly ahead of remote — a pull is a no-op; a push publishes.
    LocalAhead,
    /// Neither is an ancestor of the other — true divergence, needs resolution.
    Diverged,
}

/// Classify the local-HEAD vs fetched-tip relationship via `graph_descendant_of`
/// in both directions. `Equal` is checked first (it is not a descendant relation).
fn classify_relation(
    repo: &Repository,
    pre: git2::Oid,
    fetched: git2::Oid,
) -> Result<FetchClass, Error> {
    if pre == fetched {
        return Ok(FetchClass::Equal);
    }
    if repo.graph_descendant_of(fetched, pre)? {
        return Ok(FetchClass::RemoteAhead);
    }
    if repo.graph_descendant_of(pre, fetched)? {
        return Ok(FetchClass::LocalAhead);
    }
    Ok(FetchClass::Diverged)
}

/// Off-mode pull: fetch the current branch into a temp ref, then fast-forward
/// (or report divergence). gpm operates on a single default branch, so fetching
/// one branch is correct; using a temp ref (like the verified path) means
/// divergence is detected reliably, and the working branch is never moved
/// speculatively — so a `Diverged` result leaves the repo byte-identical.
fn pull_off(
    repo: &Repository,
    remote: &mut git2::Remote<'_>,
    callbacks: RemoteCallbacks<'_>,
) -> Result<SyncOutcome, Error> {
    let branch_name = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot pull"))?
        .to_string();
    let pre_oid = repo.head().ok().and_then(|r| r.target());

    let temp_ref = format!("refs/gpm/pending/{branch_name}");
    let refspec = format!("+refs/heads/{branch_name}:{temp_ref}");
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    remote.fetch(&[&refspec], Some(&mut fetch_opts), None)?;

    let fetched_oid = repo.refname_to_id(&temp_ref).map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Fetch produced no ref: {e}"),
        )
    })?;
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };

    let Some(pre_oid) = pre_oid else {
        util::advance_branch(repo, &branch_name, fetched_oid)?;
        cleanup();
        return Ok(SyncOutcome::FastForwarded(SyncResult {
            changed: true,
            head: util::short_hash(&fetched_oid),
            authenticity: empty_authenticity(VerifyMode::Off),
        }));
    };

    // Classify the local-vs-remote relationship so a strictly-local-ahead repo
    // (an unpushed commit, remote unchanged) is a no-op pull — not a spurious
    // divergence. Only a true both-sides split surfaces for resolution.
    match classify_relation(repo, pre_oid, fetched_oid)? {
        FetchClass::Equal => {
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: false,
                head: util::short_hash(&fetched_oid),
                authenticity: empty_authenticity(VerifyMode::Off),
            }))
        }
        // Remote is behind us: nothing to fetch. The caller pushes to publish.
        FetchClass::LocalAhead => {
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: false,
                head: util::short_hash(&pre_oid),
                authenticity: empty_authenticity(VerifyMode::Off),
            }))
        }
        FetchClass::Diverged => {
            let div = divergence::divergence_info(repo, pre_oid, fetched_oid)?;
            cleanup();
            Ok(SyncOutcome::Diverged(div))
        }
        FetchClass::RemoteAhead => {
            util::advance_branch(repo, &branch_name, fetched_oid)?;
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: true,
                head: util::short_hash(&fetched_oid),
                authenticity: empty_authenticity(VerifyMode::Off),
            }))
        }
    }
}

/// Verify the `(pre_oid, fetched_oid]` range under `policy`, then — unless
/// Enforce blocks — fast-forward to `fetched_oid`. Returns the [`SyncResult`].
/// The caller owns temp-ref cleanup. (The `RemoteAhead` arm of [`pull_verified`].)
fn verify_and_advance(
    repo: &Repository,
    branch_name: &str,
    pre_oid: git2::Oid,
    fetched_oid: git2::Oid,
    policy: &AuthenticityConfig,
) -> Result<SyncResult, Error> {
    let mode = policy.mode;
    let trusted = signing::TrustSet::from_config(policy);
    let new_commits = signing::verify_range(repo, pre_oid, fetched_oid, &trusted, &policy.ignored)?;
    let open_issues: Vec<_> = new_commits
        .iter()
        .filter(|c| !c.ignored && c.status.is_issue())
        .cloned()
        .collect();

    // Enforce: refuse to advance when a non-ignored blocking issue remains —
    // HEAD and the working tree stay put.
    if mode == VerifyMode::Enforce && !open_issues.is_empty() {
        return Ok(SyncResult {
            changed: false,
            head: util::short_hash(&pre_oid),
            authenticity: AuthenticityResult {
                mode,
                new_commits,
                open_issues,
                blocked: true,
            },
        });
    }

    // Audit (always) or Enforce (no blocking issues): advance + check out.
    util::advance_branch(repo, branch_name, fetched_oid)?;
    Ok(SyncResult {
        changed: true,
        head: util::short_hash(&fetched_oid),
        authenticity: AuthenticityResult {
            mode,
            new_commits,
            open_issues,
            blocked: false,
        },
    })
}

/// Audit/Enforce pull: fetch the current branch into a temp ref, verify the
/// new range, then conditionally advance + check out.
fn pull_verified(
    repo: &Repository,
    remote: &mut git2::Remote<'_>,
    callbacks: RemoteCallbacks<'_>,
    policy: &AuthenticityConfig,
) -> Result<SyncOutcome, Error> {
    let mode = policy.mode;

    // The current branch HEAD sits on (e.g. "main"). gpm always operates on a
    // single default branch; a detached HEAD is unsupported.
    let branch_name = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot pull"))?
        .to_string();

    let pre_fetch_oid = repo.head().ok().and_then(|r| r.target());

    // Fetch the remote branch into a temp ref — bring all new commit objects
    // into the store WITHOUT moving the working branch, so extract_signature
    // works on the fetched tip and its ancestors.
    let temp_ref = format!("refs/gpm/pending/{branch_name}");
    let refspec = format!("+refs/heads/{branch_name}:{temp_ref}");
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    remote.fetch(&[&refspec], Some(&mut fetch_opts), None)?;

    let fetched_oid = repo.refname_to_id(&temp_ref).map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Fetch produced no ref: {e}"),
        )
    })?;

    // Always clean up the temp ref before returning.
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };

    // No prior HEAD (first pull anomaly): advance without verifying a range.
    let Some(pre_oid) = pre_fetch_oid else {
        util::advance_branch(repo, &branch_name, fetched_oid)?;
        cleanup();
        return Ok(SyncOutcome::FastForwarded(SyncResult {
            changed: true,
            head: util::short_hash(&fetched_oid),
            authenticity: AuthenticityResult {
                mode,
                new_commits: Vec::new(),
                open_issues: Vec::new(),
                blocked: false,
            },
        }));
    };

    // Classify before verifying: only a genuine remote-ahead fetch has a range
    // to verify. A local-ahead repo is a no-op pull (the caller pushes); a true
    // split surfaces for resolution. This mirrors `pull_off`.
    match classify_relation(repo, pre_oid, fetched_oid)? {
        FetchClass::Equal => {
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: false,
                head: util::short_hash(&fetched_oid),
                authenticity: AuthenticityResult {
                    mode,
                    new_commits: Vec::new(),
                    open_issues: Vec::new(),
                    blocked: false,
                },
            }))
        }
        FetchClass::LocalAhead => {
            // Remote is behind us: nothing to fetch or verify. Caller pushes.
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: false,
                head: util::short_hash(&pre_oid),
                authenticity: AuthenticityResult {
                    mode,
                    new_commits: Vec::new(),
                    open_issues: Vec::new(),
                    blocked: false,
                },
            }))
        }
        FetchClass::Diverged => {
            let div = divergence::divergence_info(repo, pre_oid, fetched_oid)?;
            cleanup();
            Ok(SyncOutcome::Diverged(div))
        }
        FetchClass::RemoteAhead => {
            let result = verify_and_advance(repo, &branch_name, pre_oid, fetched_oid, policy)?;
            cleanup();
            Ok(SyncOutcome::FastForwarded(result))
        }
    }
}

/// Adopt the remote tip exactly as reviewed (`expected_remote_oid`): re-fetch,
/// refuse if the remote has moved past it, then under the configured
/// authenticity policy verify the remote-only commits and hard-advance the
/// branch to the remote tip. Mirrors `pull_verified` minus the fast-forward
/// guard (we are *resolving* divergence, so we adopt regardless).
///
/// # Errors
///
/// Returns [`ErrorCode::PullFfFailed`] if the remote advanced since the user
/// reviewed the divergence, or a git/signing error otherwise.
pub(super) fn adopt_remote(
    repo_path: &Path,
    auth: &GitAuth,
    policy: &AuthenticityConfig,
    expected_remote_oid: &str,
) -> Result<SyncResult, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    transport::ensure_https_ca_for_origin(&repo)?;

    let branch_name = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot pull"))?
        .to_string();

    let (_branch, temp_ref, fetched_oid) = transport::fetch_remote_into_temp(&repo, auth)?;
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };

    // Stale-confirmation guard: adopt exactly the tip the user reviewed.
    let expected = git2::Oid::from_str(expected_remote_oid)?;
    if fetched_oid != expected {
        cleanup();
        return Err(Error::new(
            ErrorCode::PullFfFailed,
            "Remote changed since you reviewed the divergence; pull again.",
        ));
    }

    let pre_oid = repo.head().ok().and_then(|r| r.target());
    let mode = policy.mode;

    if mode == VerifyMode::Off {
        util::advance_branch(&repo, &branch_name, fetched_oid)?;
        cleanup();
        return Ok(SyncResult {
            changed: pre_oid != Some(fetched_oid),
            head: util::short_hash(&fetched_oid),
            authenticity: empty_authenticity(VerifyMode::Off),
        });
    }

    // Audit/Enforce: verify the remote-only range (merge_base, fetched] — NOT
    // (pre_oid, fetched], which would violate `verify_range`'s descendant
    // contract on a divergence.
    let pre = pre_oid.unwrap_or(fetched_oid);
    let base = repo.merge_base(pre, fetched_oid).unwrap_or(pre);
    let trusted = signing::TrustSet::from_config(policy);
    let new_commits = signing::verify_range(&repo, base, fetched_oid, &trusted, &policy.ignored)?;
    let open_issues: Vec<_> = new_commits
        .iter()
        .filter(|c| !c.ignored && c.status.is_issue())
        .cloned()
        .collect();

    let blocked = mode == VerifyMode::Enforce && !open_issues.is_empty();
    if blocked {
        cleanup();
        return Ok(SyncResult {
            changed: false,
            head: util::short_hash(&pre),
            authenticity: AuthenticityResult {
                mode,
                new_commits,
                open_issues,
                blocked: true,
            },
        });
    }

    util::advance_branch(&repo, &branch_name, fetched_oid)?;
    cleanup();
    Ok(SyncResult {
        changed: true,
        head: util::short_hash(&fetched_oid),
        authenticity: AuthenticityResult {
            mode,
            new_commits,
            open_issues,
            blocked: false,
        },
    })
}

/// Find the default branch name (main or master).
#[cfg(test)]
fn find_default_branch(repo: &Repository) -> Result<String, Error> {
    // Try refs/heads/main first, then refs/heads/master
    for branch in &["main", "master"] {
        if repo.find_reference(&format!("refs/heads/{branch}")).is_ok() {
            return Ok(branch.to_string());
        }
    }

    // Fallback: check what HEAD points to
    if let Ok(head) = repo.head()
        && let Some(name) = head.shorthand()
    {
        return Ok(name.to_string());
    }

    Err(Error::new(
        ErrorCode::PullFfFailed,
        "Cannot determine default branch",
    ))
}

#[cfg(test)]
mod tests {
    use crate::storage::git::commit::init_repo;
    use crate::storage::git::test_support::{
        config_default_branch, create_empty_commit, test_signature,
    };

    use super::*;

    #[test]
    fn pull_repo_noops_without_origin() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        init_repo(dir.path()).unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        let oid = create_empty_commit(&repo, &test_signature());

        let policy = AuthenticityConfig::default();
        let outcome = pull_repo(dir.path(), &GitAuth::None, &policy, None, None)
            .expect("pull must no-op, not error, without origin");
        match outcome {
            SyncOutcome::FastForwarded(r) => {
                assert!(!r.changed, "no-op pull reports no change");
                assert_eq!(r.head, util::short_hash(&oid));
            }
            SyncOutcome::Diverged(d) => panic!("expected FastForwarded no-op, got Diverged: {d:?}"),
        }
    }

    #[test]
    fn find_default_branch_main() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");
        let sig = test_signature();
        let _oid = create_empty_commit(&repo, &sig);

        let expected = config_default_branch(&repo);
        let branch = find_default_branch(&repo).expect("should find a branch");
        assert_eq!(branch, expected);
    }

    #[test]
    fn find_default_branch_master() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");
        let sig = test_signature();
        let oid = create_empty_commit(&repo, &sig);

        let default_branch = config_default_branch(&repo);
        repo.find_reference(&format!("refs/heads/{default_branch}"))
            .expect("should find auto-created ref")
            .delete()
            .expect("failed to delete ref");
        repo.reference("refs/heads/master", oid, false, "test master branch")
            .expect("failed to create master ref");
        repo.set_head("refs/heads/master")
            .expect("failed to set HEAD");

        let branch = find_default_branch(&repo).expect("should find a branch");
        assert_eq!(branch, "master");
    }

    #[test]
    fn find_default_branch_head_fallback() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");
        let sig = test_signature();
        let oid = create_empty_commit(&repo, &sig);

        let default_branch = config_default_branch(&repo);
        repo.find_reference(&format!("refs/heads/{default_branch}"))
            .expect("should find auto-created ref")
            .delete()
            .expect("failed to delete ref");
        repo.reference("refs/heads/develop", oid, false, "test develop ref")
            .expect("failed to create develop ref");
        repo.set_head("refs/heads/develop")
            .expect("failed to set HEAD");

        let branch = find_default_branch(&repo).expect("should find a branch");
        assert_eq!(branch, "develop");
    }
}
