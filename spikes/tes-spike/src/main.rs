#![forbid(unsafe_code)]
mod http;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let cookie = http::cookie_header("probes/local/tes-cookies.jar")?;
    let client = http::client(&cookie)?;
    println!("spike ready: {} cookies loaded", cookie.split(';').count());

    // Task 2: server-side session authenticates from Rust (milestone kill gate one)
    let tier: serde_json::Value = client
        .get("https://www.tes.com/api/tier/gmv/me")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "authed userId={} tier={}",
        tier["userId"], tier["tier"]["name"]
    );

    // Task 3: taxonomy read
    let node: serde_json::Value = client
        .get("https://www.tes.com/taxonomy/v4/GB/1001595")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    println!("taxonomy: {} ({})", node["description"], node["country"]);

    Ok(())
}
