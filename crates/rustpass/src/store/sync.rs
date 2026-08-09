// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::future::Future;
use std::str;
use std::sync::atomic::Ordering;

use zeroize::Zeroizing;

use crate::RepoLock;
use crate::config::RepoConfig;
use crate::error::{Error, ErrorCode};
use crate::signing::AuthenticityConfig;
use crate::storage::{
    CancelSlot, CancelToken, GitAuth, KeepLocalOutcome, KeepLocalPlan, ProgressSender, RepoFiles,
    StorageCtx,
};

// Impl-split submodule: mod.rs is the shared scope for Store's split impl, so a
// super-glob is the idiomatic import (pedantic flags it; scoped allow).
#[allow(clippy::wildcard_imports)]
use super::*;

impl Store {
    /// Wrap a local-only write in the per-device autosync policy. This is the
    /// sole production write entry point: it holds the Store-wide critical
    /// section across pull → write → push so two in-flight saves can't race the
    /// git index and a manual pull/push/resolve can't interleave with a save.
    ///
    /// - **autosync off** (per-device `repo.json` flag): run `local_write` only
    ///   — a local commit, zero network. The change publishes on the next manual
    ///   Sync.
    /// - **autosync on** (the default): pull (cancellable via `cancel`) → run
    ///   `local_write` → push. A pre-write pull that **diverged** is benign
    ///   (local-ahead is common after any unpushed commit; the write still lands
    ///   on HEAD and the push decides). Only an Enforce authenticity block
    ///   aborts, and it does so before the write runs, so the repo is untouched.
    ///   The push is cancellable best-effort (the sideband callback aborts on the
    ///   cancel token; the bulk-upload window has no checkpoint, so a fast push may
    ///   complete before the abort). A `PUSH_REJECTED` is a real
    ///   divergence; a network failure leaves the local commit in place to sync
    ///   later.
    ///
    /// `local_write` must be one of the local-only primitives ([`set`] /
    /// [`delete`] / [`create`] / [`update`]) — it runs inside the critical
    /// section and must NOT re-acquire [`write_mu`] (those primitives don't).
    ///
    /// `expected`, when `Some`, carries the entry's blob oid captured at read
    /// time (RFC R026). After the pre-write pull settles HEAD, the orchestrator
    /// compares the entry's current oid against `base_oid` and refuses the write
    /// on mismatch — [`WriteOutcome::EntryConflict`] (edit or delete vs. a changed
    /// entry, or create vs. a name a teammate took first) /
    /// [`WriteOutcome::NoChange`] (delete vs. an entry a teammate already removed)
    /// — instead of silently fast-forwarding over the teammate's change. `None`
    /// (preset create / no captured base) skips it; a custom create passes `Some`
    /// for an existence-based guard. The guard runs only under autosync-on.
    ///
    /// # Errors
    ///
    /// Non-terminal outcomes are returned as [`WriteOutcome`] variants, not
    /// `Err`: [`WriteOutcome::AuthenticityBlocked`] when Enforce blocks the
    /// pre-write pull (HEAD unchanged), [`WriteOutcome::NeedsDivergenceResolve`]
    /// when the push is rejected (real divergence — the UI resolves via
    /// [`Self::resolve_sync_divergence`]). `Err` is a pull/push network error
    /// (the local commit survives, syncs later) or whatever `local_write`
    /// returns. [`WriteOutcome::Written`] is the normal success.
    pub async fn autosync_write<F, Fut>(
        &self,
        slot: &CancelSlot,
        cancel: Option<CancelToken>,
        expected: Option<ExpectedEntry>,
        local_write: F,
    ) -> Result<WriteOutcome, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<WriteResult, Error>>,
    {
        // One critical section across pull → write → push. `set`/`delete` (the
        // local-only primitives the closure calls) do NOT re-acquire this guard.
        let _guard = self.write_mu.lock().await;
        let _repo_lock = self.repo_lock()?;

        let autosync = self.autosync.load(Ordering::Relaxed);
        if !autosync {
            return local_write().await.map(WriteOutcome::Written);
        }

        // Arm the cancel slot UNDER the lock so `cancel_git` targets THIS op, not
        // one queued behind `write_mu`. Covers the pull + push
        // network phases; the guard disarms (clears the slot) when it drops.
        let _armed = cancel
            .as_ref()
            .map(|t| ArmedSlot::arm(slot.clone(), t.clone()));

        // Pull (cancellable). Divergence is benign — proceed and let the push
        // decide. Only an Enforce block aborts, before the write touches anything.
        // A cancel here aborts before the local write, so nothing is committed.
        match self.sync_with_locked(cancel.clone(), None).await {
            Ok(SyncOutcome::FastForwarded(result)) if result.authenticity.blocked => {
                return Ok(WriteOutcome::AuthenticityBlocked(result.authenticity));
            }
            Ok(_) => {}
            Err(e) if e.code == "CANCELLED" => {
                return Ok(WriteOutcome::Cancelled { committed: false });
            }
            Err(e) => return Err(e),
        }

        // Base-version guard (RFC R026): when the caller carries an expected
        // entry, refuse the write if it would silently clobber a teammate's
        // change. Edit/delete compare the captured base oid against the current
        // oid at HEAD (settled onto the remote by the pull above); create uses an
        // existence check (a teammate creating the same name first). `None` (no
        // captured base / unguarded caller) skips the check, preserving the
        // legacy path.
        if let Some(expected) = expected {
            let ExpectedEntry {
                name,
                base_oid,
                kind,
            } = expected;
            let current = self.entry_oid(&name).await?;
            // delete vs. an entry a teammate already removed: nothing to commit.
            // Distinct from `Written` so the UI toasts "already removed", not a
            // fake delete commit.
            if matches!(kind, ExpectedKind::Delete) && current.is_none() {
                let head = self
                    .current_head_hash()
                    .await?
                    .chars()
                    .take(7)
                    .collect::<String>();
                return Ok(WriteOutcome::NoChange { head });
            }
            // create vs. a name a teammate already took (existence-based — a
            // brand-new entry has no read-time base to compare): refuse so the
            // user overwrites deliberately or keeps the existing one.
            let conflict = if matches!(kind, ExpectedKind::Create) {
                current.is_some()
            } else {
                current.as_deref() != Some(base_oid.as_str())
            };
            if conflict {
                let remote_tip = self.current_head_hash().await?;
                return Ok(WriteOutcome::EntryConflict {
                    name,
                    base_oid,
                    current_oid: current,
                    remote_tip,
                    op: kind,
                });
            }
        }

        // Local write (encrypt + commit), inside the critical section.
        let result = local_write().await?;

        // Push (cancellable). A cancel here leaves the already-made local commit
        // on disk to publish on the next sync (`committed: true`). A PUSH_REJECTED
        // is a real divergence — surface it with a fresh preview. A network error
        // also leaves the local commit to sync later.
        match self.push_locked(cancel.clone(), None).await {
            Ok(()) => Ok(WriteOutcome::Written(result)),
            Err(e) if e.code == "CANCELLED" => Ok(WriteOutcome::Cancelled { committed: true }),
            Err(e) if e.code == "PUSH_REJECTED" => {
                log::warn!("autosync: push rejected, surfacing divergence");
                Ok(WriteOutcome::NeedsDivergenceResolve(
                    self.sync_divergence_preview(cancel.clone()).await?,
                ))
            }
            Err(e) => Err(e),
        }
    }

