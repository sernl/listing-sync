#![forbid(unsafe_code)]
mod attachment;
mod draft;
mod http;
mod outcome;
use anyhow::Result;

async fn run_path(client: &reqwest::Client, id: i64) -> Result<()> {
    let saved = draft::set_metadata(client, id).await?;
    println!(
        "metadata committed: {:?}",
        outcome::classify(200, &saved.to_string(), id)
    );

    let uploaded =
        attachment::upload_file(client, id, "spikes/tes-spike/fixtures/tiny.pdf").await?;
    println!("file uploaded: {uploaded}");

    let actual = draft::read_back(client, id).await?;
    let expected = serde_json::json!({"title": "ZZ-SPIKE-DELETE-ME", "mainType": 99009});
    let mismatches = draft::diff(&expected, &actual);
    println!("read-back mismatches: {mismatches:?}");
    anyhow::ensure!(
        mismatches.is_empty(),
        "read-back diff failed: {mismatches:?}"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cookie = http::cookie_header("probes/local/tes-cookies.jar")?;
    let client = http::client(&cookie)?;
    println!("spike ready: {} cookies loaded", cookie.split(';').count());

    let (id, endpoint) = draft::create_draft(&client).await?;
    println!("CREATED draft id={id} via {endpoint}");

    let path = run_path(&client, id).await;

    // guaranteed cleanup regardless of path outcome
    match draft::delete_draft(&client, id).await {
        Ok(Some(via)) => println!("DELETED draft id={id} via {via}"),
        Ok(None) => {
            println!("WARNING: draft id={id} not deleted; remove ZZ-SPIKE-DELETE-ME by hand")
        }
        Err(e) => println!("WARNING: delete errored for id={id}: {e}; remove by hand"),
    }
    path
}
