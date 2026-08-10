# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- The dropdowns on the **Setup** screen (clone / create / create-GPG), the **password generator**, and the **create-from-template** password fields now open the app's own themed option sheet — the same picker used throughout Settings — instead of the device's built-in dropdown. They're consistent with the rest of the app and a touch easier to use on a phone.

### Fixed

- On **cold start**, the first screen now appears right away instead of flashing blank for a split second before the page loads in.
- A **GPG/OpenPGP store created by gpm** now opens and decrypts cleanly under desktop gopass and other gopass clients. gpm was writing the store's recipient marker in lowercase where gopass expects uppercase, so a store you created on your phone wouldn't be recognized when cloned to desktop gopass. Opening an existing gopass-created store was already unaffected; this fixes the stores gpm itself creates. On upgrade, any GPG signing keys you already trusted are also re-normalized to the correct case, so they keep matching and any previously-dismissed unverified-signer warnings stay dismissed.

## [v0.18.0] - 2026-08-09

### Added

- gpm now shows a secret's **`Key: Value` fields** (like `user:`, `url:`, `note:`) as **named, copyable fields** instead of one text blob, and lets you **edit them as structured rows** — add, remove, and fill each field separately. A field whose name looks secret (`password`, `pin`, `token`, …) is masked by default with a show/hide toggle. This matches how gopass models these fields; the on-disk format is unchanged and existing secrets get the cleaner view automatically.
- gpm can now **open and use an existing GPG/OpenPGP-encrypted gopass store** — clone it, import a GPG secret key through the file picker, verify the key's passphrase, then list, copy, and create secrets just like an age store. This is for people with legacy or work-mandated GPG repos; no system `gpg` is needed and it works the same on Android and desktop. (In-app GPG key generation and recipient management are not part of this yet.)
- gpm can now also **create a brand-new GPG/OpenPGP gopass store** on-device by importing an existing GPG secret key — the create-side counterpart to opening an existing store. The imported key seeds the store's recipient index (`.gpg-id` + `.public-keys/`) exactly as `gopass init` does — same two init commits and `diff.gpg` config — so a store created on your phone clones and decrypts cleanly under desktop gopass. Age remains the default for a fresh start; GPG is for users who already have a key.
- gpm can now **export an entire repository** from **Settings → Repository** as a single portable file — a full, encrypted copy of your vault's history (the same data your git remote holds) that you can hand to another device, keep as a backup, or open in desktop gopass/`git`. It runs without unlocking — your secrets stay encrypted — and a short note inside the file explains how to restore it. Importing an export back into gpm is not part of this release.

### Changed

- gpm's license broadened from **Apache-2.0** to **MIT OR Apache-2.0** — you can now use, modify, and distribute gpm under either license, at your option. This matches the convention used across the Rust ecosystem. Existing use under Apache-2.0 is unaffected; the new MIT option is purely additional.

## [v0.17.2] - 2026-08-07

### Fixed

- After upgrading from **0.17.0**, turning on **App Lock** with biometrics no longer fails with "Stored vault key is malformed", and your at-rest settings and secrets decrypt again. A change in 0.17.1 altered how the on-device key store stored the encryption keys on Android, so keys written by 0.17.0 couldn't be read back — which broke both the App Lock vault and the keys that protect your data at rest. Keys are now read in a way that accepts the 0.17.0 format (and the short-lived 0.17.1 one), so your lock and your data recover on update with nothing for you to do.

## [v0.17.1] - 2026-08-06

### Changed

- On Android, **all** of your app settings — display language, theme, auto-lock timers, autosync, screen-capture mode, and the rest — are now **encrypted at rest**, with no settings file left in plaintext. (A few non-secret display preferences used to be stored unencrypted so they could be read before the app unlocked; the at-rest key is now available at startup, so that workaround is gone.) Nothing about how the app behaves changes — your settings carry over as-is. Desktop is unchanged: it has no device key store, so its settings stay in plaintext there as before.
- Diagnostic log lines that come from the app interface — the lines you see in **Settings → Logs** and in an exported bug-report bundle — no longer repeat where they're from. Each used to say "frontend" twice over; the lines are now shorter and easier to read.
- In **Settings**, the **Logs** entry moves up to sit right after **Repository**, grouped with the other reference pages — **Security**, **Permissions & data**, and **About** — just below the actual settings (General, Lock & identity, Repository). Those four pages explain or document the app rather than holding a setting, so they now read as one group instead of Logs sitting alone at the bottom.

