//! The grade axis: the one derivation this ingest performs over the mirror's
//! own vocabulary, and the only one.
//!
//! Everything else is captured verbatim, because the sources research fixed
//! that discipline: the source's own subject and node-type labels go in
//! unchanged. Grade is the exception because it is what a seller searches by
//! and every framework encodes it differently, so a comparable interval has to
//! be derived from the level codes rather than read off them.
//!
//! Deliberately not `tam_taxonomy::grades`, which derives the marketplace
//! grade crosswalk from Tes year groups and TPT grade options over age bounds.
//! That is a different scale for a different purpose, and reusing it here
//! would pull the whole taxonomy model into a crate that models a published
//! standards corpus.

use serde::{Deserialize, Serialize};

/// A school year on the one scale the four frameworks can be compared on.
///
/// Prekindergarten is -1 and kindergarten is 0 so that the ordinary grades
/// keep their own numbers, which is what makes an interval readable in the
/// data file without a legend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GradeLevel(pub i8);

impl GradeLevel {
    pub const PREKINDERGARTEN: GradeLevel = GradeLevel(-1);
    pub const KINDERGARTEN: GradeLevel = GradeLevel(0);

    /// The source's own level code, as the Common Standards Project writes it:
    /// `K`, `PK`, and the two-digit years `01` through `12`.
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "PK" | "PreK" | "Pre-K" => Some(GradeLevel::PREKINDERGARTEN),
            "K" => Some(GradeLevel::KINDERGARTEN),
            other => other
                .parse::<i8>()
                .ok()
                .filter(|year| (1..=12).contains(year))
                .map(GradeLevel),
        }
    }
}

/// The closed interval of school years a standard set covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradeInterval {
    pub low: GradeLevel,
    pub high: GradeLevel,
}

impl GradeInterval {
    pub fn covers(self, level: GradeLevel) -> bool {
        self.low <= level && level <= self.high
    }
}

/// The source's grade coding, captured verbatim, beside the interval derived
/// from it.
///
/// `uncovered` names the level codes that did not parse. They are reported
/// rather than dropped for the reason `tam-taxonomy`'s grade derivation
/// reports its residue: a level silently discarded is a set that quietly stops
/// answering a grade filter, and nothing downstream can tell that from a set
/// that genuinely covers no grade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GradeBand {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<GradeInterval>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncovered: Vec<String>,
}

impl GradeBand {
    /// Derive the interval spanning every level code that parses, and record
    /// the rest.
    pub fn derive(levels: &[String]) -> Self {
        let mut bounds: Option<(GradeLevel, GradeLevel)> = None;
        let mut uncovered = Vec::new();
        for level in levels {
            match GradeLevel::parse(level) {
                Some(parsed) => {
                    bounds = Some(match bounds {
                        None => (parsed, parsed),
                        Some((low, high)) => (low.min(parsed), high.max(parsed)),
                    });
                }
                None => uncovered.push(level.clone()),
            }
        }
        GradeBand {
            levels: levels.to_vec(),
            interval: bounds.map(|(low, high)| GradeInterval { low, high }),
            uncovered,
        }
    }

    pub fn covers(&self, level: GradeLevel) -> bool {
        self.interval.is_some_and(|interval| interval.covers(level))
    }
}

impl GradeBand {
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty() && self.interval.is_none() && self.uncovered.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_band_spans_every_level_that_parses() {
        let band = GradeBand::derive(&["09".into(), "10".into(), "11".into(), "12".into()]);
        assert_eq!(
            band.interval,
            Some(GradeInterval {
                low: GradeLevel(9),
                high: GradeLevel(12)
            }),
            "the interval must span the whole set"
        );
        assert!(band.uncovered.is_empty(), "every level parsed");
        assert!(band.covers(GradeLevel(10)), "grade 10 is inside 9-12");
        assert!(!band.covers(GradeLevel(8)), "grade 8 is outside 9-12");
    }

    #[test]
    fn kindergarten_sorts_below_grade_one() {
        let band = GradeBand::derive(&["K".into(), "02".into()]);
        assert_eq!(
            band.interval,
            Some(GradeInterval {
                low: GradeLevel::KINDERGARTEN,
                high: GradeLevel(2)
            }),
            "K must be the low bound, not a parse failure"
        );
    }

    #[test]
    fn an_unparsed_level_is_reported_rather_than_dropped() {
        let band = GradeBand::derive(&["05".into(), "Adult".into()]);
        assert_eq!(
            band.uncovered,
            vec!["Adult".to_owned()],
            "the residue is named"
        );
        assert_eq!(
            band.interval,
            Some(GradeInterval {
                low: GradeLevel(5),
                high: GradeLevel(5)
            }),
            "the levels that did parse still yield an interval"
        );
    }
}
