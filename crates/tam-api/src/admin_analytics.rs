//! The operators' site analytics: what the public site's visitors did, read
//! from PostHog and shown inside the console so nobody needs a PostHog login
//! to see it.
//!
//! One route, `GET /{version}/admin/analytics/site?range=7d|30d|90d`, answers
//! the whole page. Behind it are eleven small HogQL queries against PostHog's
//! query API (`POST {host}/api/projects/{id}/query/`, a personal API key with
//! *Query Read* as the bearer): pageviews and visitors per day and in total,
//! the top pages, referring domains, UTM sources, countries, cities, device
//! types, browsers, operating systems, and the two conversion events —
//! `cta_click` from the landing's Start free links and the server's own
//! `signup_completed`.
//!
//! **A range is whole New Zealand days.** "Last 7 days" is today and the six
//! calendar days before it in `Pacific/Auckland`, the company's calendar, so a
//! day on the chart is the day an operator would name, and a day nobody
//! visited is drawn as zero rather than left out.
//!
//! **Only the public site counts.** The console sends its pageviews to the
//! same PostHog project, so when the deployment names its landing host every
//! pageview query is narrowed to `$host` equal to it. A deployment on one
//! origin has nothing to separate and is read unfiltered.
//!
//! **Five minutes of memory.** Each answer is kept per range for
//! [`CACHE_MS`], held under one lock so two operators opening the page at once
//! cost PostHog one set of queries rather than two. PostHog limits a project
//! to three concurrent queries, so the set runs [`CONCURRENT_QUERIES`] at a
//! time; a failure is never cached.
//!
//! Absent a project id and key, the route answers 503 and the page says the
//! deployment is not configured for it.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use tam_types::Timestamp;
use tokio::sync::Mutex;

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::session::OperatorContext;
use crate::time::{civil_from_days, site_day};
use crate::AppState;

/// PostHog's EU application host, where the project lives. The query API is
/// on the app host, not the ingestion host `telemetry` posts to.
pub const DEFAULT_API_HOST: &str = "https://eu.posthog.com";

/// How long one range's answer is served from memory.
pub const CACHE_MS: i64 = 5 * 60 * 1_000;

/// How many rows each breakdown keeps.
pub const TOP_ROWS: u32 = 20;

/// PostHog runs at most three queries per project at once and queues the
/// rest; asking for more only moves the wait into its queue.
pub const CONCURRENT_QUERIES: usize = 3;

/// The zone a day on the page is a day in, as HogQL names it.
const TIME_ZONE: &str = "Pacific/Auckland";

/// PostHog stops a query after ten seconds of execution and may queue it for
/// up to thirty before that, so a call can honestly take forty; past that it
/// is a fault rather than a slow answer.
const CALL_TIMEOUT_SECS: u64 = 45;
const CONNECT_TIMEOUT_SECS: u64 = 5;

/// The longest upstream body quoted into a log line.
const QUOTED_CHARS: usize = 200;

// ------------------------------------------------------------------ secrets

/// A PostHog personal API key (`phx_…`), redacted from every `Debug` render
/// as the Stripe keys are.
#[derive(Clone, PartialEq, Eq)]
pub struct PersonalKey(String);

impl PersonalKey {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for PersonalKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PersonalKey(redacted)")
    }
}

// -------------------------------------------------------------------- range

/// The three windows the page offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Range {
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "90d")]
    Quarter,
}

impl Range {
    pub const ALL: [Self; 3] = [Self::Week, Self::Month, Self::Quarter];

    #[must_use]
    pub const fn days(self) -> u32 {
        match self {
            Self::Week => 7,
            Self::Month => 30,
            Self::Quarter => 90,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Week => "7d",
            Self::Month => "30d",
            Self::Quarter => "90d",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|range| range.as_str() == raw)
    }
}

/// A range pinned to the calendar: its first and last New Zealand day, as
/// days since 1970-01-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub range: Range,
    pub first: i64,
    pub last: i64,
}

impl Window {
    /// The range ending today in New Zealand, today included.
    #[must_use]
    pub fn ending(range: Range, now: Timestamp) -> Self {
        let last = site_day(now);
        Self {
            range,
            first: last - i64::from(range.days()) + 1,
            last,
        }
    }

