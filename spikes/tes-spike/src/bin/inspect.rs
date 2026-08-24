#![forbid(unsafe_code)]
use anyhow::{Context, Result};
use serde_json::Value;
use tes_spike::{draft, http};

#[tokio::main]
async fn main() -> Result<()> {
    let cookie = http::cookie_header("probes/local/tes-cookies.jar")?;
    let client = http::client(&cookie)?;
    let full: Value = client
        .post("https://www.tes.com/api/v2/resources")
        .json(&serde_json::json!({}))
        .send()
        .await?
        .json()
        .await?;
    let id = full["id"].as_i64().context("no id")?;
    if let Some(o) = full.as_object() {
        let mut keys: Vec<_> = o.keys().cloned().collect();
        keys.sort();
        println!("draft keys ({}): {:?}", keys.len(), keys);
        for k in &keys {
            let kl = k.to_lowercase();
            if [
                "price",
                "licen",
                "copyright",
                "free",
                "status",
                "publish",
                "live",
                "state",
                "moderat",
            ]
            .iter()
            .any(|n| kl.contains(n))
            {
                println!("  {k} = {}", o[k]);
            }
        }
    }
    draft::delete_draft(&client, id).await?;
    println!("inspected and deleted draft {id}");
    Ok(())
}
