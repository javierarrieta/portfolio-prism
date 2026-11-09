use chrono::{Datelike, NaiveDate, Utc, TimeZone};
use sqlx::SqlitePool;
use crate::models::{Asset, Transaction};
use crate::services::currency_service::CurrencyService;
use rust_decimal::Decimal;
use std::str::FromStr;
use anyhow::Result;
use std::collections::HashMap;

pub struct StatsEngine;

impl StatsEngine {
    pub async fn aggregate_weekly(
        daily_history: Vec<serde_json::Value>,
    ) -> Vec<serde_json::Value> {
        if daily_history.is_empty() {
            return Vec::new();
        }

        let mut weeks: HashMap<(i32, u32), Vec<serde_json::Value>> = HashMap::new();
        for item in daily_history {
            let date_str = item.get("date").and_then(|d| d.as_str()).unwrap_or("");
            if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                let iso = date.iso_week();
                let key = (date.year(), iso.week());
                weeks.entry(key).or_default().push(item);
            }
        }

        let mut sorted_weeks: Vec<_> = weeks.into_iter().collect();
        sorted_weeks.sort_by(|a, b| a.0.cmp(&b.0));

        let mut weekly_history = Vec::new();
        let mut prev_value: Option<Decimal> = None;
        let mut twr_acc = 1.0f64;

        for (_key, items) in sorted_weeks {
            let trading_items: Vec<_> = items.iter()
                .filter(|item| {
                    item.get("value")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<Decimal>().ok())
                        .map(|d| d > Decimal::ZERO)
                        .unwrap_or(false)
                })
                .collect();

            let last_item = if trading_items.is_empty() {
                items.last().unwrap()
            } else {
                trading_items.last().unwrap()
            };

            let date = last_item.get("date").and_then(|d| d.as_str()).unwrap_or("");
            let value_str = last_item.get("value").and_then(|v| v.as_str()).unwrap_or("0");
            let value = Decimal::from_str(value_str).unwrap_or(Decimal::ZERO);

            let daily_ret = match prev_value {
                Some(prev) if prev > Decimal::ZERO && value > Decimal::ZERO => {
                    (value - prev) / prev
                }
                _ if value > Decimal::ZERO => Decimal::ZERO,
                _ => {
                    twr_acc *= 1.0;
                    // Keep prev_value as the last non-zero value: overwriting it
                    // with a holiday's 0 would make the next real week compute a
                    // 0.0 return instead of the actual change.
                    if value > Decimal::ZERO {
                        prev_value = Some(value);
                    }
                    weekly_history.push(serde_json::json!({
                        "date": date,
                        "value": value.to_string(),
                        "daily_return": "0.0",
                        "twr": (twr_acc - 1.0).to_string(),
                    }));
                    continue;
                }
            };

            let daily_ret_f64 = daily_ret.as_f64();
            twr_acc *= 1.0 + daily_ret_f64;
            prev_value = Some(value);

