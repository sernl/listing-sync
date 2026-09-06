//! The organisation's own settings: the name the seller sees, the slug they
//! claim, and the writes the settings surface and the claim screen issue.
//! Every route reads the organisation from the session's [`OrgContext`] and
//! nowhere else, so the request carries no organisation identifier a caller
//! could substitute.

use std::collections::HashMap;
use std::sync::LazyLock;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{OrgRecord, OrgRepo, OrgWrite};
use tam_types::{OrgId, Timestamp, UserId};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn missing(what: &str) -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new(what)
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

/// The longest name the settings field accepts, counted in characters rather
/// than bytes so a name written in accented or non-Latin letters is measured
/// the way the seller who typed it sees it.
pub const NAME_MAX_CHARS: usize = 120;

/// The bounds on a slug, in characters. ASCII by construction -- the shape
/// rule admits nothing else -- so characters and bytes agree here, unlike the
/// display name above.
pub const SLUG_MIN_CHARS: usize = 3;
/// See [`SLUG_MIN_CHARS`].
pub const SLUG_MAX_CHARS: usize = 32;

/// The handles no seller may claim, because this product already means
/// something by them.
///
/// Product policy rather than shape, which is why it lives here and not in the
/// database CHECK: it moves with our own route table, and keeping it in one
/// place is what stops the two drifting. Sorted and lowercase, both asserted
/// below.
///
/// Every entry is at least [`SLUG_MIN_CHARS`] long, so each one is refused for
/// being reserved rather than for being short. Words shorter than that are
/// left out for exactly that reason -- `me`, `new` and the API's own `v1` are
/// all unreachable already, so reserving one would enforce nothing.
///
/// The list is not maintained by eye. `web/src/lib/org-slug.test.ts` derives
/// the product's own top-level paths from the route table -- every directory
/// under `web/src/routes`, every entry under `web/static`, every page under
/// `apps/landing/src/pages` -- and fails if one of them is missing from here,
/// so a route added tomorrow fails a test rather than waiting for someone to
/// notice. That check is what makes the sentence above true rather than
/// aspirational.
pub const RESERVED_SLUGS: [&str; 53] = [
    "abuse",
    "account",
    "admin",
    "analytics",
    "api",
    "app",
    "assets",
    "auth",
    "automations",
    "billing",
    "blog",
    "cdn",
    "connections",
    "docs",
    "downloads",
    "email",
    "export",
    "fonts",
    "guides",
    "help",
    "import",
    "imports",
    "inventory",
    "jobs",
    "labels",
    "library",
    "listings",
    "login",
    "mail",
    "marketplaces",
    "notifications",
    "postmaster",
    "pricing",
    "privacy",
    "purchases",
    "queue",
    "reconciliation",
    "reset",
    "resources",
    "root",
    "security",
    "settings",
    "signup",
    "static",
    "status",
    "support",
    "sync",
    "system",
    "teachouse",
    "templates",
    "terms",
    "webmaster",
    "www",
];

/// What the console should do about this organisation's slug.
///
/// Three states rather than a boolean over `slug IS NULL`, because a newly
/// provisioned organisation and one that predates the slug both hold no slug
/// and are treated differently: the first meets a gate, the second a banner it
/// can dismiss. The distinction is carried by `organisation.slug_deferred`,
/// which migration 0057 wrote once and nothing writes again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlugPrompt {
    /// A slug is claimed. Ask for nothing.
    Settled,
    /// No slug, and this organisation was created knowing it would be asked.
    /// The claim screen stands before the console.
    Claim,
    /// No slug, and this organisation predates the question. A banner it can
    /// dismiss, because it is mid-work and a gate would be a rude surprise.
    Banner,
}

