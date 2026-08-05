#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
#
# SPDX-License-Identifier: Apache-2.0
#
# Orchestrate a release: validate the new version, bump it across project files,
# commit, tag, then optionally push to origin.
#
# The new version is validated against two rules, either overridable by an
# interactive confirmation prompt:
#   1. it must be A.B.C (numeric semver — no pre-release or build suffixes)
#   2. it must be strictly greater than the current version
#
# After committing and tagging, the script asks whether to push; answering Y
# runs `git push` followed by `git push --tags`.
#
# Usage: scripts/release.sh <new-version>
# Example: scripts/release.sh 1.0.0

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $(basename "$0") <new-version>" >&2
  echo "  e.g. $(basename "$0") 1.0.0" >&2
  exit 1
fi

NEW_VERSION="$1"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CURRENT_VERSION=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$ROOT/app/src-tauri/tauri.conf.json")
if [[ -z "$CURRENT_VERSION" ]]; then
  echo "error: could not read current version from app/src-tauri/tauri.conf.json" >&2
  exit 1
fi

# --- Validation ---------------------------------------------------------------
# Return 0 if $1 is strictly greater than $2 (both expected A.B.C).
ver_gt() {
  local a b i
  IFS='.' read -ra a <<< "$1"
  IFS='.' read -ra b <<< "$2"
  for i in 0 1 2; do
    if (( ${a[i]} > ${b[i]} )); then return 0; fi
    if (( ${a[i]} < ${b[i]} )); then return 1; fi
  done
  return 1
}

problems=()
if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  problems+=("not A.B.C format (expected N.N.N)")
elif ! ver_gt "$NEW_VERSION" "$CURRENT_VERSION"; then
  problems+=("not greater than current version $CURRENT_VERSION")
fi

if [[ "${#problems[@]}" -gt 0 ]]; then
  echo "warning: $NEW_VERSION failed release validation:" >&2
  for p in "${problems[@]}"; do
    echo "  - $p" >&2
  done
  read -r -p "Proceed anyway? [y/N] " confirm || confirm=""
  if [[ ! "$confirm" =~ ^[Yy]([Ee][Ss])?$ ]]; then
    echo "aborted."
    exit 1
  fi
fi

# --- Bump, commit, tag --------------------------------------------------------
# release.sh has already validated (or the user overrode), so pass -f to skip
# bump-version.sh's own format / same-version guards.
"$ROOT/scripts/bump-version.sh" -f "$NEW_VERSION"

git add Cargo.toml Cargo.lock app/package.json app/src-tauri/tauri.conf.json CHANGELOG.md
git commit -m "build: release v$NEW_VERSION"
git tag "v$NEW_VERSION" -m "build: release v$NEW_VERSION"

echo "Released v$NEW_VERSION (committed + tagged)."

# --- Push? --------------------------------------------------------------------
read -r -p "Push commit and tag to origin? [y/N] " push || push=""
if [[ "$push" =~ ^[Yy]([Ee][Ss])?$ ]]; then
  git push
  git push --tags
  echo "Pushed."
else
  echo "Skipped push. Publish manually with: git push && git push --tags"
fi
