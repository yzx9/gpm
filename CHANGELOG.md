# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [v0.10.0] - 2026-07-06

### Added

- Opening a page now slides in from the right, and going back slides it the other way — a stack-style transition between pages. Transitions to or from a page that shows secrets swap instantly, so the screen-capture guard never leaves a secret visible during the animation.
- After you copy a password on Android, a small notification appears in your notification shade while the secret is on the clipboard — tap it to clear the clipboard immediately, without gpm taking over the foreground. It dismisses itself when the clipboard is cleared, whether by your tap or the automatic timer. The first time you copy, gpm asks once for permission to show notifications; if you decline, copying still works, just without the notification. Android only; desktop is unchanged.
- GPG/OpenPGP-signed commits are now verified, the same way SSH-signed commits already are — instead of being flagged as an unsupported format. Under Settings → Trusted signing keys you can paste a GPG public key (or import a `.asc` file from your device) to trust a signer; once trusted, a commit signed by that key shows as verified in Audit/Enforce mode and in the history view. A commit signed by a GPG key you haven't trusted shows a distinct "Unverified signature" status with a hint to add the signer's key, since GPG signatures don't carry the public key the way SSH signatures do.

### Changed

- The unlock screen now leads with your biometric option (fingerprint or face) when biometric unlock is enabled, and tucks the passphrase entry behind an "Unlock with passphrase" switch until you actually need it — so the screen shows one clear primary action instead of two competing buttons. The two actions are also the same width now. Cancelling the biometric prompt keeps you on that screen; tap the switch to type your passphrase instead.
- The History screen now shows commit ages more clearly. Recent commits still read "2h ago" or "3d ago", but anything older than a week shows an actual date — "Mar 15", or "Mar 15, 2024" for a prior year — instead of a hard-to-parse value like "249h ago".
- History's commit rows are easier to read. Each commit's message now gets its own line, with the hash, author, and time grouped on a quieter line underneath, instead of all three squeezed into one cramped row that truncated the message.
- The Back button now returns to the page you actually came from, instead of always jumping to a fixed page. Pressing Back from Settings takes you to wherever you opened Settings (usually the entry list), and the Android system back button now agrees with the in-app Back button — repeated Back no longer cycles through pages you already visited, and Back from the entry list exits the app.
- Generated passwords now use your configured clipboard auto-clear timeout (the same one stored-secret copies use) instead of a fixed 30 seconds, so the two copy paths agree on how long a secret lingers.

### Fixed

- Text and controls are easier to read in both light and dark mode — secondary labels, links, the primary button, and the dark-theme status colors (red/green/blue) now meet WCAG AA contrast. Several grays and the brand blue were previously below the 4.5:1 threshold, especially in dark mode where the status colors had never been re-tuned.
- When App Lock is on, the entry list no longer stays stuck on a "locked" message after you unlock with your fingerprint. It loads your entries on its own the moment the store unlocks, instead of making you leave the screen and come back. (The message while locked is intentional — it reminds you the content needs an unlock, and no entry data is loaded until then.)
- Under App Lock, the "locked" message on the entry list no longer falsely warns that you need to set the app up again — you don't, just unlock. It now simply tells you to unlock.
- Enforce mode can now be turned on whenever at least one trusted signing key is set — including when only a GPG key is trusted. Previously Enforce required an SSH key, so trusting only a GPG key left Enforce unavailable.
- On Android, pressing the system Back button now closes an open sheet or dialog — a commit's detail, the reset confirmation, a signature-check notice, or a sync-conflict resolve — instead of navigating away and leaving it stranded. Back on the app-lock screen is now held in place rather than backing out, so the lock can't be slipped past.

## [v0.9.0] - 2026-07-05

### Added

- Setting or changing a passphrase now asks you to type it twice, and gpm checks the two entries match before continuing, so a single typo can't silently become the passphrase that locks your identity (gpm cannot recover a lost passphrase). Each box has a show/hide toggle so you can also verify what you typed. This applies everywhere you set a new passphrase: initial setup, SSH key generation, and Settings.
- When you set or change your identity passphrase, you must now tick a box confirming you understand gpm cannot recover it — losing it permanently locks you out of your secrets. The same warning the unlock screen shows now appears at the moment you actually choose the passphrase, so the consequence is clear before you commit. This applies wherever you set a new at-rest passphrase: initial setup and Settings (set and change).

