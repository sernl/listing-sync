#![forbid(unsafe_code)]
use anyhow::{Context, Result};
use std::time::Duration;

/// Build a Cookie header from the gitignored Netscape jar (fields: domain flag path secure expiry name value).
pub fn cookie_header(jar_path: &str) -> Result<String> {
    let text = std::fs::read_to_string(jar_path).with_context(|| format!("read {jar_path}"))?;
    let mut pairs = Vec::new();
    for line in text.lines() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() >= 7 {
            pairs.push(format!("{}={}", f[5], f[6]));
        }
    }
    anyhow::ensure!(!pairs.is_empty(), "no cookies parsed from {jar_path}");
    Ok(pairs.join("; "))
}

pub fn client(cookie: &str) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::COOKIE,
        reqwest::header::HeaderValue::from_str(cookie)?,
    );
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static("Mozilla/5.0"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .context("build client")
}
