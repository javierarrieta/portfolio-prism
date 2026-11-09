use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Mutex};
use chrono::{DateTime, Utc, NaiveDate};
use reqwest::Client;
use serde::Deserialize;
use anyhow::{Result, anyhow};
use crate::models::{Asset, HistoricalPrice};
use sqlx::sqlite::SqlitePool;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

const RATE_LIMIT_DELAY: Duration = Duration::from_millis(200);
const MAX_RETRIES: usize = 3;

fn f64_to_decimal(v: f64) -> Decimal {
    Decimal::from_f64(v).unwrap_or(Decimal::ZERO)
}

/// Insert detected split events as `SPLIT` transactions for an asset, deduplicated.
///
/// Each event is `(split_date, numerator, denominator)` where the ratio is
/// `numerator : denominator` (shares after : shares before). Splits dated before
/// `earliest_tx_date` are skipped (they would be no-ops since no lots existed).
/// A split already recorded for the asset on the same calendar day is not
/// duplicated. Returns the number of transactions inserted.
pub async fn apply_split_events(
    asset_id: i32,
    earliest_tx_date: Option<NaiveDate>,
    events: &[(NaiveDate, u32, u32)],
    pool: &SqlitePool,
) -> Result<usize, sqlx::Error> {
    let mut inserted = 0;
    for (split_date, numerator, denominator) in events {
        if *numerator == 0 || *denominator == 0 {
            continue;
        }
        if let Some(earliest) = earliest_tx_date {
            if *split_date < earliest {
                continue;
            }
        }

        let existing: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM transactions
             WHERE asset_id = ? AND type = 'SPLIT' AND date(date) = ?"
        )
        .bind(asset_id)
        .bind(*split_date)
        .fetch_one(pool)
        .await?;

        if existing.0 > 0 {
            continue;
        }

        let dt = split_date.and_hms_opt(0, 0, 0).unwrap().and_utc();
        sqlx::query(
            "INSERT INTO transactions (asset_id, type, quantity, price, fee, date)
             VALUES (?, 'SPLIT', ?, ?, '0', ?)"
        )
        .bind(asset_id)
        .bind(numerator.to_string())
        .bind(denominator.to_string())
        .bind(dt)
        .execute(pool)
        .await?;
        inserted += 1;
    }
    Ok(inserted)
}

#[derive(Deserialize)]
struct YahooChartResponse {
    chart: Chart,
}

#[derive(Deserialize)]
struct Chart {
    result: Vec<ChartResult>,
}

#[derive(Deserialize)]
struct ChartResult {
    indicators: Indicators,
    #[allow(dead_code)]
    timestamp: Vec<i64>,
    #[serde(default)]
    events: Option<ChartEvents>,
}

#[derive(Deserialize, Default)]
struct ChartEvents {
    #[serde(default)]
    splits: Option<std::collections::HashMap<i64, SplitEvent>>,
}

#[derive(Deserialize)]
struct SplitEvent {
    date: i64,
    numerator: f64,
    denominator: f64,
}

#[derive(Deserialize)]
struct Indicators {
    quote: Vec<Quote>,
}

#[derive(Deserialize)]
struct Quote {
    close: Vec<Option<f64>>,
}

#[derive(Deserialize)]
struct EodhdSearchResult {
    #[serde(rename = "previousClose")]
    previous_close: Option<f64>,
}

pub struct CurrencyService {
    client: Client,
    timeout: Duration,
    cache: Arc<RwLock<HashMap<(String, String, NaiveDate), Decimal>>>,
    last_request: Mutex<Option<Instant>>,
    eodhd_api_key: Option<String>,
}

