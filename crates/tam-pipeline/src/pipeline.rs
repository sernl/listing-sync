//! The ingest driver: probe the kind, scan for malware, extract a ZIP under
//! bounds, classify entries into roles, hash and store each blob, and
//! generate the cover the marketplace requires. Composed here from the pure
//! stages; the blob-storage sink is a caller-supplied async closure so this
//! crate does not depend on `tam-storage` (which depends on this one).

use crate::archive::{extract, ArchiveError, ExtractBudget};
use crate::probe::probe_kind;
use crate::render::{cover, RenderError, RenderedImage};
use crate::scan::Scanner;
use tam_types::{ContentHash, FileKind, FileRole, ScanOutcome, Timestamp};

/// One classified, stored file: the metadata the catalogue insert consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestedFile {
    pub hash: ContentHash,
    pub role: FileRole,
    pub kind: FileKind,
    pub byte_len: u64,
    pub scan: ScanOutcome,
}

/// The result of ingesting one upload: the payload files, the required cover,
/// and any previews.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ingested {
    pub payload: Vec<IngestedFile>,
    pub cover: IngestedFile,
    pub previews: Vec<IngestedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    /// The upload's kind is not one the pipeline accepts.
    UnknownKind,
    /// The scan flagged the content; the publish is blocked.
    Infected { signature: String },
    /// The archive failed a safety bound.
    Archive(ArchiveError),
    /// The cover could not be produced, which Tes requires.
    Cover(String),
    /// A blob could not be stored.
    Store(String),
    /// The upload carried no payload file.
    EmptyPayload,
}

impl core::fmt::Display for IngestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownKind => f.write_str("the upload's kind is not accepted"),
            Self::Infected { signature } => write!(f, "content flagged: {signature}"),
            Self::Archive(error) => write!(f, "archive: {error}"),
            Self::Cover(detail) => write!(f, "cover: {detail}"),
            Self::Store(detail) => write!(f, "store: {detail}"),
            Self::EmptyPayload => f.write_str("the upload carried no payload file"),
        }
    }
}

impl core::error::Error for IngestError {}

impl From<ArchiveError> for IngestError {
    fn from(error: ArchiveError) -> Self {
        Self::Archive(error)
    }
}

/// The extract budget and the wall-clock instant an ingest runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestContext {
    pub budget: ExtractBudget,
    pub now: Timestamp,
}

/// Stores one blob's bytes and returns its content hash. The pipeline calls
/// this for every file; the implementer (a `BlobRepo`) seals and deduplicates.
pub trait BlobSink {
    fn store(
        &self,
        bytes: Vec<u8>,
    ) -> impl core::future::Future<Output = Result<ContentHash, String>> + Send;
}

/// Ingests one upload. A single file is its own payload; a ZIP is extracted
/// and its entries classified by kind. The scan runs on the raw upload before
/// extraction, so a bomb inside a scanned-clean wrapper is still bounded by
/// the extractor. The cover is generated from the first payload and its
/// failure blocks the publish.
pub async fn ingest(
    upload: &[u8],
    scanner: &impl Scanner,
    sink: &impl BlobSink,
    ctx: IngestContext,
) -> Result<Ingested, IngestError> {
    let Some(kind) = probe_kind(upload) else {
        return Err(IngestError::UnknownKind);
    };
    if let ScanOutcome::Infected { signature } = scanner.scan(upload, ctx.now).await {
        return Err(IngestError::Infected { signature });
    }

    let files: Vec<(Vec<u8>, FileKind)> = if kind == FileKind::Zip {
        let mut classified = Vec::new();
        for entry in extract(std::io::Cursor::new(upload), ctx.budget)? {
            if let Some(entry_kind) = probe_kind(&entry.bytes) {
                classified.push((entry.bytes, entry_kind));
            }
        }
        classified
    } else {
        vec![(upload.to_vec(), kind)]
    };

    let mut payload = Vec::new();
    for (bytes, file_kind) in &files {
        let stored = store_file(sink, bytes, *file_kind, FileRole::Payload, ctx.now).await?;
        payload.push(stored);
    }
    if payload.is_empty() {
        return Err(IngestError::EmptyPayload);
    }

    // The cover is generated from the first payload's bytes and kind; its
    // failure blocks the publish, which is the Tes hard requirement.
    let (cover_bytes, cover_kind) = files.first().ok_or(IngestError::EmptyPayload)?;
    let rendered = cover(*cover_kind, cover_bytes)
        .map_err(|error: RenderError| IngestError::Cover(error.to_string()))?;
    let cover_file = store_render(sink, &rendered, FileRole::Cover, ctx.now).await?;

    Ok(Ingested {
        payload,
        cover: cover_file,
        previews: Vec::new(),
    })
}