impl SlugPrompt {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 3] = [Self::Settled, Self::Claim, Self::Banner];

    /// The prompt one stored row calls for.
    ///
    /// A named function rather than a condition written into the view's
    /// construction, so that a test can hold every case without rendering a
    /// page.
    #[must_use]
    pub fn of(record: &OrgRecord) -> Self {
        match (record.slug.is_some(), record.slug_deferred) {
            (true, _) => Self::Settled,
            (false, true) => Self::Banner,
            (false, false) => Self::Claim,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrgView {
    pub id: OrgId,
    pub name: String,
    pub slug: Option<String>,
    pub slug_prompt: SlugPrompt,
}

impl OrgView {
    fn of(record: OrgRecord) -> Self {
        let slug_prompt = SlugPrompt::of(&record);
        Self {
            id: record.id,
            name: record.name,
            slug: record.slug,
            slug_prompt,
        }
    }
}

/// A verdict on one slug, not an organisation, which is why it carries no
/// identifier of any kind. `slug` is the normalised form, so the console can
/// show the seller what they would actually get.
#[derive(Debug, Serialize, Deserialize)]
pub struct SlugAvailability {
    pub slug: String,
    pub available: bool,
}

/// The settings write and the claim are one body and one route.
///
/// Both fields are optional and at least one must be present. A claim is the
/// first slug write and a second is a rename, so a separate claim route would
/// need its own already-claimed refusal and would duplicate the validator.
/// Wire-compatible with a client that sends only `name`, which is what the
/// settings form sent before the slug existed.
#[derive(Debug, Deserialize)]
pub struct UpdateBody {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
}

fn validated_name(raw: &str) -> Result<&str, APIError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(validation("an organisation name cannot be empty"));
    }
    if trimmed.chars().count() > NAME_MAX_CHARS {
        return Err(validation(&format!(
            "an organisation name is at most {NAME_MAX_CHARS} characters"
        )));
    }
    Ok(trimmed)
}

/// The slug as it will be stored, or the sentence the seller is shown.
///
/// Case is normalised rather than refused, which is the precedent
/// [`validated_name`] sets by trimming rather than refusing whitespace: the
/// route answers with what it stored, so the console renders the value the
/// server kept instead of the one the seller typed.
///
/// The refusals are ordered so the seller reads the most specific true thing
/// about what they typed: the character set before the hyphen rule before the
/// length, because "use letters, numbers and hyphens only" explains an
/// underscore where "at least three characters" would not.
fn validated_slug(raw: &str) -> Result<String, APIError> {
    let slug = raw.trim().to_lowercase();
    if slug.is_empty() {
        return Err(validation("Choose a name for your organisation."));
    }
    if !slug.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
        return Err(validation("Use letters, numbers and hyphens only."));
    }
    if slug.starts_with('-') || slug.ends_with('-') || slug.contains("--") {
        return Err(validation(
            "Hyphens go between words, so not at the start or end, and never two in a row.",
        ));
    }
    if slug.chars().count() < SLUG_MIN_CHARS {
        return Err(validation(&format!(
            "A name is at least {SLUG_MIN_CHARS} characters."
        )));
    }
    if slug.chars().count() > SLUG_MAX_CHARS {
        return Err(validation(&format!(
            "A name is at most {SLUG_MAX_CHARS} characters."
        )));
    }
    // A 32-character hexadecimal string is a UUID with its hyphens removed.
    // Refusing it keeps the slug namespace disjoint from the identifier
    // namespace, so a future route that accepted either form could never read
    // one as the other. The hyphenated form is already out, at 36 characters.
    //
    // The rule reaches further than an actual identifier: any 32-character
    // slug drawn only from `0-9a-f` is refused, `abcdefabcdefabcdefabcdefabcdef01`
    // included. That is the rule meaning what it says rather than an
    // oversight -- nothing can tell such a slug from an identifier by looking
    // -- and it costs a seller only the option of a 32-character name written
    // in six letters.
    let unhyphenated_uuid =
        slug.len() == SLUG_MAX_CHARS && slug.chars().all(|character| character.is_ascii_hexdigit());
    if unhyphenated_uuid || RESERVED_SLUGS.contains(&slug.as_str()) {
        return Err(validation("That name is reserved. Try another."));
    }
    Ok(slug)
}

