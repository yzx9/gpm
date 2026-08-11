// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { ClipboardNotifyText } from "@/i18n/native";
import type { AuthenticityResult } from "./common";
import type { PullResult, SyncDivergence } from "./repo";

/**
 * Secret read/create/edit IPC — folds together the backend `read`, `clipboard`,
 * `generator`, and secret-write half of `write` modules. All decrypted content
 * is {@link SensitiveContent} (password + notes); the backend auto-clears
 * clipboard/view timers.
 */

/** A secret entry: its `.age` path and the display name (`.age` stripped). */
export interface Entry {
  path: string;
  name: string;
}

/** One page of entries from the paginated list/search commands. */
export interface EntryPage {
  entries: Entry[];
  /** Total entries matching the query, independent of this page's slice. */
  total: number;
  /** `true` when more pages remain past this slice. */
  has_more: boolean;
}

/** Result of `copy_password`: clipboard armed with an auto-clear timer. */
export interface CopyResult {
  success: boolean;
  entry_name: string;
  cleared_after_secs: number;
  /** Free byproduct of the decrypt: whether the entry's body carries a TOTP
   *  seed, so the UI can show/hide the 2FA button without a second read. */
  has_totp: boolean;
  /** Free byproduct of the decrypt: whether the entry is a binary attachment. */
  has_attachment: boolean;
  /** Free byproduct of the decrypt: the password isn't valid UTF-8, so the
   *  backend skipped the clipboard write (copy it with the gopass CLI). */
  password_non_utf8: boolean;
  /** Free byproduct of the decrypt: the password is empty (a bare legacy-YAML
   *  document), so the backend skipped the clipboard write — there is nothing
   *  to copy. */
  password_empty: boolean;
}

/** Result of `copy_totp`: `copied` is `false` when the entry has no TOTP seed
 *  (no clipboard write happened). Neither the seed nor the code crosses IPC. */
export interface TotpCopyResult {
  copied: boolean;
  entry_name: string;
  cleared_after_secs: number;
}

/** Metadata for a binary attachment (non-secret): filename + decoded size. */
export interface AttachmentMeta {
  filename: string | null;
  size: number;
}

/** Why an entry's Edit affordance is disabled: `"nonUtf8"` — the secret holds
 *  non-UTF-8 bytes a text editor can't round-trip without corrupting;
 *  `"legacyYaml"` — a legacy gopass YAML secret (a `---` line), which gpm
 *  shows read-only rather than corrupt on write-back. */
export type EditBlockReason = "nonUtf8" | "legacyYaml";

/** One `Key: Value` attribute (gopass AKV) — mirrors `rustpass::Attribute` on the
 *  wire. Both halves are decrypted content. */
export interface AttributeView {
  key: string;
  value: string;
}

/** Structured edit/resolve input (R069 2b): the password, attribute region, and
 *  free-text body as separate parts. Rust reassembles the on-disk plaintext via
 *  `Secret::from_parts` → `to_bytes`, so the frontend sends parts (not a
 *  pre-joined string). */
export interface SecretParts {
  password: string;
  attributes: AttributeView[];
  body: string;
}

/** Decrypted secret content (password first line, attributes the `Key: Value`
 *  region, notes the free-text rest). */
export interface SensitiveContent {
  password: string;
  notes: string;
  /** The parsed `Key: Value` attribute region (gopass AKV) for named-field
   *  display + structured edit. Empty for attachments. */
  attributes: AttributeView[];
  /** Free byproduct of the decrypt: whether the entry's body carries a TOTP
   *  seed, so the UI can show/hide the 2FA button without a second read. */
  has_totp: boolean;
  /** When set, the entry is a binary attachment: `notes` is empty (the base64
   *  body never crosses IPC) and the UI shows Export + this metadata instead. */
  attachment: AttachmentMeta | null;
  /** When set, the entry can't be safely text-edited; the UI disables Edit. */
  edit_blocked: EditBlockReason | null;
  /** The blob oid (base version) captured atomically with this decrypt — the
   *  R026 base-version the edit screen sends back as `baseOid` to guard a save
   *  against a stale snapshot. Non-secret; `null` only when not captured. */
  version: string | null;
}