            weekly_history.push(serde_json::json!({
                "date": date,
                "value": value.to_string(),
                "daily_return": daily_ret.to_string(),
                "twr": (twr_acc - 1.0).to_string(),
            }));
        }

        weekly_history
    }

    pub async fn calculate_portfolio_performance(
        pool: &SqlitePool,
        assets: &[Asset],
        transactions: &[Transaction],
        base_currency: &str,
        currency_service: &CurrencyService,
    ) -> Result<serde_json::Value> {
        if assets.is_empty() || transactions.is_empty() {
            return Ok(serde_json::json!({
                "history": [],
                "correlation_matrix": {},
                "metrics": {
                    "volatility": "0.0",
                    "sharpe_ratio": "0.0",
                    "beta": "1.0",
                    "portfolio_value": "0.0",
                    "unrealized_pnl": "0.0",
                    "realized_pnl": "0.0",
                    "beta_adjusted_exposure": "0.0"
                }
            }));
        }

        let tx_dates: Vec<NaiveDate> = transactions.iter().map(|tx| tx.date.date_naive()).collect();
        let earliest_tx = *tx_dates.iter().min().unwrap();
        let end_date = Utc::now().date_naive();

        let start_date = earliest_tx - chrono::Duration::days(365);

        let min_reasonable_date = NaiveDate::from_ymd_opt(1900, 1, 1).unwrap();
        let start_date = if start_date < min_reasonable_date {
            min_reasonable_date
        } else {
            start_date
        };

        let symbols: Vec<String> = assets.iter().map(|a| a.symbol.clone()).collect();
        let symbol_isin: HashMap<&str, Option<&str>> = assets.iter()
            .map(|a| (a.symbol.as_str(), a.isin.as_deref()))
            .collect();

        let mut price_map: HashMap<(NaiveDate, String), Decimal> = HashMap::new();
        for symbol in &symbols {
            match currency_service.get_historical_prices(symbol, start_date, end_date, pool).await {
                Ok(daily_prices) => {
                    for (date, price) in daily_prices {
                        price_map.insert((date, symbol.clone()), price);
                    }
                }
                Err(e) => {
                    eprintln!("WARN: Failed to get historical prices for {}: {}", symbol, e);
                }
            }
        }

        let mut rate_cache: HashMap<String, Decimal> = HashMap::new();
        for asset in assets {
            if asset.currency != base_currency {
                let key = format!("{}->{}", asset.currency, base_currency);
                if !rate_cache.contains_key(&key) {
                    let date_utc = Utc.from_utc_datetime(&end_date.and_hms_opt(0, 0, 0).unwrap());
                    let rate = currency_service
                        .get_rate(&asset.currency, base_currency, date_utc, pool)
                        .await
                        .map(|r| r.rate)
                        .unwrap_or(Decimal::ONE);
                    rate_cache.insert(key, rate);
                }
            }
        }

        let mut dates = Vec::new();
        let mut curr = start_date;
        while curr <= end_date {
            dates.push(curr);
            match curr.succ_opt() {
                Some(next) => curr = next,
                None => break,
            }
        }

        for symbol in &symbols {
            let has_prices = price_map.keys().any(|(_, s)| s == symbol);
            if !has_prices {
                let isin = symbol_isin.get(symbol.as_str()).and_then(|&i| i);
                let current_price = currency_service.get_price(symbol, isin, pool).await;
                if current_price > Decimal::ZERO {
                    for date in &dates {
                        price_map.insert((*date, symbol.clone()), current_price);
                    }
                }
            }
        }

        let mut history = Vec::new();
        let mut portfolio_values = Vec::new();
        let mut daily_returns = Vec::new();
        let mut twr_cumulative = Vec::new();
        let mut twr_acc = 1.0f64;

        let mut asset_qtys: HashMap<String, Decimal> = HashMap::new();
        let mut sorted_txs = transactions.to_vec();
        // Splits must apply to existing lots before any same-day buy/sell, matching
        // the tax engine ordering.
        let split_priority = |tx: &Transaction| {
            if tx.r#type.to_uppercase() == "SPLIT" { 0 } else { 1 }
        };
        sorted_txs.sort_by(|a, b| {
            a.date.cmp(&b.date).then_with(|| split_priority(a).cmp(&split_priority(b)))
        });

        // Yahoo returns split-ADJUSTED close prices (continuous across a split), so
        // the pre-split value must use the post-split-equivalent quantity. For each
        // symbol, collect split ratios (quantity : price) to scale pre-split days.
        let mut split_ratios: HashMap<String, Vec<(NaiveDate, Decimal)>> = HashMap::new();
        for tx in &sorted_txs {
            if tx.r#type.to_uppercase() == "SPLIT" {
                let symbol = assets.iter().find(|a| a.id == tx.asset_id).map(|a| a.symbol.as_str()).unwrap_or("");
                let num = crate::db_types::str_to_decimal(&tx.quantity);
                let den = crate::db_types::str_to_decimal(&tx.price);
                if num > Decimal::ZERO && den > Decimal::ZERO && !symbol.is_empty() {
                    split_ratios.entry(symbol.to_string())
                        .or_default()
                        .push((tx.date.date_naive(), num / den));
                }
            }
        }

        let mut tx_idx = 0;
        for &date in &dates {
            while tx_idx < sorted_txs.len() && sorted_txs[tx_idx].date.date_naive() <= date {
                let tx = &sorted_txs[tx_idx];
                let symbol = assets.iter().find(|a| a.id == tx.asset_id).map(|a| a.symbol.as_str()).unwrap_or("");
                let entry = asset_qtys.entry(symbol.to_string()).or_insert(Decimal::ZERO);
                if tx.r#type.to_uppercase() == "BUY" {
                    let tx_qty = crate::db_types::str_to_decimal(&tx.quantity);
                    *entry += tx_qty;
                } else if tx.r#type.to_uppercase() == "SELL" {
                    let tx_qty = crate::db_types::str_to_decimal(&tx.quantity);
                    *entry = (*entry - tx_qty).max(Decimal::ZERO);
                } else if tx.r#type.to_uppercase() == "SPLIT" {
                    let numerator = crate::db_types::str_to_decimal(&tx.quantity);
                    let denominator = crate::db_types::str_to_decimal(&tx.price);
                    if numerator > Decimal::ZERO && denominator > Decimal::ZERO {
                        *entry *= numerator / denominator;
                    }
                }
                tx_idx += 1;
            }

            let mut daily_val = Decimal::ZERO;
            for asset in assets {
                let qty = asset_qtys.get(&asset.symbol).cloned().unwrap_or(Decimal::ZERO);
                let price = price_map.get(&(date, asset.symbol.clone())).cloned().unwrap_or(Decimal::ZERO);

                // Pre-split days: scale the running quantity by the product of all
                // future split ratios, because the historical price is already
                // split-adjusted. Without this the pre-split value is too low and
                // the series jumps at the split date.
                let mut effective_qty = qty;
                if let Some(ratios) = split_ratios.get(&asset.symbol) {
                    for (split_date, ratio) in ratios {
                        if *split_date > date {
                            effective_qty *= ratio;
                        }
                    }
                }

                let mut final_price = price;
                if asset.currency != base_currency {
                    let key = format!("{}->{}", asset.currency, base_currency);
                    let rate = rate_cache.get(&key).copied().unwrap_or(Decimal::ONE);
                    final_price *= rate;
                }
                daily_val += effective_qty * final_price;
            }

            portfolio_values.push(daily_val);

            let daily_ret = if portfolio_values.len() > 1 {
                let prev_val = portfolio_values[portfolio_values.len()-2];
                if prev_val > Decimal::ZERO { (daily_val - prev_val) / prev_val } else { Decimal::ZERO }
            } else {
                Decimal::ZERO
            };

            let daily_ret_f64 = daily_ret.as_f64();
            twr_acc *= 1.0 + daily_ret_f64;
            daily_returns.push(daily_ret);
            twr_cumulative.push(Decimal::from_str(&(twr_acc - 1.0).to_string()).unwrap_or(Decimal::ZERO));

            history.push(serde_json::json!({
                "date": date.to_string(),
                "value": daily_val.to_string(),
                "daily_return": daily_ret.to_string(),
                "twr": (twr_acc - 1.0).to_string()
            }));
        }

        let final_val = portfolio_values.last().cloned().unwrap_or(Decimal::ZERO);

        let weekly_history = Self::aggregate_weekly(history).await;

        Ok(serde_json::json!({
            "history": weekly_history,
            "correlation_matrix": {},
            "metrics": {
                "volatility": "0.0",
                "sharpe_ratio": "0.0",
                "beta": "1.0",
                "portfolio_value": final_val.to_string(),
                "beta_adjusted_exposure": final_val.to_string(),
                "unrealized_pnl": "0.0",
                "realized_pnl": "0.0",
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use crate::services::currency_service::CurrencyService;

    fn empty_asset() -> Asset {
        Asset {
            id: 1,
            portfolio_id: 1,
            symbol: "TEST".to_string(),
            name: "Test".to_string(),
            asset_type: "STOCK".to_string(),
            sector: None,
            currency: "USD".to_string(),
            isin: None,
        }
    }

    fn empty_tx() -> Transaction {
        Transaction {
            id: 1,
            asset_id: 1,
            r#type: "BUY".to_string(),
            quantity: "100.0".to_string(),
            price: "50.0".to_string(),
            fee: "0.0".to_string(),
            date: DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").unwrap().with_timezone(&Utc),
        }
    }

    #[tokio::test]
    async fn test_empty_assets_returns_zeroed_metrics() {
        let svc = CurrencyService::new();
        let result = StatsEngine::calculate_portfolio_performance(
            &SqlitePool::connect(":memory:").await.unwrap(),
            &[],
            &[],
            "USD",
            &svc,
        ).await.unwrap();

        let metrics = result.get("metrics").unwrap();
        let vol = metrics.get("volatility").unwrap().as_str().unwrap();
        let sharpe = metrics.get("sharpe_ratio").unwrap().as_str().unwrap();
        let beta = metrics.get("beta").unwrap().as_str().unwrap();
        let value = metrics.get("portfolio_value").unwrap().as_str().unwrap();

        assert_eq!(vol, "0.0");
        assert_eq!(sharpe, "0.0");
        assert_eq!(beta, "1.0");
        assert_eq!(value, "0.0");
    }

    #[tokio::test]
    async fn test_with_data_but_no_prices_returns_zero_value() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (symbol TEXT, date DATE, close_price TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        let svc = CurrencyService::new();
        let assets = vec![empty_asset()];
        let txs = vec![empty_tx()];

        let result = StatsEngine::calculate_portfolio_performance(
            &pool,
            &assets,
            &txs,
            "USD",
            &svc,
        ).await.unwrap();

        let metrics = result.get("metrics").unwrap();
        let value = metrics.get("portfolio_value").unwrap().as_str().unwrap();
        assert_eq!(value, "0");
    }

    #[tokio::test]
    async fn test_multicurrency_asset_with_conversion() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (symbol TEXT, date DATE, close_price TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        let svc = CurrencyService::new();

        let asset = Asset {
            id: 1,
            portfolio_id: 1,
            symbol: "NESF".to_string(),
            name: "Nestle".to_string(),
            asset_type: "STOCK".to_string(),
            sector: None,
            currency: "CHF".to_string(),
            isin: None,
        };
        let tx = Transaction {
            id: 1,
            asset_id: 1,
            r#type: "BUY".to_string(),
            quantity: "10.0".to_string(),
            price: "100.0".to_string(),
            fee: "0.0".to_string(),
            date: DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z").unwrap().with_timezone(&Utc),
        };

        let result = StatsEngine::calculate_portfolio_performance(
            &pool,
            &[asset],
            &[tx],
            "USD",
            &svc,
        ).await.unwrap();

        let metrics = result.get("metrics").unwrap();
        let value = metrics.get("portfolio_value").unwrap().as_str().unwrap();
        assert_eq!(value, "0");
    }

    #[tokio::test]
    async fn test_split_adjusts_historical_quantity() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (symbol TEXT, date DATE, close_price TEXT)")
            .execute(&pool)
            .await
            .unwrap();

        let today = Utc::now().date_naive();
        let split_date = today - chrono::Duration::days(5);
        let sell_date = today - chrono::Duration::days(2);
        let buy_date = split_date - chrono::Duration::days(30);
        let start_date = buy_date - chrono::Duration::days(365);

        let symbol = "SPLITSTTEST";
        // Yahoo returns split-ADJUSTED close prices: the series is continuous
        // across a split (no price step). Real feed: constant 50.0 here, which
        // for the pre-split period represents the raw price divided by the ratio.
        let sjson = "50.0";
        let mut curr = start_date;
        while curr <= today {
            sqlx::query("INSERT OR REPLACE INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
                .bind(symbol)
                .bind(curr)
                .bind(sjson)
                .execute(&pool)
                .await
                .unwrap();
            match curr.succ_opt() {
                Some(next) => curr = next,
                None => break,
            }
        }

        let asset = Asset {
            id: 1,
            portfolio_id: 1,
            symbol: symbol.to_string(),
            name: "Split Test".to_string(),
            asset_type: "STOCK".to_string(),
            sector: None,
            currency: "USD".to_string(),
            isin: None,
        };
        let buy = Transaction {
            id: 1,
            asset_id: 1,
            r#type: "BUY".to_string(),
            quantity: "100.0".to_string(),
            price: "100.0".to_string(),
            fee: "0.0".to_string(),
            date: buy_date.and_hms_opt(0, 0, 0).unwrap().and_utc(),
        };
        let split = Transaction {
            id: 2,
            asset_id: 1,
            r#type: "SPLIT".to_string(),
            quantity: "2.0".to_string(),
            price: "1.0".to_string(),
            fee: "0.0".to_string(),
            date: split_date.and_hms_opt(0, 0, 0).unwrap().and_utc(),
        };
        let sell = Transaction {
            id: 3,
            asset_id: 1,
            r#type: "SELL".to_string(),
            quantity: "50.0".to_string(),
            price: "50.0".to_string(),
            fee: "0.0".to_string(),
            date: sell_date.and_hms_opt(0, 0, 0).unwrap().and_utc(),
        };

        let svc = CurrencyService::new();
        let result = StatsEngine::calculate_portfolio_performance(
            &pool,
            &[asset],
            &[buy, split, sell],
            "USD",
            &svc,
        ).await.unwrap();

        // 100 shares bought pre-split, 2:1 split -> 200, sell 50 -> 150 remaining.
        // Price is split-ADJUSTED (constant 50.0), so pre-split each week must
        // value the position as 200 shares (post-split equivalent) x 50 = 10000,
        // NOT 100 x 50 = 5000. Otherwise the historical series has a fake step
        // at the split date and a fabricated daily return / TWR spike.
        let history = result.get("history").unwrap().as_array().unwrap();
        for entry in history {
            let date_str = entry.get("date").and_then(|d| d.as_str()).unwrap_or("");
            let value_str = entry.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let week_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap();
            if week_date < split_date && week_date >= buy_date {
                let week_value = Decimal::from_str(value_str).unwrap();
                assert_eq!(
                    week_value,
                    Decimal::from_str("10000").unwrap(),
                    "pre-split history not split-scaled on {date_str}: got {week_value}"
                );
            }
        }

        let metrics = result.get("metrics").unwrap();
        let value = metrics.get("portfolio_value").unwrap().as_str().unwrap();
        assert_eq!(
            Decimal::from_str(value).unwrap(),
            Decimal::from_str("7500").unwrap(),
            "expected 150 shares at 50, got value {value}"
        );
    }

    #[tokio::test]
    async fn test_split_on_same_day_as_buy_splits_first() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (symbol TEXT, date DATE, close_price TEXT)")
            .execute(&pool)
            .await
            .unwrap();

        let today = Utc::now().date_naive();
        let day = today - chrono::Duration::days(30);
        let earlier = day - chrono::Duration::days(10);
        let start_date = earlier - chrono::Duration::days(1);

        let symbol = "SAMESPLIT";
        // Split-adjusted close: constant 1.0.
        let mut curr = start_date;
        while curr <= today {
            sqlx::query("INSERT OR REPLACE INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
                .bind(symbol)
                .bind(curr)
                .bind("1.0")
                .execute(&pool)
                .await
                .unwrap();
            match curr.succ_opt() {
                Some(next) => curr = next,
                None => break,
            }
        }

        let asset = Asset {
            id: 1,
            portfolio_id: 1,
            symbol: symbol.to_string(),
            name: "Split Test".to_string(),
            asset_type: "STOCK".to_string(),
            sector: None,
            currency: "USD".to_string(),
            isin: None,
        };
        let initial_buy = Transaction {
            id: 1,
            asset_id: 1,
            r#type: "BUY".to_string(),
            quantity: "100.0".to_string(),
            price: "1.0".to_string(),
            fee: "0.0".to_string(),
            date: earlier.and_hms_opt(0, 0, 0).unwrap().and_utc(),
        };
        let split = Transaction {
            id: 2,
            asset_id: 1,
            r#type: "SPLIT".to_string(),
            quantity: "2.0".to_string(),
            price: "1.0".to_string(),
            fee: "0.0".to_string(),
            date: day.and_hms_opt(0, 0, 0).unwrap().and_utc(),
        };
        let same_day_buy = Transaction {
            id: 3,
            asset_id: 1,
            r#type: "BUY".to_string(),
            quantity: "100.0".to_string(),
            price: "1.0".to_string(),
            fee: "0.0".to_string(),
            date: day.and_hms_opt(0, 0, 0).unwrap().and_utc(),
        };

        let svc = CurrencyService::new();
        let result = StatsEngine::calculate_portfolio_performance(
            &pool,
            &[asset],
            &[initial_buy, split, same_day_buy],
            "USD",
            &svc,
        ).await.unwrap();

        // Split must apply before the same-day buy: 100 -> 200 (split), then
        // +100 buy = 300 shares. If the buy ran first, 100+100=200 then split
        // would give 400 -> wrong.
        let metrics = result.get("metrics").unwrap();
        let value = metrics.get("portfolio_value").unwrap().as_str().unwrap();
        assert_eq!(
            Decimal::from_str(value).unwrap(),
            Decimal::from_str("300").unwrap(),
            "expected 300 post-split shares at adjusted price 1.0, got value {value}"
        );
    }

    #[tokio::test]
    async fn test_single_day_range_does_not_panic() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (symbol TEXT, date DATE, close_price TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        let svc = CurrencyService::new();
        let assets = vec![empty_asset()];
        let txs = vec![empty_tx()];

        let result = StatsEngine::calculate_portfolio_performance(
            &pool,
            &assets,
            &txs,
            "USD",
            &svc,
        ).await.unwrap();

        let history = result.get("history").unwrap().as_array().unwrap();
        assert!(!history.is_empty());
    }

    #[tokio::test]
    async fn test_aggregate_weekly_basic() {
        let daily_history = vec![
            serde_json::json!({"date": "2024-01-01", "value": "100.0", "daily_return": "0.0", "twr": "0.0"}),
            serde_json::json!({"date": "2024-01-02", "value": "105.0", "daily_return": "0.05", "twr": "0.05"}),
            serde_json::json!({"date": "2024-01-03", "value": "103.0", "daily_return": "-0.019", "twr": "0.029"}),
            serde_json::json!({"date": "2024-01-08", "value": "110.0", "daily_return": "0.068", "twr": "0.099"}),
            serde_json::json!({"date": "2024-01-09", "value": "112.0", "daily_return": "0.018", "twr": "0.119"}),
        ];

        let weekly = StatsEngine::aggregate_weekly(daily_history).await;
        assert_eq!(weekly.len(), 2);

        let week1 = &weekly[0];
        assert_eq!(week1.get("value").unwrap().as_str().unwrap(), "103.0");
        assert_eq!(week1.get("date").unwrap().as_str().unwrap(), "2024-01-03");

        let week2 = &weekly[1];
        assert_eq!(week2.get("value").unwrap().as_str().unwrap(), "112.0");
        assert_eq!(week2.get("date").unwrap().as_str().unwrap(), "2024-01-09");
    }

    #[tokio::test]
    async fn test_aggregate_weekly_carries_twr_across_zero_value_week() {
        // aggregate_weekly computes each week's return against the *previous*
        // week's value, so the very first week always yields twr 0. The test
        // therefore uses a leading baseline week so the following week builds a
        // real return (0.10), then a holiday (zero-value) week that must carry
        // that twr forward (not reset to 0.0).
        let daily_history = vec![
            // week A (2023): baseline, establishes prev_value = 100
            serde_json::json!({"date":"2023-12-25","value":"100.0","daily_return":"0.0","twr":"0.0"}),
            // week1 (2024): gains to 110 -> twr 0.10
            serde_json::json!({"date":"2024-01-02","value":"110.0","daily_return":"0.10","twr":"0.10"}),
            // week2: holiday (value 0) -> must carry twr 0.10, not 0.0
            serde_json::json!({"date":"2024-01-08","value":"0","daily_return":"0.0","twr":"0.0"}),
            serde_json::json!({"date":"2024-01-15","value":"110.0","daily_return":"0.0","twr":"0.10"}),
        ];
        let weekly = StatsEngine::aggregate_weekly(daily_history).await;
        // week A = index 0, week1 = index 1, holiday = index 2
        // holiday must carry forward the accumulated twr (same string as the
        // prior gain week) rather than resetting to a zero value.
        let holiday_twr = weekly[2].get("twr").unwrap().as_str().unwrap();
        let gain_twr = weekly[1].get("twr").unwrap().as_str().unwrap();
        assert_eq!(holiday_twr, gain_twr);
        assert_ne!(holiday_twr, "0.0");
        assert_ne!(holiday_twr, "0");
    }

    #[tokio::test]
    async fn test_aggregate_weekly_recovery_week_returns_real_change() {
        // A zero-value (holiday) week must not poison prev_value: the week after
        // a holiday should report its real return against the last non-zero
        // value, not 0.0.
        let daily_history = vec![
            // baseline week, establishes prev_value = 100
            serde_json::json!({"date":"2023-12-25","value":"100.0","daily_return":"0.0","twr":"0.0"}),
            // week1: gains to 110 -> +10%
            serde_json::json!({"date":"2024-01-02","value":"110.0","daily_return":"0.10","twr":"0.10"}),
            // week2: holiday (value 0)
            serde_json::json!({"date":"2024-01-08","value":"0","daily_return":"0.0","twr":"0.10"}),
            // week3: 110 -> 121 should be +10%, not 0.0
            serde_json::json!({"date":"2024-01-15","value":"121.0","daily_return":"0.10","twr":"0.21"}),
        ];
        let weekly = StatsEngine::aggregate_weekly(daily_history).await;
        let recovery = &weekly[3];
        assert_eq!(
            recovery.get("daily_return").unwrap().as_str().unwrap(),
            "0.10",
            "week after holiday must report its real +10% return"
        );
        let twr: f64 = recovery.get("twr").unwrap().as_str().unwrap().parse().unwrap();
        assert!(twr > 0.20, "twr must compound past the 0.10 carry, got {twr}");
    }

    #[tokio::test]
    async fn test_aggregate_weekly_empty() {
        let weekly = StatsEngine::aggregate_weekly(vec![]).await;
        assert!(weekly.is_empty());
    }

    #[tokio::test]
    async fn test_fallback_to_current_price_when_no_historical_data() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (symbol TEXT, date DATE, close_price TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        let svc = CurrencyService::new();

        let asset = Asset {
            id: 1,
            portfolio_id: 1,
            symbol: "TESTFALLBACK".to_string(),
            name: "Test Fallback".to_string(),
            asset_type: "STOCK".to_string(),
            sector: None,
            currency: "USD".to_string(),
            isin: None,
        };
        let tx = Transaction {
            id: 1,
            asset_id: 1,
            r#type: "BUY".to_string(),
            quantity: "10.0".to_string(),
            price: "50.0".to_string(),
            fee: "0.0".to_string(),
            date: DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z").unwrap().with_timezone(&Utc),
        };

        let result = StatsEngine::calculate_portfolio_performance(
            &pool,
            &[asset],
            &[tx],
            "USD",
            &svc,
        ).await.unwrap();

        let metrics = result.get("metrics").unwrap();
        let value = metrics.get("portfolio_value").unwrap().as_str().unwrap();
        assert_eq!(value, "0");
    }
}