// SPDX-License-Identifier: MIT OR Apache-2.0

//! A small reader for the zip files the study build exports (task 1.9): stored or
//! deflated entries, no encryption, no zip64. An export comes from a participant's
//! machine, so nothing in it is trusted: every offset and length is checked, sizes are
//! capped, and every entry's CRC-32 is verified.

use std::collections::BTreeMap;

/// Most entries in one archive.
const MAX_ENTRIES: usize = 10_000;
/// Largest single entry, uncompressed.
const MAX_ENTRY_BYTES: usize = 256 << 20;
/// Largest archive contents, uncompressed.
const MAX_TOTAL_BYTES: usize = 1 << 30;

const END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const LOCAL_HEADER: u32 = 0x0403_4b50;

fn u16_at(bytes: &[u8], at: usize) -> Option<usize> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(usize::from(u16::from_le_bytes([b[0], b[1]])))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// CRC-32 (IEEE 802.3), as zip uses.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// Every file in the archive, by name. Directory entries are skipped.
///
/// # Errors
/// A description of what is wrong with the archive. Nothing is returned from an
/// archive with any bad entry.
pub fn read(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let bad = |what: &str| format!("not a usable zip file: {what}");
    // The end record is the last 22 bytes, or earlier if a comment follows it.
    let latest = bytes
        .len()
        .checked_sub(22)
        .ok_or_else(|| bad("too short"))?;
    let earliest = latest.saturating_sub(usize::from(u16::MAX));
    let end = (earliest..=latest)
        .rev()
        .find(|at| u32_at(bytes, *at) == Some(END_OF_CENTRAL_DIRECTORY))
        .ok_or_else(|| bad("no end record"))?;
    let truncated = || bad("truncated");
    let entries = u16_at(bytes, end + 10).ok_or_else(truncated)?;
    let directory = u32_at(bytes, end + 16).ok_or_else(truncated)?;
    if entries == usize::from(u16::MAX) || directory == u32::MAX {
        return Err(bad("zip64 archives are not supported"));
    }
    if entries > MAX_ENTRIES {
        return Err(bad("too many entries"));
    }

    let mut files = BTreeMap::new();
    let mut total = 0usize;
    let mut at = directory as usize;
    for _ in 0..entries {
        if u32_at(bytes, at) != Some(CENTRAL_HEADER) {
            return Err(bad("bad central directory"));
        }
        let flags = u16_at(bytes, at + 8).ok_or_else(truncated)?;
        let method = u16_at(bytes, at + 10).ok_or_else(truncated)?;
        let crc = u32_at(bytes, at + 16).ok_or_else(truncated)?;
        let packed = u32_at(bytes, at + 20).ok_or_else(truncated)?;
        let size = u32_at(bytes, at + 24).ok_or_else(truncated)?;
        let name_len = u16_at(bytes, at + 28).ok_or_else(truncated)?;
        let extra_len = u16_at(bytes, at + 30).ok_or_else(truncated)?;
        let comment_len = u16_at(bytes, at + 32).ok_or_else(truncated)?;
        let local = u32_at(bytes, at + 42).ok_or_else(truncated)?;
        let name = bytes
            .get(at + 46..at + 46 + name_len)
            .ok_or_else(truncated)?;
        let name = std::str::from_utf8(name).map_err(|_| bad("a name is not UTF-8"))?;
        at += 46 + name_len + extra_len + comment_len;

        if packed == u32::MAX || size == u32::MAX || local == u32::MAX {
            return Err(bad("zip64 archives are not supported"));
        }
        if flags & 1 != 0 {
            return Err(bad("encrypted entries are not supported"));
        }
        if name.ends_with('/') {
            continue;
        }
        let (packed, size, local) = (packed as usize, size as usize, local as usize);
        total = total.saturating_add(size);
        if size > MAX_ENTRY_BYTES || total > MAX_TOTAL_BYTES {
            return Err(bad("contents too large"));
        }
        // The local header repeats the name and has its own extra field.
        if u32_at(bytes, local) != Some(LOCAL_HEADER) {
            return Err(bad("bad local header"));
        }
        let skip = u16_at(bytes, local + 26).ok_or_else(truncated)?
            + u16_at(bytes, local + 28).ok_or_else(truncated)?;
        let start = local + 30 + skip;
        let data = start
            .checked_add(packed)
            .and_then(|stop| bytes.get(start..stop))
            .ok_or_else(truncated)?;
        let contents = match method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, size)
                .map_err(|_| bad("an entry does not inflate"))?,
            _ => return Err(bad("unsupported compression method")),
        };
        if contents.len() != size || crc32(&contents) != crc {
            return Err(format!("not a usable zip file: {name} is corrupt"));
        }
        if files.insert(name.to_owned(), contents).is_some() {
            return Err(format!("not a usable zip file: {name} appears twice"));
        }
    }
    Ok(files)
}