impl CurrencyService {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent("investment-portfolio-manager/1.0")
                .build()
                .unwrap_or_else(|_| Client::new()),
            timeout: Duration::from_secs(10),
            cache: Arc::new(RwLock::new(HashMap::new())),
            last_request: Mutex::new(None),
            eodhd_api_key: std::env::var("EODHD_API_KEY").ok(),
        }
    }

    fn redact_url(url: &str) -> String {
        // EODHD embeds the API key as a query parameter; never surface it in
        // logs or errors. Yahoo URLs have no query string, so they pass through.
        match url.split_once("api_token=") {
            Some((prefix, rest)) => {
                let end = rest.find('&').unwrap_or(rest.len());
                format!("{prefix}api_token=***{}", &rest[end..])
            }
            None => url.to_string(),
        }
    }

    async fn fetch_with_retry(&self, url: &str) -> Result<reqwest::Response> {
        let mut retry_delay = Duration::from_secs(1);

        for attempt in 0..MAX_RETRIES {
            {
                let last = self.last_request.lock().await;
                if let Some(instant) = *last {
                    let elapsed = instant.elapsed();
                    if elapsed < RATE_LIMIT_DELAY {
                        let wait = RATE_LIMIT_DELAY - elapsed;
                        drop(last);
                        tokio::time::sleep(wait).await;
                    }
                }
            }

            let response = self.client.get(url).timeout(self.timeout).send().await;

            {
                let mut last = self.last_request.lock().await;
                *last = Some(Instant::now());
            }

            match response {
                Ok(resp) if resp.status() == 429 => {
                    eprintln!(
                        "WARN: Yahoo Finance returned 429 (attempt {}/{}), retrying in {:?}",
                        attempt + 1,
                        MAX_RETRIES,
                        retry_delay
                    );
                    tokio::time::sleep(retry_delay).await;
                    retry_delay *= 2;
                    continue;
                }
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    if attempt < MAX_RETRIES - 1 {
                        eprintln!(
                            "WARN: Request to {} failed (attempt {}/{}): {}, retrying in {:?}",
                            Self::redact_url(url),
                            attempt + 1,
                            MAX_RETRIES,
                            e,
                            retry_delay
                        );
                        tokio::time::sleep(retry_delay).await;
                        retry_delay *= 2;
                        continue;
                    }
                    return Err(anyhow!(
                        "Request to {} failed after {} retries: {}",
                        Self::redact_url(url),
                        MAX_RETRIES,
                        e
                    ));
                }
            }
        }

        Err(anyhow!("Max retries exceeded for {}", Self::redact_url(url)))
    }

    pub async fn get_rate(&self, from_curr: &str, to_curr: &str, date: DateTime<Utc>) -> Result<Decimal> {
        let date_naive = date.date_naive();
        let cache_key = (from_curr.to_string(), to_curr.to_string(), date_naive);

        {
            let cache = self.cache.read().await;
            if let Some(rate) = cache.get(&cache_key) {
                return Ok(*rate);
            }
        }

        if from_curr == to_curr {
            return Ok(Decimal::ONE);
        }

        let symbol = format!("{}={}", from_curr, to_curr);
        let url = format!("https://query1.finance.yahoo.com/v8/finance/chart/{}", symbol);

        let response = self.fetch_with_retry(&url).await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Yahoo Finance returned status {} for symbol {}",
                response.status(),
                symbol
            ));
        }

        let yahoo_resp: YahooChartResponse = response.json().await?;

        let result = yahoo_resp.chart.result.first()
            .ok_or_else(|| anyhow!("No result found for symbol {}", symbol))?;

        let close_prices = &result.indicators.quote.first()
            .ok_or_else(|| anyhow!("No quote data found for symbol {}", symbol))?
            .close;

        let rate = close_prices.iter().rev()
            .find_map(|x| x.map(f64_to_decimal))
            .ok_or_else(|| anyhow!("No close price found for symbol {}", symbol))?;

        let mut cache = self.cache.write().await;
        cache.insert(cache_key, rate);

        Ok(rate)
    }

    pub async fn get_price(&self, symbol: &str, isin: Option<&str>, pool: &SqlitePool) -> Decimal {
        let today = chrono::Utc::now().date_naive();
        let existing = sqlx::query_as::<_, HistoricalPrice>(
            "SELECT * FROM historical_prices WHERE symbol = ? AND date = ?"
        )
        .bind(symbol)
        .bind(today)
        .fetch_optional(pool)
        .await;

        if let Ok(Some(hp)) = existing {
            return crate::db_types::str_to_decimal(&hp.close_price);
        }

        let price = self.fetch_yahoo_price(symbol).await;
        if price > Decimal::ZERO {
            self.cache_price(symbol, today, price, pool).await;
            return price;
        }

        if let Some(isin) = isin {
            if !isin.is_empty() {
                if let Some(eodhd_key) = &self.eodhd_api_key {
                    let price = self.fetch_eodhd_price(isin, eodhd_key).await;
                    if price > Decimal::ZERO {
                        self.cache_price(symbol, today, price, pool).await;
                        return price;
                    }
                }
            }
        }

        Decimal::ZERO
    }

    async fn fetch_yahoo_price(&self, symbol: &str) -> Decimal {
        let url = format!("https://query1.finance.yahoo.com/v8/finance/chart/{}", symbol);
        let response = match self.fetch_with_retry(&url).await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("WARN: Failed to fetch Yahoo price for {}: {}", symbol, e);
                return Decimal::ZERO;
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body_preview = response.text().await.unwrap_or_default();
            eprintln!(
                "WARN: Yahoo Finance returned status {} for {}. Body: {}",
                status, symbol,
                &body_preview[..body_preview.len().min(500)]
            );
            return Decimal::ZERO;
        }

        let body_bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("WARN: Failed to read Yahoo response for {}: {}", symbol, e);
                return Decimal::ZERO;
            }
        };

        let yahoo_resp: YahooChartResponse = match serde_json::from_slice(&body_bytes) {
            Ok(r) => r,
            Err(e) => {
                let body_preview = String::from_utf8_lossy(&body_bytes);
                eprintln!(
                    "WARN: Failed to parse Yahoo response for {}: {}. Body: {}",
                    symbol, e,
                    &body_preview[..body_preview.len().min(500)]
                );
                return Decimal::ZERO;
            }
        };

        let result = yahoo_resp.chart.result.into_iter().next();
        let close_prices = match result {
            Some(r) => match r.indicators.quote.first() {
                Some(q) => q.close.clone(),
                None => {
                    eprintln!("WARN: No quote data for symbol: {}", symbol);
                    return Decimal::ZERO;
                }
            },
            None => {
                eprintln!("WARN: No price data for symbol: {}", symbol);
                return Decimal::ZERO;
            }
        };

        close_prices.iter().rev().find_map(|x| x.map(f64_to_decimal)).unwrap_or(Decimal::ZERO)
    }

    async fn fetch_eodhd_price(&self, isin: &str, api_key: &str) -> Decimal {
        let url = format!(
            "https://eodhd.com/api/search/{}?api_token={}&fmt=json",
            isin, api_key
        );

        let response = match self.fetch_with_retry(&url).await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("WARN: EODHD search failed for {}: {}", isin, e);
                return Decimal::ZERO;
            }
        };

        let status = response.status();
        if !status.is_success() {
            eprintln!("WARN: EODHD returned status {} for {}", status, isin);
            return Decimal::ZERO;
        }

        let results: Vec<EodhdSearchResult> = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("WARN: Failed to parse EODHD response for {}: {}", isin, e);
                return Decimal::ZERO;
            }
        };

        results.first()
            .and_then(|r| r.previous_close.map(f64_to_decimal))
            .unwrap_or(Decimal::ZERO)
    }

    async fn cache_price(&self, symbol: &str, date: NaiveDate, price: Decimal, pool: &SqlitePool) {
        let _ = sqlx::query(
            "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)
                 ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
        )
        .bind(symbol)
        .bind(date)
        .bind(crate::db_types::decimal_to_str(&price))
        .execute(pool)
        .await;
    }

    pub async fn get_historical_prices(
        &self,
        symbol: &str,
        start_date: NaiveDate,
        end_date: NaiveDate,
        pool: &SqlitePool,
    ) -> Result<Vec<(NaiveDate, Decimal)>> {
        let start_ts = start_date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let end_ts = end_date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={}&period2={}&interval=1d&includePrePost=false",
            symbol, start_ts, end_ts
        );

        let response = match self.fetch_with_retry(&url).await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("WARN: Failed to fetch historical prices for {}: {}", symbol, e);
                return self.get_historical_prices_from_db(symbol, start_date, end_date, pool).await;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body_preview = response.text().await.unwrap_or_default();
            eprintln!(
                "WARN: Yahoo Finance returned status {} for symbol {}. Body: {}",
                status,
                symbol,
                &body_preview[..body_preview.len().min(500)]
            );
            return self.get_historical_prices_from_db(symbol, start_date, end_date, pool).await;
        }

        let yahoo_resp: YahooChartResponse = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("WARN: Failed to parse historical Yahoo response for {}: {}", symbol, e);
                return self.get_historical_prices_from_db(symbol, start_date, end_date, pool).await;
            }
        };

        let result = yahoo_resp.chart.result.into_iter().next();
        let (timestamps, close_prices) = match result {
            Some(r) => {
                let ts = r.timestamp;
                let cp = r
                    .indicators
                    .quote
                    .first()
                    .map(|q| q.close.clone())
                    .unwrap_or_default();
                (ts, cp)
            }
            None => {
                eprintln!("WARN: No chart result for symbol: {}", symbol);
                return self.get_historical_prices_from_db(symbol, start_date, end_date, pool).await;
            }
        };

        let mut daily_prices = Vec::new();
        for (i, ts) in timestamps.iter().enumerate() {
            if i < close_prices.len() {
                if let Some(close) = close_prices[i] {
                    let price = f64_to_decimal(close);
                    if price > Decimal::ZERO {
                        let date = DateTime::from_timestamp(*ts, 0)
                            .map(|dt| dt.date_naive())
                            .unwrap_or_default();
                        daily_prices.push((date, price));
                    }
                }
            }
        }

        if !daily_prices.is_empty() {
            let mut tx = pool.begin().await?;
            for (date, price) in &daily_prices {
                let _ = sqlx::query(
                    "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)
                         ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
                )
                .bind(symbol)
                .bind(*date)
                .bind(crate::db_types::decimal_to_str(price))
                .execute(&mut *tx)
                .await;
            }
            let _ = tx.commit().await;
        }

        if daily_prices.is_empty() {
            return self.get_historical_prices_from_db(symbol, start_date, end_date, pool).await;
        }

        let db_prices = self.get_historical_prices_from_db(symbol, start_date, end_date, pool).await?;
        let yahoo_dates: std::collections::HashSet<NaiveDate> = daily_prices.iter().map(|(d, _)| *d).collect();

        let mut merged: Vec<(NaiveDate, Decimal)> = daily_prices;
        for (date, price) in db_prices {
            if !yahoo_dates.contains(&date) {
                merged.push((date, price));
            }
        }
        merged.sort_by_key(|(date, _)| *date);

        Ok(merged)
    }

    async fn get_historical_prices_from_db(
        &self,
        symbol: &str,
        start_date: NaiveDate,
        end_date: NaiveDate,
        pool: &SqlitePool,
    ) -> Result<Vec<(NaiveDate, Decimal)>> {
        let prices = sqlx::query_as::<_, HistoricalPrice>(
            "SELECT * FROM historical_prices WHERE symbol = ? AND date >= ? AND date <= ?"
        )
        .bind(symbol)
        .bind(start_date)
        .bind(end_date)
        .fetch_all(pool)
        .await?;

        Ok(prices.into_iter().map(|p| (p.date, crate::db_types::str_to_decimal(&p.close_price))).collect())
    }

    /// Fetch Yahoo Finance split history for an asset and insert any splits not
    /// already recorded as `SPLIT` transactions. Runs at most once per symbol per
    /// 30-day window (tracked in the `split_sync` table). Splits dated before the
    /// asset's first transaction are skipped. Returns the number inserted.
    pub async fn sync_splits_for_asset(
        &self,
        asset: &Asset,
        pool: &SqlitePool,
    ) -> Result<usize> {
        let today = Utc::now().date_naive();

        let last_sync: Option<(String,)> = sqlx::query_as(
            "SELECT last_synced_at FROM split_sync WHERE symbol = ?"
        )
        .bind(&asset.symbol)
        .fetch_optional(pool)
        .await?;

        if let Some((last,)) = last_sync {
            if let Ok(last_date) = NaiveDate::parse_from_str(&last, "%Y-%m-%d") {
                if (today - last_date).num_days() < 30 {
                    return Ok(0);
                }
            }
        }

        let earliest: Option<(NaiveDate,)> = sqlx::query_as(
            "SELECT MIN(date(date)) FROM transactions WHERE asset_id = ?"
        )
        .bind(asset.id)
        .fetch_optional(pool)
        .await?;
        let earliest_date = earliest.map(|(d,)| d);

        let end_ts = Utc::now().timestamp();
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1=0&period2={}&interval=1d&events=splits",
            asset.symbol, end_ts
        );

        let response = match self.fetch_with_retry(&url).await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("WARN: Failed to fetch splits for {}: {}", asset.symbol, e);
                return Ok(0);
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body_preview = response.text().await.unwrap_or_default();
            eprintln!(
                "WARN: Yahoo Finance returned status {} for splits of {}. Body: {}",
                status,
                asset.symbol,
                &body_preview[..body_preview.len().min(500)]
            );
            return Ok(0);
        }

        let yahoo_resp: YahooChartResponse = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("WARN: Failed to parse splits response for {}: {}", asset.symbol, e);
                return Ok(0);
            }
        };

        let mut events: Vec<(NaiveDate, u32, u32)> = Vec::new();
        if let Some(splits) = yahoo_resp
            .chart
            .result
            .first()
            .and_then(|r| r.events.as_ref())
            .and_then(|e| e.splits.as_ref())
        {
            for split in splits.values() {
                let date = DateTime::from_timestamp(split.date, 0)
                    .map(|dt| dt.date_naive())
                    .unwrap_or_default();
                let numerator = split.numerator.round() as u32;
                let denominator = split.denominator.round() as u32;
                events.push((date, numerator, denominator));
            }
        }
        events.sort_by_key(|(d, _, _)| *d);

        let inserted = apply_split_events(asset.id, earliest_date, &events, pool).await?;

        sqlx::query("INSERT OR REPLACE INTO split_sync (symbol, last_synced_at) VALUES (?, ?)")
            .bind(&asset.symbol)
            .bind(today)
            .execute(pool)
            .await?;

        Ok(inserted)
    }

    pub fn detect_currency(symbol: &str) -> String {
        let upper = symbol.to_uppercase();
        if upper.ends_with(".DE") || upper.ends_with(".F") || upper.ends_with(".FR") || upper.ends_with(".MC") {
            "EUR".to_string()
        } else if upper.ends_with(".L") {
            "GBP".to_string()
        } else if upper.ends_with(".T") {
            "JPY".to_string()
        } else if upper.ends_with(".HK") {
            "HKD".to_string()
        } else if upper.ends_with(".SX") || upper.ends_with(".SW") {
            "CHF".to_string()
        } else if upper.ends_with(".TO") {
            "CAD".to_string()
        } else if upper.ends_with(".AX") {
            "AUD".to_string()
        } else if upper.ends_with(".K") {
            "KRW".to_string()
        } else if upper.contains("USD") || upper.contains("BTC") || upper.contains("ETH") {
            "USD".to_string()
        } else {
            "USD".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn f64_to_decimal_round_trips_cleanly() {
        for (input, expected) in [
            (160.32_f64, "160.32"),
            (9.99_f64, "9.99"),
            (0.1_f64, "0.1"),
            (1.0_f64, "1"),
        ] {
            assert_eq!(
                f64_to_decimal(input).to_string(),
                expected,
                "input {input} should round-trip to {expected}"
            );
        }
    }

    #[test]
    fn test_redact_url_removes_api_token() {
        let url = "https://eodhd.com/api/search/AAPL.US?api_token=secret123&fmt=json";
        assert_eq!(
            CurrencyService::redact_url(url),
            "https://eodhd.com/api/search/AAPL.US?api_token=***&fmt=json"
        );
        assert!(!CurrencyService::redact_url(url).contains("secret123"));
    }

    #[test]
    fn test_redact_url_passes_through_plain_url() {
        let url = "https://query1.finance.yahoo.com/v8/finance/chart/AAPL=USD";
        assert_eq!(CurrencyService::redact_url(url), url);
    }

    #[tokio::test]
    async fn test_same_currency_returns_one() {
        let svc = CurrencyService::new();
        let date = DateTime::from_timestamp(1705312200, 0).unwrap();
        let result = svc.get_rate("USD", "USD", date).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Decimal::ONE);
    }

    #[tokio::test]
    async fn test_same_currency_returns_one_consistently() {
        let svc = CurrencyService::new();
        let date1 = DateTime::from_timestamp(1700000001, 0).unwrap();
        let date2 = DateTime::from_timestamp(1700000001, 0).unwrap();

        let r1 = svc.get_rate("GBP", "GBP", date1).await.unwrap();
        let r2 = svc.get_rate("GBP", "GBP", date2).await.unwrap();
        assert_eq!(r1, r2);
    }

    #[tokio::test]
    async fn test_get_price_uses_today_cached_price() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE historical_prices (symbol TEXT NOT NULL, date DATE NOT NULL, close_price TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE UNIQUE INDEX uq_historical_prices_symbol_date ON historical_prices(symbol, date)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let today = chrono::Utc::now().date_naive();
        let _ = sqlx::query(
            "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)"
        )
        .bind("TEST")
        .bind(today)
        .bind("42.0")
        .execute(&pool)
        .await;

        let svc = CurrencyService::new();
        let price = svc.get_price("TEST", None, &pool).await;
        assert_eq!(price, Decimal::from_str("42.0").unwrap());
    }

    #[tokio::test]
    async fn test_get_price_ignores_stale_cache() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE historical_prices (symbol TEXT NOT NULL, date DATE NOT NULL, close_price TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE UNIQUE INDEX uq_historical_prices_symbol_date ON historical_prices(symbol, date)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let stale_date = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let _ = sqlx::query(
            "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)"
        )
        .bind("TEST")
        .bind(stale_date)
        .bind("99.0")
        .execute(&pool)
        .await;

        let svc = CurrencyService::new();
        let price = svc.get_price("TEST", None, &pool).await;
        assert_ne!(price, Decimal::from_str("99.0").unwrap());
    }

    async fn make_split_tx_table(pool: &SqlitePool) {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS transactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                asset_id INTEGER NOT NULL,
                type TEXT NOT NULL,
                quantity TEXT NOT NULL,
                price TEXT NOT NULL,
                fee TEXT NOT NULL,
                date TEXT NOT NULL
            )"
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_apply_split_events_inserts_split_transactions() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        make_split_tx_table(&pool).await;

        let events = vec![
            (NaiveDate::from_ymd_opt(2020, 8, 31).unwrap(), 4u32, 1u32),
            (NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(), 4u32, 1u32),
        ];
        let earliest = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let inserted = apply_split_events(1, Some(earliest), &events, &pool).await.unwrap();

        assert_eq!(inserted, 2);

        let rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT type, quantity, price, fee FROM transactions WHERE asset_id = 1"
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        for (ty, qty, price, fee) in rows {
            assert_eq!(ty, "SPLIT");
            assert_eq!(qty, "4");
            assert_eq!(price, "1");
            assert_eq!(fee, "0");
        }
    }

    #[tokio::test]
    async fn test_apply_split_events_dedupes_manual_split_same_date() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        make_split_tx_table(&pool).await;

        let _ = sqlx::query(
            "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) VALUES (1, 'SPLIT', '4', '1', '0', ?)"
        )
        .bind(DateTime::from_timestamp(1717200000, 0).unwrap())
        .execute(&pool)
        .await
        .unwrap();

        let events = vec![(NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(), 4u32, 1u32)];
        let inserted = apply_split_events(1, None, &events, &pool).await.unwrap();

        assert_eq!(inserted, 0);
    }

    #[tokio::test]
    async fn test_apply_split_events_skips_splits_before_first_transaction() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        make_split_tx_table(&pool).await;

        let events = vec![
            (NaiveDate::from_ymd_opt(2019, 1, 1).unwrap(), 2u32, 1u32),
            (NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(), 4u32, 1u32),
        ];
        let earliest = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let inserted = apply_split_events(1, Some(earliest), &events, &pool).await.unwrap();

        assert_eq!(inserted, 1);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE asset_id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_parse_yahoo_splits_events() {
        let json = r#"{
            "chart": {
                "result": [{
                    "meta": {"symbol": "AAPL"},
                    "timestamp": [],
                    "indicators": {"quote": [{"close": []}]},
                    "events": {
                        "splits": {
                            "1402320600": {"date": 1402320600, "numerator": 7.0, "denominator": 1.0, "splitRatio": "7:1"},
                            "1598880600": {"date": 1598880600, "numerator": 4.0, "denominator": 1.0, "splitRatio": "4:1"}
                        }
                    }
                }]
            }
        }"#;

        let parsed: YahooChartResponse = serde_json::from_str(json).unwrap();
        let splits = parsed.chart.result[0].events.as_ref().unwrap().splits.as_ref().unwrap();
        assert_eq!(splits.len(), 2);
        let seven = splits.get(&1402320600).unwrap();
        assert_eq!(seven.numerator, 7.0);
        assert_eq!(seven.denominator, 1.0);
        let four = splits.get(&1598880600).unwrap();
        assert_eq!(four.numerator, 4.0);
    }

    #[test]
    fn test_parse_yahoo_response_without_events_still_works() {
        let json = r#"{
            "chart": {
                "result": [{
                    "meta": {"symbol": "AAPL"},
                    "timestamp": [],
                    "indicators": {"quote": [{"close": []}]}
                }]
            }
        }"#;

        let parsed: YahooChartResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.chart.result[0].events.is_none());
    }

    #[tokio::test]
    async fn test_sync_splits_skips_fetch_within_throttle_window() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        make_split_tx_table(&pool).await;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS split_sync (symbol TEXT PRIMARY KEY, last_synced_at DATE NOT NULL)"
        )
        .execute(&pool)
        .await
        .unwrap();

        // A recent sync record must short-circuit before any network call, so the
        // symbol here can be bogus — the function should not try to resolve it.
        let today = Utc::now().date_naive();
        sqlx::query("INSERT OR REPLACE INTO split_sync (symbol, last_synced_at) VALUES (?, ?)")
            .bind("THROTTLETEST")
            .bind(today)
            .execute(&pool)
            .await
            .unwrap();

        let asset = Asset {
            id: 1,
            portfolio_id: 1,
            symbol: "THROTTLETEST".to_string(),
            name: "Throttle".to_string(),
            asset_type: "STOCK".to_string(),
            sector: None,
            currency: "USD".to_string(),
            isin: None,
        };

        let svc = CurrencyService::new();
        let inserted = svc.sync_splits_for_asset(&asset, &pool).await.unwrap();
        assert_eq!(inserted, 0);

        // No split transaction was created for the asset.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE asset_id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_detect_currency_german_stock() {
        assert_eq!(CurrencyService::detect_currency("SAP.DE"), "EUR");
        assert_eq!(CurrencyService::detect_currency("SIE.DE"), "EUR");
    }

    #[test]
    fn test_detect_currency_madrid_stock() {
        assert_eq!(CurrencyService::detect_currency("BBVA.MC"), "EUR");
        assert_eq!(CurrencyService::detect_currency("SAN.MC"), "EUR");
    }

    #[test]
    fn test_detect_currency_uk_stock() {
        assert_eq!(CurrencyService::detect_currency("SHEL.L"), "GBP");
        assert_eq!(CurrencyService::detect_currency("BP.L"), "GBP");
    }

    #[test]
    fn test_detect_currency_japanese_stock() {
        assert_eq!(CurrencyService::detect_currency("7203.T"), "JPY");
    }

    #[test]
    fn test_detect_currency_btc() {
        assert_eq!(CurrencyService::detect_currency("BTC-USD"), "USD");
    }

    #[test]
    fn test_detect_currency_us_stock() {
        assert_eq!(CurrencyService::detect_currency("AAPL"), "USD");
        assert_eq!(CurrencyService::detect_currency("MSFT"), "USD");
    }

    #[test]
    fn test_detect_currency_case_insensitive() {
        assert_eq!(CurrencyService::detect_currency("sap.de"), "EUR");
        assert_eq!(CurrencyService::detect_currency("shel.l"), "GBP");
    }
}