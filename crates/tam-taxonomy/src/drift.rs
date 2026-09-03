//! The vocabulary drift diff: what a fresh capture says that the committed
//! one does not.
//!
//! Pure, and deliberately so. The diff runs server-side on a cron-shaped
//! schedule and contacts nothing; the re-capture that produces its second
//! input is a marketplace request, and D1 puts that on the seller's own
//! device for a no-API marketplace. Keeping the comparison in the pure core
//! is what stops the alerting surface quietly becoming the thing that issues
//! the request.
//!
//! Option sets are discovered rather than listed: any top-level key whose
//! value is an object carrying an `options` object is one. That covers all
//! ten sets in the TPT capture and all six in the Tes capture without a
//! table to maintain, and a set the platform adds tomorrow is diffed the day
//! it appears rather than the day someone remembers to name it.
//!
//! The three outcomes the design names map onto four rows, because a stable
//! identifier whose payload moved is two different facts. A facet that
//! appeared has no outbound edges and is an operator question. A value that
//! disappeared leaves edges pointing at an identifier no capture holds, which
//! must stop those edges being used rather than guess a replacement. A facet
//! that changed its parent or its category moved between axes or under a
//! different root, which is structural and changes what the derivations
//! build. And a facet that changed only its presentation is not structural at
//! all, because labels are read out of the captures by `native_label` rather
//! than stored beside the terms, so it is reported and does not block.
//!
//! What this diff cannot see is cardinality and requiredness, because those
//! are properties of the form rather than of the vocabulary. The design's
//! answer is that the first failed create after a re-poll is a registry
//! question rather than a retry, and that belongs to the adapter.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What changed about one option, named by the identifier the wire addresses
/// it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftKind {
    /// The fresh capture holds an option the committed one does not. It seeds
    /// a term with no outbound edges, which is an operator question.
    Added,
    /// The committed capture holds an option the fresh one does not. Every
    /// edge onto it now points at an identifier the platform no longer
    /// offers, and no replacement may be guessed.
    Removed,
    /// The option kept its identifier and changed its parent or its category,
    /// so it moved under a different root or between axes.
    Restructured,
    /// The option kept its identifier, its parent and its category, and
    /// changed something else. Presentational, and structural of nothing.
    Relabelled,
}

impl DriftKind {
    /// Whether this row must stop a run rather than be recorded and passed
    /// over. Presentation is the only thing that does not.
    #[must_use]
    pub const fn blocks(self) -> bool {
        match self {
            Self::Added | Self::Removed | Self::Restructured => true,
            Self::Relabelled => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftRow {
    /// The option set the row belongs to, as the capture names it.
    pub set: String,
    pub native_id: String,
    pub kind: DriftKind,
    /// The committed payload, absent where the option is new.
    pub committed: Option<Value>,
    /// The fresh payload, absent where the option has gone.
    pub fresh: Option<Value>,
}

/// One capture compared against another, ordered by set and then by
/// identifier so two runs over the same pair produce byte-identical reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftReport {
    /// The `_source` line of the capture that was already committed.
    pub committed_source: String,
    /// The `_source` line of the capture it was compared against.
    pub fresh_source: String,
    /// Every set present in either capture, so a report names what it looked
    /// at rather than only what moved.
    pub sets_compared: Vec<String>,
    pub rows: Vec<DriftRow>,
}

impl DriftReport {
    /// The rows an operator must act on before the relation is trusted again.
    pub fn blocking(&self) -> impl Iterator<Item = &DriftRow> {
        self.rows.iter().filter(|row| row.kind.blocks())
    }

    /// Whether the run may pass. A relabelled option alone is clean, because
    /// nothing structural reads a stored label.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.blocking().next().is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftError {
    Parse { file: &'static str, detail: String },
    NotAnObject { file: &'static str },
}

impl core::fmt::Display for DriftError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse { file, detail } => write!(f, "{file} is not the expected shape: {detail}"),
            Self::NotAnObject { file } => write!(f, "{file} is not a JSON object"),
        }
    }
}

impl core::error::Error for DriftError {}

/// Diffs a fresh capture against the committed one, by native identifier.
pub fn diff(committed: &str, fresh: &str) -> Result<DriftReport, DriftError> {
    let before = parse(committed, "the committed capture")?;
    let after = parse(fresh, "the fresh capture")?;
    let before_sets = option_sets(&before);
    let after_sets = option_sets(&after);

    let names: BTreeSet<&String> = before_sets.keys().chain(after_sets.keys()).collect();
    let mut rows = Vec::new();
    for name in &names {
        let empty = BTreeMap::new();
        let old = before_sets.get(*name).unwrap_or(&empty);
        let new = after_sets.get(*name).unwrap_or(&empty);
        for id in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
            let row = match (old.get(id), new.get(id)) {
                (None, Some(value)) => Some((DriftKind::Added, None, Some(value.clone()))),
                (Some(value), None) => Some((DriftKind::Removed, Some(value.clone()), None)),
                (Some(was), Some(is)) if was != is => Some((
                    if structural(was) == structural(is) {
                        DriftKind::Relabelled
                    } else {
                        DriftKind::Restructured
                    },
                    Some(was.clone()),
                    Some(is.clone()),
                )),
                _ => None,
            };
            if let Some((kind, committed_value, fresh_value)) = row {
                rows.push(DriftRow {
                    set: (*name).clone(),
                    native_id: (*id).clone(),
                    kind,
                    committed: committed_value,
                    fresh: fresh_value,
                });
            }
        }
    }

    Ok(DriftReport {
        committed_source: source_of(&before),
        fresh_source: source_of(&after),
        sets_compared: names.into_iter().cloned().collect(),
        rows,
    })
}

/// The fields whose movement changes what the derivations build, as against
/// the ones that only change what a form renders. `parentId` decides which
/// root a facet hangs under and `category` decides which axis reads it at
/// all, so either moving is a different vocabulary rather than a new label
/// for the same one.
fn structural(option: &Value) -> (Option<&Value>, Option<&Value>) {
    (option.get("parentId"), option.get("category"))
}

fn parse(text: &str, file: &'static str) -> Result<serde_json::Map<String, Value>, DriftError> {
    let value: Value = serde_json::from_str(text).map_err(|error| DriftError::Parse {
        file,
        detail: error.to_string(),
    })?;
    match value {
        Value::Object(map) => Ok(map),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err(DriftError::NotAnObject { file })
        }
    }
}

/// Every option set a capture holds. A set is any top-level key whose value
/// is an object carrying an `options` object, which is the shape both
/// captures already use and the one the derivations already read.
fn option_sets(
    capture: &serde_json::Map<String, Value>,
) -> BTreeMap<String, BTreeMap<String, Value>> {
    capture
        .iter()
        .filter_map(|(name, value)| {
            let options = value.get("options")?.as_object()?;
            let members = options
                .iter()
                .map(|(id, member)| (id.clone(), member.clone()))
                .collect();
            Some((name.clone(), members))
        })
        .collect()
}

fn source_of(capture: &serde_json::Map<String, Value>) -> String {
    capture
        .get("_source")
        .and_then(Value::as_str)
        .unwrap_or("unrecorded")
        .to_owned()
}

#[cfg(test)]
mod tests;