### Changed

- The entry list no longer shows a copy button on each row. Tapping anywhere on a row now opens that entry's detail page, where you can copy the password — a small arrow on each row marks it as tappable. Copying from the list was rarely useful, and this makes the whole row the tap target instead of a small button.
- Syncing is now pull-to-refresh — drag the entry list down from the top and release to sync with your remote. The toolbar Sync button is gone; while a sync runs, a Cancel button appears in the progress row so you can still stop it.
- The Home toolbar now holds just two buttons — new secret and settings. The signature-status indicator moved next to the gpm logo as a small colored light (tap it any time to open History), since it was always an indicator dressed as a button that didn't match the others' size. "Generate password" moved into the new-secret picker: tap ＋, then pick Generate password alongside the create-secret types.
- Setting or changing your passphrase, and entering it to enable biometric unlock or identity auto-unlock, now happens in a focused prompt instead of an inline form. A successful submit makes it obvious the change was saved; closing or backing out of the prompt discards what you typed.
- Editing your commit identity or pasting a new trusted signing key in Settings now marks that card with an "Unsaved changes" highlight, and leaving Settings with uncommitted edits asks whether to save, discard, or keep editing — so a stray back-tap no longer silently throws away what you typed.
- Tapping Cancel while cloning a repository now shows a disabled "Cancelling…" state on the button right away, so it's clear the cancel was received instead of looking like the tap did nothing. If the cancel request itself fails, you now see a message instead of a silent failure. A clone that's still connecting — handshaking, authenticating — may take a moment to actually stop.
- The History screen now shows commit ages more clearly. Recent commits still read "2h ago" or "3d ago", but anything older than a week shows an actual date — "Mar 15", or "Mar 15, 2024" for a prior year — instead of a hard-to-parse value like "249h ago".
- History's commit rows are easier to read. Each commit's message now gets its own line, with the hash, author, and time grouped on a quieter line underneath, instead of all three squeezed into one cramped row that truncated the message.

### Fixed

- On Android, tapping a button, list row, or other tappable element no longer leaves its highlight stuck on screen after you lift your finger. The highlight now appears only while your finger is pressed and disappears the moment you lift it, and every tappable element gives the same consistent press feedback.
- Tapping the "Showing" button while a password is already revealed now hides it, instead of asking you to unlock again and decrypting it a second time. The reveal button now works as a toggle: tap to show, tap again to hide.
- Tapping Edit on an entry now prompts for your passphrase when the identity is locked, instead of showing an "Identity is encrypted — unlock with passphrase first" error. You no longer have to reveal or copy a secret first just to edit it.
- Cloning a gopass repository now discovers recipients from the `.age-recipients` file only, matching gopass exactly. gpm no longer also reads a `.gopass-recipients` file that gopass itself never writes or uses, so the two stay in sync on a shared store.
- Copying a password twice in a row no longer clears the second copy early. Previously the first copy's clear timer could fire and wipe the second copy short of its full timeout window.

## [v0.8.1] - 2026-07-03

### Added

- Debug Android builds now install as a separate app — "gpm Debug", application id `xyz.yzx9.gpm.debug` — so a debug build sits alongside the signed release instead of overwriting it. Install a debug build to diagnose an issue without uninstalling your release gpm; the two keep separate data and keys.

### Changed

- Resetting all data from Settings now asks you to type "RESET" to confirm, so a single accidental tap can no longer trigger the wipe. No passphrase is required, so you can still reset if you've forgotten yours.
- The unlock dialog has a new ? button next to its title that explains what your passphrase is and warns that gpm cannot recover or reset it — lose it and your secrets are gone for good. Tap it again to dismiss the explanation.
- The unlock dialog now shows your current auto-lock policy — "cleared after every action" (Immediate), "auto-locks after N min of inactivity", or "stays unlocked until you lock manually" (Never) — so it's clear how long the identity stays cached after you unlock.
- Removed the "Reset all data" button from the unlock and app-lock dialogs — too dangerous for a screen you reach often. Reset now lives only in Settings → Danger Zone. If all your fingerprints are removed and gpm can no longer unlock its store, the app-lock screen tells you to clear gpm's app data from Android Settings (or uninstall and reinstall) to set it up again.
- The unlock dialog can now be dismissed with the × button, a backdrop tap, or the Android back button — even when the app is hard-locked. Dismissing hides the dialog without unlocking: your identity stays locked and secrets stay wiped, but you can read the entry list and open Settings without typing your passphrase first. The next action that needs the identity prompts you again.
- gpm's buttons, status indicators, and empty states now use clean vector icons instead of emoji, so they render consistently and sharply on every device instead of varying with the platform's emoji set (Android vs desktop).
- Transient messages — "✓ Copied", save/delete results, copy failures — now appear in one consistent style at the top of the screen. Several can stack up instead of a new one replacing the last, and errors are now visually distinct from successes.

