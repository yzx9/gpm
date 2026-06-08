// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod tests {
    use rustpass::Config;

    fn create_config() -> (Config, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let config = Config::new(dir.path().to_path_buf());
        (config, dir)
    }

    #[test]
    fn full_setup_save_load_cycle() {
        let (config, _dir) = create_config();

        let identity = b"AGE-SECRET-KEY-1TEST1234567890ABCDEF";
        config
            .save_identity(identity)
            .expect("save_identity failed");
        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat-token-123"),
                None,
                None,
                "/local/repo/path",
            )
            .expect("save_repo_config failed");

        let loaded_identity = config.load_identity().expect("load_identity failed");
        assert_eq!(
            loaded_identity, identity,
            "identity bytes must round-trip exactly"
        );

        let repo_config = config.load_repo_config().expect("load_repo_config failed");
        assert_eq!(repo_config.url, "https://example.com/repo.git");
        assert_eq!(repo_config.pat, Some(String::from("pat-token-123")));
        assert_eq!(repo_config.local_path, "/local/repo/path");
    }

    #[test]
    fn clear_all_then_reconfigure() {
        let (config, _dir) = create_config();

        config
            .save_identity(b"AGE-SECRET-KEY-1FIRST")
            .expect("initial save_identity failed");
        config
            .save_repo_config(
                "https://first.example.com/repo.git",
                Some("first-pat"),
                None,
                None,
                "/first",
            )
            .expect("initial save_repo_config failed");
        assert!(config.is_configured(), "should be configured after setup");

        config.clear_all().expect("clear_all failed");
        assert!(
            !config.is_configured(),
            "should NOT be configured after clear_all"
        );

        config
            .save_identity(b"AGE-SECRET-KEY-1SECOND")
            .expect("second save_identity failed");
        config
            .save_repo_config(
                "https://second.example.com/repo.git",
                None,
                None,
                None,
                "/second",
            )
            .expect("second save_repo_config failed");
        assert!(
            config.is_configured(),
            "should be configured after reconfigure"
        );

        let identity = config
            .load_identity()
            .expect("load_identity after reconfigure failed");
        assert_eq!(identity, b"AGE-SECRET-KEY-1SECOND");

        let repo_config = config
            .load_repo_config()
            .expect("load_repo_config after reconfigure failed");
        assert_eq!(repo_config.url, "https://second.example.com/repo.git");
        assert_eq!(repo_config.pat, None);
        assert_eq!(repo_config.local_path, "/second");
    }

    #[test]
    fn corrupted_repo_config_errors() {
        let (config, dir) = create_config();

        let repo_json_path = dir.path().join("repo.json");
        std::fs::write(&repo_json_path, "{{{{not valid json!!!!")
            .expect("failed to write corrupted config");

        let err = config
            .load_repo_config()
            .expect_err("loading corrupted config should fail");

        assert_eq!(
            err.code, "CONFIG_ERROR",
            "corrupted JSON must produce CONFIG_ERROR, got: {err:?}"
        );
    }

    #[test]
    fn identity_persistence_across_instances() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let config_a = Config::new(dir.path().to_path_buf());
        let identity = b"AGE-SECRET-KEY-1PERSIST123";
        config_a
            .save_identity(identity)
            .expect("save_identity on first instance failed");

        let config_b = Config::new(dir.path().to_path_buf());
        let loaded = config_b
            .load_identity()
            .expect("load_identity on second instance failed");

        assert_eq!(
            loaded, identity,
            "identity must persist across Config instances"
        );
    }
}
