//! Fetch the four standards frameworks from the Common Standards Project and
//! write the committed ingest under `docs/design/data/standards`.
//!
//! The only crate in this pair that touches the network, and it touches
//! exactly one host: `api.commonstandardsproject.com`, unauthenticated, plain
//! JSON over HTTPS, no browser and no automation. It contacts no marketplace.
//!
//! Usage:
//!
//! ```text
//! tam-standards-fetch <out-dir> <fetched-at-rfc3339>
//! ```
//!
//! The timestamp is an argument rather than a clock read, for the reason the
//! lint that forbids the clock read states: time enters as data. Two runs over
//! the same upstream bytes and the same timestamp produce byte-identical
//! files, which is what makes the manifest's content hash a check rather than
//! a record.

#![forbid(unsafe_code)]

mod csp;

use std::collections::BTreeSet;
use std::time::Duration;

use tam_standards::{
    content_hash, model::FRAMEWORKS, render_jsonl, Framework, FrameworkManifest, Manifest, FORMAT,
};

use crate::csp::{declared_licences, Envelope, FrameworkNodes, JurisdictionDetail, SetDetail};

type Failure = Box<dyn std::error::Error>;

const API: &str = "https://api.commonstandardsproject.com";

/// The mirror's jurisdiction identifier per framework.
const JURISDICTIONS: [(Framework, &str); 4] = [
    (Framework::Ccss, "67810E9EF6944F9383DCC602A3484C23"),
    (Framework::Ngss, "71E5AA409D894EB0B43A8CD82F727BFE"),
    (Framework::Teks, "28903EF2A9F9469C9BF592D4D0BE10F8"),
    (Framework::VaSol, "27D1AF28D2F54DD3A69C057DDA772BD0"),
];

/// Which of a jurisdiction's sets are ingested.
///
/// Common Core and NGSS are taken whole: their jurisdictions hold only their
/// own frameworks, thirty-eight and sixty-four sets, and the duplicates among
/// them collapse on the mirror's own node identifiers.
///
/// Texas and Virginia cannot be. Their jurisdictions carry every vintage the
/// mirror ever held -- Texas alone offers `Science (2010-2017)`,
/// `Science (2017-2020)` and `Science (2020-)` side by side -- and every
/// subject, including 190 CTE sets. So the subject label is matched against
/// the current cycle of the four core academic subjects a teaching-resource
/// seller tags at, which is also the scope the sources research measured its
/// expected counts against. A retired vintage is not ingested: a listing
/// tagged to a retired code is the migration problem the update strategy
/// names, and seeding one deliberately would manufacture it.
///
/// The trailing space in one Texas label is the mirror's, not a typo here.
const TEKS_SUBJECTS: [&str; 7] = [
    "English Language Arts and Reading (2017-)",
    "English Language Arts and Reading (2017-) ",
    "Mathematics (2012-)",
    "Mathematics (2025-)",
    "Science (2020-)",
    "Social Studies (2020-)",
    "Social Studies (2024-)",
];

const VA_SOL_SUBJECTS: [&str; 4] = [
    "English (2024-)",
    "History and Social Science (2023-)",
    "Mathematics (2023-)",
    "Science (2018-)",
];

fn wanted(framework: Framework, subject: Option<&str>) -> bool {
    match framework {
        Framework::Ccss | Framework::Ngss => true,
        Framework::Teks => subject.is_some_and(|label| TEKS_SUBJECTS.contains(&label)),
        Framework::VaSol => subject.is_some_and(|label| VA_SOL_SUBJECTS.contains(&label)),
    }
}

/// Reject a timestamp that is not the shape the manifest promises, here rather
/// than after it has been written into every file.
fn checked_timestamp(argument: &str) -> Result<String, Failure> {
    let shaped = argument.len() == 20
        && argument.ends_with('Z')
        && argument
            .as_bytes()
            .iter()
            .zip(b"0000-00-00T00:00:00Z")
            .all(|(found, pattern)| match pattern {
                b'0' => found.is_ascii_digit(),
                other => found == other,
            });
    if !shaped {
        return Err(format!(
            "`{argument}` is not an RFC 3339 UTC timestamp; expected 2026-09-03T12:00:00Z"
        )
        .into());
    }
    Ok(argument.to_owned())
}

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, Failure> {
    let response = client.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("GET {url} answered {status}").into());
    }
    Ok(response.json().await?)
}

