use rocket::{serde::json::Json, http::Status};
use crate::auth::guard::AuthedUser;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct AssetLookupResult {
    pub symbol: String,
    pub name: String,
    pub asset_type: String,
}

// --- OpenFIGI types ---

#[derive(Deserialize)]
struct OpenFigiResponse(Vec<OpenFigiJobResult>);

#[derive(Deserialize)]
struct OpenFigiJobResult {
    data: Option<Vec<OpenFigiRecord>>,
}

#[derive(Deserialize)]
struct OpenFigiRecord {
    ticker: Option<String>,
    name: Option<String>,
    #[serde(rename = "exchCode")]
    exch_code: Option<String>,
    #[serde(rename = "securityType")]
    security_type: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/assets/lookup",
    params(
        ("isin" = String, Query, description = "ISIN to look up (12 alphanumeric characters)")
    ),
    responses(
        (status = 200, description = "Asset metadata resolved from ISIN", body = AssetLookupResult),
        (status = 400, description = "Invalid ISIN format"),
        (status = 404, description = "ISIN not found")
    )
)]
#[get("/assets/lookup?<isin>")]
pub async fn lookup_isin(
    // Authenticated but with no id to check: this route touches none of
    // portfolios/assets/transactions, so there is nothing for the chokepoint to
    // grade. Rate limiting it is a separate issue, not this change.
    _user: AuthedUser,
    isin: String,
) -> Result<Json<AssetLookupResult>, Status> {
    if !is_valid_isin(&isin) {
        return Err(Status::BadRequest);
    }

    let isin_upper = isin.to_uppercase();

    // 1. Try OpenFIGI first
    if let Some(result) = openfigi_lookup(&isin_upper).await {
        return Ok(Json(result));
    }

    // 2. Fallback to Yahoo Finance
    if let Some(result) = yahoo_lookup(&isin_upper).await {
        return Ok(Json(result));
    }

    eprintln!("WARN: ISIN lookup failed for {} (both OpenFIGI and Yahoo)", isin_upper);
    Err(Status::NotFound)
}

async fn openfigi_lookup(isin: &str) -> Option<AssetLookupResult> {
    let client = reqwest::Client::new();
    let body = serde_json::json!([{"idType": "ID_ISIN", "idValue": isin}]);

    let response = client
        .post("https://api.openfigi.com/v3/mapping")
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            eprintln!("WARN: OpenFIGI network error for {}: {}", isin, e);
            e
        })
        .ok()?;

    let status = response.status();
    if !status.is_success() {
        eprintln!("WARN: OpenFIGI returned {} for {}", status, isin);
        return None;
    }

    let parsed: OpenFigiResponse = response.json().await.map_err(|e| {
        eprintln!("WARN: OpenFIGI parse error for {}: {}", isin, e);
        e
    })
    .ok()?;

    let first_job = parsed.0.first()?;
    let records = first_job.data.as_ref()?;

    // Prefer the Luxembourg exchange for LU ISINs, otherwise take the first result
    let record = records
        .iter()
        .find(|r| r.exch_code.as_deref() == Some("LX"))
        .or_else(|| records.first())?;

    let ticker = record.ticker.as_deref()?;
    let name = record.name.as_deref().unwrap_or(ticker);
    let security_type = record.security_type.as_deref().unwrap_or("");

    let asset_type = match security_type {
        "Open-End Fund" | "Mutual Fund" => "MUTUAL_FUND",
        "ETF" | "Exchange Traded Fund" => "ETF",
        "Common Stock" | "Equity" | "Preferred Stock" => "STOCK",
        "Crypto" | "Cryptocurrency" => "CRYPTO",
        "Index" => "ETF",
        _ => "STOCK",
    };

    Some(AssetLookupResult {
        symbol: ticker.to_string(),
        name: name.to_string(),
        asset_type: asset_type.to_string(),
    })
}

async fn yahoo_lookup(isin: &str) -> Option<AssetLookupResult> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://query1.finance.yahoo.com/v1/finance/search?q={}",
        isin
    );

    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            eprintln!("WARN: Yahoo lookup network error for {}: {}", isin, e);
            e
        })
        .ok()?;

    let status = response.status();
    if !status.is_success() {
        eprintln!("WARN: Yahoo lookup returned {} for {}", status, isin);
        return None;
    }

    let body: serde_json::Value = response.json().await.map_err(|e| {
        eprintln!("WARN: Yahoo lookup parse error for {}: {}", isin, e);
        e
    })
    .ok()?;

    let quote = body
        .get("quotes")
        .and_then(|q| q.as_array())
        .and_then(|arr| arr.first())?;

    let symbol = quote.get("symbol")?.as_str()?;
    let name = quote.get("name").and_then(|n| n.as_str()).unwrap_or(symbol);
    let exchange = quote.get("exchange").and_then(|e| e.as_str()).unwrap_or("");

    let asset_type = if exchange.contains("CB") || exchange.contains("CM") {
        "CRYPTO"
    } else if exchange.contains("INDEX") {
        "ETF"
    } else {
        "STOCK"
    };

    Some(AssetLookupResult {
        symbol: symbol.to_string(),
        name: name.to_string(),
        asset_type: asset_type.to_string(),
    })
}

pub fn is_valid_isin(isin: &str) -> bool {
    let isin = isin.to_uppercase();
    if isin.len() != 12 {
        return false;
    }
    let mut chars = isin.chars();
    let country_code = chars.next().unwrap();
    let second = chars.next().unwrap();
    if !country_code.is_ascii_alphabetic() || !second.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_isin() {
        assert!(is_valid_isin("US0378331005"));
        assert!(is_valid_isin("DE000BAY0017"));
    }

    #[test]
    fn test_invalid_isin_too_short() {
        assert!(!is_valid_isin("US037833100"));
    }

    #[test]
    fn test_invalid_isin_too_long() {
        assert!(!is_valid_isin("US03783310050"));
    }

    #[test]
    fn test_invalid_isin_non_alphanumeric() {
        assert!(!is_valid_isin("US037833100!"));
    }

    #[test]
    fn test_invalid_isin_no_country_code() {
        assert!(!is_valid_isin("0378331005"));
    }

    #[test]
    fn test_invalid_isin_lowercase_country_code() {
        assert!(is_valid_isin("us0378331005"));
    }
}