/// The refusal a collision earns. 409 rather than 422 because nothing about
/// what the seller typed is wrong -- someone else simply got there first --
/// and the console renders the two differently.
fn slug_taken() -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new("That name is taken. Try another.")
            .code(APIErrorCode::OrgSlugTaken)
            .kind(APIErrorKind::Validation),
    )
}

/// The slug and the prompt it calls for, for the two routes that answer the
/// console's gate rather than the settings form.
///
/// `whoami` and the session exchange both carry it, because the client learns
/// on its route load whether the claim screen stands before the console, and
/// the exchange is the very first answer a fresh sign-in receives. One indexed
/// row on a table the session extractor has already resolved against.
pub(crate) async fn slug_state(
    state: &AppState,
    org: OrgId,
) -> Result<(Option<String>, SlugPrompt), APIError> {
    let record = OrgRepo::new(state.pool.clone())
        .get(org)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("no such organisation"))?;
    let prompt = SlugPrompt::of(&record);
    Ok((record.slug, prompt))
}

pub(crate) async fn org_view(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<OrgView>, APIError> {
    let record = OrgRepo::new(state.pool.clone())
        .get(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such organisation"))?;
    Ok(Json(OrgView::of(record)))
}

/// The one write: a rename, a claim, or both at once.
///
/// It answers with the row as re-read rather than with what it was sent. The
/// rename already had one reason for that -- the name is trimmed on the way in
/// -- and the slug adds a second: a body naming one field leaves the other
/// alone, so only the stored row states both.
pub(crate) async fn update_org(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<UpdateBody>,
) -> Result<Json<OrgView>, APIError> {
    let name = body.name.as_deref().map(validated_name).transpose()?;
    let slug = body.slug.as_deref().map(validated_slug).transpose()?;
    if name.is_none() && slug.is_none() {
        return Err(validation(
            "a change must name the organisation's name, its slug, or both",
        ));
    }
    let repo = OrgRepo::new(state.pool.clone());
    match repo
        .update(context.org, name, slug.as_deref())
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        OrgWrite::Stored => {}
        OrgWrite::SlugTaken => return Err(slug_taken()),
        OrgWrite::NoSuchOrg => return Err(missing("no such organisation")),
    }
    let stored = repo
        .get(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such organisation"))?;
    Ok(Json(OrgView::of(stored)))
}

/// Whether a slug is free, as the claim screen asks while the seller types.
///
/// Advisory, and deliberately so: check-then-write is a race that only the
/// unique index settles, so the console must render the 409 from
/// [`update_org`] even when this route said the slug was free.
///
/// A malformed or reserved slug is refused here with the same 422 the write
/// gives it, rather than answered `available: false`. The two refusals are not
/// the same thing -- one is about the shape of what was typed and the other
/// about who holds it -- and collapsing them would make the console render
/// "that name is taken" for an underscore.
pub(crate) async fn slug_availability(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, slug)): Path<(String, String)>,
) -> Result<Json<SlugAvailability>, APIError> {
    within_probe_budget(context.user, (state.wall)()).await?;
    let slug = validated_slug(&slug)?;
    let taken = OrgRepo::new(state.pool.clone())
        .slug_taken(context.org, &slug)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(SlugAvailability {
        slug,
        available: !taken,
    }))
}

/// How long one session's probe window lasts, and how many probes it holds.
///
/// The availability route is a directory of every tenant's slug, read one
/// guess at a time, and the session gate alone does not stop a caller who has
/// an account from walking it. The bound is what makes that cost something.
/// Thirty in a minute is several times what a debounced field needs while a
/// seller types a name and tries a second.
///
/// Deliberately not in `tam-limits`. That crate holds the limits the product's
/// behaviour is specified against -- quotas, body sizes, attempt counts -- and
/// is founder-gated because changing one changes what the product promises.
/// This governs one advisory route and promises the seller nothing.
const PROBE_WINDOW_MS: i64 = 60_000;
/// See [`PROBE_WINDOW_MS`].
const PROBES_PER_WINDOW: u32 = 30;
/// When the budget map holds more sessions than this, the windows that have
/// already lapsed are dropped. Without it the map is an unbounded structure
/// keyed by whoever signs in.
const PROBE_SESSIONS_MAX: usize = 1024;