    /// Resolve a [`SyncOutcome::Diverged`] with the user's [`DivergenceChoice`].
    ///
    /// - [`DivergenceChoice::AdoptRemote`] adopts the reviewed remote tip exactly
    ///   (delegating to the storage backend).
    /// - [`DivergenceChoice::KeepMine`] re-encrypts the local-only `.age` entries
    ///   onto the reviewed remote tip (with the current recipient set) and pushes
    ///   (see [`Self::resolve_keep_mine`]).
    ///
    /// "Cancel" is client-side (the frontend just doesn't call this). Carries no
    /// plaintext across the call boundary — for "keep mine" the local blobs are
    /// decrypted in-process, used to re-encrypt, and dropped.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::PullFfFailed`] if the remote moved past the reviewed
    /// tip; [`ErrorCode::PushRejected`] for an irreconcilable same-secret
    /// conflict or an undecryptable local entry under "keep mine"; or a
    /// git/signing error otherwise. Under Enforce, an authenticity block returns
    /// `Ok` with [`SyncResult::authenticity`] `.blocked = true` (HEAD unchanged).
    pub async fn resolve_sync_divergence(
        &self,
        slot: &CancelSlot,
        expected_remote_oid: &str,
        choice: DivergenceChoice,
        cancel: Option<CancelToken>,
    ) -> Result<SyncResult, Error> {
        let _guard = self.write_mu.lock().await;
        let _repo_lock = self.repo_lock()?;
        // Arm under the lock so a cancel during the keep-mine push (the
        // DivergenceModal "Cancel push" affordance) targets this resolve.
        let _armed = cancel
            .as_ref()
            .map(|t| ArmedSlot::arm(slot.clone(), t.clone()));
        self.resolve_sync_divergence_locked(expected_remote_oid, choice, cancel)
            .await
    }

