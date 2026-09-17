//! Dated, informational exchange-rate observations. A quote never reprices a resource.

use std::{future::Future, pin::Pin};

use serde_json::Value;
use tam_types::Currency;

pub const ECB_DATA_URL: &str = "https://data-api.ecb.europa.eu/service/data/EXR/D.GBP+USD.EUR.SP00.A?lastNObservations=1&format=jsondata";
pub const ECB_NOTICE: &str = "ECB reference rates are informational, not transaction quotes. These proposals exclude marketplace fees and settlement costs. Accepting a proposal fixes its rate; refreshing the reference does not reprice resources.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceRate {
    pub source: Currency,
    pub target: Currency,
    pub rate_micros: i64,
    pub as_of: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceUnavailable(pub String);

impl std::fmt::Display for ReferenceUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ReferenceUnavailable {}

pub type ReferenceFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ReferenceRate, ReferenceUnavailable>> + Send + 'a>>;

pub trait ExchangeRateSource: Send + Sync {
    fn fetch(&self, source: Currency, target: Currency) -> ReferenceFuture<'_>;
}

fn invalid(what: &str) -> ReferenceUnavailable {
    ReferenceUnavailable(format!("ECB reference data: {what}"))
}

fn array<'a>(value: Option<&'a Value>, what: &str) -> Result<&'a [Value], ReferenceUnavailable> {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| invalid(what))
}

fn currency_code(currency: Currency) -> &'static str {
    match currency {
        Currency::Usd => "USD",
        Currency::Gbp => "GBP",
    }
}

fn dimension<'a>(
    dimensions: &'a [Value],
    indices: &[usize],
    id: &str,
) -> Result<&'a str, ReferenceUnavailable> {
    let position = dimensions
        .iter()
        .position(|dimension| dimension["id"] == id)
        .ok_or_else(|| invalid("missing series dimension"))?;
    let index = indices
        .get(position)
        .ok_or_else(|| invalid("incomplete series key"))?;
    dimensions[position]["values"]
        .get(*index)
        .and_then(|value| value["id"].as_str())
        .ok_or_else(|| invalid("series key names an unknown dimension value"))
}

fn period(document: &Value, index: usize) -> Result<&str, ReferenceUnavailable> {
    let dimensions = array(
        document.pointer("/structure/dimensions/observation"),
        "missing observation dimensions",
    )?;
    if dimensions.len() != 1 || dimensions[0]["id"] != "TIME_PERIOD" {
        return Err(invalid("unexpected observation dimensions"));
    }
    let date = dimensions[0]["values"]
        .get(index)
        .and_then(|value| value["id"].as_str())
        .ok_or_else(|| invalid("observation names an unknown date"))?;
    let parsed = sqlx::types::chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| invalid("invalid reference date"))?;
    if parsed.format("%Y-%m-%d").to_string() != date {
        return Err(invalid("reference date is not an ISO date"));
    }
    Ok(date)
}

