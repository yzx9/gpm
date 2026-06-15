// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for repository authenticity: signing.json persistence,
//! trusted-key management, and the verify-before-checkout pull flow (Audit
//! reporting + Enforce abort), including the riskiest Enforce-abort path
//! (HEAD and the working tree must stay put when a pull is refused).

mod common;

use std::path::Path;

use git2::Repository;
use rustpass::signing::{CommitSigStatus, VerifyMode, fingerprint_of_public_key};
use rustpass::store::Store;
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey, rand_core::OsRng};

/// A signing keypair + its public-key string + fingerprint (test fixture).
struct SigningFixture {
    private: PrivateKey,
    public_key: String,
    fingerprint: String,
}

fn signing_fixture() -> SigningFixture {
    let private = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).expect("keygen");
    let public_key = private.public_key().to_openssh().expect("pubkey");
    let fingerprint = fingerprint_of_public_key(&public_key).expect("fingerprint");
    SigningFixture {
        private,
        public_key,
        fingerprint,
    }
}

/// HEAD hash of the store's local repo (full 40-char hex).
fn store_head(store: &Store) -> String {
    let repo_config = block_on(store.config()).expect("repo config");
    let repo = Repository::discover(&repo_config.local_path).expect("open repo");
    repo.head()
        .expect("head")
        .target()
        .expect("oid")
        .to_string()
}

/// Add an **unsigned** commit to the bare repo (acts as the malicious/unsigned
/// new commit a compromised remote could feed).
fn add_unsigned_commit_to_bare(bare_path: &Path, recipient_str: &str, message: &str) -> git2::Oid {
    common::add_commit_to_bare(bare_path, vec![], recipient_str, message)
}

/// Add an **SSH-signed** commit to the bare repo, signed by `signer`.
fn add_signed_commit_to_bare(
    bare_path: &Path,
    recipient_str: &str,
    message: &str,
    signer: &PrivateKey,
) -> git2::Oid {
    // Clone bare → temp working dir, add a (no-op) change so the index/tree
    // differs, build a signed commit, push back.
    let work_dir = tempfile::tempdir().expect("work dir");
    let repo =
        Repository::clone(bare_path.to_str().expect("utf8"), work_dir.path()).expect("clone");

    let sig =
        git2::Signature::new("Signer", "signer@example.com", &git2::Time::new(0, 0)).expect("sig");

    // Touch a file so the tree actually changes (avoids an empty diff commit).
    let marker = work_dir.path().join(".touch");
    std::fs::write(&marker, recipient_str.as_bytes()).expect("write");
    let mut index = repo.index().expect("index");
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("tree");
    let tree = repo.find_tree(tree_id).expect("find tree");

    let head = repo.head().expect("head").target().expect("oid");
    let parent = repo.find_commit(head).expect("parent");

    let buffer = repo
        .commit_create_buffer(&sig, &sig, message, &tree, &[&parent])
        .expect("buffer");
    let ssh_sig = signer
        .sign("git", HashAlg::Sha512, &buffer)
        .expect("ssh-key sign");
    let armor = ssh_sig.to_pem(LineEnding::LF).expect("pem");
    let content = std::str::from_utf8(&buffer).expect("utf8");
    let signed_oid = repo
        .commit_signed(content, &armor, None)
        .expect("commit_signed");

    // Advance the branch ref to the signed commit and push back to the bare.
    let branch = repo.head().expect("head").shorthand().unwrap().to_string();
    repo.reference(
        &format!("refs/heads/{branch}"),
        signed_oid,
        true,
        "signed test commit",
    )
    .expect("advance ref");
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let mut remote = repo.find_remote("origin").expect("origin");
    remote.push(&[&refspec], None).expect("push");

    drop(tree);
    drop(index);
    signed_oid
}

