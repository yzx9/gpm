// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod tests {
    use gpm_lib::test_support::SecureStorage;

    fn create_storage() -> (SecureStorage, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let storage = SecureStorage::new(dir.path().to_path_buf());
        (storage, dir)
    }

    /// Save identity + repo config, then load both back and verify every field.
    #[test]
    fn full_setup_save_load_cycle() {
        let (storage, _dir) = create_storage();

        let identity = b"AGE-SECRET-KEY-1TEST1234567890ABCDEF";
        storage
            .save_identity(identity)
            .expect("save_identity failed");
        storage
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat-token-123"),
                "/local/repo/path",
            )
            .expect("save_repo_config failed");

        let loaded_identity = storage.load_identity().expect("load_identity failed");
        assert_eq!(
            loaded_identity, identity,
            "identity bytes must round-trip exactly"
        );

        let config = storage.load_repo_config().expect("load_repo_config failed");
        assert_eq!(config.url, "https://example.com/repo.git");
        assert_eq!(config.pat, Some(String::from("pat-token-123")));
        assert_eq!(config.local_path, "/local/repo/path");
    }

    /// Full setup, then clear_all, then reconfigure with different values.
    /// Verifies that clear_all allows a fresh reconfiguration.
    #[test]
    fn clear_all_then_reconfigure() {
        let (storage, _dir) = create_storage();

        // Initial configuration
        storage
            .save_identity(b"AGE-SECRET-KEY-1FIRST")
            .expect("initial save_identity failed");
        storage
            .save_repo_config(
                "https://first.example.com/repo.git",
                Some("first-pat"),
                "/first",
            )
            .expect("initial save_repo_config failed");
        assert!(storage.is_configured(), "should be configured after setup");

        // Clear everything
        storage.clear_all().expect("clear_all failed");
        assert!(
            !storage.is_configured(),
            "should NOT be configured after clear_all"
        );

        // Reconfigure with different values
        storage
            .save_identity(b"AGE-SECRET-KEY-1SECOND")
            .expect("second save_identity failed");
        storage
            .save_repo_config("https://second.example.com/repo.git", None, "/second")
            .expect("second save_repo_config failed");
        assert!(
            storage.is_configured(),
            "should be configured after reconfigure"
        );

        // Verify the new values are loaded (not the old ones)
        let identity = storage
            .load_identity()
            .expect("load_identity after reconfigure failed");
        assert_eq!(identity, b"AGE-SECRET-KEY-1SECOND");

        let config = storage
            .load_repo_config()
            .expect("load_repo_config after reconfigure failed");
        assert_eq!(config.url, "https://second.example.com/repo.git");
        assert_eq!(config.pat, None);
        assert_eq!(config.local_path, "/second");
    }

    /// Write garbage JSON to repo.json, then verify load_repo_config returns
    /// an error with the CONFIG_ERROR code.
    #[test]
    fn corrupted_repo_config_errors() {
        let (storage, dir) = create_storage();

        // Write invalid JSON directly to the config file
        let repo_json_path = dir.path().join("repo.json");
        std::fs::write(&repo_json_path, "{{{{not valid json!!!!")
            .expect("failed to write corrupted config");

        let err = storage
            .load_repo_config()
            .expect_err("loading corrupted config should fail");

        assert_eq!(
            err.code, "CONFIG_ERROR",
            "corrupted JSON must produce CONFIG_ERROR, got: {err:?}"
        );
    }

    /// Save identity with one SecureStorage instance, create a second instance
    /// pointing at the same directory, and verify the second instance can load
    /// the identity. This tests that persistence is directory-based, not
    /// instance-based.
    #[test]
    fn identity_persistence_across_instances() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        // First instance saves the identity
        let storage_a = SecureStorage::new(dir.path().to_path_buf());
        let identity = b"AGE-SECRET-KEY-1PERSIST123";
        storage_a
            .save_identity(identity)
            .expect("save_identity on first instance failed");

        // Second instance (same directory) loads it
        let storage_b = SecureStorage::new(dir.path().to_path_buf());
        let loaded = storage_b
            .load_identity()
            .expect("load_identity on second instance failed");

        assert_eq!(
            loaded, identity,
            "identity must persist across SecureStorage instances"
        );
    }
}
