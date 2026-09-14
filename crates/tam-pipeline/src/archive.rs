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

/// Every file entry's path, from the central directory alone.
///
/// No entry is inflated, which is the point: a caller choosing between
/// entries reads the index for the price of the index and then inflates the
/// one it chose through [`entry`]. `None` for anything that is not a readable
/// archive, the same answer [`sole_entry`] gives it.
#[must_use]
pub fn file_paths(bundle: &[u8]) -> Option<Vec<String>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bundle)).ok()?;
    let mut paths = Vec::new();
    for index in 0..archive.len() {
        let found = archive.by_index_raw(index).ok()?;
        if found.is_dir() {
            continue;
        }
        // `enclosed_name` rather than `name`, so a path this answers is one a
        // caller may pass straight back to `entry`: an escaping name is left
        // out of the index rather than offered and then refused.
        if let Some(path) = found.enclosed_name() {
            paths.push(path.to_string_lossy().into_owned());
        }
    }
    Some(paths)
}

/// One named entry, inflated alone under the same rails [`extract`] applies.
///
/// `Ok(None)` is an archive that holds no such file entry. The rails are not
/// restated here: the symlink refusal, the enclosed name, the running total
/// and the ratio all live in the shared `read_entry`, so a targeted read
/// cannot drift from a whole extraction's bounds. Only the named entry is
/// inflated, so a caller inspecting one file of a fifty-file bundle pays for
/// one.
///
/// `spent` is the caller's running total across however many of these reads
/// it makes, and it is charged as the bytes arrive rather than on success.
/// That ordering is the bound: an entry whose stream fails its CRC after
/// inflating most of itself has still done that work, and a counter advanced
/// only by a successful read would let a bundle of deliberately corrupt
/// members be inflated over and over for free. `budget.total_bytes_max` is
/// therefore the cap on that total, not on each read.
///
/// The member is chosen from the index before any decoder exists, and that
/// ordering is the whole of what makes this read about one entry.
/// `by_index` constructs a reader, and constructing one fails outright for a
/// member that is encrypted or uses a compression method this build does not
/// carry — so opening every member on the way to the requested one let an
/// unrelated entry decide the answer, and the same bundle written in the
/// other order gave a different one. The raw index carries the name, the
/// mode and the compressed size, which is everything the choice needs.
///
/// # Errors
///
/// The archive is malformed, or the named entry is a symlink, cannot be
/// opened, or exceeds the budget.
pub fn entry(
    bundle: &[u8],
    wanted: &str,
    budget: ExtractBudget,
    spent: &mut u64,
) -> Result<Option<ExtractedEntry>, ArchiveError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bundle))
        .map_err(|error| ArchiveError::Malformed(error.to_string()))?;
    let mut selected: Option<(usize, String, u64)> = None;
    for index in 0..archive.len() {
        // A member whose own index entry will not even be read is passed
        // over: the caller named one file, and failing over another entry's
        // metadata would let a bundle's worst member decide its best one's
        // fate. A directory that cannot be read is not the requested file
        // either.
        let Ok(found) = archive.by_index_raw(index) else {
            continue;
        };
        if found.is_dir() {
            continue;
        }
        let Some(path) = found.enclosed_name() else {
            continue;
        };
        let path = path.to_string_lossy().into_owned();
        if path != wanted {
            continue;
        }
        if let Some(mode) = found.unix_mode() {
            if mode & S_IFMT == S_IFLNK {
                return Err(ArchiveError::Symlink { path });
            }
        }
        selected = Some((index, path, found.compressed_size()));
        break;
    }
    let Some((index, path, compressed)) = selected else {
        return Ok(None);
    };
    // The one decoder this read constructs, for the one member it chose.
    let mut found = archive
        .by_index(index)
        .map_err(|error| ArchiveError::Malformed(error.to_string()))?;
    let bytes = read_entry(&mut found, &path, compressed, budget, spent)?;
    Ok(Some(ExtractedEntry { path, bytes }))
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
        let mut found = archive
            .by_index(index)
            .map_err(|error| ArchiveError::Malformed(error.to_string()))?;

        if let Some(mode) = found.unix_mode() {
            if mode & S_IFMT == S_IFLNK {
                let path = found.name().to_owned();
                return Err(ArchiveError::Symlink { path });
            }
        }
        if found.is_dir() {
            continue;
        }
        // enclosed_name resolves `..` and rejects absolute or escaping paths;
        // name() would hand back the attacker's raw string.
        let Some(path) = found.enclosed_name() else {
            return Err(ArchiveError::UnsafePath { index });
        };
        let path = path.to_string_lossy().into_owned();
        let compressed = found.compressed_size();
        let bytes = read_entry(&mut found, &path, compressed, budget, &mut running_total)?;
        extracted.push(ExtractedEntry { path, bytes });
    }
    Ok(extracted)
}