    /// Every day of the window, oldest first, as `YYYY-MM-DD`.
    #[must_use]
    pub fn dates(&self) -> Vec<String> {
        (self.first..=self.last).map(date_of).collect()
    }
}

fn date_of(day: i64) -> String {
    let (year, month, date) = civil_from_days(day);
    format!("{year:04}-{month:02}-{date:02}")
}

// ------------------------------------------------------------------ queries

/// One query in the set the page needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Totals,
    Daily,
    Pages,
    Referrers,
    UtmSources,
    Countries,
    Cities,
    Devices,
    Browsers,
    Systems,
    Conversions,
}

impl Section {
    pub const ALL: [Self; 11] = [
        Self::Totals,
        Self::Daily,
        Self::Pages,
        Self::Referrers,
        Self::UtmSources,
        Self::Countries,
        Self::Cities,
        Self::Devices,
        Self::Browsers,
        Self::Systems,
        Self::Conversions,
    ];

    /// The name the query carries into PostHog's `query_log`, so a slow or
    /// expensive one is findable there.
    #[must_use]
    pub const fn log_name(self) -> &'static str {
        match self {
            Self::Totals => "teachouse admin site: totals",
            Self::Daily => "teachouse admin site: daily",
            Self::Pages => "teachouse admin site: pages",
            Self::Referrers => "teachouse admin site: referrers",
            Self::UtmSources => "teachouse admin site: utm sources",
            Self::Countries => "teachouse admin site: countries",
            Self::Cities => "teachouse admin site: cities",
            Self::Devices => "teachouse admin site: devices",
            Self::Browsers => "teachouse admin site: browsers",
            Self::Systems => "teachouse admin site: operating systems",
            Self::Conversions => "teachouse admin site: conversions",
        }
    }

    /// The property a plain breakdown groups by, for the sections that are
    /// one.
    const fn breakdown(self) -> Option<&'static str> {
        match self {
            Self::Pages => Some("properties.$pathname"),
            Self::Referrers => Some("properties.$referring_domain"),
            Self::UtmSources => Some("properties.utm_source"),
            Self::Countries => Some("properties.$geoip_country_name"),
            Self::Devices => Some("properties.$device_type"),
            Self::Browsers => Some("properties.$browser"),
            Self::Systems => Some("properties.$os"),
            Self::Totals | Self::Daily | Self::Cities | Self::Conversions => None,
        }
    }
}

/// A host as a HogQL string literal, or `None` for anything that is not
/// plainly a host. The value is this deployment's own `--landing-host`, and
/// it is checked anyway: a literal is only safe to splice when it cannot
/// contain a quote.
#[must_use]
pub fn host_literal(host: &str) -> Option<String> {
    let plain = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    plain.then(|| format!("'{}'", host.to_ascii_lowercase()))
}

/// The time half of every filter: the window's New Zealand days, with a
/// coarse UTC bound beside it so PostHog can skip partitions outside it.
fn time_filter(window: &Window) -> String {
    format!(
        "timestamp >= now() - INTERVAL {scan} DAY \
         AND toDate(toTimeZone(timestamp, '{TIME_ZONE}')) >= toDate('{first}')",
        scan = window.range.days() + 1,
        first = date_of(window.first),
    )
}

/// The filter every pageview query shares.
fn pageview_filter(window: &Window, site_host: Option<&str>) -> String {
    let mut filter = format!("event = '$pageview' AND {}", time_filter(window));
    if let Some(literal) = site_host.and_then(host_literal) {
        filter.push_str(" AND properties.$host = ");
        filter.push_str(&literal);
    }
    filter
}