async fn ingest(
    client: &reqwest::Client,
    framework: Framework,
    jurisdiction_id: &str,
) -> Result<FrameworkNodes, Failure> {
    let url = format!("{API}/api/v1/jurisdictions/{jurisdiction_id}");
    let detail: Envelope<JurisdictionDetail> = get_json(client, &url).await?;

    let mut chosen: Vec<String> = detail
        .data
        .standard_sets
        .iter()
        .filter(|set| wanted(framework, set.subject.as_deref()))
        .map(|set| set.id.clone())
        .collect();
    chosen.sort();
    chosen.dedup();

    eprintln!("{framework}: {} sets selected", chosen.len());

    let mut nodes = FrameworkNodes::default();
    for (position, id) in chosen.iter().enumerate() {
        let set_url = format!("{API}/api/v1/standard_sets/{id}");
        let set: Envelope<SetDetail> = get_json(client, &set_url).await?;
        nodes.absorb(set.data, set_url);
        if position % 20 == 19 {
            eprintln!("{framework}: {} of {} sets", position + 1, chosen.len());
        }
        // The mirror answered 102 sequential requests at roughly five per
        // second without throttling, which is evidence of absence at that rate
        // only; this stays under it.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(nodes)
}

fn arguments() -> Vec<String> {
    std::env::args().skip(1).collect()
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let arguments = arguments();
    let out_dir = arguments
        .first()
        .ok_or("usage: tam-standards-fetch <out-dir> <fetched-at>")?;
    let fetched_at = checked_timestamp(
        arguments
            .get(1)
            .ok_or("missing fetched-at; pass `date -u +%Y-%m-%dT%H:%M:%SZ`")?,
    )?;

    let client = reqwest::Client::builder()
        .user_agent("tam-standards-fetch/0 (listing-sync; standards ingestion)")
        .timeout(Duration::from_mins(1))
        .connect_timeout(Duration::from_secs(15))
        .build()?;

    let mut frameworks = Vec::new();
    for framework in FRAMEWORKS {
        let jurisdiction_id = JURISDICTIONS
            .iter()
            .find(|(named, _)| *named == framework)
            .map(|(_, id)| *id)
            .ok_or("no jurisdiction is registered for a framework")?;
        let nodes = ingest(&client, framework, jurisdiction_id).await?;

        let mut normalised = nodes.normalise(framework);
        let addressable_count = normalised.iter().filter(|node| node.addressable).count();
        let addressable_codes = normalised
            .iter()
            .filter(|node| node.addressable)
            .filter_map(|node| node.code.as_deref())
            .collect::<BTreeSet<&str>>()
            .len();
        let rendered = render_jsonl(&mut normalised)?;
        let file = format!("{}.jsonl", framework.file_stem());
        let path = std::path::Path::new(out_dir).join(&file);
        std::fs::write(&path, rendered.as_bytes())?;

        eprintln!(
            "{framework}: {} nodes, {} distinct codes, {} addressable ({} distinct codes), {} duplicate rows collapsed, {} bytes",
            normalised.len(),
            nodes.distinct_codes(),
            addressable_count,
            addressable_codes,
            nodes.duplicate_nodes,
            rendered.len(),
        );

        frameworks.push(FrameworkManifest {
            framework,
            jurisdiction: framework.jurisdiction(),
            fetched_at: fetched_at.clone(),
            source: "commonstandardsproject".to_owned(),
            source_api: API.to_owned(),
            file,
            file_bytes: u64::try_from(rendered.len())?,
            file_sha256: content_hash(rendered.as_bytes()),
            node_count: normalised.len(),
            distinct_code_count: nodes.distinct_codes(),
            addressable_count,
            licences: declared_licences(nodes.sets()),
            sets: nodes.sets().to_vec(),
        });
    }

    let manifest = Manifest {
        format: FORMAT.to_owned(),
        generated_at: fetched_at,
        frameworks,
    };
    let manifest_path = std::path::Path::new(out_dir).join("manifest.json");
    let mut rendered = serde_json::to_string_pretty(&manifest)?;
    rendered.push('\n');
    std::fs::write(&manifest_path, rendered.as_bytes())?;
    eprintln!("wrote {}", manifest_path.display());

    Ok(())
}