/// One entry's bytes, read in chunks with every bound checked as they arrive.
///
/// The running total is the caller's, so a whole extraction's entries share
/// one budget while a single targeted read carries its own — the check itself
/// is the same code either way, which is why the bounds cannot differ between
/// the two callers.
fn read_entry(
    source: &mut impl Read,
    path: &str,
    compressed: u64,
    budget: ExtractBudget,
    running_total: &mut u64,
) -> Result<Vec<u8>, ArchiveError> {
    let mut bytes = Vec::new();
    let mut buffer = vec![0u8; READ_CHUNK_BYTES].into_boxed_slice();
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|error| ArchiveError::Malformed(error.to_string()))?;
        if read == 0 {
            break;
        }
        let read_u64 = read as u64;
        let entry_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX) + read_u64;
        *running_total += read_u64;
        if *running_total > budget.total_bytes_max {
            return Err(ArchiveError::TotalTooLarge {
                cap: budget.total_bytes_max,
            });
        }
        if entry_len > budget.total_bytes_max {
            return Err(ArchiveError::EntryTooLarge {
                path: path.to_owned(),
                cap: budget.total_bytes_max,
            });
        }
        // Ratio is checked against actual compressed bytes on disk, not a
        // header field, and only once an entry is large enough that a high
        // ratio is meaningful rather than an artefact of a tiny file.
        if compressed > 0 && entry_len > compressed.saturating_mul(budget.ratio_max) {
            return Err(ArchiveError::RatioTooLarge {
                path: path.to_owned(),
                cap: budget.ratio_max,
            });
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{entry, extract, file_paths, sole_entry, ArchiveError, ExtractBudget};
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
    fn the_index_names_files_and_the_targeted_read_inflates_one() {
        let archive = zip_with(&[
            ("pack/", b""),
            ("pack/worksheet.pdf", b"%PDF-1.7 the worksheet"),
            ("pack/preview.png", b"not really a png, but bytes"),
        ]);
        assert_eq!(
            file_paths(&archive).expect("an index"),
            vec!["pack/worksheet.pdf", "pack/preview.png"],
            "the index names every file entry and no directory, in archive order"
        );
        let mut spent = 0u64;
        let found = entry(
            &archive,
            "pack/preview.png",
            ExtractBudget::default(),
            &mut spent,
        )
        .expect("a readable archive")
        .expect("the named entry");
        assert_eq!(found.bytes, b"not really a png, but bytes");
        assert_eq!(
            spent,
            found.bytes.len() as u64,
            "the read charges the caller exactly what it inflated"
        );
        assert!(
            entry(
                &archive,
                "pack/absent.png",
                ExtractBudget::default(),
                &mut spent
            )
            .expect("a readable archive")
            .is_none(),
            "an entry the archive does not hold is an absence rather than an error"
        );
        assert!(
            file_paths(b"%PDF-1.7 not an archive").is_none(),
            "what is not an archive has no index"
        );
    }

    #[test]
    fn a_targeted_read_is_bounded_by_the_same_rails() {
        let archive = zip_with(&[("bomb", &vec![0u8; 1024 * 1024])]);
        let budget = ExtractBudget {
            total_bytes_max: 8 * 1024 * 1024,
            ratio_max: 50,
        };
        let mut spent = 0u64;
        assert!(
            matches!(
                entry(&archive, "bomb", budget, &mut spent),
                Err(ArchiveError::RatioTooLarge { .. })
            ),
            "naming one entry does not exempt it from the bomb check"
        );
        assert!(
            spent > 0,
            "and the inflation it did before the refusal is charged, not forgiven"
        );
    }

    /// The compression method code this fixture writes over a member's own:
    /// 42 is unassigned in the ZIP specification, so `CompressionMethod`
    /// parses it as `Unsupported(42)` and a decoder for it cannot be
    /// constructed.
    ///
    /// Deliberately not 99. That is the WinZip-AES marker, and a member
    /// declaring it is expected to carry an AES extra field as well — writing
    /// the code alone produces an archive the reader rejects while parsing
    /// the central directory, which is a malformed archive rather than the
    /// case under test.
    const UNSUPPORTED_METHOD: u16 = 42;

    /// Rewrites one member's compression method, in both the local header and
    /// the central directory, to a code no build of this decoder carries. The
    /// member stays indexable and becomes unopenable, which is the shape of an
    /// encrypted or exotically compressed entry a real bundle can hold.
    fn make_unopenable(archive: &mut [u8], name: &str) {
        let bytes = name.as_bytes();
        let mut at = 0usize;
        while at + 46 <= archive.len() {
            let (name_at, method_at) = match &archive[at..at + 4] {
                [b'P', b'K', 3, 4] => (at + 30, at + 8),
                [b'P', b'K', 1, 2] => (at + 46, at + 10),
                _ => {
                    at += 1;
                    continue;
                }
            };
            if archive[name_at..].starts_with(bytes) {
                archive[method_at..method_at + 2]
                    .copy_from_slice(&UNSUPPORTED_METHOD.to_le_bytes());
            }
            at += 4;
        }
    }

    #[test]
    fn a_member_this_build_cannot_open_does_not_deny_the_one_that_was_asked_for() {
        // The decoder is constructed for the chosen member and no other, so
        // an unopenable entry earlier in the archive is passed over. Before
        // that ordering, the same bundle answered differently depending on
        // which member happened to be written first.
        let mut archive = zip_with(&[
            ("locked.bin", b"whatever an encrypted member holds"),
            ("preview.png", b"the picture that was asked for"),
        ]);
        make_unopenable(&mut archive, "locked.bin");
        let mut spent = 0u64;
        let found = entry(
            &archive,
            "preview.png",
            ExtractBudget::default(),
            &mut spent,
        )
        .expect("a readable archive")
        .expect("the named entry");
        assert_eq!(
            found.bytes, b"the picture that was asked for",
            "an unrelated member nobody can open is not the requested one, and does not \
             decide its fate"
        );
        assert!(
            entry(&archive, "locked.bin", ExtractBudget::default(), &mut spent).is_err(),
            "and asking for that member itself is an error rather than a silent absence"
        );
    }

    #[test]
    fn a_stream_that_fails_late_is_charged_for_what_it_inflated() {
        // The hole this closes: a member whose data is corrupt inflates until
        // the decoder gives up, and if that work were credited back, a bundle
        // of such members could be inflated as many times as a caller cared
        // to ask. The counter is advanced as the bytes arrive, so a failed
        // read costs what it cost.
        let payload: Vec<u8> = (0..200_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        let mut archive = zip_with(&[("broken.png", &payload)]);
        // Corrupt the compressed stream a long way in — past where a decoder
        // has already produced most of the entry, and clear of the central
        // directory at the end, so the archive still indexes and the failure
        // is the member's data rather than its structure.
        let end = archive.len().saturating_sub(200);
        for byte in &mut archive[end - 1_000..end] {
            *byte ^= 0xFF;
        }

        let mut spent = 0u64;
        let read = entry(&archive, "broken.png", ExtractBudget::default(), &mut spent);
        assert!(
            read.is_err(),
            "a stream that does not survive its own checksum is an error, not bytes"
        );
        assert!(
            spent > 0,
            "and the inflation it did before failing is on the caller's running total: \
             crediting it back is what would make a bundle of broken members free to retry"
        );

        // The same total, offered a second time, is not a second free
        // inflation: a caller that has spent its budget stops asking.
        let before = spent;
        let budget = ExtractBudget {
            total_bytes_max: before,
            ratio_max: ExtractBudget::default().ratio_max,
        };
        let again = entry(&archive, "broken.png", budget, &mut spent);
        assert!(
            matches!(
                again,
                Err(ArchiveError::TotalTooLarge { .. } | ArchiveError::Malformed(_))
            ),
            "a second read against an exhausted total is refused rather than run again"
        );
    }

    #[test]
    fn one_unsafe_name_does_not_deny_a_read_of_a_safe_sibling() {
        // `extract` refuses the whole archive, because a partial extraction
        // to disk is the hazard there. A targeted read of a named entry has
        // no such half-state, so the escaping sibling is simply not it.
        let archive = zip_with(&[
            ("../escape.txt", b"escape"),
            ("preview.png", b"the sibling's bytes"),
        ]);
        assert!(
            matches!(
                extract(Cursor::new(archive.clone()), ExtractBudget::default()),
                Err(ArchiveError::UnsafePath { .. })
            ),
            "a whole extraction still refuses an archive holding an escaping name"
        );
        let mut spent = 0u64;
        let found = entry(
            &archive,
            "preview.png",
            ExtractBudget::default(),
            &mut spent,
        )
        .expect("a readable archive")
        .expect("the named entry");
        assert_eq!(found.bytes, b"the sibling's bytes");
        assert_eq!(
            file_paths(&archive).expect("an index"),
            vec!["preview.png"],
            "the index offers only names a targeted read may be given"
        );
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