/** One-shot entry probe: one decrypt returns both the 2FA-presence signal and
 *  attachment metadata so the detail view settles both affordances from one
 *  read. `null` when the identity is not cached (the probe never prompts). */
export interface EntryProbe {
  has_totp: boolean;
  attachment: AttachmentMeta | null;
  /** When set, the entry can't be safely text-edited; the detail view greys
   *  Edit (mirrors the attachment case). */
  edit_blocked: EditBlockReason | null;
}

/** Result of `export_attachment`. `exported` is `false` when the entry holds no
 *  modern attachment (nothing staged, no dialog). No secret data. */
export interface AttachmentExportResult {
  exported: boolean;
  entry_name: string;
}

/** One input field of a create preset (mirrors `rustpass::template::PresetField`). */
export interface PresetField {
  key: string;
  label: string;
  required: boolean;
  /** gopass field `type`: `"password"` (generatable + masked), `"hostname"`, `"string"`, `"multiline"`. */
  type: string;
  /** gopass per-attribute `charset`; locks generation when set on a `"password"` field (e.g. `"0123456789"` for a PIN). */
  charset: string | null;
  /** gopass `min` length bound for a generated value. */
  min: number | null;
  /** gopass `max` length bound for a generated value. */
  max: number | null;
  /** gopass `strict`: require every character class present in the alphabet. */
  strict: boolean;
}

/** Password generator method (mirrors `rustpass::GenerateMode`, lowercase). */
export type GenerateMode = "random" | "memorable" | "xkcd";

/** A built-in secret-creation preset (mirrors `rustpass::template::CreatePreset`). */
export interface CreatePreset {
  id: string;
  label: string;
  /** Directory prefix the secret is generated under (e.g. "websites"). */
  prefix: string;
  /** Field keys whose values join to form the secret's name under `prefix`. */
  name_from: string[];
  fields: PresetField[];
}

/** A successful write — short hash of the commit that recorded it. */
export interface WriteResult {
  commit: string;
}

/** Whether a base-version-aware write is an edit, delete, or create (serde
 *  snake_case). Create is existence-based (a name a teammate took first). */
export type EntryConflictOp = "edit" | "delete" | "create";

/** How to resolve an `entry_conflict` outcome (serde snake_case). "cancel" is
 *  client-side (the frontend dismisses the modal), so it is absent here. */
export type EntryConflictChoice = "keep_mine" | "keep_theirs";

/** Outcome of a create/edit/delete (serde tagged by `kind`, snake_case). A
 *  normal save is `written`; `needs_divergence_resolve` means the push was
 *  rejected (a race with a newer remote) and the carried {@link SyncDivergence}
 *  lets the UI show the resolve modal without a second round-trip;
 *  `authenticity_blocked` means the pre-write pull was refused under Enforce;
 *  `entry_conflict` means a base-version-aware edit/delete/create collided with a
 *  newer remote (R026) — edit/delete when a teammate changed it, create when a
 *  teammate took the name first — and surfaces the per-entry resolve modal;
 *  `no_change` means a delete found the entry already removed (delete-only) and
 *  toasts "already removed". */
export type WriteOutcome =
  | ({ kind: "written" } & WriteResult)
  | ({ kind: "needs_divergence_resolve" } & SyncDivergence)
  | ({ kind: "authenticity_blocked" } & AuthenticityResult)
  | { kind: "cancelled"; committed: boolean }
  | {
      kind: "entry_conflict";
      name: string;
      base_oid: string;
      current_oid: string | null;
      remote_tip: string;
      /** "edit" | "delete" | "create" — named `op` because the serde tag is already `kind`. */
      op: EntryConflictOp;
    }
  | { kind: "no_change"; head: string };

/** List one page of entries (no query). */
export async function listEntries(
  repoId: string,
  offset: number,
  limit: number,
): Promise<EntryPage> {
  return invoke<EntryPage>("list_entries", { repoId, offset, limit });
}

/** Search entries by query; returns one page of matches. */
export async function searchEntries(
  repoId: string,
  query: string,
  offset: number,
  limit: number,
): Promise<EntryPage> {
  return invoke<EntryPage>("search_entries", { repoId, query, offset, limit });
}

