//! The site-wide switches: maintenance mode, the landing page's seasonal
//! theme, and the announcement banner.
//!
//! One public read and two operator routes over the `site_setting` table
//! (migration 0087). The public read is what the landing page and the console
//! fetch at load, so it takes no session and is cached by the browser for a
//! minute; the operator's read is the same answer uncached, which is what the
//! admin page and the console's maintenance gate need after a change.
//!
//! [`maintenance_gate`] is the server's half of maintenance mode: `tam-server`
//! asks it before serving a landing page or the console shell, and it answers
//! with the maintenance setting only when the request is not an operator's.

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{OperatorRepo, SessionRepo, SiteSettingRepo};

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::session::{token_from_cookie_header, OperatorContext};
use crate::time::{days_from_date, utc_day};
use crate::version::APIVersion;
use crate::AppState;

/// The keys this module reads. Every other key in the table, the discounts
/// surface's `sale.*` among them, is somebody else's and is never answered
/// here.
const MAINTENANCE_KEY: &str = "maintenance";
const THEME_KEY: &str = "theme";
const BANNER_KEY: &str = "banner";

/// How long the public answer may be reused. A minute is the longest a switch
/// flipped by an operator takes to reach a visitor who already loaded it.
const PUBLIC_CACHE: &str = "public, max-age=60";

/// Longest maintenance message, in characters: one line on a phone, twice.
const MESSAGE_MAX_CHARS: usize = 280;
/// Longest banner text, in characters.
const BANNER_TEXT_MAX_CHARS: usize = 140;
/// Longest banner link.
const BANNER_HREF_MAX_CHARS: usize = 500;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Maintenance {
    pub on: bool,
    /// When the site expects to be back, in the operator's words. Shown on
    /// the maintenance page under its heading; absent, the page says it
    /// expects to be back shortly.
    pub message: Option<String>,
}

/// The seasonal themes, as the landing page's `html[data-season]` and the
/// console's `/seasons/<name>.svg` mark spell them. Keep in step with
/// `SeasonName` in `web/src/lib/api.ts`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeName {
    #[default]
    None,
    Halloween,
    Christmas,
    Valentines,
    AprilFools,
    FourthOfJuly,
    BackToSchool,
    Winter,
    Summer,
    Spring,
    Autumn,
    Thanksgiving,
    NewYear,
    Matariki,
    GuyFawkes,
    StPatricks,
    Easter,
}

/// The theme as stored: a name and the UTC days it runs between, both ends
/// included. An absent end is open.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeSetting {
    pub name: ThemeName,
    pub from: Option<String>,
    pub until: Option<String>,
}

/// The theme as answered: what is stored, and whether it is showing today by
/// the server's clock. Decided here rather than in the page, so a visitor's
/// wrong clock cannot show Halloween in March.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeView {
    pub name: ThemeName,
    pub from: Option<String>,
    pub until: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Banner {
    pub text: String,
    pub href: String,
}

/// What `GET /{version}/site` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteView {
    pub maintenance: Maintenance,
    pub theme: ThemeView,
    pub banner: Option<Banner>,
}

/// A change to any of the three settings. An absent field is left as it is;
/// `banner: null` removes the banner, which is why that one field tells
/// absent from null.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SitePatch {
    #[serde(default)]
    pub maintenance: Option<Maintenance>,
    #[serde(default)]
    pub theme: Option<ThemeSetting>,
    #[serde(default, deserialize_with = "banner_patch")]
    pub banner: BannerPatch,
}

/// The three things a patch can say about the banner: nothing, take it
/// down, or put this one up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BannerPatch {
    #[default]
    Keep,
    Remove,
    Set(Banner),
}

/// A field that is present, even as null, is a change; `#[serde(default)]`
/// makes an absent one `Keep`.
fn banner_patch<'de, D>(deserializer: D) -> Result<BannerPatch, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Option::<Banner>::deserialize(deserializer)? {
        None => BannerPatch::Remove,
        Some(banner) => BannerPatch::Set(banner),
    })
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

