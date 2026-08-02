// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret revision history (R027) — `Store::list_revisions` +
//! `Store::get_at_revision`. Covers the path-bound walk, the HEAD==live
//! invariant, recipient-rotation (`Undecryptable`), a delete-commit
//! (`Deleted`), pagination + the base-oid anchor, and a single-commit history.

mod common;
use common::{
    TEST_RECIPIENTS_FILE, add_commit_to_bare, create_test_git_repo_with, crypto_permit,
    generate_test_keypair, store_with_base,
};
use rustpass::{RevisionContent, Store};

/// `set("foo")` 3× builds a 3-commit history; a `set("bar")` commit touching a
/// different file must NOT appear in foo's list (proves the pathspec filter, not
/// just inclusion). Newest first; the base commit's subject is "initial commit".
#[tokio::test]
async fn list_is_path_bound_newest_first_and_excludes_other_files() {
    let _crypto = crypto_permit().await;
    let (_bare, _cfg, store, _rec) = store_with_base(vec![("foo.age", b"v1")]).await;

    store.set("foo", b"v2").await.expect("set v2");
    store.set("foo", b"v3").await.expect("set v3");
    // A commit touching a DIFFERENT secret — must be excluded from foo's list.
    store.set("bar", b"other").await.expect("set bar");

    let page = store
        .list_revisions("foo", 0, 50, None)
        .await
        .expect("list");
    assert!(!page.has_more);
    assert_eq!(page.commits.len(), 3, "foo touched in exactly 3 commits");
    // Newest first: the last `set("foo")` is on top.
    assert_eq!(page.commits[0].subject, "Save secret: foo");
    // The base commit (from store_with_base) is the oldest.
    assert_eq!(page.commits.last().unwrap().subject, "initial commit");
    // The bar commit is not among foo's revisions.
    assert!(
        !page.commits.iter().any(|c| c.subject.contains("bar")),
        "a commit touching bar leaked into foo's history"
    );
}

/// The HEAD revision decrypts to the same value as the live `Store::get`.
#[tokio::test]
async fn head_revision_matches_live_get() {
    let _crypto = crypto_permit().await;
    let (_bare, _cfg, store, _rec) = store_with_base(vec![("foo.age", b"v1")]).await;
    store
        .set("foo", b"the-current-password")
        .await
        .expect("set");

    let page = store
        .list_revisions("foo", 0, 50, None)
        .await
        .expect("list");
    let head = &page.commits[0].hash;

    let live = store.get("foo").await.expect("get");
    let past = store
        .get_at_revision("foo", head)
        .await
        .expect("get_at_revision HEAD");
    match past {
        RevisionContent::Decrypted(secret) => {
            assert_eq!(secret.password(), live.password());
            assert_eq!(secret.password(), "the-current-password");
        }
        other => panic!("HEAD revision should decrypt, got {other:?}"),
    }
}

/// An old revision encrypted to a recipient the current identity isn't in
/// (recipient rotation) is `Undecryptable`; the newer one (encrypted to the
/// current identity) decrypts. Ciphertext never surfaces.
#[tokio::test]
async fn undecryptable_for_recipient_swapped_old_revision() {
    let _crypto = crypto_permit().await;
    let (id_a, rec_a) = generate_test_keypair(); // the store's identity
    let (_id_b, rec_b) = generate_test_keypair(); // a departed recipient

    // commit 1: foo encrypted to B; .age-recipients carries A (verbatim).
    let (bare, _clone) = create_test_git_repo_with(
        vec![("foo.age", b"the-password")],
        vec![(TEST_RECIPIENTS_FILE, rec_a.as_bytes())],
        &rec_b,
    );
    // commit 2: foo re-encrypted to A.
    add_commit_to_bare(
        bare.path(),
        vec![("foo.age", b"the-password")],
        &rec_a,
        "re-encrypt to A",
    );

    let cfg = tempfile::tempdir().expect("config dir");
    let store = Store::new(cfg.path().to_path_buf(), None);
    store
        .configure(
            bare.path().to_str().expect("utf-8"),
            None,
            None,
            None,
            &id_a,
            None,
        )
        .await
        .expect("configure");

    let page = store
        .list_revisions("foo", 0, 50, None)
        .await
        .expect("list");
    assert_eq!(page.commits.len(), 2);
    // Newest (commit 2, encrypted to A) decrypts.
    assert!(
        matches!(
            store.get_at_revision("foo", &page.commits[0].hash).await,
            Ok(RevisionContent::Decrypted(_))
        ),
        "newest revision (encrypted to A) should decrypt"
    );
    // Oldest (commit 1, encrypted to B) is undecryptable by A.
    assert!(
        matches!(
            store.get_at_revision("foo", &page.commits[1].hash).await,
            Ok(RevisionContent::Undecryptable)
        ),
        "old revision (encrypted to departed B) should be Undecryptable"
    );
}

/// A revision that deleted the entry is `Deleted` (no blob at that commit); an
/// earlier revision still decrypts.
#[tokio::test]
async fn deleted_revision_returns_deleted() {
    let _crypto = crypto_permit().await;
    let (_bare, _cfg, store, _rec) = store_with_base(vec![("foo.age", b"v1")]).await;
    store.set("foo", b"v2").await.expect("set v2");
    store.delete("foo").await.expect("delete");

    let page = store
        .list_revisions("foo", 0, 50, None)
        .await
        .expect("list");
    assert_eq!(page.commits.len(), 3);
    // Newest commit deleted foo → Deleted.
    assert!(
        matches!(
            store.get_at_revision("foo", &page.commits[0].hash).await,
            Ok(RevisionContent::Deleted)
        ),
        "delete-commit should be Deleted"
    );
    // The prior revision still decrypts.
    assert!(
        matches!(
            store.get_at_revision("foo", &page.commits[1].hash).await,
            Ok(RevisionContent::Decrypted(_))
        ),
        "pre-delete revision should decrypt"
    );
}

