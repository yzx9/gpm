// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

// API-surface lints (missing_docs, pedantic, …) target library code; tests opt out.
#![allow(
    missing_docs,
    unused_qualifications,
    trivial_casts,
    trivial_numeric_casts,
    clippy::pedantic,
    clippy::indexing_slicing
)]

//! Cross-binary compatibility for the **GPG/OpenPGP** crypto backend against
//! the real `gopass` + `gpg` binaries — the GPG twin of `gopass_interop.rs`
//! (which covers only age).
//!
//! gpm's GPG backend (`GpgBackend` + `crypto::openpgp`) was aligned to gopass by
//! reading gopass's source and by round-tripping rpgp output through the
//! standalone `gpg` CLI (`openpgp::tests::rpgp_encrypts_gpg_decrypts`). What was
//! never exercised end-to-end is a store produced by a real `gopass --crypto gpg`
//! binary flowing through gpm's full `Store` stack, and the mirror direction.
//! These tests close that gap:
//!
//! - **Forward (gopass → gpm):** `gopass init --crypto gpg` + `gopass insert`,
//!   then gpm clones the store, saves the exported GPG identity, and decrypts
//!   the gopass-written `<name>.gpg` through `Store::get`.
//! - **Reverse (gpm → gopass):** gpm's `Store::set` writes a `.gpg` (rpgp
//!   encrypting to the `.public-keys/` recipient), the ciphertext is planted
//!   into gopass's working store, and the real `gopass show` decrypts it.
//!
//! gopass is driven fully non-interactively and isolated into a temp dir (its
//! own `GNUPGHOME` keyring, `GOPASS_HOMEDIR`, mock pinentry) so the developer's
//! real gopass/gpg state is never touched. `HOME` is intentionally NOT overridden
//! — same isolation posture as the age interop suite — so git commits during
//! `gopass init`/`insert` reuse the developer's git identity.
//!
//! Skips gracefully when `gopass`/`gpg` are not on PATH. Needs `gpg-agent` (an
//! `AF_UNIX` socket the build sandbox blocks), so run with the sandbox disabled:
//! `direnv exec . env RUSTC_WRAPPER= SCCACHE_DISABLED=1 cargo test -p rustpass --test gopass_interop_gpg -- --nocapture`

mod common;

mod tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use super::common::{create_test_git_repo_with, expect_fast_forwarded};
    use rustpass::crypto::{CryptoBackend, GpgBackend};
    use rustpass::{GitAuth, Secret, store::Store};
    use tempfile::TempDir;

    /// The fixture UID every throwaway key shares. Its value is irrelevant.
    const KEY_NAME: &str = "gpm interop";
    const KEY_EMAIL: &str = "interop@gpm";

    /// Passphrase for the passphrase-protected key variant, and the value the
    /// mock pinentry hands to gpg-agent via `$PINENTRY_PASSPHRASE`. Its value is
    /// irrelevant; it only has to agree between keygen and a gopass read.
    const PASSPHRASE: &str = "gpm-gpg-interop-passphrase";

    /// A pinentry that always returns `$PINENTRY_PASSPHRASE`. Speaks just enough
    /// of the Assuan protocol for gpg-agent's helper to read a passphrase
    /// without a TTY. Unused for the no-passphrase keys these tests generate,
    /// but kept so a future passphrase-protected variant needs no rewiring.
    const MOCK_PINENTRY: &str = r#"#!/bin/sh
printf 'OK Pleased to meet you\n'
while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in
    GETPIN) printf 'D %s\nOK\n' "$PINENTRY_PASSPHRASE" ;;
    BYE) printf 'OK closing connection\n'; exit 0 ;;
    *) printf 'OK\n' ;;
  esac
