// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Shared helpers for the [`super`] git backend unit tests. Declared
//! `#[cfg(test)]` from the module root, so this file only compiles under test.

use git2::Repository;

/// Shared test signature used across tests.
pub(super) fn test_signature() -> git2::Signature<'static> {
    git2::Signature::new("Test", "test@test.com", &git2::Time::new(0, 0))
        .expect("failed to create signature")
}

/// Create an empty initial commit in a test repository.
///
/// Builds a commit from an empty tree so the repo has a valid HEAD
/// without requiring any working-tree files.
pub(super) fn create_empty_commit(repo: &Repository, sig: &git2::Signature<'_>) -> git2::Oid {
    let mut index = repo.index().expect("failed to get index");
    let tree_id = index.write_tree().expect("failed to write tree");
    let tree = repo.find_tree(tree_id).expect("failed to find tree");
    let parents: &[&git2::Commit<'_>] = &[];
    repo.commit(Some("HEAD"), sig, sig, "initial commit", &tree, parents)
        .expect("failed to create commit")
}

/// Read the system's default branch name from git config.
pub(super) fn config_default_branch(repo: &Repository) -> String {
    repo.config()
        .and_then(|c| c.get_string("init.defaultBranch"))
        .unwrap_or_else(|_| "master".to_string())
}