### Fixed

- **Copy password** — and copying a TOTP code, a generated password, or an old revision's value — no longer fails with an error. The clipboard-clear notification's text was reaching the native layer in a form it couldn't read, so every copy broke at the last step; copying and the auto-clear notification now work as intended.
- On Android, tapping the **biometric** row on the **Permissions & data** page (offered when no fingerprint is set up) now opens the system Security settings so you can enroll one — it previously did nothing.

## [v0.17.0] - 2026-08-05

### Added

- You can now **browse a secret's past versions** and view or copy an old value. Open any secret and tap **Revisions** to see every change recorded for it, newest first, each with its signature status. Tap a version to reveal it — an old value is always marked as a past version (date and commit) so it can't be mistaken for the current one, and it auto-clears like any revealed password. A version encrypted for an identity you no longer have shows as "can't decrypt" instead of failing, and one that deleted the secret is called out. This is the recovery and audit counterpart to gopass's `history` / `show --revision`.
- gpm now asks you to confirm before turning off three settings that can expose your secrets: **screen capture protection** (otherwise screenshots or screen recording could capture a revealed password), **Auto-lock → Never** (which keeps the identity unlocked for the whole session), and **clipboard auto-clear** (which leaves a copied password on the clipboard for other apps to read). Each prompt states the consequence and cancelling leaves the setting unchanged — this guards against an accidental toggle, not a decision you've already made.
- The **App Lock** screen now fills the whole display as a solid surface, so nothing behind it — your entry list or an open secret — is visible while the app is locked. It used to be a small card over a dimmed, still-readable view. A discreet **Export diagnostics** link at the bottom lets you grab a bug-report bundle without unlocking (log, device info, and redacted settings — no secrets), and a temporarily unavailable fingerprint sensor now says so with a clear retry message instead of a generic failure.

### Changed

- **Settings → General** now keeps **AutoSync** and its **Background sync** option together in one card — background sync appears beneath AutoSync only while it's on — instead of two separate cards, and the AutoSync toggle now uses the same On/Off order as every other toggle.
- When **Auto-sync** is on, editing, deleting, or creating a secret that collides with another device's change no longer silently overwrites it — you now get a clear per-entry choice (keep your version or theirs); a delete a teammate already did is recognized as "already removed" instead of claiming a commit, and a create that reuses a name another device took asks before overwriting. With Auto-sync off it still surfaces when you manually sync.
- Cloning a repository, syncing, or testing a connection against a server that never responds no longer hangs for a long time — gpm now gives up after about 20 seconds of trying to connect (about 60 seconds overall, covering the SSH sign-in step) and shows a clear error, instead of waiting out the system's long network timeout. Cancelling while gpm is still signing in, or during a "remote copy exists" check, now also responds promptly instead of only after data starts moving.

### Fixed

- Text fields and dropdowns throughout the app no longer have a faint gray fill that could make a live setting look disabled or switched off. They now read as active outlined fields. Read-only boxes that display a key, token, or log keep their tinted background, since that signals "display only."
- Switching the **display language** back to **Follow system** now switches to your device's language right away. It used to stay stuck on the language you'd pinned earlier (for example, Chinese) and only correct itself after restarting the app.
- Secrets whose content isn't valid text (for example, one whose body somehow holds raw bytes) used to be silently corrupted if you opened and saved them in gpm — the save rewrote them with mangled characters. gpm now refuses to edit such a secret and points you to the gopass command line instead, so the original is never damaged. Reading and syncing these secrets is unchanged. A secret whose password itself isn't valid text likewise can't be copied in gpm (it would only copy an empty string); copying it now shows a hint to use the gopass command line instead.
- If you pinned the theme to **Light** or **Dark**, the app no longer flashed your system theme for a split second on cold start before switching to your pinned theme. It now opens in your pinned theme right away — app colors, scrollbars, and form-control colors included.
- If you pinned the **display language** (for example, English on a Chinese-language device), the app no longer flashed your system language for a split second on cold start before switching to your pinned language. It now opens in your pinned language right away.
- Bottom sheets (the edit/delete conflict and sync-divergence prompts, and other bottom-sheet dialogs) no longer tuck their lowest button under the Android gesture-navigation bar — they now respect the bottom safe-area inset so every button stays tappable.

