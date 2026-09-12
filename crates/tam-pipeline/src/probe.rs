//! Magic-byte kind probing. A file's kind comes from its leading bytes and,
//! for the OOXML container, from an actual part inside it — never from its
//! declared name or a caller's claim. An unrecognised file returns `None`
//! rather than a guess, because a wrong kind ships the wrong upload flow.

use std::io::{Cursor, Read};

use tam_types::FileKind;

const PDF_MAGIC: &[u8] = b"%PDF-";
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];
/// The two signatures a GIF actually begins with. `GIF8` alone would admit
/// `GIF8` followed by anything, and a file this probe calls an image is one
/// the image-serving routes then have to name and a renderer has to decode.
/// The upload admits only GIF87a and GIF89a, because those are the two
/// signatures the blob route can name and serve.
const GIF87A_MAGIC: &[u8] = b"GIF87a";
const GIF89A_MAGIC: &[u8] = b"GIF89a";

#[must_use]
pub fn probe_kind(bytes: &[u8]) -> Option<FileKind> {
    if bytes.starts_with(PDF_MAGIC) {
        return Some(FileKind::Pdf);
    }
    if bytes.starts_with(PNG_MAGIC)
        || bytes.starts_with(JPEG_MAGIC)
        || bytes.starts_with(GIF87A_MAGIC)
        || bytes.starts_with(GIF89A_MAGIC)
    {
        return Some(FileKind::Image);
    }
    if bytes.starts_with(ZIP_MAGIC) {
        return probe_zip_container(bytes);
    }
    None
}

/// A ZIP container is a plain ZIP unless it carries an OOXML content-types
/// part naming the document kind. The part is small and near the front, so
/// reading the archive directory is cheap.
///
/// The directory is read to decide the kind and, before that, to decide
/// there is a file here at all. The four leading bytes open one entry's
/// local header; what makes the bytes an archive is the central directory at
/// the end, which a download cut short, a truncated blob store read or a
/// ranged response does not carry. Answering `Zip` on the magic alone let
/// those bytes be stored as a payload, rendered a placeholder cover and
/// reported as a file the seller owns — evidence of a file no reader can
/// open. An archive whose directory does not parse is therefore `None`, the
/// same answer this probe gives anything else it cannot recognise.
fn probe_zip_container(bytes: &[u8]) -> Option<FileKind> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let Ok(mut entry) = archive.by_name("[Content_Types].xml") else {
        return Some(FileKind::Zip);
    };
    let mut content_types = String::new();
    if entry.read_to_string(&mut content_types).is_err() {
        return Some(FileKind::Zip);
    }
    if content_types.contains("presentationml") {
        Some(FileKind::Pptx)
    } else if content_types.contains("wordprocessingml") {
        Some(FileKind::Docx)
    } else {
        Some(FileKind::Zip)
    }
}

#[cfg(test)]
mod tests {
    use super::probe_kind;
    use std::io::{Cursor, Write};
    use tam_types::FileKind;
    use zip::write::SimpleFileOptions;

    fn ooxml(content_type_marker: &str) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = SimpleFileOptions::default();
        writer
            .start_file("[Content_Types].xml", options)
            .expect("start content types");
        writer
            .write_all(format!("<Types>{content_type_marker}</Types>").as_bytes())
            .expect("write content types");
        writer.start_file("body", options).expect("start body");
        writer.write_all(b"x").expect("write body");
        writer.finish().expect("finish");
        buffer.into_inner()
    }

    fn plain_zip() -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        writer
            .start_file("a.txt", SimpleFileOptions::default())
            .expect("start");
        writer.write_all(b"plain").expect("write");
        writer.finish().expect("finish");
        buffer.into_inner()
    }

    #[test]
    fn a_pdf_probes_by_its_magic() {
        assert_eq!(probe_kind(b"%PDF-1.7\nrest"), Some(FileKind::Pdf));
    }

    #[test]
    fn a_png_probes_as_image() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        assert_eq!(probe_kind(&png), Some(FileKind::Image));
    }

    #[test]
    fn an_ooxml_presentation_probes_as_pptx() {
        assert_eq!(
            probe_kind(&ooxml(
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            )),
            Some(FileKind::Pptx),
            "the content-types part names the document kind, not the file name"
        );
    }

    #[test]
    fn an_ooxml_document_probes_as_docx() {
        assert_eq!(
            probe_kind(&ooxml(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            )),
            Some(FileKind::Docx)
        );
    }

    #[test]
    fn a_plain_zip_probes_as_zip_not_a_document() {
        assert_eq!(probe_kind(&plain_zip()), Some(FileKind::Zip));
    }

    /// An archive whose directory never arrived is not a file this pipeline
    /// recognises. The four leading bytes are a claim; the central directory
    /// is what makes the claim true, and a download cut short keeps the
    /// first and loses the second. Calling that a Zip stored a payload no
    /// reader can open and minted evidence that a file exists.
    #[test]
    fn a_structurally_truncated_zip_is_not_a_recognised_file() {
        let whole = plain_zip();
        let truncated = whole
            .get(..16)
            .expect("the ZIP fixture contains its local-file header");
        assert_eq!(probe_kind(truncated), None);
    }

    #[test]
    fn both_real_gif_signatures_probe_as_image() {
        assert_eq!(probe_kind(b"GIF87a\x08\x00"), Some(FileKind::Image));
        assert_eq!(probe_kind(b"GIF89a\x08\x00"), Some(FileKind::Image));
    }

    /// A four-byte `GIF8` prefix is not a GIF, and admitting one stored a file
    /// that reported as an image and then could not be served or drawn.
    #[test]
    fn a_truncated_gif_signature_is_not_an_image() {
        assert_eq!(probe_kind(b"GIF8ZZ\x08\x00"), None);
        assert_eq!(probe_kind(b"GIF8"), None);
    }

    #[test]
    fn an_unrecognised_file_is_none_never_a_guess() {
        assert_eq!(probe_kind(b"not a known magic"), None);
    }
}