done
"#;

    fn gopass_present() -> bool {
        Command::new("gopass")
            .arg("version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            && Command::new("gpg")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
    }

    /// Lay down `home/bin/pinentry` (the mock) and `home/gnupg` (mode 0700) so an
    /// isolated `GNUPGHOME` keeps gopass/gpg off the user's running gpg-agent.
    fn install_mock_pinentry(home: &Path) {
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mock = bin.join("pinentry");
        std::fs::write(&mock, MOCK_PINENTRY).unwrap();
        let mut perm = std::fs::metadata(&mock).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&mock, perm).unwrap();

        let gnupg = home.join("gnupg");
        std::fs::create_dir_all(&gnupg).unwrap();
        let mut gperm = std::fs::metadata(&gnupg).unwrap().permissions();
        gperm.set_mode(0o700);
        std::fs::set_permissions(&gnupg, gperm).unwrap();

        // Pin the mock pinentry explicitly in gpg-agent.conf so the agent uses it
        // regardless of PATH (some distros/builds don't search PATH for pinentry).
        std::fs::write(
            gnupg.join("gpg-agent.conf"),
            format!("pinentry-program {}\n", mock.display()),
        )
        .unwrap();
    }

    /// A `TempDir` carrying an isolated GPG keyring (`<home>/gnupg`) whose
    /// `Drop` kills the `gpg-agent` it spawned. `TempDir`'s drop removes the
    /// socket but leaves the daemon alive, so without this repeated test runs
    /// leak `gpg-agent` processes (each holding the throwaway key in memory).
    struct GpgHome {
        inner: TempDir,
    }

    impl GpgHome {
        /// A fresh isolated GPG home.
        fn new() -> Self {
            Self {
                inner: tempfile::tempdir().unwrap(),
            }
        }
        /// The temp dir path (the keyring lives at `<path>/gnupg`).
        fn path(&self) -> &Path {
            self.inner.path()
        }
    }

    impl Drop for GpgHome {
        fn drop(&mut self) {
            let _ = Command::new("gpgconf")
                .env("GNUPGHOME", self.inner.path().join("gnupg"))
                .args(["--kill", "gpg-agent"])
                .status();
        }
    }

    /// Prepend `home/bin` to PATH (mock pinentry) and resolve the isolated
    /// `GNUPGHOME`. Shared by the gpg and gopass command builders.
    fn env_paths(home: &Path) -> Vec<PathBuf> {
        let mut paths = vec![home.join("bin")];
        if let Ok(p) = std::env::var("PATH") {
            paths.extend(std::env::split_paths(&p));
        }
        paths
    }

    /// A `gpg` command isolated into `home`'s throwaway `GNUPGHOME`, with the
    /// mock-pinentry dir leading PATH so a passphrase prompt (if any) never
    /// reaches a real pinentry or the user's gpg-agent.
    fn gpg(home: &Path, args: &[&str]) -> Command {
        let mut c = Command::new("gpg");
        c.env("GNUPGHOME", home.join("gnupg"));
        c.env("PATH", std::env::join_paths(env_paths(home)).unwrap());
        c.env("PINENTRY_PASSPHRASE", PASSPHRASE);
        // loopback pinentry: a no-passphrase key never prompts; a
        // passphrase-protected key keeps keygen/list/export off gpg-agent's TTY
        // (the passphrase comes from the keygen script or `--passphrase`).
        c.args(["--batch", "--yes", "--pinentry-mode", "loopback"]);
        c.args(args);
        c
    }

    /// A `gopass` command fully isolated into `home`: its config + data dir point
    /// there, `GNUPGHOME` is the throwaway keyring, and the mock-pinentry dir
    /// leads PATH.
    fn gopass(home: &Path, args: &[&str]) -> Command {
        let mut c = Command::new("gopass");
        c.env("GOPASS_CONFIG", home.join("config.yml"));
        c.env("GOPASS_HOMEDIR", home);
        c.env("GNUPGHOME", home.join("gnupg"));
        c.env("PATH", std::env::join_paths(env_paths(home)).unwrap());
        // gopass shells out to gpg, which decrypts through gpg-agent; the mock
        // pinentry reads `$PINENTRY_PASSPHRASE`, so a passphrase-protected key
        // decrypts without a TTY.
        c.env("PINENTRY_PASSPHRASE", PASSPHRASE);
        // Refuse any interactive git prompt (auth, merge editor) rather than
        // hang the test binary; gopass shells out to git for push/pull/sync.
        c.env("GIT_TERMINAL_PROMPT", "0");
        c.args(args);
        c
    }

    /// `gpg --batch --gen-key` a keypair into `home`'s throwaway keyring.
    /// `ecc` picks EdDSA+Curve25519 (a gpg `--expert` ECC key); the default is
    /// RSA/RSA-2048 — gopass's `GenerateIdentity` key shape (RSA-2048 primary
    /// sign+cert, RSA-2048 encrypt subkey). `passphrase` S2K-protects the key
    /// (the gopass `setup` default); `None` emits `%no-protection`. Returns the
    /// primary fingerprint (40 hex).
    fn gpg_keygen(home: &Path, ecc: bool, passphrase: Option<&str>) -> String {
        // ECC (EdDSA primary + ECDH subkey) vs RSA-2048 (gopass's default).
        let (key_type, key_param, sub_type, sub_param) = if ecc {
            (
                "EDDSA",
                "Key-Curve: ed25519",
                "ECDH",
                "Subkey-Curve: cv25519",
            )
        } else {
            ("RSA", "Key-Length: 2048", "RSA", "Subkey-Length: 2048")
        };
        let protection = match passphrase {
            Some(p) => format!("Passphrase: {p}"),
            None => "%no-protection".to_string(),
        };
        let script = format!(
            "Key-Type: {key_type}\n\
             {key_param}\n\
             Key-Usage: sign,cert\n\
             Subkey-Type: {sub_type}\n\
             {sub_param}\n\
             Subkey-Usage: encrypt\n\
             Name-Real: {KEY_NAME}\n\
             Name-Email: {KEY_EMAIL}\n\
             {protection}\n\
             %commit\n"
        );
        let script_path = home.join("keygen.txt");
        std::fs::write(&script_path, script).unwrap();
        let out = gpg(home, &["--gen-key"])
            .arg(script_path.to_str().unwrap())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "gpg keygen failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // Primary fingerprint = first fpr line from --with-colons.
        let list = gpg(home, &["--list-keys", "--with-colons", KEY_EMAIL])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&list.stdout);
        let fingerprint = stdout
            .lines()
            .find(|l| l.starts_with("fpr:"))
            .and_then(|l| l.split(':').nth(9))
            .unwrap_or_else(|| panic!("no fpr after keygen: {stdout}"))
            .to_string();
        // Pre-start gpg-agent so the first decrypt under parallel test load
        // doesn't race the agent auto-start ("no running gpg-agent").
        let _ = Command::new("gpgconf")
            .env("GNUPGHOME", home.join("gnupg"))
            .args(["--launch", "gpg-agent"])
            .status();
        fingerprint
    }

    /// `gpg --armor --export-secret-key` → the armored secret key gpm stores as
    /// its GPG identity (the form `Store::save_identity` accepts). The S2K layer
    /// is preserved verbatim (gpg exports without unlocking). A passphrase key's
    /// export cooperates with gpg-agent via loopback + `--passphrase`, avoiding
    /// the agent's pinentry timeout.
    fn export_secret_key(home: &Path, passphrase: Option<&str>) -> String {
        // gpg wants options before the command — a trailing `--passphrase` after
        // the key id is rejected as "not an option", so assemble argv in order.
        let mut args: Vec<String> = vec!["--armor".to_string()];
        if let Some(pw) = passphrase {
            args.push(format!("--passphrase={pw}"));
        }
        args.push("--export-secret-key".to_string());
        args.push(KEY_EMAIL.to_string());
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = gpg(home, &refs).output().unwrap();
        assert!(
            out.status.success(),
            "export-secret-key failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// `gopass insert -f <name>` reading `plaintext` from stdin. gopass encrypts
    /// to the `.gpg-id` recipient (the key we generated) and stores the bytes
    /// verbatim under `<name>.gpg`.
    fn gopass_insert(home: &Path, name: &str, plaintext: &str) {
        let mut cmd = gopass(home, &["insert", "-f", name]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("spawn gopass insert");
        {
            let mut stdin = child.stdin.take().expect("piped stdin");
            stdin.write_all(plaintext.as_bytes()).expect("write secret");
        }
        let out = child.wait_with_output().expect("wait gopass insert");
        assert!(
            out.status.success(),
            "gopass insert {name:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// `gopass show <name>` through gopass's real GPG decrypt path, returning
    /// stdout (password on line 1, then the rendered body).
    fn gopass_show(home: &Path, name: &str) -> String {
        let out = gopass(home, &["show", name]).output().unwrap();
        assert!(
            out.status.success(),
            "gopass show {name:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Commit `store`'s worktree with an explicit (gpgsign-off) identity — gpm
    /// clones HEAD, not the worktree, so a gopass write must be committed before
    /// a clone. gopass itself commits on `init`/`insert`, though, so a clean tree
    /// is expected and tolerated (git writes "nothing to commit" to **stdout**,
    /// hence the combined-output check). Mirrors the age interop suite's
    /// `commit_worktree`/`commit_worktree` split, unified.
    fn commit_worktree(store: &Path) {
        let add = Command::new("git")
            .arg("-C")
            .arg(store)
            .args(["add", "-A"])
            .output()
            .unwrap();
        assert!(
            add.status.success(),
            "git add failed: {}",
            String::from_utf8_lossy(&add.stderr)
        );
        let commit = Command::new("git")
            .arg("-C")
            .arg(store)
            // LC_ALL=C keeps git's "nothing to commit" English (the check below
            // matches English literals); GIT_TERMINAL_PROMPT=0 refuses any
            // interactive prompt instead of hanging the test binary.
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=gpm-interop",
                "-c",
                "user.email=interop@gpm",
                "commit",
                "-m",
                "gpm gpg interop test",
            ])
            .output()
            .unwrap();
        if !commit.status.success() {
            // gopass already committed (init/insert) → clean tree. Anything else
            // is a real failure.
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&commit.stdout),
                String::from_utf8_lossy(&commit.stderr)
            );
            assert!(
                combined.contains("nothing to commit") || combined.contains("no changes"),
                "git commit failed: {combined}"
            );
        }
    }

    /// Provision an isolated gopass **GPG** store with a chosen key shape.
    /// `ecc`/`passphrase` select the keygen profile; the store is `gopass init
    /// --crypto gpg --storage gitfs` + a bootstrap commit. Returns `(home,
    /// store_dir, identity_armor, recipient_token)` — `identity_armor` is the
    /// exported secret key and `recipient_token` the literal string gopass wrote
    /// into `.gpg-id` (`0x` + last 16 hex of the primary fingerprint).
    fn provision_gopass_gpg_store_with(
        ecc: bool,
        passphrase: Option<&str>,
    ) -> (GpgHome, PathBuf, String, String) {
        let home = GpgHome::new();
        install_mock_pinentry(home.path());

        let _fingerprint = gpg_keygen(home.path(), ecc, passphrase);

        let store_dir = home.path().join("store");
        let init = gopass(
            home.path(),
            &[
                "--yes",
                "init",
                "--crypto",
                "gpg",
                "--storage",
                "gitfs",
                "--path",
                store_dir.to_str().unwrap(),
            ],
        )
        .output()
        .unwrap();
        assert!(
            init.status.success(),
            "gopass init --crypto gpg failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        commit_worktree(&store_dir);

        let recipient_token = std::fs::read_to_string(store_dir.join(".gpg-id"))
            .unwrap()
            .lines()
            .find(|l| !l.trim().is_empty())
            .expect("gopass init wrote a .gpg-id recipient")
            .trim()
            .to_string();

        let identity_armor = export_secret_key(home.path(), passphrase);

        (home, store_dir, identity_armor, recipient_token)
    }

    /// The default profile: RSA/RSA-2048, no passphrase (the shape the forward
    /// and reverse interop tests use).
    fn provision_gopass_gpg_store() -> (GpgHome, PathBuf, String, String) {
        provision_gopass_gpg_store_with(false, None)
    }

    /// A shared bare git remote plus an isolated gopass **GPG** store wired to
    /// it as `origin`, used by the git-sync round-trip test. The gopass key is
    /// generated in the throwaway keyring and exported as `identity` (the gpm
    /// side saves it to decrypt). The `TempDir` fields anchor lifetimes.
    struct SharedBareGpg {
        _home: GpgHome,
        _bare_dir: TempDir,
        home_path: PathBuf,
        bare_path: PathBuf,
        identity: String,
    }

    /// Provision a [`SharedBareGpg`]: a gopass GPG store, an empty bare repo,
    /// gopass wired to the bare as `origin`, the bootstrap commit pushed, and
    /// the bare HEAD re-pointed at gopass's branch. The bare-HEAD re-point is
    /// load-bearing — gpm's clone follows bare HEAD, and a fresh `init --bare`
    /// defaults HEAD to git's compile-time default branch, which may not match
    /// gopass's, silently making gpm's later push/pull a no-op. Mirrors the age
    /// interop suite's `provision_shared_bare_interop`.
    fn provision_shared_bare_gpg() -> SharedBareGpg {
        let home = GpgHome::new();
        install_mock_pinentry(home.path());
        gpg_keygen(home.path(), false, None);

        let store_dir = home.path().join("store");
        let store_str = store_dir.to_str().unwrap();
        let init = gopass(
            home.path(),
            &[
                "--yes",
                "init",
                "--crypto",
                "gpg",
                "--storage",
                "gitfs",
                "--path",
                store_str,
            ],
        )
        .output()
        .unwrap();
        assert!(
            init.status.success(),
            "gopass init --crypto gpg failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        commit_worktree(&store_dir);

        let bare_dir = tempfile::tempdir().unwrap();
        let bare_path = bare_dir.path().to_path_buf();
        let bare_str = bare_path.to_str().unwrap();

        let init_bare = Command::new("git")
            .args(["init", "--bare", bare_str])
            .output()
            .unwrap();
        assert!(
            init_bare.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init_bare.stderr)
        );

        let remote = Command::new("git")
            .arg("-C")
            .arg(store_str)
            .args(["remote", "add", "origin", bare_str])
            .output()
            .unwrap();
        assert!(
            remote.status.success(),
            "git remote add failed: {}",
            String::from_utf8_lossy(&remote.stderr)
        );

        // Detect gopass's branch (do NOT assume main/master).
        let branch_out = Command::new("git")
            .arg("-C")
            .arg(store_str)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap();
        assert!(branch_out.status.success(), "rev-parse HEAD failed");
        let branch = String::from_utf8_lossy(&branch_out.stdout)
            .trim()
            .to_owned();

        let push = Command::new("git")
            .arg("-C")
            .arg(store_str)
            .args(["push", "-u", "origin", &branch])
            .output()
            .unwrap();
        assert!(
            push.status.success(),
            "bootstrap push failed: {}",
            String::from_utf8_lossy(&push.stderr)
        );

        // Re-point bare HEAD at gopass's branch so gpm's clone lands on it.
        let sym = Command::new("git")
            .arg("-C")
            .arg(bare_str)
            .args(["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])
            .output()
            .unwrap();
        assert!(
            sym.status.success(),
            "symbolic-ref HEAD failed: {}",
            String::from_utf8_lossy(&sym.stderr)
        );

        let identity = export_secret_key(home.path(), None);
        let home_path = home.path().to_path_buf();

        SharedBareGpg {
            _home: home,
            _bare_dir: bare_dir,
            home_path,
            bare_path,
            identity,
        }
    }

    /// Split a secret plaintext into `(password, free-text body)` the way gpm's
    /// parser and gopass's `Body()` do: first line is the password; the body is
    /// every subsequent line that is NOT a `Key: Value` pair (the free-text
    /// notes), trailing newline trimmed. Attribute lines (`": "`) are excluded —
    /// gpm models them as structured attributes, not body (R069 phase 2b).
    fn expected_password_body(plaintext: &str) -> (&str, String) {
        match plaintext.split_once('\n') {
            Some((pw, body)) => {
                let free_text: String = body
                    .split('\n')
                    .filter(|l| !l.contains(": "))
                    .collect::<Vec<_>>()
                    .join("\n");
                (pw, free_text.trim_end_matches('\n').to_string())
            }
            None => (plaintext, String::new()),
        }
    }

    /// The structured `Key: Value` attribute pairs a plaintext carries, the way
    /// gpm's `Secret::parse` (gopass AKV) models them: the password is line 0;
    /// every later line containing the `": "` separator is one attribute — key
    /// before the first `": "`, value after. Pairs `expected_password_body` so
    /// the interop tests pin BOTH halves of the R069 body/attribute split, not
    /// just the free-text body.
    fn expected_attributes(plaintext: &str) -> Vec<(&str, &str)> {
        plaintext
            .split('\n')
            .skip(1)
            .filter_map(|l| l.split_once(": "))
            .collect()
    }

    /// Assert the decrypted secret's attribute region matches the plaintext's
    /// `Key: Value` pairs — count, then each key/value (gopass `Body()`/AKV
    /// parity).
    fn assert_attributes(secret: &Secret, plaintext: &str, name: &str) {
        let want = expected_attributes(plaintext);
        assert_eq!(
            secret.attributes().len(),
            want.len(),
            "{name}: attribute count mismatch"
        );
        for (k, v) in want {
            assert_eq!(
                secret.attribute_str(k),
                Some(v),
                "{name}: attribute {k:?} mismatch"
            );
        }
    }

    /// **Forward interop (gopass → gpm):** a store created and populated by the
    /// real `gopass --crypto gpg` binary is cloned by gpm, and gpm decrypts the
    /// gopass-written `<name>.gpg` through the full `Store::get` stack (clone →
    /// save GPG identity → unlock → list → decrypt). This is the GPG twin of
    /// `gopass_interop::gpm_decrypts_secrets_written_by_real_gopass`.
    ///
    /// Also pins recipient-token agreement: the id gpm derives from the exported
    /// key (`identity_recipient`, `0x` + last 16 hex) must byte-match the token
    /// gopass wrote into `.gpg-id` — a case/length drift here would silently
    /// break `.public-keys/<token>` resolution on a non-gopass-created store.
    #[tokio::test]
    async fn gpm_decrypts_gpg_secret_written_by_real_gopass() {
        if !gopass_present() {
            eprintln!("skipping gpg interop test: `gopass`/`gpg` not on PATH");
            return;
        }

        let (home, store_dir, identity_armor, _recipient_token) = provision_gopass_gpg_store();

        // Several secret shapes gopass writes and gpm must decrypt + parse back
        // identically (password-only, multiline AKV, non-ASCII, dotted name).
        let cases: &[(&str, &str)] = &[
            ("test/password-only", "s3cret"),
            (
                "test/multiline",
                "hunter2\nuser: alice\nurl: https://example.com",
            ),
            ("test/unicode", "pässwörd\nnote: 日本語 emoji 🔑"),
            ("svc/api.key", "dot-pw\nscope: read"),
        ];
        for (name, plaintext) in cases {
            gopass_insert(home.path(), name, plaintext);
        }
        commit_worktree(&store_dir);

        // gpm clones the gopass store directly (local file transport, no auth).
        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .clone_only(store_dir.to_str().unwrap(), &GitAuth::None)
            .await
            .expect("gpm clones the gopass GPG store");

        // Save the GPG identity: classify → PgpSecretKey → membership gate
        // (fingerprint match against .public-keys) → persist crypto=gpg.
        store
            .save_identity(&identity_armor, None)
            .await
            .expect("save_identity accepts the GPG key matching .gpg-id");
        // No-passphrase key: unlock("") strips nothing but populates the
        // identity cache `Store::get` requires for a PgpSecretKey identity.
        store.unlock("").await.expect("unlock the plain GPG key");

        // Recipient-token CASE agreement is pinned separately by
        // `gpm_recipient_token_case_matches_gopass` (gpm now uppercases the hex
        // it derives, matching gopass/gpg). It never blocked this test: gpm reads
        // gopass's verbatim uppercase token from `.gpg-id`/`.public-keys/` rather
        // than re-deriving it, so reading a gopass store is unaffected — the case
        // only mattered for gpm-side token generation.

        // Structural compat: gpm lists exactly the entries gopass wrote.
        let entries: Vec<String> = store
            .list(0, usize::MAX)
            .await
            .expect("gpm lists the cloned gopass GPG store")
            .entries
            .into_iter()
            .map(|e| e.name)
            .collect();
        for (name, _) in cases {
            assert!(
                entries.iter().any(|e| e == name),
                "gpm should list the gopass-written GPG entry {name}; got {entries:?}"
            );
        }

        // Full-stack compat: gpm decrypts each entry and parses the body back to
        // exactly what gopass stored.
        for (name, plaintext) in cases {
            let secret = store
                .get(name)
                .await
                .unwrap_or_else(|e| panic!("gpm decrypts gopass-written {name}: {e:?}"));
            let (pw, body) = expected_password_body(plaintext);
            assert_eq!(secret.password(), pw, "password mismatch for {name}");
            assert_eq!(secret.body(), body.as_str(), "body mismatch for {name}");
            assert_attributes(&secret, plaintext, name);
        }
    }

    /// **Reverse interop (gpm writes → gopass decrypts):** a `.gpg` secret
    /// written by gpm's real `Store::set` (rpgp encrypting to the `.public-keys/`
    /// recipient) is planted into gopass's working store and decrypted by the
    /// real `gopass show`. Pins that gpm's rpgp-produced GPG ciphertext is
    /// readable by gopass/gpg — the direction the fixture-only
    /// `openpgp::tests::rpgp_encrypts_gpg_decrypts` leaves at the unit level.
    #[tokio::test]
    async fn gopass_decrypts_gpg_secret_written_by_gpm() {
        if !gopass_present() {
            eprintln!("skipping gpg interop test: `gopass`/`gpg` not on PATH");
            return;
        }

        let (home, store_dir, identity_armor, _recipient) = provision_gopass_gpg_store();

        // gpm clones the gopass store directly (the forward test's pattern).
        let config_dir = tempfile::tempdir().unwrap();
        let gpm_repo = config_dir.path().join("repo");
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .clone_only(store_dir.to_str().unwrap(), &GitAuth::None)
            .await
            .expect("gpm clones the gopass GPG store");
        store
            .save_identity(&identity_armor, None)
            .await
            .expect("save_identity accepts the GPG key");
        store.unlock("").await.expect("unlock the plain GPG key");
        // No origin → local-only writes (avoid the push path).
        store.set_autosync(false);

        let cases: &[(&str, &str)] = &[
            ("gpm/password-only", "s3cret"),
            (
                "gpm/multiline",
                "hunter2\nuser: alice\nurl: https://example.com",
            ),
            ("gpm/unicode", "pässwörd\nnote: 日本語 emoji 🔑"),
        ];
        for (name, plaintext) in cases {
            store
                .set(name, plaintext.as_bytes())
                .await
                .unwrap_or_else(|e| panic!("gpm writes GPG secret {name}: {e:?}"));
        }

        // Plant each gpm-written ciphertext into gopass's working store — the
        // only way to get gpm-written bytes in front of gopass's reader without
        // a shared bare. gopass reads from its own worktree.
        for (name, _plaintext) in cases {
            let src = gpm_repo.join(format!("{name}.gpg"));
            let dst = store_dir.join(format!("{name}.gpg"));
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::copy(&src, &dst).unwrap();
        }
        commit_worktree(&store_dir);

        for (name, plaintext) in cases {
            let gopass_out = gopass_show(home.path(), name);
            let (gopass_pw, gopass_body) = expected_password_body(&gopass_out);
            let (want_pw, want_body) = expected_password_body(plaintext);
            assert_eq!(
                gopass_pw, want_pw,
                "{name}: gopass password from gpm-written GPG bytes"
            );
            assert_eq!(
                gopass_body, want_body,
                "{name}: gopass body from gpm-written GPG bytes"
            );
        }
    }

    /// **Passphrase-protected identity (gopass `setup` default):** a key
    /// generated by real `gpg` with an S2K passphrase — the shape `gopass setup`
    /// (`GenerateIdentity`) always produces — is exported, saved as gpm's
    /// identity, unlocked (S2K strip), and used to decrypt a gopass-written
    /// secret. The forward/reverse interop tests above use a no-passphrase key,
    /// which sidesteps gpm's S2K unlock path; this exercises the real gopass
    /// setup shape end-to-end and pins that gpm's `strip_passphrase` agrees with
    /// gpg's iterated+salted S2K.
    #[tokio::test]
    async fn gpm_unlocks_passphrase_protected_gpg_key() {
        if !gopass_present() {
            eprintln!("skipping gpg interop test: `gopass`/`gpg` not on PATH");
            return;
        }
        let (home, store_dir, identity_armor, _recipient) =
            provision_gopass_gpg_store_with(false, Some(PASSPHRASE));

        // gopass writes a secret encrypted to the passphrase key.
        let plaintext = "s3cret\nuser: alice\nnote: passphrase key";
        gopass_insert(home.path(), "test/secret", plaintext);
        commit_worktree(&store_dir);

        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .clone_only(store_dir.to_str().unwrap(), &GitAuth::None)
            .await
            .expect("gpm clones the gopass GPG store");
        // The exported armor carries the S2K layer; save_identity stores it as-is.
        store
            .save_identity(&identity_armor, None)
            .await
            .expect("save_identity accepts the passphrase-protected GPG key");
        // unlock strips the S2K layer with the passphrase — the gopass setup path.
        store
            .unlock(PASSPHRASE)
            .await
            .expect("unlock strips gpg's S2K layer");

        let secret = store
            .get("test/secret")
            .await
            .expect("gpm decrypts through the unlocked passphrase key");
        assert_eq!(secret.password(), "s3cret");
        // R069: the `Key: Value` tail lines parse as attributes, not body
        // (gopass `Body()` parity), so the free-text body is empty.
        assert_eq!(secret.body(), "");
        assert_attributes(&secret, plaintext, "test/secret");
    }

    /// **ECC identity:** a key generated by real `gpg` as EdDSA primary +
    /// Curve25519 ECDH subkey (a `gpg --expert` ECC key) is exported, saved, and
    /// used to decrypt a gopass-written secret. gopass `setup` defaults to RSA,
    /// but a user with an existing ECC key uses it; this pins that gpm handles a
    /// real gpg-produced ECC identity (parse + ECDH decrypt), not just RSA.
    #[tokio::test]
    async fn gpm_handles_ecc_gpg_key() {
        if !gopass_present() {
            eprintln!("skipping gpg interop test: `gopass`/`gpg` not on PATH");
            return;
        }
        let (home, store_dir, identity_armor, _recipient) =
            provision_gopass_gpg_store_with(true, None);

        let plaintext = "ecc-pw\nuser: bob\nnote: elliptic-curve key";
        gopass_insert(home.path(), "test/secret", plaintext);
        commit_worktree(&store_dir);

        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .clone_only(store_dir.to_str().unwrap(), &GitAuth::None)
            .await
            .expect("gpm clones the gopass GPG store");
        store
            .save_identity(&identity_armor, None)
            .await
            .expect("save_identity accepts the ECC GPG key");
        store.unlock("").await.expect("unlock the plain ECC key");

        let secret = store
            .get("test/secret")
            .await
            .expect("gpm decrypts through the ECC key");
        assert_eq!(secret.password(), "ecc-pw");
        // R069: the `Key: Value` tail lines parse as attributes, not body
        // (gopass `Body()` parity), so the free-text body is empty.
        assert_eq!(secret.body(), "");
        assert_attributes(&secret, plaintext, "test/secret");
    }

    /// **git-sync round-trip (GPG):** gpm and the real `gopass` binary push and
    /// pull a GPG-encrypted store to one shared bare git remote, each via its
    /// own sync path — the GPG twin of the age suite's
    /// `gpm_and_gopass_sync_through_shared_bare_remote`. This exercises the
    /// transport/wire layer the format-only forward/reverse tests do not cover:
    /// gopass's git conventions (commit messages, `.gitattributes`'s `*.gpg
    /// diff=gpg`) flowing into gpm's git sync, and gpm's commits flowing back
    /// into gopass. Every step is a clean fast-forward (between any pull and the
    /// next push the bare advances exactly once), so divergence is unreachable.
    #[tokio::test]
    async fn gpm_and_gopass_sync_gpg_store_through_shared_bare() {
        if !gopass_present() {
            eprintln!("skipping gpg interop test: `gopass`/`gpg` not on PATH");
            return;
        }
        let env = provision_shared_bare_gpg();

        // gpm clones the shared bare — lands on bare HEAD (gopass's branch).
        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .clone_only(env.bare_path.to_str().unwrap(), &GitAuth::None)
            .await
            .expect("gpm clones the shared bare");
        store
            .save_identity(&env.identity, None)
            .await
            .expect("save_identity accepts the gopass GPG key");
        store.unlock("").await.expect("unlock the plain key");

        // ── Direction 1: gopass writes + pushes → gpm pulls ───────────────────
        let gopass_entry = "gopass-side/secret";
        let gopass_plaintext = "from-gopass\nkind: written by real gopass";
        gopass_insert(env.home_path.as_path(), gopass_entry, gopass_plaintext);
        let push = gopass(env.home_path.as_path(), &["git", "push"])
            .output()
            .unwrap();
        assert!(
            push.status.success(),
            "gopass git push failed: {}",
            String::from_utf8_lossy(&push.stderr)
        );

        let outcome = store.sync().await.expect("gpm pulls");
        expect_fast_forwarded(outcome);

        let secret = store
            .get(gopass_entry)
            .await
            .expect("gpm decrypts the gopass-pushed GPG entry");
        let (want_pw, want_body) = expected_password_body(gopass_plaintext);
        assert_eq!(secret.password(), want_pw);
        assert_eq!(secret.body(), want_body.as_str());
        assert_attributes(&secret, gopass_plaintext, gopass_entry);

        // ── Direction 2: gpm writes + pushes → gopass pulls ───────────────────
        // gpm's HEAD == bare tip (just pulled) and nothing pushed in between, so
        // gpm's commit + push is a clean fast-forward.
        let gpm_entry = "gpm-side/secret";
        let gpm_plaintext = "from-gpm\nkind: written by gpm Store::set";
        store
            .set(gpm_entry, gpm_plaintext.as_bytes())
            .await
            .expect("gpm writes");
        store.push().await.expect("gpm pushes");

        let pull = gopass(env.home_path.as_path(), &["git", "pull"])
            .output()
            .unwrap();
        assert!(
            pull.status.success(),
            "gopass git pull failed: {}",
            String::from_utf8_lossy(&pull.stderr)
        );

        // Cross-decrypt via the real gopass binary — proves gopass both resolves
        // and decrypts gpm's pushed GPG entry.
        let gopass_out = gopass_show(env.home_path.as_path(), gpm_entry);
        let (got_pw, got_body) = expected_password_body(&gopass_out);
        let (want_pw, want_body) = expected_password_body(gpm_plaintext);
        assert_eq!(got_pw, want_pw, "gopass must decrypt gpm-pushed password");
        assert_eq!(got_body, want_body, "gopass must render gpm-pushed body");
    }

    /// **Multiple recipients:** a store whose `.gpg-id` lists TWO recipients (a
    /// gopass team store) must be encrypted to BOTH by gpm — one PKESK per
    /// recipient — and each recipient's key decrypts the result. The
    /// forward/reverse/sync tests above use a single recipient; this pins the
    /// multi-recipient PKESK shape against a real two-key store. The store is
    /// assembled by hand (`.gpg-id` + two `.public-keys/` entries committed)
    /// rather than `gopass recipients add`, to isolate the gpm encryption
    /// question from gopass's recipient-add ceremony.
    #[tokio::test]
    async fn gpm_encrypts_to_every_gpg_id_recipient() {
        if !gopass_present() {
            eprintln!("skipping gpg interop test: `gopass`/`gpg` not on PATH");
            return;
        }
        // Two throwaway keyrings, one key each.
        let home_a = GpgHome::new();
        let home_b = GpgHome::new();
        install_mock_pinentry(home_a.path());
        install_mock_pinentry(home_b.path());
        gpg_keygen(home_a.path(), false, None);
        gpg_keygen(home_b.path(), false, None);

        let armor_a = export_secret_key(home_a.path(), None);
        let armor_b = export_secret_key(home_b.path(), None);
        // gpm-derived recipient tokens become the `.public-keys/` filenames.
        let token_a = GpgBackend
            .identity_recipient(&armor_a, None)
            .expect("derive token A");
        let token_b = GpgBackend
            .identity_recipient(&armor_b, None)
            .expect("derive token B");
        // Armored public halves — gopass's `.public-keys/<token>` blob format.
        let pub_a = gpg(home_a.path(), &["--armor", "--export", KEY_EMAIL])
            .output()
            .unwrap();
        assert!(
            pub_a.status.success(),
            "export pub A failed: {}",
            String::from_utf8_lossy(&pub_a.stderr)
        );
        let pub_b = gpg(home_b.path(), &["--armor", "--export", KEY_EMAIL])
            .output()
            .unwrap();
        assert!(
            pub_b.status.success(),
            "export pub B failed: {}",
            String::from_utf8_lossy(&pub_b.stderr)
        );

        // Hand-built two-recipient store committed to a git repo gpm can clone.
        let gpg_id = format!("{token_a}\n{token_b}\n");
        let pa_path = format!(".public-keys/{token_a}");
        let pb_path = format!(".public-keys/{token_b}");
        let (bare, _repo) = create_test_git_repo_with(
            vec![],
            vec![
                (".gpg-id", gpg_id.as_bytes()),
                (pa_path.as_str(), pub_a.stdout.as_slice()),
                (pb_path.as_str(), pub_b.stdout.as_slice()),
            ],
            // Unused dummy — `entries` is empty so nothing is age-encrypted; the
            // recipient_str is required by create_test_git_repo_with's signature.
            "age1qcpwGY9xztuw39d8pe8cx3uyhu2v8pz39f6tje0x06d8tnz5eyqqt8z6e2",
        );

        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .clone_only(bare.path().to_str().unwrap(), &GitAuth::None)
            .await
            .expect("gpm clones the two-recipient store");
        store
            .save_identity(&armor_a, None)
            .await
            .expect("save_identity accepts recipient A's key");
        store.unlock("").await.expect("unlock recipient A");
        // No origin → local-only write.
        store.set_autosync(false);

        let plaintext = b"multi-pw\nk: v";
        store
            .set("multi", plaintext)
            .await
            .expect("gpm writes a multi-recipient secret");

        // The written ciphertext carries one PKESK per recipient (two).
        let written = config_dir.path().join("repo").join("multi.gpg");
        let packets = gpg(
            home_a.path(),
            &["--list-packets", written.to_str().unwrap()],
        )
        .output()
        .unwrap();
        assert!(
            packets.status.success(),
            "gpg --list-packets failed: {}",
            String::from_utf8_lossy(&packets.stderr)
        );
        let pkesk_count = String::from_utf8_lossy(&packets.stdout)
            .matches("pubkey enc packet")
            .count();
        assert_eq!(
            pkesk_count, 2,
            "gpm must emit one PKESK per recipient (got {pkesk_count})"
        );

        // gpm (recipient A) decrypts its own write.
        let secret = store
            .get("multi")
            .await
            .expect("recipient A decrypts the multi-recipient write");
        assert_eq!(secret.password(), "multi-pw");
        // R069: the `k: v` line parses as an attribute, not body (gopass
        // `Body()` parity), so the free-text body is empty.
        assert_eq!(secret.body(), "");
        assert_attributes(&secret, std::str::from_utf8(plaintext).unwrap(), "multi");

        // Recipient B (a separate keyring) also decrypts — real gpg.
        let dec_b = gpg(home_b.path(), &["--decrypt", written.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            dec_b.status.success(),
            "recipient B must decrypt gpm-written multi-recipient file: {}",
            String::from_utf8_lossy(&dec_b.stderr)
        );
        assert_eq!(dec_b.stdout, plaintext);
    }

    /// **Recipient-token case (gopass compatibility, #41).** The id gpm derives
    /// from a key (`identity_recipient`, `0x` + last 16 hex of the primary
    /// fingerprint) must byte-match the token gopass writes into `.gpg-id` and
    /// uses as the `.public-keys/<token>` filename. gopass/gpg emit UPPERCASE hex
    /// (`gpg --with-colons`'s fpr field, and gopass's `Key.ID()` = `0x` + that);
    /// rpgp's `Fingerprint` Display is lowercase, so gpm canonicalizes to
    /// uppercase in `fingerprint_hex` — this pins that agreement end-to-end
    /// against the real gopass binary. (Reading an existing store already worked:
    /// gpm uses gopass's verbatim token rather than re-deriving it; the fix
    /// matters for gpm-side token GENERATION — creating a store, the
    /// `.public-keys/` filename, the setup preview recipient.)
    #[tokio::test]
    async fn gpm_recipient_token_case_matches_gopass() {
        if !gopass_present() {
            eprintln!("skipping gpg interop test: `gopass`/`gpg` not on PATH");
            return;
        }
        let (_home, _store_dir, identity_armor, recipient_token) = provision_gopass_gpg_store();
        let derived = GpgBackend
            .identity_recipient(&identity_armor, None)
            .expect("derive recipient from exported key");
        assert_eq!(
            derived, recipient_token,
            "gpm-derived recipient token must match gopass's .gpg-id token (both uppercase)"
        );
    }

    /// **Recipient-token case, GENERATION side (#41).** The mirror of
    /// [`gpm_recipient_token_case_matches_gopass`] (the read side). This pins the
    /// WRITE side: a store whose `.gpg-id` / `.public-keys/<token>` carry gpm's
    /// `identity_recipient` output must be resolved by the real gopass —
    /// `FindKey` matches by fingerprint suffix, case-sensitive, against the
    /// uppercase hex gpg emits, so gpm must emit uppercase too. A store gpm
    /// creates (or a recipient gpm adds) is readable by gopass once #41 is fixed.
    #[tokio::test]
    async fn gopass_resolves_store_built_from_gpm_recipient_token() {
        if !gopass_present() {
            eprintln!("skipping gpg interop test: `gopass`/`gpg` not on PATH");
            return;
        }
        // Provision a gopass GPG store — gopass writes the CORRECT uppercase token.
        let (home, store_dir, identity, gopass_token) = provision_gopass_gpg_store();
        // gpm-derived token for the SAME key — uppercase after #41.
        let gpm_token = GpgBackend
            .identity_recipient(&identity, None)
            .expect("derive token from the exported key");

        // Mimic a store gpm would create / a recipient gpm would add: rewrite
        // `.gpg-id` and the `.public-keys/` filename to gpm's token, reusing the
        // same armored pubkey gopass exported.
        let old_pubkey = store_dir.join(format!(".public-keys/{gopass_token}"));
        let pubkey = std::fs::read(&old_pubkey).unwrap();
        let _ = std::fs::remove_file(&old_pubkey);
        std::fs::write(store_dir.join(".gpg-id"), format!("{gpm_token}\n")).unwrap();
        std::fs::write(store_dir.join(format!(".public-keys/{gpm_token}")), pubkey).unwrap();
        commit_worktree(&store_dir);

        // gopass must resolve the gpm-written recipient to encrypt — its
        // case-sensitive fingerprint-suffix match succeeds because gpm emits
        // uppercase (#41), so gpm_token == gopass_token.
        let mut cmd = gopass(home.path(), &["insert", "-f", "gen/x"]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("spawn gopass insert");
        {
            let mut stdin = child.stdin.take().expect("piped stdin");
            stdin.write_all(b"v").expect("write secret");
        }
        let out = child.wait_with_output().expect("wait gopass insert");
        assert!(
            out.status.success(),
            "gopass must resolve a gpm-generated recipient token; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