## [v0.16.1] - 2026-08-04

### Changed

- The timing pickers in **Settings** — **auto-lock**, **re-lock when inactive**, **background sync**, and the **password-view** and **clipboard auto-clear** timers — now lead with the decision that matters (On/Off, or for auto-lock, Immediate / After idle / Never) and reveal the exact duration as a themed in-app sheet only when it's relevant. They used to be a long, wrapping row of every option at once, or (for background sync) the phone's system dropdown that ignored the app's theme. The **display-language** picker is now the same themed sheet, ready for more languages.
- Every bottom sheet — the sync-conflict review, the unlock prompt, and the new background-sync picker — now leaves room for the phone's gesture-navigation bar, so its last row is never half-covered or awkward to tap.

### Fixed

- On Android, **App Lock** and **Biometric Unlock** were broken: enabling or disabling either, or unlocking with them on, could fail with an error. For upgraders this showed up right after the fingerprint prompt when opening the app. Both now work correctly.

## [v0.16.0] - 2026-08-03

### Added

- On the **Permissions & data** screen, the biometric row now says **Enabled** when fingerprint/face unlock is on (and **Ready** when the hardware is set up but it isn't), and a link takes you to **Lock & Identity** to turn it on or off. Landing there scrolls to the biometric card and briefly highlights it so you can find it.
- Secrets saved in gopass's older `GOPASS-SECRET-1.0` format (used for a few months in 2020–2021 and still produced by some older stores) now open correctly. gpm used to treat the format's identifying header line as the password, so it showed — and copied — the wrong string; the real password, which that format stores in a `Password:` field, is now used instead. Editing one of these secrets still rewrites it in the current format, exactly as gopass does; gpm never writes the old format.
- gpm can now **export a gopass binary attachment** to a file you choose. Entries that are attachments (created with `gopass fscopy` / `gopass binary attach`) show their filename and size and an **Export Attachment** action instead of an empty password and a wall of base64 — tap it to save the original file to your device or desktop. The decoded bytes never pass through the app's UI. Editing or replacing an attachment isn't supported yet.
- A new **personal access token** screen (Settings → Repository → Manage token) shows a masked preview of your stored token, lets you replace it — the new token is checked against the remote before it's saved, so a mistyped or expired one is caught immediately — and clear it. A matching **Remove key** action on the SSH key screen lets you switch away from SSH authentication (to a stored token, or to none) without re-running setup.

### Changed

- On the **Permissions & data** screen, the Notifications row's off-state now just says **Off** instead of the longer hint — the arrow already shows you can tap to re-enable it.
- The **Permissions & data** screen now spaces its cards apart — they used to sit edge to edge, matching the Security screen's stacked-card layout.
- The diagnostic log now records the app's own activity — each screen you open, and any operation that fails (a copy, reveal, sync, create, and so on) — alongside the existing backend trace, so a failure leaves a clue in **Settings → Logs** instead of disappearing. A failed copy, for example, used to leave no trace at all; the log now captures what went wrong.
- The many small buttons throughout the app — copy, close, show/hide, cancel, back, retry, and so on — now share one consistent look and the same tap, hover, and keyboard-focus feedback, instead of each screen having its own slightly different version. A few inline destructive actions (removing a trusted signing key, removing a picked identity file, and retrying a failed entry-list load) are now a quiet red link rather than a generic chip, so they read clearly without competing with the main action.
- The Repository settings page now shows a dedicated **Git Authentication** card naming the active method — SSH key, personal access token, or none — instead of a small line under the URL, and surfaces a second stored credential when both are present so neither sits hidden.
- Opening the SSH key screen with no key configured is no longer shown as a red error — it's a normal empty state.
- Your stored personal access token and SSH private key (with its passphrase) are now masked before they reach the app's interface, so only a masked preview is shown; the full values stay in the backend.

### Fixed

- Creating a new store without a remote now works: the repository-URL field in setup was marked required, so leaving it empty silently blocked the **Create** button in the real app — even though the on-screen hint already said a remote was optional. You can now skip the URL and keep the store entirely local.
- **Background sync** now runs while **App Lock** is on. Every scheduled background pull used to be silently skipped as soon as the app-launch biometric lock was enabled, so a store you rarely opened never caught up. The sync only needs the git credential (not your identity) to pull, so it now keeps your secrets current even when the app stays locked behind biometrics.

## [v0.15.1] - 2026-08-02

### Changed

- The diagnostic log now opens with the app's version and build (so a bug report is never ambiguous about what was running) and records each time the app returns to the foreground, loses or regains window focus, or exits — giving a clearer picture of what the app was doing when you attach the log.
- Turning off **Biometric Unlock**, **App Lock**, or **Identity Auto-Unlock** now asks for confirmation first. Re-enabling any of them takes your fingerprint or passphrase again, so an accidental tap no longer disables them instantly.
- The **Logs** screen is reworked so it isn't cramped on a phone: Refresh, Export, and Clear have moved out of the header (where three long buttons didn't fit) into a toolbar below the title. The verbose (debug-logging) toggle is now a single switch that shows the time remaining while it's on, instead of an On/Off picker, and the log opens scrolled to the newest entries at the bottom rather than the oldest.
- The Settings list drops the one-line summaries that used to sit next to each category — each category's own page already shows the detail, so the summaries only added clutter and often got cut off. Only About still shows the installed version; everything else is now just the label and arrow, and the Logs row's arrow lines up with the others again.
- Confirming a destructive action — deleting an entry, resetting all settings, or tapping a red Confirm in a dialog — now shows a solid red button instead of an outlined one, so it's clearly the main action next to its Cancel.