    /// Lock-free inner of [`resolve_sync_divergence`] (see [`sync_with_locked`]).
    async fn resolve_sync_divergence_locked(
        &self,
        expected_remote_oid: &str,
        choice: DivergenceChoice,
        cancel: Option<CancelToken>,
    ) -> Result<SyncResult, Error> {
        match choice {
            DivergenceChoice::AdoptRemote => {
                let rcs = self.rcs_ctx().await?;
                let expected = expected_remote_oid.to_string();
                self.storage()?
                    .adopt_remote(&rcs.ctx(), &expected, cancel.clone())
                    .await
            }
            DivergenceChoice::KeepMine => self.resolve_keep_mine(expected_remote_oid, cancel).await,
        }
    }

    /// "Keep mine" divergence resolution ([`DivergenceChoice::KeepMine`]):
    /// re-encrypt the local-only `.age` entries onto the reviewed remote tip and
    /// push, preserving local changes with the **current** recipient set (so a
    /// remote recipient-list change is honored — not a stale-recipient rebase).
    ///
    /// Five steps, with crypto kept in `Store` (git stays pure): plan (single
    /// fetch + stale-guard + authenticity-verify + replay/conflict computation)
    /// → decrypt local blobs → advance to the reviewed tip (no second fetch)
    /// → re-encrypt to current recipients → write + commit + push.
    async fn resolve_keep_mine(
        &self,
        expected_remote_oid: &str,
        cancel: Option<CancelToken>,
    ) -> Result<SyncResult, Error> {
        let rcs = self.rcs_ctx().await?;
        let expected = expected_remote_oid.to_string();
        let ext = self.secret_ext()?;

        // 1. Plan: fetch once, stale-guard, authenticity-verify, compute the
        //    replay set + conflict detection. Does NOT move HEAD.
        let plan = match self
            .storage()?
            .keep_local_plan(&rcs.ctx(), &expected, ext, cancel.clone())
            .await?
        {
            KeepLocalOutcome::Blocked(result) => return Ok(result),
            KeepLocalOutcome::Plan(p) => p,
        };
        let KeepLocalPlan {
            fetched_oid,
            replays,
            deletes,
            authenticity,
        } = plan;

        // 2. Decrypt each local blob to plaintext (identity). An undecryptable
        //    local entry can't be re-encrypted → refuse (adopt or cancel rather
        //    than silently drop it). `get_identity_bytes` returns the cached
        //    *unlocked* identity, so this works for passphrase-protected SSH keys
        //    (the PEM is already decrypted); the re-encrypt step (4) reuses it.
        let identity = self.get_identity_bytes().await?;
        let crypto = self.crypto()?;
        let mut decrypted: Vec<(String, Zeroizing<Vec<u8>>)> = Vec::with_capacity(replays.len());
        for r in replays {
            let plaintext = crypto.decrypt(&r.blob, &identity).await.map_err(|_| {
                Error::new(
                    ErrorCode::PushRejected,
                    format!(
                        "Can't keep mine: \"{}\" can't be decrypted to re-encrypt. \
                             Adopt the remote or cancel.",
                        r.rel_path.trim_end_matches(ext.as_str())
                    ),
                )
            })?;
            decrypted.push((r.rel_path, Zeroizing::new(plaintext)));
        }

        // 3. Advance to the reviewed remote tip — reuses the plan's fetched oid
        //    (objects still in the DB), so no second fetch can race past the
        //    reviewed tip and bypass the authenticity check under Enforce.
        let fetched = fetched_oid.clone();
        self.storage()?.keep_local_advance(&fetched).await?;

        // 4. Re-encrypt to the CURRENT (remote-tip) recipients + our own key
        //    (ensureOurKeyID) via the backend. It re-reads the recipients index
        //    and re-derives our recipient per entry — cheap for age, and the
        //    replay set is small. The view binds to the storage backend (the
        //    guard and the read share its owned root).
        let storage = self.storage()?;
        let view = RepoFiles::new(&*storage);
        let mut ciphertexts: Vec<(String, Vec<u8>)> = Vec::with_capacity(decrypted.len());
        for (rel, plaintext) in decrypted {
            let ct = crypto.encrypt(&plaintext, &identity, &view).await?;
            ciphertexts.push((rel, ct));
        }

        // 5. Write the re-encrypted entries, apply local deletes, commit, push.
        let deletes = deletes.clone();
        let head = self
            .storage()?
            .keep_local_finalize(&rcs.ctx(), &ciphertexts, &deletes, cancel, None)
            .await?;

        Ok(SyncResult {
            changed: true,
            head,
            authenticity,
        })
    }