/// Set up a Store cloned from `bare`, with the initial commit as HEAD.
fn store_cloned_from_bare(bare_path: &Path) -> (tempfile::TempDir, Store) {
    let config_dir = tempfile::tempdir().expect("config dir");
    let store = Store::new(config_dir.path().to_path_buf());
    let bare_url = bare_path.to_str().expect("utf8").to_string();
    block_on(store.clone_only(&bare_url, None, None, None)).expect("clone_only");
    (config_dir, store)
}

/// A Store backed by a freshly-cloned repo (so `repo.json` exists) — for tests
/// that exercise authenticity persistence without caring about pull behavior.
/// The authenticity mutation paths require a configured repo.
fn store_with_repo() -> (tempfile::TempDir, Store) {
    let (_identity, recipient) = common::generate_test_keypair();
    let (bare_dir, _initial_clone) =
        common::create_test_git_repo(vec![("first.age", b"pw")], &recipient);
    store_cloned_from_bare(bare_dir.path())
}

fn block_on<F>(f: F) -> F::Output
where
    F: std::future::Future,
{
    tokio::runtime::Runtime::new().expect("runtime").block_on(f)
}

// ── signing.json persistence ─────────────────────────────────────────────

#[test]
fn authenticity_defaults_when_no_repo() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::new(dir.path().to_path_buf());
    let cfg = block_on(store.authenticity_config()).expect("config");
    assert_eq!(cfg.mode, VerifyMode::Off);
    assert!(cfg.trusted_keys.is_empty());
    assert!(cfg.ignored.is_empty());
}

#[test]
fn add_trusted_key_persists_and_dedupes() {
    let (_dir, store) = store_with_repo();
    let fixture = signing_fixture();

    let added = block_on(store.add_trusted_key(&fixture.public_key, "Alice")).expect("add");
    assert_eq!(added.fingerprint, fixture.fingerprint);
    assert_eq!(added.label, "Alice");

    // Persisted.
    let cfg = block_on(store.authenticity_config()).expect("config");
    assert_eq!(cfg.trusted_keys.len(), 1);

    // Dedup: adding the same key again returns the existing entry, no dup.
    let again = block_on(store.add_trusted_key(&fixture.public_key, "Other")).expect("add2");
    assert_eq!(again.fingerprint, fixture.fingerprint);
    let cfg = block_on(store.authenticity_config()).expect("config2");
    assert_eq!(cfg.trusted_keys.len(), 1, "duplicate add must not append");
}

#[test]
fn remove_trusted_key_downgrades_enforce_to_audit() {
    let (_dir, store) = store_with_repo();
    let fixture = signing_fixture();

    block_on(store.add_trusted_key(&fixture.public_key, "Alice")).expect("add");
    block_on(store.set_verification_mode(VerifyMode::Enforce)).expect("mode enforce");

    // Removing the last trusted key while in Enforce → downgrades to Audit.
    block_on(store.remove_trusted_key(&fixture.fingerprint)).expect("remove");
    let cfg = block_on(store.authenticity_config()).expect("config");
    assert_eq!(
        cfg.mode,
        VerifyMode::Audit,
        "Enforce with no keys must downgrade"
    );
}

#[test]
fn enforce_refused_without_trusted_keys() {
    let (_dir, store) = store_with_repo();
    let err = block_on(store.set_verification_mode(VerifyMode::Enforce)).expect_err("refuse");
    assert_eq!(err.code, "CONFIG_ERROR");
}

#[test]
fn authenticity_persists_across_reload() {
    // After the merge, authenticity lives inside repo.json — verify it survives
    // a Store reload (proving it's in repo.json, not an in-memory cache).
    let (dir, store) = store_with_repo();
    let fixture = signing_fixture();
    block_on(store.add_trusted_key(&fixture.public_key, "Alice")).expect("add");
    block_on(store.set_verification_mode(VerifyMode::Audit)).expect("mode");

    // A brand-new Store over the same config dir sees the persisted state.
    let store2 = Store::new(dir.path().to_path_buf());
    let cfg = block_on(store2.authenticity_config()).expect("config");
    assert_eq!(cfg.mode, VerifyMode::Audit);
    assert_eq!(cfg.trusted_keys.len(), 1);
    assert_eq!(
        cfg.trusted_keys.first().expect("key").fingerprint,
        fixture.fingerprint
    );
}

