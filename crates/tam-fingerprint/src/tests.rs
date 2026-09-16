//! What the sketches have to survive, and what they have to separate.
//!
//! The two PDFs below are generated here rather than committed, because the
//! property under test is about the *bytes* differing while the text does
//! not: a fixture pair would prove the generator's determinism instead. Each
//! carries the same page text through a different object order, a different
//! object numbering and different metadata, which is what a marketplace's own
//! re-export does to a file a seller uploaded once.

use super::*;

/// Two paragraphs of ordinary worksheet prose, long enough to yield a few
/// hundred shingles, which is where the MinHash estimator is worth reading.
const LESSON: &str = "Fractions on a number line. Draw a number line from zero to one and \
mark the halves, then the quarters, then the eighths. Ask your pupils which of the marks fall \
on top of one another and why. A half is the same point as two quarters and as four eighths, \
and seeing that on the line is the whole of equivalence. Then hide the labels and ask them to \
place three eighths without counting from zero. The second sheet repeats the task from zero to \
two, so a pupil who has learned the pattern on one interval has to decide whether it still \
holds when the whole is longer. Finish by asking each pupil to invent one question for the \
person beside them, and to write down the answer before they hand it over.";

/// The same words with the sentences in a different order and two of them
/// reworded, which is a re-export that regenerated the text layer rather than
/// copying it.
const LESSON_REFLOWED: &str = "Fractions on a number line. Draw a number line from zero to one \
and mark the halves, then the quarters, then the eighths. Ask your pupils which of the marks \
fall on top of one another and why. A half is the same point as two quarters and as four \
eighths, and seeing that on the line is the whole of equivalence. Then hide the labels and ask \
them to place three eighths without counting from zero. The second sheet repeats the task from \
zero to two, so a pupil who has learned the pattern on one interval has to decide whether it \
still holds when the whole is longer. Finish by asking each pupil to write one question for \
their neighbour, and to note the answer before handing it over.";

const UNRELATED: &str = "Photosynthesis in the school garden. Over four weeks the class keeps \
two trays of cress, one on the windowsill and one in the cupboard, and measures the height of \
five seedlings in each tray every Monday. Record the measurements in the table and plot them on \
the grid. Ask which variable the class changed and which ones it had to hold still, and write \
the conclusion as a sentence that mentions light, water and growth. The extension asks a pupil \
to design a third tray that would test whether the colour of the light matters, and to say what \
they would measure.";

/// A PDF carrying one page of the given text, written with the objects in the
/// order the caller names.
///
/// `lopdf` rather than a hand-assembled byte string, because the point is a
/// file a real reader accepts: a test that asserted over bytes no extractor
/// could read would pass while proving nothing.
fn pdf_of(text: &str, reverse_objects: bool, producer: &str) -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::dictionary;
    use lopdf::{Document, Object, Stream};

    let mut doc = Document::with_version("1.5");
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    // One Tj per line, because a PDF text object has no wrapping of its own
    // and a single enormous string would extract as one line either way.
    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
    ];
    for (line, chunk) in text.as_bytes().chunks(70).enumerate() {
        operations.push(Operation::new(
            "Td",
            vec![
                20.into(),
                (760 - i64::try_from(line).unwrap_or(0) * 14).into(),
            ],
        ));
        operations.push(Operation::new(
            "Tj",
            vec![Object::string_literal(chunk.to_vec())],
        ));
    }
    operations.push(Operation::new("ET", vec![]));

    let content = Content { operations };
    let content_id = doc.add_object(Stream::new(
        dictionary! {},
        content.encode().unwrap_or_default(),
    ));
    let tree_id = doc.new_object_id().0;
    let leaf = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => (tree_id, 0),
        "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    });
    doc.objects.insert(
        (tree_id, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![leaf.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => (tree_id, 0),
    });
    let info_id = doc.add_object(dictionary! {
        "Producer" => Object::string_literal(producer),
        "Title" => Object::string_literal(producer),
    });
    doc.trailer.set("Root", catalog_id);
    doc.trailer.set("Info", info_id);

    if reverse_objects {
        // Renumber every object in reverse, which changes the byte layout,
        // the cross-reference table and every internal reference while
        // leaving the page identical.
        doc.renumber_objects_with(1000);
    }
    doc.compress();
    let mut bytes = Vec::new();
    drop(doc.save_to(&mut bytes));
    bytes
}

