use tam_api::exchange_rates::{
    parse_ecb, ExchangeRateSource, ReferenceFuture, ReferenceUnavailable, ECB_DATA_URL,
};
use tam_types::Currency;

const REFERENCE_BYTES_MAX: usize = 256 * 1024;

pub(crate) struct EcbRates {
    client: reqwest::Client,
}

impl EcbRates {
    pub(crate) fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(core::time::Duration::from_secs(10))
                .connect_timeout(core::time::Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .https_only(true)
                .build()?,
        })
    }
}

impl ExchangeRateSource for EcbRates {
    fn fetch(&self, source: Currency, target: Currency) -> ReferenceFuture<'_> {
        Box::pin(async move {
            let mut response = self
                .client
                .get(ECB_DATA_URL)
                .header(reqwest::header::ACCEPT, "application/vnd.sdmx.data+json")
                .send()
                .await
                .map_err(|error| ReferenceUnavailable(format!("ECB request failed: {error}")))?;
            if !response.status().is_success() {
                return Err(ReferenceUnavailable(format!(
                    "ECB answered {}",
                    response.status()
                )));
            }
            let media = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(str::trim);
            if !media.is_some_and(|value| value == "application/json" || value.ends_with("+json")) {
                return Err(ReferenceUnavailable("ECB did not answer JSON".to_owned()));
            }
            if response
                .content_length()
                .is_some_and(|length| length > REFERENCE_BYTES_MAX as u64)
            {
                return Err(ReferenceUnavailable(
                    "ECB response exceeded the reference-data bound".to_owned(),
                ));
            }
            let mut bytes = Vec::with_capacity(16 * 1024);
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| ReferenceUnavailable(format!("ECB response failed: {error}")))?
            {
                if chunk.len() > REFERENCE_BYTES_MAX.saturating_sub(bytes.len()) {
                    return Err(ReferenceUnavailable(
                        "ECB response exceeded the reference-data bound".to_owned(),
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            let document = serde_json::from_slice(&bytes).map_err(|error| {
                ReferenceUnavailable(format!("ECB response is not valid JSON: {error}"))
            })?;
            parse_ecb(&document, source, target)
        })
    }
}
