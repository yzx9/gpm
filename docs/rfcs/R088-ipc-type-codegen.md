# Single-source IPC types (ts-rs codegen)

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

Generate the frontend ↔ Rust IPC TypeScript types from the Rust definitions
with `ts-rs`, so there is one source of truth for the wire shape instead of the
hand-written interfaces in `app/src/api/*.ts` that today mirror the Rust structs
by hand (kept in sync by docstring cross-references and a Rust-side wire-shape
test).

No feature spec — this is a tooling/quality RFC, not a product feature. It was
identified while completing the R069 attribute-region work, which added a new
`attributes` field to two IPC types and a structured edit payload, and chose to
hand-write them to match the project's existing convention.

## Why

Today every IPC type exists twice — once in Rust (the `Serialize`/`Deserialize`
struct behind a `#[tauri::command]`) and once in TypeScript (the `interface`
the frontend imports from `@/api`). They are kept in sync by discipline: a
docstring on the TS interface points at the Rust type, and one Rust test pins a
representative wire shape. That holds for the current ~handful of types, but a
field added on one side and forgotten on the other would compile and run until
it silently mismatched at runtime — exactly the class of bug a single source
prevents.

`ts-rs` would make Rust the source: `#[derive(TS)]` on the IPC structs emits
`.ts` files the frontend imports, so the shapes can't drift. This RFC records
that option and **why it is deferred**, so the analysis isn't re-derived next
time someone adds an IPC field.

## Why deferred (decided during R069)

- **No existing ts-rs to mirror.** `ts-rs` is not used anywhere in gpm — not in
  the app crate, not in the local plugins, not in `Cargo.lock`. Introducing it
  is a from-scratch tooling decision (dependency + `#[derive(TS)]` + an export
  step + committed generated `.ts` + a freshness gate), not "extend the existing
  setup." An earlier plan draft assumed (wrongly) that the plugins already used
  ts-rs and it could be mirrored — that premise was corrected during R069
  planning (R083 is a GPG recipient/keyring RFC, unrelated to codegen).
- **Cost vs. current benefit.** For the current small set of IPC types, the
  drift risk is low and has not caused a bug; the hand-written types are simple
  and stable. The ts-rs setup cost (and its sharp edges, below) is not justified
  yet.

## Gotchas to handle if/when adopted

Captured here so a future implementer doesn't rediscover them:

- **`u64`/`usize` → `bigint`**: ts-rs maps 64-bit unsigned integers to TS
  `bigint`, not `number`. Pin such fields with `#[ts(type = "number")]` (or use
  a `number`-typed alias) so the frontend doesn't have to deal with `bigint`.
- **Export API**: `ts_rs::TS::export_to_string(&Config)` for in-memory use, or
  `export!` to write files — pick one and be consistent.
- **`missing_docs`**: the workspace lints `missing_docs`; ts-rs surfaces
  doc-comment text into the generated TS, so pub-reachable enum variants and
  fields need docs or the build warns.
- **Freshness gate in TWO places**: a stale-generated-TS check must run in BOTH
  `justfile`'s `lint` recipe AND `.github/workflows/lint.yml`, or CI passes
  while local lint (or vice-versa) lets drift slip.
- **Publishable plugins**: if a plugin ever adopts ts-rs, feature-gate the
  dependency so the published crate doesn't force it on consumers.

## When to revisit

Re-open this RFC when the IPC type set grows substantially, or when a
Rust↔TS shape drift actually causes a shipped bug — i.e. when the single-source
benefit clears the from-scratch setup bar. Until then, hand-written types
remain the convention.
