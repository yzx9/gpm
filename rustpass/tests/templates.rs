// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Templates: `.pass-template` lookup/render + built-in create presets.

mod common;

use std::collections::HashMap;

use common::*;
use rustpass::store::{Store, WriteOutcome};
use rustpass::template;

/// Configure a store against a bare repo carrying a recipients file and an
/// optional root `.pass-template`. Returns `(bare_dir, config_dir, store)`
/// — `bare_dir` MUST stay alive for the store's origin to resolve.
async fn templated_store(
    root_template: Option<&str>,
) -> (tempfile::TempDir, tempfile::TempDir, Store) {
    let (identity, recipient) = generate_test_keypair();
    let mut plaintext: Vec<(&str, &[u8])> = vec![(".gopass-recipients", recipient.as_bytes())];
    let tmpl_bytes;
    if let Some(t) = root_template {
        tmpl_bytes = t.as_bytes().to_vec();
        plaintext.push((".pass-template", tmpl_bytes.leak()));
    }
    let (bare_dir, _clone_dir) = create_test_git_repo_with(vec![], plaintext, &recipient);

    let config_dir = tempfile::tempdir().expect("config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure");
    (bare_dir, config_dir, store)
}

/// A matching `.pass-template` is rendered with `.Content` and stored.
#[tokio::test]
async fn create_applies_content_template() {
    let (_bare, _cfg, store) = templated_store(Some("{{ .Content }}\n\nuser: \nurl: ")).await;

    let outcome = store
        .create("email/gmail", b"s3kr3t")
        .await
        .expect("create");
    assert!(matches!(outcome, WriteOutcome::Written(_)));

    let secret = store.get("email/gmail").await.expect("get");
    assert_eq!(secret.password(), "s3kr3t");
    assert!(secret.body().contains("user:"));
    assert!(secret.body().contains("url:"));
}

/// Without a template, `create` stores the content verbatim.
#[tokio::test]
async fn create_without_template_is_verbatim() {
    let (_bare, _cfg, store) = templated_store(None).await;

    store
        .create("plain/entry", b"just-a-password\nnote: hi")
        .await
        .expect("create");

    let secret = store.get("plain/entry").await.expect("get");
    assert_eq!(secret.password(), "just-a-password");
    assert!(secret.body().contains("note: hi"));
}

/// Template variables `.Name`, `.Path`, `.Dir` resolve to the entry's parts.
#[tokio::test]
async fn create_template_resolves_name_path_dir() {
    let (_bare, _cfg, store) = templated_store(Some(
        "{{ .Content }}\nname={{ .Name }}\npath={{ .Path }}\ndir={{ .Dir }}",
    ))
    .await;

    store.create("sites/example", b"pw").await.expect("create");

    let secret = store.get("sites/example").await.expect("get");
    assert_eq!(secret.password(), "pw");
    assert!(secret.body().contains("name=example"));
    assert!(secret.body().contains("path=sites/example"));
    assert!(secret.body().contains("dir=sites"));
}

/// `lookup_template` walks up to the nearest `.pass-template`.
#[tokio::test]
async fn lookup_template_nearest_wins() {
    // Bare ships BOTH a root template and a `websites/.pass-template`.
    let (identity, recipient) = generate_test_keypair();
    let plaintext: Vec<(&str, &[u8])> = vec![
        (".gopass-recipients", recipient.as_bytes()),
        (".pass-template", b"ROOT"),
        ("websites/.pass-template", b"{{ .Content }}\nuser: "),
    ];
    let (bare_dir, _clone_dir) = create_test_git_repo_with(vec![], plaintext, &recipient);

    let config_dir = tempfile::tempdir().expect("config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure");

    // A websites/ entry picks up the nearer template.
    let near = store.lookup_template("websites/foo").await.expect("lookup");
    assert_eq!(near.as_deref(), Some("{{ .Content }}\nuser: "));
    // An unrelated entry falls back to the root template.
    let root = store.lookup_template("misc/x").await.expect("lookup");
    assert_eq!(root.as_deref(), Some("ROOT"));

    // And the nearer template is actually applied on create.
    store.create("websites/foo", b"pw").await.expect("create");
    let secret = store.get("websites/foo").await.expect("get");
    assert_eq!(secret.password(), "pw");
    assert!(secret.body().contains("user:"), "used the nearer template");
}

