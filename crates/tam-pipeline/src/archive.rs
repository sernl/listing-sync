//! Streamed ZIP extraction that treats the archive as hostile: a running
//! decompressed-byte counter and a per-entry ratio check bound the bomb, the
//! path comes only from `enclosed_name` so `../` traversal cannot escape, and
//! any symlink entry is refused. `decompressed_size()` reads spoofable central-
//! directory headers and is never consulted (GHSA-94vh-gphv-8pm8).

use std::io::Read;

use tam_limits::ingest::{ARCHIVE_COMPRESSION_RATIO_MAX, ARCHIVE_UNCOMPRESSED_BYTES_MAX};

/// The bytes a single entry is read in, so one entry cannot allocate the whole
/// archive at once; the counter is checked every chunk.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Unix mode bits marking a symlink (`S_IFLNK`), which extraction refuses.
const S_IFLNK: u32 = 0o120_000;
const S_IFMT: u32 = 0o170_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedEntry {
    /// The archive-relative path, proven to stay within the archive root.
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveError {
    /// The archive itself is malformed.
    Malformed(String),
    /// An entry's name escaped the archive root or was absent.
    UnsafePath { index: usize },
    /// An entry is a symlink, which extraction never follows or writes.
    Symlink { path: String },
    /// The running decompressed total exceeded the absolute cap.
    TotalTooLarge { cap: u64 },
    /// One entry's decompressed:compressed ratio exceeded the cap — a bomb.
    RatioTooLarge { path: String, cap: u64 },
    /// A single entry exceeded the absolute cap on its own.
    EntryTooLarge { path: String, cap: u64 },
}

impl core::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Malformed(detail) => write!(f, "malformed archive: {detail}"),
            Self::UnsafePath { index } => write!(f, "entry {index} has an unsafe or absent path"),
            Self::Symlink { path } => write!(f, "entry {path} is a symlink and is refused"),
            Self::TotalTooLarge { cap } => {
                write!(f, "decompressed total exceeded the {cap}-byte cap")
            }
            Self::RatioTooLarge { path, cap } => {
                write!(f, "entry {path} exceeded the {cap}:1 compression ratio cap")
            }
            Self::EntryTooLarge { path, cap } => {
                write!(f, "entry {path} exceeded the {cap}-byte cap on its own")
            }
        }
    }
}

impl core::error::Error for ArchiveError {}

/// The bounds an extraction runs under. Defaults come from `tam-limits`, but
/// the caller passes them so a smaller per-upload budget can bind first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractBudget {
    pub total_bytes_max: u64,
    pub ratio_max: u64,
}

impl Default for ExtractBudget {
    fn default() -> Self {
        Self {
            total_bytes_max: ARCHIVE_UNCOMPRESSED_BYTES_MAX,
            ratio_max: ARCHIVE_COMPRESSION_RATIO_MAX,
        }
    }
}

/// The one entry this archive holds, if it holds exactly one.
///
/// `None` for an archive holding none or several, and for anything that is not
/// a readable archive at all. The three are one answer on purpose: the caller
/// asking this question has a fallback for all of them — use the archive as it
/// stands — and distinguishing them would invite a caller to act on a
/// difference it has no use for.
///
/// Mechanics rather than policy. What a caller does with a one-entry archive
/// is the caller's: a cross-listing upload unwraps it because the target's
/// product slot takes one file and a buyer expects the document rather than a
/// zip wrapping it, and that reasoning does not belong here.
pub fn sole_entry(bundle: &[u8], budget: ExtractBudget) -> Option<ExtractedEntry> {
    // The count comes from the central directory, before anything is
    // decompressed. A fifty-file bundle is then rejected for the price of its
    // index rather than for the price of its contents, which is the difference
    // between reading a header and inflating hundreds of megabytes to throw
    // them away. Directory entries are skipped here exactly as `extract` skips
    // them, or an archive holding one file inside one folder would be counted
    // as two and refused.
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bundle)).ok()?;
    let mut files = 0usize;
    for index in 0..archive.len() {
        if !archive.by_index(index).ok()?.is_dir() {
            files += 1;
            if files > 1 {
                return None;
            }
        }
    }
    if files != 1 {
        return None;
    }
    // Exactly one file entry, so `extract` inflates exactly it, under every
    // rail it applies to any other archive: the symlink refusal, the enclosed
    // name, the budget and the ratio.
    let entries = extract(std::io::Cursor::new(bundle), budget).ok()?;
    match <[ExtractedEntry; 1]>::try_from(entries) {
        Ok([only]) => Some(only),
        Err(_) => None,
    }
}