/// Whether the theme shows on `today`, a UTC day number. A stored date that
/// no longer parses shows nothing rather than everything.
fn theme_active(theme: &ThemeSetting, today: i64) -> bool {
    if theme.name == ThemeName::None {
        return false;
    }
    let starts = match theme.from.as_deref().map(days_from_date) {
        None => true,
        Some(Ok(from)) => today >= from,
        Some(Err(_)) => false,
    };
    let runs = match theme.until.as_deref().map(days_from_date) {
        None => true,
        Some(Ok(until)) => today <= until,
        Some(Err(_)) => false,
    };
    starts && runs
}

/// A stored value in the shape its key promises, or the key's off state. A
/// row that fails to parse is logged and read as off: a switch that cannot be
/// read must not take the site down.
fn parsed<T: serde::de::DeserializeOwned + Default>(key: &str, value: serde_json::Value) -> T {
    serde_json::from_value(value).unwrap_or_else(|error| {
        eprintln!("tam-api: site_setting {key} does not parse ({error}); reading it as unset");
        T::default()
    })
}

/// The three settings, read in one query and folded into the answer.
pub async fn read_site(state: &AppState) -> Result<SiteView, APIError> {
    let rows = SiteSettingRepo::new(state.pool.clone())
        .many(&[MAINTENANCE_KEY, THEME_KEY, BANNER_KEY])
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let mut maintenance = Maintenance::default();
    let mut theme = ThemeSetting::default();
    let mut banner = None;
    for (key, value) in rows {
        match key.as_str() {
            MAINTENANCE_KEY => maintenance = parsed(&key, value),
            THEME_KEY => theme = parsed(&key, value),
            BANNER_KEY => banner = parsed::<Option<Banner>>(&key, value),
            _ => {}
        }
    }
    let active = theme_active(&theme, utc_day((state.wall)()));
    Ok(SiteView {
        maintenance,
        theme: ThemeView {
            name: theme.name,
            from: theme.from,
            until: theme.until,
            active,
        },
        banner,
    })
}

/// `GET /{version}/site`: public, and cached by the browser for a minute.
pub(crate) async fn site_view(
    _version: APIVersion,
    State(state): State<AppState>,
) -> Result<Response, APIError> {
    let view = read_site(&state).await?;
    Ok(([(header::CACHE_CONTROL, PUBLIC_CACHE)], Json(view)).into_response())
}

/// `GET /{version}/admin/site`: the same answer, uncached, for an operator.
pub(crate) async fn admin_site_view(
    _version: APIVersion,
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Response, APIError> {
    let view = read_site(&state).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(view)).into_response())
}

/// The maintenance setting as it will be stored: the message trimmed, and an
/// empty one dropped.
fn checked_maintenance(maintenance: Maintenance) -> Result<Maintenance, APIError> {
    let message = maintenance
        .message
        .map(|message| message.trim().to_owned())
        .filter(|message| !message.is_empty());
    if message
        .as_ref()
        .is_some_and(|message| message.chars().count() > MESSAGE_MAX_CHARS)
    {
        return Err(validation(
            "the maintenance message is at most two hundred and eighty characters",
        ));
    }
    Ok(Maintenance {
        on: maintenance.on,
        message,
    })
}

fn checked_theme(theme: ThemeSetting) -> Result<ThemeSetting, APIError> {
    let day = |raw: &Option<String>| -> Result<Option<i64>, APIError> {
        raw.as_deref()
            .map(|raw| {
                days_from_date(raw)
                    .map_err(|_| validation("a theme date is a calendar day, YYYY-MM-DD"))
            })
            .transpose()
    };
    if let (Some(from), Some(until)) = (day(&theme.from)?, day(&theme.until)?) {
        if from > until {
            return Err(validation("a theme cannot end before it starts"));
        }
    }
    Ok(theme)
}