### Fixed

- Syncing no longer writes the pulled commits' messages, author identities (name and email), or signer key fingerprints to the diagnostics log — those fields are redacted before they can reach a log line. The log is mirrored to Android logcat and shipped inside an exported diagnostics bundle, so this keeps your commit messages and git identity out of both.
- In the Lock & Identity passphrase prompts (set, change, enable biometric, and enable identity auto-unlock), the action button now uses the accent color and matches the Cancel button's width — previously it looked the same as Cancel and stretched wider, making the two hard to tell apart.
- The headers on the Settings pages no longer carry an icon next to the title, so they read like the rest of the app's headers.

## [v0.15.0] - 2026-08-01

### Added

- gpm now syncs your store automatically when you open the app or come back to it from the background, so it stays current with your other devices without a manual pull-down. This runs only when **Auto-sync** is on (turn Auto-sync off and nothing syncs automatically, as before). It's best-effort and stays out of your way: it never raises a sync-conflict dialog on its own — if your local and remote copies have diverged, a small status badge appears and you tap it when you're ready to review; network hiccups are retried quietly without nagging.
- When **Auto-sync** is on, gpm can now **fetch new entries from your other devices in the background** (Android), so they appear without you opening the app — useful if you mostly rely on autofill. Pick a cadence in **Settings** (every 1 hour to every 3 days, or off). It's pull-only, runs only while Auto-sync is on, and skips while the app is open or under App Lock; if a background fetch finds your local and remote have diverged, the same status badge as the on-open sync appears for you to review when you next open the app.
- A new **Settings → Permissions & data** screen lists what gpm accesses on your device — notifications, biometrics, clipboard, network, and files — explains why each is needed, and, when Android has stopped re-asking about notifications or biometrics after you dismissed the prompt twice, links you straight to the relevant system settings to turn them back on. The clipboard, network, and files rows are explainers only; Android offers no separate permission for the clipboard.
- You can now **cancel a save in flight**. If a create, edit, or delete hangs while syncing to a slow or dropping remote, a Cancel button appears next to the Saving button — tap it to abort within a second or two instead of waiting out the network timeout. Your change either stays local and publishes on the next manual Sync (if the save had already committed), or nothing was saved yet (if it canceled during the initial pull).
- **Settings → Logs → Export diagnostics** saves a single zip bundle to a location you choose, ready to attach to a bug report. It packages the full log, your display preferences, and device info, plus your repository settings (remote host, commit identity, trusted public keys) when the app is unlocked. Access tokens, SSH keys, and passphrases are replaced with `[REDACTED]`, and when the app is locked the repository settings are omitted entirely; a confirmation first tells you what's leaving the device.
- **App Lock can now re-lock itself when it sits idle.** If you leave gpm open and walk away, it can re-lock after a few minutes of inactivity (Off, or 5 / 15 / 30 minutes) so the vault isn't left open. Unlike opening or returning to gpm — which asks for your fingerprint right away — an idle re-lock just covers the screen and waits for you to tap, since you're likely still right there. New installs default to 5 minutes; if you already had App Lock on it starts off, and you can turn it on in Lock & Identity.