// ── verify-before-checkout pull flow ─────────────────────────────────────

#[test]
fn audit_pull_reports_unsigned_issue_but_advances() {
    let (_identity, recipient) = common::generate_test_keypair();
    let (bare_dir, _initial_clone) =
        common::create_test_git_repo(vec![("first.age", b"pw")], &recipient);
    let (_cfg_dir, store) = store_cloned_from_bare(bare_dir.path());

    // Trust a key + Audit mode.
    let fixture = signing_fixture();
    block_on(store.add_trusted_key(&fixture.public_key, "Alice")).expect("add");
    block_on(store.set_verification_mode(VerifyMode::Audit)).expect("audit");

    let head_before = store_head(&store);

    // Remote feeds an unsigned commit.
    add_unsigned_commit_to_bare(bare_dir.path(), &recipient, "unsigned update");

    let result = block_on(store.sync()).expect("audit pull");
    assert!(result.changed, "Audit must still advance HEAD");
    assert!(!result.authenticity.blocked, "Audit never blocks");
    assert!(
        !result.authenticity.open_issues.is_empty(),
        "Audit must surface the unsigned commit as an open issue"
    );
    assert_eq!(
        result
            .authenticity
            .open_issues
            .first()
            .expect("issue")
            .status,
        CommitSigStatus::Unsigned
    );

    let head_after = store_head(&store);
    assert_ne!(head_after, head_before, "HEAD must advance in Audit");
}

#[test]
fn enforce_aborts_pull_on_unsigned_commit_head_unchanged() {
    let (_identity, recipient) = common::generate_test_keypair();
    let (bare_dir, _initial_clone) =
        common::create_test_git_repo(vec![("first.age", b"pw")], &recipient);
    let (_cfg_dir, store) = store_cloned_from_bare(bare_dir.path());

    let fixture = signing_fixture();
    block_on(store.add_trusted_key(&fixture.public_key, "Alice")).expect("add");
    block_on(store.set_verification_mode(VerifyMode::Enforce)).expect("enforce");

    let head_before = store_head(&store);
    let files_before: Vec<String> = std::fs::read_dir(repo_local_path(&store))
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();

    // Remote feeds an unsigned commit.
    add_unsigned_commit_to_bare(bare_dir.path(), &recipient, "malicious unsigned");

    let result = block_on(store.sync()).expect("sync returns (blocked, not error)");
    assert!(
        !result.changed,
        "Enforce must NOT advance HEAD on a blocking issue"
    );
    assert!(
        result.authenticity.blocked,
        "Enforce must report blocked on an unsigned commit"
    );
    assert!(
        !result.authenticity.open_issues.is_empty(),
        "the blocking issue must be reported"
    );

    // The critical assertion: HEAD and the working tree are unchanged.
    let head_after = store_head(&store);
    assert_eq!(
        head_after, head_before,
        "HEAD must stay put when Enforce refuses checkout"
    );
    let files_after: Vec<String> = std::fs::read_dir(repo_local_path(&store))
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        files_after, files_before,
        "working tree must be unchanged when Enforce refuses checkout"
    );
}

