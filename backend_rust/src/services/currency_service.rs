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

/// How far back a first fetch for a pair reaches. One wide window per pair, so a
/// year of dated lookups costs a single outbound call instead of one per distinct
/// transaction date.
pub(crate) const FX_SERIES_WINDOW_DAYS: i64 = 365;

/// How stale a close may be and still answer for a date. Thirty days is generous
/// for a currency pair, which trades nearly every day, and tight enough that a
/// delisted or mis-symbolled pair produces an error instead of a years-old rate
/// silently used as a tax figure.
pub(crate) const FX_LOOKBACK_DAYS: i64 = 30;

/// How stale a stored series may be and still answer for a window ending at
/// `end_date`. Four days covers a long market weekend; beyond that the window is
/// treated as incomplete and the provider is asked for the tail.
///
/// `pub` rather than `pub(crate)` so `tests/pricing_outage_test.rs` can place a
/// stored window exactly one day past this bound instead of hardcoding the number
/// and silently testing the wrong thing if it changes.
pub const PRICE_WINDOW_FRESH_DAYS: i64 = 4;

/// Yahoo's FX pair symbology. `EUR=USD` returns "No data found"; `EURUSD=X`
/// returns data. `get_rate` built the first form, which is why every asset whose
/// currency differed from the portfolio's base currency failed to price at all.
pub(crate) fn fx_symbol(from: &str, to: &str) -> String {
    format!("{}{}=X", from.to_uppercase(), to.to_uppercase())
}

/// The last close on or before `date`, within `lookback_days`. Returns the date
/// actually used alongside the value: a caller that reports a figure "as of" a
/// date must be able to say which date priced it, and silent substitution is the
/// failure this whole change exists to remove.
///
/// Pure and order-independent on purpose, so the rule is testable without a pool
/// or a network.
pub(crate) fn select_rate(
    series: &[(NaiveDate, Decimal)],
    date: NaiveDate,
    lookback_days: i64,
) -> Option<(NaiveDate, Decimal)> {
    let earliest = date.checked_sub_signed(chrono::Duration::days(lookback_days))?;
    series
        .iter()
        .filter(|(d, _)| *d <= date && *d >= earliest)
        .max_by_key(|(d, _)| *d)
        .cloned()
}

/// Does a stored window answer for `end_date`? Only if its newest row is within
/// `PRICE_WINDOW_FRESH_DAYS` of it; an empty window is never fresh.
///
/// Pure for the same reason as [`select_rate`]: the rule is what needs testing, and
/// a test of it must not need a pool, a provider, or a network.
///
/// `pub(crate)` because the dated tax report resolves one close per asset before
/// replaying, and it has to decide "can the database answer?" the *same* way this
/// function does. Re-implementing the comparison in the route is how the route's read
/// order and this one drift apart.
pub(crate) fn stored_window_is_fresh(newest: Option<NaiveDate>, end_date: NaiveDate) -> bool {
    matches!(newest, Some(d) if (end_date - d).num_days() <= PRICE_WINDOW_FRESH_DAYS)
}

/// Combine what the provider returned for the tail of a window with the rows
/// already stored for its head. Provider rows win on a shared date; the result is
/// ascending by date.
///
/// Pure so the merge is testable too: it only runs in production when a real
/// provider answers, and this crate has no dev-dependencies to mock one with.
fn merge_series(
    fetched: Vec<(NaiveDate, Decimal)>,
    stored: Vec<(NaiveDate, Decimal)>,
) -> Vec<(NaiveDate, Decimal)> {
    let fetched_dates: std::collections::HashSet<NaiveDate> =
        fetched.iter().map(|(date, _)| *date).collect();
    let mut merged = fetched;
    for (date, price) in stored {
        if !fetched_dates.contains(&date) {
            merged.push((date, price));
        }
    }
    merged.sort_by_key(|(date, _)| *date);
    merged
}

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
    cache: Arc<RwLock<HashMap<(String, String), FxSeries>>>,
    last_request: Mutex<Option<Instant>>,
    eodhd_api_key: Option<String>,
    /// Chart-API host, without a trailing slash. See [`CurrencyService::with_base_url`].
    base_url: String,
}