### Changed

- The Settings → Logs screen replaces its four-level log selector (Errors / Warnings / Info / Debug) with a single **Verbose** toggle. Turn it on to capture everything — Debug level — for about ten minutes, handy for attaching to a bug report; it turns itself off after the window so logging stays focused the rest of the time. A verbose session survives a restart, so relaunching to reproduce an issue keeps capturing (including startup), and gpm lets you know when you relaunch with verbose still on. Outside that window the app records at the Info level as before.
- On Android, copying a secret no longer shows gpm's own "Allow notifications?" confirmation before the system notification-permission prompt — gpm now goes straight to the system prompt. If you had dismissed gpm's old in-app prompt, the next copy surfaces the Android permission dialog for the first time. After you allow or deny twice, Android stops re-asking; change the choice any time from Android's notification settings. The clipboard auto-clear timer is unaffected either way.
- Page transitions now slide on every navigation. Previously the animation was skipped when moving between a screen that shows a secret (entry detail, edit, create, generate) and one that doesn't, because screen-capture protection used to cover the whole page. Protection now follows the secret itself — the screenshot block is raised only while a secret is actually on screen — so the boundary no longer needs freezing, and pages that never show a secret (like Settings → Repository) are screenshot-safe the whole time you're on them.
- **Identity Auto-Unlock now ties your passwords' lock to App Lock.** When Auto-Unlock is on (which requires App Lock), your passwords unlock and lock together with the app, so the separate Auto-lock timing doesn't apply meanwhile — it's shown as managed by App Lock. Turn Auto-Unlock off to set a separate auto-lock time again.
- The separate **Locking** and **Identity & unlock** settings pages are now combined into a single **Lock & Identity** page.
- Confirmation prompts for destructive actions — deleting an entry, exporting your private SSH key, clearing the log, exporting diagnostics, and removing a trusted signing key — now use gpm's own dialog instead of the phone's generic system popup, so they match the rest of the app and dismiss the same way as your other in-app prompts.

### Fixed

- A save or manual Sync whose push hung on an unresponsive remote no longer locks the whole app until the network timeout — the push phase is now abortable, so Cancel frees the store and you can keep working immediately instead of waiting minutes for the transfer to time out.

- On Android, tapping a control no longer leaves it stuck in a highlighted state until you tap somewhere else. The highlight that previews a press now only appears for a mouse or trackpad, the way it does on desktop; touch press feedback is unchanged.

### Security

- Lock mode, auto-sync, the screen-capture protection setting, and the other app behavior preferences are now encrypted at rest on Android, where they were previously stored in plain text. Nothing about how they work changes; this only affects what someone inspecting the device's stored data could see. Desktop is unaffected — it has no hardware key store to encrypt with, same as the rest of gpm's stored data there.

## [v0.14.2] - 2026-07-27

### Changed

- In Settings, the option pickers now all run in the same left-to-right direction — the more cautious choice on the left, the less cautious on the right — matching the Auto-lock and auto-clear pickers that already worked that way. Screen capture protection now reads Always → Sensitive → Off, commit signature verification reads Enforce → Audit → Off, and Auto-sync reads Off → On. Nothing about what each option does has changed; only the order they sit in.
- The About → Licenses list now also includes the project's dev and build tooling — Vite, Vitest, TypeScript, and the rest — not just runtime dependencies, so the open-source attribution covers everything gpm is built with.
- When the identity auto-locks after a stretch of inactivity, the unlock screen no longer automatically pops up the fingerprint/face prompt. That lock fired because you stepped away, so the prompt would usually have expired by the time you picked the phone back up. The unlock screen now just waits for you to tap — opening the app, or coming back from a manual lock, still prompts automatically.

### Fixed

- On the About → Licenses screen, the search box sat a little narrower than the license rows beneath it. It now lines up flush with them.

## [v0.14.1] - 2026-07-24

### Fixed

- The Settings → Logs screen no longer fills with low-level system trace lines on Android — in particular the repeating JNI method-call chatter that appeared around startup. gpm records no trace-level diagnostics of its own, so those lines carried no useful information; the log now stays focused on meaningful app activity.
- When you returned to gpm after it had auto-locked — with App Lock and fingerprint/face unlock both turned on — unlocking could take several seconds, sometimes much longer, or appear stuck on the spinner. A second biometric prompt was being triggered on top of the app-unlock one. Unlocking now happens once, so returning to a locked app is as quick as a fresh launch.