/// The HogQL one section runs.
#[must_use]
pub fn hogql(section: Section, window: &Window, site_host: Option<&str>) -> String {
    let pageviews = pageview_filter(window, site_host);
    if let Some(property) = section.breakdown() {
        let present = if section == Section::UtmSources {
            format!(" AND {property} IS NOT NULL AND {property} != ''")
        } else {
            String::new()
        };
        return format!(
            "SELECT {property} AS label, count(DISTINCT person_id) AS visitors, \
             count() AS pageviews FROM events WHERE {pageviews}{present} \
             GROUP BY label ORDER BY visitors DESC, pageviews DESC LIMIT {TOP_ROWS}"
        );
    }
    match section {
        Section::Totals => format!(
            "SELECT count() AS pageviews, count(DISTINCT person_id) AS visitors \
             FROM events WHERE {pageviews}"
        ),
        Section::Daily => format!(
            "SELECT toString(toDate(toTimeZone(timestamp, '{TIME_ZONE}'))) AS day, \
             count() AS pageviews, count(DISTINCT person_id) AS visitors \
             FROM events WHERE {pageviews} GROUP BY day ORDER BY day LIMIT {limit}",
            limit = window.range.days() + 1,
        ),
        Section::Cities => format!(
            "SELECT properties.$geoip_city_name AS city, \
             properties.$geoip_country_name AS country, \
             count(DISTINCT person_id) AS visitors, count() AS pageviews \
             FROM events WHERE {pageviews} AND properties.$geoip_city_name IS NOT NULL \
             GROUP BY city, country ORDER BY visitors DESC, pageviews DESC LIMIT {TOP_ROWS}"
        ),
        // Not narrowed to the landing host: `signup_completed` is captured by
        // the server, which has no host, and `cta_click` only fires on the
        // landing anyway.
        Section::Conversions => format!(
            "SELECT countIf(event = 'signup_completed') AS signups, \
             countIf(event = 'cta_click') AS cta_clicks FROM events \
             WHERE event IN ('signup_completed', 'cta_click') AND {}",
            time_filter(window)
        ),
        Section::Pages
        | Section::Referrers
        | Section::UtmSources
        | Section::Countries
        | Section::Devices
        | Section::Browsers
        | Section::Systems => String::new(),
    }
}

/// The JSON body one section posts to the query API.
#[must_use]
pub fn request_body(
    section: Section,
    window: &Window,
    site_host: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "query": {
            "kind": "HogQLQuery",
            "query": hogql(section, window, site_host),
        },
        "name": section.log_name(),
    })
}

/// The part of the query API's answer this module reads: one array of cells
/// per row, in the order the query selected them.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct QueryAnswer {
    #[serde(default)]
    pub results: Vec<Vec<serde_json::Value>>,
}

// -------------------------------------------------------------------- views

/// One day on the chart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayPoint {
    /// `YYYY-MM-DD`, a New Zealand day.
    pub day: String,
    pub pageviews: u64,
    pub visitors: u64,
}

/// One row of a breakdown. `label` is `None` where PostHog recorded nothing
/// for the property; `detail` is the country beside a city and absent
/// elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteRow {
    pub label: Option<String>,
    pub detail: Option<String>,
    pub visitors: u64,
    pub pageviews: u64,
}

/// The headline figures. Visitors are counted once across the whole range,
/// which is why they are not the sum of the days.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteTotals {
    pub pageviews: u64,
    pub visitors: u64,
    pub signups: u64,
    pub cta_clicks: u64,
}

/// The whole page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteAnalyticsView {
    pub range: Range,
    /// The window's first and last New Zealand day, `YYYY-MM-DD`.
    pub from: String,
    pub to: String,
    /// The host pageviews were narrowed to, or `None` when they were not.
    pub site_host: Option<String>,
    pub totals: SiteTotals,
    pub days: Vec<DayPoint>,
    pub pages: Vec<SiteRow>,
    pub referrers: Vec<SiteRow>,
    pub utm_sources: Vec<SiteRow>,
    pub countries: Vec<SiteRow>,
    pub cities: Vec<SiteRow>,
    pub devices: Vec<SiteRow>,
    pub browsers: Vec<SiteRow>,
    pub systems: Vec<SiteRow>,
    /// When PostHog was asked, which is what the page's "as of" reads.
    pub fetched_at: Timestamp,
}

// ------------------------------------------------------------------ mapping

/// A count cell. PostHog answers counts as JSON numbers; a string or a float
/// is read too rather than zeroed, because a type change upstream should not
/// read as nobody visiting.
fn count(cell: Option<&serde_json::Value>) -> u64 {
    match cell {
        Some(serde_json::Value::Number(number)) => number.as_u64().unwrap_or_else(|| {
            number
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .map_or(0, |value| {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "finite, non-negative and a count; a fractional part is noise"
                    )]
                    let whole = value as u64;
                    whole
                })
        }),
        Some(serde_json::Value::String(text)) => text.trim().parse().unwrap_or(0),
        Some(
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Array(_)
            | serde_json::Value::Object(_),
        )
        | None => 0,
    }
}

