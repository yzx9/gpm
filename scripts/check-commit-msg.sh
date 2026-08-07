#!/usr/bin/env bash

# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Conventional-Commit gate (git `commit-msg` stage).
#
# Validates the commit subject against Conventional Commits and confines the
# scope to a CLOSED allowlist, which is the union of:
#   - feature scopes, read live from docs/specs/*/prd.md frontmatter
#     (`scope: <token>`), so adding a spec + its token auto-extends the list;
#   - the static code-area scopes listed below.
# Empty scope is always allowed — Conventional Commits makes scope optional.
#
# Invoked by git/pre-commit as:  check-commit-msg.sh <path-to-commit-msg-file>
# See CONTRIBUTING.md -> Commit Conventions.

set -uo pipefail

if [[ $# -lt 1 || ! -f $1 ]]; then
  echo "[conventional-commit] no commit message file passed" >&2
  exit 1
fi

subject="$(head -n1 "$1")"

# Skip non-editorial / git-generated subjects that have no Conventional form.
case "$subject" in
"" | "Merge "* | "fixup!"* | "squash!"* | "amend!"* | "Revert "*)
  exit 0
  ;;
esac

# --- 1. Conventional Commits structure -------------------------------------
# Whole regex in a variable: bash's [[ =~ ]] parser mishandles unquoted parens
# in an inline pattern, so quoting-via-variable is the canonical fix.
re='^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\(([^)]*)\))?(!)?: .+'
if ! [[ $subject =~ $re ]]; then
  cat >&2 <<'EOF'
[conventional-commit] subject must follow Conventional Commits:

    <type>(<scope>): <summary>          # scope is optional
    feat(lock): skip the biometric prompt after an idle relock
    fix: refresh the entry list on app-unlock

  types: feat fix docs style refactor perf test build ci chore revert
  See CONTRIBUTING.md -> Commit Conventions.
EOF
  exit 1
fi

scope="${BASH_REMATCH[3]:-}"

# Empty scope is always valid.
if [[ -z $scope ]]; then
  exit 0
fi

# --- 2. Build the closed allowlist -----------------------------------------
code_areas=(rustpass app frontend plugin android nix deps)

root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
feat_scopes=()
feat_labels=() # human label (spec title) printed next to each token
shopt -s nullglob
for prd in "$root"/docs/specs/*/prd.md; do
  if [[ $prd == *"/000-template/"* ]]; then
    continue
  fi
  # Pull the scope token (frontmatter) and the first "# " heading (the spec
  # title) in one pass, tab-separated. The title is shown beside the token so
  # the right scope is easy to pick instead of guessing from the bare token.
  IFS=$'\t' read -r tok title <<<"$(awk '
    /^---[[:space:]]*$/ { fences++; next }
    fences == 1 && /^[[:space:]]*scope:/ {
      sub(/^[[:space:]]*scope:[[:space:]]*/, "")
      sub(/[[:space:]].*$/, "")
      scope = $0
    }
    fences >= 2 && title == "" && /^#[[:space:]]+/ {
      sub(/^#[[:space:]]+/, "")
      sub(/[[:space:]]+$/, "")
      title = $0
    }
    END { print scope "\t" title }
  ' "$prd")"
  if [[ -n $tok ]]; then
    feat_scopes+=("$tok")
    # Fall back to the directory slug if the PRD has no heading title.
    feat_labels+=("${title:-$(basename "$(dirname "$prd")")}")
  fi
done
shopt -u nullglob

in_list() {
  local needle="$1"
  shift
  local item
  for item in "$@"; do
    [[ $item == "$needle" ]] && return 0
  done
  return 1
}

# --- 3. Validate -----------------------------------------------------------
if in_list "$scope" "${code_areas[@]}" "${feat_scopes[@]}"; then
  exit 0
fi

if [[ ! $scope =~ ^[a-z0-9-]+$ ]]; then
  reason="scope must be lowercase kebab-case ([a-z0-9-]); no spaces, slashes, or capitals"
else
  reason="scope '$scope' is not in the allowlist"
fi

{
  echo "[conventional-commit] $reason"
  echo "  subject: $subject"
  echo
  echo "  feature scopes (token -> spec, from docs/specs/*/prd.md 'scope:'):"
  if ((${#feat_scopes[@]})); then
    for i in "${!feat_scopes[@]}"; do
      printf '    %-10s %s\n' "${feat_scopes[i]}" "${feat_labels[i]}"
    done
  else
    echo "    (none registered yet)"
  fi
  echo "  code-area scopes:"
  printf '    %s\n' "${code_areas[@]}"
  echo
  echo "  How to choose a scope:"
  echo "    - feature scope   -> a product change, often spanning rustpass + frontend;"
  echo "    - code-area scope -> an internal / tooling change (rustpass, frontend, ci, ...);"
  echo "    - none            -> cross-cutting or no clear home; just drop the (...)."
  echo "  Scope is always optional. See CONTRIBUTING.md -> Commit Conventions."
} >&2
exit 1
