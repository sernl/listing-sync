//! The serving binary's half of a seller deleting their own account
//! (`tam_api::account`): the identity service's two internal routes and the
//! goodbye mail.
//!
//! Both routes sit beside `/internal/address/{subject}` in `tam-auth` and are
//! fenced by the same shared secret in the same header:
//!
//! - `POST /internal/reauth/{subject}` with `{password, sessionCookie}`
//!   checks the proof — the password for an account that has one, otherwise
//!   a sign-in of this browser younger than five minutes — and answers the
//!   address and name on success;
//! - `POST /internal/delete/{subject}` deletes the identity account, and its
//!   sessions, linked accounts and passkeys go with it by cascade.
//!
//! The address is held for the length of the request and stored nowhere, as
//! in `notify.rs`.

use tam_api::account::{
    Departing, Offboarding, OffboardingFault, OffboardingFuture, Proof, Reauthentication,
};
use tam_types::Uuid;

use crate::notify::{compose_goodbye, http_client, Relay, ResendRelay, INTERNAL_SECRET_HEADER};

pub(crate) struct AuthOffboarding {
    client: reqwest::Client,
    base: String,
    secret: String,
    relay: ResendRelay,
    console_url: String,
}

impl AuthOffboarding {
    pub(crate) fn new(
        base: &str,
        secret: &str,
        relay: ResendRelay,
        console_url: &str,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: http_client()?,
            base: base.trim_end_matches('/').to_owned(),
            secret: secret.to_owned(),
            relay,
            console_url: console_url.to_owned(),
        })
    }

    fn url(&self, route: &str, subject: Uuid) -> String {
        format!("{}/internal/{route}/{}", self.base, subject.to_hyphenated())
    }
}

fn unreachable(error: &reqwest::Error) -> OffboardingFault {
    OffboardingFault(format!("the identity service is unreachable: {error}"))
}

/// The reauthentication route's answer, read. Pure so the mapping is
/// testable without a network: this is where a wrong password and a fault
/// are told apart, and confusing the two would either delete on a fault or
/// tell a seller their right password is wrong.
fn verdict(status: u16, body: &serde_json::Value) -> Result<Reauthentication, OffboardingFault> {
    let refusal = body.get("refusal").and_then(serde_json::Value::as_str);
    match (status, refusal) {
        (200, _) => {
            let email = body
                .get("email")
                .and_then(serde_json::Value::as_str)
                .filter(|email| !email.is_empty())
                .ok_or_else(|| {
                    OffboardingFault("the identity service confirmed no address".to_owned())
                })?;
            Ok(Reauthentication::Confirmed(Departing {
                email: email.to_owned(),
                name: body
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
            }))
        }
        (404, _) => Ok(Reauthentication::NoIdentity),
        (403, Some("password")) => Ok(Reauthentication::PasswordWrong),
        (403, Some("stale")) => Ok(Reauthentication::NotFresh),
        (422, Some("password_required")) => Ok(Reauthentication::PasswordRequired),
        _ => Err(OffboardingFault(format!(
            "the identity service answered {status} to a reauthentication"
        ))),
    }
}

impl Offboarding for AuthOffboarding {
    fn reauthenticate<'a>(
        &'a self,
        subject: Uuid,
        proof: Proof<'a>,
    ) -> OffboardingFuture<'a, Reauthentication> {
        Box::pin(async move {
            let answer = self
                .client
                .post(self.url("reauth", subject))
                .header(INTERNAL_SECRET_HEADER, &self.secret)
                .json(&serde_json::json!({
                    "password": proof.password,
                    "sessionCookie": proof.session_cookie,
                }))
                .send()
                .await
                .map_err(|error| unreachable(&error))?;
            let status = answer.status().as_u16();
            let body: serde_json::Value = answer.json().await.unwrap_or_default();
            verdict(status, &body)
        })
    }

    fn farewell<'a>(&'a self, to: &'a Departing) -> OffboardingFuture<'a, ()> {
        Box::pin(async move {
            let mail = compose_goodbye(to.first_name(), &self.console_url);
            self.relay
                .send(&to.email, &mail)
                .await
                .map_err(|error| OffboardingFault(format!("{error:?}")))
        })
    }

    fn delete_identity(&self, subject: Uuid) -> OffboardingFuture<'_, ()> {
        Box::pin(async move {
            let answer = self
                .client
                .post(self.url("delete", subject))
                .header(INTERNAL_SECRET_HEADER, &self.secret)
                .send()
                .await
                .map_err(|error| unreachable(&error))?;
            let status = answer.status();
            // Gone already is what was asked for: a retry after the identity
            // half succeeded and the erasure did not lands here.
            if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
                Ok(())
            } else {
                Err(OffboardingFault(format!(
                    "the identity service answered {status} to a deletion"
                )))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::verdict;
    use tam_api::account::{Departing, Reauthentication};

    #[test]
    fn a_confirmed_proof_carries_the_address_and_a_trimmed_name() {
        let body = serde_json::json!({ "email": "seller@example.test", "name": "  Aroha " });
        assert_eq!(
            verdict(200, &body),
            Ok(Reauthentication::Confirmed(Departing {
                email: "seller@example.test".to_owned(),
                name: Some("Aroha".to_owned()),
            }))
        );
        let nameless = serde_json::json!({ "email": "seller@example.test", "name": "" });
        assert!(matches!(
            verdict(200, &nameless),
            Ok(Reauthentication::Confirmed(Departing { name: None, .. }))
        ));
        let full = Departing {
            email: "seller@example.test".to_owned(),
            name: Some("Aroha Te Whare".to_owned()),
        };
        assert_eq!(
            full.first_name(),
            Some("Aroha"),
            "the goodbye greets by first name"
        );
    }

    #[test]
    fn each_refusal_is_named_and_anything_else_is_a_fault() {
        let refusal = |word: &str| serde_json::json!({ "refusal": word });
        assert_eq!(
            verdict(403, &refusal("password")),
            Ok(Reauthentication::PasswordWrong)
        );
        assert_eq!(
            verdict(403, &refusal("stale")),
            Ok(Reauthentication::NotFresh)
        );
        assert_eq!(
            verdict(422, &refusal("password_required")),
            Ok(Reauthentication::PasswordRequired)
        );
        assert_eq!(
            verdict(404, &serde_json::Value::Null),
            Ok(Reauthentication::NoIdentity)
        );
        assert!(
            verdict(403, &serde_json::Value::Null).is_err(),
            "a bare 403 is not a wrong password"
        );
        assert!(verdict(503, &refusal("password")).is_err());
        assert!(
            verdict(200, &serde_json::json!({})).is_err(),
            "a confirmation with no address is not a confirmation"
        );
    }
}