/// A text cell, `None` for null or blank.
fn text(cell: Option<&serde_json::Value>) -> Option<String> {
    match cell {
        Some(serde_json::Value::String(value)) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Some(serde_json::Value::Number(number)) => Some(number.to_string()),
        Some(serde_json::Value::Bool(flag)) => Some(flag.to_string()),
        Some(
            serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_),
        )
        | None => None,
    }
}

/// A plain breakdown's rows: `[label, visitors, pageviews]`.
#[must_use]
pub fn breakdown_rows(answer: &QueryAnswer) -> Vec<SiteRow> {
    answer
        .results
        .iter()
        .map(|row| SiteRow {
            label: text(row.first()),
            detail: None,
            visitors: count(row.get(1)),
            pageviews: count(row.get(2)),
        })
        .collect()
}

/// The cities' rows: `[city, country, visitors, pageviews]`.
#[must_use]
pub fn city_rows(answer: &QueryAnswer) -> Vec<SiteRow> {
    answer
        .results
        .iter()
        .map(|row| SiteRow {
            label: text(row.first()),
            detail: text(row.get(1)),
            visitors: count(row.get(2)),
            pageviews: count(row.get(3)),
        })
        .collect()
}

/// Every day of the window from the daily answer, `[day, pageviews,
/// visitors]`, with the days PostHog has no row for drawn as zero and any row
/// outside the window dropped.
#[must_use]
pub fn day_points(window: &Window, answer: &QueryAnswer) -> Vec<DayPoint> {
    let counted: HashMap<String, (u64, u64)> = answer
        .results
        .iter()
        .filter_map(|row| Some((text(row.first())?, (count(row.get(1)), count(row.get(2))))))
        .collect();
    window
        .dates()
        .into_iter()
        .map(|day| {
            let (pageviews, visitors) = counted.get(&day).copied().unwrap_or_default();
            DayPoint {
                day,
                pageviews,
                visitors,
            }
        })
        .collect()
}

/// The headline figures from the totals answer `[pageviews, visitors]` and
/// the conversions answer `[signups, cta_clicks]`.
#[must_use]
pub fn totals(site: &QueryAnswer, conversions: &QueryAnswer) -> SiteTotals {
    let first = site.results.first();
    let converted = conversions.results.first();
    SiteTotals {
        pageviews: count(first.and_then(|row| row.first())),
        visitors: count(first.and_then(|row| row.get(1))),
        signups: count(converted.and_then(|row| row.first())),
        cta_clicks: count(converted.and_then(|row| row.get(1))),
    }
}

/// The eleven answers folded into the page.
#[must_use]
pub fn assemble<S: core::hash::BuildHasher>(
    window: &Window,
    site_host: Option<&str>,
    answers: &HashMap<Section, QueryAnswer, S>,
    fetched_at: Timestamp,
) -> SiteAnalyticsView {
    let empty = QueryAnswer::default();
    let answer = |section: Section| answers.get(&section).unwrap_or(&empty);
    SiteAnalyticsView {
        range: window.range,
        from: date_of(window.first),
        to: date_of(window.last),
        site_host: site_host
            .filter(|host| host_literal(host).is_some())
            .map(str::to_owned),
        totals: totals(answer(Section::Totals), answer(Section::Conversions)),
        days: day_points(window, answer(Section::Daily)),
        pages: breakdown_rows(answer(Section::Pages)),
        referrers: breakdown_rows(answer(Section::Referrers)),
        utm_sources: breakdown_rows(answer(Section::UtmSources)),
        countries: breakdown_rows(answer(Section::Countries)),
        cities: city_rows(answer(Section::Cities)),
        devices: breakdown_rows(answer(Section::Devices)),
        browsers: breakdown_rows(answer(Section::Browsers)),
        systems: breakdown_rows(answer(Section::Systems)),
        fetched_at,
    }
}

// ------------------------------------------------------------------- client

/// Why PostHog gave no figures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostHogError {
    Transport(String),
    Api { status: u16, message: String },
    Malformed(String),
}