    /// Resolve a [`WriteOutcome::EntryConflict`] (RFC R026): keep-mine or
    /// keep-theirs for an entry whose base version differed from the remote's
    /// current version. Holds the write critical section, re-fetches
    /// (authenticity-checked, like the save pull), and TOCTOU-guards on the
    /// reviewed tip.
    ///
    /// `KeepMine` re-sends the caller's edit ([`Self::set`], re-encrypted with the
    /// cached identity) or removes the entry ([`Self::delete`]), then pushes.
    /// `KeepTheirs` is a guarded no-op — local HEAD already sits at the reviewed tip,
    /// so confirming the tip suffices ([`Self::adopt_remote`] is NOT called: it
    /// re-fetches and could race past the reviewed tip). Unlike
    /// [`Self::resolve_keep_mine`] there are no local-only entries to replay — the
    /// save refused to write.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::PullFfFailed`] when the remote moved again between the conflict
    /// and the resolve (the TOCTOU guard), or the repository diverged unexpectedly.
    /// An Enforce block on the re-fetch returns `Ok` with `authenticity.blocked`
    /// (HEAD unchanged, nothing committed) — mirroring [`Self::resolve_keep_mine`].
    #[allow(clippy::too_many_arguments)] // slot/cancel + entry (name, content) + decision (tip, kind, choice)
    pub async fn resolve_entry_conflict(
        &self,
        slot: &CancelSlot,
        name: &str,
        content: Option<&[u8]>,
        expected_remote_oid: &str,
        kind: ExpectedKind,
        choice: EntryConflictChoice,
        cancel: Option<CancelToken>,
    ) -> Result<SyncResult, Error> {
        let _guard = self.write_mu.lock().await;
        let _repo_lock = self.repo_lock()?;
        // Arm under the lock so a cancel during the keep-mine push targets this
        // resolve, not one queued behind write_mu (mirrors autosync_write).
        let _armed = cancel
            .as_ref()
            .map(|t| ArmedSlot::arm(slot.clone(), t.clone()));

        // Re-fetch + authenticity (mirrors the save pull). An Enforce block refuses
        // the resolve (HEAD unchanged, nothing committed) and surfaces the block.
        let pull = match self.sync_with_locked(cancel.clone(), None).await? {
            SyncOutcome::FastForwarded(r) => r,
            // Local diverged from the remote — impossible in normal flow (the
            // conflict left local at the reviewed tip; writes are serialized by
            // write_mu). Treat it as a moved remote so the UI re-prompts rather
            // than acting blind.
            SyncOutcome::Diverged(_) => {
                return Err(Error::new(
                    ErrorCode::PullFfFailed,
                    "Entry conflict resolve: the repository diverged — re-check and retry",
                ));
            }
        };
        if pull.authenticity.blocked {
            return Ok(pull);
        }

        // TOCTOU guard: the remote tip must still be the one the user reviewed. The
        // fetch above fast-forwarded local HEAD onto the current remote; if that
        // differs from the reviewed tip, the remote moved again — refuse (the UI
        // re-prompts, mirroring useDivergence's recovery).
        let tip = self.current_head_hash().await?;
        if tip != expected_remote_oid {
            return Err(Error::new(
                ErrorCode::PullFfFailed,
                "Entry conflict resolve: the remote changed again since you reviewed it",
            ));
        }

        match choice {
            EntryConflictChoice::KeepTheirs => {
                // Local HEAD is already at the reviewed tip (the conflict's pull put
                // it there; the fetch + tip-guard confirmed it hasn't moved). No-op.
                Ok(SyncResult {
                    changed: false,
                    ..pull
                })
            }
            EntryConflictChoice::KeepMine => {
                // Each primitive returns the short hash of the commit it just made;
                // reuse it instead of re-reading HEAD (push_locked sends refs without
                // moving local HEAD, so the re-read would return the same commit).
                let written = match kind {
                    ExpectedKind::Edit => {
                        let content = content.ok_or_else(|| {
                            Error::new(
                                ErrorCode::StoreError,
                                "Entry conflict keep-mine (edit) requires the edited content",
                            )
                        })?;
                        self.set(name, content).await?
                    }
                    ExpectedKind::Create => {
                        let content = content.ok_or_else(|| {
                            Error::new(
                                ErrorCode::StoreError,
                                "Entry conflict keep-mine (create) requires the new content",
                            )
                        })?;
                        // `create` (not `set`) so the same template that shaped
                        // the original attempt applies on the overwrite.
                        self.create(name, content).await?
                    }
                    ExpectedKind::Delete => self.delete(name).await?,
                };
                // The keep-mine already committed locally (`written.commit`). A push
                // failure here would strand that commit: returning the raw error
                // leaves the modal retryable, but a retry's tip-guard would compare
                // the moved local HEAD against the reviewed tip and misfire ("remote
                // changed" — it didn't; the local moved). So map EVERY push failure
                // to PullFfFailed (terminal — the UI drops the modal and re-checks
                // from the list), mirroring autosync_write's push-rejection handling.
                // The stranded commit self-heals: the list's foreground sync pulls +
                // pushes, publishing it (or surfacing a clean divergence). This
                // matches autosync_write, which likewise strands a committed write on
                // a non-rejection push error and recovers it on the next save/sync.
                if let Err(e) = self.push_locked(cancel.clone(), None).await {
                    return Err(if e.code == "PUSH_REJECTED" {
                        Error::new(
                            ErrorCode::PullFfFailed,
                            "Entry conflict resolve: the remote changed again before the push",
                        )
                    } else {
                        // Network/auth/etc.: the change IS saved locally — Sync to
                        // publish it. Keep the original error in the message.
                        Error::new(
                            ErrorCode::PullFfFailed,
                            format!(
                                "Entry conflict resolve: your change is saved locally \
                                 — Sync to publish it (push failed: {e})"
                            ),
                        )
                    });
                }
                Ok(SyncResult {
                    changed: true,
                    head: written.commit,
                    authenticity: pull.authenticity,
                })
            }
        }
    }

