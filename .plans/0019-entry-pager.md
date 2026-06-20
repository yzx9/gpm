# Entry list pagination (pager)

**Priority:** P2
**Status:** Implemented
**Phase:** Done

## Resolution

Landed as offset pagination with a load-more / infinite-scroll UI, covering both
browse (empty query) and search through one backend page call. The offset-vs-cursor
question resolved to **offset**: the entry set only mutates on an explicit pull, which
the frontend already treats as a reset-to-page-0, so the drift window is nil and
cursor/snapshot complexity was not warranted. Cursor paging (drift-robust against
duplicates) and a server-side ranked snapshot keyed on `(query, git HEAD)` remain the
fallbacks if stores ever grow past ~10k entries.

## What

Paginate the entry list and the fuzzy-search results so the frontend never holds the full entry set — it fetches and renders only a page at a time. Both the browse (no query) and search (with query) paths become page-by-page backend calls.

## Why

Today the WebView loads every entry's name/path on mount and keeps it in memory. That is fine for small stores but blocks large stores (thousands of entries) from loading quickly and keeps the entire entry-name surface resident in the WebView. Pagination lets listing and search scale independently of store size, and is the motivation that drove moving search into the backend in the first place.

## Context

Two properties matter for paging correctness; one is already settled, one is the open design work.

- **Ranking stability (settled, load-bearing).** Search ranking is already a strict total order: matches are ordered by relevance score descending, then by a per-entry key that is genuinely unique (the entry's file-system identity). Because the tiebreak key is unique, there are no ties, so the order is fully deterministic for a fixed entry set and query — offset slicing will never split a tie or reorder between requests. This must be preserved by any future ranker change: an earlier draft tiebroke on the display name, which is _not_ unique after case-folding (two entries can differ only in case on a case-sensitive remote), and that silently broke the total-order claim.

- **Set drift (open — the real work this RFC owes).** Paging is a sequence of independent requests. If the store changes between page 1 and page 2 (a pull adds, removes, or renames an entry), page 2 is ranked over a different set, and naive offset paging can then duplicate or skip entries. This is the classic offset-vs-cursor tradeoff.

Design-level decisions owed: page size (and whether client- or server-chosen); offset paging (simplest, drift-sensitive) vs cursor paging keyed on the last-seen ranking position (drift-robust against duplicates, may skip entries newly inserted ahead of the cursor) vs a server-side ranked snapshot keyed by a generation token (most consistent across pages, but adds server state and invalidation); and how an in-flight search re-runs against a store that changed mid-session (e.g. after a pull). The page-size and page-or-cursor parameters land on the existing search and list commands, which currently return the full set.

Threat-model note: pagination leaks no new secret data — only names/paths, which the full list already exposes. Any cursor or generation token must not encode anything sensitive.

## Alternatives considered

- **Virtual scrolling only (no backend paging).** Keep loading the full list but render a window. Rejected: it does not solve "the frontend holds every entry," which is what motivated backend search, and it does not bound load time for large stores.
- **Paginate search but leave browse unpaginated.** Rejected: browsing a large store (no query) has the same load and memory cost; both paths need paging.

## Effort

~1–2 days (human) for design plus a first offset or cursor implementation / ~30–60 min (CC).

## Depends on / Supersedes

Backend fuzzy search (the search path this paginates).
