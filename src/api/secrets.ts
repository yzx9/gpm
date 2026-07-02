// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import type { ConflictChoice, WriteOutcome } from "./common";

/**
 * Secret read/create/edit IPC — folds together the backend `read`, `clipboard`,
 * `generator`, and secret-write half of `write` modules, plus the conflict arm
 * of the write path. All decrypted content is {@link SensitiveContent}
 * (password + notes); the backend auto-clears clipboard/view timers.
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
}

/** Decrypted secret content (password first line, notes the rest). */
export interface SensitiveContent {
  password: string;
  notes: string;
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

/** A write-path conflict on a same-name remote entry. Carries NO plaintext. */
export interface WriteConflict {
  name: string;
  /** Whether the remote version decrypts with our key. */
  remote_decryptable: boolean;
}

/** List one page of entries (no query). */
export async function listEntries(
  offset: number,
  limit: number,
): Promise<EntryPage> {
  return invoke<EntryPage>("list_entries", { offset, limit });
}

/** Search entries by query; returns one page of matches. */
export async function searchEntries(
  query: string,
  offset: number,
  limit: number,
): Promise<EntryPage> {
  return invoke<EntryPage>("search_entries", { query, offset, limit });
}

/** Decrypt + copy the entry's password; clipboard auto-clears after a timer. */
export async function copyPassword(entryPath: string): Promise<CopyResult> {
  return invoke<CopyResult>("copy_password", { entryPath });
}

/** Decrypt + return the entry's content for in-app reveal. */
export async function showPassword(
  entryPath: string,
): Promise<SensitiveContent> {
  return invoke<SensitiveContent>("show_password", { entryPath });
}

/** Decrypt + return the REMOTE copy of a conflicted entry (origin tip); null if unavailable. */
export async function showRemoteSecret(
  name: string,
): Promise<SensitiveContent | null> {
  return invoke<SensitiveContent | null>("show_remote_secret", { name });
}

/** Copy an already-generated password string; clipboard auto-clears after 30s. */
export async function copyGeneratedPassword(text: string): Promise<void> {
  await invoke("copy_generated_password", { text });
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
export async function lookupTemplate(name: string): Promise<string | null> {
  return invoke<string | null>("lookup_template", { name });
}

/** Preview the rendered body of a custom secret (template-expanded). */
export async function previewCreate(
  name: string,
  content: string,
): Promise<string | null> {
  return invoke<string | null>("preview_create", { name, content });
}

/** Create a secret from a preset; returns the write outcome (written or conflict). */
export async function createFromPresetSecret(
  presetId: string,
  fields: Record<string, string>,
): Promise<WriteOutcome> {
  return invoke<WriteOutcome>("create_from_preset_secret", {
    presetId,
    fields,
  });
}

/** Create a custom secret; returns the write outcome (written or conflict). */
export async function createSecret(
  name: string,
  content: string,
): Promise<WriteOutcome> {
  return invoke<WriteOutcome>("create_secret", { name, content });
}

/** Edit an existing secret; returns the write outcome (written or conflict). */
export async function editSecret(
  name: string,
  content: string,
): Promise<WriteOutcome> {
  return invoke<WriteOutcome>("edit_secret", { name, content });
}

/** Delete a secret; returns the recording commit hash. */
export async function deleteSecret(name: string): Promise<WriteResult> {
  return invoke<WriteResult>("delete_secret", { name });
}

/** Resolve a write conflict per `choice`; returns the commit if one was made. */
export async function resolveWriteConflict(
  choice: ConflictChoice,
): Promise<WriteResult | null> {
  return invoke<WriteResult | null>("resolve_write_conflict", { choice });
}