/// Pagination: a limit under the count sets `has_more`; the base-oid anchor is
/// returned and reused. A single-commit history lists exactly one revision.
#[tokio::test]
async fn pagination_and_single_commit_history() {
    let _crypto = crypto_permit().await;

    // Single-commit history (a freshly seeded secret): exactly one revision.
    let (_bare, _cfg, store, _rec) = store_with_base(vec![("solo.age", b"only")]).await;
    let page = store
        .list_revisions("solo", 0, 50, None)
        .await
        .expect("list");
    assert_eq!(page.commits.len(), 1);
    assert!(!page.has_more);

    // Build a 3-commit history on `foo` for the pagination assertions.
    store.set("foo", b"v1b").await.expect("set");
    store.set("foo", b"v2").await.expect("set");
    store.set("foo", b"v3").await.expect("set");

    let p0 = store
        .list_revisions("foo", 0, 2, None)
        .await
        .expect("page 0");
    assert_eq!(p0.commits.len(), 2);
    assert!(p0.has_more);
    let base = p0.base_oid.clone();
    assert!(!base.is_empty());

    // Page 1 anchored to the same base_oid → the remaining commit, no more.
    let p1 = store
        .list_revisions("foo", 2, 2, Some(&base))
        .await
        .expect("page 1");
    assert_eq!(p1.commits.len(), 1);
    assert!(!p1.has_more);
    // Anchored to the same base across pages.
    assert_eq!(p1.base_oid, base);
    // No overlap between the two pages.
    let p0_hashes: std::collections::HashSet<&str> =
        p0.commits.iter().map(|c| c.hash.as_str()).collect();
    assert!(
        !p1.commits
            .iter()
            .any(|c| p0_hashes.contains(c.hash.as_str())),
        "pages overlap"
    );
}

/// A1 — the base-oid anchor survives a HEAD fast-forward between page turns.
/// Page 0 captures `base_oid`; a background sync then advances HEAD by two
/// commits on the same secret. Page 1, anchored to page 0's `base_oid`, walks
/// the SAME window page 0 did — no overlap with page 0 and no post-drift commit
/// leaks in. An unanchored walk would re-emit page 0's commits (the drift).
#[tokio::test]
async fn pagination_base_oid_anchors_against_mid_walk_head_drift() {
    let _crypto = crypto_permit().await;
    let (_bare, _cfg, store, _rec) = store_with_base(vec![("foo.age", b"v0")]).await;
    store.set("foo", b"v1").await.expect("set v1");
    store.set("foo", b"v2").await.expect("set v2");
    store.set("foo", b"v3").await.expect("set v3");

    let p0 = store
        .list_revisions("foo", 0, 2, None)
        .await
        .expect("page 0");
    let base = p0.base_oid.clone();

    // Background sync fast-forwards HEAD by two commits AFTER page 0 captured
    // the anchor. An unanchored page 1 would walk the new HEAD and re-emit
    // page 0's window.
    store.set("foo", b"v4").await.expect("set v4");
    store.set("foo", b"v5").await.expect("set v5");

    let p1 = store
        .list_revisions("foo", 2, 2, Some(&base))
        .await
        .expect("anchored page 1");

    // No overlap with page 0's window.
    let p0_hashes: std::collections::HashSet<&str> =
        p0.commits.iter().map(|c| c.hash.as_str()).collect();
    assert!(
        !p1.commits
            .iter()
            .any(|c| p0_hashes.contains(c.hash.as_str())),
        "anchored page 1 drifted into page 0's window"
    );

    // No post-drift commit leaked into the anchored page 1 (it walked `base`,
    // not the new HEAD).
    let post = store
        .list_revisions("foo", 0, 2, None)
        .await
        .expect("new HEAD page");
    let post_hashes: std::collections::HashSet<&str> =
        post.commits.iter().map(|c| c.hash.as_str()).collect();
    assert!(
        !p1.commits
            .iter()
            .any(|c| post_hashes.contains(c.hash.as_str())),
        "anchored page 1 leaked a post-drift commit"
    );

    // And page 1 actually advanced — anchored, not stalled.
    assert!(
        !p1.commits.is_empty(),
        "anchored page 1 should still advance"
    );
}

/// Failure-mode #2: a name with no history lists empty with `has_more = false`
/// and still captures a `base_oid` (HEAD). Guards against a pathspec regression
/// that returns a phantom commit or defaults `has_more` to true.
#[tokio::test]
async fn missing_entry_lists_empty_with_no_more() {
    let _crypto = crypto_permit().await;
    let (_bare, _cfg, store, _rec) = store_with_base(vec![("foo.age", b"v1")]).await;

    let page = store
        .list_revisions("never-existed", 0, 50, None)
        .await
        .expect("list");
    assert!(
        page.commits.is_empty(),
        "no commit touches a non-existent entry"
    );
    assert!(!page.has_more);
    assert!(
        !page.base_oid.is_empty(),
        "base_oid is still captured (HEAD) for an empty result"
    );
}
