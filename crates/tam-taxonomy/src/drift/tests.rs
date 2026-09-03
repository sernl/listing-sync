//! The drift diff's tests, over the committed captures and over mutated
//! copies of them, with no I/O beyond the `include_str!` the other pure tests
//! already use.

use super::{diff, DriftKind};

const TPT: &str = include_str!("../../../../docs/design/data/tpt-vocabulary.json");
const TES: &str = include_str!("../../../../docs/design/data/tes-vocabulary.json");

/// Removes one option from a capture by native id, which is what a facet the
/// platform has retired looks like on the wire.
fn without(capture: &str, set: &str, native_id: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(capture).expect("the capture parses");
    let options = value
        .get_mut(set)
        .and_then(|set| set.get_mut("options"))
        .and_then(serde_json::Value::as_object_mut)
        .expect("the set exists");
    assert!(
        options.remove(native_id).is_some(),
        "{native_id} is in {set} to remove"
    );
    value.to_string()
}

fn changed(capture: &str, set: &str, native_id: &str, field: &str, to: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(capture).expect("the capture parses");
    let option = value
        .get_mut(set)
        .and_then(|set| set.get_mut("options"))
        .and_then(|options| options.get_mut(native_id))
        .expect("the option exists");
    option[field] = serde_json::Value::String(to.to_owned());
    value.to_string()
}

/// A capture against itself is the case that must be silent, and it is the
/// one a diff gets wrong by being too eager: every one of the 597 TPT options
/// and 104 Tes options compares equal, so the cron run says nothing on the
/// day nothing happened.
#[test]
fn a_capture_against_itself_reports_nothing() {
    for capture in [TPT, TES] {
        let report = diff(capture, capture).expect("the committed captures diff");
        assert!(report.rows.is_empty(), "{:?}", report.rows);
        assert!(report.is_clean());
    }
}

/// The sets are discovered rather than listed, so the report names what it
/// looked at. These are the counts the captures state.
#[test]
fn every_option_set_in_both_captures_is_compared() {
    let tpt = diff(TPT, TPT).expect("the TPT capture diffs");
    assert_eq!(
        tpt.sets_compared.len(),
        10,
        "the TPT capture carries ten option sets: {:?}",
        tpt.sets_compared
    );
    assert!(tpt.sets_compared.contains(&"taxonomyTags".to_owned()));
    let tes = diff(TES, TES).expect("the Tes capture diffs");
    assert_eq!(
        tes.sets_compared.len(),
        6,
        "the Tes capture carries six option sets: {:?}",
        tes.sets_compared
    );
    assert!(tes.sets_compared.contains(&"resourceTypes".to_owned()));
}

/// The step's named verification: one facet deleted from a copy yields one
/// row and a run that does not pass. It fails if the diff reports clean.
#[test]
fn one_deleted_facet_is_one_blocking_row() {
    let fresh = without(TPT, "taxonomyTags", "escape-rooms");
    let report = diff(TPT, &fresh).expect("the captures diff");
    let [row] = &report.rows[..] else {
        panic!("one row, not {:?}", report.rows);
    };
    assert_eq!(row.set, "taxonomyTags");
    assert_eq!(row.native_id, "escape-rooms");
    assert_eq!(row.kind, DriftKind::Removed);
    assert!(row.committed.is_some(), "the row carries what was lost");
    assert!(row.fresh.is_none());
    assert!(
        !report.is_clean(),
        "a retired facet leaves edges pointing at an identifier no capture holds"
    );
    assert_eq!(report.blocking().count(), 1);
}

/// The same deletion read in the other direction is an addition, which is the
/// operator question rather than the broken edge.
#[test]
fn a_facet_the_platform_has_added_is_a_blocking_row() {
    let committed = without(TPT, "taxonomyTags", "escape-rooms");
    let report = diff(&committed, TPT).expect("the captures diff");
    let [row] = &report.rows[..] else {
        panic!("one row, not {:?}", report.rows);
    };
    assert_eq!(row.kind, DriftKind::Added);
    assert!(row.committed.is_none());
    assert!(row.fresh.is_some(), "the row carries what appeared");
    assert!(!report.is_clean());
}

/// A relabelled value with a stable identifier touches nothing structural,
/// because the derivations read labels out of the capture rather than storing
/// them beside the terms. It is reported and it does not block.
#[test]
fn a_relabelled_option_is_reported_and_does_not_block() {
    let fresh = changed(TPT, "taxonomyTags", "escape-rooms", "name", "Escape Room");
    let report = diff(TPT, &fresh).expect("the captures diff");
    let [row] = &report.rows[..] else {
        panic!("one row, not {:?}", report.rows);
    };
    assert_eq!(row.kind, DriftKind::Relabelled);
    assert!(
        report.is_clean(),
        "a label is not structural, so the cron run passes and still records the change"
    );
    assert_eq!(report.blocking().count(), 0);
}

/// Re-parenting is not relabelling. A facet that keeps its identifier and
/// moves under a different root changes what the derivation builds, because
/// a child is paired inside its root's denotation.
#[test]
fn a_reparented_facet_blocks_where_a_relabelled_one_does_not() {
    let fresh = changed(
        TPT,
        "taxonomyTags",
        "escape-rooms",
        "parentId",
        "instruction",
    );
    let report = diff(TPT, &fresh).expect("the captures diff");
    let [row] = &report.rows[..] else {
        panic!("one row, not {:?}", report.rows);
    };
    assert_eq!(row.kind, DriftKind::Restructured);
    assert!(!report.is_clean());
}

/// A facet moving between categories moves between axes, which is the same
/// severity for a different reason: a subject that becomes a resource type is
/// read by a different derivation entirely.
#[test]
fn a_recategorised_facet_blocks() {
    let fresh = changed(
        TPT,
        "taxonomyTags",
        "escape-rooms",
        "category",
        "PreK-12-Subject-Area",
    );
    let report = diff(TPT, &fresh).expect("the captures diff");
    let [row] = &report.rows[..] else {
        panic!("one row, not {:?}", report.rows);
    };
    assert_eq!(row.kind, DriftKind::Restructured);
    assert!(!report.is_clean());
}

/// The Tes capture's options are bare strings rather than objects, so the
/// structural comparison must hold for a shape that carries neither
/// `parentId` nor `category` rather than treating every change as a move.
#[test]
fn a_shape_with_no_structural_fields_relabels_rather_than_restructures() {
    let mut value: serde_json::Value = serde_json::from_str(TES).expect("the capture parses");
    value["resourceTypes"]["options"]["99001"] = serde_json::Value::String("Assemblies".to_owned());
    let report = diff(TES, &value.to_string()).expect("the captures diff");
    let [row] = &report.rows[..] else {
        panic!("one row, not {:?}", report.rows);
    };
    assert_eq!(row.set, "resourceTypes");
    assert_eq!(row.kind, DriftKind::Relabelled);
    assert!(report.is_clean());
}

/// Two runs over the same pair produce the same report, which is what lets a
/// committed drift file be compared rather than merely read.
#[test]
fn the_diff_is_reproducible() {
    let fresh = without(TPT, "taxonomyTags", "escape-rooms");
    assert_eq!(
        diff(TPT, &fresh).expect("the first run"),
        diff(TPT, &fresh).expect("the second run")
    );
}