    /// Compute the local-vs-remote divergence preview on demand, WITHOUT moving
    /// the working branch. Called by the write path after a push rejection (where
    /// divergence is known to be real) so the app can surface the resolution
    /// modal without a separate sync round-trip.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured or the fetch fails.
    pub async fn sync_divergence_preview(
        &self,
        cancel: Option<CancelToken>,
    ) -> Result<SyncDivergence, Error> {
        let rcs = self.rcs_ctx().await?;
        let ext = self.secret_ext()?;
        self.storage()?
            .preview_divergence(&rcs.ctx(), ext, cancel)
            .await
    }

    /// Acquire the cross-process repo lock. Non-blocking; on contention
    /// returns [`ErrorCode::RepoBusy`] so a best-effort sync caller can skip
    /// rather than race another `Store` instance / process on the git index.
    /// The lock auto-releases on drop and on process death (no stale-lockfile).
    /// Mutating callers hold `write_mu` first (called right after acquiring it);
    /// read-only callers such as [`create_bundle`](Self::create_bundle) take
    /// this directly — the lock's contract is cross-process exclusion either
    /// way, and the only contention is cross-instance (a background Worker vs
    /// the foreground app during cold-start overlap).
    pub(super) fn repo_lock(&self) -> Result<RepoLock, Error> {
        RepoLock::try_acquire(self.config.config_dir())
    }

