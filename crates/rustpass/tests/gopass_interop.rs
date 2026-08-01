// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Cross-binary compatibility against the real `gopass` binary (age backend).
//!
//! gpm mirrors gopass's on-disk formats, but until now that alignment was
//! asserted only by reading gopass's source and by round-tripping gpm's own
//! output through the standalone `age` CLI. These tests close the remaining
//! gap: a store produced by a real `gopass` binary is cloned and decrypted by
//! gpm's full read stack — recipients parse, git clone, age decrypt, secret
//! body parse.
//!
//! gopass is driven fully non-interactively and isolated into a temp dir so the
//! developer's real gopass config is never touched. gopass encrypts its age
//! identity at rest and prompts for that passphrase via pinentry on every read;
//! we install a mock pinentry returning a fixed passphrase. gopass's recipient
//! machinery rejects an arbitrary pasted recipient, so gpm's recipient is
//! written directly into the store's recipients file — gopass's own format —
//! which gopass honors on every insert.
//!
//! Skips gracefully when `gopass` is not on PATH.

mod common;

mod tests {
    use super::common::generate_test_keypair;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use rustpass::store::Store;

    /// Passphrase the mock pinentry hands back to gopass, and that gopass uses to
    /// protect the throwaway identity it generates. Its value is irrelevant; it
    /// just has to be non-empty and agree between keygen and reads.
    const PIN: &str = "gpm-interop-test-passphrase";