/// One fetched window of a pair's dated closes. Keyed by pair, not by pair-and-date:
/// the old key (`(from, to, date)`) meant every distinct transaction date was its own
/// outbound call, and the value was never the date it claimed.
///
/// The cache is process-local and dies with the process, so it needs no key version:
/// entries written by the old buggy code cannot outlive a deploy. The persistent store
/// is `historical_prices`, and it is versioned by symbology — `EURUSD=X` rows cannot
/// have been written by the `EUR=USD` code path.
struct FxSeries {
    /// First date of the fetched window. A request earlier than this needs a new
    /// fetch rather than a wrong answer from a window that never covered it.
    window_start: NaiveDate,
    /// Last date the fetch covered — the `date` whose request produced this series.
    /// A cached series never holds closes after it, so it may not answer for a later
    /// date either. See [`FxSeries::rate_at`].
    window_end: NaiveDate,
    series: Vec<(NaiveDate, Decimal)>,
}

impl FxSeries {
    /// The close this window is allowed to answer for `date`, or `None` when the
    /// date falls outside the range the fetch actually covered. Both bounds are
    /// load-bearing: a date before `window_start` was never in the window, and a
    /// date after `window_end` would be priced by the window's last close, which
    /// stopped days short of it — inside the lookback, so nothing downstream would
    /// notice the substitution.
    fn rate_at(&self, date: NaiveDate, lookback_days: i64) -> Option<(NaiveDate, Decimal)> {
        if date < self.window_start || date > self.window_end {
            return None;
        }
        select_rate(&self.series, date, lookback_days)
    }
}

/// Bound on cached pairs. Unbounded is not an option in a 256Mi pod once past dates
/// became reachable; 64 pairs is far beyond any portfolio in this app, and the
/// eviction is a full clear rather than an LRU because a miss is one cheap call.
const MAX_FX_SERIES_CACHED: usize = 64;

/// A rate and the date that produced it. The date is not decoration: a report that
/// says "as of 2024-03-31" has to be able to state that a Friday close priced it.
#[derive(Debug, Clone, PartialEq)]
pub struct RateAt {
    pub rate: Decimal,
    pub date_used: NaiveDate,
}

/// "There is no close for this symbol" and "we could not ask" are different failures
/// with different user actions. `get_price` returned `Decimal::ZERO` for both, and
/// zero is a plausible price.
///
/// `std::error::Error` is what lets callers use `?` over it (notably `get_rate`, which
/// keeps its `anyhow::Result` return type).
#[derive(Debug, Clone, PartialEq)]
pub enum PricingError {
    NoData,
    ProviderUnavailable(String),
}

impl std::fmt::Display for PricingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PricingError::NoData => write!(f, "no price data available"),
            PricingError::ProviderUnavailable(reason) => write!(f, "price provider unavailable: {reason}"),
        }
    }
}

impl std::error::Error for PricingError {}

/// The error for "this pair has no close near that date", built separately from the
/// `ok_or_else` in `get_rate` so the message is testable without a provider. Public so
/// the integration test can assert on it; `select_rate` stays `pub(crate)`.
pub fn no_rate_error(symbol: &str, date: NaiveDate, lookback_days: i64) -> anyhow::Error {
    anyhow!(
        "No {} close on or before {} within {} days",
        symbol,
        date,
        lookback_days
    )
}

/// Chart-API host used when `YAHOO_BASE_URL` is unset.
const DEFAULT_YAHOO_BASE_URL: &str = "https://query1.finance.yahoo.com";