/// Decode the two official EUR-denominated observations, then cross-divide.
/// Series ordering and observation indices are data, not fixed array offsets.
///
/// # Errors
/// Missing, duplicate, incompatible or differently dated observations refuse the quote.
pub fn parse_ecb(
    document: &Value,
    source: Currency,
    target: Currency,
) -> Result<ReferenceRate, ReferenceUnavailable> {
    if source == target {
        return Err(invalid("choose different source and target currencies"));
    }
    let dimensions = array(
        document.pointer("/structure/dimensions/series"),
        "missing series dimensions",
    )?;
    if dimensions.len() != 5 {
        return Err(invalid("unexpected series dimensions"));
    }
    let datasets = array(document.get("dataSets"), "missing dataset")?;
    if datasets.len() != 1 {
        return Err(invalid("ambiguous dataset"));
    }
    let series = datasets[0]["series"]
        .as_object()
        .ok_or_else(|| invalid("missing series"))?;
    let mut source_value = None;
    let mut target_value = None;
    for (key, series) in series {
        let indices = key
            .split(':')
            .map(str::parse::<usize>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| invalid("invalid series key"))?;
        if indices.len() != dimensions.len()
            || dimension(dimensions, &indices, "FREQ")? != "D"
            || dimension(dimensions, &indices, "CURRENCY_DENOM")? != "EUR"
            || dimension(dimensions, &indices, "EXR_TYPE")? != "SP00"
            || dimension(dimensions, &indices, "EXR_SUFFIX")? != "A"
        {
            return Err(invalid("series is not a daily EUR spot reference"));
        }
        let currency = dimension(dimensions, &indices, "CURRENCY")?;
        let observations = series["observations"]
            .as_object()
            .ok_or_else(|| invalid("missing observations"))?;
        if observations.len() != 1 {
            return Err(invalid(
                "expected exactly one latest observation per currency",
            ));
        }
        let (index, observation) = observations
            .iter()
            .next()
            .ok_or_else(|| invalid("missing observation"))?;
        let index = index
            .parse::<usize>()
            .map_err(|_| invalid("invalid observation index"))?;
        let date = period(document, index)?;
        let value = observation
            .get(0)
            .and_then(Value::as_number)
            .ok_or_else(|| invalid("missing numeric rate"))?;
        let micros = tam_domain::seller_rules::parse_rate(&value.to_string())
            .map_err(|_| invalid("rate is not a positive supported decimal"))?;
        let destination = if currency == currency_code(source) {
            &mut source_value
        } else if currency == currency_code(target) {
            &mut target_value
        } else {
            return Err(invalid("unexpected currency"));
        };
        if destination.replace((micros, date)).is_some() {
            return Err(invalid("duplicate currency observation"));
        }
    }
    let (source_micros, source_date) =
        source_value.ok_or_else(|| invalid("source currency is missing"))?;
    let (target_micros, target_date) =
        target_value.ok_or_else(|| invalid("target currency is missing"))?;
    if source_date != target_date {
        return Err(invalid("currency observations have different dates"));
    }
    let denominator = i128::from(source_micros);
    let numerator = i128::from(target_micros)
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_add(denominator.div_euclid(2)))
        .ok_or_else(|| invalid("cross rate overflows"))?;
    let rate_micros = i64::try_from(numerator.div_euclid(denominator))
        .map_err(|_| invalid("cross rate overflows"))?;
    if rate_micros <= 0 {
        return Err(invalid("cross rate rounds to zero"));
    }
    Ok(ReferenceRate {
        source,
        target,
        rate_micros,
        as_of: source_date.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document() -> Value {
        json!({
            "structure":{"dimensions":{
                "series":[
                    {"id":"FREQ","values":[{"id":"D"}]},
                    {"id":"CURRENCY","values":[{"id":"GBP"},{"id":"USD"}]},
                    {"id":"CURRENCY_DENOM","values":[{"id":"EUR"}]},
                    {"id":"EXR_TYPE","values":[{"id":"SP00"}]},
                    {"id":"EXR_SUFFIX","values":[{"id":"A"}]}
                ],
                "observation":[{"id":"TIME_PERIOD","values":[{"id":"2026-09-16"},{"id":"2026-09-15"}]}]
            }},
            "dataSets":[{"series":{
                "0:0:0:0:0":{"observations":{"0":[0.85740]}},
                "0:1:0:0:0":{"observations":{"0":[1.1537]}}
            }}]
        })
    }

    #[test]
    fn daily_eur_observations_produce_directional_reference_rates() {
        let dollars =
            parse_ecb(&document(), Currency::Usd, Currency::Gbp).expect("valid official shape");
        assert_eq!(dollars.rate_micros, 743_174);
        assert_eq!(dollars.as_of, "2026-09-16");
        let pounds =
            parse_ecb(&document(), Currency::Gbp, Currency::Usd).expect("valid reverse direction");
        assert_eq!(pounds.rate_micros, 1_345_580);
    }

    #[test]
    fn observations_from_different_days_cannot_be_combined_into_a_quote() {
        let mut value = document();
        value["dataSets"][0]["series"]["0:1:0:0:0"]["observations"] = json!({"1":[1.1537]});
        assert!(
            parse_ecb(&value, Currency::Usd, Currency::Gbp).is_err(),
            "a cross rate must use observations from the same day"
        );
    }
}