fn checked_banner(banner: &Banner) -> Result<Banner, APIError> {
    let text = banner.text.trim().to_owned();
    let href = banner.href.trim().to_owned();
    if text.is_empty() || text.chars().count() > BANNER_TEXT_MAX_CHARS {
        return Err(validation(
            "a banner carries text, in at most a hundred and forty characters",
        ));
    }
    // A path on this origin or an https link. Anything else -- `javascript:`
    // above all -- is refused, because the landing page renders this as a link.
    let linkable =
        (href.starts_with('/') && !href.starts_with("//")) || href.starts_with("https://");
    if !linkable || href.chars().count() > BANNER_HREF_MAX_CHARS {
        return Err(validation(
            "a banner links to a path on this site or to an https address",
        ));
    }
    Ok(Banner { text, href })
}

/// `PATCH /{version}/admin/site`: change any of the three settings, answering
/// the whole view as it now stands.
///
/// Every field is validated before any is written, so a patch with one bad
/// field changes nothing.
pub(crate) async fn update_site(
    _version: APIVersion,
    State(state): State<AppState>,
    operator: OperatorContext,
    Json(patch): Json<SitePatch>,
) -> Result<Response, APIError> {
    let maintenance = patch.maintenance.map(checked_maintenance).transpose()?;
    let theme = patch.theme.map(checked_theme).transpose()?;
    let banner = match patch.banner {
        BannerPatch::Keep => None,
        BannerPatch::Remove => Some(None),
        BannerPatch::Set(banner) => Some(Some(checked_banner(&banner)?)),
    };
    let repo = SiteSettingRepo::new(state.pool.clone());
    let now = (state.wall)();
    let encode = |value: serde_json::Result<serde_json::Value>| {
        value.map_err(|error| state.internal(&error.to_string()))
    };
    let writes = [
        (MAINTENANCE_KEY, maintenance.map(serde_json::to_value)),
        (THEME_KEY, theme.map(serde_json::to_value)),
        (BANNER_KEY, banner.map(serde_json::to_value)),
    ];
    for (key, value) in writes {
        if let Some(value) = value {
            repo.set(key, &encode(value)?, operator.user, now)
                .await
                .map_err(|error| state.internal(&error.to_string()))?;
        }
    }
    let view = read_site(&state).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(view)).into_response())
}

/// The maintenance setting a request for a page should be answered with, or
/// `None` where the page should be served as usual.
///
/// `None` when maintenance is off, and when the request carries the session of
/// a platform operator, who has to be able to use the site to turn maintenance
/// off again. Everybody else -- signed out, or a seller -- gets `Some`.
///
/// A fault reading the setting is `None` and logged: maintenance mode is a
/// courtesy, and a database hiccup must not turn every page into a 503.
pub async fn maintenance_gate(state: &AppState, headers: &HeaderMap) -> Option<Maintenance> {
    let stored = SiteSettingRepo::new(state.pool.clone())
        .get(MAINTENANCE_KEY)
        .await
        .map_err(|error| eprintln!("tam-api: maintenance setting unreadable: {error}"))
        .ok()??;
    let maintenance: Maintenance = parsed(MAINTENANCE_KEY, stored);
    if !maintenance.on {
        return None;
    }
    if is_operator(state, headers).await {
        return None;
    }
    Some(maintenance)
}

