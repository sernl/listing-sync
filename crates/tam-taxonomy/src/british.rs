//! The British name for each TPT grade, as the founder declared it on
//! 2026-09-11.
//!
//! A declared table and not a derivation. The grade crosswalk in
//! [`crate::grades`] deliberately never inverts a Tes age band into a year
//! group, because ages 1-4 have no band and a nearest-band rule would file
//! them under one silently; nothing here touches that rule or the edges it
//! emits. This table is a second, smaller thing: the words the create form
//! shows a British teacher above the same checkbox an American teacher ticks.
//! One selection underneath, two labels over it.
//!
//! The rule, in the founder's own words: Reception is Pre-K, Year 1 is
//! Kindergarten, and Year N is Grade N minus one through Year 13 for 12th
//! Grade. Preschool, Higher Education, Adult Education and Not Grade Specific
//! have no British counterpart and carry none rather than an approximation.
//! The three roll-ups no seller can tick still carry a band name, because the
//! form renders them as an explanation of a browse filter and the explanation
//! has to be readable in either dialect.

/// The declared pairs, keyed by the TPT facet slug the grid's checkbox posts.
///
/// Slug rather than label, because the slug is what the wire carries and what
/// `taxonomyTags` keys the facet by; a label join would break the first time
/// TPT retitled a checkbox.
const BRITISH_GRADE_LABELS: [(&str, &str); 17] = [
    ("pre-k", "Reception"),
    ("kindergarten", "Year 1"),
    ("1st-grade", "Year 2"),
    ("2nd-grade", "Year 3"),
    ("3rd-grade", "Year 4"),
    ("4th-grade", "Year 5"),
    ("5th-grade", "Year 6"),
    ("6th-grade", "Year 7"),
    ("7th-grade", "Year 8"),
    ("8th-grade", "Year 9"),
    ("9th-grade", "Year 10"),
    ("10th-grade", "Year 11"),
    ("11th-grade", "Year 12"),
    ("12th-grade", "Year 13"),
    ("elementary", "Primary School"),
    ("middle-school", "Secondary School"),
    ("high-school", "College"),
];

/// The four columns of the grade grid, each named in both dialects.
///
/// In the order the columns are laid out, so a client zips this against the
/// column sizes it is already served rather than matching on a name. The
/// fourth column holds the non-grade bands and is "Other" either side, which
/// is stated rather than left absent so the toggle relabels every heading.
pub const GRADE_BANDS: [(&str, &str); 4] = [
    ("Elementary", "Primary School"),
    ("Middle School", "Secondary School"),
    ("High School", "College"),
    ("Other", "Other"),
];

/// What a British teacher calls this grade, or `None` where the declared
/// table names no counterpart.
#[must_use]
pub fn british_grade_label(slug: &str) -> Option<&'static str> {
    BRITISH_GRADE_LABELS
        .into_iter()
        .find(|(american, _)| *american == slug)
        .map(|(_, british)| british)
}

#[cfg(test)]
mod tests {
    use super::{british_grade_label, BRITISH_GRADE_LABELS, GRADE_BANDS};

    /// The founder's arithmetic, checked rather than transcribed: Year N is
    /// Grade N minus one, so every numbered grade's year is one higher than
    /// its own number. A table typed by hand is exactly where an off-by-one
    /// hides, and it would put a Year 6 class on a Year 5 listing.
    #[test]
    fn every_numbered_grade_is_one_year_above_its_own_number() {
        let numbered: Vec<(u8, &str)> = BRITISH_GRADE_LABELS
            .into_iter()
            .filter_map(|(slug, british)| {
                let digits: String = slug.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().ok().map(|grade| (grade, british))
            })
            .collect();
        assert_eq!(numbered.len(), 12, "1st Grade through 12th Grade");
        for (grade, british) in numbered {
            assert_eq!(
                british,
                format!("Year {}", grade + 1),
                "{grade} is one behind its British year"
            );
        }
    }

    #[test]
    fn the_two_ends_of_the_table_are_the_declared_ones() {
        assert_eq!(british_grade_label("pre-k"), Some("Reception"));
        assert_eq!(british_grade_label("kindergarten"), Some("Year 1"));
        assert_eq!(british_grade_label("12th-grade"), Some("Year 13"));
    }

    /// The four TPT options the declared table names no counterpart for reach
    /// the form as an American label alone rather than as a guess.
    #[test]
    fn the_grades_outside_the_declared_table_carry_no_british_label() {
        for slug in [
            "preschool",
            "higher-education",
            "adult-education",
            "not-grade-specific",
        ] {
            assert_eq!(
                british_grade_label(slug),
                None,
                "{slug} was invented a year"
            );
        }
    }

    #[test]
    fn the_three_roll_ups_are_named_as_bands_in_both_dialects() {
        assert_eq!(british_grade_label("elementary"), Some("Primary School"));
        assert_eq!(
            british_grade_label("middle-school"),
            Some("Secondary School")
        );
        assert_eq!(british_grade_label("high-school"), Some("College"));
        assert_eq!(
            GRADE_BANDS.map(|(_, british)| british),
            ["Primary School", "Secondary School", "College", "Other"],
            "the column headings and the roll-up labels are one table",
        );
    }
}