## [v0.8.0] - 2026-07-03

### Added

- Control screen-capture protection from Settings — a master toggle (on by default) blocks screenshots and screen recording on pages that show secrets: setup, create, generate, entry detail, and settings (including the SSH key export). Turn it off to allow screenshots anywhere. Android only; elsewhere the toggle has no screen effect
- Cloning a repository and pulling updates now show live progress — how many objects and bytes have transferred — instead of a generic spinner, and either can be cancelled mid-flight with an on-screen Cancel button
- gpm now recognizes age plugin recipients (such as `age1yubikey1...` from age-plugin-yubikey) and can encrypt secrets to them, so a shared store that includes a teammate's hardware-key recipient keeps working. The matching `age-plugin-<name>` tool must be installed for encryption, which runs on desktop only (Android can't launch it). Decrypting with a plugin identity is not supported yet.
- When a save collides with a newer remote version, gpm now opens a resolve prompt showing exactly what differs and lets you keep your change or adopt the remote — instead of failing with a generic sync error. The same prompt appears on the Sync button
- New AutoSync setting (on by default): turn it off to keep saves local — no automatic push — and publish later with the Sync button
- The Sync button now does both pull and push (not just pull), so an AutoSync-off workflow can publish on demand, and a divergence at either phase opens the same resolve prompt

### Changed

- Signature status colors are now consistent everywhere they appear: the glyphs in the pull-review modals (shown when a pull brings signature issues) are now colored green/amber/red to match the history page, instead of plain.
- Screen-capture protection is now per-page instead of app-wide. Previously every screen blocked screenshots; now only pages that show secrets do (when the toggle is on), so you can screenshot the entry list and history. The entry list shows secret names and history shows commit signatures — neither reveals secret content
- The resolve prompt only catches the rare case where a save and a remote change directly collide (a push rejection). Editing from an out-of-date view can still overwrite a newer remote change without a prompt — those are recoverable in git history. A note to this effect appears under the AutoSync setting

### Fixed

