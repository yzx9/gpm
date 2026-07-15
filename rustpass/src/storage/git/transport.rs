// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Platform transport for the git backend: the Android HTTPS CA-bundle
//! workaround, libgit2 credential/progress/cancel callbacks, the shared
//! fetch-into-temp-ref primitive, and libgit2 → [`Error`] classification.
//!
//! Co-located with the only `git2` network call sites. This is the transport
//! leaf: [`commit`](super::commit)/[`pull`](super::pull)/[`divergence`](super::divergence)
//! all pull callbacks, the fetch primitive, and error mapping from here.

use std::sync::atomic::Ordering;

#[cfg(target_os = "android")]
use std::ffi::c_int;

#[cfg(target_os = "android")]
use foreign_types::ForeignType;
use git2::{FetchOptions, RemoteCallbacks, Repository};

use crate::error::{Error, ErrorCode};
use crate::storage::{CancelToken, GitAuth, GitProgress, ProgressSender};

/// Mozilla (curl) root CA bundle, embedded so the libgit2 OpenSSL backend has a
/// trust anchor on targets with no discoverable system CA store — notably
/// Android, where the vendored OpenSSL build cannot read the system keystore and
/// HTTPS git otherwise fails with `SSL certificate is invalid`. Refreshed via
/// the `refresh-ca` just recipe; a unit test guards against a truncated/corrupt
/// bundle shipping.
#[cfg(any(target_os = "android", test))]
const EMBEDDED_CA_BUNDLE: &str = include_str!("../../../data/cacert.pem");

