#![forbid(unsafe_code)]

#[derive(Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Committed,
    Failed(String),
    Ambiguous(String),
}

/// Positive-assertion classification: Committed only if the body is JSON carrying the expected id.
pub fn classify(status: u16, body_text: &str, expected_id: i64) -> WriteOutcome {
    let looks_signin = body_text.contains("<html")
        && (body_text.contains("sign-in") || body_text.contains("login"));
    match status {
        200 | 201 => match serde_json::from_str::<serde_json::Value>(body_text) {
            Ok(v) if v.get("id").and_then(|x| x.as_i64()) == Some(expected_id) => {
                WriteOutcome::Committed
            }
            Ok(_) => WriteOutcome::Ambiguous("2xx JSON without the expected id".into()),
            Err(_) if looks_signin => {
                WriteOutcome::Ambiguous("2xx but a sign-in interstitial".into())
            }
            Err(_) => WriteOutcome::Ambiguous("2xx but body is not the expected JSON".into()),
        },
        401 | 403 => {
            WriteOutcome::Ambiguous(format!("{status}: auth challenge, write state unknown"))
        }
        429 => WriteOutcome::Ambiguous("429: rate limited, write state unknown".into()),
        408 | 502 | 503 | 504 => {
            WriteOutcome::Ambiguous(format!("{status}: transient, write state unknown"))
        }
        400 | 404 | 422 => WriteOutcome::Failed(format!("{status}: definitively rejected")),
        _ => WriteOutcome::Ambiguous(format!("{status}: unclassified")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: i64 = 42;
    #[test]
    fn committed_needs_the_expected_id() {
        assert_eq!(classify(200, r#"{"id":42}"#, ID), WriteOutcome::Committed);
    }
    #[test]
    fn truncated_body_is_ambiguous() {
        assert!(matches!(
            classify(200, r#"{"id"#, ID),
            WriteOutcome::Ambiguous(_)
        ));
    }
    #[test]
    fn signin_interstitial_is_ambiguous() {
        assert!(matches!(
            classify(200, "<html>please sign-in</html>", ID),
            WriteOutcome::Ambiguous(_)
        ));
    }
    #[test]
    fn rate_limit_is_ambiguous() {
        assert!(matches!(classify(429, "", ID), WriteOutcome::Ambiguous(_)));
    }
    #[test]
    fn auth_challenge_is_ambiguous() {
        assert!(matches!(classify(403, "", ID), WriteOutcome::Ambiguous(_)));
    }
    #[test]
    fn hard_reject_is_failed() {
        assert!(matches!(classify(422, "", ID), WriteOutcome::Failed(_)));
    }
    #[test]
    fn wrong_id_is_ambiguous_not_committed() {
        assert!(matches!(
            classify(200, r#"{"id":99}"#, ID),
            WriteOutcome::Ambiguous(_)
        ));
    }
}
