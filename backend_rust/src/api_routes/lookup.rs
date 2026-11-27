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

    match resolve_isin(&isin.to_uppercase()).await {
        Some(result) => Ok(Json(result)),
        None => Err(Status::NotFound),
    }
}

/// The two-provider fallback chain, in one place.
///
/// The route and the agent's `lookup_asset` tool both need exactly this order — OpenFIGI
/// first, Yahoo as the fallback — and two copies would be two places where the order can
/// disagree. A tool that quietly preferred a different provider would answer an agent
/// differently from the UI, which is the kind of difference nobody notices until a
/// normalized symbol fails to match an existing asset.
pub async fn resolve_isin(isin_upper: &str) -> Option<AssetLookupResult> {
    // 1. Try OpenFIGI first
    if let Some(result) = openfigi_lookup(isin_upper).await {
        return Some(result);
    }

    // 2. Fallback to Yahoo Finance
    if let Some(result) = yahoo_lookup(isin_upper).await {
        return Some(result);
    }

    eprintln!("WARN: ISIN lookup failed for {} (both OpenFIGI and Yahoo)", isin_upper);
    None
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

/// Shape **and** the ISO 6166 check digit.
///
/// The digit is not decoration. An ISIN whose check digit is wrong cannot resolve at any
/// provider — OpenFIGI and Yahoo both answer "no such instrument" — so accepting one moves
/// the failure from a free local rejection to a paid network round trip that ends in a
/// vaguer error. Worse, `services::assets::insert` uses the ISIN as a cross-portfolio
/// uniqueness key, so garbage ISINs are garbage that participates in a uniqueness
/// constraint and can collide with each other.
///
/// The algorithm: expand letters to two digits each (`A`=10 … `Z`=35), then Luhn over the
/// expanded string, with the twelfth character as the check digit.
///
/// This is a tightening. Input that is twelve shape-valid characters but a wrong digit was
/// accepted before and is a 400 now. Real ISINs are unaffected — the vectors in the tests
/// below are live instruments, and the algorithm reproduces each of their digits from the
/// first eleven characters.
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
    if !chars.all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }

    let (body, check) = isin.split_at(11);
    let expected: u8 = match check.parse::<u8>() {
        Ok(d) => d,
        Err(_) => return false,
    };
    isin_check_digit(body) == Some(expected)
}

/// The ISO 6166 check digit for the first eleven characters, or `None` if they are not a
/// valid body.
fn isin_check_digit(body: &str) -> Option<u8> {
    // Letters become two decimal digits each, so the expanded form is longer than the input.
    let mut digits: Vec<u8> = Vec::with_capacity(body.len() + 4);
    for ch in body.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch as u8 - b'0');
        } else if ch.is_ascii_alphabetic() {
            let value = (ch as u8 - b'A') + 10;
            digits.push(value / 10);
            digits.push(value % 10);
        } else {
            return None;
        }
    }

    // Luhn, read from the right of the body: the digit immediately left of the check digit
    // is doubled, then every second one after that.
    let mut sum: u32 = 0;
    for (i, d) in digits.iter().rev().enumerate() {
        let mut d = u32::from(*d);
        if i % 2 == 0 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    Some(((10 - (sum % 10)) % 10) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live instruments, each verified against its own check digit. These are the proof that
    /// the tightening does not reject real ISINs: every one of these digits is reproduced
    /// from the first eleven characters, including the two whose bodies carry letters.
    #[test]
    fn accepts_real_instruments() {
        for isin in [
            "US0378331005", // Apple
            "US5949181045", // Microsoft
            "US88160R1014", // Tesla — body contains a letter
            "DE000BAY0017", // Bayer AG — body contains letters
            "DE0007164600", // Deutsche Telekom
            "FR0000131906", // Renault
            "GB0002634946", // Shell
            "IE00B3RBWM25", // Vanguard FTSE All-World — body contains letters
            "CH0038863350", // Novartis
            "JP3756600007", // Sony Group
        ] {
            assert!(is_valid_isin(isin), "{isin} is a real ISIN and must be accepted");
        }
    }

    #[test]
    fn rejects_a_wrong_check_digit() {
        // The shape is perfect; only the digit is wrong. This is the whole point: before the
        // digit was computed, every one of these passed.
        for bad in [
            "US0378331004", // Apple, digit decremented
            "US0378331006", // Apple, digit incremented
            "IE00B3RBWM24", // Vanguard, digit decremented
            "BTC123456789", // shape-valid, never issued
        ] {
            assert!(!is_valid_isin(bad), "{bad} must be rejected on its check digit");
        }
    }

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
