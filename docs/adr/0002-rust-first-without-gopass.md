# ADR 0001: Rust-First Architecture Without gopass Integration

**Status:** Accepted

**Date:** 2026-06-08

**Context**

gopass PW published PR [#3438](https://github.com/gopasspw/gopass/pull/3438), establishing a "best-effort stable" API guarantee for `pkg/gopass`. This raised the question: should gpm use gopass as its backend instead of the current custom Rust implementation?

The appeal is clear — gopass provides a complete password store implementation (age + GPG encryption, git sync, recipients management, CRUD operations) with a now-stable Go API. Using it as a backend would unlock read-write operations and GPG support without writing that logic ourselves.

## Integration Paths Evaluated

### 1. gopass as Go library (via CGo/FFI)

**Rejected.** gopass is a Go library. gpm is a Rust/Tauri application. You cannot import a Go package from Rust. The only way to call Go from Rust is through C FFI (`go build -buildmode=c-shared`), which requires:

- Cross-compiling gopass as a `.so` for Android ARM64 with full NDK toolchain
- Writing a C ABI wrapper layer that flattens Go interfaces (`secrets.Secret`) into C structs
- Each operation (list, get, set, delete, move) needs a hand-written bridge function
- Go runtime initializes in the same process, potentially conflicting with tokio and JVM signal handlers

Security: Go strings are immutable and managed by GC. `C.CString()` allocates on the C heap (freeable), but the original Go `string` remains on the Go heap until GC collects it. **Rust's `Zeroizing<String>` cannot cover Go-side memory.** Go's GC provides no zeroize guarantees.

### 2. gopass binary as subprocess (via gopass-jsonapi)

**Rejected.** Ship a gopass binary (~20-30 MB) in the APK, communicate via gopass-jsonapi's JSON-over-stdin/stdout protocol.

Feasible technically:

- Go cross-compiles to Android ARM64 easily (`GOOS=android GOARCH=arm64`)
- gopass-jsonapi is a proven IPC mechanism (used by browser extensions)
- Tauri supports sidecar binaries via `externalBin`

But introduces:

- Password data flows through IPC pipe (stdin → Go process → stdout), existing in Go process memory without zeroize
- Subprocess lifecycle management on Android
- Go binary version management and update pipeline
- +20-30 MB APK size increase

### 3. Hybrid: Rust for age, gopass subprocess for GPG only

**Rejected.** Most complex option — two code paths for crypto operations, two testing strategies, unified error handling across language boundaries. Low return on complexity investment.

### 4. Stay in Rust, expand current implementation

**Accepted.** Current Rust backend is 1,537 lines. Adding read-write operations (create, edit, delete, move) requires approximately 300-400 additional lines of Rust code, leveraging the existing `age` crate (which has `Encryptor` for encryption) and `git2` crate (which has commit/push operations).

## Security Model Comparison

| Aspect                   | gpm (Rust)                         | gopass (Go)                          |
| ------------------------ | ---------------------------------- | ------------------------------------ |
| Password memory clearing | `Zeroizing<String>` on Drop        | None — relies on Go GC               |
| Identity key clearing    | Zeroized per-decrypt call          | Not implemented                      |
| Threat model             | Includes local attacker mitigation | Explicitly excludes local attackers¹ |
| Auditability             | All code in repository             | External dependency (black box)      |

¹ gopass `docs/security.md`: "The threat model of gopass assumes there are no attackers on your local machine."

**Nuance on local attacker threat:**

- On non-rooted Android (gpm's primary target), app sandboxing makes local memory attacks difficult. The practical risk difference between Rust zeroize and Go GC is modest in this context.
- On desktop (secondary target), same-user processes can read memory via `ptrace`/`/proc/pid/mem`. Zeroize provides real additional value here.
- gpm itself already has trust boundaries where zeroize cannot apply (WebView JS heap for `show_password`, JVM heap for clipboard plugin). Adding Go GC to this list is same-category, not a new class of problem.

The security argument alone does not make gopass integration unsafe for Android. The real cost is **loss of full auditability** — gpm goes from "every line is ours" to "partially depends on an external binary we didn't write."

## Decision

**Remain Rust-first. Do not introduce gopass as a dependency.**

Rationale:

1. **Complexity cost outweighs benefits for a personal project.** gopass integration adds cross-language IPC, Go cross-compilation CI, binary lifecycle management, and a C ABI bridge layer — all to avoid writing ~400 lines of Rust.
2. **Full auditability is a genuine differentiator.** gpm's "trust IS the product" positioning depends on users being able to audit the entire codebase. Introducing a Go binary black box weakens this story.
3. **The Rust implementation is small and maintainable.** 1,537 lines is well within the range where extending in-place is cheaper than integrating an external system.
4. **GPG support is not required for the target audience.** gpm targets age-only gopass users — a growing segment. GPG support is explicitly deferred.

## Consequences

- **Read-write operations will be implemented in Rust.** New commands: `create_entry`, `edit_entry`, `delete_entry`, `move_entry`. Uses existing `age` crate for encryption and `git2` for commit/push.
- **GPG support remains out of scope.** The `gpgme` crate's Android NDK cross-compilation is a known pain point. If GPG demand materializes, this ADR should be revisited.
- **No new build dependencies.** No Go toolchain, no cross-compilation pipeline, no binary management.
- **Trait abstraction recommended.** Define a `PasswordStore` trait that the current implementation satisfies. This preserves the option to introduce an alternative backend (including gopass) in the future without changing frontend code.