#[derive(Debug, Clone, Copy)]
struct ProbeWindow {
    started_at: i64,
    probes: u32,
}

/// Charges one probe against a session's window, answering whether it fits.
///
/// Pure over the map it is given, so every case -- a first probe, the last
/// one that fits, the one that does not, and the window rolling over -- is a
/// unit test rather than a timing experiment against a global.
fn charge(budgets: &mut HashMap<UserId, ProbeWindow>, user: UserId, now: Timestamp) -> bool {
    if budgets.len() > PROBE_SESSIONS_MAX {
        budgets.retain(|_, window| now.0.saturating_sub(window.started_at) < PROBE_WINDOW_MS);
    }
    let window = budgets.entry(user).or_insert(ProbeWindow {
        started_at: now.0,
        probes: 0,
    });
    if now.0.saturating_sub(window.started_at) >= PROBE_WINDOW_MS {
        *window = ProbeWindow {
            started_at: now.0,
            probes: 0,
        };
    }
    window.probes = window.probes.saturating_add(1);
    window.probes <= PROBES_PER_WINDOW
}

/// The probe budgets, keyed by the session's user.
///
/// Process-local, and deliberately not a field on [`AppState`]. The bound is a
/// property of this one route rather than of the deployment, and a mutable
/// field on the state every handler receives would put it in reach of every
/// other route. What process-local costs is that a restart forgives a budget
/// and a second process keeps its own, which is honest for a bound whose
/// purpose is to make scripted enumeration tedious rather than to enforce a
/// quota the product promises.
static PROBE_BUDGETS: LazyLock<tokio::sync::Mutex<HashMap<UserId, ProbeWindow>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

