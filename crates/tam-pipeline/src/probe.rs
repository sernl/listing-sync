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
const GIF_MAGIC: &[u8] = b"GIF8";

#[must_use]
pub fn probe_kind(bytes: &[u8]) -> Option<FileKind> {
    if bytes.starts_with(PDF_MAGIC) {
        return Some(FileKind::Pdf);
    }
    if bytes.starts_with(PNG_MAGIC) || bytes.starts_with(JPEG_MAGIC) || bytes.starts_with(GIF_MAGIC)
    {
        return Some(FileKind::Image);
    }
    if bytes.starts_with(ZIP_MAGIC) {
        return Some(probe_zip_container(bytes));
    }
    None
}

/// A ZIP container is a plain ZIP unless it carries an OOXML content-types
/// part naming the document kind. The part is small and near the front, so
/// reading the archive directory is cheap.
fn probe_zip_container(bytes: &[u8]) -> FileKind {
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(bytes)) else {
        return FileKind::Zip;
    };
    let Ok(mut entry) = archive.by_name("[Content_Types].xml") else {
        return FileKind::Zip;
    };
    let mut content_types = String::new();
    if entry.read_to_string(&mut content_types).is_err() {
        return FileKind::Zip;
    }
    if content_types.contains("presentationml") {
        FileKind::Pptx
    } else if content_types.contains("wordprocessingml") {
        FileKind::Docx
    } else {
        FileKind::Zip
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

    #[test]
    fn an_unrecognised_file_is_none_never_a_guess() {
        assert_eq!(probe_kind(b"not a known magic"), None);
    }
}
