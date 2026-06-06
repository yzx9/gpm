// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Shared test helpers used across integration test files.

use std::io::Write;
use std::str::FromStr;

use age::secrecy::ExposeSecret;
use age::x25519::{Identity, Recipient};

/// Generate a random x25519 keypair, returning `(identity_str, recipient_str)`.
pub fn generate_test_keypair() -> (String, String) {
    let sk = Identity::generate();
    let pk = sk.to_public();
    let identity_str = sk.to_string().expose_secret().to_string();
    let recipient_str = pk.to_string();
    (identity_str, recipient_str)
}

/// Encrypt `plaintext` to the given recipient string, returning ciphertext bytes.
pub fn encrypt_to_recipient(plaintext: &[u8], recipient_str: &str) -> Vec<u8> {
    let recipient = Recipient::from_str(recipient_str).unwrap();
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .unwrap();
    let mut encrypted = Vec::new();
    let mut writer = encryptor.wrap_output(&mut encrypted).unwrap();
    writer.write_all(plaintext).unwrap();
    writer.finish().unwrap();
    encrypted
}

/// Create a temporary directory that acts as a gopass store.
///
/// Each entry is `(relative_path, plaintext_content)` — the content is
/// encrypted with `recipient_str` and written to the path.
#[allow(dead_code)]
pub fn create_test_store(entries: Vec<(&str, &[u8])>, recipient_str: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in entries {
        let file_path = dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let encrypted = encrypt_to_recipient(content, recipient_str);
        std::fs::write(file_path, encrypted).unwrap();
    }
    dir
}

/// Create a local git repository suitable for integration tests.
///
/// Returns `(bare_dir, clone_dir)` where:
/// - `bare_dir` is a bare repo (acts as "remote")
/// - `clone_dir` is a working clone (acts as "local")
///
/// The bare repo has one initial commit on `refs/heads/main` containing
/// any provided `.age` entries (encrypted to `recipient_str`).
#[allow(dead_code)]
pub fn create_test_git_repo(
    entries: Vec<(&str, &[u8])>,
    recipient_str: &str,
) -> (tempfile::TempDir, tempfile::TempDir) {
    let bare_dir = tempfile::tempdir().unwrap();
    let clone_dir = tempfile::tempdir().unwrap();

    // Create a working repo first, add content, then clone --bare from it
    let work_dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(work_dir.path()).unwrap();

    let sig = git2::Signature::new("Test", "test@test.com", &git2_time(0)).unwrap();

    // Write entries to the working tree
    for (path, content) in &entries {
        let file_path = work_dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let encrypted = encrypt_to_recipient(content, recipient_str);
        std::fs::write(&file_path, encrypted).unwrap();
    }

    // Stage and commit all entries
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let commit_id = repo
        .commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
        .unwrap();

    // Create bare repo from the working repo
    let mut builder = git2::build::RepoBuilder::new();
    builder.bare(true);
    builder
        .clone(work_dir.path().to_str().unwrap(), bare_dir.path())
        .unwrap();

    // Clone from bare into clone_dir
    git2::Repository::clone(bare_dir.path().to_str().unwrap(), clone_dir.path()).unwrap();

    // Drop borrow-holding values before the owner
    let _ = commit_id;
    drop(tree);
    drop(index);
    drop(repo);
    drop(work_dir);

    (bare_dir, clone_dir)
}

/// Add a new commit to the bare repo with additional entries.
#[allow(dead_code)]
pub fn add_commit_to_bare(
    bare_path: &std::path::Path,
    entries: Vec<(&str, &[u8])>,
    recipient_str: &str,
    message: &str,
) -> git2::Oid {
    // Clone bare to a temp working dir, make changes, push back
    let work_dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::clone(bare_path.to_str().unwrap(), work_dir.path()).unwrap();

    let sig = git2::Signature::new("Test", "test@test.com", &git2_time(0)).unwrap();

    // Write new/updated entries
    for (path, content) in &entries {
        let file_path = work_dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let encrypted = encrypt_to_recipient(content, recipient_str);
        std::fs::write(&file_path, encrypted).unwrap();
    }

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();

    let head = repo.head().unwrap().target().unwrap();
    let parent = repo.find_commit(head).unwrap();

    let commit_id = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .unwrap();

    // Push back to bare repo (origin)
    let mut remote = repo.find_remote("origin").unwrap();
    remote
        .push(&["refs/heads/main:refs/heads/main"], None)
        .unwrap();

    commit_id
}

/// Helper to create a `git2::Time`.
#[allow(dead_code)]
fn git2_time(secs: i64) -> git2::Time {
    git2::Time::new(secs, 0)
}
