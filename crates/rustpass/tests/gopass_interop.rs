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
    use super::common::{encrypt_to_recipients, generate_test_keypair};
    use std::fs;
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

    /// Write the mock pinentry into `home/bin` and mark it executable. Also
    /// creates an empty `home/gnupg` (mode 0700) so an isolated `GNUPGHOME`
    /// keeps gopass's pinentry helper off the user's running gpg-agent —
    /// otherwise gopass routes the age-identity passphrase through the agent's
    /// TTY-needing pinentry and bypasses our mock.
    fn install_mock_pinentry(home: &Path) {
        let bin = home.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let mock = bin.join("pinentry");
        fs::write(&mock, MOCK_PINENTRY).unwrap();
        let mut perm = std::fs::metadata(&mock).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&mock, perm).unwrap();

        let gnupg = home.join("gnupg");
        std::fs::create_dir_all(&gnupg).unwrap();
        let mut gperm = std::fs::metadata(&gnupg).unwrap().permissions();
        gperm.set_mode(0o700);
        std::fs::set_permissions(&gnupg, gperm).unwrap();
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
        c.env("GNUPGHOME", home.join("gnupg"));
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

    /// Like [`provision_gopass_store`], but captures gopass's own age recipient
    /// (so a planted file can be encrypted to it for gopass to decrypt) and
    /// lists BOTH gpm's and gopass's recipients in `.age-recipients`. Returns
    /// the store dir plus gopass's recipient string.
    fn provision_gopass_store_with_gopass_recipient(
        gpm_recipient: &str,
    ) -> (tempfile::TempDir, std::path::PathBuf, String) {
        let home = tempfile::tempdir().unwrap();
        install_mock_pinentry(home.path());

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

        // gopass wrote its own recipient during init; capture it before any
        // rewrite. The holder of the matching identity (gopass) can decrypt a
        // file encrypted to it, independent of the store's recipients list.
        let gopass_recipient = std::fs::read_to_string(store_dir.join(".age-recipients"))
            .unwrap()
            .lines()
            .find(|l| !l.trim().is_empty())
            .expect("gopass init wrote a recipient")
            .trim()
            .to_owned();

        // List both so either side can decrypt planted files.
        std::fs::write(
            store_dir.join(".age-recipients"),
            format!("{gpm_recipient}\n{gopass_recipient}\n"),
        )
        .unwrap();

        (home, store_dir, gopass_recipient)
    }

    /// Plant `plaintext` (a legacy `GOPASS-SECRET-1.0` blob) at
    /// `<store>/<name>.age`, encrypted to every recipient in `recipients` so
    /// both gopass (its own identity) and gpm (its identity) can decrypt it.
    /// Bypasses gopass's writer, which only emits the modern format — this is
    /// the only way to get gopass to *read* a legacy entry it never writes.
    fn plant_legacy_entry(store: &Path, name: &str, plaintext: &[u8], recipients: &[&str]) {
        let file = store.join(format!("{name}.age"));
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&file, encrypt_to_recipients(plaintext, recipients)).unwrap();
    }

    /// `gopass show <name>`: decrypt through gopass's real parse cascade
    /// (legacy MIME first), returning the full stdout — password on line 1,
    /// then the body gopass renders. Asserts gopass succeeded.
    fn gopass_show(home: &Path, name: &str) -> String {
        let out = gopass(home, &["show", name]).output().unwrap();
        assert!(
            out.status.success(),
            "gopass show {name:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// **Reverse interop (legacy format → gopass + gpm agree):** a deprecated
    /// `GOPASS-SECRET-1.0` plaintext is planted — encrypted to both sides, so
    /// gopass's own identity and gpm's identity can each decrypt it — then read
    /// by the real `gopass` binary AND by gpm. Both must lift the same password
    /// out of the `Password:` header and render the same body. Before R054, gpm
    /// yielded the literal `GOPASS-SECRET-1.0` magic as the password while
    /// gopass correctly read the header; this test pins that gpm now agrees.
    #[tokio::test]
    async fn gpm_and_gopass_agree_on_legacy_gopass_secret_1_0() {
        if !gopass_present() {
            eprintln!("skipping gopass interop test: `gopass` not on PATH");
            return;
        }

        let (identity, recipient) = generate_test_keypair();
        let (home, store_dir, gopass_recipient) =
            provision_gopass_store_with_gopass_recipient(&recipient);

        // One non-Password attribute per case so gopass's attribute sort and
        // gpm's source-order render can't diverge — the body compare is exact.
        let cases: &[(&str, &str)] = &[
            (
                "legacy/basic",
                "GOPASS-SECRET-1.0\nPassword: hunter2\nUsername: alice\n\nfree text body",
            ),
            (
                "legacy/password-only",
                "GOPASS-SECRET-1.0\nPassword: hunter2",
            ),
            (
                "legacy/no-body",
                "GOPASS-SECRET-1.0\nPassword: hunter2\nUsername: alice",
            ),
        ];
        let recipients: [&str; 2] = [&recipient, &gopass_recipient];
        for (name, plaintext) in cases {
            plant_legacy_entry(&store_dir, name, plaintext.as_bytes(), &recipients);
        }
        commit_worktree(&store_dir);

        // gpm clones the store and decrypts with its own identity.
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

        for (name, _plaintext) in cases {
            // gopass: real binary, full parse cascade (legacy MIME first).
            let gopass_out = gopass_show(home.path(), name);
            let (gopass_pw, gopass_body) = expected_password_body(&gopass_out);

            // gpm: our parser on the same plaintext bytes.
            let secret = store
                .get(name)
                .await
                .expect("gpm decrypts the planted legacy entry");

            // The load-bearing assertion: gpm's password matches gopass's. The
            // R054 bug was gpm returning the magic string here.
            assert_eq!(
                secret.password(),
                gopass_pw,
                "{name}: gpm password must match gopass's parse of the same legacy bytes"
            );
            assert_eq!(
                secret.body(),
                gopass_body,
                "{name}: gpm body must match gopass's render"
            );
            // Guard against regressing back to the magic-as-password bug.
            assert_ne!(
                secret.password(),
                "GOPASS-SECRET-1.0",
                "{name}: gpm regressed to treating the magic as the password"
            );
        }
    }

    /// `gopass fscopy <from> <to>`: upload a real file into the store as a
    /// base64 attachment — gopass's binary write path, which runs the bytes
    /// through `secFromBytes` (Content-Disposition + Content-Transfer-Encoding:
    /// Base64 + base64 body). The write encrypts only to the recipients file, so
    /// it needs no identity and never reaches the mock pinentry. The source
    /// file's basename becomes the attachment's Content-Disposition filename.
    fn gopass_fscopy(home: &Path, from: &Path, to: &str) {
        let out = gopass(home, &["fscopy", from.to_str().unwrap(), to])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "gopass fscopy {from:?} → {to:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// **Forward attachment interop (gopass writes a binary attachment → gpm
    /// decodes it byte-identically):** a file uploaded through gopass's real
    /// binary write path (`gopass fscopy` → `secFromBytes`) is read back by
    /// gpm's attachment decoder. Pins that gpm interprets exactly what gopass
    /// produced — the gap R053 deferred to the attachment work. Every byte
    /// value, a PNG signature, and a multi-KB payload stress the base64 decode.
    #[tokio::test]
    async fn gpm_decodes_attachment_written_by_real_gopass() {
        if !gopass_present() {
            eprintln!("skipping gopass interop test: `gopass` not on PATH");
            return;
        }

        let (identity, recipient) = generate_test_keypair();
        let (home, store_dir) = provision_gopass_store(&recipient);

        // gopass fscopy uploads a file as a base64 attachment. The source file's
        // basename becomes the Content-Disposition filename (secFromBytes), so
        // each case also pins filename recovery, not just the bytes.
        let all_bytes: Vec<u8> = (0u8..=255).collect();
        let png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let big: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let cases: &[(&str, &[u8])] = &[
            ("all-bytes.bin", &all_bytes),
            ("photo.png", &png),
            ("4kb-blob.dat", &big),
        ];
        let work = tempfile::tempdir().unwrap();
        for (i, &(fname, bytes)) in cases.iter().enumerate() {
            let path = work.path().join(fname);
            std::fs::write(&path, bytes).unwrap();
            gopass_fscopy(home.path(), &path, &format!("att/{i}"));
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

        // The load-bearing interop assertion: gpm's attachment decoder yields
        // exactly the bytes gopass uploaded, and recovers the original filename.
        for (i, &(fname, bytes)) in cases.iter().enumerate() {
            let entry = format!("att/{i}");
            let secret = store
                .get(&entry)
                .await
                .expect("gpm decrypts the gopass-written attachment entry");
            let attachment = rustpass::attachment::extract(secret.body())
                .expect("attachment body decodes")
                .expect("the entry is recognized as an attachment");
            assert_eq!(
                attachment.bytes(),
                bytes,
                "{entry}: gpm-decoded bytes must match the file gopass uploaded"
            );
            assert_eq!(
                attachment.filename(),
                Some(fname),
                "{entry}: filename must be the uploaded file's basename"
            );
        }
    }
}