/// Whether the request's session cookie belongs to an active operator. Any
/// fault answers `false`: the caller then sees the maintenance page, which is
/// the conservative reading.
async fn is_operator(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(token_from_cookie_header)
    else {
        return false;
    };
    let Ok(Some(identity)) = SessionRepo::new(state.pool.clone())
        .resolve(&token, (state.wall)())
        .await
    else {
        return false;
    };
    OperatorRepo::new(state.pool.clone())
        .is_active(identity.user)
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(name: ThemeName, from: Option<&str>, until: Option<&str>) -> ThemeSetting {
        ThemeSetting {
            name,
            from: from.map(str::to_owned),
            until: until.map(str::to_owned),
        }
    }

    /// A literal date's day number; a typo in one reads as the far past, which
    /// fails the assertion that uses it rather than passing quietly.
    fn day(raw: &str) -> i64 {
        days_from_date(raw).unwrap_or(i64::MIN)
    }

    /// Both ends are included, by UTC day.
    #[test]
    fn a_theme_shows_from_its_first_day_through_its_last() {
        let october = theme(ThemeName::Halloween, Some("2026-10-01"), Some("2026-10-31"));
        assert!(!theme_active(&october, day("2026-09-30")));
        assert!(theme_active(&october, day("2026-10-01")));
        assert!(theme_active(&october, day("2026-10-31")));
        assert!(!theme_active(&october, day("2026-11-01")));
    }

    #[test]
    fn an_open_end_runs_forever_and_none_never_shows() {
        let open = theme(ThemeName::Christmas, None, None);
        assert!(theme_active(&open, day("2031-06-15")));
        let none = theme(ThemeName::None, None, None);
        assert!(!theme_active(&none, day("2026-10-15")));
        let broken = theme(ThemeName::Halloween, Some("not-a-date"), None);
        assert!(!theme_active(&broken, day("2026-10-15")));
    }

    #[test]
    fn the_utc_day_turns_at_midnight_utc() {
        let midnight = crate::time::instant_from_rfc3339("2026-10-01T00:00:00Z").map(utc_day);
        let before = crate::time::instant_from_rfc3339("2026-09-30T23:59:59Z").map(utc_day);
        assert_eq!(midnight, Ok(day("2026-10-01")));
        assert_eq!(before, Ok(day("2026-09-30")));
    }

    #[test]
    fn a_banner_links_only_to_this_site_or_https() {
        let banner = |href: &str| {
            checked_banner(&Banner {
                text: "Sale on".to_owned(),
                href: href.to_owned(),
            })
        };
        assert!(banner("/pricing/").is_ok());
        assert!(banner("https://teachouse.io/pricing/").is_ok());
        assert!(banner("javascript:alert(1)").is_err());
        assert!(banner("//evil.example/").is_err());
        assert!(banner("http://teachouse.io/").is_err());
    }

    #[test]
    fn a_theme_cannot_end_before_it_starts_or_name_a_day_that_does_not_exist() {
        let backwards = theme(ThemeName::Halloween, Some("2026-10-31"), Some("2026-10-01"));
        assert!(checked_theme(backwards).is_err());
        let impossible = theme(ThemeName::Halloween, Some("2026-02-30"), None);
        assert!(checked_theme(impossible).is_err());
    }

    /// The wire names are the landing page's and the console's contract: the
    /// stylesheet keys on them and the sticker folders are named after them.
    #[test]
    fn every_theme_round_trips_under_its_wire_name() {
        let names = [
            (ThemeName::None, "none"),
            (ThemeName::Halloween, "halloween"),
            (ThemeName::Christmas, "christmas"),
            (ThemeName::Valentines, "valentines"),
            (ThemeName::AprilFools, "april-fools"),
            (ThemeName::FourthOfJuly, "fourth-of-july"),
            (ThemeName::BackToSchool, "back-to-school"),
            (ThemeName::Winter, "winter"),
            (ThemeName::Summer, "summer"),
            (ThemeName::Spring, "spring"),
            (ThemeName::Autumn, "autumn"),
            (ThemeName::Thanksgiving, "thanksgiving"),
            (ThemeName::NewYear, "new-year"),
            (ThemeName::Matariki, "matariki"),
            (ThemeName::GuyFawkes, "guy-fawkes"),
            (ThemeName::StPatricks, "st-patricks"),
            (ThemeName::Easter, "easter"),
        ];
        for (name, wire) in names {
            let json = serde_json::to_value(name).ok();
            assert_eq!(json, Some(serde_json::Value::from(wire)), "{name:?}");
            let back: Option<ThemeName> = serde_json::from_value(serde_json::json!(wire)).ok();
            assert_eq!(back, Some(name), "{wire}");
        }
        let unknown: Result<ThemeName, _> = serde_json::from_value(serde_json::json!("diwali"));
        assert!(unknown.is_err());
    }
}
