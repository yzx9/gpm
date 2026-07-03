// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Crypto backend abstraction.
//!
//! Home of the `CryptoBackend` trait (the swappable encryption-backend
//! interface, mirroring gopass's `internal/backend/crypto.go`). The age
//! implementation lives in [`age`] and is the sole backend today; the next
//! commit introduces the trait + an `AgeBackend` impl and wires `Store` to
//! `Box<dyn CryptoBackend>`.
//!
//! For now the age functions are re-exported here so existing `crypto::`
//! call sites keep resolving unchanged.

/// The age encryption backend (the sole `CryptoBackend` implementation today).
pub mod age;

#[allow(unused_imports)]
// re-export brings the age impl surface to `crypto::` for existing callers
pub use age::*;
