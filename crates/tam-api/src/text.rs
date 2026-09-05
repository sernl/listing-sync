//! What a text field a person typed may contain, for the bounds every route
//! applies to one.
//!
//! One predicate rather than a rule per module: the refusal it expresses is
//! the same everywhere, and the input that motivates it — a zero byte — is
//! refused by Postgres itself for every `text` column at once. A field that
//! reaches the database carrying one raises `22021 invalid byte sequence for
//! encoding "UTF8"`, which surfaces as a fault rather than as the validation
//! answer it is, so the check has to happen at the boundary or not at all.

/// Whether this is text a person could have meant to type.
///
/// False for any Unicode control character, which is the class `str::trim`
/// does not remove and a length bound does not notice: `U+0000`, which no
/// Postgres `text` column can hold at all, and the rest — newlines and tabs
/// included — which a single-line field has no use for and which a listing
/// cannot render without lying about what was stored.
///
/// Deliberately not a general sanitiser. It says nothing about scripts,
/// direction marks or homoglyphs, and does not try to: those are display
/// questions for the surface that renders the value, and a server that
/// second-guessed them would refuse names people actually have.
#[must_use]
pub(crate) fn is_typed_text(value: &str) -> bool {
    !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::is_typed_text;

    #[test]
    fn ordinary_text_in_any_script_is_accepted() {
        for accepted in [
            "Amped Up Learning",
            "Ressources d'\u{e9}t\u{e9}",
            "\u{5b66}\u{6821}\u{306e}\u{6559}\u{6750}",
            "Term one \u{2014} autumn",
            "\u{1f600} bundle",
        ] {
            assert!(
                is_typed_text(accepted),
                "a person can type this: {accepted}"
            );
        }
    }

    #[test]
    fn a_control_character_is_refused_wherever_it_sits() {
        for refused in [
            "a\u{0}b",
            "\u{0}",
            "line\nbreak",
            "tab\there",
            "carriage\rreturn",
            "bell\u{7}",
            "delete\u{7f}",
        ] {
            assert!(
                !is_typed_text(refused),
                "a control character is not typed text: {refused:?}"
            );
        }
    }
}
