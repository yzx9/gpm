<!--
Feature-level threat model for Android autofill. Complements docs/SECURITY.md
and 001/security.md (in-app access). Planned — not shipped. Living.
-->

# 008 — Android Autofill: threat model

## Assets & trust boundary

The decrypted secret, plus a new at-rest artifact: the learned mapping from
app / website to entry. Autofill adds an OS-registered entry point invocable
from any app, on top of 001's in-app access paths (copy / show).

## Net effect on the existing model

Autofill does **not** change at-rest encryption or the per-op identity lifecycle
— an autofill request is simply another operation that triggers the same
biometric-gated unlock as copy/show, with no cached bypass. The credential goes
straight into the target app's focused field and never reaches the clipboard, so
vs. copy it is a net security improvement.

## New surfaces specific to autofill

- **View-tree snapshot.** The OS hands the service a snapshot of the focused
  screen's view tree, which may include other visible fields and text. A fill
  request must never persist or log that snapshot — it is inspected only to
  locate the username/password fields and then discarded.
- **Learned association mapping.** Matching an entry to a target needs to join on
  the app's identity (or, for in-browser fields, the website). The learned
  `app/site → entry` mapping is a new at-rest artifact and must receive the same
  at-rest encryption as the config and identity, so a reader learns nothing about
  which entries exist.
- **Push-based, not scanning.** The service never polls or reads the screen on its
  own initiative — it sees only the focused screen the OS chooses to share — so
  its read surface is the on-demand snapshot, not every app's full content.

## Cross-references

- In-app access paths (copy / show): `001-entry-access/security.md`.
- System-wide model: `docs/SECURITY.md`.