impl CurrencyService {
    pub fn new() -> Self {
        Self::with_base_url(
            &std::env::var("YAHOO_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_YAHOO_BASE_URL.to_string()),
        )
    }

    /// Build the service against a different chart-API host.
    ///
    /// This exists so the test suite can point every outbound pricing call at a
    /// local listener instead of the internet. `cargo test` used to make 21 outbound
    /// calls per run — measured, not guessed — and one of them was a provider-outage
    /// test that proved the outage by making a real request that happened to fail.
    /// A constructor rather than an
    /// env var in tests because this crate is edition 2024, where `std::env::set_var`
    /// is `unsafe`, and the environment is process-global, so parallel tests would
    /// race on it.
    ///
    /// A trailing slash is trimmed, so both forms work at the call site.
    pub fn with_base_url(base_url: &str) -> Self {
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
            base_url: base_url.trim_end_matches('/').to_string(),
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

    /// The FX rate to convert `from_curr` into `to_curr` for `date`: the last close
    /// on or before that date *within a window that actually reaches `date`*, within
    /// `FX_LOOKBACK_DAYS`, together with the date actually used.
    ///
    /// "A window that actually reaches `date`" is a real limit, not a figure of
    /// speech, and the two read paths enforce it differently. The cache records the
    /// window its fetch covered, so [`FxSeries::rate_at`](FxSeries::rate_at) rejects a
    /// date outside those bounds exactly. `historical_prices` records no coverage, so
    /// the database path uses a proxy: the newest stored row must be within
    /// `PRICE_WINDOW_FRESH_DAYS` of `date`, or the window is treated as incomplete and
    /// the provider is asked for the tail. A per-symbol coverage record is the
    /// follow-up that would replace the proxy.
    ///
    /// Read order is database first, provider second — the same order
    /// [`get_historical_prices`](Self::get_historical_prices) now uses, and for the same
    /// reason: it is what makes a past-dated conversion testable without a network.
    pub async fn get_rate(
        &self,
        from_curr: &str,
        to_curr: &str,
        date: DateTime<Utc>,
        pool: &SqlitePool,
    ) -> Result<RateAt> {
        let date_naive = date.date_naive();

        if from_curr.to_uppercase() == to_curr.to_uppercase() {
            return Ok(RateAt { rate: Decimal::ONE, date_used: date_naive });
        }

        let key = (from_curr.to_uppercase(), to_curr.to_uppercase());

        {
            let cache = self.cache.read().await;
            if let Some((date_used, rate)) = cache
                .get(&key)
                .and_then(|fx| fx.rate_at(date_naive, FX_LOOKBACK_DAYS))
            {
                return Ok(RateAt { rate, date_used });
            }
        }

        let symbol = fx_symbol(from_curr, to_curr);
        let window_start = date_naive - chrono::Duration::days(FX_SERIES_WINDOW_DAYS);

        // Database first: a seeded or previously fetched window answers with no
        // outbound call at all.
        if let Some(hit) = self.rate_from_db(&symbol, window_start, date_naive, pool).await? {
            return Ok(hit);
        }

        // get_historical_prices already builds the range URL, parses the chart
        // response, persists the series through the upsert, and merges what it
        // fetched over stored rows it found too stale to answer with. Reaching this
        // line means `rate_from_db` found nothing it was allowed to answer with — no
        // row inside `FX_LOOKBACK_DAYS`, or a newest row past
        // `PRICE_WINDOW_FRESH_DAYS` — so any stored row in the window is merged under
        // whatever the provider returns. Reusing it is why this change is small.
        let series = self
            .get_historical_prices(&symbol, window_start, date_naive, pool)
            .await?;

        // An empty result is not a window. Caching one would spend a slot on a pair
        // that produced nothing, and then answer nothing from it.
        if !series.is_empty() {
            let mut cache = self.cache.write().await;
            if cache.len() >= MAX_FX_SERIES_CACHED && !cache.contains_key(&key) {
                cache.clear();
            }
            cache.insert(
                key,
                FxSeries {
                    window_start: series.first().map(|(d, _)| *d).unwrap_or(window_start),
                    window_end: date_naive,
                    series: series.clone(),
                },
            );
        }

        select_rate(&series, date_naive, FX_LOOKBACK_DAYS)
            .map(|(date_used, rate)| RateAt { rate, date_used })
            .ok_or_else(|| no_rate_error(&symbol, date_naive, FX_LOOKBACK_DAYS))
    }

    /// The pair's dated closes as already stored, narrowed to the same rules
    /// `select_rate` and `stored_window_is_fresh` apply. `None` means "the database
    /// cannot answer", which is not the same as "no rate exists" — only the provider
    /// can say that.
    async fn rate_from_db(
        &self,
        symbol: &str,
        window_start: NaiveDate,
        date: NaiveDate,
        pool: &SqlitePool,
    ) -> Result<Option<RateAt>> {
        let rows: Vec<(NaiveDate, String)> = sqlx::query_as(
            "SELECT date, close_price FROM historical_prices
                 WHERE symbol = ? AND date >= ? AND date <= ? ORDER BY date",
        )
        .bind(symbol)
        .bind(window_start)
        .bind(date)
        .fetch_all(pool)
        .await?;

        let series: Vec<(NaiveDate, Decimal)> = rows
            .into_iter()
            .map(|(d, p)| (d, crate::db_types::str_to_decimal(&p)))
            .collect();

        // The same coverage rule `get_historical_prices` applies to its stored
        // window, and for the same reason: rows that stop short of `date` are an
        // incomplete window, not an answer. Without this gate the two DB-first read
        // paths disagree about the same database — `get_historical_prices` asks the
        // provider for the tail while this one serves a close from inside the
        // lookback — and `replay` walks transactions ascending, so the first FX call
        // pins the earliest transaction's rate over every later date within the
        // lookback that has no stored close of its own.
        let newest = series.iter().map(|(d, _)| *d).max();
        if !stored_window_is_fresh(newest, date) {
            return Ok(None);
        }

        Ok(
            select_rate(&series, date, FX_LOOKBACK_DAYS)
                .map(|(date_used, rate)| RateAt { rate, date_used }),
        )
    }

    /// The close for `symbol` as of today, or why there is none.
    ///
    /// Read order, stated rather than inherited: **the database answers when it holds
    /// today's row**; the provider is asked only on a miss.
    ///
    /// `Err(ProviderUnavailable)` means we could not ask. `Err(NoData)` means we asked
    /// and there is no close for this symbol.
    pub async fn get_price(
        &self,
        symbol: &str,
        isin: Option<&str>,
        pool: &SqlitePool,
    ) -> Result<Decimal, PricingError> {
        let today = chrono::Utc::now().date_naive();
        let existing = sqlx::query_as::<_, HistoricalPrice>(
            "SELECT * FROM historical_prices WHERE symbol = ? AND date = ?"
        )
        .bind(symbol)
        .bind(today)
        .fetch_optional(pool)
        .await
        .map_err(|e| PricingError::ProviderUnavailable(e.to_string()))?;

        if let Some(hp) = existing {
            return Ok(crate::db_types::str_to_decimal(&hp.close_price));
        }

        // A provider outage is remembered rather than swallowed: the EODHD fallback
        // below still gets its turn, exactly as it did when every failure was zero,
        // but if neither source answers the caller is told we could not ask rather
        // than that no such price exists.
        let mut outage = None;
        match self.fetch_yahoo_price(symbol).await {
            Ok(price) => {
                self.cache_price(symbol, today, price, pool).await;
                return Ok(price);
            }
            Err(e @ PricingError::ProviderUnavailable(_)) => outage = Some(e),
            Err(PricingError::NoData) => {}
        }

        if let Some(isin) = isin {
            if !isin.is_empty() {
                if let Some(eodhd_key) = &self.eodhd_api_key {
                    let price = self.fetch_eodhd_price(isin, eodhd_key).await;
                    if price > Decimal::ZERO {
                        self.cache_price(symbol, today, price, pool).await;
                        return Ok(price);
                    }
                }
            }
        }

        Err(outage.unwrap_or(PricingError::NoData))
    }

    /// Yahoo's latest close for `symbol`. A request failure and an unknown symbol are
    /// reported apart from each other, because `get_price` decides what to tell the
    /// user from that difference.
    async fn fetch_yahoo_price(&self, symbol: &str) -> Result<Decimal, PricingError> {
        let url = format!("{}/v8/finance/chart/{}", self.base_url, symbol);
        let response = match self.fetch_with_retry(&url).await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("WARN: Failed to fetch Yahoo price for {}: {}", symbol, e);
                return Err(PricingError::ProviderUnavailable(e.to_string()));
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
            return Err(PricingError::ProviderUnavailable(format!("status {status}")));
        }

        let body_bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("WARN: Failed to read Yahoo response for {}: {}", symbol, e);
                return Err(PricingError::ProviderUnavailable(e.to_string()));
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
                return Err(PricingError::ProviderUnavailable(e.to_string()));
            }
        };