async fn within_probe_budget(user: UserId, now: Timestamp) -> Result<(), APIError> {
    let fits = {
        let mut budgets = PROBE_BUDGETS.lock().await;
        charge(&mut budgets, user, now)
    };
    if fits {
        return Ok(());
    }
    Err(APIError::new(
        StatusCode::TOO_MANY_REQUESTS,
        APIErrorEntry::new("Too many checks. Wait a moment and try again."),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        charge, validated_name, validated_slug, ProbeWindow, SlugPrompt, NAME_MAX_CHARS,
        PROBES_PER_WINDOW, PROBE_WINDOW_MS, RESERVED_SLUGS, SLUG_MAX_CHARS, SLUG_MIN_CHARS,
    };
    use crate::error::APIErrorKind;
    use axum::http::StatusCode;
    use std::collections::HashMap;
    use tam_storage::OrgRecord;
    use tam_types::{OrgId, Timestamp, UserId, Uuid};

    fn record(slug: Option<&str>, slug_deferred: bool) -> OrgRecord {
        OrgRecord {
            id: OrgId(Uuid([0x11; 16])),
            name: "Riverbend Resources".to_owned(),
            slug: slug.map(str::to_owned),
            slug_deferred,
        }
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_rather_than_stored() {
        assert_eq!(
            validated_name("  Riverbend Resources \n").ok(),
            Some("Riverbend Resources"),
            "the stored name is the trimmed one"
        );
    }

    #[test]
    fn a_name_that_is_only_whitespace_is_refused_as_empty() {
        for raw in ["", "   ", "\t\n"] {
            let refused = validated_name(raw).expect_err("a blank name is refused");
            assert_eq!(
                refused.status_code(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "a blank name is the caller's error, not a fault: {raw:?}"
            );
            assert_eq!(
                refused.errors[0].kind,
                Some(APIErrorKind::Validation),
                "the refusal carries the validation kind the client branches on"
            );
        }
    }

    #[test]
    fn the_length_bound_is_counted_in_characters_and_after_trimming() {
        // Two bytes per character, so a byte-counting bound would refuse the
        // name a character-counting one accepts.
        let longest = "\u{e9}".repeat(NAME_MAX_CHARS);
        assert!(
            validated_name(&format!(" {longest} ")).is_ok(),
            "{NAME_MAX_CHARS} characters is accepted, and the surrounding space is not counted"
        );
        let over = "\u{e9}".repeat(NAME_MAX_CHARS + 1);
        let refused = validated_name(&over).expect_err("one character too many is refused");
        assert_eq!(
            refused.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "an overlong name is the caller's error"
        );
        assert!(
            refused.errors[0]
                .message
                .contains(&NAME_MAX_CHARS.to_string()),
            "the refusal states the bound it applied: {}",
            refused.errors[0].message
        );
    }

    #[test]
    fn a_well_formed_slug_is_accepted_in_each_of_its_shapes() {
        for raw in [
            "abc",
            "riverbend",
            "riverbend-resources",
            "a-b-c",
            "year-6-maths",
            "123",
            &"z".repeat(SLUG_MAX_CHARS),
        ] {
            assert_eq!(
                validated_slug(raw).ok().as_deref(),
                Some(raw),
                "a well-formed slug is stored as typed: {raw:?}"
            );
        }
    }

    #[test]
    fn case_is_normalised_rather_than_refused() {
        assert_eq!(
            validated_slug("  RiverBend-Resources  ").ok().as_deref(),
            Some("riverbend-resources"),
            "the slug is lowercased and trimmed on the way in, not refused for case"
        );
    }

    #[test]
    fn each_violation_class_is_refused_on_its_own() {
        let cases = [
            ("", "empty"),
            ("   ", "whitespace only"),
            ("ab", "one character under the floor"),
            (
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "one character over the ceiling",
            ),
            ("-abc", "a leading hyphen"),
            ("abc-", "a trailing hyphen"),
            ("ab--c", "a doubled hyphen"),
            ("ab_c", "an underscore"),
            ("ab c", "a space"),
            ("ab.c", "a dot"),
            ("ab/c", "a slash"),
            ("caf\u{e9}", "a non-ASCII letter"),
            ("\u{440}\u{438}\u{432}", "a non-Latin script"),
            (
                "0123456789abcdef0123456789abcdef",
                "an unhyphenated UUID's 32 hexadecimal characters",
            ),
        ];
        for (raw, what) in cases {
            let refused = validated_slug(raw).expect_err(&format!("{what} is refused: {raw:?}"));
            assert_eq!(
                refused.status_code(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{what} is the seller's to fix, not a fault: {raw:?}"
            );
            assert_eq!(
                refused.errors[0].kind,
                Some(APIErrorKind::Validation),
                "{what} carries the validation kind the console branches on: {raw:?}"
            );
        }
    }

    #[test]
    fn the_bounds_are_stated_exactly_rather_than_off_by_one() {
        // `z` rather than `a`: a run of `a` is hexadecimal, so a 32-character
        // one is refused by the unhyphenated-UUID rule and would prove the
        // ceiling wrong for a reason that has nothing to do with length.
        assert!(
            validated_slug(&"z".repeat(SLUG_MIN_CHARS)).is_ok(),
            "the floor itself is accepted"
        );
        assert!(
            validated_slug(&"z".repeat(SLUG_MIN_CHARS - 1)).is_err(),
            "one under the floor is refused"
        );
        assert!(
            validated_slug(&"z".repeat(SLUG_MAX_CHARS)).is_ok(),
            "the ceiling itself is accepted"
        );
        assert!(
            validated_slug(&"z".repeat(SLUG_MAX_CHARS + 1)).is_err(),
            "one over the ceiling is refused"
        );
    }

    #[test]
    fn a_hexadecimal_string_is_refused_only_at_the_full_uuid_length() {
        assert!(
            validated_slug("0123456789abcdef0123456789abcde").is_ok(),
            "thirty-one hexadecimal characters is a slug, not an identifier"
        );
        assert!(
            validated_slug("0123456789abcdef0123456789abcdef").is_err(),
            "thirty-two is an unhyphenated UUID and is refused"
        );
    }

    #[test]
    fn every_reserved_word_is_refused_for_being_reserved() {
        for reserved in RESERVED_SLUGS {
            let refused = validated_slug(reserved).expect_err(&format!("{reserved:?} is refused"));
            assert_eq!(
                refused.status_code(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "a reserved word is a policy refusal, not a collision: {reserved:?}"
            );
            assert!(
                refused.errors[0].message.contains("reserved"),
                "{reserved:?} is refused for being reserved rather than for its shape: {}",
                refused.errors[0].message
            );
        }
    }

    #[test]
    fn the_reserved_list_is_sorted_lowercase_and_reachable() {
        let mut sorted = RESERVED_SLUGS;
        sorted.sort_unstable();
        assert_eq!(
            sorted, RESERVED_SLUGS,
            "the list is kept sorted so a reader can find a word in it"
        );
        for reserved in RESERVED_SLUGS {
            assert_eq!(
                *reserved,
                reserved.to_lowercase(),
                "a reserved word is compared against a lowercased slug: {reserved:?}"
            );
            assert!(
                reserved.chars().count() >= SLUG_MIN_CHARS,
                "a word shorter than the floor is already unreachable, so reserving it \
                 enforces nothing: {reserved:?}"
            );
        }
    }

    #[test]
    fn the_prompt_separates_a_new_organisation_from_one_that_predates_the_slug() {
        assert_eq!(
            SlugPrompt::of(&record(Some("riverbend"), false)),
            SlugPrompt::Settled,
            "a claimed slug asks for nothing"
        );
        assert_eq!(
            SlugPrompt::of(&record(Some("riverbend"), true)),
            SlugPrompt::Settled,
            "a claimed slug asks for nothing even where the row was allowed to defer"
        );
        assert_eq!(
            SlugPrompt::of(&record(None, false)),
            SlugPrompt::Claim,
            "an organisation created since the slug existed meets the gate"
        );
        assert_eq!(
            SlugPrompt::of(&record(None, true)),
            SlugPrompt::Banner,
            "an organisation that predates the slug meets a banner it can dismiss"
        );
    }

    #[test]
    fn the_probe_budget_admits_its_allowance_and_refuses_the_next() {
        let mut budgets: HashMap<UserId, ProbeWindow> = HashMap::new();
        let user = UserId(Uuid([0x0A; 16]));
        let now = Timestamp(1_000);
        for probe in 1..=PROBES_PER_WINDOW {
            assert!(
                charge(&mut budgets, user, now),
                "probe {probe} of {PROBES_PER_WINDOW} is within the window's allowance"
            );
        }
        assert!(
            !charge(&mut budgets, user, now),
            "the probe after the allowance is refused"
        );
    }

    #[test]
    fn the_window_rolls_rather_than_barring_the_session_for_good() {
        let mut budgets: HashMap<UserId, ProbeWindow> = HashMap::new();
        let user = UserId(Uuid([0x0A; 16]));
        for _ in 0..=PROBES_PER_WINDOW {
            charge(&mut budgets, user, Timestamp(1_000));
        }
        assert!(
            !charge(&mut budgets, user, Timestamp(1_000 + PROBE_WINDOW_MS - 1)),
            "a probe one millisecond inside the window is still refused"
        );
        assert!(
            charge(&mut budgets, user, Timestamp(1_000 + PROBE_WINDOW_MS)),
            "the window rolls, so the session is bounded rather than barred"
        );
    }

    #[test]
    fn one_sessions_budget_is_not_anothers() {
        let mut budgets: HashMap<UserId, ProbeWindow> = HashMap::new();
        let spender = UserId(Uuid([0x0A; 16]));
        let other = UserId(Uuid([0x0B; 16]));
        let now = Timestamp(1_000);
        for _ in 0..=PROBES_PER_WINDOW {
            charge(&mut budgets, spender, now);
        }
        assert!(
            !charge(&mut budgets, spender, now),
            "the session that spent its allowance is refused"
        );
        assert!(
            charge(&mut budgets, other, now),
            "a different session carries its own window"
        );
    }
}
