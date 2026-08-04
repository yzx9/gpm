// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
// SPDX-License-Identifier: Apache-2.0

use std::str;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32Str};

use crate::entry::Entry;

/// One page of a ranked entry set: a slice of up to `limit` entries starting at
/// `offset`, together with the **total** count of the full ranked set the slice
/// was taken from (independent of the slice). The caller derives `has_more` as
/// `offset + entries.len() < total`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedPage {
    /// The page's entries (up to `limit`, starting at `offset`).
    pub entries: Vec<Entry>,
    /// Total entries in the full ranked set the page was sliced from.
    pub total: usize,
}

// ---------------------------------------------------------------------------
// Low-level functions (used by Store, also publicly accessible)
// ---------------------------------------------------------------------------

/// Fuzzy-rank `entries` by `query`, best match first.
///
/// Each entry is scored against its `name` and `path` (the higher score wins),
/// so an entry appears at most once. Entries are ordered by score descending,
/// then by `path` ascending. Because `path` is unique, this is a **strict total
/// order** — stable and safe to paginate by offset for a fixed entry set.
///
/// Matching is subsequence-based (fzf-style) and case-insensitive; it is not
/// typo-tolerant. An empty query returns `entries` unchanged, preserving the
/// caller's order (e.g. the alpha order from [`list_entries`]).
#[must_use]
pub fn rank_entries(entries: Vec<Entry>, query: &str) -> Vec<Entry> {
    let q = query.trim();
    if q.is_empty() {
        return entries;
    }
    let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
    let pattern = Pattern::parse(q, CaseMatching::Ignore, Normalization::Smart);
    let mut scored: Vec<(u32, Entry)> = entries
        .into_iter()
        .filter_map(|e| {
            let best = fuzzy_score(&mut matcher, &pattern, &e.name).max(fuzzy_score(
                &mut matcher,
                &pattern,
                &e.path,
            ))?;
            Some((best, e))
        })
        .collect();
    // score desc, then path asc (path is unique → strict total order)
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
    scored.into_iter().map(|(_, e)| e).collect()
}

/// Slice a ranked `Vec<Entry>` to one page of up to `limit` entries starting at
/// `offset`. An `offset` past the end yields an empty page carrying the real
/// `total`. Pure over the input order — the caller ranks first.
#[must_use]
pub fn slice_page(ranked: Vec<Entry>, offset: usize, limit: usize) -> RankedPage {
    let total = ranked.len();
    let entries = ranked.into_iter().skip(offset).take(limit).collect();
    RankedPage { entries, total }
}

