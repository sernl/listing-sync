#![forbid(unsafe_code)]
use anyhow::{Context, Result};
use base64::Engine;
use reqwest::Client;

/// Decode the base64 S3 POST policy to recover the bucket and the starts-with field names.
fn policy_info(policy_b64: &str) -> Result<(String, Vec<String>)> {
    let raw = base64::engine::general_purpose::STANDARD.decode(policy_b64)?;
    let v: serde_json::Value = serde_json::from_slice(&raw)?;
    let mut bucket = String::new();
    let mut starts_with = Vec::new();
    if let Some(conds) = v["conditions"].as_array() {
        for c in conds {
            if let Some(b) = c.get("bucket").and_then(|x| x.as_str()) {
                bucket = b.to_string();
            }
            if let Some(arr) = c.as_array() {
                if arr.first().and_then(|x| x.as_str()) == Some("starts-with") {
                    if let Some(field) = arr.get(1).and_then(|x| x.as_str()) {
                        starts_with.push(field.trim_start_matches('$').to_string());
                    }
                }
            }
        }
    }
    Ok((bucket, starts_with))
}

pub async fn upload_file(client: &Client, id: i64, path: &str) -> Result<bool> {
    let name = std::path::Path::new(path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let temp_id = "TEMP-SPIKE-0";

    let presign: serde_json::Value = client
        .post(format!(
            "https://www.tes.com/api/resources/v3/draft/{id}/attachment"
        ))
        .json(&serde_json::json!([{"name": name, "tempId": temp_id, "previewOption": 0}]))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let att = &presign[0];
    let params = att["s3pending"]["params"]
        .as_object()
        .context("presign params not an object")?;
    let policy = params
        .get("policy")
        .and_then(|v| v.as_str())
        .context("no policy in params")?;
    let (bucket, starts_with) = policy_info(policy)?;
    let s3_url = format!("https://{bucket}.s3.amazonaws.com/");
    eprintln!(
        "presign: param fields {:?}; bucket={bucket}; starts-with {:?}",
        params.keys().collect::<Vec<_>>(),
        starts_with
    );

    // Build the S3 POST form: signed params, then the starts-with fields, then the file last.
    let mut form = reqwest::multipart::Form::new();
    for (k, v) in params {
        if let Some(s) = v.as_str() {
            form = form.text(k.clone(), s.to_string());
        }
    }
    for field in &starts_with {
        let value = match field.as_str() {
            "name" => name.clone(),
            "Content-Type" => "application/pdf".to_string(),
            "Content-Disposition" => format!("inline; filename=\"{name}\""),
            _ => String::new(),
        };
        form = form.text(field.clone(), value);
    }
    let bytes = std::fs::read(path)?;
    form = form.part(
        "file",
        reqwest::multipart::Part::bytes(bytes).file_name(name.clone()),
    );

    let s3 = client.post(&s3_url).multipart(form).send().await?;
    let status = s3.status();
    if !status.is_success() {
        let body = s3.text().await.unwrap_or_default();
        eprintln!(
            "S3 POST {s3_url} -> {status}; body: {}",
            body.chars().take(400).collect::<String>()
        );
        anyhow::bail!("S3 upload failed: {status}");
    }
    eprintln!("S3 POST {s3_url} -> {status}");

    // Echo the full presign attachment object back (with type/isUploaded set) so the server
    // can verify the S3 object by the key carried in s3pending, exactly as the real client does.
    let mut echo = att.clone();
    echo["type"] = serde_json::json!("file");
    echo["isUploaded"] = serde_json::json!(true);
    let confirm: serde_json::Value = client
        .post(format!(
            "https://www.tes.com/api/resources/v3/draft/{id}/attachment"
        ))
        .json(&serde_json::json!([echo]))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let uploaded = confirm[0]["isUploaded"].as_bool().unwrap_or(false);
    eprintln!("attachment confirm: isUploaded={uploaded}");
    Ok(uploaded)
}
