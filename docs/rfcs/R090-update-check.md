# Passive update-availability check (detect-and-link, no self-update)

**Priority:** P3
**Status:** Draft
**Phase:** Next

## What

Add a detection-only update check: on cold start the app probes whether a newer
stable release exists than the built-in version, and surfaces availability
**passively** — a red dot on the Settings _About_ entry (which dismisses once
the user has opened About for that version) and a persistent red dot beside the
version on the _About_ page, next to an action that links out to the latest
release. **No download, no install, no banner, no notification** — and therefore
platform-agnostic; the same logic serves Android and desktop unchanged.

Serves a feature that has no separate spec PRD yet — it is a small,
self-contained enhancement, so the product context is carried inline below. A
`docs/specs/NNN-update-check/` PRD can be split out if this grows.

## Why

gpm ships as GitHub Releases of signed, split-per-ABI APKs only — no Play Store,
no F-Droid, no desktop distribution. Every user sideloads, and **no store
auto-updates anyone**. Today nothing in the app signals that a newer version
exists; users discover releases — including security fixes — only by chance. For
a password manager whose users must run patched versions, that is a real gap,
and closing it costs little: the app already talks to github.com for git sync,
so an unauthenticated release probe is strictly less sensitive than behavior the
app already has.

The design intent is **minimal and non-intrusive**: it does not try to update the
app (impossible on Android without a store, and unjustified on desktop, which
isn't distributed) — it only answers "is there something newer?" and, if so,
offers the same link the user would have used to install in the first place.

## Context

**Decoupled from secrets.** The probe is a public, unauthenticated GET; it needs
no identity, no vault unlock, and no App Lock interaction, so it adds no surface
to any secret-handling path. It runs Rust-side because the WebView CSP forbids
external connects — meaning the network call never involves the WebView and needs
no new WebView capability or permission. The "open the release page" action
reuses the existing open-URL capability. Net new permissions: none.

**Trigger.** On cold start, fire-and-forget and non-blocking, behind a ~24h
staleness cache (≤1 probe per day). The red dots read the cached result and
render instantly, which is the reason cold-start beats a lazy Settings-open
probe (see Alternatives). It deliberately does **not** use the background-work
scheduler (R077): a headless daily network probe to light a dot the user only
sees in Settings is heavier than the job warrants.

**Version sources.** _Current_ version is the build-baked app version, already
available client-side with no IPC. _Latest_ version is read from GitHub's
release-redirect target — a request to the "latest" URL resolves to
`/releases/tag/vX.Y.Z`, so the version comes out of the redirect URL itself,
unbounded (unlike the API's 60/hr unauthenticated cap), with no token and no
JSON to parse. Comparison is semantic-version, ignoring a leading `v` and any
pre-release suffix (only stable releases light the dot).

**Surfacing.** Two red dots with deliberately different rules:

- _Settings About entry_ — lit while there is a newer version the user has not
  yet acknowledged; opening About acknowledges that version, so the main
  Settings surface goes quiet. (Settings is rarely visited, so even
  pre-acknowledgment this rarely intrudes.)
- _About page, beside the version_ — lit while _any_ newer version exists,
  regardless of acknowledgment, sitting next to the link the user taps to act.
  This is the persistent reminder; the Settings dot is just the passive cue that
  draws the user in.

**State.** A small, non-sensitive cache in the app data dir holds the last-known
latest version, the last-checked time, and the last-acknowledged version. It
lives outside the encrypted app config because it is neither secret nor user
preference.

**Failure mode.** Silent fail-closed: offline, unreachable, or unparseable → no
dot and no error. The app never claims an update exists when it can't confirm
one.

**Privacy.** Default-on, with a toggle on the About page. Because the probe runs
at most once per day and its result only matters when the user opens
Settings/About, default-on exposure is small; the toggle is the escape hatch for
users who want zero unsolicited network calls.

**Threat-model impact: minimal.** A network-level attacker could suppress the
redirect (hiding a real update) or forge a version string (lighting a phantom
dot) — but the only consequence is a misleading indicator or a link, and the
link lands on the real github.com release page that authenticates the release
itself. The app downloads and verifies no binary, so there is no code-execution
path and **no signing required** — that property is exactly what keeps this
feature out of the threat model's sensitive surface. Fail-closed means the app
never prods the user toward a false positive.

## Alternatives considered

- **In-app APK download + system install intent.** A one-tap upgrade would be
  slicker, but it adds APK fetch and disk handling, a file-provider config,
  "install unknown apps" friction attributed to gpm, and — for a password
  manager — the risk and optics of downloading and offering an executable.
  Rejected: link-out is exactly how sideload users already obtain updates, with
  none of that surface, and the detection layer is identical either way.
- **Lazy check on Settings-open.** Rejected as the trigger. The probe would
  start only when Settings opens, so a user who leaves within a second (and
  Settings is a simple, brief screen) sees no dot that visit; since Settings is
  rarely opened, they could miss an update for a long time. Cold-start gives the
  probe the entire launch-plus-navigation window to finish before the user can
  possibly reach Settings, removing the race.
- **Background-periodic check via the background-work scheduler (R077).**
  Rejected: reusing the periodic headless worker for a daily probe that lights a
  Settings-only dot is disproportionate to the value, and reintroduces headless
  foreground-skip/retry machinery for no gain. Cold-start plus a staleness cache
  achieves the same effective cadence far more cheaply.
- **GitHub API `releases/latest` JSON.** Rejected for this path: the API caps
  unauthenticated reads at 60/hr per IP and returns JSON to parse; the
  release-redirect target yields the version in the URL with neither cost.
- **Tauri updater plugin (desktop self-update).** Out of scope, not merely
  deferred-on-priority: desktop is not distributed (dev-only; CI ships Android
  APKs only), so there is no audience, and the updater demands a signing keypair
  to generate and guard forever, updater artifacts in CI, and an embedded
  pubkey/config — a large, separate effort warranted only if desktop ships. The
  detection layer here is shaped so a future desktop self-install could attach
  without rework.
- **Store the cache in the encrypted app config.** Rejected: the cached
  version/timestamp/ack is neither secret nor preference, so coupling it to the
  encrypted config adds cost without benefit; a plaintext cache in the app data
  dir is proportionate.
- **Global seen-state (dismiss everywhere once viewed) / no seen-state at all.**
  Both rejected for the dot rules: the user wanted the Settings entry to fall
  quiet once acknowledged while the About page keeps reminding until the update
  is actually installed — which is exactly the scoped, per-version
  acknowledgment adopted.

## Residual risks (what we accept)

- **Default-on phone-home.** The probe is on by default and reaches github.com
  on cold start (≤1/day), unauthenticated and payload-free — less sensitive than
  the authenticated git sync the app already performs, but a new default network
  call for purely-local users. Mitigated by the staleness cache, the toggle, and
  the fact that the result only matters when Settings/About is opened.
- **Stale dot on long sessions.** A session running >24h without a cold start
  won't refresh the dot until restart. On Android, process recycling makes cold
  starts frequent; a stale-refresh-on-Settings-open is a cheap future add-on,
  intentionally omitted from v1.
- **MITM can hide or phantom a dot.** Accepted: the only effect is a misleading
  indicator or link, and the link target authenticates the release. No signing,
  because the app downloads/verifies nothing.
- **Manual three-file version bump remains.** The current-version source is
  still the build-baked version, hand-synchronized across the three places it
  lives today; this RFC does not introduce build-time version injection (out of
  scope).

## Effort

~S (human) / ~S (CC). A small backend module (probe, redirect-parse, semver
compare, cache), one IPC surface to read status / toggle / acknowledge, frontend
additions to the About page and the Settings About entry (two dots, a toggle, a
link), and a plaintext cache file. No plugin, capability, permission, or CI
changes. Tests center on version comparison (newer/older/equal, leading `v`,
pre-release filtering), staleness logic, and redirect-target parsing, with the
network mocked.

## Depends on / Supersedes

- None. Deliberately does **not** depend on R077's background-work scheduler
  (see Alternatives); cold-start with a staleness cache replaces it for this use.
- No `docs/specs/NNN-*` PRD exists yet for this feature; this RFC carries the
  product context inline.