/// An unknown template variable surfaces a `TemplateError` and writes nothing.
#[tokio::test]
async fn create_bad_template_errors() {
    let (_bare, _cfg, store) = templated_store(Some("{{ .Content }} {{ .Nope }}")).await;

    let err = store.create("bad/entry", b"pw").await.unwrap_err();
    assert_eq!(err.code, "TEMPLATE_ERROR");

    // Nothing was written.
    let err = store.get("bad/entry").await.unwrap_err();
    assert_eq!(err.code, "ENTRY_NOT_FOUND");
}

/// `create_from_preset` (website) generates the secret under `websites/`.
#[tokio::test]
async fn create_from_website_preset() {
    let (_bare, _cfg, store) = templated_store(None).await;

    let mut fields: HashMap<&str, String> = HashMap::new();
    fields.insert("url", "example.com".to_string());
    fields.insert("username", "alice".to_string());
    fields.insert("password", "hunter2".to_string());

    let outcome = store
        .create_from_preset("website", &fields)
        .await
        .expect("create");
    assert!(matches!(outcome, WriteOutcome::Written(_)));

    // Generated at the prefixed path derived from url + username.
    let secret = store.get("websites/example.com/alice").await.expect("get");
    assert_eq!(secret.password(), "hunter2");
    assert!(secret.body().contains("url: example.com"));
    assert!(secret.body().contains("username: alice"));
}

/// `create_from_preset` (pin) generates a numerical-PIN secret under `pin/`.
#[tokio::test]
async fn create_from_pin_preset() {
    let (_bare, _cfg, store) = templated_store(None).await;

    let mut fields: HashMap<&str, String> = HashMap::new();
    fields.insert("authority", "bank".to_string());
    fields.insert("application", "app".to_string());
    fields.insert("password", "1234".to_string());

    store
        .create_from_preset("pin", &fields)
        .await
        .expect("create");

    let secret = store.get("pin/bank/app").await.expect("get");
    assert_eq!(secret.password(), "1234");
    assert!(secret.body().contains("authority: bank"));
    assert!(secret.body().contains("application: app"));
}

/// An unknown preset id is rejected.
#[tokio::test]
async fn create_from_unknown_preset_errors() {
    let (_bare, _cfg, store) = templated_store(None).await;
    let fields: HashMap<&str, String> = HashMap::new();
    let err = store.create_from_preset("nope", &fields).await.unwrap_err();
    assert_eq!(err.code, "INVALID_ENTRY_NAME");
}

/// The built-in presets are the documented "few options" set.
#[test]
fn builtin_presets_present() {
    let presets = template::builtin_presets();
    let ids: Vec<_> = presets.iter().map(|p| p.id).collect();
    assert!(ids.contains(&"website"));
    assert!(ids.contains(&"pin"));
}

/// `preview_create` renders the matching template without writing anything.
#[tokio::test]
async fn preview_create_renders_template() {
    let (_bare, _cfg, store) = templated_store(Some("{{ .Content }}\nuser: ")).await;

    let preview = store
        .preview_create("email/gmail", b"s3kr3t")
        .await
        .expect("preview");
    assert_eq!(preview.as_deref(), Some("s3kr3t\nuser: "));

    // Nothing was written — the entry is absent.
    let err = store.get("email/gmail").await.unwrap_err();
    assert_eq!(err.code, "ENTRY_NOT_FOUND");
}

/// With no template, `preview_create` returns `None` (content stored verbatim).
#[tokio::test]
async fn preview_create_none_without_template() {
    let (_bare, _cfg, store) = templated_store(None).await;
    let preview = store
        .preview_create("plain/x", b"pw")
        .await
        .expect("preview");
    assert!(preview.is_none());
}

/// A template referencing an unknown variable surfaces a `TemplateError`.
#[tokio::test]
async fn preview_create_bad_template_errors() {
    let (_bare, _cfg, store) = templated_store(Some("{{ .Content }} {{ .Nope }}")).await;
    let err = store.preview_create("bad/x", b"pw").await.unwrap_err();
    assert_eq!(err.code, "TEMPLATE_ERROR");
}

/// Invalid names are rejected before lookup (same gate as `create`/`set`).
#[tokio::test]
async fn preview_create_rejects_bad_name() {
    let (_bare, _cfg, store) = templated_store(None).await;
    let err = store.preview_create("../escape", b"pw").await.unwrap_err();
    assert_eq!(err.code, "INVALID_ENTRY_NAME");
}