#[test]
fn enforce_allows_pull_when_signed_by_trusted_key() {
    let (_identity, recipient) = common::generate_test_keypair();
    let (bare_dir, _initial_clone) =
        common::create_test_git_repo(vec![("first.age", b"pw")], &recipient);
    let (_cfg_dir, store) = store_cloned_from_bare(bare_dir.path());

    let fixture = signing_fixture();
    block_on(store.add_trusted_key(&fixture.public_key, "Alice")).expect("add");
    block_on(store.set_verification_mode(VerifyMode::Enforce)).expect("enforce");

    let head_before = store_head(&store);

    // Remote feeds a commit signed by the trusted key.
    add_signed_commit_to_bare(
        bare_dir.path(),
        &recipient,
        "signed update",
        &fixture.private,
    );

    let result = block_on(store.sync()).expect("enforce pull");
    assert!(
        result.changed,
        "Enforce must advance HEAD when the new commit is signed by a trusted key"
    );
    assert!(
        !result.authenticity.blocked,
        "no blocking issue for a trusted signature"
    );
    assert!(
        result.authenticity.open_issues.is_empty(),
        "a Verified commit must not be an open issue"
    );
    assert_eq!(
        result
            .authenticity
            .new_commits
            .first()
            .expect("commit")
            .status,
        CommitSigStatus::Verified {
            signer_fp: fixture.fingerprint
        }
    );

    let head_after = store_head(&store);
    assert_ne!(head_after, head_before, "HEAD must advance");
}

#[test]
fn enforce_blocks_on_untrusted_signer() {
    let (_identity, recipient) = common::generate_test_keypair();
    let (bare_dir, _initial_clone) =
        common::create_test_git_repo(vec![("first.age", b"pw")], &recipient);
    let (_cfg_dir, store) = store_cloned_from_bare(bare_dir.path());

    // Trust Alice's key.
    let alice = signing_fixture();
    block_on(store.add_trusted_key(&alice.public_key, "Alice")).expect("add");
    block_on(store.set_verification_mode(VerifyMode::Enforce)).expect("enforce");

    let head_before = store_head(&store);

    // Remote feeds a commit signed by an ATTACKER key (not trusted).
    let attacker = signing_fixture();
    add_signed_commit_to_bare(
        bare_dir.path(),
        &recipient,
        "attacker commit",
        &attacker.private,
    );

    let result = block_on(store.sync()).expect("sync");
    assert!(!result.changed, "Enforce must block an untrusted signer");
    assert!(result.authenticity.blocked);
    assert_eq!(
        result
            .authenticity
            .open_issues
            .first()
            .expect("issue")
            .status,
        CommitSigStatus::UntrustedKey {
            signer_fp: attacker.fingerprint
        }
    );
    assert_eq!(store_head(&store), head_before, "HEAD unchanged on block");
}

#[test]
fn ignored_issue_no_longer_blocks_enforce() {
    let (_identity, recipient) = common::generate_test_keypair();
    let (bare_dir, _initial_clone) =
        common::create_test_git_repo(vec![("first.age", b"pw")], &recipient);
    let (_cfg_dir, store) = store_cloned_from_bare(bare_dir.path());

    let fixture = signing_fixture();
    block_on(store.add_trusted_key(&fixture.public_key, "Alice")).expect("add");
    block_on(store.set_verification_mode(VerifyMode::Enforce)).expect("enforce");

    let head_before = store_head(&store);
    add_unsigned_commit_to_bare(bare_dir.path(), &recipient, "unsigned");

    // First pull blocks.
    let blocked = block_on(store.sync()).expect("sync1");
    assert!(blocked.authenticity.blocked);
    assert_eq!(store_head(&store), head_before);

    // Ignore the offending commit (status recomputed server-side).
    let offending = blocked.authenticity.open_issues.first().expect("issue");
    block_on(store.ignore_commit_issue(&offending.hash)).expect("ignore");

    // Second pull: the unsigned commit is now ignored → Enforce advances.
    let result = block_on(store.sync()).expect("sync2");
    assert!(result.changed, "an ignored issue must not block Enforce");
    assert!(!result.authenticity.blocked);
    assert!(result.authenticity.open_issues.is_empty());
}

/// The local repo path the Store is backed by.
fn repo_local_path(store: &Store) -> String {
    let repo_config = block_on(store.config()).expect("repo config");
    repo_config.local_path
}