/// A one-page PDF with no text operators at all: the scanned worksheet whose
/// text layer is empty rather than unreadable.
fn image_only_pdf() -> Vec<u8> {
    use lopdf::dictionary;
    use lopdf::{Document, Object, Stream};

    let mut doc = Document::with_version("1.5");
    let content_id = doc.add_object(Stream::new(dictionary! {}, b"0 0 1 rg\n".to_vec()));
    let tree_id = doc.new_object_id().0;
    let leaf = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => (tree_id, 0),
        "Contents" => content_id,
        "Resources" => dictionary! {},
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    });
    doc.objects.insert(
        (tree_id, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![leaf.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => (tree_id, 0),
    });
    doc.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    drop(doc.save_to(&mut bytes));
    bytes
}

fn text_of(print: &Fingerprint) -> &TextSketch {
    match print.text.as_ref() {
        Some(sketch) => sketch,
        None => panic!("the fingerprint carries no text layer"),
    }
}

/// The layer's whole purpose: a marketplace's re-export of one seller's file
/// is two different files and one document.
#[test]
fn two_re_exports_of_one_pdf_sketch_the_same_document() {
    let first = pdf_of(LESSON, false, "Tes export 2026-01");
    let second = pdf_of(LESSON, true, "TPT export 2026-06");
    assert_ne!(first, second, "the two exports must differ byte for byte");

    let one = fingerprint(FileKind::Pdf, &first, "Fractions worksheet", None);
    let other = fingerprint(FileKind::Pdf, &second, "Fractions worksheet | TPT", None);
    let (one, other) = (text_of(&one), text_of(&other));

    let distance = hamming(one.simhash, other.simhash);
    assert!(
        distance <= 3,
        "two re-exports must block together: Hamming {distance}"
    );
    let jaccard = minhash_jaccard(&one.minhash, &other.minhash);
    assert!(
        jaccard >= 0.9,
        "two re-exports must score strong: Jaccard {jaccard}"
    );
}

/// A re-export that regenerated two sentences out of nine lands in the
/// moderate band rather than the strong one, and that is the layer behaving
/// correctly rather than failing: 0.9 is the auto-merge threshold, and a
/// listing whose prose has actually changed is a pair to ask about.
#[test]
fn a_reworded_re_export_scores_moderate() {
    let one = sketch_of(LESSON);
    let other = sketch_of(LESSON_REFLOWED);
    let jaccard = minhash_jaccard(&one.minhash, &other.minhash);
    assert!(
        (0.6..0.9).contains(&jaccard),
        "two sentences reworded out of nine is a moderate match: Jaccard {jaccard}"
    );
}

/// The other half of the same claim, which is the one that stops a merge: two
/// worksheets by one seller, on different topics, must not look alike.
#[test]
fn two_unrelated_texts_score_apart() {
    let one = sketch_of(LESSON);
    let other = sketch_of(UNRELATED);
    let jaccard = minhash_jaccard(&one.minhash, &other.minhash);
    assert!(
        jaccard < 0.2,
        "unrelated worksheets must not score: Jaccard {jaccard}"
    );
}

/// An empty text layer is a fact the matcher reads, not an absent one.
#[test]
fn an_image_only_pdf_reports_no_characters_rather_than_no_text_layer() {
    let print = fingerprint(FileKind::Pdf, &image_only_pdf(), "Scanned worksheet", None);
    let sketch = text_of(&print);
    assert_eq!(
        sketch.extracted_chars, 0,
        "a page with no text operators extracts nothing"
    );
    assert_eq!(sketch.shingle_count, 0, "and yields no shingles");
    assert_eq!(
        print.page_count,
        Some(1),
        "but its page tree is still readable"
    );
}