async fn store_file(
    sink: &impl BlobSink,
    bytes: &[u8],
    kind: FileKind,
    role: FileRole,
    now: Timestamp,
) -> Result<IngestedFile, IngestError> {
    let hash = sink
        .store(bytes.to_vec())
        .await
        .map_err(IngestError::Store)?;
    Ok(IngestedFile {
        hash,
        role,
        kind,
        byte_len: bytes.len() as u64,
        scan: ScanOutcome::Clean { at: now },
    })
}

async fn store_render(
    sink: &impl BlobSink,
    rendered: &RenderedImage,
    role: FileRole,
    now: Timestamp,
) -> Result<IngestedFile, IngestError> {
    store_file(sink, &rendered.png, FileKind::Image, role, now).await
}

#[cfg(test)]
mod tests {
    use super::{ingest, BlobSink, IngestContext, IngestError};
    use crate::archive::ExtractBudget;
    use crate::hash::content_hash;
    use crate::scan::{AllowAllScanner, EicarScanner, EICAR};
    use std::io::{Cursor, Write};
    use tam_types::{ContentHash, FileRole, Timestamp};
    use zip::write::SimpleFileOptions;

    /// A sink that hashes but stores nothing, for pipeline-shape tests.
    struct HashOnlySink;

    impl BlobSink for HashOnlySink {
        fn store(
            &self,
            bytes: Vec<u8>,
        ) -> impl core::future::Future<Output = Result<ContentHash, String>> + Send {
            core::future::ready(Ok(content_hash(&bytes)))
        }
    }

    fn ctx() -> IngestContext {
        IngestContext {
            budget: ExtractBudget::default(),
            now: Timestamp(1),
        }
    }

    fn pdf() -> Vec<u8> {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        bytes.extend_from_slice(b"a small pdf body");
        bytes
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .expect("start");
            writer.write_all(bytes).expect("write");
        }
        writer.finish().expect("finish");
        buffer.into_inner()
    }

    #[test]
    fn a_single_pdf_ingests_with_a_generated_cover() {
        let ingested =
            futures::executor::block_on(ingest(&pdf(), &AllowAllScanner, &HashOnlySink, ctx()))
                .expect("a pdf ingests");
        assert_eq!(ingested.payload.len(), 1, "the pdf is its own payload");
        assert_eq!(
            ingested.cover.role,
            FileRole::Cover,
            "a cover was generated"
        );
    }

    #[test]
    fn a_zip_ingests_its_recognised_entries() {
        let archive = zip_of(&[
            ("doc.pdf", &pdf()),
            ("readme.txt", b"ignored, unrecognised"),
        ]);
        let ingested =
            futures::executor::block_on(ingest(&archive, &AllowAllScanner, &HashOnlySink, ctx()))
                .expect("the zip ingests");
        assert_eq!(
            ingested.payload.len(),
            1,
            "only the recognised entry became a payload file"
        );
    }

    #[test]
    fn an_infected_upload_is_blocked() {
        let mut infected = b"%PDF-1.7\n".to_vec();
        infected.extend_from_slice(EICAR);
        let blocked =
            futures::executor::block_on(ingest(&infected, &EicarScanner, &HashOnlySink, ctx()));
        assert!(
            matches!(blocked, Err(IngestError::Infected { .. })),
            "an infected upload blocks the publish before it is stored"
        );
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        let refused = futures::executor::block_on(ingest(
            b"not a known magic at all",
            &AllowAllScanner,
            &HashOnlySink,
            ctx(),
        ));
        assert_eq!(
            refused,
            Err(IngestError::UnknownKind),
            "an unknown kind is refused"
        );
    }
}
