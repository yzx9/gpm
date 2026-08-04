// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

// API-surface lints (missing_docs, pedantic, …) target library code; tests opt out.
#![allow(
    missing_docs,
    unused_qualifications,
    trivial_casts,
    trivial_numeric_casts,
    clippy::pedantic,
    clippy::indexing_slicing
)]

use std::fs;

use rustpass::{Config, GitAuth};

fn create_config() -> (Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let config = Config::new(dir.path().to_path_buf(), None);
    (config, dir)
}

#[tokio::test]
async fn full_setup_save_load_cycle() {
    let (config, _dir) = create_config();

    let identity = b"AGE-SECRET-KEY-1TEST1234567890ABCDEF";
    config
        .save_identity(identity, None)
        .await
        .expect("save_identity failed");
    config
        .save_repo_config(
            "https://example.com/repo.git",
            &GitAuth::from_pat("pat-token-123"),
            "/local/repo/path",
        )
        .await
        .expect("save_repo_config failed");

    let loaded_identity = config.load_identity().await.expect("load_identity failed");
    assert_eq!(
        loaded_identity, identity,
        "identity bytes must round-trip exactly"
    );

    let repo_config = config
        .load_repo_config()
        .await
        .expect("load_repo_config failed");
    assert_eq!(repo_config.url, "https://example.com/repo.git");
    assert_eq!(repo_config.pat, Some(String::from("pat-token-123")));
    assert_eq!(repo_config.local_path, "/local/repo/path");
}

#[tokio::test]
async fn clear_all_then_reconfigure() {
    let (config, _dir) = create_config();

    config
        .save_identity(b"AGE-SECRET-KEY-1FIRST", None)
        .await
        .expect("initial save_identity failed");
    config
        .save_repo_config(
            "https://first.example.com/repo.git",
            &GitAuth::from_pat("first-pat"),
            "/first",
        )
        .await
        .expect("initial save_repo_config failed");
    assert!(config.is_configured(), "should be configured after setup");

    config.clear_all().await.expect("clear_all failed");
    assert!(
        !config.is_configured(),
        "should NOT be configured after clear_all"
    );

    config
        .save_identity(b"AGE-SECRET-KEY-1SECOND", None)
        .await
        .expect("second save_identity failed");
    config
        .save_repo_config(
            "https://second.example.com/repo.git",
            &GitAuth::None,
            "/second",
        )
        .await
        .expect("second save_repo_config failed");
    assert!(
        config.is_configured(),
        "should be configured after reconfigure"
    );

    let identity = config
        .load_identity()
        .await
        .expect("load_identity after reconfigure failed");
    assert_eq!(identity, b"AGE-SECRET-KEY-1SECOND");

    let repo_config = config
        .load_repo_config()
        .await
        .expect("load_repo_config after reconfigure failed");
    assert_eq!(repo_config.url, "https://second.example.com/repo.git");
    assert_eq!(repo_config.pat, None);
    assert_eq!(repo_config.local_path, "/second");
}

#[tokio::test]
async fn corrupted_repo_config_errors() {
    let (config, dir) = create_config();

    let repo_json_path = dir.path().join("repo.json");
    fs::write(&repo_json_path, "{{{{not valid json!!!!").expect("failed to write corrupted config");

    let err = config
        .load_repo_config()
        .await
        .expect_err("loading corrupted config should fail");

    assert_eq!(
        err.code, "CONFIG_ERROR",
        "corrupted JSON must produce CONFIG_ERROR, got: {err:?}"
    );
}

#[tokio::test]
async fn identity_persistence_across_instances() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");

    let config_a = Config::new(dir.path().to_path_buf(), None);
    let identity = b"AGE-SECRET-KEY-1PERSIST123";
    config_a
        .save_identity(identity, None)
        .await
        .expect("save_identity on first instance failed");

    let config_b = Config::new(dir.path().to_path_buf(), None);
    let loaded = config_b
        .load_identity()
        .await
        .expect("load_identity on second instance failed");

    assert_eq!(
        loaded, identity,
        "identity must persist across Config instances"
    );
}
