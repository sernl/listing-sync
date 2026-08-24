#![forbid(unsafe_code)]
//! One-off LIVE publish test. Creates a clearly-marked free test resource, publishes it,
//! and reports the result. Does NOT delete: the founder verifies, then deletion is a separate step.
use anyhow::{Context, Result};
use serde_json::Value;
use tes_spike::{attachment, draft, http};

async fn set_metadata(client: &reqwest::Client, id: i64) -> Result<()> {
    let body = serde_json::json!({
        "title": "TEST LISTING - automated check, will be deleted",
        "descriptionRaw": "This is an automated test listing created to verify publishing. It is free and will be deleted shortly. - EBMC Resources",
        "descriptionRawType": "md",
        "categories": [{"id": 1000448}],
        "ageRanges": [4],
        "ages": [11, 12, 13, 14],
        "yearGroups": [],
        "mainType": 99009,
        "mainAge": 4,
        "licence": "CC-BY"
    });
    let r: Value = client
        .post(format!("https://www.tes.com/api/v2/resources/{id}/draft"))
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    eprintln!("metadata set: title={}", r["title"]);
    Ok(())
}

async fn is_published(client: &reqwest::Client, id: i64) -> Result<(bool, Value)> {
    let v = draft::read_back(client, id).await?;
    let published = v["draft"].as_bool() == Some(false)
        || v["isPublic"].as_bool() == Some(true)
        || v["isDraftOnly"].as_bool() == Some(false) && v["draft"].as_bool() != Some(true);
    Ok((published, v))
}

async fn try_publish(client: &reqwest::Client, id: i64) -> Result<Option<String>> {
    let candidates: [(&str, String, Value); 4] = [
        (
            "POST",
            format!("https://www.tes.com/api/v2/resources/{id}/publish"),
            Value::Null,
        ),
        (
            "POST",
            format!("https://www.tes.com/api/v2/resources/{id}/submit"),
            Value::Null,
        ),
        (
            "POST",
            format!("https://www.tes.com/api/v2/resources/{id}/live"),
            Value::Null,
        ),
        (
            "PUT",
            format!("https://www.tes.com/api/v2/resources/{id}"),
            serde_json::json!({"draft": false}),
        ),
    ];
    for (m, url, body) in candidates {
        let req = match m {
            "PUT" => client.put(&url).json(&body),
            _ => client.post(&url).json(&serde_json::json!({})),
        };
        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        eprintln!(
            "publish {m} {url} -> {status}; body: {}",
            text.chars().take(300).collect::<String>()
        );
        if status.is_success() {
            let (pub_ok, _) = is_published(client, id).await?;
            if pub_ok {
                return Ok(Some(format!("{m} {url}")));
            }
        }
    }
    Ok(None)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cookie = http::cookie_header("probes/local/tes-cookies.jar")?;
    let client = http::client(&cookie)?;

    let (id, _) = draft::create_draft(&client).await?;
    println!("draft id={id}");
    set_metadata(&client, id).await?;
    let uploaded =
        attachment::upload_file(&client, id, "spikes/tes-spike/fixtures/test.pdf").await?;
    println!("file uploaded={uploaded}");

    if std::env::var("TES_LIVE_PUBLISH").as_deref() != Ok("1") {
        println!("draft {id} populated; set TES_LIVE_PUBLISH=1 to publish live. Not publishing.");
        return Ok(());
    }
    match try_publish(&client, id).await? {
        Some(via) => {
            let (_, v) = is_published(&client, id).await?;
            println!("PUBLISHED id={id} via {via}");
            println!(
                "state: draft={} isPublic={} review={} title={}",
                v["draft"], v["isPublic"], v["review"], v["title"]
            );
            println!("dashboard: https://www.tes.com/teaching-resources/dashboard/resource-management/uploads");
            println!("NOT deleting; awaiting founder confirmation.");
        }
        None => {
            println!("NO publish candidate succeeded; the resource remains a draft id={id}.");
            println!(
                "Read the responses above for the required fields (licence/price), then iterate."
            );
            // leave as draft; safe. Report id so we can delete or retry.
        }
    }
    Ok(())
}