impl core::fmt::Display for PostHogError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Transport(detail) => write!(f, "PostHog unreachable: {detail}"),
            Self::Api { status, message } => write!(f, "PostHog answered {status}: {message}"),
            Self::Malformed(detail) => write!(f, "PostHog's answer did not parse: {detail}"),
        }
    }
}

struct Cached {
    at: Timestamp,
    view: SiteAnalyticsView,
}

/// The configured reader: who to ask, with what, about which project, and
/// the five-minute memory of what it last said.
#[derive(Clone)]
pub struct SiteAnalytics {
    http: reqwest::Client,
    key: PersonalKey,
    api_host: String,
    project_id: String,
    site_host: Option<String>,
    cache: Arc<Mutex<HashMap<Range, Cached>>>,
}

impl core::fmt::Debug for SiteAnalytics {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SiteAnalytics")
            .field("api_host", &self.api_host)
            .field("project_id", &self.project_id)
            .field("site_host", &self.site_host)
            .finish_non_exhaustive()
    }
}

impl PartialEq for SiteAnalytics {
    /// The same reader when it asks the same host about the same project with
    /// the same key and filter. `Config` derives equality; neither the client
    /// nor the cache has any worth comparing.
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.api_host == other.api_host
            && self.project_id == other.project_id
            && self.site_host == other.site_host
    }
}

impl Eq for SiteAnalytics {}

/// The one HTTP client this module makes, with both bounds, as `stripe`
/// builds its own.
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(core::time::Duration::from_secs(CALL_TIMEOUT_SECS))
        .connect_timeout(core::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .build()
        .unwrap_or_default()
}

impl SiteAnalytics {
    /// `api_host` is PostHog's app host (see [`DEFAULT_API_HOST`]), or a
    /// local double in a test; a trailing slash is dropped. `site_host` is
    /// the landing's host, which narrows every pageview query to it.
    #[must_use]
    pub fn new(
        key: PersonalKey,
        api_host: &str,
        project_id: String,
        site_host: Option<String>,
    ) -> Self {
        Self {
            http: http_client(),
            key,
            api_host: api_host.trim_end_matches('/').to_owned(),
            project_id,
            site_host,
            cache: Arc::default(),
        }
    }

    #[must_use]
    pub fn api_host(&self) -> &str {
        &self.api_host
    }

    #[must_use]
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    #[must_use]
    pub fn site_host(&self) -> Option<&str> {
        self.site_host.as_deref()
    }

    /// The page for one range: from memory when it is younger than
    /// [`CACHE_MS`], from PostHog otherwise. The lock is held across the
    /// fetch on purpose, so a second operator waits for the first answer
    /// instead of asking again.
    pub async fn view(
        &self,
        range: Range,
        now: Timestamp,
    ) -> Result<SiteAnalyticsView, PostHogError> {
        let mut cache = self.cache.lock().await;
        if let Some(held) = cache.get(&range) {
            if now.0 >= held.at.0 && now.0 - held.at.0 < CACHE_MS {
                return Ok(held.view.clone());
            }
        }
        let view = self.fetch(Window::ending(range, now), now).await?;
        cache.insert(
            range,
            Cached {
                at: now,
                view: view.clone(),
            },
        );
        drop(cache);
        Ok(view)
    }

    async fn fetch(
        &self,
        window: Window,
        now: Timestamp,
    ) -> Result<SiteAnalyticsView, PostHogError> {
        let site_host = self.site_host.as_deref();
        let answered: Vec<(Section, Result<QueryAnswer, PostHogError>)> =
            futures_util::stream::iter(Section::ALL)
                .map(|section| async move {
                    let body = request_body(section, &window, site_host);
                    (section, self.query(&body).await)
                })
                .buffered(CONCURRENT_QUERIES)
                .collect()
                .await;
        let mut answers = HashMap::with_capacity(answered.len());
        for (section, answer) in answered {
            answers.insert(section, answer?);
        }
        Ok(assemble(&window, site_host, &answers, now))
    }

