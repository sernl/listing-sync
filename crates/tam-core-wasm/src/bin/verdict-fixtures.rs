//! Records what the native path decides for each fixture draft.
//!
//! The same shape `tam-api` uses for `vocab.ts`: the generator prints and the
//! lane diffs, so a rule that changed shows up as a diff in a committed file
//! rather than as two implementations quietly disagreeing.
//!
//! `just web-wasm-fixtures` writes `crates/tam-core-wasm/fixtures/verdicts.json`
//! from this.

use std::collections::BTreeMap;

use tam_authoring::{verdict, DraftInput};

const DRAFTS: &str = include_str!("../../fixtures/drafts.json");

fn main() {
    let Ok(drafts) = serde_json::from_str::<BTreeMap<String, DraftInput>>(DRAFTS) else {
        eprintln!("the fixture drafts do not parse");
        return;
    };
    let decided: BTreeMap<String, _> = drafts
        .into_iter()
        .map(|(name, draft)| (name, verdict(&draft)))
        .collect();
    match serde_json::to_string_pretty(&decided) {
        Ok(rendered) => println!("{rendered}"),
        Err(failure) => eprintln!("the verdicts do not encode: {failure}"),
    }
}