    /// A pinentry that always returns `$PINENTRY_PASSPHRASE`. Speaks just enough
    /// of the Assuan protocol (greet, ACK everything, answer GETPIN, exit on BYE)
    /// for gopass's age askpass to read its identity passphrase without a TTY.
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
    }

    /// Write the mock pinentry into `home/bin` and mark it executable.
    fn install_mock_pinentry(home: &Path) {
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mock = bin.join("pinentry");
        std::fs::write(&mock, MOCK_PINENTRY).unwrap();
        let mut perm = std::fs::metadata(&mock).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&mock, perm).unwrap();
    }

    /// Build a `gopass` command fully isolated into `home`: its config and data
    /// dir point there, and the mock-pinentry dir leads PATH so identity reads
    /// never reach a real pinentry or the user's gpg-agent.
    fn gopass(home: &Path, args: &[&str]) -> Command {
        let mut paths = vec![home.join("bin")];
        if let Ok(p) = std::env::var("PATH") {
            paths.extend(std::env::split_paths(&p));
        }
        let mut c = Command::new("gopass");
        c.env("GOPASS_CONFIG", home.join("config.yml"));
        c.env("GOPASS_HOMEDIR", home);
        c.env("PINENTRY_PASSPHRASE", PIN);
        c.env("PATH", std::env::join_paths(paths).unwrap());
        c.args(args);
        c
    }

    /// Provision an isolated gopass age store whose recipients file lists only
    /// `recipient`, so every secret gopass inserts is decryptable by the holder
    /// of the matching identity (gpm). Returns the temp home (which pins the
    /// lifetimes of everything under it) and the store directory.
    fn provision_gopass_store(recipient: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let home = tempfile::tempdir().unwrap();
        install_mock_pinentry(home.path());

        // Bootstrap a throwaway gopass identity purely so `init` finds a usable
        // private key; its recipient is discarded by the recipients rewrite below.
        let keygen = gopass(
            home.path(),
            &["age", "identities", "keygen", "--password", PIN],
        )
        .output()
        .unwrap();
        assert!(
            keygen.status.success(),
            "gopass age keygen failed: {}",
            String::from_utf8_lossy(&keygen.stderr)
        );

        let store_dir = home.path().join("store");
        let init = gopass(
            home.path(),
            &[
                "--yes",
                "init",
                "--crypto",
                "age",
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
            "gopass init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        // Rewrite the recipients file to gpm's recipient (gopass's own format:
        // one recipient per line, trailing newline). gopass honors this on every
        // insert and does not normalize it away.
        std::fs::write(store_dir.join(".age-recipients"), format!("{recipient}\n")).unwrap();

        (home, store_dir)
    }

    /// `gopass insert -f <name>` reading `plaintext` from stdin — the same path a
    /// piped shell user takes. gopass stores the bytes verbatim.
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

    /// Guarantee the store's git HEAD matches its working tree. gpm clones HEAD,
    /// not the worktree, so the hand-written recipients file must be committed;
    /// gopass commits on insert, but this is a no-op when nothing is pending and
    /// a safety net when there is.
    fn commit_worktree(store: &Path) {
        let _ = Command::new("git")
            .arg("-C")
            .arg(store)
            .args(["add", "-A"])
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(store)
            .args([
                "-c",
                "user.name=gpm-interop",
                "-c",
                "user.email=interop@gpm",
                "commit",
                "-m",
                "gpm interop test",
            ])
            .status();
    }

    /// Split a secret plaintext into `(password, body)` the way gpm's parser
    /// will: first line is the password, the remainder is the body. gpm strips
    /// trailing whitespace from the body, so the expected body carries no
    /// trailing newline.
    fn expected_password_body(plaintext: &str) -> (&str, &str) {
        match plaintext.split_once('\n') {
            Some((pw, body)) => (pw, body.trim_end_matches('\n')),
            None => (plaintext, ""),
        }
    }

    /// **Forward interop (gopass → gpm):** a store created and populated by the
    /// real `gopass` binary is cloned and decrypted by gpm. Exercises the full
    /// read stack against gopass-produced output, across secret shapes that
    /// stress the body parser (password-only, multiline AKV, non-ASCII).
    #[tokio::test]
    async fn gpm_decrypts_secrets_written_by_real_gopass() {
        if !gopass_present() {
            eprintln!("skipping gopass interop test: `gopass` not on PATH");
            return;
        }

        let (identity, recipient) = generate_test_keypair();
        let (home, store_dir) = provision_gopass_store(&recipient);

        // Several secret shapes gopass writes and gpm must parse back identically.
        // The name-shape cases additionally stress gpm's path resolution and
        // `.age` extension stripping on names gopass produces — a dotted final
        // component must survive (`svc/api.key.age` lists as `svc/api.key`).
        let cases: &[(&str, &str)] = &[
            ("test/password-only", "s3cret"),
            (
                "test/multiline",
                "hunter2\nuser: alice\nurl: https://example.com",
            ),
            ("test/unicode", "pässwörd\nnote: 日本語 emoji 🔑"),
            // Name-shape matrix: deep nesting, dotted final component, non-ASCII name.
            ("team/infra/prod/db", "deep-pw\nenv: prod\nrole: admin"),
            ("svc/api.key", "dot-pw\nscope: read"),
            ("café/login", "uni-pw\nnote: name is non-ASCII"),
        ];
        for (name, plaintext) in cases {
            gopass_insert(home.path(), name, plaintext);
        }
        commit_worktree(&store_dir);

        // gpm clones the gopass store and decrypts with the identity whose
        // recipient gopass encrypted to.
        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .configure(
                store_dir.to_str().unwrap(),
                None,
                None,
                None,
                &identity,
                None,
            )
            .await
            .expect("gpm clones and configures the gopass store");

        // Structural compat: gpm lists exactly the entries gopass wrote.
        let entries: Vec<String> = store
            .list()
            .await
            .expect("gpm lists the cloned gopass store")
            .into_iter()
            .map(|e| e.name)
            .collect();
        for (name, _) in cases {
            assert!(
                entries.iter().any(|e| e == name),
                "gpm should list the gopass entry {name}; got {entries:?}"
            );
        }

        // Full-stack compat: gpm decrypts each entry and parses the body back to
        // exactly what gopass stored.
        for (name, plaintext) in cases {
            let secret = store
                .get(name)
                .await
                .expect("gpm decrypts the gopass-written entry");
            let (pw, body) = expected_password_body(plaintext);
            assert_eq!(secret.password(), pw, "password mismatch for {name}");
            assert_eq!(secret.body(), body, "body mismatch for {name}");
        }
    }
}
