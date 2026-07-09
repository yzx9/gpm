// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Shared pagination helpers for the Tauri-IPC layer.
//!
//! `clamp_limit` / `MAX_PAGE_SIZE` cap a client-requested page size so a buggy
//! or malicious caller can't ask for `usize::MAX`. Both paged commands — the
//! entry list ([`crate::read`]) and the commit-signature history
//! ([`crate::authenticity`]) — route their `limit` through this.
//!
//! `has_more` is deliberately NOT shared here: the entry list derives it from a
//! full `total` (`offset + len < total`), while commit history derives it from a
//! page-full walk (`rustpass::CommitSigPage::has_more`). Different derivations,
//! kept separate to avoid a leaky shared predicate.

/// Upper bound on a client-requested page size. The frontend requests 50 by
/// default.
pub(crate) const MAX_PAGE_SIZE: usize = 200;

/// Clamp a client-requested page size to a sane, non-zero bound.
#[must_use]
pub(crate) fn clamp_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_PAGE_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_limit_bounds_request_size() {
        assert_eq!(clamp_limit(0), 1);
        assert_eq!(clamp_limit(1), 1);
        assert_eq!(clamp_limit(50), 50);
        assert_eq!(clamp_limit(MAX_PAGE_SIZE), MAX_PAGE_SIZE);
        assert_eq!(clamp_limit(MAX_PAGE_SIZE + 1), MAX_PAGE_SIZE);
        assert_eq!(clamp_limit(usize::MAX), MAX_PAGE_SIZE);
    }
}