## [v0.14.0] - 2026-07-23

### Added

- You can now pin the app's color scheme to Light or Dark from Settings → General, right under Display Language. "System default" (the previous, and still the default, behavior) keeps following your device's light/dark setting with no flash; pinning Light or Dark overrides it so the app stays in your chosen scheme even when your device disagrees.

### Changed

- The **Copy 2FA Code** button on an entry's detail page now appears only for entries that actually store a 2FA code, instead of on every entry. When your lock setting keeps you unlocked between actions, gpm detects this automatically as you open each entry; when it locks again after each action, the button shows on first view and then settles to the right state once you copy or view that entry.
- Android's "Screen capture protection" setting is now three modes instead of a single on/off switch: **Off** (nothing blocks capture), **Sensitive** (the previous default — screens that show a secret block capture while the list, history, and other non-secret screens stay capturable), and a new **Always** (every screen blocks capture at all times). Your current choice carries over automatically — On becomes Sensitive, Off stays Off. Under Always, screen transitions always slide smoothly, since capture protection no longer toggles between routes (the same way it already behaved under Off).
- Most Settings screens — the Settings hub, General, Locking, Add-trusted-key, and Logs — no longer block screenshots and screen recording on Android. They show only non-secret configuration (your language, lock timing, public signing-key fingerprints, or log entries), so they're now treated like the secret list and history, which were already capturable. The Repository, Identity, and SSH Key screens still block screenshots: Repository shows your full git remote address (which can contain an embedded access token), and Identity and SSH Key can reveal a passphrase or your private key.

### Fixed