    /// Pull latest changes from the remote (fast-forward only).
    ///
    /// Applies repository-authenticity verification (per the stored
    /// [`AuthenticityConfig`]) before checkout: in Audit mode issues are
    /// reported without blocking, in Enforce mode a blocking issue aborts the
    /// pull leaving HEAD unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured, the remote is
    /// unreachable, the branches have diverged, or Enforce mode refuses the
    /// pull.
    pub async fn sync(&self) -> Result<SyncOutcome, Error> {
        // Plain (non-cancellable) sync: lock + the lock-free inner directly. Does
        // not arm the cancel slot (no caller-facing cancel), so no slot needed.
        let _guard = self.write_mu.lock().await;
        let _repo_lock = self.repo_lock()?;
        self.sync_with_locked(None, None).await
    }

    /// Cancellable, progress-reporting variant of [`sync`](Store::sync).
    ///
    /// `cancel` aborts the in-progress fetch (mapped to [`ErrorCode::Cancelled`]);
    /// `progress` receives transfer stats. The internal pre-push sync of the
    /// write path keeps using the plain [`sync`](Store::sync) (silent,
    /// non-cancellable) — only the user-initiated pull opts in.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured, the remote is
    /// unreachable, the branches have diverged, or Enforce mode refuses the
    /// pull.
    pub async fn sync_with(
        &self,
        slot: &CancelSlot,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<SyncOutcome, Error> {
        let _guard = self.write_mu.lock().await;
        let _repo_lock = self.repo_lock()?;
        // Arm under the lock so `cancel_git` targets this running op, not one
        // queued behind `write_mu` (mirrors autosync_write).
        let _armed = cancel
            .as_ref()
            .map(|t| ArmedSlot::arm(slot.clone(), t.clone()));
        self.sync_with_locked(cancel, progress).await
    }

    /// Lock-free inner of [`sync_with`]. The caller already holds the
    /// [`write_mu`] critical section: [`sync_with`] acquires it for the
    /// standalone pull, and [`autosync_write`] holds it across pull → write →
    /// push and calls this directly.
    async fn sync_with_locked(
        &self,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<SyncOutcome, Error> {
        let rcs = self.rcs_ctx().await?;
        let ext = self.secret_ext()?;
        self.storage()?
            .pull(&rcs.ctx(), ext, cancel, progress)
            .await
    }

    /// Push the current branch to `origin`.
    ///
    /// Used by the create flow's deferred first push — performed after the
    /// identity is durable (via `complete_setup`) so the remote only receives the
    /// store once it can be decrypted locally. A missing `origin` is a no-op
    /// (local-only store), mirroring [`sync`](Store::sync)'s pull no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or the push fails for a
    /// reason other than a missing origin (which is treated as a no-op).
    pub async fn push(&self) -> Result<(), Error> {
        let _guard = self.write_mu.lock().await;
        let _repo_lock = self.repo_lock()?;
        self.push_locked(None, None).await
    }

    /// Lock-free inner of [`push`] (see [`sync_with_locked`]). `cancel`/`progress`
    /// mirror [`sync_with_locked`]; the push is cancellable via the sideband
    /// callback (see [`push`](crate::storage::StorageBackend::push)).
    async fn push_locked(
        &self,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<(), Error> {
        let rcs = self.rcs_ctx().await?;
        self.storage()?.push(&rcs.ctx(), cancel, progress).await
    }

    /// Manual sync (pull → push) — the publish path when autosync is off, and the
    /// "reconcile both directions" action behind the Sync button.
    ///
    /// Acquires [`write_mu`] for the whole pull → push. The pull phase is
    /// cancellable and surfaces [`SyncOutcome::Diverged`] (pull-side divergence)
    /// or an Enforce block (`FastForwarded` with `authenticity.blocked`, HEAD
    /// unchanged) without pushing. If the pull is clean, the push runs; a push
    /// rejection (someone pushed between our pull and our push — a race) is
    /// surfaced as [`SyncOutcome::Diverged`] with a fresh preview. On success the
    /// returned [`SyncResult`] reflects the pull (the push doesn't move local
    /// HEAD); a missing `origin` is a no-op at both phases (local-only store).
    ///
    /// # Errors
    ///
    /// Returns a network error from the pull or push (any local commit survives
    /// to sync later), or whatever [`sync_with_locked`] returns.
    pub async fn sync_repo(
        &self,
        slot: &CancelSlot,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<SyncOutcome, Error> {
        let _guard = self.write_mu.lock().await;
        let _repo_lock = self.repo_lock()?;
        // Arm under the lock so `cancel_git` targets this running op, not one
        // queued behind `write_mu` (mirrors autosync_write).
        let _armed = cancel
            .as_ref()
            .map(|t| ArmedSlot::arm(slot.clone(), t.clone()));

        // Pull (cancellable, progress-reporting). Hand back Diverged / an Enforce
        // block unchanged for the UI to resolve; otherwise keep the pull result
        // for the success return.
        let pull_result = match self
            .sync_with_locked(cancel.clone(), progress.clone())
            .await?
        {
            SyncOutcome::Diverged(d) => return Ok(SyncOutcome::Diverged(d)),
            SyncOutcome::FastForwarded(r) if r.authenticity.blocked => {
                return Ok(SyncOutcome::FastForwarded(r));
            }
            SyncOutcome::FastForwarded(r) => r,
        };

        // Push (cancellable). A PUSH_REJECTED is a real divergence — surface it
        // as Diverged with a fresh preview. A network error leaves any local
        // commits to sync later. Push doesn't move local HEAD, so the pull result
        // still reflects the post-sync state.
        match self.push_locked(cancel.clone(), progress).await {
            Ok(()) => Ok(SyncOutcome::FastForwarded(pull_result)),
            Err(e) if e.code == "PUSH_REJECTED" => {
                log::warn!("sync: push rejected, surfacing divergence");
                Ok(SyncOutcome::Diverged(
                    self.sync_divergence_preview(cancel).await?,
                ))
            }
            Err(e) => Err(e),
        }
    }

    // ── Repository authenticity ───────────────────────────────────────────

    /// Load the current `RepoConfig` and build the per-op RCS context (auth,
    /// policy, commit identity). `RepoConfig` is stable for the op's duration —
    /// every caller runs under `write_mu` (or is a setup path with no
    /// concurrency). The repo path is no longer carried here: the
    /// backend owns it.
    pub(super) async fn rcs_ctx(&self) -> Result<RcsCtx, Error> {
        let repo_config = self.config.load_repo_config().await?;
        Ok(RcsCtx {
            auth: repo_config.to_git_auth(),
            policy: repo_config.authenticity,
            commit_name: repo_config.commit_user_name,
            commit_email: repo_config.commit_user_email,
        })
    }

    /// Set the HTTPS personal access token. `None` (or blank/whitespace) clears
    /// it. Returns the persisted [`RepoConfig`] (the app layer masks the PAT
    /// before it crosses IPC — see `RepoConfigPublic`).
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be loaded or persisted.
    pub async fn set_pat(&self, pat: Option<String>) -> Result<RepoConfig, Error> {
        let pat = pat.and_then(|s| {
            let t = s.trim().to_string();
            (!t.is_empty()).then_some(t)
        });
        let mut rc = self.config.load_repo_config().await?;
        rc.pat = pat;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Remove the stored SSH key + passphrase, clearing SSH auth. A stored PAT,
    /// if any, then becomes the active auth method on the next op (SSH takes
    /// precedence in `to_git_auth` only while the key is present). Returns the
    /// persisted [`RepoConfig`].
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be loaded or persisted.
    pub async fn clear_ssh_key(&self) -> Result<RepoConfig, Error> {
        let mut rc = self.config.load_repo_config().await?;
        rc.ssh_key = None;
        rc.ssh_passphrase = None;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Read-only auth probe: fetch `origin` into a throwaway ref (HEAD untouched)
    /// using `pat` to prove the credential works, without persisting it. Used to
    /// validate a PAT before saving it. Authenticity policy is irrelevant here
    /// (nothing is checked out), so a default policy is used.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::CloneFailed`] on an auth failure, [`ErrorCode::NetworkError`]
    /// on a network problem.
    pub async fn verify_pat(&self, pat: String, cancel: Option<CancelToken>) -> Result<(), Error> {
        // Load the repo config so a no-repo state surfaces here (the backend owns
        // its root now; the loaded config is otherwise unused).
        let _rc = self.config.load_repo_config().await?;
        let auth = GitAuth::Pat(pat);
        let policy = AuthenticityConfig::default();
        let ctx = StorageCtx {
            auth: &auth,
            policy: &policy,
            commit_name: None,
            commit_email: None,
        };
        self.storage()?.verify_auth(&ctx, cancel).await
    }
}
