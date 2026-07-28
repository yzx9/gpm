<!--
Product personas for gpm. Feature PRDs (docs/specs/) note how these people act
*in that feature*, not who they are — that lives here. Living doc.
-->

# Personas

gpm's audience, in priority order. Each feature PRD's Use-Cases section describes
how a persona behaves _within that feature_ (and what they adapt to); it does not
re-define them — that's this file's job.

## Jordan — Primary · self-hosting gopass power user

**Who.** Early-30s backend / platform engineer at a mid-sized company, running a
homelab — a self-hosted Forgejo instance, a few self-deployed services, an
always-on mini server. Works on a Linux desktop + macOS laptop, lives in the
terminal, keeps a YubiKey on their keychain, and manages dotfiles in git with
their age identities and a pile of SSH keys inside.

**Relationship to passwords.** Migrated off 1Password years ago — refuses to keep
the vault on someone else's server. Uses gopass with an age-encrypted repo
self-hosted on their Forgejo. `gopass show` / `gopass insert` is the daily
interface. A few directories shared with their partner are multi-recipient
encrypted; work credentials live under a separate identity.

**What they believe.** A near-principled preference for "my data on my own server";
refuses any third-party cloud-hosted vault. Trusts things they can audit — SSH
keys, age, hardware keys — and distrusts closed-source cloud. Reads the threat
model; turns commit verification up to Enforce themselves.

**Why gpm.** When they're out with only their phone, they occasionally need a
password — a server root password, a token for some self-hosted service, the
account at the domain registrar. They don't want a second vault on the phone; they
want secure access to the same gopass repo. Cares about: commit-signature
authenticity, multi-identity routing (work / personal / partner), YubiKey-class
hardware-key decryption.

**Technical comfort.** Very high — SSH key generation, age identities, git,
YubiKey, GPG are all second nature; they'll seek out power features.

## Casey — Secondary · mobile-first newcomer

**Who.** Mid-20s UI designer at a startup. Phone is the primary device; the laptop
is mostly for Figma and office apps. Has been reusing the same three or four
passwords for years — panics briefly after a breach, changes two, slips back. Has
heard of password managers but bounced off 1Password (subscription) and Bitwarden
(cloud account).

**Relationship to passwords.** No concept of gopass, age, or git, and no interest
in learning. "Clone a repo" is a meaningless phrase to them. What they want is
simple: open the app → create a local vault → enter a few accounts → unlock with a
fingerprint and copy passwords from now on. Offline, no cloud, free is best.

**Why gpm.** Probably recommended by a technical friend (someone like Jordan), or
found while looking for a "local, no-cloud" option. To them gpm is "a
fingerprint-unlocked offline password book." Multi-identity, commit signatures,
recipients pinning — they'll never touch these, and the app must keep them out of
the way of "enter an account."

**Future.** If they ever move into development, gpm's gopass-compat lets this vault
"graduate" to desktop gopass — but that's later.

**Technical comfort.** Low — power features are a burden, not a capability;
defaults must be secure and work out of the box.

## Not for — Anti-personas

gpm is deliberately not built for these people. Naming them keeps scope honest:
when a request only serves one of them, that's a signal to say no, and to point
them at the right tool instead.

### Morgan — wants hosted, zero-ops sync

Wants the 1Password / Bitwarden experience: make an account, everything syncs
across devices automatically, never think about servers, keys, or repos. They
value "it just works" and convenience above sovereignty over their data. gpm will
disappoint them at every turn — there's no hosted backend, "sync" means
configuring a git remote, and recovery assumes you understand your own setup.

**Right tool:** Bitwarden or 1Password. gpm isn't trying to win them.

### Avery — wants enterprise team administration

Wants centralized administration for an organization: SSO, role-based access,
directory provisioning, audit logs, compliance certifications (SOC 2), shared-vault
management at scale. They're evaluating a credential solution _for a company_. gpm
is a personal / small-shared-repo tool — a git repo with optional commit signing,
not enterprise IAM — and will never offer the policy, reporting, or
identity-integration surface they need.

**Right tool:** an enterprise password manager or secrets platform. The scope line
is "a few people who explicitly trust each other sharing one repo" (that's Jordan)
versus "centralized control of many users" (that's Avery) — the second is out.