        let result = yahoo_resp.chart.result.into_iter().next();
        let close_prices = match result {
            Some(r) => match r.indicators.quote.first() {
                Some(q) => q.close.clone(),
                None => {
                    eprintln!("WARN: No quote data for symbol: {}", symbol);
                    return Err(PricingError::NoData);
                }
            },
            None => {
                eprintln!("WARN: No price data for symbol: {}", symbol);
                return Err(PricingError::NoData);
            }
        };

        // A close of zero is not a price, it is a hole in the series: the same answer
        // the old `price > Decimal::ZERO` test in `get_price` gave, now named.
        close_prices
            .iter()
            .rev()
            .find_map(|x| x.map(f64_to_decimal))
            .filter(|price| *price > Decimal::ZERO)
            .ok_or(PricingError::NoData)
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

    /// Daily closes for `symbol` in `[start_date, end_date]`.
    ///
    /// Read order, stated rather than inherited: **the database answers when its
    /// newest row in the window is within `PRICE_WINDOW_FRESH_DAYS` of
    /// `end_date`**; otherwise the provider is asked for the tail and what it returns
    /// is merged over the stored rows. "Any row in the window" is not enough, because
    /// a window whose stored rows stop short of `end_date` is incomplete, not an
    /// answer: [`calculate_portfolio_performance`](crate::engines::stats_engine::StatsEngine::calculate_portfolio_performance)
    /// asks for a window that ends today and prices a date with no close as zero, so
    /// answering from a series that stopped days ago reports today's value as zero —
    /// the headline figure freezes the day after any fetch. Four days is a rule, not a
    /// parameter: it has to cover a market holiday weekend and nothing longer, because
    /// a longer gap means the series really was not updated.
    ///
    /// The previous order was provider-first with a database fallback on every
    /// failure, which is the only reason the hermetic pricing tests worked. With the
    /// fallback gone, a seeded window has to answer on its own — which it does, as
    /// long as it runs up to `end_date`.
    ///
    /// `Err(ProviderUnavailable)` means we could not ask. `Err(NoData)` means we asked
    /// and there is nothing. Neither is papered over with the stale rows: an
    /// incomplete window the provider could not top up stays an error rather than
    /// being promoted to a success.
    ///
    /// Either way the returned series is in ascending date order, which `get_rate`
    /// depends on when it records the window a fetch covered.
    pub async fn get_historical_prices(
        &self,
        symbol: &str,
        start_date: NaiveDate,
        end_date: NaiveDate,
        pool: &SqlitePool,
    ) -> Result<Vec<(NaiveDate, Decimal)>, PricingError> {
        let mut db_prices = self
            .get_historical_prices_from_db(symbol, start_date, end_date, pool)
            .await
            .map_err(|e| PricingError::ProviderUnavailable(e.to_string()))?;

        // The freshness rule. It lives in `stored_window_is_fresh` so the predicate
        // under test is the predicate that runs; no stored row at all is the most
        // incomplete window there is, so it falls through too.
        let newest = db_prices.iter().map(|(date, _)| *date).max();
        if stored_window_is_fresh(newest, end_date) {
            db_prices.sort_by_key(|(date, _)| *date);
            return Ok(db_prices);
        }

        let start_ts = start_date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let end_ts = end_date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

        let url = format!(
            "{}/v8/finance/chart/{}?period1={}&period2={}&interval=1d&includePrePost=false",
            self.base_url, symbol, start_ts, end_ts
        );

        let response = match self.fetch_with_retry(&url).await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("WARN: Failed to fetch historical prices for {}: {}", symbol, e);
                return Err(PricingError::ProviderUnavailable(e.to_string()));
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
            return Err(PricingError::ProviderUnavailable(format!("status {status}")));
        }