    async fn query(&self, body: &serde_json::Value) -> Result<QueryAnswer, PostHogError> {
        let response = self
            .http
            .post(format!(
                "{}/api/projects/{}/query/",
                self.api_host, self.project_id
            ))
            .bearer_auth(self.key.expose())
            .json(body)
            .send()
            .await
            .map_err(|error| PostHogError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|error| PostHogError::Transport(error.to_string()))?;
        if !(200..300).contains(&status) {
            let message = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|refusal| {
                    refusal
                        .get("detail")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| text.chars().take(QUOTED_CHARS).collect());
            return Err(PostHogError::Api { status, message });
        }
        serde_json::from_str(&text).map_err(|error| PostHogError::Malformed(error.to_string()))
    }
}

// ------------------------------------------------------------------ handler

/// The query string: `range`, one of `7d`, `30d`, `90d`; `7d` when absent.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SiteQuery {
    #[serde(default)]
    pub range: Option<String>,
}

fn unconfigured() -> APIError {
    APIError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        APIErrorEntry::new("Site analytics is not configured").kind(APIErrorKind::Internal),
    )
}

fn invalid_range() -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new("Pick a range of 7d, 30d or 90d.").kind(APIErrorKind::Validation),
    )
}

fn upstream(error: &PostHogError) -> APIError {
    eprintln!("tam-api: site analytics: {error}");
    let message = match error {
        PostHogError::Api {
            status: 401 | 403, ..
        } => "PostHog refused this server's key. Check it has Query Read on the project.",
        PostHogError::Api { status: 429, .. } => {
            "PostHog is busy with this project's queries. Try again in a minute."
        }
        PostHogError::Api { .. } | PostHogError::Transport(_) | PostHogError::Malformed(_) => {
            "We could not read the figures from PostHog. Try again in a minute."
        }
    };
    APIError::new(
        StatusCode::BAD_GATEWAY,
        APIErrorEntry::new(message).kind(APIErrorKind::Internal),
    )
}