/// A kind that is not a PDF is not read for text, whatever its bytes are.
#[test]
fn a_non_pdf_carries_no_text_layer() {
    let print = fingerprint(FileKind::Zip, b"PK\x03\x04 not a pdf", "Bundle", None);
    assert_eq!(print.text, None, "a zip has no PDF text layer");
    assert_eq!(print.page_count, None, "and no page tree");
    assert_eq!(print.cover_phash, None, "and L3 is for image payloads only");
}

/// The bytes of a PDF handed to a reader that is not a PDF reader: the
/// extraction fails and the absence is reported rather than faked.
#[test]
fn bytes_that_are_not_a_pdf_under_a_pdf_kind_report_an_absent_text_layer() {
    let print = fingerprint(
        FileKind::Pdf,
        b"this is not a pdf at all",
        "Worksheet",
        None,
    );
    assert_eq!(print.text, None);
    assert_eq!(print.page_count, None);
}

#[test]
fn deeply_nested_pdf_preserves_the_import_process() {
    // RUSTSEC-2026-0187: construct reader input without recursing through a PDF writer.
    let mut bytes = b"%PDF-1.5\n".to_vec();
    let catalog_offset = bytes.len();
    bytes.extend_from_slice(
        format!(
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /X {}0{} >>\nendobj\n",
            "[".repeat(10_380),
            "]".repeat(10_380),
        )
        .as_bytes(),
    );
    let pages_offset = bytes.len();
    bytes.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    let xref_offset = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n0 3\n0000000000 65535 f \n{catalog_offset:010} 00000 n \n\
             {pages_offset:010} 00000 n \ntrailer\n<< /Size 3 /Root 1 0 R >>\n\
             startxref\n{xref_offset}\n%%EOF\n"
        )
        .as_bytes(),
    );

    let rejected = fingerprint(FileKind::Pdf, &bytes, "Nested worksheet", None);
    assert_eq!(
        rejected.title_norm, "nested worksheet",
        "an unreadable PDF must not discard the resource's metadata"
    );
    let next = fingerprint(
        FileKind::Pdf,
        &pdf_of(LESSON, false, "Ordinary worksheet"),
        "Ordinary worksheet",
        None,
    );
    assert_eq!(
        next.page_count,
        Some(1),
        "the next resource must remain readable after rejecting the hostile PDF"
    );
}

/// L3 fires for an image payload and for nothing else.
#[test]
fn an_image_payload_hashes_its_cover() {
    let png = one_by_one_png();
    let print = fingerprint(FileKind::Image, &png, "A poster", Some(&png));
    assert!(
        print.cover_phash.is_some(),
        "an image payload carries a cover hash"
    );
    let as_pdf = fingerprint(FileKind::Pdf, &png, "A poster", Some(&png));
    assert_eq!(
        as_pdf.cover_phash, None,
        "a placeholder card is not a cover worth hashing"
    );
}

/// A 1x1 white PNG, written out rather than generated, so the test needs no
/// encoder.
fn one_by_one_png() -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
        .unwrap_or_default()
}

#[test]
fn a_title_loses_its_marketplace_suffix_and_nothing_else() {
    assert_eq!(
        normalise_title("Fractions on a Number Line | TPT"),
        "fractions on a number line"
    );
    assert_eq!(
        normalise_title("Fractions on a Number Line - TPT"),
        "fractions on a number line"
    );
    assert_eq!(
        normalise_title("Fractions on a Number Line (Tes)"),
        "fractions on a number line"
    );
    // The words the design names as kept: they are the seller's own and two
    // listings differing by one of them are plausibly two products.
    assert_eq!(
        normalise_title("Fractions BUNDLE — Distance Learning, No Prep, Printable"),
        "fractions bundle distance learning no prep printable"
    );
    // "tpt" on its own is a word in the title, not a suffix the marketplace
    // appended, so it stays.
    assert_eq!(normalise_title("How I use TPT"), "how i use tpt");
    assert_eq!(
        normalise_title("   spaced\tout\n title  "),
        "spaced out title"
    );
    assert_eq!(
        normalise_title("***"),
        "",
        "a title of punctuation is empty"
    );
}