/** Decrypt + copy the entry's password; clipboard auto-clears after a timer.
 *  `notify` supplies the localized notification text; when absent the
 *  native layer falls back to a generic safety string. */
export async function copyPassword(
  repoId: string,
  entryPath: string,
  notify?: ClipboardNotifyText,
): Promise<CopyResult> {
  return invoke<CopyResult>("copy_password", {
    repoId,
    entryPath,
    notifyText: notify,
  });
}

/** Decrypt + compute the entry's TOTP code in Rust + copy it; the clipboard
 *  auto-clears after a timer. `copied` is `false` when the entry has no TOTP
 *  seed. Neither the seed nor the code reaches the WebView. */
export async function copyTotp(
  repoId: string,
  entryPath: string,
  notify?: ClipboardNotifyText,
): Promise<TotpCopyResult> {
  return invoke<TotpCopyResult>("copy_totp", {
    repoId,
    entryPath,
    notifyText: notify,
  });
}

/** One-shot **cache-only** probe (never triggers an unlock): returns both the
 *  2FA-presence signal and attachment metadata from a single decrypt, or `null`
 *  when the identity is not currently cached. Never returns secret data. */
export async function entryProbe(
  repoId: string,
  entryPath: string,
): Promise<EntryProbe | null> {
  return invoke<EntryProbe | null>("entry_probe", { repoId, entryPath });
}

/** Detect a binary attachment and export its decoded bytes to a user-chosen
 *  file. `exported` is `false` when the entry has no attachment. Rejects with
 *  `CANCELLED` (dismissed picker), `REPO_BUSY` (another export in progress), or
 *  a real error. Decoded bytes never reach the WebView. */
export async function exportAttachment(
  repoId: string,
  entryPath: string,
): Promise<AttachmentExportResult> {
  return invoke<AttachmentExportResult>("export_attachment", {
    repoId,
    entryPath,
  });
}

/** Decrypt + return the entry's content for in-app reveal. */
export async function showPassword(
  repoId: string,
  entryPath: string,
): Promise<SensitiveContent> {
  return invoke<SensitiveContent>("show_password", { repoId, entryPath });
}

/** Blob oid (base version) of `entry` at HEAD, or `null` if absent — the R026
 *  base-version capture for a base-version-aware delete (fetched on the detail
 *  page mount so a delete-without-reveal is still protected). Non-secret. */
export async function entryOid(
  repoId: string,
  entryPath: string,
): Promise<string | null> {
  return invoke<string | null>("entry_oid", { repoId, entryPath });
}

/** Why an entry-cache event fired — the cause of the transition. Mirrors the
 *  backend `EntryCacheReason` enum (R086). */
export type EntryCacheReason = "warmed" | "timer" | "lock" | "leave";

/** Payload of an `entry-cache-warmed` / `entry-cache-wiped` event: the backend's
 *  entry-view cache snapshot. `cached` is true on warm, false on wipe; `reason`
 *  is the transition cause (see {@link EntryCacheReason}). */
export interface EntryCacheState {
  cached: boolean;
  reason: EntryCacheReason;
}

/** Wipe the entry-view cache (R086). The frontend calls this on leave/switch so
 *  the just-left entry's decrypted content does not linger in backend memory.
 *  Idempotent — a no-op if nothing is cached. */
export async function wipeEntryCache(entryPath: string): Promise<void> {
  await invoke("wipe_entry_cache", { entryPath });
}

/** Subscribe to `entry-cache-warmed` (a miss just populated the cache). Returns
 *  an unlisten handle. The backend is the source of truth; warm transitions fire
 *  here so the frontend can mirror cache state from both sides (R086 D9). */
export async function subscribeEntryCacheWarmed(
  cb: (e: EntryCacheState) => void,
): Promise<UnlistenFn> {
  return listen<EntryCacheState>("entry-cache-warmed", (e) => cb(e.payload));
}

/** Subscribe to `entry-cache-wiped` (timer / lock / leave emptied the cache).
 *  Returns an unlisten handle. Mirrors {@link subscribeEntryCacheWarmed}. */
export async function subscribeEntryCacheWiped(
  cb: (e: EntryCacheState) => void,
): Promise<UnlistenFn> {
  return listen<EntryCacheState>("entry-cache-wiped", (e) => cb(e.payload));
}

