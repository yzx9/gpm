// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

use common::*;
use rustpass::store;

/// Empty store directory (no files at all) should return empty list.
#[test]
fn list_empty_directory() {
    let dir = tempfile::tempdir().unwrap();
    let entries = store::list_entries(dir.path()).unwrap();
    assert!(
        entries.is_empty(),
        "empty directory should return no entries"
    );
}

/// Entries nested more than 5 levels deep should still be discovered.
#[test]
fn list_deeply_nested_entries() {
    let (_identity, recipient) = generate_test_keypair();
    let dir = create_test_store(
        vec![("a/b/c/d/e/f/secret.age", b"deep-password")],
        &recipient,
    );

    let entries = store::list_entries(dir.path()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a/b/c/d/e/f/secret");
}

/// Files with extensions other than .age should be ignored.
#[test]
fn list_mixed_extensions() {
    let (_identity, recipient) = generate_test_keypair();
    let dir = create_test_store(vec![("valid.age", b"password")], &recipient);
    std::fs::write(dir.path().join("notes.txt"), b"not encrypted").unwrap();
    std::fs::write(dir.path().join("data.json"), b"{}").unwrap();
    std::fs::write(dir.path().join("backup.gpg"), b"gpg data").unwrap();

    let entries = store::list_entries(dir.path()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "valid");
}

/// Filenames with spaces and special characters.
#[test]
fn list_special_characters_in_names() {
    let (_identity, recipient) = generate_test_keypair();
    let dir = create_test_store(
        vec![
            ("my entry.age", b"pw1"),
            ("with-dash.age", b"pw2"),
            ("under_score.age", b"pw3"),
        ],
        &recipient,
    );

    let entries = store::list_entries(dir.path()).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().any(|e| e.name == "my entry"));
    assert!(entries.iter().any(|e| e.name == "with-dash"));
    assert!(entries.iter().any(|e| e.name == "under_score"));
}

/// Unicode filenames should work.
#[test]
fn list_unicode_filenames() {
    let (_identity, recipient) = generate_test_keypair();
    let dir = create_test_store(
        vec![
            ("日本語/銀行.age", b"jp-bank-pw"),
            ("中文/密码.age", b"cn-pw"),
            ("emoji/🔑.age", b"emoji-pw"),
        ],
        &recipient,
    );

    let entries = store::list_entries(dir.path()).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().any(|e| e.name == "日本語/銀行"));
    assert!(entries.iter().any(|e| e.name == "中文/密码"));
    assert!(entries.iter().any(|e| e.name == "emoji/🔑"));
}

/// Many entries (>100) should all be found and sorted.
#[test]
fn list_many_entries_sorted() {
    let (_identity, recipient) = generate_test_keypair();

    // Create entries directly (can't easily build Vec<(&str, &[u8])> with owned strings)
    let dir = tempfile::tempdir().unwrap();
    for i in 0..120u32 {
        let name = format!("entry{i:03}.age");
        let content = format!("password{i}");
        let file_path = dir.path().join(&name);
        let encrypted = encrypt_to_recipient(content.as_bytes(), &recipient);
        std::fs::write(&file_path, encrypted).unwrap();
    }

    let entries = store::list_entries(dir.path()).unwrap();
    assert_eq!(entries.len(), 120);

    // Verify sorted order (case-insensitive)
    for window in entries.windows(2) {
        assert!(
            window[0].name.to_lowercase() <= window[1].name.to_lowercase(),
            "entries should be sorted case-insensitively: {} > {}",
            window[0].name,
            window[1].name
        );
    }
}

/// Hidden directories (starting with .) other than .git should still have
/// their .age files listed (gopass behavior).
#[test]
fn list_includes_hidden_directories() {
    let (_identity, recipient) = generate_test_keypair();
    let dir = create_test_store(
        vec![
            (".hidden/secret.age", b"hidden-pw"),
            ("visible.age", b"visible-pw"),
        ],
        &recipient,
    );

    let entries = store::list_entries(dir.path()).unwrap();
    // .hidden directory entries should be included (gopass behavior)
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|e| e.name == ".hidden/secret"));
    assert!(entries.iter().any(|e| e.name == "visible"));
}

/// .age-recipients files should not appear in the entry list.
#[test]
fn list_skips_age_recipients_file() {
    let (_identity, recipient) = generate_test_keypair();
    let dir = create_test_store(vec![("real.age", b"password")], &recipient);
    // Write a .age-recipients file (gopass metadata, not an entry)
    std::fs::write(dir.path().join(".age-recipients"), "age1abc123...").unwrap();

    let entries = store::list_entries(dir.path()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "real");
}