#[test]
fn a_normalised_title_fits_the_column_that_stores_it() {
    let long = "worksheet ".repeat(200);
    let normalised = normalise_title(&long);
    assert!(normalised.chars().count() <= TITLE_NORM_MAX);
}

#[test]
fn the_bands_are_the_hash_and_nothing_else() {
    let hash = 0xDEAD_BEEF_1234_5678u64;
    let bands = simhash_bands(hash);
    assert_eq!(bands, [0x5678, 0x1234, 0xBEEF, 0xDEAD]);
    let rebuilt = u64::from(bands[0])
        | u64::from(bands[1]) << 16
        | u64::from(bands[2]) << 32
        | u64::from(bands[3]) << 48;
    assert_eq!(rebuilt, hash, "the four bands are the whole hash");
}

/// Manku's premise, which the four band indexes depend on: two hashes within
/// three bits must agree on at least one band.
#[test]
fn hashes_within_three_bits_share_a_band() {
    let base = 0x0123_4567_89AB_CDEFu64;
    for bits in [0x1u64, 0x8000_0000u64, 0x3, 0x1_0000_0000_0000, 0x7] {
        let near = base ^ bits;
        assert!(hamming(base, near) <= 3);
        let (left, right) = (simhash_bands(base), simhash_bands(near));
        assert!(
            left.iter()
                .zip(right.iter())
                .any(|(one, other)| one == other),
            "a three-bit difference must leave one band intact"
        );
    }
}

#[test]
fn a_sketch_survives_the_wire() {
    let sketch = sketch_of(LESSON);
    let print = Fingerprint {
        version: FINGERPRINT_VERSION,
        text: Some(sketch),
        page_count: Some(4),
        cover_phash: Some(0xFFFF_0000_FFFF_0000),
        title_norm: normalise_title("Fractions on a Number Line | TPT"),
    };
    let encoded = serde_json::to_string(&print).unwrap_or_default();
    let decoded: Fingerprint = match serde_json::from_str(&encoded) {
        Ok(decoded) => decoded,
        Err(why) => panic!("a fingerprint must round trip: {why}"),
    };
    assert_eq!(decoded, print);
    // And the minhash is exactly 512 bytes of base64 rather than an array of
    // numbers, because the column it lands in is a `bytea` of that width.
    let value: serde_json::Value = match serde_json::from_str(&encoded) {
        Ok(value) => value,
        Err(why) => panic!("the encoding is JSON: {why}"),
    };
    let wire = value
        .pointer("/text/minhash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    assert_eq!(
        wire.len(),
        684,
        "512 bytes is 684 characters of base64, one padding character"
    );
}

#[test]
fn the_minhash_bytes_round_trip() {
    let sketch = sketch_of(LESSON).minhash;
    let bytes = minhash_bytes(&sketch);
    assert_eq!(bytes.len(), MINHASH_BYTES);
    assert_eq!(minhash_from_bytes(&bytes), Some(sketch));
    assert_eq!(
        minhash_from_bytes(&bytes[..511]),
        None,
        "a short sketch is not a sketch"
    );
}

#[test]
fn a_source_with_no_file_asserts_its_title_and_four_absences() {
    let print = Fingerprint::of_title("Fractions on a Number Line | TPT");
    assert_eq!(print.title_norm, "fractions on a number line");
    assert_eq!(print.text, None);
    assert_eq!(print.page_count, None);
    assert_eq!(print.cover_phash, None);
    assert_eq!(print.version, FINGERPRINT_VERSION);
}

#[test]
fn titles_score_by_their_words() {
    assert!(
        token_jaccard(
            "Fractions on a number line",
            "Fractions on a Number Line | TPT"
        ) > 0.99
    );
    assert!(token_jaccard("Fractions worksheet pack", "Photosynthesis lab sheet") < 0.2);
}