        let yahoo_resp: YahooChartResponse = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("WARN: Failed to parse historical Yahoo response for {}: {}", symbol, e);
                return Err(PricingError::ProviderUnavailable(e.to_string()));
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
                return Err(PricingError::NoData);
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
            let mut tx = pool
                .begin()
                .await
                .map_err(|e| PricingError::ProviderUnavailable(e.to_string()))?;
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
            return Err(PricingError::NoData);
        }

        // The provider was asked because the stored window stopped short of
        // `end_date`, so what came back is the tail, not the whole series. Merge it
        // over the rows read above — the fetched rows win on a shared date, since the
        // provider just wrote them through the upsert — so a partial stored window is
        // kept rather than discarded. Re-reading the window here would return these
        // same rows plus the ones just persisted, which are already in
        // `daily_prices`.
        Ok(merge_series(daily_prices, db_prices))
    }

    /// The rows already stored for `symbol` in `[start_date, end_date]`, with no
    /// freshness rule and no provider. `pub(crate)` so
    /// [`calculate_portfolio_performance`](crate::engines::stats_engine::StatsEngine::calculate_portfolio_performance)
    /// can serve them when the provider is down.
    pub(crate) async fn get_historical_prices_from_db(
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
            "{}/v8/finance/chart/{}?period1=0&period2={}&interval=1d&events=splits",
            self.base_url, asset.symbol, end_ts
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

    /// Same-currency requests short-circuit in `get_rate` before any query, so an
    /// empty in-memory pool is the whole fixture — there are no seeded prices here.
    async fn empty_pool() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE historical_prices (symbol TEXT NOT NULL, date DATE NOT NULL, close_price TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_same_currency_returns_one() {
        let svc = crate::offline_pricing_service();
        let date = DateTime::from_timestamp(1705312200, 0).unwrap();
        let pool = empty_pool().await;
        let result = svc.get_rate("USD", "USD", date, &pool).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().rate, Decimal::ONE);
    }

    #[tokio::test]
    async fn test_same_currency_returns_one_consistently() {
        let svc = crate::offline_pricing_service();
        let date1 = DateTime::from_timestamp(1700000001, 0).unwrap();
        let date2 = DateTime::from_timestamp(1700000001, 0).unwrap();
        let pool = empty_pool().await;

        let r1 = svc.get_rate("GBP", "GBP", date1, &pool).await.unwrap().rate;
        let r2 = svc.get_rate("GBP", "GBP", date2, &pool).await.unwrap().rate;
        assert_eq!(r1, r2);
    }

    fn date_of(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn ts_utc(s: &str) -> DateTime<Utc> {
        date_of(s).and_hms_opt(0, 0, 0).unwrap().and_utc()
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    async fn seed_prices(pool: &SqlitePool, symbol: &str, rows: &[(&str, &str)]) {
        for (d, price) in rows {
            sqlx::query("INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
                .bind(symbol)
                .bind(date_of(d))
                .bind(*price)
                .execute(pool)
                .await
                .unwrap();
        }
    }

    #[test]
    fn fx_series_answers_only_dates_its_window_covered() {
        // The shape a request for Saturday 2024-03-09 leaves behind: the window ends
        // on the requested date, the last close inside it is Friday's.
        let window = FxSeries {
            window_start: date_of("2023-03-09"),
            window_end: date_of("2024-03-09"),
            series: vec![(date_of("2024-03-07"), dec("0.9100")), (date_of("2024-03-08"), dec("0.9125"))],
        };

        // Inside the window: the last close on or before the date, with its own date.
        assert_eq!(window.rate_at(date_of("2024-03-08"), 30), Some((date_of("2024-03-08"), dec("0.9125"))));
        assert_eq!(window.rate_at(date_of("2024-03-09"), 30), Some((date_of("2024-03-08"), dec("0.9125"))));

        // Past the end: this window never fetched 2024-03-25, so its 2024-03-08 close
        // — 17 days back, comfortably inside the lookback — must not price it.
        assert_eq!(window.rate_at(date_of("2024-03-25"), 30), None);

        // Before the start: the same rule on the other side, and it has to hold even
        // though the series here holds a close the lookback would happily accept.
        let late_window = FxSeries {
            window_start: date_of("2024-03-01"),
            window_end: date_of("2024-03-08"),
            series: vec![(date_of("2024-02-20"), dec("0.8700"))],
        };
        assert_eq!(late_window.rate_at(date_of("2024-02-26"), 30), None);
    }

    #[tokio::test]
    async fn a_fresh_stored_window_answers_without_the_provider() {
        // The hermetic half of the read order, at the level it is decided. The symbol
        // cannot resolve, so the provider could not have produced this answer: the
        // newest row sits exactly on the freshness bound, which means the database
        // returned and no outbound call was made. The stale side of the rule is a
        // pure predicate now (`stored_window_is_fresh_up_to_the_bound_and_stale_past_it`)
        // rather than a test that has to reach the network to fail.
        let pool = empty_pool().await;
        let end = date_of("2024-06-14");
        let window_start = end - chrono::Duration::days(30);
        let svc = crate::offline_pricing_service();

        let fresh_newest = end - chrono::Duration::days(PRICE_WINDOW_FRESH_DAYS);
        let fresh_newest_str = fresh_newest.format("%Y-%m-%d").to_string();
        seed_prices(&pool, "FRESHWINDOW", &[(&fresh_newest_str, "1.0500")]).await;
        let fresh = svc
            .get_historical_prices("FRESHWINDOW", window_start, end, &pool)
            .await
            .expect("a window whose newest row is PRICE_WINDOW_FRESH_DAYS old answers from the database");
        assert_eq!(fresh, vec![(fresh_newest, dec("1.0500"))]);
    }

    #[tokio::test]
    async fn get_rate_refuses_a_cached_window_that_stops_short_of_the_date() {
        // The reachable version of the bug above: `replay` walks a foreign-currency
        // asset's transactions in ascending date order, so a later transaction
        // routinely asks a later date than the one that populated the cache, and
        // `CurrencyService` is Rocket managed state — the cache is shared across
        // requests for the whole process.
        let pool = empty_pool().await;
        seed_prices(&pool, "EURUSD=X", &[("2024-03-08", "0.9125"), ("2024-03-22", "1.0500")]).await;

        let svc = crate::offline_pricing_service();
        // Exactly what a request for 2024-03-08 leaves behind.
        svc.cache.write().await.insert(
            ("EUR".to_string(), "USD".to_string()),
            FxSeries {
                window_start: date_of("2023-03-09"),
                window_end: date_of("2024-03-08"),
                series: vec![(date_of("2024-03-08"), dec("0.9125"))],
            },
        );

        // An unbounded cache would answer 0.9125 here. The database has the real
        // 2024-03-22 close, and refusing the stale window is what lets it answer.
        let got = svc.get_rate("EUR", "USD", ts_utc("2024-03-25"), &pool).await.unwrap();
        assert_eq!(got.rate, dec("1.0500"), "a cached window must not price a date past its end");
        assert_eq!(got.date_used, date_of("2024-03-22"));
    }

    /// C1: `rate_from_db` is the other DB-first read path `get_rate` takes, and it
    /// must apply the same coverage rule `get_historical_prices` applies. The stale
    /// row here sits 10 days before the requested date — inside `FX_LOOKBACK_DAYS`,
    /// so `select_rate` alone serves it and the provider is never asked.
    #[tokio::test]
    async fn rate_from_db_refuses_a_stored_window_that_stops_short_of_the_date() {
        let svc = crate::offline_pricing_service();
        let requested = date_of("2024-03-25");
        let window_start = requested - chrono::Duration::days(FX_SERIES_WINDOW_DAYS);

        // Incomplete coverage: the newest stored close is 10 days short of the date.
        let stale_pool = empty_pool().await;
        let stale = requested - chrono::Duration::days(10);
        let stale_str = stale.format("%Y-%m-%d").to_string();
        seed_prices(&stale_pool, "EURUSD=X", &[(&stale_str, "0.9125")]).await;
        let incomplete = svc
            .rate_from_db("EURUSD=X", window_start, requested, &stale_pool)
            .await
            .unwrap();
        assert!(
            incomplete.is_none(),
            "a window whose newest close is 10 days short of the requested date is \
             incomplete coverage, not an answer; got {incomplete:?}"
        );

        // Honest coverage, same distance from the lookback bound: a newest close
        // within `PRICE_WINDOW_FRESH_DAYS` still answers, with its own date.
        let fresh_pool = empty_pool().await;
        let fresh = requested - chrono::Duration::days(PRICE_WINDOW_FRESH_DAYS);
        let fresh_str = fresh.format("%Y-%m-%d").to_string();
        seed_prices(&fresh_pool, "EURUSD=X", &[(&fresh_str, "1.0500")]).await;
        let complete = svc
            .rate_from_db("EURUSD=X", window_start, requested, &fresh_pool)
            .await
            .unwrap();
        assert_eq!(
            complete,
            Some(RateAt { rate: dec("1.0500"), date_used: fresh }),
            "a window that runs up to the freshness bound must still answer from the database"
        );
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

        let svc = crate::offline_pricing_service();
        let price = svc.get_price("TEST", None, &pool).await.unwrap();
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

        let svc = crate::offline_pricing_service();
        // The stale row must not answer for today. The mock provider has no price
        // for `TEST`; what this pins is that the 2020 close is not the answer. It
        // used to be the live provider that had no price for `TEST`, which made a
        // network call the load-bearing part of a cache test.
        let price = svc.get_price("TEST", None, &pool).await;
        assert_ne!(price.ok(), Some(Decimal::from_str("99.0").unwrap()));
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

        let svc = crate::offline_pricing_service();
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

    fn series(pairs: &[(&str, &str)]) -> Vec<(NaiveDate, Decimal)> {
        pairs
            .iter()
            .map(|(d, p)| {
                (
                    NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap(),
                    Decimal::from_str(p).unwrap(),
                )
            })
            .collect()
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn select_rate_uses_the_exact_date_when_present() {
        let s = series(&[("2024-03-01", "0.9000"), ("2024-03-04", "0.9050")]);
        let (used, rate) = select_rate(&s, day("2024-03-04"), FX_LOOKBACK_DAYS).unwrap();
        assert_eq!(used, day("2024-03-04"));
        assert_eq!(rate, Decimal::from_str("0.9050").unwrap());
    }

    #[test]
    fn select_rate_falls_back_to_the_prior_trading_day() {
        // Review Focus 2: 2024-03-09 is a Saturday; the Friday close is the answer,
        // and the caller learns which date was used instead of assuming its own.
        let s = series(&[("2024-03-07", "0.9100"), ("2024-03-08", "0.9125")]);
        let (used, rate) = select_rate(&s, day("2024-03-09"), FX_LOOKBACK_DAYS).unwrap();
        assert_eq!(used, day("2024-03-08"));
        assert_eq!(rate, Decimal::from_str("0.9125").unwrap());
    }

    #[test]
    fn select_rate_rejects_a_date_before_every_close() {
        let s = series(&[("2024-03-07", "0.9100")]);
        assert_eq!(select_rate(&s, day("2024-03-01"), FX_LOOKBACK_DAYS), None);
    }

    #[test]
    fn select_rate_rejects_a_gap_wider_than_the_lookback() {
        // Review Focus 1: a delisted pair whose last close is three years old must
        // produce no rate at all rather than a stale one.
        let s = series(&[("2021-01-04", "0.8800")]);
        assert_eq!(select_rate(&s, day("2024-03-09"), FX_LOOKBACK_DAYS), None);
    }

    #[test]
    fn select_rate_accepts_a_gap_exactly_at_the_lookback_edge() {
        let date = day("2024-03-09");
        let edge = date - chrono::Duration::days(FX_LOOKBACK_DAYS);
        let s = series(&[("2024-02-08", "0.8900")]);
        assert_eq!(edge, day("2024-02-08"), "fixture must sit on the boundary");
        assert_eq!(select_rate(&s, date, FX_LOOKBACK_DAYS).map(|(d, _)| d), Some(edge));
    }

    #[test]
    fn select_rate_handles_an_unsorted_series_and_an_empty_one() {
        let s = series(&[("2024-03-08", "0.9125"), ("2024-03-06", "0.9075"), ("2024-03-07", "0.9100")]);
        assert_eq!(select_rate(&s, day("2024-03-09"), FX_LOOKBACK_DAYS).map(|(d, _)| d), Some(day("2024-03-08")));
        assert_eq!(select_rate(&[], day("2024-03-09"), FX_LOOKBACK_DAYS), None);
    }

    #[test]
    fn stored_window_is_fresh_up_to_the_bound_and_stale_past_it() {
        // The rule that decides whether a stored window answers, tested on its own so
        // that pinning it costs no pool, no provider and no outbound call.
        let end = day("2024-06-14");
        assert!(
            stored_window_is_fresh(Some(end - chrono::Duration::days(PRICE_WINDOW_FRESH_DAYS)), end),
            "a newest row exactly on the bound still answers"
        );
        assert!(
            !stored_window_is_fresh(Some(end - chrono::Duration::days(PRICE_WINDOW_FRESH_DAYS + 1)), end),
            "one day past the bound the window is incomplete and the provider is asked"
        );
        assert!(stored_window_is_fresh(Some(end), end), "a window that ends on the date is fresh");
        assert!(
            !stored_window_is_fresh(None, end),
            "no stored row at all is the most incomplete window there is"
        );
        // A newest row *after* `end_date` is not reachable at the only call site:
        // `get_historical_prices_from_db` reads `date >= start_date AND date <=
        // end_date`, so the newest row it can hand back never passes `end_date`. Said
        // rather than tested, because inventing the case would test a shape that
        // cannot occur.
    }

    #[test]
    fn merge_series_keeps_both_sides_of_the_window_and_sorts_them() {
        // The provider answers the tail; the stored rows are the head. Both survive,
        // the fetched row wins where they overlap (the upsert just rewrote it), and
        // the result is ascending even though both inputs came in unordered — the
        // ordering `get_rate` depends on when it records a window.
        let merged = merge_series(
            series(&[("2024-06-13", "1.0700"), ("2024-06-10", "1.0600")]),
            series(&[("2024-06-10", "0.9000"), ("2024-06-07", "0.9100")]),
        );
        assert_eq!(
            merged,
            series(&[
                ("2024-06-07", "0.9100"),
                ("2024-06-10", "1.0600"),
                ("2024-06-13", "1.0700"),
            ])
        );

        // The first-fetch case: nothing stored, so the merge is the fetched rows.
        assert_eq!(
            merge_series(series(&[("2024-06-10", "1.0600")]), vec![]),
            series(&[("2024-06-10", "1.0600")])
        );
    }

    #[test]
    fn fx_symbol_uses_yahoo_pair_symbology() {
        // Review Focus 5: the live endpoint returns "No data found" for GBP=JPY and
        // data for GBPJPY=X. The old form is why no foreign-currency asset priced.
        assert_eq!(fx_symbol("EUR", "USD"), "EURUSD=X");
        assert_eq!(fx_symbol("GBP", "JPY"), "GBPJPY=X");
    }
}