/// Before an HTTPS git op on Android, load the embedded Mozilla roots into
/// libgit2's OpenSSL trust store. No-op on non-Android targets and on non-HTTPS
/// remotes (SSH / local-only stores), so a failed load never blocks those flows.
///
/// `rustpass` owns the `git2`/libgit2 dependency, so it owns this
/// platform-specific transport workaround. The vendored OpenSSL is built
/// `no-stdio` on Android (`openssl-src`), which compiles out `BIO_new_file` —
/// the usual approach (`git2::opts::set_ssl_cert_file`) is impossible there,
/// since libgit2 eagerly calls `SSL_CTX_load_verify_locations` → `BIO_new_file`
/// → NULL → `x509 certificate routines::BIO lib`. Instead the bundle is parsed
/// from memory (`BIO_new_mem_buf` survives `no-stdio`) and each root is added
/// via libgit2's `GIT_OPT_ADD_SSL_X509_CERT` → `X509_STORE_add_cert` (no file
/// BIO). Runs at most once process-wide; see [`ensure_https_ca_loaded`].
#[allow(clippy::unnecessary_wraps)] // returns Err only on the android branch below
pub(super) fn ensure_https_ca_for_origin(repo: &Repository) -> Result<(), Error> {
    #[cfg(target_os = "android")]
    {
        let origin_is_https = repo
            .find_remote("origin")
            .ok()
            .and_then(|r| r.url().map(|u| u.starts_with("https://")))
            .unwrap_or(false);
        if origin_is_https {
            ensure_https_ca_loaded()?;
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = repo;
    }
    Ok(())
}

/// libgit2 option to push a raw X509 root into its OpenSSL trust store. Not
/// exported by name in `libgit2-sys` 0.18.x; the value is the last member of
/// `git_libgit2_opt_t` in the vendored libgit2 1.9.4. [`ensure_https_ca_loaded`]
/// asserts the version, so a `git2` bump that shifts this fails loudly instead
/// of silently corrupting the trust store.
#[cfg(target_os = "android")]
const GIT_OPT_ADD_SSL_X509_CERT: c_int = 45;

/// Cache the one-time in-memory CA load. A second call would make
/// `X509_STORE_add_cert` reject every root as a duplicate, so the `OnceLock`
/// runs the load at most once process-wide and makes repeats a safe no-op
/// returning the cached outcome.
#[cfg(target_os = "android")]
static CA_LOAD_RESULT: std::sync::OnceLock<Result<(), Error>> = std::sync::OnceLock::new();

/// Load [`EMBEDDED_CA_BUNDLE`] into libgit2's trust store, once. Android only.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the bundle parses zero roots or a
/// mid-bundle parse failure would leave a partial trust store.
#[cfg(target_os = "android")]
pub(super) fn ensure_https_ca_loaded() -> Result<(), Error> {
    CA_LOAD_RESULT
        .get_or_init(|| {
            // `add_certs_from_pem` mutates libgit2's process-global trust store,
            // so the `OnceLock` is load-bearing for thread-safety (not just the
            // duplicate-cert correctness noted on `CA_LOAD_RESULT`): it runs the
            // load exactly once process-wide; a repeat returns the cached
            // outcome.
            match add_certs_from_pem(EMBEDDED_CA_BUNDLE.as_bytes()) {
                Ok(n) if n > 0 => Ok(()),
                Ok(_) => Err(Error::new(
                    ErrorCode::StoreError,
                    "embedded CA bundle parsed 0 certificates; HTTPS git cannot verify servers",
                )),
                Err(e) => Err(e),
            }
        })
        .clone()
}

/// Initialize libgit2 (`git2::init` is crate-private) and assert the vendored
/// libgit2 is 1.9.x — the version the [`GIT_OPT_ADD_SSL_X509_CERT`] value is
/// pinned to.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
fn init_libgit2_for_ca_opts() -> Result<(), Error> {
    unsafe {
        libgit2_sys::git_libgit2_init();
        let (mut major, mut minor, mut rev): (c_int, c_int, c_int) = (0, 0, 0);
        let rc = libgit2_sys::git_libgit2_version(&mut major, &mut minor, &mut rev);
        if rc != 0 || !(major == 1 && minor == 9) {
            return Err(Error::new(
                ErrorCode::StoreError,
                format!(
                    "GIT_OPT_ADD_SSL_X509_CERT=45 is pinned to libgit2 1.9.x; got \
                     {major}.{minor}.{rev}"
                ),
            ));
        }
    }
    Ok(())
}

/// Parse every `BEGIN CERTIFICATE` block from a concatenated PEM bundle and call
/// `on_cert` for each. Returns the count parsed.
///
/// `openssl::x509::X509::stack_from_pem` owns the EOF-vs-parse-failure
/// distinction (clean end-of-data → a shorter/empty `Vec`; a corrupt block →
/// `Err`), so a truncated/corrupt bundle surfaces as an error instead of a
/// silent partial trust store. Desktop-runnable (only the `openssl` crate, no
/// libgit2 mutation) so the bundle-validity unit test exercises the real
/// OpenSSL parser instead of a string match.
#[cfg(any(target_os = "android", test))]
fn for_each_cert_in_pem(
    pem: &[u8],
    mut on_cert: impl FnMut(&openssl::x509::X509) -> Result<(), Error>,
) -> Result<usize, Error> {
    let certs = openssl::x509::X509::stack_from_pem(pem).map_err(|e| {
        Error::new(
            ErrorCode::StoreError,
            format!("CA bundle failed to parse: {e}"),
        )
    })?;
    for cert in &certs {
        on_cert(cert)?;
    }
    Ok(certs.len())
}

/// Parse `pem` and add every root to libgit2's OpenSSL trust store via
/// `GIT_OPT_ADD_SSL_X509_CERT` (Android only — desktop finds the system store
/// itself). Returns the count added. Reuses [`for_each_cert_in_pem`] so the
/// parse path is shared with the desktop unit test.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
fn add_certs_from_pem(pem: &[u8]) -> Result<usize, Error> {
    init_libgit2_for_ca_opts()?;
    for_each_cert_in_pem(pem, |cert| {
        // SAFETY: `cert` is a valid X509 parsed from the embedded bundle, and
        // `git_libgit2_opts` is variadic with the X509* as the option arg.
        let rc = unsafe { libgit2_sys::git_libgit2_opts(GIT_OPT_ADD_SSL_X509_CERT, cert.as_ptr()) };
        if rc != 0 {
            return Err(Error::new(
                ErrorCode::StoreError,
                "libgit2 rejected an embedded CA root (GIT_OPT_ADD_SSL_X509_CERT)",
            ));
        }
        Ok(())
    })
}

/// `true` if `cancel` is set, signalling the running git operation to abort.
pub(super) fn cancelled(cancel: Option<&CancelToken>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

/// Map a git2 transfer-progress snapshot onto the serialisable [`GitProgress`].
fn progress_from_transfer(p: &git2::Progress<'_>) -> GitProgress {
    GitProgress {
        total_objects: p.total_objects(),
        received_objects: p.received_objects(),
        indexed_objects: p.indexed_objects(),
        received_bytes: p.received_bytes(),
        total_deltas: p.total_deltas(),
        indexed_deltas: p.indexed_deltas(),
        message: None,
    }
}

/// If `result` failed and the cancel token is set, re-map it to
/// [`ErrorCode::Cancelled`]; otherwise pass it through. Wraps the fetch paths
/// (`pull_off`/`pull_verified`) whose `remote.fetch` `?`-propagates a libgit2
/// abort when `transfer_progress` returned `false`.
pub(super) fn cancelled_or<T>(
    result: Result<T, Error>,
    cancel: Option<&CancelToken>,
) -> Result<T, Error> {
    match result {
        Err(_) if cancelled(cancel) => Err(Error::new(ErrorCode::Cancelled, "Pull cancelled")),
        other => other,
    }
}

/// Build credential callbacks based on the authentication method.
///
/// When `cancel`/`progress` are `Some`, also register `transfer_progress`
/// (reports object/byte stats and aborts the transfer by returning `false` once
/// the token is set) and `sideband_progress` (forwards textual messages).
pub(super) fn build_remote_callbacks<'a>(
    auth: &'a GitAuth,
    cancel: Option<&CancelToken>,
    progress: Option<&ProgressSender>,
) -> RemoteCallbacks<'a> {
    let mut callbacks = RemoteCallbacks::new();
    match auth {
        GitAuth::None => {}
        GitAuth::Pat(token) => {
            let token = token.clone();
            callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
                git2::Cred::userpass_plaintext(&token, "")
                    .or_else(|_| git2::Cred::userpass_plaintext("", &token))
            });
        }
        GitAuth::Ssh {
            username,
            private_key,
            passphrase,
        } => {
            let username = username.clone();
            let private_key = private_key.clone();
            let passphrase = passphrase.clone();
            callbacks.credentials(move |_url, username_from_url, _allowed_types| {
                let user = username_from_url.unwrap_or(&username);
                git2::Cred::ssh_key_from_memory(user, None, &private_key, passphrase.as_deref())
                    .map_err(|e| {
                        git2::Error::from_str(&format!(
                            "SSH key error: {}. Ensure the key is in OpenSSH or PEM format.",
                            e.message()
                        ))
                    })
            });
        }
    }

    // Report object/byte transfer stats; abort the transfer (return `false`)
    // once the cancel token is set — libgit2 then errors the in-flight fetch.
    let cancel_tok = cancel.cloned();
    let progress_tx = progress.cloned();
    callbacks.transfer_progress(move |p| {
        if let Some(tx) = progress_tx.as_ref() {
            let _ = tx.send(progress_from_transfer(&p));
        }
        !cancelled(cancel_tok.as_ref())
    });

    // Forward textual sideband messages (e.g. "Counting objects..."). Also
    // abort the transfer once the cancel token is set: sideband fires more
    // often than `transfer_progress` during the counting/resolving phases, so
    // honouring it keeps cancel latency tight there too.
    let progress_tx = progress.cloned();
    let cancel_tok = cancel.cloned();
    callbacks.sideband_progress(move |msg| {
        if let Some(tx) = progress_tx.as_ref() {
            let _ = tx.send(GitProgress {
                message: Some(String::from_utf8_lossy(msg).into_owned()),
                ..Default::default()
            });
        }
        !cancelled(cancel_tok.as_ref())
    });

    callbacks
}