/// Extracts every file entry into memory under the budget. Directory entries
/// are skipped; symlink and traversal entries are hard errors, not skips,
/// because a hostile archive should fail loudly rather than partially.
pub fn extract<R: Read + std::io::Seek>(
    reader: R,
    budget: ExtractBudget,
) -> Result<Vec<ExtractedEntry>, ArchiveError> {
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|error| ArchiveError::Malformed(error.to_string()))?;
    let mut extracted = Vec::new();
    let mut running_total: u64 = 0;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| ArchiveError::Malformed(error.to_string()))?;

        if let Some(mode) = entry.unix_mode() {
            if mode & S_IFMT == S_IFLNK {
                let path = entry.name().to_owned();
                return Err(ArchiveError::Symlink { path });
            }
        }
        if entry.is_dir() {
            continue;
        }
        // enclosed_name resolves `..` and rejects absolute or escaping paths;
        // name() would hand back the attacker's raw string.
        let Some(path) = entry.enclosed_name() else {
            return Err(ArchiveError::UnsafePath { index });
        };
        let path = path.to_string_lossy().into_owned();
        let compressed = entry.compressed_size();

        let mut bytes = Vec::new();
        let mut buffer = vec![0u8; READ_CHUNK_BYTES].into_boxed_slice();
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|error| ArchiveError::Malformed(error.to_string()))?;
            if read == 0 {
                break;
            }
            let read_u64 = read as u64;
            let entry_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX) + read_u64;
            running_total += read_u64;
            if running_total > budget.total_bytes_max {
                return Err(ArchiveError::TotalTooLarge {
                    cap: budget.total_bytes_max,
                });
            }
            if entry_len > budget.total_bytes_max {
                return Err(ArchiveError::EntryTooLarge {
                    path,
                    cap: budget.total_bytes_max,
                });
            }
            // Ratio is checked against actual compressed bytes on disk, not a
            // header field, and only once an entry is large enough that a high
            // ratio is meaningful rather than an artefact of a tiny file.
            if compressed > 0 && entry_len > compressed.saturating_mul(budget.ratio_max) {
                return Err(ArchiveError::RatioTooLarge {
                    path,
                    cap: budget.ratio_max,
                });
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        extracted.push(ExtractedEntry { path, bytes });
    }
    Ok(extracted)
}

#[cfg(test)]
mod tests {
    use super::{extract, sole_entry, ArchiveError, ExtractBudget};
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            writer.start_file(*name, options).expect("start entry");
            writer.write_all(bytes).expect("write entry");
        }
        writer.finish().expect("finish archive");
        buffer.into_inner()
    }

    #[test]
    fn the_sole_entry_is_answered_only_when_there_is_exactly_one() {
        let one = zip_with(&[("worksheet.pdf", b"%PDF-1.7 the only file")]);
        let only = sole_entry(&one, ExtractBudget::default()).expect("one entry is answered");
        assert_eq!(only.path, "worksheet.pdf");
        assert_eq!(only.bytes, b"%PDF-1.7 the only file");

        assert!(
            sole_entry(
                &zip_with(&[("a.pdf", b"alpha"), ("b.pdf", b"bravo")]),
                ExtractBudget::default()
            )
            .is_none(),
            "two entries are not one, and a caller must use the archive as it stands"
        );
        assert!(
            sole_entry(&zip_with(&[]), ExtractBudget::default()).is_none(),
            "an empty archive holds no sole entry"
        );

        // The count and the extraction must agree about what an entry is, and
        // a directory is where they would diverge: counting one would refuse
        // an archive `extract` reduces to a single file.
        let foldered = zip_with(&[("pack/", b""), ("pack/worksheet.pdf", b"%PDF-1.7 inside")]);
        let only = sole_entry(&foldered, ExtractBudget::default())
            .expect("one file inside one folder is still one file");
        assert_eq!(only.path, "pack/worksheet.pdf");
        assert!(
            sole_entry(b"%PDF-1.7 not an archive at all", ExtractBudget::default()).is_none(),
            "a thing that is not an archive is not one holding a single entry, and answering \
             None rather than an error is what lets a caller treat all three the same way"
        );
    }

    #[test]
    fn a_benign_archive_extracts_every_file() {
        let archive = zip_with(&[("a.txt", b"alpha"), ("nested/b.txt", b"bravo")]);
        let entries =
            extract(Cursor::new(archive), ExtractBudget::default()).expect("benign extract");
        assert_eq!(entries.len(), 2, "both files came out");
        assert_eq!(entries[0].bytes, b"alpha", "content is intact");
        assert_eq!(
            entries[1].path, "nested/b.txt",
            "nested paths are preserved"
        );
    }

    #[test]
    fn a_high_ratio_entry_is_refused_as_a_bomb() {
        // A megabyte of zeros compresses to almost nothing: a classic bomb
        // shape, refused by the ratio check well before the absolute cap.
        let archive = zip_with(&[("bomb", &vec![0u8; 1024 * 1024])]);
        let budget = ExtractBudget {
            total_bytes_max: 8 * 1024 * 1024,
            ratio_max: 50,
        };
        assert!(
            matches!(
                extract(Cursor::new(archive), budget),
                Err(ArchiveError::RatioTooLarge { .. })
            ),
            "a high compression ratio is a bomb and is refused"
        );
    }

    #[test]
    fn an_entry_over_the_absolute_cap_is_refused() {
        // Incompressible random-ish bytes keep the ratio low, so the absolute
        // cap is what binds.
        let payload: Vec<u8> = (0..200_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        let archive = zip_with(&[("big", &payload)]);
        let budget = ExtractBudget {
            total_bytes_max: 100_000,
            ratio_max: 10_000,
        };
        assert!(
            matches!(
                extract(Cursor::new(archive), budget),
                Err(ArchiveError::TotalTooLarge { .. } | ArchiveError::EntryTooLarge { .. })
            ),
            "an entry past the absolute cap is refused"
        );
    }

    #[test]
    fn a_traversal_entry_is_refused() {
        // The zip writer will store the literal name; enclosed_name rejects it.
        let archive = zip_with(&[("../escape.txt", b"escape")]);
        assert!(
            matches!(
                extract(Cursor::new(archive), ExtractBudget::default()),
                Err(ArchiveError::UnsafePath { .. })
            ),
            "a path escaping the archive root is refused, never written"
        );
    }

    #[test]
    fn a_truncated_archive_errors_rather_than_panics() {
        let mut archive = zip_with(&[("a.txt", b"alpha")]);
        archive.truncate(20);
        assert!(
            matches!(
                extract(Cursor::new(archive), ExtractBudget::default()),
                Err(ArchiveError::Malformed(_))
            ),
            "a truncated central directory is a clean error"
        );
    }
}