/** Copy an already-generated password string; clipboard auto-clears after 30s. */
export async function copyGeneratedPassword(
  text: string,
  notify?: ClipboardNotifyText,
): Promise<void> {
  await invoke("copy_generated_password", { text, notifyText: notify });
}

/** Generate one password. The arg object is passed through verbatim. */
export async function generatePassword(opts: {
  mode: GenerateMode;
  charset: string | null;
  minLen: number | null;
  maxLen: number | null;
  strict: boolean;
}): Promise<string> {
  return invoke<string>("generate_password", opts);
}

/** Generate a batch of `count` passwords. The arg object is passed through verbatim. */
export async function generatePasswordBatch(opts: {
  mode: GenerateMode;
  charset: string | null;
  minLen: number | null;
  maxLen: number | null;
  strict: boolean;
  count: number;
}): Promise<string[]> {
  return invoke<string[]>("generate_password_batch", opts);
}

/** List the built-in create presets. */
export async function listCreatePresets(): Promise<CreatePreset[]> {
  return invoke<CreatePreset[]>("list_create_presets");
}

/** Whether a gopass location-based template exists for `name`. */
export async function lookupTemplate(
  repoId: string,
  name: string,
): Promise<string | null> {
  return invoke<string | null>("lookup_template", { repoId, name });
}

/** Preview the rendered body of a custom secret (template-expanded). */
export async function previewCreate(
  repoId: string,
  name: string,
  content: string,
): Promise<string | null> {
  return invoke<string | null>("preview_create", { repoId, name, content });
}

/** Create a secret from a preset; returns the write outcome. */
export async function createFromPresetSecret(
  repoId: string,
  presetId: string,
  fields: Record<string, string>,
): Promise<WriteOutcome> {
  return invoke<WriteOutcome>("create_from_preset_secret", {
    repoId,
    presetId,
    fields,
  });
}

/** Create a custom secret; returns the write outcome. `entry_conflict` (R026) is
 *  returned when a teammate already created the same name — the per-entry resolve
 *  modal lets the user overwrite or keep theirs. */
export async function createSecret(
  repoId: string,
  name: string,
  content: string,
): Promise<WriteOutcome> {
  return invoke<WriteOutcome>("create_secret", { repoId, name, content });
}

/** Edit an existing secret; returns the write outcome. `baseOid` (the blob oid
 *  captured at read time) enables the R026 base-version guard — when set, a
 *  stale edit surfaces `entry_conflict` instead of silently clobbering. */
export async function editSecret(
  repoId: string,
  name: string,
  parts: SecretParts,
  baseOid?: string | null,
): Promise<WriteOutcome> {
  return invoke<WriteOutcome>("edit_secret", {
    repoId,
    name,
    parts,
    ...(baseOid != null && { baseOid }),
  });
}

/** Delete a secret; returns the write outcome (usually `written`,
 *  `needs_divergence_resolve` when the delete's push lost a race, `entry_conflict`
 *  when a teammate changed it since the read, or `no_change` when a teammate
 *  already removed it). `baseOid` enables the R026 base-version guard. */
export async function deleteSecret(
  repoId: string,
  name: string,
  baseOid?: string | null,
): Promise<WriteOutcome> {
  return invoke<WriteOutcome>("delete_secret", {
    repoId,
    name,
    ...(baseOid != null && { baseOid }),
  });
}

/** Resolve a per-entry edit/delete conflict (R026 `entry_conflict`) per `choice`
 *  against the reviewed remote tip. `keep_mine` (edit) is identity-gated
 *  backend-side (re-encrypts the caller's `parts`); `keep_mine` (delete) and
 *  `keep_theirs` need no identity. Returns the post-resolve result. */
export async function resolveEntryConflict(
  repoId: string,
  name: string,
  parts: SecretParts | null,
  expectedRemoteOid: string,
  op: EntryConflictOp,
  choice: EntryConflictChoice,
): Promise<PullResult> {
  return invoke<PullResult>("resolve_entry_conflict", {
    repoId,
    name,
    parts,
    expectedRemoteOid,
    op,
    choice,
  });
}