/// Fetch `origin`'s current branch into a temp ref and return
/// `(branch_name, temp_ref_name, fetched_oid)`. The caller **must** delete the
/// temp ref when done. Mirrors the verified-pull probe so we can inspect remote
/// objects without moving the working branch.
pub(super) fn fetch_remote_into_temp(
    repo: &Repository,
    auth: &GitAuth,
) -> Result<(String, String, git2::Oid), Error> {
    let branch = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot fetch"))?
        .to_string();

    let temp_ref = format!("refs/gpm/probe/{branch}");
    let refspec = format!("+refs/heads/{branch}:{temp_ref}");

    let mut remote = repo.find_remote("origin").map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Cannot find origin remote: {}", e.message()),
        )
    })?;
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(build_remote_callbacks(auth, None, None));
    remote.fetch(&[&refspec], Some(&mut fetch_opts), None)?;

    let oid = repo.refname_to_id(&temp_ref).map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Fetch produced no ref: {e}"),
        )
    })?;
    Ok((branch, temp_ref, oid))
}

/// Map a libgit2 push error onto an [`Error`]. A non-fast-forward rejection
/// becomes `PushRejected` so the write path can distinguish "remote moved" from
/// generic network/auth failures.
pub(super) fn classify_push_error(msg: &str) -> Error {
    // libgit2 reports divergence variously: "non-fast-forward",
    // "cannot push non-fastforwardable reference", code "NotFastForward", plus
    // the server-side "rejected" / "fetch first". Match case-insensitively.
    let lower = msg.to_ascii_lowercase();
    let rejected = lower.contains("non-fast-forward")
        || lower.contains("non-fastforward")
        || lower.contains("fastforwardable")
        || lower.contains("notfastforward")
        || lower.contains("fetch first")
        || lower.contains("rejected");
    if rejected {
        Error::new(
            ErrorCode::PushRejected,
            "Push rejected: remote has diverged. A sync/merge is required.",
        )
    } else if lower.contains("authentication") || lower.contains("credential") {
        Error::new(ErrorCode::CloneFailed, format!("Push auth failed: {msg}"))
    } else if lower.contains("unable to connect") || lower.contains("timeout") {
        Error::new(ErrorCode::NetworkError, format!("Network error: {msg}"))
    } else {
        Error::new(ErrorCode::StoreError, format!("Push failed: {msg}"))
    }
}