- A shared store containing an age plugin recipient (e.g. a teammate's `age1yubikey1...` hardware key) no longer breaks adding or editing secrets — such recipients were previously misread and aborted every write.
- On Android, the back gesture now closes the unlock prompt and the "remote copy exists" dialog instead of navigating away from them. A locked screen can no longer be stepped past with back (use the Home gesture or button to leave); cancelling a per-operation unlock prompt no longer flashes an error
- On Android, resolving a "remote copy exists" conflict — cancel or keep the existing copy — no longer asks you to unlock first
- On Android, content no longer slides under the status bar or a display cutout (notch) — the safe-area insets on all four edges (status bar / notch at the top, navigation bar at the bottom, and side cutouts in landscape) are read directly from the system and re-applied on rotation, so they stay correct from launch instead of getting stuck at zero.

## [v0.7.3] - 2026-06-28

### Fixed

- Restored the Android build, which failed to compile in v0.7.2. Desktop was unaffected.

## [v0.7.2] - 2026-06-28

### Fixed

- On Android, HTTPS clone/sync/push over public-WebPKI servers (e.g. GitHub) now verifies correctly — the bundled Mozilla roots are loaded into the git TLS trust store on first use. (Servers behind a private/enterprise CA are not covered; use an SSH remote for those.)

## [v0.7.1] - 2026-06-27

### Fixed

- On Android, cloning or syncing a repository over HTTPS no longer fails with a certificate verification error — gpm now bundles the trusted root certificates (Mozilla's set) so the git connection can verify servers like GitHub. Desktop is unchanged

## [v0.7.0] - 2026-06-26

### Added

- Lock gpm behind your fingerprint or face with the new **App Lock** (Settings → App Lock, Android 11+). When on, gpm demands biometrics every time you open it or switch back from another app — nothing is visible until you authenticate, and the whole store is sealed behind your biometric, not just masked by a screen. Adding a new fingerprint won't lock you out; removing _all_ enrolled biometrics will, and the only recovery is to set gpm up again
- Optionally have gpm **unlock your passwords at the same time as the app**. A separate, off-by-default toggle in the App Lock section (Identity Auto-Unlock) makes the one app-unlock prompt also unlock your passwords, so you don't get asked again on the next copy or view. It's independent of the "stay unlocked for a while" timing presets
- Start a brand-new password store right on this device — no existing repo and no second tool required. Setup now offers "Create a new store" alongside "Clone": generate an age or SSH identity in-app, seed the store, and optionally add a git remote to sync later. A store gpm creates is indistinguishable from one gopass creates, so you can mix tools or migrate freely
- Delete a secret right from its detail page — gpm removes it, commits, and syncs the change like any other edit. If the remote has moved, the delete is safely rolled back and you're asked to sync first. gpm has no in-app undo, so a deleted entry is gone from the app and recoverable only via git history with external tooling
- Edit a secret's password and notes in place from its detail page — gpm saves, commits, and syncs the change like any other edit, without re-applying a creation template. If another device changed the same entry and your save can't fast-forward, gpm asks how to resolve it (keep yours, keep theirs, or cancel) instead of failing. Caveat: if another device's newer edit to the same entry is fast-forwarded over by your save, that newer change is overwritten on the tip — recoverable via git history — until a follow-up makes edit base-version-aware
- Control how and when gpm locks, all from the new "Auto-Lock & Auto-Clear" section in Settings. Pick when the app locks (immediately after each action, after a few minutes idle, or never), how long a shown password stays on screen, and how long the clipboard holds a copy — each with sensible presets and a "Never" option
- Generate a strong password right in the New secret form — tap the 🎲 button next to a password field and gpm fills it in for you. Website passwords can be random, memorable, or a multi-word passphrase; a PIN field generates a numeric code. Generated values are masked by default (tap 👁 to reveal) and are cleared on lock or when you leave the page, just like anything else you type there
- Open a dedicated password generator from the entry list (🎲) to produce a whole batch of strong passwords at once — pick a style (random, memorable, or a multi-word passphrase), a length, and how many to show, then tap any one to copy it. The clipboard clears itself 30 seconds later, just like copying a saved secret, and the list clears the moment you leave the screen or the app locks

### Changed

- gpm now defaults to re-checking your fingerprint or passphrase each time you copy, view, or create a secret, rather than staying unlocked for minutes at a time. This keeps your decryption key in memory only for the instant it's needed. Browsing the list is unaffected (it never needs unlocking). If you prefer the old "stay unlocked for a while" behavior, switch Auto-Lock to an idle timeout in Settings
- A shown password now auto-clears after 45 seconds by default (was 30), and a copied password clears from the clipboard after 45 seconds by default (was 30) — both are now adjustable in Settings

### Fixed

- On Android, if you enrolled a new fingerprint or face after enabling biometric unlock, gpm no longer pops a biometric prompt that can only fail on every launch — it goes straight to the passphrase form so you can re-enable biometric
- Pulling no longer shows a false divergence warning when your device has an unpushed change but the remote hasn't moved — it's a no-op pull, and your change syncs on the next push

## [v0.6.0] - 2026-06-20

### Added

- On Android, gpm now encrypts your local configuration and identity at rest with a key sealed in the device's hardware-backed Keystore, so someone who copies the app's private files (a stolen backup, a forensic dump) gets ciphertext rather than your git credentials or decryption key. Existing data is wrapped automatically on the next launch, and tampering with these files is detected and rejected. Desktop is unchanged
- The backend can now write a new secret the way gopass does: it syncs first, encrypts the content to every store recipient (always including your own key, so you can read back what you wrote), saves it at the chosen path, and commits and pushes the change. This is the foundation for in-app secret creation
- When a write collides with a newer remote copy of the same secret (e.g. you wrote offline and the remote moved), the backend detects the conflict instead of failing blindly. It reports whether the remote copy is one you can decrypt, and lets the caller resolve it: keep your version, keep the remote's, back out, or (with explicit confirmation) force your version over one you can't read. The conflict result never contains any plaintext, so the choice stays safe to pass to the UI
- The backend understands gopass content templates and creation presets. A `.pass-template` placed in a store directory now shapes any new secret created beneath it (filling in the password and layout), and the built-in "Website login" and "PIN Code" presets generate a secret at a fixed location (under `websites/` or `pin/`) from a few fields — the same "create" flow gopass offers
- Create new secrets right from the app: pick a Website login, PIN code, or a custom name, and gpm encrypts and pushes it just like gopass. A `.pass-template` in a folder automatically shapes any new secret created beneath it, and you can preview the result before saving
- If a new secret collides with a newer remote copy, the app asks how to resolve it instead of failing — keep yours, keep the existing one, or cancel. When the existing copy is one you can read, you can preview it first; overwriting one you can't read is blocked behind an explicit confirmation so you can't unknowingly destroy it
- When a pull finds the local and remote password stores have diverged, the app lists the local-only and modified secrets (and other files) that would be lost and offers to adopt the remote, discarding those local changes behind an explicit confirmation — instead of failing with an unresolvable error
- You can now choose the name and email gpm writes on each git commit — set it under Advanced Settings during setup, or change it later in Settings. Leave it blank to keep the built-in default, which follows app updates until you pick your own

### Changed

- The entry list and search now load one page at a time instead of pulling every entry into the app at once — as you scroll, more entries load automatically, with a "Load more" button as a fallback. This keeps the list fast and light on memory as your store grows, and search results page the same way
- Searching entries is now fuzzy: type a few letters in order (like `awroot`) to jump to `cloud/aws/root`, matching anywhere in the name or path. Search also runs in the backend now, so it stays fast as the store grows and keeps working when the list later loads on demand
- When gpm auto-locks after 5 minutes (or on launch of a passphrase-protected identity), the unlock prompt now appears as an overlay over whatever screen you were on, and unlocking drops you back exactly where you were — your scroll position and current entry are preserved. The biometric auto-prompt, cancel, and reset handling moved into the overlay unchanged
- Unlocking with an SSH key is faster: the key is decrypted once when you unlock, so opening each secret afterwards is quicker instead of paying that cost on every copy or show. The unlock passphrase is also no longer held in memory for the whole session — it's used to decrypt your identity and then dropped

### Fixed

- The instant the identity locks, every currently-revealed secret across the app is cleared — a shown password, an exported SSH key, a half-typed new secret. Previously the old unlock redirect gave this for free by unmounting the page; the new overlay keeps pages mounted, so clear-on-lock is now explicit
- A stale auto-lock timer could re-lock the app moments after a fresh unlock; the timer now carries a monotonic generation tag and disarms itself if a newer unlock happened while it slept

### Security

- Enabling biometric unlock is now refused in the backend for identities that have no passphrase, instead of relying on the settings screen to hide the option — a defense-in-depth backstop in case that UI gate ever regresses

## [v0.5.0] - 2026-06-15

### Added

- Upload an identity file instead of pasting it during setup. The file is opened, read, and parsed entirely on-device by the backend; its contents never reach the app UI. Encrypted files (a passphrase-protected SSH key, or an age-encrypted identity) prompt for the passphrase immediately and discard the file on a wrong one; once usable, the derived public key is shown so you can confirm it matches a recipient. Files produced by `age-keygen` (with `#` comment lines) are also supported
- Optional repository authenticity verification: detect a compromised git remote feeding validly encrypted but wrong entries by verifying the SSH signature on every commit pulled. A new tri-state setting (Off / Audit / Enforce) controls behaviour — Audit warns on a mismatch but always pulls, Enforce blocks the pull when a commit is unsigned, untrusted, or tampered, leaving your store on the last verified state. Manage trusted signing keys in Settings and review per-commit signature status in the new History screen. Off by default; nothing changes until you enable it

### Fixed

- Use plain val for Charset constant in KeystorePlugin

## [v0.4.0] - 2026-06-14

### Added

- Biometric unlock (fingerprint or face) for passphrase-protected identities on Android 11 and above — unlock gpm with biometrics instead of typing your passphrase on every launch. The passphrase is sealed in the Android Keystore with hardware-backed, biometric-gated encryption, and works for both age and SSH identities that have a passphrase. Enabling or changing your passphrase invalidates biometric unlock and asks you to re-enable it. Desktop and Android below 11 keep the passphrase-only flow

### Changed

- Migrated the entire Rust backend library (`rustpass`) from synchronous `std::fs` to `tokio::fs`, eliminating UI freezes during file I/O on Android devices
- Post-quantum (X-Wing) age keys are now recognized and show a clear "not yet supported" message during setup and decryption, instead of failing with a confusing error. Post-quantum recipients in the repository are also labeled accurately in the setup wizard rather than appearing as ordinary age keys

### Removed

- SSH key identities are no longer re-encrypted by gpm; they rely on their own native passphrase protection, matching how age handles them. The setup wizard now uses a single passphrase field (for x25519 at-rest encryption or SSH key decryption, depending on the identity type) instead of two separate fields

## [v0.3.0] - 2026-06-12

### Added

- Optional passphrase to encrypt identity at rest (setup wizard or settings)
- Unlock screen when identity is passphrase-encrypted
- Auto-lock after 5 minutes of inactivity
- Passphrase management in settings: set, change, or remove
- SSH key authentication for Git operations (`git@host:repo` and `ssh://` URLs)
- Passphrase-encrypted SSH private keys as age identities (passphrase prompted during setup, cached at runtime)

## [v0.2.0] - 2026-06-10

### Added

- On-device ed25519 SSH key generation with optional passphrase
- Settings page with public key display and private key export
- Two-step setup wizard: clone repo first, then select a recipient and provide matching age identity
- Recipient discovery from `.gopass-recipients` / `.age-recipients` files in cloned repositories
- Identity validation on setup: derived public key is checked against known recipients
- SSH key recipient support: decrypt entries encrypted to `ssh-ed25519` or `ssh-rsa` public keys using the corresponding SSH private key as identity
- Recipient type detection (x25519, SSH ed25519, SSH RSA) with SSH badge in setup wizard
- SSH key reuse: one-click "Use my SSH key for decryption" when Git auth and age recipient use the same key

## [v0.1.0] - 2026-06-08

In this initial release, we have implement a read-only age-only gopass password client for Android, built with Tauri v2 + Rust + Vue 3.

### Added

- Clone age-encrypted gopass repositories via HTTPS + PAT
- List and search password entries
- Decrypt and copy passwords to clipboard
- Show password with 30-second auto-clear and lifecycle cleanup
- Pull-to-refresh to sync with remote repository
- Android APK signing and per-architecture release builds

[Unreleased]: https://github.com/yzx9/gpm/compare/v0.10.0...HEAD
[v0.10.0]: https://github.com/yzx9/gpm/compare/v0.9.0...v0.10.0
[v0.9.0]: https://github.com/yzx9/gpm/compare/v0.8.1...v0.9.0
[v0.8.1]: https://github.com/yzx9/gpm/compare/v0.8.0...v0.8.1
[v0.8.0]: https://github.com/yzx9/gpm/compare/v0.7.3...v0.8.0
[v0.7.3]: https://github.com/yzx9/gpm/compare/v0.7.2...v0.7.3
[v0.7.2]: https://github.com/yzx9/gpm/compare/v0.7.1...v0.7.2
[v0.7.1]: https://github.com/yzx9/gpm/compare/v0.7.0...v0.7.1
[v0.7.0]: https://github.com/yzx9/gpm/compare/v0.6.0...v0.7.0
[v0.6.0]: https://github.com/yzx9/gpm/compare/v0.5.0...v0.6.0
[v0.5.0]: https://github.com/yzx9/gpm/compare/v0.4.0...v0.5.0
[v0.4.0]: https://github.com/yzx9/gpm/compare/v0.3.0...v0.4.0
[v0.3.0]: https://github.com/yzx9/gpm/compare/v0.2.0...v0.3.0
[v0.2.0]: https://github.com/yzx9/gpm/compare/v0.1.0...v0.2.0
[v0.1.0]: https://github.com/yzx9/gpm/releases/tag/v0.1.0
