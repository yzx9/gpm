// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Pagination — the slicing + ranking + `total` semantics of
//! [`Store::list_page`] / [`Store::search_page`] (and the pure
//! [`store::slice_page`] they reduce to). These are the bits the entry-list
//! pager depends on: `total` must stay the full match count regardless of
//! `offset`/`limit`, the slice must be a stable window over the ranked set, and
//! ranking must be best-first with a stable tie order so paging never reorders
//! or drops entries between requests.

mod common;

use common::*;
use rustpass::store;
use rustpass::{Entry, Store};

// ---------------------------------------------------------------------------
// slice_page — the pure pagination-slicing core (no Store, no ranking).
// ---------------------------------------------------------------------------

fn entry(name: &str) -> Entry {
    Entry {
        path: format!("{name}.age"),
        name: name.to_string(),
    }
}

fn ranked(names: &[&str]) -> Vec<Entry> {
    names.iter().map(|n| entry(n)).collect()
}

fn names(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn slice_page_first_and_mid_pages() {
    let r = ranked(&["a", "b", "c", "d", "e"]);
    assert_eq!(
        names(&store::slice_page(r.clone(), 0, 2).entries),
        &["a", "b"]
    );
    assert_eq!(
        names(&store::slice_page(r.clone(), 2, 2).entries),
        &["c", "d"]
    );
}

#[test]
fn slice_page_last_partial_page() {
    // 5 entries, offset 4 limit 2 → just the tail entry.
    let p = store::slice_page(ranked(&["a", "b", "c", "d", "e"]), 4, 2);
    assert_eq!(names(&p.entries), &["e"]);
    assert_eq!(p.total, 5);
}

#[test]
fn slice_page_total_is_full_count_independent_of_window() {
    let r = ranked(&["a", "b", "c", "d", "e"]);
    // total is always the full ranked length, whatever offset/limit ask for.
    assert_eq!(store::slice_page(r.clone(), 0, 2).total, 5);
    assert_eq!(store::slice_page(r.clone(), 4, 2).total, 5);
    assert_eq!(store::slice_page(r.clone(), 0, 100).total, 5);
}

#[test]
fn slice_page_offset_past_end_is_empty_with_full_total() {
    let p = store::slice_page(ranked(&["a", "b", "c", "d", "e"]), 10, 2);
    assert!(p.entries.is_empty());
    assert_eq!(p.total, 5);
}

#[test]
fn slice_page_limit_zero_is_empty_with_full_total() {
    let p = store::slice_page(ranked(&["a", "b", "c", "d", "e"]), 0, 0);
    assert!(p.entries.is_empty());
    assert_eq!(p.total, 5);
}

#[test]
fn slice_page_empty_input() {
    let p = store::slice_page(Vec::new(), 0, 5);
    assert!(p.entries.is_empty());
    assert_eq!(p.total, 0);
}

// ---------------------------------------------------------------------------
// list_page / search_page — ranking + slicing through a real configured Store.
// Listing walks files (no decryption), so the store need not be unlocked.
// ---------------------------------------------------------------------------

/// Configure a store over a temp repo seeded with `entries` (encrypted to a
/// throwaway recipient). Returns the store plus the temp config dir that must
/// outlive it (the repo lives under it).
async fn store_with(entries: Vec<(&str, &[u8])>) -> (tempfile::TempDir, Store) {
    let (identity, recipient) = generate_test_keypair();
    let (bare, _clone) = create_test_git_repo(entries, &recipient);
    let config_dir = tempfile::tempdir().expect("config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare.path().to_str().unwrap(),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure");
    // `bare` is no longer needed — configure already cloned it into config/repo.
    drop(bare);
    (config_dir, store)
}

#[tokio::test]
async fn list_page_slices_a_stable_alpha_sorted_window() {
    // Empty query ranks to alpha-by-name: bank < cloud/aws/root < email/personal.
    let (_dir, store) = store_with(vec![
        ("email/personal.age", b"x"),
        ("bank.age", b"x"),
        ("cloud/aws/root.age", b"x"),
    ])
    .await;

    let p0 = store.list_page(0, 2).await.unwrap();
    assert_eq!(names(&p0.entries), &["bank", "cloud/aws/root"]);
    assert_eq!(p0.total, 3);

    // Page 2 returns the remaining entry; total unchanged.
    let p1 = store.list_page(2, 2).await.unwrap();
    assert_eq!(names(&p1.entries), &["email/personal"]);
    assert_eq!(p1.total, 3);
}

#[tokio::test]
async fn list_page_total_is_independent_of_offset_and_limit() {
    let (_dir, store) = store_with(vec![
        ("a.age", b"x"),
        ("b.age", b"x"),
        ("c.age", b"x"),
        ("d.age", b"x"),
        ("e.age", b"x"),
    ])
    .await;

    assert_eq!(store.list_page(0, 2).await.unwrap().total, 5);
    assert_eq!(store.list_page(4, 2).await.unwrap().total, 5);
    assert_eq!(store.list_page(0, 100).await.unwrap().total, 5);
}

#[tokio::test]
async fn list_page_offset_past_end_is_empty_with_full_total() {
    let (_dir, store) = store_with(vec![("a.age", b"x"), ("b.age", b"x")]).await;

    let p = store.list_page(10, 5).await.unwrap();
    assert!(p.entries.is_empty());
    assert_eq!(p.total, 2);
}

#[tokio::test]
async fn search_page_total_is_match_count_not_slice_length() {
    // "aws" matches all three as a subsequence; total must be 3 even when the
    // page only returns one.
    let (_dir, store) = store_with(vec![
        ("aws/root.age", b"x"),
        ("drawings/sketch.age", b"x"),
        ("lawyers/contract.age", b"x"),
    ])
    .await;

    let p = store.search_page("aws", 0, 1).await.unwrap();
    assert_eq!(p.entries.len(), 1);
    assert_eq!(p.total, 3);
}

#[tokio::test]
async fn search_page_ranks_best_match_first_and_pages_stably() {
    let (_dir, store) = store_with(vec![
        ("aws/root.age", b"x"),
        ("drawings/sketch.age", b"x"),
        ("lawyers/contract.age", b"x"),
    ])
    .await;

    // "aws/root" matches "aws" as a contiguous prefix — strictly better than the
    // scattered subsequence matches in the other two, so it ranks first.
    let p = store.search_page("aws", 0, 10).await.unwrap();
    assert_eq!(p.entries.first().unwrap().name, "aws/root");
    assert_eq!(p.total, 3);

    // Offsetting past the best match drops only it; ranking is stable.
    let p_rest = store.search_page("aws", 1, 10).await.unwrap();
    assert!(p_rest.entries.iter().all(|e| e.name != "aws/root"));
    assert_eq!(p_rest.total, 3);
}

#[tokio::test]
async fn search_page_no_match_is_empty_with_zero_total() {
    let (_dir, store) = store_with(vec![("cloud/aws/root.age", b"x")]).await;

    let p = store.search_page("zzznomatch", 0, 5).await.unwrap();
    assert!(p.entries.is_empty());
    assert_eq!(p.total, 0);
}

#[tokio::test]
async fn search_page_empty_query_matches_list_page() {
    // search_page("") is list_page: same alpha-sorted full set.
    let (_dir, store) = store_with(vec![("bank.age", b"x"), ("cloud/aws/root.age", b"x")]).await;

    let listed_page = store.list_page(0, 10).await.unwrap();
    let listed = names(&listed_page.entries);
    let searched_page = store.search_page("", 0, 10).await.unwrap();
    let searched = names(&searched_page.entries);
    assert_eq!(listed, searched);
    assert_eq!(listed, &["bank", "cloud/aws/root"]);
}
