//! Compare a freshly captured marketplace vocabulary against the committed
//! one and report what moved, writing the report under
//! `docs/design/data/drift/` and failing the run when an operator must act.
//!
//! This binary contacts nothing, and that is the point rather than an
//! omission. The diff is pure and runs server-side on a cron-shaped schedule;
//! the re-capture that produces its second input is a marketplace request,
//! and D1 puts that on the seller's own device for a marketplace with no
//! official API. So the fresh capture arrives here as a file that something
//! else obtained — uploaded from a seller's device for TPT and Tes, or
//! fetched server-side under an issued token for an API-branch marketplace —
//! and this crate never learns which.
//!
//! It is modelled on `tam-standards-fetch`, which is the tree's precedent for
//! a thin binary around a committed capture, and differs from it in exactly
//! the way that matters: that one holds `reqwest` and touches one host, and
//! this one holds no HTTP client at all, which the purity gate asserts.
//!
//! Usage:
//!
//! ```text
//! tam-vocab-drift <inventory> <committed.json> <fresh.json> <out-dir> <captured-at>
//! ```
//!
//! The timestamp is an argument rather than a clock read, for the same reason
//! `tam-standards-fetch` takes one: time enters as data, so two runs over the
//! same pair of captures produce byte-identical reports and a committed drift
//! file can be compared rather than merely read.
//!
//! The exit status is the alerting surface, as it is for `tam-canary`: zero
//! when nothing structural moved, one when something did. A relabelled value
//! with a stable identifier is written to the report and does not fail the
//! run, because labels are read out of the captures rather than stored beside
//! the terms.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use std::io::Read as _;

use tam_taxonomy::drift::diff;

/// Reads one committed capture through a handle rather than through
/// `std::fs::read_to_string`, which the workspace disallows because that
/// method is also how an unbounded upload gets slurped whole. These captures
/// are a hundred kilobytes and committed, so the shape is the convention
/// rather than the constraint, and it matches `tam-taxonomy-seed`.
fn read_file(path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut text = String::new();
    std::fs::File::open(path)?.read_to_string(&mut text)?;
    Ok(text)
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [inventory, committed_path, fresh_path, out_dir, captured_at] = &arguments[..] else {
        return Err(
            "usage: tam-vocab-drift <inventory> <committed.json> <fresh.json> \
                    <out-dir> <captured-at>"
                .into(),
        );
    };

    let committed = read_file(committed_path)?;
    let fresh = read_file(fresh_path)?;
    let report = diff(&committed, &fresh)?;

    std::fs::create_dir_all(out_dir)?;
    let path = std::path::Path::new(out_dir).join(format!("{inventory}-{captured_at}.json"));
    let mut rendered = serde_json::to_string_pretty(&report)?;
    rendered.push('\n');
    std::fs::write(&path, rendered)?;

    eprintln!(
        "{inventory}: {} option sets compared, {} rows, {} of them blocking; report at {}",
        report.sets_compared.len(),
        report.rows.len(),
        report.blocking().count(),
        path.display(),
    );
    for row in report.blocking() {
        eprintln!("{inventory}: {:?} {}/{}", row.kind, row.set, row.native_id);
    }
    Ok(report.is_clean())
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("tam-vocab-drift: {error}");
            ExitCode::FAILURE
        }
    }
}
