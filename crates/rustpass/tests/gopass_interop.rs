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
    use super::common::{encrypt_to_recipients, expect_fast_forwarded, generate_test_keypair};
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use rustpass::{GitAuth, store::Store};
    use tempfile::TempDir;

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
        let mut perm = fs::metadata(&mock).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&mock, perm).unwrap();

        let gnupg = home.join("gnupg");
        fs::create_dir_all(&gnupg).unwrap();
        let mut gperm = fs::metadata(&gnupg).unwrap().permissions();
        gperm.set_mode(0o700);
        fs::set_permissions(&gnupg, gperm).unwrap();
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
    fn provision_gopass_store(recipient: &str) -> (TempDir, PathBuf) {
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
        fs::write(store_dir.join(".age-recipients"), format!("{recipient}\n")).unwrap();

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

    /// Like [`commit_worktree`] but asserts both git commands succeeded, for the
    /// load-bearing commits (the dual-recipient commit before a clone/push, the
    /// planted-ciphertext commit before a gopass read) where a silent no-op would
    /// violate a test invariant. Forces `commit.gpgsign=false` so a developer
    /// machine with signing on (plus the test's isolated/empty GNUPGHOME) can't
    /// fail the commit silently. Callers always have pending changes, so the
    /// "nothing to commit" no-op path does not arise here.
    fn commit_worktree_strict(store: &Path) {
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
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=gpm-interop",
                "-c",
                "user.email=interop@gpm",
                "commit",
                "-m",
                "gpm interop test",
            ])
            .output()
            .unwrap();
        assert!(
            commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
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
            // Mixed-case AKV keys (the divergent direction): gopass writes them
            // verbatim, gpm must parse key case byte-exact (`Secret::parse_attributes`,
            // secret.rs) — pinning that gpm does not lowercase what gopass wrote.
            (
                "fwd/casekeys",
                "Secret\nUserName: alice\nURL: https://example.com",
            ),
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
            .configure(store_dir.to_str().unwrap(), &GitAuth::None, &identity, None)
            .await
            .expect("gpm clones and configures the gopass store");

        // Structural compat: gpm lists exactly the entries gopass wrote.
        let entries: Vec<String> = store
            .list(0, usize::MAX)
            .await
            .expect("gpm lists the cloned gopass store")
            .entries
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

    /// **Reverse interop (gpm writes → gopass decrypts):** secrets written by
    /// gpm's REAL modern writer (`Store::set`) are decrypted and parsed by the
    /// real `gopass` binary. This is the mirror of the forward test above — it
    /// pins that gpm's modern-format output is byte/format-compatible with
    /// gopass's parser, closing the direction the forward test leaves open.
    ///
    /// Plant-style (no git transport): gpm writes into its clone of gopass's
    /// store, the ciphertext bytes are copied into gopass's working store, and
    /// `gopass show` reads them back. This isolates the FORMAT question (gpm
    /// writer ↔ gopass parser) from transport — the analogue of the legacy
    /// plant test, but planting bytes from gpm's real `Store::set` rather than
    /// from the test helper's encryptor. It is safe because gopass's
    /// `.gitattributes` filters only `*.gpg`, so a planted `.age` passes
    /// `git add`/`commit` untransformed (no `clean`/`smudge` filter on `.age`).
    #[tokio::test]
    async fn gopass_decrypts_modern_secrets_written_by_gpm() {
        if !gopass_present() {
            eprintln!("skipping gopass interop test: `gopass` not on PATH");
            return;
        }

        let (identity, recipient) = generate_test_keypair();
        let (home, store_dir, _gopass_recipient) =
            provision_gopass_store_with_gopass_recipient(&recipient);
        // The helper rewrote `.age-recipients` to dual-recipient but did not
        // commit it; gpm clones HEAD, so commit first or gpm's clone would carry
        // only gopass's single-recipient file (mirrors provision_shared_bare_interop).
        commit_worktree_strict(&store_dir);

        // gpm clones gopass's committed store directly (the forward test's
        // pattern). The dual-recipient file travels with the clone, so gpm's
        // `set` encrypts to BOTH recipients — gopass's identity can decrypt what
        // gpm writes.
        let config_dir = tempfile::tempdir().unwrap();
        let gpm_repo = config_dir.path().join("repo");
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .configure(store_dir.to_str().unwrap(), &GitAuth::None, &identity, None)
            .await
            .expect("gpm clones the gopass store");

        // Several secret shapes gpm writes and gopass must parse back identically
        // — the reverse twin of the forward test's matrix (password-only, multiline
        // AKV, non-ASCII, deep nesting, dotted final component, non-ASCII name).
        // The mixed-case case additionally pins cross-binary AKV key-case
        // agreement: gpm preserves keys byte-exact (`Secret::parse_attributes`,
        // secret.rs) and gopass renders them verbatim (its key lookup is
        // case-sensitive), so a `UserName` key must round-trip unchanged — any
        // normalization on either side surfaces here.
        let cases: &[(&str, &str)] = &[
            ("test/password-only", "s3cret"),
            (
                "test/multiline",
                "hunter2\nuser: alice\nurl: https://example.com",
            ),
            ("test/unicode", "pässwörd\nnote: 日本語 emoji 🔑"),
            ("team/infra/prod/db", "deep-pw\nenv: prod\nrole: admin"),
            ("svc/api.key", "dot-pw\nscope: read"),
            ("café/login", "uni-pw\nnote: name is non-ASCII"),
            // Mixed-case AKV keys: pins that gopass reads gpm's byte-exact keys.
            (
                "rev/casekeys",
                "Secret\nURL: https://example.com\nUserName: alice",
            ),
        ];
        for (name, plaintext) in cases {
            store
                .set(name, plaintext.as_bytes())
                .await
                .expect("gpm writes the modern-format secret");
        }

        // Plant each gpm-written ciphertext into gopass's working store — the
        // only way to get gpm-written bytes in front of gopass's reader without a
        // shared bare. gopass reads from its own worktree.
        for (name, _plaintext) in cases {
            let src = gpm_repo.join(format!("{name}.age"));
            let dst = store_dir.join(format!("{name}.age"));
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::copy(&src, &dst).unwrap();
        }
        commit_worktree_strict(&store_dir);

        for (name, plaintext) in cases {
            // gopass: real binary, full parse cascade.
            let gopass_out = gopass_show(home.path(), name);
            let (gopass_pw, gopass_body) = expected_password_body(&gopass_out);

            // The load-bearing reverse assertion: gopass's parser, fed gpm-written
            // bytes, yields the same password and body gpm wrote.
            let (want_pw, want_body) = expected_password_body(plaintext);
            assert_eq!(
                gopass_pw, want_pw,
                "{name}: gopass password from gpm-written bytes"
            );
            assert_eq!(
                gopass_body, want_body,
                "{name}: gopass body from gpm-written bytes"
            );
        }
    }

    /// A shared bare git remote plus an isolated gopass store wired to it as
    /// `origin`, used by the git-sync round-trip test. gopass bootstraps the
    /// bare, so it carries gopass's own initial commit plus a dual-recipient
    /// `.age-recipients` (gpm's + gopass's). The `TempDir` fields anchor lifetimes.
    struct SharedBareInterop {
        _home: TempDir,
        _bare_dir: TempDir,
        home_path: PathBuf,
        bare_path: PathBuf,
        store_dir: PathBuf,
        gopass_recipient: String,
    }

    /// Provision a [`SharedBareInterop`]: a dual-recipient gopass store (gopass's
    /// own recipient + `gpm_recipient`), an empty bare repo, gopass wired to the
    /// bare as `origin`, the dual-recipient file committed and pushed so gpm's
    /// clone of the bare carries the SAME recipients (no recipients-file
    /// divergence across sync), and the bare HEAD pointed at gopass's branch.
    ///
    /// The bare-HEAD re-point is load-bearing: gpm's clone follows the bare's
    /// HEAD symbolic ref, and gpm's push/pull refspecs derive from its
    /// checked-out branch (`commit.rs:199` / `pull.rs:152`). A fresh
    /// `init --bare` defaults HEAD to the git compile-time default branch, which
    /// may not match gopass's; a silent mismatch makes gpm's push/pull no-op.
    fn provision_shared_bare_interop(gpm_recipient: &str) -> SharedBareInterop {
        let (home, store_dir, gopass_recipient) =
            provision_gopass_store_with_gopass_recipient(gpm_recipient);
        let store_str = store_dir.to_str().unwrap();
        // Commit the dual-recipient rewrite so it travels with the bootstrap
        // push — otherwise gpm clones gopass's single-recipient HEAD and the two
        // sides' .age-recipients diverge on the first sync.
        commit_worktree_strict(&store_dir);

        let bare_dir = tempfile::tempdir().unwrap();
        let bare_path = bare_dir.path().to_path_buf();
        let bare_str = bare_path.to_str().unwrap();

        // Empty bare remote (local file transport — no auth).
        let init = Command::new("git")
            .args(["init", "--bare", bare_str])
            .output()
            .unwrap();
        assert!(
            init.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        // Wire gopass's store to the bare as origin.
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

        // Bootstrap push with -u so gopass's local branch tracks origin/<branch>
        // — needed for gopass's later no-arg `git push` / `git pull` / `sync`.
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

        // Re-point the bare HEAD at gopass's branch so gpm's clone lands on it.
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

        let home_path = home.path().to_path_buf();
        SharedBareInterop {
            _home: home,
            _bare_dir: bare_dir,
            home_path,
            bare_path,
            store_dir,
            gopass_recipient,
        }
    }

    /// **git-sync round-trip:** gpm and the real `gopass` binary push and pull to
    /// one shared bare git remote, each via its own sync path. This exercises the
    /// transport/wire layer — gopass's git conventions (commit messages,
    /// `.gitattributes`) flowing into gpm's git sync, and gpm's commits flowing
    /// back into gopass — the drift surface the format-only tests do not cover.
    ///
    /// Every step is a clean fast-forward: between any pull and the next push the
    /// bare advances exactly once, so divergence is unreachable on this path.
    ///
    /// ```text
    /// bare HEAD ─ gopass bootstrap push ─► symbolic-ref ─► gpm clones (gopass's branch)
    /// Dir 1: gopass insert+push ─► gpm sync() pull ─► list + decrypt   (bare moves once)
    /// Dir 2: gpm set + push()   ─► gopass git pull  ─► show            (bare moves once)
    /// sync : gpm set + push()   ─► gopass sync      ─► show            (bare moves once)
    /// ```
    ///
    /// After EVERY gopass operation we assert `.age-recipients` still lists both
    /// recipients — recipient-file drift is the exact silent failure live-binary
    /// interop exists to catch. Default signature verification is `VerifyMode::Off`,
    /// so gopass's unsigned commits fast-forward with no special handling; the
    /// commit graph is deliberately heterogeneous (gpm-interop test commits plus
    /// gopass's own identity), which is fine under Off.
    #[tokio::test]
    async fn gpm_and_gopass_sync_through_shared_bare_remote() {
        if !gopass_present() {
            eprintln!("skipping gopass interop test: `gopass` not on PATH");
            return;
        }

        let (identity, gpm_recipient) = generate_test_keypair();
        let env = provision_shared_bare_interop(&gpm_recipient);

        macro_rules! recipients_intact {
            () => {{
                let recips = fs::read_to_string(env.store_dir.join(".age-recipients")).unwrap();
                assert!(
                    recips.contains(gpm_recipient.as_str()),
                    "gpm recipient dropped from .age-recipients:\n{recips}"
                );
                assert!(
                    recips.contains(env.gopass_recipient.as_str()),
                    "gopass recipient dropped from .age-recipients:\n{recips}"
                );
            }};
        }
        recipients_intact!();

        // gpm clones the shared bare — lands on bare HEAD (gopass's branch).
        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .configure(
                env.bare_path.to_str().unwrap(),
                &GitAuth::None,
                &identity,
                None,
            )
            .await
            .expect("gpm clones the shared bare");

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
        recipients_intact!();

        // gpm pulls — fast-forward (gpm's HEAD == bare pre-push tip; bare moved once).
        let outcome = store.sync().await.expect("gpm pulls");
        expect_fast_forwarded(outcome);

        // List agreement + cross-decrypt.
        let entries: Vec<String> = store
            .list(0, usize::MAX)
            .await
            .expect("gpm lists")
            .entries
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(
            entries.iter().any(|e| e == gopass_entry),
            "gpm should list the gopass-pushed entry; got {entries:?}"
        );
        let secret = store
            .get(gopass_entry)
            .await
            .expect("gpm decrypts the gopass-pushed entry");
        let (want_pw, want_body) = expected_password_body(gopass_plaintext);
        assert_eq!(secret.password(), want_pw);
        assert_eq!(secret.body(), want_body);

        // ── Direction 2: gpm writes + pushes → gopass pulls ───────────────────
        // Invariant: gpm's HEAD == bare tip (just pulled) and nothing pushed in
        // between, so gpm's commit + push is a clean fast-forward.
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
        recipients_intact!();

        // Cross-decrypt via the real gopass binary — load-bearing: it proves
        // gopass both resolves and decrypts gpm's pushed entry.
        let gopass_out = gopass_show(env.home_path.as_path(), gpm_entry);
        let (got_pw, got_body) = expected_password_body(&gopass_out);
        let (want_pw, want_body) = expected_password_body(gpm_plaintext);
        assert_eq!(got_pw, want_pw, "gopass must decrypt gpm-pushed password");
        assert_eq!(got_body, want_body, "gopass must render gpm-pushed body");

        // ── gopass sync cycle (the user-facing sync path) ─────────────────────
        // Safe per the pre-implementation probe: age-backend `gopass sync` over a
        // local-file remote is a near-no-op over git pull/push (no recipient
        // prune, no keyserver hop), so this exercises gopass's high-level
        // reconcile path without flakiness.
        let sync_entry = "sync-cycle/secret";
        let sync_plaintext = "via-sync\nkind: gopass sync round-trip";
        store
            .set(sync_entry, sync_plaintext.as_bytes())
            .await
            .expect("gpm writes for sync cycle");
        store.push().await.expect("gpm pushes for sync cycle");
        let sync = gopass(env.home_path.as_path(), &["sync"]).output().unwrap();
        assert!(
            sync.status.success(),
            "gopass sync failed: {}",
            String::from_utf8_lossy(&sync.stderr)
        );
        recipients_intact!();
        let gopass_out = gopass_show(env.home_path.as_path(), sync_entry);
        let (got_pw, got_body) = expected_password_body(&gopass_out);
        let (want_pw, want_body) = expected_password_body(sync_plaintext);
        assert_eq!(got_pw, want_pw, "gopass sync: decrypt password");
        assert_eq!(got_body, want_body, "gopass sync: render body");
    }

    /// Like [`provision_gopass_store`], but captures gopass's own age recipient
    /// (so a planted file can be encrypted to it for gopass to decrypt) and
    /// lists BOTH gpm's and gopass's recipients in `.age-recipients`. Returns
    /// the store dir plus gopass's recipient string.
    fn provision_gopass_store_with_gopass_recipient(
        gpm_recipient: &str,
    ) -> (TempDir, PathBuf, String) {
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
        let gopass_recipient = fs::read_to_string(store_dir.join(".age-recipients"))
            .unwrap()
            .lines()
            .find(|l| !l.trim().is_empty())
            .expect("gopass init wrote a recipient")
            .trim()
            .to_owned();

        // List both so either side can decrypt planted files.
        fs::write(
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
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&file, encrypt_to_recipients(plaintext, recipients)).unwrap();
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
            .configure(store_dir.to_str().unwrap(), &GitAuth::None, &identity, None)
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
    /// produced — a gap the earlier text-only interop tests left to this
    /// attachment coverage. Every byte
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
            fs::write(&path, bytes).unwrap();
            gopass_fscopy(home.path(), &path, &format!("att/{i}"));
        }
        commit_worktree(&store_dir);

        // gpm clones the gopass store and decrypts with the identity whose
        // recipient gopass encrypted to.
        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .configure(store_dir.to_str().unwrap(), &GitAuth::None, &identity, None)
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
            let attachment = rustpass::attachment::extract(&secret)
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