/// `GET /{version}/admin/analytics/site`.
pub async fn site(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Query(query): Query<SiteQuery>,
) -> Result<Json<SiteAnalyticsView>, APIError> {
    let range = match query.range.as_deref() {
        None | Some("") => Range::Week,
        Some(raw) => Range::parse(raw).ok_or_else(invalid_range)?,
    };
    let analytics = state
        .config
        .site_analytics
        .as_ref()
        .ok_or_else(unconfigured)?;
    analytics
        .view(range, (state.wall)())
        .await
        .map(Json)
        .map_err(|error| upstream(&error))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-04 09:00 in New Zealand (20:00 UTC the day before).
    const NOW: Timestamp = Timestamp(1_791_057_600_000);

    /// A whole query-API answer around `rows`, as PostHog sends it.
    fn answer(rows: serde_json::Value) -> QueryAnswer {
        let mut body = serde_json::json!({
            "columns": ["a", "b", "c"],
            "types": [],
            "hogql": "",
        });
        body["results"] = rows;
        serde_json::from_value(body).expect("the fixture is a query answer")
    }

    #[test]
    fn a_range_is_whole_new_zealand_days_ending_today() {
        let window = Window::ending(Range::Week, NOW);
        let dates = window.dates();
        assert_eq!(dates.len(), 7, "seven days");
        assert_eq!(dates.first().map(String::as_str), Some("2026-09-28"));
        assert_eq!(
            dates.last().map(String::as_str),
            Some("2026-10-04"),
            "today in Auckland, not yesterday in UTC"
        );
        assert_eq!(
            Window::ending(Range::Quarter, NOW).dates().len(),
            90,
            "ninety"
        );
    }

    #[test]
    fn ranges_parse_only_their_three_spellings() {
        assert_eq!(Range::parse("7d"), Some(Range::Week));
        assert_eq!(Range::parse("30d"), Some(Range::Month));
        assert_eq!(Range::parse("90d"), Some(Range::Quarter));
        assert_eq!(Range::parse("365d"), None, "not offered");
        assert_eq!(
            serde_json::to_value(Range::Month).expect("serialises"),
            serde_json::json!("30d"),
            "the wire spelling is the query spelling"
        );
    }

    #[test]
    fn the_daily_body_is_a_hogql_query_over_nz_days_on_the_landing_host() {
        let window = Window::ending(Range::Week, NOW);
        let body = request_body(Section::Daily, &window, Some("teachouse.io"));
        assert_eq!(body["query"]["kind"], "HogQLQuery", "the HogQL kind");
        assert_eq!(
            body["name"], "teachouse admin site: daily",
            "named for query_log"
        );
        assert_eq!(
            body["query"]["query"],
            "SELECT toString(toDate(toTimeZone(timestamp, 'Pacific/Auckland'))) AS day, \
             count() AS pageviews, count(DISTINCT person_id) AS visitors FROM events \
             WHERE event = '$pageview' AND timestamp >= now() - INTERVAL 8 DAY \
             AND toDate(toTimeZone(timestamp, 'Pacific/Auckland')) >= toDate('2026-09-28') \
             AND properties.$host = 'teachouse.io' GROUP BY day ORDER BY day LIMIT 8"
        );
    }

    #[test]
    fn breakdown_bodies_group_by_their_property_and_keep_twenty() {
        let window = Window::ending(Range::Month, NOW);
        for (section, property) in [
            (Section::Pages, "properties.$pathname"),
            (Section::Referrers, "properties.$referring_domain"),
            (Section::UtmSources, "properties.utm_source"),
            (Section::Countries, "properties.$geoip_country_name"),
            (Section::Devices, "properties.$device_type"),
            (Section::Browsers, "properties.$browser"),
            (Section::Systems, "properties.$os"),
        ] {
            let sql = hogql(section, &window, None);
            assert!(
                sql.starts_with(&format!("SELECT {property} AS label, ")),
                "{section:?} groups by {property}: {sql}"
            );
            assert!(sql.ends_with("LIMIT 20"), "{section:?} keeps twenty: {sql}");
            assert!(
                sql.contains("INTERVAL 31 DAY"),
                "{section:?} scans the range: {sql}"
            );
            assert!(!sql.contains("$host"), "no host, no host filter: {sql}");
        }
        assert!(
            hogql(Section::UtmSources, &window, None)
                .contains("properties.utm_source IS NOT NULL AND properties.utm_source != ''"),
            "visits with no UTM source are not a source"
        );
    }

    #[test]
    fn the_cities_body_names_the_country_beside_each_city() {
        let sql = hogql(Section::Cities, &Window::ending(Range::Week, NOW), None);
        assert!(
            sql.starts_with(
                "SELECT properties.$geoip_city_name AS city, properties.$geoip_country_name AS country, "
            ),
            "{sql}"
        );
        assert!(sql.contains("GROUP BY city, country"), "{sql}");
    }

    #[test]
    fn the_conversions_body_counts_both_events_without_a_host_filter() {
        let sql = hogql(
            Section::Conversions,
            &Window::ending(Range::Week, NOW),
            Some("teachouse.io"),
        );
        assert_eq!(
            sql,
            "SELECT countIf(event = 'signup_completed') AS signups, \
             countIf(event = 'cta_click') AS cta_clicks FROM events \
             WHERE event IN ('signup_completed', 'cta_click') \
             AND timestamp >= now() - INTERVAL 8 DAY \
             AND toDate(toTimeZone(timestamp, 'Pacific/Auckland')) >= toDate('2026-09-28')"
        );
    }

    #[test]
    fn a_host_that_could_carry_a_quote_is_never_spliced() {
        assert_eq!(
            host_literal("Teachouse.io"),
            Some("'teachouse.io'".to_owned())
        );
        assert_eq!(
            host_literal("localhost:8080"),
            Some("'localhost:8080'".to_owned())
        );
        assert_eq!(host_literal("x' OR 1=1 --"), None, "a quote is refused");
        assert_eq!(host_literal(""), None, "nothing is not a host");
        let sql = hogql(
            Section::Totals,
            &Window::ending(Range::Week, NOW),
            Some("x' OR '1'='1"),
        );
        assert!(
            !sql.contains("$host"),
            "an unsafe host filters nothing: {sql}"
        );
    }

    #[test]
    fn every_section_has_a_query() {
        let window = Window::ending(Range::Week, NOW);
        for section in Section::ALL {
            assert!(
                hogql(section, &window, None).starts_with("SELECT "),
                "{section:?} selects something"
            );
        }
    }

    #[test]
    fn breakdown_rows_read_nulls_as_unknown_and_counts_as_numbers() {
        let rows = breakdown_rows(&answer(serde_json::json!([
            ["/pricing", 41, 63],
            [null, 3, 4],
            ["  ", "7", 9.0],
        ])));
        assert_eq!(
            rows,
            vec![
                SiteRow {
                    label: Some("/pricing".to_owned()),
                    detail: None,
                    visitors: 41,
                    pageviews: 63
                },
                SiteRow {
                    label: None,
                    detail: None,
                    visitors: 3,
                    pageviews: 4
                },
                SiteRow {
                    label: None,
                    detail: None,
                    visitors: 7,
                    pageviews: 9
                },
            ]
        );
    }

    #[test]
    fn city_rows_carry_their_country() {
        let rows = city_rows(&answer(serde_json::json!([
            ["Auckland", "New Zealand", 12, 30],
            ["Wellington", null, 2, 2],
        ])));
        assert_eq!(rows.len(), 2, "both cities");
        assert_eq!(rows[0].label.as_deref(), Some("Auckland"));
        assert_eq!(rows[0].detail.as_deref(), Some("New Zealand"));
        assert_eq!((rows[0].visitors, rows[0].pageviews), (12, 30));
        assert_eq!(rows[1].detail, None, "a city with no country says so");
    }

    #[test]
    fn days_without_a_row_are_zero_and_rows_outside_the_window_are_dropped() {
        let window = Window::ending(Range::Week, NOW);
        let points = day_points(
            &window,
            &answer(serde_json::json!([
                ["2026-09-27", 99, 99],
                ["2026-09-29", 10, 6],
                ["2026-10-04", 4, 3],
            ])),
        );
        assert_eq!(points.len(), 7, "every day of the window");
        assert_eq!(points[0].day, "2026-09-28");
        assert_eq!(
            (points[0].pageviews, points[0].visitors),
            (0, 0),
            "a quiet day is zero"
        );
        assert_eq!((points[1].pageviews, points[1].visitors), (10, 6));
        assert_eq!((points[6].pageviews, points[6].visitors), (4, 3));
        assert!(
            points.iter().all(|point| point.pageviews != 99),
            "the day before the window is not in it"
        );
    }

    #[test]
    fn totals_read_the_two_single_row_answers() {
        let figures = totals(
            &answer(serde_json::json!([[120, 45]])),
            &answer(serde_json::json!([[3, 11]])),
        );
        assert_eq!(
            figures,
            SiteTotals {
                pageviews: 120,
                visitors: 45,
                signups: 3,
                cta_clicks: 11
            }
        );
        assert_eq!(
            totals(&QueryAnswer::default(), &QueryAnswer::default()),
            SiteTotals::default(),
            "no rows is nobody, not a fault"
        );
    }

    #[test]
    fn the_view_folds_every_answer_and_names_its_window() {
        let window = Window::ending(Range::Week, NOW);
        let mut answers = HashMap::new();
        answers.insert(Section::Totals, answer(serde_json::json!([[20, 8]])));
        answers.insert(
            Section::Countries,
            answer(serde_json::json!([["New Zealand", 7, 18]])),
        );
        let view = assemble(&window, Some("teachouse.io"), &answers, NOW);
        assert_eq!(view.range, Range::Week);
        assert_eq!(
            (view.from.as_str(), view.to.as_str()),
            ("2026-09-28", "2026-10-04")
        );
        assert_eq!(view.site_host.as_deref(), Some("teachouse.io"));
        assert_eq!(view.totals.visitors, 8, "the totals answer");
        assert_eq!(view.countries.len(), 1, "the countries answer");
        assert!(view.pages.is_empty(), "a section with no answer is empty");
        assert_eq!(view.days.len(), 7, "the chart still has every day");
    }

    #[test]
    fn the_key_never_reaches_a_debug_render() {
        let analytics = SiteAnalytics::new(
            PersonalKey::new("phx_secret_value".to_owned()),
            "https://eu.posthog.com/",
            "12345".to_owned(),
            None,
        );
        let rendered = format!("{analytics:?}");
        assert!(!rendered.contains("phx_secret_value"), "{rendered}");
        assert_eq!(
            analytics.api_host(),
            "https://eu.posthog.com",
            "the slash is dropped"
        );
    }
}