/// Score `haystack` against the parsed `pattern` (`None` when it does not
/// fuzzy-match). ASCII haystacks take the fast [`Utf32Str::Ascii`] path;
/// non-ASCII names fall back to a `Vec<char>` buffer.
fn fuzzy_score(matcher: &mut Matcher, pattern: &Pattern, haystack: &str) -> Option<u32> {
    if haystack.is_ascii() {
        pattern.score(Utf32Str::Ascii(haystack.as_bytes()), matcher)
    } else {
        let buf: Vec<char> = haystack.chars().collect();
        pattern.score(Utf32Str::Unicode(&buf), matcher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    // ── rank_entries (fuzzy search ranking) ───────────────────────────

    fn rank_sample_entries() -> Vec<Entry> {
        vec![
            Entry {
                path: "cloud/aws/root.age".to_string(),
                name: "cloud/aws/root".to_string(),
            },
            Entry {
                path: "github.com/user.age".to_string(),
                name: "github-token".to_string(),
            },
            Entry {
                path: "email/personal.age".to_string(),
                name: "personal-email".to_string(),
            },
            Entry {
                path: "servers/prod.age".to_string(),
                name: "prod-server".to_string(),
            },
        ]
    }

    #[test]
    fn rank_entries_empty_query_returns_all_unchanged() {
        assert_eq!(
            rank_entries(rank_sample_entries(), ""),
            rank_sample_entries()
        );
        assert_eq!(
            rank_entries(rank_sample_entries(), "   "),
            rank_sample_entries(),
            "whitespace-only query is treated as empty"
        );
    }

    #[test]
    fn rank_entries_subsequence_non_contiguous_match() {
        // "awroot" matches "cloud/aws/root" as a subsequence (chars in order, gaps ok).
        let r = rank_entries(rank_sample_entries(), "awroot");
        assert!(r.iter().any(|e| e.path == "cloud/aws/root.age"));
    }

    #[test]
    fn rank_entries_case_insensitive() {
        let r = rank_entries(rank_sample_entries(), "AWS");
        assert!(r.iter().any(|e| e.path == "cloud/aws/root.age"));
    }

    #[test]
    fn rank_entries_matches_non_ascii_names() {
        // Exercises the Utf32Str::Unicode branch in fuzzy_score (non-ASCII haystack).
        let e = vec![Entry {
            path: "accounts/café.age".to_string(),
            name: "accounts/café".to_string(),
        }];
        let r = rank_entries(e, "café");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn rank_entries_no_match_returns_empty() {
        assert!(rank_entries(rank_sample_entries(), "zzznomatch").is_empty());
    }

    #[test]
    fn rank_entries_query_longer_than_any_target_excluded() {
        assert!(rank_entries(rank_sample_entries(), "abcdefghijklmnopqrstuvwxyz").is_empty());
    }

    #[test]
    fn rank_entries_best_match_first() {
        let r = rank_entries(rank_sample_entries(), "github");
        assert_eq!(
            r.first().map(|e| e.path.as_str()),
            Some("github.com/user.age")
        );
    }

    #[test]
    fn rank_entries_dedups_across_name_and_path() {
        // "github" matches both the name and the path of one entry → appears once.
        let r = rank_entries(rank_sample_entries(), "github");
        assert_eq!(
            r.iter().filter(|e| e.path == "github.com/user.age").count(),
            1
        );
    }

    #[test]
    fn rank_entries_strict_total_order_tiebreak_by_path() {
        // Two entries with identical names → equal name-score → tiebreak by unique path.
        let e = vec![
            Entry {
                path: "b/zzz.age".to_string(),
                name: "same".to_string(),
            },
            Entry {
                path: "a/zzz.age".to_string(),
                name: "same".to_string(),
            },
        ];
        let r = rank_entries(e, "same");
        assert_eq!(r.len(), 2);
        let paths: Vec<&str> = r.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(paths, vec!["a/zzz.age", "b/zzz.age"]);
    }

    #[test]
    fn rank_entries_perf_5k_synthetic() {
        // Coarse regression guard: ranking 5k entries must stay under this
        // deliberately-loose budget (debug build; generous to avoid CI flakes;
        // catches an O(n^2) or accidental-clone regression). Measured time printed.
        let entries: Vec<Entry> = (0..5_000)
            .map(|i| Entry {
                path: format!("dir/entry-{i}.age"),
                name: format!("dir/entry-{i}"),
            })
            .collect();
        let start = Instant::now();
        let r = rank_entries(entries, "entry-42");
        let elapsed = start.elapsed();
        eprintln!("rank_entries 5k: {elapsed:?}");
        assert!(
            elapsed.as_millis() < 1000,
            "rank_entries 5k took too long: {elapsed:?}"
        );
        assert!(r.iter().any(|e| e.name == "dir/entry-42"));
    }

    // ── slice_page (pagination) ───────────────────────────────────────

    fn page_sample() -> Vec<Entry> {
        vec![
            Entry {
                path: "a.age".to_string(),
                name: "a".to_string(),
            },
            Entry {
                path: "b.age".to_string(),
                name: "b".to_string(),
            },
            Entry {
                path: "c.age".to_string(),
                name: "c".to_string(),
            },
            Entry {
                path: "d.age".to_string(),
                name: "d".to_string(),
            },
            Entry {
                path: "e.age".to_string(),
                name: "e".to_string(),
            },
        ]
    }

    #[test]
    fn slice_page_basic_offset_limit() {
        let p = slice_page(page_sample(), 0, 2);
        assert_eq!(p.total, 5);
        assert_eq!(p.entries.len(), 2);
        let paths: Vec<&str> = p.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a.age", "b.age"]);
    }

    #[test]
    fn slice_page_second_page() {
        let p = slice_page(page_sample(), 2, 2);
        assert_eq!(p.total, 5);
        let paths: Vec<&str> = p.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["c.age", "d.age"]);
    }

    #[test]
    fn slice_page_last_partial_page() {
        // 5 entries, pages of 2 → the last page has 1.
        let p = slice_page(page_sample(), 4, 2);
        assert_eq!(p.entries.len(), 1);
        let paths: Vec<&str> = p.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["e.age"]);
        assert_eq!(p.total, 5);
    }

    #[test]
    fn slice_page_offset_beyond_total_is_empty() {
        let p = slice_page(page_sample(), 10, 2);
        assert!(p.entries.is_empty());
        assert_eq!(p.total, 5, "total stays the real full count");
    }

    #[test]
    fn slice_page_offset_at_boundary() {
        // offset exactly == len → empty page, total preserved.
        let p = slice_page(page_sample(), 5, 2);
        assert!(p.entries.is_empty());
        assert_eq!(p.total, 5);
    }

    #[test]
    fn slice_page_limit_zero_is_empty() {
        let p = slice_page(page_sample(), 0, 0);
        assert!(p.entries.is_empty());
        assert_eq!(p.total, 5);
    }

    #[test]
    fn slice_page_preserves_strict_order_across_pages() {
        // The load-bearing pagination-correctness test: concatenating pages
        // reproduces the full order, including a final partial page. Empty
        // query keeps the input order, so this is deterministic.
        let ranked = rank_entries(page_sample(), "");
        let full: Vec<String> = ranked.iter().map(|e| e.path.clone()).collect();
        let mut paged: Vec<String> = Vec::new();
        let mut offset = 0;
        loop {
            let p = slice_page(ranked.clone(), offset, 2);
            paged.extend(p.entries.iter().map(|e| e.path.clone()));
            offset += 2;
            if p.entries.len() < 2 {
                break;
            }
        }
        assert_eq!(paged, full);
        assert_eq!(full.len(), 5, "sanity: spans 2 full pages + 1 partial");
    }
}
