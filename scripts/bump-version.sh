#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
#
# SPDX-License-Identifier: Apache-2.0
#
# Bump version across project files for a new release.
#
# Usage: scripts/bump-version.sh [-f|--force] <new-version>
# Example: scripts/bump-version.sh 1.0.0
#   -f, --force  skip the A.B.C format and same-version guards (release.sh,
#                which has already confirmed with the user, passes this)

set -euo pipefail

FORCE=0
NEW_VERSION=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -f|--force)
      FORCE=1
      ;;
    -*)
      echo "error: unknown option: $1" >&2
      exit 1
      ;;
    *)
      if [[ -n "$NEW_VERSION" ]]; then
        echo "error: unexpected argument: $1" >&2
        exit 1
      fi
      NEW_VERSION="$1"
      ;;
  esac
  shift
done

if [[ -z "$NEW_VERSION" ]]; then
  echo "Usage: $(basename "$0") [-f|--force] <new-version>" >&2
  echo "  e.g. $(basename "$0") 1.0.0" >&2
  exit 1
fi

DATE=$(date +%Y-%m-%d)
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ "$FORCE" -eq 0 ]] && ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: version must be semver (0.0.0), got: $NEW_VERSION (use -f to override)" >&2
  exit 1
fi

CURRENT_VERSION=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$ROOT/app/src-tauri/tauri.conf.json")
if [[ -z "$CURRENT_VERSION" ]]; then
  echo "error: could not read current version from app/src-tauri/tauri.conf.json" >&2
  exit 1
fi

if [[ "$FORCE" -eq 0 ]] && [[ "$CURRENT_VERSION" == "$NEW_VERSION" ]]; then
  echo "error: already at $NEW_VERSION (use -f to override)" >&2
  exit 1
fi

echo "Bumping $CURRENT_VERSION → $NEW_VERSION"

# Detect sed flavor for in-place editing (GNU vs BSD)
if sed --version >/dev/null 2>&1; then
  _sed_i=(sed -i)
else
  _sed_i=(sed -i "")
fi

# 1. Cargo.toml — workspace.package.version
"${_sed_i[@]}" "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" \
  "$ROOT/Cargo.toml"

# 2. Sync Cargo.lock
cd "$ROOT" && cargo check 2>&1

# 3. package.json — also syncs pnpm-lock.yaml if needed
cd "$ROOT/app" && pnpm version --no-git-checks --no-git-tag-version "$NEW_VERSION"

# 4. src-tauri/tauri.conf.json
"${_sed_i[@]}" "s/\"version\": \"$CURRENT_VERSION\"/\"version\": \"$NEW_VERSION\"/" \
  "$ROOT/app/src-tauri/tauri.conf.json"

# 5. CHANGELOG.md — insert new version header after [Unreleased]
"${_sed_i[@]}" "/^## \[Unreleased\]$/a\\
\\
## [v$NEW_VERSION] - $DATE
" "$ROOT/CHANGELOG.md"

# Update [Unreleased] comparison link
"${_sed_i[@]}" "s|compare/v${CURRENT_VERSION}...HEAD|compare/v${NEW_VERSION}...HEAD|" \
  "$ROOT/CHANGELOG.md"

# Add new version link before the current version link
"${_sed_i[@]}" "/^\[v${CURRENT_VERSION}\]/i\\
[v${NEW_VERSION}]: https://github.com/yzx9/gpm/compare/v${CURRENT_VERSION}...v${NEW_VERSION}
" "$ROOT/CHANGELOG.md"

echo "Done. Updated: Cargo.toml, Cargo.lock, app/package.json, app/src-tauri/tauri.conf.json, CHANGELOG.md"