- Pinching with two fingers (or double-tapping) used to zoom the app's interface, which could accidentally scale it out of shape and break the layout. The app now stays at its fixed size, like a native app.
- With the Android "Screen capture protection" setting turned off, sliding between screens is now consistent everywhere. A move between a page that can show a secret (an entry's detail, the create or generate screens, the identity or repository settings) and one that can't (the list, history) used to snap instantly with no slide, because that transition was frozen on secure↔non-secure boundaries regardless of the setting. With protection on, those boundaries still snap on purpose — so a secret page is never caught mid-slide while capture protection is being cleared.
- On Settings screens, the option pickers (language, auto-lock timing, auto-sync, signature mode) used to draw each option as its own bordered, rounded tile inside an already-bordered, rounded card, which read as a confusing box-inside-a-box. Each option is now a clean segment of its group, with the current choice clearly highlighted instead.

## [v0.13.0] - 2026-07-17

### Added

- gpm can now copy the current two-factor (TOTP) code for an entry that stores one. Add a `totp:` line or an `otpauth://` link to an entry's notes — the same format gopass uses — then use the new **Copy 2FA Code** button on that entry's detail page. Like copying a password, the code goes straight to your clipboard and clears automatically; the 2FA seed itself never leaves the app's encrypted core. See the security model for when it's better to keep a 2FA code in a separate app instead.
- A new "About" screen, reached from Settings, brings together gpm's overview, acknowledgements, and the full open-source license list in one place. See the projects gpm is built on — gopass, age, Tauri, Vue, and more — and search or expand any of the hundreds of bundled dependencies to read its license text.
- A new **Logs** screen, reached from Settings, shows gpm's diagnostics log so you can review what the app has been doing — handy when troubleshooting or filing a bug report. Pick how detailed the log is (Errors / Warnings / Info / Debug) right from the screen: the change takes effect immediately and is remembered across restarts. You can also clear the log. Logs record entry names and operation outcomes only, never secret content.
- The in-app Logs screen now captures a richer set of operations — copying and viewing entries, creating and editing secrets, syncing, unlocking, app-lock toggles, setup, and signature-trust changes — so the diagnostic trail matches what you actually did. As before, only entry names and operation outcomes are recorded, never secret content.
- A new "Security" screen, reached from Settings, explains in plain language — in both English and Chinese — how gpm keeps your secrets safe: secrets stay on your device in an age-encrypted copy synced over git to your own repository, copying a password never brings it into the app's interface, the unlock key is wiped after every use, files are encrypted at rest on Android, and there's an optional fingerprint/face App Lock and optional commit-signature verification. It also states plainly what gpm does and does not protect against, with a link to the full security model.

### Changed

- Every screen's Back button now sits in the same place — a back arrow at the top-left — instead of appearing top-left on some screens, top-right on the Settings / SSH key / add-key screens, and not at all on History. The History screen also gained its own Back button instead of relying only on the system back gesture.
- Settings is now a hub — General, Locking & auto-clear, Identity & unlock, Repository, and About — instead of one long scrolling page. Each row shows a quick summary of its current state (your language, lock mode, identity status, repo host, or app version), and tapping one drills into just those settings. Grouping the repository-specific settings on their own page also clears the way for managing multiple repositories later.

### Fixed

- When creating a custom or preset secret hit a sync conflict — the same secret changed on another device since you last synced — the "keep mine / adopt remote" choice never appeared, so you couldn't resolve the conflict from the create screen. The prompt now shows as intended.
- Adding a trusted signing key in Settings silently dropped its error message when the add failed; the message is now shown again.
- On the first setup screen, the "Clone an existing store" and "Create a new store" choices could show their internal labels (e.g. `setup.clone`) instead of the readable text.

## [v0.12.1] - 2026-07-14

### Fixed

- On cold start with App Lock on, the small signature-status light next to the app name stayed stuck showing "verification off" (or an unchecked state) even after you unlocked — it now refreshes to the real status the moment you unlock, matching the secret list that already reloads on unlock.
- When sliding between screens — for example, tapping a create-secret step like "Website" — the Back button on the outgoing screen used to leap up into the camera notch for a moment before settling back, most noticeably on phones with a large display cutout. The outgoing screen now keeps its top spacing throughout the slide.
- With an auto-lock timeout set (e.g. "after 1 minute"), browsing the secret list, searching, or otherwise using the app no longer triggers a surprise unlock screen mid-browse. Any tap, scroll, or key press now keeps the auto-lock timer refreshed — previously only viewing or copying a secret did, so simply reading the list for a minute would lock you out.

## [v0.12.0] - 2026-07-10

### Added

- The Android biometric prompts (unlocking the app and unlocking your identity) and the clipboard-clear notification — including its name in your system notification settings — now follow your display language instead of always being English.
- The History screen now loads in pages instead of stopping at the latest 50 commits — scroll to the bottom (or tap "Load more") to browse older commits and their signature status all the way back to the repo's first commit. An explicit "Load more" button is always available even when the browser can't observe scrolling.
- During setup, the identity key you paste is now masked (shown as dots) instead of displayed in plain text, so it stays hidden from anyone glancing at your screen.

### Changed

- The pull-to-refresh sync indicator is rebuilt: syncing now leads with a spinning refresh icon and a full-width progress bar instead of a lone stop button, the stop control is smaller and calmer, and the pull-down spinner sits with breathing room above the search box.
- While a revealed password is on screen, its "auto-clears in Ns" hint now counts down live each second instead of sitting on a static number, so you can see exactly how long is left before it wipes.
- The SSH key view, the add-trusted-key form, the edit-entry screen, and the create-secret steps are now their own screens — pressing Back returns you to the previous screen instead of jumping all the way out to the secret list. During setup, Back on the identity step now returns to the clone step instead of leaving setup.

### Fixed

- Your auto-lock timing, the password view and clipboard auto-clear timers, and the AutoSync choice are now device preferences — they stick around when you reset your repository or connect a different one, instead of being wiped along with the repository the way they used to be. The App Lock toggle in Settings also now reflects the lock's real on/off state rather than a stored flag that could drift out of sync.
- On the App Lock screen, the "Unlock with biometric" button was shrunk to its label and left-aligned at the edge of the card. It now stretches the full width with centered text, matching the other unlock buttons.
- On Android, tapping a button no longer flashes a solid color block that hides the button's text and rounded shape — buttons now show a clean press highlight in their own color. Long-pressing a button also no longer pops up the system text-selection menu on its label.
- Sensitive values on a screen — an exported private key, a typed passphrase, a pasted identity, or a secret you're editing — are now cleared the moment you leave that screen (or your identity locks), instead of lingering in the app's memory until later. Previously only the entry detail screen did this consistently; Settings, the setup flow, the generator, and the unlock screen now match.
- While an unlock, sync-resolve, or other dialog was open, you could still scroll the list behind it by dragging on the dimmed background. The background now stays frozen for as long as any dialog is up.

## [v0.11.0] - 2026-07-08

### Added

- You can now choose gpm's display language — English, 中文, or “System default” to follow your device — under Settings. The app ships English and Chinese to start, remembers your choice across launches and repository resets, and follows your device language by default.

### Changed

- Relative timestamps in the entry list and history — like "5 minutes ago" or "Mar 15" — now follow your display language instead of always being English.
- The Android clipboard-clear notification now shows how long until the secret auto-clears (for example, "auto-clears in 45s"), so the timeout is visible right in the notification shade instead of only in Settings. The tap-to-clear action is unchanged.
- The entry list now shows one line per entry — just the name — instead of repeating the path underneath it, since the two were nearly identical (the path was only the name plus its file extension) and the second line added little while crowding the list. The full path now appears at the bottom of the entry detail screen, as quiet footer metadata that sits without competing with the title.

### Fixed

- With App Lock turned on, the password list sometimes stayed empty after you unlocked with your fingerprint or face, needing a manual pull-to-refresh before your entries appeared. The list now loads reliably as soon as the app unlocks.

## [v0.10.0] - 2026-07-06

### Added

- Opening a page now slides in from the right, and going back slides it the other way — a stack-style transition between pages. Transitions to or from a page that shows secrets swap instantly, so the screen-capture guard never leaves a secret visible during the animation.
- After you copy a password on Android, a small notification appears in your notification shade while the secret is on the clipboard — tap it to clear the clipboard immediately, without gpm taking over the foreground. It dismisses itself when the clipboard is cleared, whether by your tap or the automatic timer. The first time you copy, gpm asks once for permission to show notifications; if you decline, copying still works, just without the notification. Android only; desktop is unchanged.
- GPG/OpenPGP-signed commits are now verified, the same way SSH-signed commits already are — instead of being flagged as an unsupported format. Under Settings → Trusted signing keys you can paste a GPG public key (or import a `.asc` file from your device) to trust a signer; once trusted, a commit signed by that key shows as verified in Audit/Enforce mode and in the history view. A commit signed by a GPG key you haven't trusted shows a distinct "Unverified signature" status with a hint to add the signer's key, since GPG signatures don't carry the public key the way SSH signatures do.

### Changed

- gpm now uses clearer language for what it stores on your device to open your repository — your private key and git sign-in together are called your **app key**. Setup and the App Lock setting explain this in plain words instead of the confusing "at rest" phrasing, so it's clearer what your passphrase guards (the private key) and what App Lock seals behind your fingerprint (the whole app key).
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

[Unreleased]: https://github.com/yzx9/gpm/compare/v0.18.0...HEAD
[v0.18.0]: https://github.com/yzx9/gpm/compare/v0.17.2...v0.18.0
[v0.17.2]: https://github.com/yzx9/gpm/compare/v0.17.1...v0.17.2
[v0.17.1]: https://github.com/yzx9/gpm/compare/v0.17.0...v0.17.1
[v0.17.0]: https://github.com/yzx9/gpm/compare/v0.16.1...v0.17.0
[v0.16.1]: https://github.com/yzx9/gpm/compare/v0.16.0...v0.16.1
[v0.16.0]: https://github.com/yzx9/gpm/compare/v0.15.1...v0.16.0
[v0.15.1]: https://github.com/yzx9/gpm/compare/v0.15.0...v0.15.1
[v0.15.0]: https://github.com/yzx9/gpm/compare/v0.14.2...v0.15.0
[v0.14.2]: https://github.com/yzx9/gpm/compare/v0.14.1...v0.14.2
[v0.14.1]: https://github.com/yzx9/gpm/compare/v0.14.0...v0.14.1
[v0.14.0]: https://github.com/yzx9/gpm/compare/v0.13.0...v0.14.0
[v0.13.0]: https://github.com/yzx9/gpm/compare/v0.12.1...v0.13.0
[v0.12.1]: https://github.com/yzx9/gpm/compare/v0.12.0...v0.12.1
[v0.12.0]: https://github.com/yzx9/gpm/compare/v0.11.0...v0.12.0
[v0.11.0]: https://github.com/yzx9/gpm/compare/v0.10.0...v0.11.0
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