/// Classify a git2 error message into the appropriate [`Error`].
pub(super) fn classify_git_error(msg: &str) -> Error {
    if msg.contains("authentication")
        || msg.contains("unsupported URL")
        || msg.contains("SSH key error")
    {
        Error::new(ErrorCode::CloneFailed, format!("Clone failed: {msg}"))
    } else if msg.contains("unable to connect") || msg.contains("timeout") {
        Error::new(ErrorCode::NetworkError, format!("Network error: {msg}"))
    } else {
        Error::new(ErrorCode::CloneFailed, format!("Clone failed: {msg}"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use crate::storage::git::test_support::test_signature;

    use super::*;

    #[test]
    fn git_auth_none_debug() {
        let auth = GitAuth::None;
        assert_eq!(format!("{auth:?}"), "None");
    }

    #[test]
    fn git_auth_pat_debug_masks_token() {
        let auth = GitAuth::Pat("secret-token".to_string());
        let debug = format!("{auth:?}");
        // The redacting `Debug` (storage/mod.rs) masks the token — the test's
        // name has always promised this; the assertion now matches the impl.
        assert_eq!(debug, "Pat(\"[REDACTED]\")");
        assert!(
            !debug.contains("secret-token"),
            "PAT leaked into Debug: {debug}"
        );
    }

    #[test]
    fn git_auth_ssh_debug_format() {
        let auth = GitAuth::Ssh {
            username: "git".to_string(),
            private_key: "secret-key-data".to_string(),
            passphrase: Some("secret-pass".to_string()),
        };
        let debug = format!("{auth:?}");
        assert!(
            debug.contains("Ssh"),
            "SSH variant debug should contain 'Ssh': {debug}"
        );
    }

    #[test]
    fn git_auth_ssh_without_passphrase() {
        let auth = GitAuth::Ssh {
            username: "git".to_string(),
            private_key: "key-data".to_string(),
            passphrase: None,
        };
        let debug = format!("{auth:?}");
        assert!(
            debug.contains("Ssh"),
            "SSH variant debug should contain 'Ssh': {debug}"
        );
    }

    #[test]
    fn classify_git_error_ssh_key() {
        let err = classify_git_error("SSH key error: invalid format");
        assert_eq!(
            err.code, "CLONE_FAILED",
            "SSH key errors should map to CLONE_FAILED"
        );
        assert!(
            err.message.contains("SSH key error"),
            "message should preserve SSH context"
        );
    }

    #[test]
    fn classify_git_error_auth() {
        let err = classify_git_error("authentication required but no callback set");
        assert_eq!(err.code, "CLONE_FAILED");
    }

    #[test]
    fn classify_git_error_network() {
        let err = classify_git_error("unable to connect to host");
        assert_eq!(err.code, "NETWORK_ERROR");
    }

    #[test]
    fn classify_git_error_unsupported_url() {
        let err = classify_git_error("unsupported URL protocol");
        assert_eq!(err.code, "CLONE_FAILED");
    }

    #[test]
    fn classify_git_error_generic() {
        let err = classify_git_error("some unknown error");
        assert_eq!(err.code, "CLONE_FAILED");
    }

    // ── cancel token + progress ──────────────────────────────────────────

    #[test]
    fn cancelled_is_false_without_token() {
        assert!(!cancelled(None));
    }

    #[test]
    fn cancelled_reads_token_state() {
        let token: CancelToken = Arc::new(AtomicBool::new(false));
        assert!(!cancelled(Some(&token)));
        token.store(true, Ordering::Relaxed);
        assert!(cancelled(Some(&token)));
    }

    #[test]
    fn cancelled_or_passes_through_success() {
        let token = Arc::new(AtomicBool::new(true));
        let ok: Result<(), Error> = Ok(());
        let out: Result<(), Error> = cancelled_or(ok, Some(&token));
        assert!(out.is_ok(), "an Ok result is never re-mapped to Cancelled");
    }

    #[test]
    fn cancelled_or_maps_failure_to_cancelled_when_set() {
        let token = Arc::new(AtomicBool::new(true));
        let err = Error::new(ErrorCode::NetworkError, "boom");
        let out: Result<(), Error> = cancelled_or(Err(err), Some(&token));
        let e = out.expect_err("set token must remap the error to Cancelled");
        assert_eq!(e.code, "CANCELLED");
    }

    #[test]
    fn cancelled_or_keeps_original_error_when_not_set() {
        let token = Arc::new(AtomicBool::new(false));
        let err = Error::new(ErrorCode::NetworkError, "boom");
        let out: Result<(), Error> = cancelled_or(Err(err), Some(&token));
        let e = out.expect_err("unset token must keep the original error");
        assert_eq!(e.code, "NETWORK_ERROR");
    }

    // ── transfer_progress callback wiring ────────────────────────────────
    //
    // Local clones copy objects directly and bypass the fetch callbacks, so we
    // force the REMOTE transport via `CloneLocal::None` — the same
    // `transfer_progress` path a real https/ssh clone drives, where our
    // cancel/progress hooks ride.

    /// Build a bare "remote" seeded with one committed file.
    fn bare_repo_with_file() -> tempfile::TempDir {
        let work = tempfile::tempdir().expect("work dir");
        let repo = Repository::init(work.path()).expect("init work");
        std::fs::write(work.path().join("f.age"), b"secret").expect("write file");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("f.age")).expect("add path");
        index.write().expect("index write");
        let tree_id = index.write_tree().expect("write tree");
        drop(index);
        let tree = repo.find_tree(tree_id).expect("tree");
        let sig = test_signature();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .expect("commit");
        drop(tree);
        drop(repo);

        let bare = tempfile::tempdir().expect("bare dir");
        git2::build::RepoBuilder::new()
            .bare(true)
            .clone(work.path().to_str().expect("work path utf-8"), bare.path())
            .expect("bare clone");
        bare
    }

    /// Clone `src` → `dest` over the fetch transport (`CloneLocal::None`), using
    /// [`build_remote_callbacks`] with the given cancel/progress hooks.
    fn clone_via_fetch(
        src: &Path,
        dest: &Path,
        cancel: Option<&CancelToken>,
        progress: Option<&ProgressSender>,
    ) -> Result<(), git2::Error> {
        let auth = GitAuth::None;
        let callbacks = build_remote_callbacks(&auth, cancel, progress);
        let mut fetch_opts = FetchOptions::new();
        fetch_opts.remote_callbacks(callbacks);
        let mut builder = git2::build::RepoBuilder::new();
        builder.clone_local(git2::build::CloneLocal::None);
        builder.fetch_options(fetch_opts);
        builder.clone(src.to_str().expect("src path utf-8"), dest)?;
        Ok(())
    }

    #[test]
    fn transfer_progress_reports_objects_over_fetch_transport() {
        let bare = bare_repo_with_file();
        let dest = tempfile::tempdir().expect("dest dir");
        let (tx, rx) = std::sync::mpsc::channel::<GitProgress>();
        clone_via_fetch(bare.path(), dest.path(), None, Some(&tx))
            .expect("clone via fetch transport should succeed");
        drop(tx); // close the channel so the drain terminates

        let messages: Vec<GitProgress> = rx.iter().collect();
        assert!(
            messages.iter().any(|m| m.total_objects > 0),
            "expected a transfer-progress update reporting objects, got: {messages:?}"
        );
    }

    #[test]
    fn transfer_progress_aborts_clone_when_cancel_pre_armed() {
        let bare = bare_repo_with_file();
        let dest = tempfile::tempdir().expect("dest dir");
        let cancel: CancelToken = Arc::new(AtomicBool::new(true));
        let result = clone_via_fetch(bare.path(), dest.path(), Some(&cancel), None);
        assert!(
            result.is_err(),
            "a pre-armed cancel token must make transfer_progress return false and abort the clone"
        );
    }

    #[test]
    fn embedded_ca_bundle_parses_cleanly() {
        // Regression guard: parse the shipped bundle through the real OpenSSL
        // PEM reader — the same path Android uses to load roots — not just a
        // string match. A botched `refresh-ca` (truncated/corrupt bundle) must
        // either fail to parse or yield the full ~120+ root set, never a silent
        // partial trust store.
        let count = for_each_cert_in_pem(EMBEDDED_CA_BUNDLE.as_bytes(), |_| Ok(()))
            .expect("embedded CA bundle must parse cleanly under the real OpenSSL PEM reader");
        assert!(
            count >= 100,
            "embedded CA bundle parsed only {count} certs, expected a full Mozilla root set (~120+)"
        );
    }

    #[test]
    fn for_each_cert_in_pem_rejects_corrupt_pem() {
        // A `BEGIN CERTIFICATE` line followed by non-base64 must surface as a
        // parse failure, not silently parse zero certs (which would otherwise
        // load an empty trust store).
        let corrupt =
            b"-----BEGIN CERTIFICATE-----\n@@@ not valid base64 @@@\n-----END CERTIFICATE-----\n";
        let result = for_each_cert_in_pem(corrupt, |_| Ok(()));
        assert!(
            result.is_err(),
            "corrupt PEM must error, not silently parse zero certs"
        );
    }

    #[test]
    fn for_each_cert_in_pem_empty_is_zero() {
        // Empty / non-PEM input parses to zero certs with no error (clean EOF).
        // The Android loader's zero-count guard turns `Ok(0)` into a hard error
        // so HTTPS never proceeds with an empty trust store.
        let count = for_each_cert_in_pem(b"", |_| Ok(()))
            .expect("empty input is clean EOF, not a parse failure");
        assert_eq!(count, 0, "empty PEM should parse zero certs without error");
    }
}
