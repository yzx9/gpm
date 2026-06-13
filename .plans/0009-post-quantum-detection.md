# Recognize age post-quantum (X-Wing) identities & recipients

**Priority:** P2
**Status:** TODO
**Phase:** Near-term

## What

Detect age's post-quantum key formats (`age1pq1...` recipients, `AGE-SECRET-KEY-PQ-1...` identities) and surface a clear "not yet supported" error. No decryption is implemented in this phase — only recognition and honest failure.

## Why

- gopass `master` depends on `filippo.io/age v1.3.1`, which has **native post-quantum support**. A gopass repo's `.age-recipients` can now contain `age1pq1...`, and the resulting secrets are post-quantum-encrypted.
- gpm depends on the Rust `age` crate `0.11.3`, which has **no PQ capability** and cannot decrypt such files.
- Worse, gpm currently **mis-classifies** these silently: `age1pq1...` starts with `age1` and is treated as X25519; `AGE-SECRET-KEY-PQ-1...` starts with `AGE-SECRET-KEY-` and is handed to the underlying age parser, producing a confusing parse error. The user sees an opaque failure instead of an honest explanation.

## Significance / benefits

- **Honest failure.** Turns a confusing parse error into a clear, actionable "post-quantum keys aren't supported yet" message — distinct from "invalid key format."
- **Correct classification.** Recipient lists no longer disguise PQ keys as X25519; the UI can label them truthfully.
- **Foundation for the future.** Establishes the recognize → distinguish → degrade-gracefully skeleton, so real PQ decryption (see 0010) plugs in with minimal surface area later.
- **Zero crypto risk.** String-prefix recognition only — no cryptographic primitives are touched, no security audit needed.

## Difficulties

- Essentially none. The only subtlety is **prefix-match ordering**: more specific prefixes (`AGE-SECRET-KEY-PQ-`, `age1pq1`) must be checked before the generic ones (`AGE-SECRET-KEY-`, `age1`), or they get swallowed.
- Consistency: a new enum variant must be mirrored across the IPC serialization layer and the frontend type mirrors — but the compiler enforces this, which is a feature.

## Blockers

- **None.** Purely local change, no upstream dependency. Can be done now.

## Rough approach (not implementation-detail)

- Distinguish PQ prefixes in the identity/recipient classification logic.
- Introduce a "recognized-but-unsupported" error semantics, separate from "invalid format."
- Intercept PQ identities at the decryption entry point, before the underlying age call.
- Render the error as a human sentence in the frontend (e.g., "Post-quantum keys aren't supported yet — tracking 0010").

## Verification

- Feed the real example strings from the C2SP spec (`age1pq1...`, `AGE-SECRET-KEY-PQ-1...`) through the classify / validate / decrypt paths; confirm the result is "unsupported," not a parse failure and not mis-classified as X25519.
- Regression: existing x25519 / SSH paths are unaffected.

## References

- age PQ spec: https://c2sp.org/age (section "The MLKEM768-X25519 (i.e. X-Wing) hybrid post-quantum recipient type")
- Follow-up roadmap: [0010-post-quantum-roadmap.md](0010-post-quantum-roadmap.md)
