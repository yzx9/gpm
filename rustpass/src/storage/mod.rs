// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Storage / RCS backend abstraction.
//!
//! This module is the future home of the `StorageBackend` trait (mirroring
//! gopass's `Storage` interface, which embeds RCS). It already owns the shared
//! sync / write / commit-identity *result* types that the trait will surface,
//! relocated here from `store` so the upcoming `Store` → `StorageBackend` edge
//! doesn't form a `store ↔ storage` module cycle: the trait (defined here) must
//! be able to return `SyncOutcome` without importing `store`, and the git
//! storage impl must be able to construct it without depending on `store`.
//!
//! `GitAuth` and the progress/cancellation types stay in `git` for now and move
//! here when `git.rs` is folded into `storage/git`.

use serde::{Deserialize, Serialize};

use crate::signing::{CommitSigInfo, VerifyMode};

/// Result of a sync (pull) operation — aligned with gopass `Store.Sync`.
#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    /// Whether any new commits were pulled (HEAD advanced).
    pub changed: bool,
    /// Short hash (7 chars) of the (current) HEAD commit.
    pub head: String,
    /// Repository-authenticity outcome of this pull.
    pub authenticity: AuthenticityResult,
}

/// Authenticity outcome of a sync (pull) — surfaced to the frontend so it can
/// pop the Audit mismatch modal / Enforce-block modal without re-verifying.
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticityResult {
    /// The mode in force during this pull.
    pub mode: VerifyMode,
    /// Commits in the pulled range `(old HEAD, new HEAD]`, newest first.
    /// Empty for `VerifyMode::Off` (no verification is done).
    pub new_commits: Vec<CommitSigInfo>,
    /// Subset of `new_commits` that are non-Verified and not ignored — the
    /// actionable issues.
    pub open_issues: Vec<CommitSigInfo>,
    /// `true` only when Enforce refused checkout (HEAD did not advance).
    pub blocked: bool,
}

/// Outcome of a sync (pull): a normal pull, or a divergence the caller must
/// resolve. Replaces the bare [`SyncResult`] return of [`crate::store::Store::sync`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncOutcome {
    /// Normal fast-forward pull (changed or not).
    FastForwarded(SyncResult),
    /// Local and remote have diverged; the working branch is unchanged. The
    /// caller must resolve via [`crate::store::Store::resolve_sync_divergence`].
    Diverged(SyncDivergence),
}

/// Local-vs-remote divergence, surfaced so the user can decide whether to
/// discard local-only changes and adopt the remote. Carries no secrets — entry
/// *names* / file paths only.
#[derive(Debug, Clone, Serialize)]
pub struct SyncDivergence {
    /// Commits reachable from local HEAD but not the merge base.
    pub local_ahead: usize,
    /// Commits reachable from the remote tip but not the merge base.
    pub remote_ahead: usize,
    /// Full hash of the remote tip this preview was computed against. Passed
    /// back to [`crate::store::Store::resolve_sync_divergence`] so we adopt
    /// exactly what was reviewed (no stale-confirmation TOCTOU).
    pub remote_tip: String,
    /// Secret entries (`.age` stripped) present locally, absent remotely —
    /// **deleted** by "adopt remote".
    pub local_only_entries: Vec<String>,
    /// Secret entries present on both sides whose `.age` bytes differ —
    /// **overwritten** by "adopt remote". (May over-report identical-plaintext
    /// re-encryptions until the decrypt-and-compare enhancement lands.)
    pub modified_entries: Vec<String>,
    /// Non-secret tracked files changed on the local side (templates,
    /// recipients, …) — also discarded/overwritten by a hard reset.
    pub other_changed_files: Vec<String>,
}

/// Result of a successful write (`Store::set`) — the new HEAD commit hash.
#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    /// Short hash (7 chars) of the commit that recorded the write.
    pub commit: String,
}

/// The default commit author identity, surfaced to the UI so it can display
/// what's in effect without hardcoding the value.
#[derive(Debug, Clone, Serialize)]
pub struct CommitIdentity {
    /// Commit author name.
    pub name: String,
    /// Commit author email.
    pub email: String,
}

/// Outcome of a write attempt.
///
/// Writes run through [`crate::store::Store::autosync_write`] (pull → write →
/// push). A normal save returns [`WriteOutcome::Written`]. Two non-terminal
/// outcomes surface a modal instead of a generic error:
/// - [`WriteOutcome::NeedsDivergenceResolve`] — the push was rejected because
///   the remote moved during the write (a race); the carried [`SyncDivergence`]
///   lets the UI show the resolve modal without a second round-trip.
/// - [`WriteOutcome::AuthenticityBlocked`] — the pre-write pull was refused
///   under Enforce signature verification (HEAD did not advance); the carried
///   [`AuthenticityResult`] reuses the pull path's block-issue UI.
///
/// **Limitation (unchanged — see `.plans/0026-edit-base-version-aware.md`):**
/// this only catches the push-rejection *race*. A write built on a stale read
/// can still fast-forward over a newer remote change and push cleanly — no
/// modal — silently overwriting it (recoverable in git history). That limitation
/// is surfaced to the user in Settings, not per-write.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriteOutcome {
    /// The secret was written and committed — and pushed when autosync is on.
    /// Carries the new HEAD.
    Written(WriteResult),
    /// The push was rejected — the remote moved during the write (a race). The
    /// local commit was made; the caller resolves via
    /// [`crate::store::Store::resolve_sync_divergence`] using the carried
    /// preview. Carries no plaintext.
    NeedsDivergenceResolve(SyncDivergence),
    /// The pre-write pull was refused under Enforce authenticity mode (HEAD did
    /// not advance). No local write was made. The caller reuses the pull path's
    /// block-issue UI with the carried result.
    AuthenticityBlocked(AuthenticityResult),
}

/// How to resolve a [`SyncOutcome::Diverged`] (the user's choice). "Cancel" is
/// client-side — the frontend simply doesn't call
/// [`crate::store::Store::resolve_sync_divergence`] — so it is absent here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceChoice {
    /// Discard local-only changes and adopt the reviewed remote tip exactly.
    AdoptRemote,
    /// Keep local changes: re-encrypt the local-only `.age` entries onto the
    /// reviewed remote tip (with the current recipient set) and push. Refused
    /// ([`crate::error::ErrorCode::PushRejected`]) for an irreconcilable
    /// same-secret conflict or an undecryptable local entry — the user must
    /// adopt or cancel.
    KeepMine,
}
