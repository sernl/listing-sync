#![forbid(unsafe_code)]
use anyhow::{bail, Result};
use reqwest::Client;

pub async fn create_draft(client: &Client) -> Result<(i64, String)> {
    let candidates = [
        "https://www.tes.com/api/v2/resources",
        "https://www.tes.com/api/v2/resources/draft",
    ];
    for url in candidates {
        let resp = client.post(url).json(&serde_json::json!({})).send().await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        if let Some(id) = body.get("id").and_then(|v| v.as_i64()) {
            eprintln!("create via {url} -> {status} id={id}");
            return Ok((id, url.to_string()));
        }
    }
    bail!("no create-draft candidate returned an id")
}

pub async fn set_metadata(client: &Client, id: i64) -> Result<serde_json::Value> {
    let body = serde_json::json!({
        "title": "ZZ-SPIKE-DELETE-ME",
        "descriptionRaw": "Spike test. **Delete me.**",
        "descriptionRawType": "md",
        "categories": [{"id": 1000448}],
        "ageRanges": [4],
        "ages": [11, 12, 13, 14],
        "yearGroups": [],
        "mainType": 99009,
        "mainAge": 4
    });
    let resp: serde_json::Value = client
        .post(format!("https://www.tes.com/api/v2/resources/{id}/draft"))
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    eprintln!(
        "metadata set: title={} mainType={} ageRanges={}",
        resp["title"], resp["mainType"], resp["ageRanges"]
    );
    Ok(resp)
}

pub async fn delete_draft(client: &Client, id: i64) -> Result<Option<String>> {
    let url = format!("https://www.tes.com/api/v2/resources/{id}/draft");
    let status = client.delete(&url).send().await?.status();
    eprintln!("delete DELETE {url} -> {status}");
    Ok(if status.is_success() { Some(url) } else { None })
}

pub async fn read_back(client: &Client, id: i64) -> Result<serde_json::Value> {
    let candidates = [
        format!("https://www.tes.com/api/v2/resources/{id}/draft"),
        format!("https://www.tes.com/api/v2/resources/{id}"),
    ];
    for url in candidates {
        let resp = client.get(&url).send().await?;
        if resp.status().is_success() {
            let v: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
            if v.get("id").is_some() {
                eprintln!("read-back via GET {url}");
                return Ok(v);
            }
        }
    }
    bail!("no read-back GET candidate returned the resource")
}

pub fn diff(expected: &serde_json::Value, actual: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(obj) = expected.as_object() {
        for (k, v) in obj {
            if actual.get(k) != Some(v) {
                out.push(format!(
                    "{k}: expected {v}, got {}",
                    actual.get(k).cloned().unwrap_or(serde_json::Value::Null)
                ));
            }
        }
    }
    out
}