/// Writes an archive, for tests: each entry deflated or stored.
#[cfg(test)]
pub(crate) fn write(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, contents) in entries {
        let packed = if deflate {
            miniz_oxide::deflate::compress_to_vec(contents, 6)
        } else {
            contents.to_vec()
        };
        let method: u16 = if deflate { 8 } else { 0 };
        let offset = out.len() as u32;
        let mut fields = Vec::new();
        fields.extend_from_slice(&20u16.to_le_bytes()); // version needed
        fields.extend_from_slice(&0u16.to_le_bytes()); // flags
        fields.extend_from_slice(&method.to_le_bytes());
        fields.extend_from_slice(&[0; 4]); // time, date
        fields.extend_from_slice(&crc32(contents).to_le_bytes());
        fields.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        fields.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        fields.extend_from_slice(&(name.len() as u16).to_le_bytes());
        fields.extend_from_slice(&0u16.to_le_bytes()); // extra length
        out.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
        out.extend_from_slice(&fields);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&packed);
        directory.extend_from_slice(&CENTRAL_HEADER.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes()); // version made by
        directory.extend_from_slice(&fields);
        directory.extend_from_slice(&[0; 10]); // comment, disk, attributes
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(&END_OF_CENTRAL_DIRECTORY.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // disk numbers
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<(&'static str, Vec<u8>)> {
        let long: Vec<u8> = (0..4_000u32).flat_map(|i| (i % 97).to_le_bytes()).collect();
        vec![
            ("manifest.json", b"{\"format\": \"x\"}\n".to_vec()),
            ("sessions/baseline-01.jsonl", long),
            ("empty", Vec::new()),
        ]
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn reads_stored_and_deflated_archives() {
        let entries = entries();
        let borrowed: Vec<(&str, &[u8])> =
            entries.iter().map(|(n, c)| (*n, c.as_slice())).collect();
        for deflate in [false, true] {
            let archive = write(&borrowed, deflate);
            let files = read(&archive).unwrap();
            assert_eq!(files.len(), 3);
            for (name, contents) in &entries {
                assert_eq!(&files[*name], contents, "{name}");
            }
        }
        // Deflate really compressed something.
        assert!(write(&borrowed, true).len() < write(&borrowed, false).len() / 2);
    }

    /// An archive from another writer (CPython's `zipfile`, zlib deflate): one entry,
    /// `a.txt`, holding "hello hello hello hello\n".
    #[test]
    fn reads_an_archive_from_another_writer() {
        let files = read(&PYTHON_ZIP).unwrap();
        assert_eq!(files["a.txt"], b"hello hello hello hello\n");
    }

    const PYTHON_ZIP: [u8; 119] = [
        0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x21, 0x00, 0x00,
        0x88, 0x59, 0x0b, 0x0b, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00,
        0x61, 0x2e, 0x74, 0x78, 0x74, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0xb9,
        0x00, 0x50, 0x4b, 0x01, 0x02, 0x14, 0x03, 0x14, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00,
        0x21, 0x00, 0x00, 0x88, 0x59, 0x0b, 0x0b, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, 0x05,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x61, 0x2e, 0x74, 0x78, 0x74, 0x50, 0x4b, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x01, 0x00, 0x33, 0x00, 0x00, 0x00, 0x2e, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn corruption_and_truncation_are_errors_not_panics() {
        let entries = entries();
        let borrowed: Vec<(&str, &[u8])> =
            entries.iter().map(|(n, c)| (*n, c.as_slice())).collect();
        for deflate in [false, true] {
            let archive = write(&borrowed, deflate);
            for cut in [0, 1, 21, 22, archive.len() / 2, archive.len() - 1] {
                assert!(read(&archive[..cut]).is_err(), "cut at {cut}");
            }
            // Flip one byte at a time through the whole archive (sampled): never a
            // panic, and never the original contents under a wrong checksum.
            for at in (0..archive.len()).step_by(archive.len() / 400 + 1) {
                let mut broken = archive.clone();
                broken[at] ^= 0x41;
                if let Ok(files) = read(&broken) {
                    for (name, contents) in &files {
                        let original = entries.iter().find(|(n, _)| n == name);
                        assert!(
                            original.is_none_or(|(_, c)| c == contents),
                            "byte {at}: {name} changed silently"
                        );
                    }
                }
            }
        }
        assert!(read(b"").is_err());
        assert!(read(&[0x50, 0x4b, 0x05, 0x06]).is_err());
    }
}
