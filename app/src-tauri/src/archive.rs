// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared tarball helpers for the two export assemblers
//! ([`crate::diagnostics_export`] and [`crate::repo_export`]). Both wrap a
//! `flate2` gzip encoder around a [`tar::Builder`]; this module owns the two
//! pieces they have in common — appending an entry with deterministic metadata
//! ([`append_entry`]) and finalizing the tar+gzip trailers ([`finish_tar_gz`])
//! — so the metadata policy and the non-obvious trailer ordering live in one
//! place.

use std::io::Read;

use rustpass::{Error, ErrorCode};

/// Append one entry to a tar [`Builder`](tar::Builder) with deterministic
/// metadata (GNU header, mode `0o644`, mtime `0`) and **verify the byte count**.
///
/// `tar::Builder::append_data` copies `data` via `io::copy`, which returns
/// `Ok(short)` on early EOF and pads by bytes written — so a mismatch between
/// `size` and the bytes `data` actually yields would silently produce a
/// structurally corrupt archive that still returns `Ok`. For a backup producer
/// that is the worst failure mode, so we wrap `data` in a [`CountReader`] and
/// fail loud when `count != size`.
///
/// Pinning the tar-entry mtime to `0` (here) and the gzip-header mtime to `0`
/// (via `GzEncoder::new` at the call sites) keeps the headers deterministic;
/// two exports of identical inputs differ only by the manifest's `generated`
/// timestamp (and the input bytes themselves).
pub(crate) fn append_entry<W>(
    builder: &mut tar::Builder<W>,
    name: &str,
    data: &mut dyn Read,
    size: u64,
) -> Result<(), Error>
where
    W: std::io::Write,
{
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(0o644);
    header.set_mtime(0);
    let mut counter = CountReader::new(data);
    builder
        .append_data(&mut header, name, &mut counter)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("tar append {name}: {e}")))?;
    let read = counter.count();
    if read != size {
        return Err(Error::new(
            ErrorCode::StoreError,
            format!("tar append {name}: size mismatch (declared {size}, read {read})"),
        ));
    }
    Ok(())
}

/// Finalize a tar+gzip pipeline and return the inner writer. Flushes the tar
/// trailer first (`Builder::into_inner` writes the two terminating zero blocks)
/// then the gzip trailer (`GzEncoder::finish` writes the CRC/ISIZE). Ordering
/// matters: `into_inner` must precede `finish` so the tar trailer is flushed
/// through the gzip encoder — reversing would strand it in the encoder buffer.
pub(crate) fn finish_tar_gz<F>(
    builder: tar::Builder<flate2::write::GzEncoder<F>>,
) -> Result<F, Error>
where
    F: std::io::Write,
{
    let encoder = builder
        .into_inner()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("tar finish: {e}")))?;
    encoder
        .finish()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("gzip finish: {e}")))
}

/// Counts bytes read through an inner reader so [`append_entry`] can verify the
/// streamed length matches the declared `size`. Never buffers.
struct CountReader<'a> {
    inner: &'a mut dyn Read,
    count: u64,
}

impl<'a> CountReader<'a> {
    fn new(inner: &'a mut dyn Read) -> Self {
        Self { inner, count: 0 }
    }

    fn count(&self) -> u64 {
        self.count
    }
}

impl Read for CountReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip one entry through the helper: it lands at the right name with
    /// the right bytes and the deterministic metadata we promised.
    #[test]
    fn append_entry_round_trips_name_size_and_bytes() {
        let mut builder = tar::Builder::new(Vec::new());
        let bytes = b"hello tarball";
        append_entry(
            &mut builder,
            "a.txt",
            &mut std::io::Cursor::new(&bytes[..]),
            bytes.len() as u64,
        )
        .unwrap();
        let written = builder.into_inner().unwrap();

        let mut ar = tar::Archive::new(std::io::Cursor::new(written));
        let mut entries = ar.entries().unwrap();
        let mut e = entries.next().unwrap().unwrap();
        assert_eq!(e.path().unwrap().to_str().unwrap(), "a.txt");
        assert_eq!(e.header().size().unwrap(), bytes.len() as u64);
        assert_eq!(e.header().mode().unwrap(), 0o644);
        assert_eq!(e.header().mtime().unwrap(), 0);
        let mut out = Vec::new();
        e.read_to_end(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert!(entries.next().is_none(), "exactly one entry");
    }

    /// A declared size larger than the data must fail loud — otherwise tar would
    /// silently write a corrupt archive and return `Ok` (the backup-safety fix).
    #[test]
    fn append_entry_errors_when_declared_size_exceeds_data() {
        let mut builder = tar::Builder::new(Vec::new());
        let bytes = b"hello";
        let err = append_entry(
            &mut builder,
            "a.txt",
            &mut std::io::Cursor::new(&bytes[..]),
            100,
        )
        .expect_err("size mismatch should error");
        assert_eq!(err.code, "STORE_ERROR");
        assert!(err.message.contains("size mismatch"), "{}", err.message);
    }

    /// The overshoot direction (more bytes than declared) is also a mismatch.
    #[test]
    fn append_entry_errors_when_data_exceeds_declared_size() {
        let mut builder = tar::Builder::new(Vec::new());
        let bytes = b"hello tarball";
        let err = append_entry(
            &mut builder,
            "a.txt",
            &mut std::io::Cursor::new(&bytes[..]),
            3,
        )
        .expect_err("size mismatch should error");
        assert_eq!(err.code, "STORE_ERROR");
    }
}
