//! The condition that makes this crate worth having: the browser's answer and
//! the server's answer are the same answer.
//!
//! `verdicts.json` is recorded from the native path — the one
//! `POST /v1/authoring/check` runs — and the vitest suite beside the loader
//! replays the identical drafts through the compiled module and compares
//! against the same file. Neither side can drift without one of the two tests
//! failing, and a rule that changed on purpose is a visible diff in a
//! committed file rather than a silent divergence between two implementations.

use std::collections::BTreeMap;

use tam_authoring::{verdict, DraftInput};

const DRAFTS: &str = include_str!("../fixtures/drafts.json");
const VERDICTS: &str = include_str!("../fixtures/verdicts.json");

/// Every fixture draft as the native path decides it, keyed by fixture name.
///
/// `BTreeMap` rather than a `HashMap`, because the recorded file is compared
/// as a whole and an iteration order that varied per run would make the
/// comparison meaningless. Nothing here unwraps: a fixture that does not parse
/// yields an empty map or a null verdict, and both fail the assertions below
/// with the difference on screen rather than a backtrace.
fn recorded() -> BTreeMap<String, serde_json::Value> {
    let drafts: BTreeMap<String, DraftInput> = serde_json::from_str(DRAFTS).unwrap_or_default();
    assert!(
        !drafts.is_empty(),
        "fixtures/drafts.json parses into at least one draft"
    );
    drafts
        .into_iter()
        .map(|(name, draft)| {
            let view = serde_json::to_value(verdict(&draft)).unwrap_or_default();
            (name, view)
        })
        .collect()
}

/// The recorded file is what the native path actually decides.
///
/// Regenerate with `just web-wasm-fixtures` when a rule changes on purpose.
/// The vitest suite beside the loader replays the same drafts through the
/// compiled module against the same file, so this failing and that one passing
/// cannot both happen.
#[test]
fn the_recorded_verdicts_are_the_ones_the_server_reaches() {
    let held: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(VERDICTS).unwrap_or_default();
    assert_eq!(
        held,
        recorded(),
        "fixtures/verdicts.json no longer matches what tam_authoring::verdict decides; \
         regenerate it if the rule changed on purpose"
    );
}

/// The fixture set covers both outcomes rather than only the happy path, so a
/// comparison that passed vacuously would be visible here.
#[test]
fn the_fixtures_exercise_both_a_submittable_draft_and_a_refused_one() {
    let decided = recorded();
    let submittable = decided
        .values()
        .filter(|view| view["submittable"] == serde_json::Value::Bool(true))
        .count();
    assert!(
        submittable > 0 && submittable < decided.len(),
        "an equivalence proof over drafts that all agree trivially proves nothing"
    );
}
