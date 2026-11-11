use crate::models::Transaction;
use crate::schemas::{AssetTaxSummary, DatedLot, DatedReplay, LotMatch, SellBreakdown, TaxLot};
use crate::services::currency_service::CurrencyService;
use rust_decimal::Decimal;
use chrono::{DateTime, NaiveDate, Utc};
use anyhow::Result;
use sqlx::sqlite::SqlitePool;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::currency_service::CurrencyService;
    use chrono::Utc as ChronoUtc;
    use std::str::FromStr;

    fn make_tx(id: i32, tx_type: &str, qty: &str, price: &str, fee: &str, date_str: &str) -> Transaction {
        Transaction {
            id,
            asset_id: 1,
            r#type: tx_type.to_string(),
            quantity: qty.to_string(),
            price: price.to_string(),
            fee: fee.to_string(),
            date: DateTime::parse_from_rfc3339(date_str).unwrap().with_timezone(&ChronoUtc),
        }
    }

    /// Every in-file fixture is USD/USD, which `get_rate` short-circuits before it
    /// touches the database, so an empty pool is a real pool that answers nothing.
    /// There are no seeded prices in this file to look for.
    async fn empty_pool() -> SqlitePool {
        SqlitePool::connect(":memory:").await.unwrap()
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// A pool with the real schema, for the fixtures that seed FX rows. `empty_pool()`
    /// above is a bare `:memory:` connection with no tables, which is enough for the
    /// USD/USD fixtures and not enough for these.
    async fn priced_pool() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        let cfg = crate::auth::StartupConfig::from_env_with(&|key| {
            (key == "AUTH_MODE").then(|| "disabled".to_string())
        })
        .expect("AUTH_MODE=disabled parses in a debug build");
        crate::init_db(&pool, &cfg).await.unwrap();
        pool
    }

    async fn seed_fx(pool: &SqlitePool, symbol: &str, rows: &[(&str, &str)]) {
        for (date, price) in rows {
            sqlx::query("INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
                .bind(symbol)
                .bind(day(date))
                .bind(*price)
                .execute(pool)
                .await
                .unwrap();
        }
    }

    async fn run_engine(transactions: &[Transaction], strategy: &str, threshold_days: i64) -> AssetTaxSummary {
        let svc = CurrencyService::new();
        let pool = empty_pool().await;
        TaxLotEngine::calculate_lots(
            "TEST", "STOCK", transactions, Decimal::from_str("110.0").unwrap(), "USD", "USD", &svc, strategy, threshold_days, &pool,
        ).await.unwrap()
    }

    #[tokio::test]
    async fn test_fifo_matches_oldest_lot_first() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "100.0", "110.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "50.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.tax_lots.len(), 2);
        assert_eq!(result.tax_lots[0].remaining_qty, Decimal::from_str("50.0").unwrap());
        assert_eq!(result.tax_lots[1].remaining_qty, Decimal::from_str("100.0").unwrap());
        assert_eq!(result.realized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[tokio::test]
    async fn test_lifo_matches_newest_lot_first() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "100.0", "110.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "50.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "LIFO", 30).await;
        assert_eq!(result.tax_lots.len(), 2);
        assert_eq!(result.tax_lots[0].remaining_qty, Decimal::from_str("100.0").unwrap());
        assert_eq!(result.tax_lots[1].remaining_qty, Decimal::from_str("50.0").unwrap());
        assert_eq!(result.realized_pnl, Decimal::from_str("500.0").unwrap());
    }

    #[tokio::test]
    async fn test_hybrid_sells_short_term_first() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "100.0", "110.0", "0.0", "2024-11-15T00:00:00Z"),
            make_tx(3, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "HYBRID", 30).await;
        assert_eq!(result.tax_lots.len(), 1);
        assert_eq!(result.tax_lots[0].remaining_qty, Decimal::from_str("100.0").unwrap());
        assert_eq!(result.realized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[tokio::test]
    async fn test_sell_exceeds_total_buys() {
        let txs = vec![
            make_tx(1, "BUY", "50.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.tax_lots.len(), 0);
        assert_eq!(result.current_shares, Decimal::ZERO);
        assert_eq!(result.realized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[tokio::test]
    async fn test_sell_with_no_prior_buys_ignored() {
        let txs = vec![
            make_tx(1, "SELL", "50.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.tax_lots.len(), 0);
        assert_eq!(result.realized_pnl, Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_fee_increases_cost_basis() {
        let txs_no_fee = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
        ];
        let txs_with_fee = vec![
            make_tx(1, "BUY", "100.0", "100.0", "50.0", "2024-01-01T00:00:00Z"),
        ];
        let r_no_fee = run_engine(&txs_no_fee, "FIFO", 30).await;
        let r_with_fee = run_engine(&txs_with_fee, "FIFO", 30).await;
        let expected_diff = Decimal::from_str("0.5").unwrap();
        assert_eq!(r_with_fee.average_cost - r_no_fee.average_cost, expected_diff);
    }

    #[tokio::test]
    async fn test_empty_transactions_returns_zero() {
        let result = run_engine(&[], "FIFO", 30).await;
        assert_eq!(result.current_shares, Decimal::ZERO);
        assert_eq!(result.total_cost, Decimal::ZERO);
        assert_eq!(result.market_value, Decimal::ZERO);
        assert_eq!(result.tax_lots.len(), 0);
    }

    #[tokio::test]
    async fn test_single_buy_no_sells() {
        let txs = vec![
            make_tx(1, "BUY", "200.0", "50.0", "10.0", "2024-01-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.tax_lots.len(), 1);
        assert_eq!(result.average_cost, Decimal::from_str("50.05").unwrap());
        assert_eq!(result.market_value, Decimal::from_str("22000.0").unwrap());
        assert_eq!(result.unrealized_pnl, Decimal::from_str("11990.0").unwrap());
    }

    #[tokio::test]
    async fn test_partial_sell_across_multiple_lots() {
        let txs = vec![
            make_tx(1, "BUY", "60.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "40.0", "120.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "80.0", "130.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.tax_lots.len(), 1);
        assert_eq!(result.tax_lots[0].remaining_qty, Decimal::from_str("20.0").unwrap());
        assert_eq!(result.realized_pnl, Decimal::from_str("2000.0").unwrap());
    }

    #[tokio::test]
    async fn test_forward_split_adjusts_qty_and_cost_basis() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SPLIT", "2.0", "1.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "100.0", "60.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.current_shares, Decimal::from_str("100.0").unwrap());
        assert_eq!(result.average_cost, Decimal::from_str("50.0").unwrap());
        assert_eq!(result.realized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[tokio::test]
    async fn test_reverse_split_adjusts_qty_and_cost_basis() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "10.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SPLIT", "1.0", "10.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "5.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.current_shares, Decimal::from_str("5.0").unwrap());
        assert_eq!(result.average_cost, Decimal::from_str("100.0").unwrap());
        assert_eq!(result.realized_pnl, Decimal::from_str("100.0").unwrap());
    }

    #[tokio::test]
    async fn test_split_on_same_day_as_sell_applies_first() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SPLIT", "2.0", "1.0", "0.0", "2024-12-01T00:00:00Z"),
            make_tx(3, "SELL", "100.0", "60.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.current_shares, Decimal::from_str("100.0").unwrap());
        assert_eq!(result.realized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[tokio::test]
    async fn test_split_with_no_prior_lots_is_noop() {
        let txs = vec![
            make_tx(1, "SPLIT", "2.0", "1.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "100.0", "100.0", "0.0", "2024-06-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.current_shares, Decimal::from_str("100.0").unwrap());
        assert_eq!(result.average_cost, Decimal::from_str("100.0").unwrap());
        assert_eq!(result.realized_pnl, Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_multi_currency_conversion() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SELL", "50.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let result = run_engine(&txs, "FIFO", 30).await;
        assert_eq!(result.tax_lots.len(), 1);
        assert_eq!(result.current_shares, Decimal::from_str("50.0").unwrap());
    }

    async fn run_breakdown(txs: &[Transaction], sell_id: i32, strategy: &str, threshold: i64) -> Option<SellBreakdown> {
        let svc = CurrencyService::new();
        let pool = empty_pool().await;
        TaxLotEngine::sell_breakdown(
            sell_id, txs, "USD", "USD", &svc, strategy, threshold, &pool,
        ).await.unwrap()
    }

    fn two_buys_one_sell() -> Vec<Transaction> {
        vec![
            make_tx(1, "BUY", "60.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "40.0", "120.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "80.0", "130.0", "0.0", "2024-12-01T00:00:00Z"),
        ]
    }

    #[tokio::test]
    async fn test_sell_breakdown_fifo_consumes_oldest_first() {
        let txs = two_buys_one_sell();
        let b = run_breakdown(&txs, 3, "FIFO", 30).await.expect("breakdown for sell 3");
        assert_eq!(b.sell_transaction_id, 3);
        assert_eq!(b.sold_qty, Decimal::from_str("80.0").unwrap());
        assert_eq!(b.matches.len(), 2);
        assert_eq!(b.matches[0].buy_transaction_id, 1);
        assert_eq!(b.matches[0].matched_qty, Decimal::from_str("60.0").unwrap());
        assert_eq!(b.matches[0].lot_qty_before_match, Decimal::from_str("60.0").unwrap());
        assert_eq!(b.matches[0].pnl, Decimal::from_str("1800.0").unwrap());
        assert_eq!(b.matches[0].holding_days, 335);
        assert_eq!(b.matches[1].buy_transaction_id, 2);
        assert_eq!(b.matches[1].matched_qty, Decimal::from_str("20.0").unwrap());
        assert_eq!(b.matches[1].lot_qty_before_match, Decimal::from_str("40.0").unwrap());
        assert_eq!(b.matches[1].pnl, Decimal::from_str("200.0").unwrap());
        assert_eq!(b.matches[1].holding_days, 183);
        assert_eq!(b.realized_pnl, Decimal::from_str("2000.0").unwrap());
        assert_eq!(b.unmatched_qty, Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_sell_breakdown_lifo_consumes_newest_first() {
        let txs = two_buys_one_sell();
        let b = run_breakdown(&txs, 3, "LIFO", 30).await.unwrap();
        assert_eq!(b.matches[0].buy_transaction_id, 2);
        assert_eq!(b.matches[0].matched_qty, Decimal::from_str("40.0").unwrap());
        assert_eq!(b.matches[1].buy_transaction_id, 1);
        assert_eq!(b.realized_pnl, Decimal::from_str("1600.0").unwrap());
    }

    #[tokio::test]
    async fn test_sell_breakdown_term_is_null_except_hybrid() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "100.0", "110.0", "0.0", "2024-11-15T00:00:00Z"),
            make_tx(3, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let fifo = run_breakdown(&txs, 3, "FIFO", 30).await.unwrap();
        assert_eq!(fifo.matches[0].term, None, "FIFO must not claim a tax term");
        assert_eq!(fifo.matches[0].holding_days, 335);

        let hybrid = run_breakdown(&txs, 3, "HYBRID", 30).await.unwrap();
        assert_eq!(hybrid.matches[0].buy_transaction_id, 2, "HYBRID takes the 16-day lot first");
        assert_eq!(hybrid.matches[0].term, Some("SHORT".to_string()));
        assert_eq!(hybrid.matches[0].holding_days, 16);
    }

    #[tokio::test]
    async fn test_sell_breakdown_hybrid_labels_long_term_bucket() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let b = run_breakdown(&txs, 2, "HYBRID", 30).await.unwrap();
        assert_eq!(b.matches[0].term, Some("LONG".to_string()));
    }

    #[tokio::test]
    async fn test_sell_breakdown_reports_unmatched_quantity() {
        let txs = vec![
            make_tx(1, "BUY", "50.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let b = run_breakdown(&txs, 2, "FIFO", 30).await.unwrap();
        assert_eq!(b.matched_qty, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.unmatched_qty, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.matches.len(), 1);
        // Only the matched half contributes: 50 * (120 - 100), not 100 * 20.
        assert_eq!(b.realized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[tokio::test]
    async fn test_sell_breakdown_before_any_buy_is_all_unmatched() {
        let txs = vec![
            make_tx(1, "SELL", "50.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
            make_tx(2, "BUY", "50.0", "100.0", "0.0", "2025-01-01T00:00:00Z"),
        ];
        let b = run_breakdown(&txs, 1, "FIFO", 30).await.unwrap();
        assert!(b.matches.is_empty(), "a sell cannot consume a lot bought after it");
        assert_eq!(b.unmatched_qty, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.matched_qty, Decimal::ZERO);
        assert_eq!(b.cost_basis, Decimal::ZERO);
        assert_eq!(b.realized_pnl, Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_sell_breakdown_reports_split_adjusted_cost() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SPLIT", "2.0", "1.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "100.0", "60.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let b = run_breakdown(&txs, 3, "FIFO", 30).await.unwrap();
        assert_eq!(b.matches.len(), 1);
        assert_eq!(b.matches[0].buy_price, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.matches[0].unit_cost, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.matches[0].lot_qty_before_match, Decimal::from_str("200.0").unwrap());
        assert_eq!(b.matches[0].cost_basis, Decimal::from_str("5000.0").unwrap());
        assert_eq!(b.realized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[tokio::test]
    async fn test_sell_breakdown_unknown_id_returns_none() {
        let txs = two_buys_one_sell();
        assert!(run_breakdown(&txs, 999, "FIFO", 30).await.is_none());
    }

    #[tokio::test]
    async fn test_sell_breakdowns_sum_to_asset_realized_pnl() {
        // The reconciliation invariant, checked under every strategy, over a
        // fixture with two sells, a partial lot, an unmatched tail and a split.
        let txs = vec![
            make_tx(1, "BUY", "60.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "40.0", "120.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SPLIT", "2.0", "1.0", "0.0", "2024-08-01T00:00:00Z"),
            make_tx(4, "SELL", "80.0", "130.0", "5.0", "2024-12-01T00:00:00Z"),
            make_tx(5, "SELL", "300.0", "70.0", "0.0", "2025-01-01T00:00:00Z"),
        ];
        for strategy in ["FIFO", "LIFO", "HYBRID"] {
            let svc = CurrencyService::new();
            let pool = empty_pool().await;
            let summary = TaxLotEngine::calculate_lots(
                "TEST", "STOCK", &txs, Decimal::from_str("110.0").unwrap(),
                "USD", "USD", &svc, strategy, 30, &pool,
            ).await.unwrap();

            let sorted = TaxLotEngine::sorted_transactions(&txs);
            let (_, breakdowns) = TaxLotEngine::replay(
                &sorted, &pool, &svc, "USD", "USD", strategy, 30,
            ).await.unwrap();

            let summed: Decimal = breakdowns.iter().map(|b| b.realized_pnl).sum();
            assert_eq!(summed, summary.realized_pnl, "per-sell totals must equal the asset total under {strategy}");

            let sold: Decimal = breakdowns.iter().map(|b| b.sold_qty).sum();
            let accounted: Decimal = breakdowns.iter().map(|b| b.matched_qty + b.unmatched_qty).sum();
            assert_eq!(sold, accounted, "every sold unit is either matched or declared unmatched under {strategy}");
        }
    }

    #[tokio::test]
    async fn test_replay_at_excludes_a_sell_after_the_end_date() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let svc = CurrencyService::new();
        let pool = empty_pool().await;
        let out = TaxLotEngine::replay_at(
            &txs, day("2024-06-30"), day("2024-06-30"),
            Decimal::from_str("110.0").unwrap(), "USD", "USD", &svc, "FIFO", 30, &pool,
        )
        .await
        .unwrap();

        assert!(out.sell_breakdowns.is_empty(), "a sell after `end` is outside the window");
        assert_eq!(out.open_lots.len(), 1);
        assert_eq!(out.open_lots[0].remaining_qty, Decimal::from_str("100.0").unwrap());
        assert_eq!(out.open_lots[0].buy_transaction_id, 1);
    }

    #[tokio::test]
    async fn test_replay_at_includes_transactions_dated_exactly_the_end_date() {
        // `end` is INCLUSIVE, and for a quarterly capital-gains report that boundary is the
        // quarter end: a sale on the last day of the period belongs to the period. An
        // off-by-one using `<` instead of `<=` would silently drop every transaction dated
        // on the closing day and understate realized P&L with no other symptom.
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SELL", "40.0", "120.0", "0.0", "2024-06-30T00:00:00Z"),
        ];
        let svc = CurrencyService::new();
        let pool = empty_pool().await;
        let out = TaxLotEngine::replay_at(
            &txs, day("2024-06-30"), day("2024-06-30"),
            Decimal::from_str("110.0").unwrap(), "USD", "USD", &svc, "FIFO", 30, &pool,
        )
        .await
        .unwrap();

        assert_eq!(
            out.sell_breakdowns.len(),
            1,
            "a sell dated exactly `end` is inside the window, not outside it"
        );
        assert_eq!(out.open_lots[0].remaining_qty, Decimal::from_str("60.0").unwrap());
    }

    #[tokio::test]
    async fn test_replay_at_an_empty_window_reports_nothing_and_costs_no_rate() {
        // Everything is dated after `end`, so there are no lots and no market value to
        // convert. This pins the SHAPE of the early return (no lots, no breakdowns, zero
        // price, no rate date). The "costs no outbound call" property is true of the code
        // but is proven by POSITION, not by this fixture: the return sits above the FX
        // lookup, and this fixture is USD/USD, which would skip that lookup anyway. A
        // foreign-currency fixture that proved it would reach the provider when the code
        // was wrong, which the no-network rule forbids.
        let txs = vec![make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-07-01T00:00:00Z")];
        let svc = CurrencyService::new();
        let pool = empty_pool().await;
        let out = TaxLotEngine::replay_at(
            &txs, day("2024-06-30"), day("2024-06-30"),
            Decimal::from_str("110.0").unwrap(), "USD", "USD", &svc, "FIFO", 30, &pool,
        )
        .await
        .unwrap();

        assert!(out.open_lots.is_empty(), "nothing was bought on or before `end`");
        assert!(out.sell_breakdowns.is_empty());
        assert_eq!(out.current_price_base, Decimal::ZERO);
        assert_eq!(out.fx_rate_date_used, None, "no conversion happened, so no rate date to report");
    }

    #[tokio::test]
    async fn test_replay_at_a_split_after_end_does_not_move_the_window() {
        // The split branch rescales every lot in scope, so the filter has to run before
        // sorting and matching rather than after. Without the filter this fixture
        // reports a 200-unit lot at a 50.0 unit cost for a window that closed a month
        // before the split happened.
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SPLIT", "2.0", "1.0", "0.0", "2024-08-01T00:00:00Z"),
        ];
        let svc = CurrencyService::new();
        let pool = empty_pool().await;
        let out = TaxLotEngine::replay_at(
            &txs, day("2024-06-30"), day("2024-06-30"),
            Decimal::from_str("100.0").unwrap(), "USD", "USD", &svc, "FIFO", 30, &pool,
        )
        .await
        .unwrap();

        assert_eq!(out.open_lots.len(), 1);
        assert_eq!(out.open_lots[0].remaining_qty, Decimal::from_str("100.0").unwrap());
        assert_eq!(out.open_lots[0].original_qty, Decimal::from_str("100.0").unwrap());
        assert_eq!(out.open_lots[0].unit_cost, Decimal::from_str("100.0").unwrap());
        assert!(out.sell_breakdowns.is_empty());
    }

    #[tokio::test]
    async fn test_replay_at_prices_the_market_value_at_the_valuation_date() {
        // Review Focus 1. The realized leg converts the buy at the buy's own date
        // (1.0000 here) and the market-value leg converts at `valuation_date` (1.2500).
        // A replay that used one rate for both legs reports a 10000.0 market value and
        // no gain at all -- plausible enough to survive a reader who is not looking for
        // a date.
        let pool = priced_pool().await;
        // Deliberately NO row on 2024-03-31. `select_rate` answers with the newest row
        // dated at or before the request, so the rate comes from 2024-03-30 and
        // `fx_rate_date_used` must say 03-30. Seeding a row exactly on the requested date
        // would let an implementation that echoes `valuation_date` and ignores
        // `at.date_used` pass every assertion here — which is the whole point of this
        // field: it is provenance, and provenance that restates the question is worthless.
        seed_fx(&pool, "EURUSD=X", &[
            ("2024-01-01", "1.0000"),
            ("2024-03-29", "1.2500"),
            ("2024-03-30", "1.2500"),
        ]).await;
        let txs = vec![make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z")];
        let svc = CurrencyService::new();

        let out = TaxLotEngine::replay_at(
            &txs, day("2024-03-31"), day("2024-03-31"),
            Decimal::from_str("100.0").unwrap(), "EUR", "USD", &svc, "FIFO", 30, &pool,
        )
        .await
        .unwrap();

        assert_eq!(out.fx_rate_date_used, Some(day("2024-03-30")),
            "the rate came from the 03-30 row, so that is the date this must report");
        assert_eq!(out.current_price_base, Decimal::from_str("125.0").unwrap());
        assert_eq!(out.open_lots[0].unit_cost, Decimal::from_str("100.0").unwrap(),
            "the realized leg converts the buy at the buy's own date");
        assert_eq!(out.open_lots[0].market_value, Decimal::from_str("12500.0").unwrap());
        assert_eq!(out.open_lots[0].unrealized_gain_loss, Decimal::from_str("2500.0").unwrap());
    }

    #[tokio::test]
    async fn test_replay_at_a_split_inside_the_window_still_rescales_the_lots() {
        // The mirror of the split-after-`end` test. The filter's whole purpose is to keep
        // post-`end` splits OUT; that only means something if in-window splits are still
        // applied. A filter written too aggressively would drop both and still satisfy the
        // exclusion test. A 2:1 split on the closing day doubles quantity and halves unit
        // cost, and both quantity fields move together.
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SPLIT", "2.0", "1.0", "0.0", "2024-06-30T00:00:00Z"),
        ];
        let svc = CurrencyService::new();
        let pool = empty_pool().await;
        let out = TaxLotEngine::replay_at(
            &txs, day("2024-06-30"), day("2024-06-30"),
            Decimal::from_str("100.0").unwrap(), "USD", "USD", &svc, "FIFO", 30, &pool,
        )
        .await
        .unwrap();

        assert_eq!(out.open_lots.len(), 1);
        assert_eq!(out.open_lots[0].remaining_qty, Decimal::from_str("200.0").unwrap());
        assert_eq!(out.open_lots[0].original_qty, Decimal::from_str("200.0").unwrap());
        assert_eq!(out.open_lots[0].unit_cost, Decimal::from_str("50.0").unwrap());
    }

    #[tokio::test]
    async fn test_replay_at_agrees_with_calculate_lots_when_both_look_at_today() {
        // Two copies of the same valuation loop is the cost of leaving `calculate_lots`
        // alone; this is what keeps them from drifting. USD/USD, so neither leg needs a
        // rate and `calculate_lots`'s `Utc::now()` is a no-op.
        let txs = vec![
            make_tx(1, "BUY", "60.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "40.0", "120.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "80.0", "130.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let end = day("2030-01-01");
        let price = Decimal::from_str("110.0").unwrap();

        for strategy in ["FIFO", "LIFO", "HYBRID"] {
            let svc = CurrencyService::new();
            let pool = empty_pool().await;
            let summary = TaxLotEngine::calculate_lots(
                "TEST", "STOCK", &txs, price, "USD", "USD", &svc, strategy, 30, &pool,
            )
            .await
            .unwrap();
            let out = TaxLotEngine::replay_at(
                &txs, end, end, price, "USD", "USD", &svc, strategy, 30, &pool,
            )
            .await
            .unwrap();

            assert_eq!(out.open_lots.len(), summary.tax_lots.len(), "{strategy}: one row per open lot");
            for (dated, taxed) in out.open_lots.iter().zip(summary.tax_lots.iter()) {
                assert_eq!(dated.buy_transaction_id, taxed.buy_transaction_id, "{strategy}");
                assert_eq!(dated.buy_price, taxed.buy_price, "{strategy}");
                assert_eq!(dated.original_qty, taxed.original_qty, "{strategy}");
                assert_eq!(dated.remaining_qty, taxed.remaining_qty, "{strategy}");
                assert_eq!(dated.unrealized_gain_loss, taxed.latent_gain_loss, "{strategy}");
                assert_eq!(
                    dated.unrealized_roi.unwrap_or(Decimal::ZERO),
                    taxed.latent_roi,
                    "{strategy}: the dated report calls a zero-cost lot's ROI null, the undated summary calls it zero",
                );
            }
            assert_eq!(
                out.sell_breakdowns.iter().map(|b| b.realized_pnl).sum::<Decimal>(),
                summary.realized_pnl,
                "{strategy}: every breakdown is present, so the selection is the only difference",
            );
        }
    }
}

pub struct TaxLotEngine;

struct InternalLot {
    source_tx_id: i32,
    date: DateTime<Utc>,
    price: Decimal,
    qty: Decimal,
    /// The quantity this lot was opened with, split-adjusted alongside `qty`, so a
    /// lot that has been partly sold still reports what was bought.
    original_qty: Decimal,
    unit_cost: Decimal,
}

impl TaxLotEngine {
    /// Canonical transaction order. Splits must apply to existing lots before
    /// any same-day buy/sell, so they sort ahead of everything else on a tie.
    fn sorted_transactions(transactions: &[Transaction]) -> Vec<Transaction> {
        let split_priority = |tx: &Transaction| {
            if tx.r#type.to_uppercase() == "SPLIT" { 0 } else { 1 }
        };
        let mut sorted_txs = transactions.to_vec();
        sorted_txs.sort_by(|a, b| {
            a.date.cmp(&b.date).then_with(|| split_priority(a).cmp(&split_priority(b)))
        });
        sorted_txs
    }

    /// Single source of truth for lot matching. Both public entry points call
    /// this, which is what makes per-sell realized P&L reconcile with the
    /// asset-level total: `realized_pnl` is *derived* from the returned
    /// breakdowns by the caller, never accumulated on a separate path.
    async fn replay(
        sorted_txs: &[Transaction],
        pool: &SqlitePool,
        currency_service: &CurrencyService,
        asset_currency: &str,
        base_currency: &str,
        strategy: &str,
        hybrid_threshold_days: i64,
    ) -> Result<(Vec<InternalLot>, Vec<SellBreakdown>)> {
        let mut buy_lots: Vec<InternalLot> = Vec::new();
        let mut breakdowns: Vec<SellBreakdown> = Vec::new();

        for tx in sorted_txs {
            let tx_rate = if asset_currency != base_currency {
                currency_service
                    .get_rate(asset_currency, base_currency, tx.date, pool)
                    .await?
                    .rate
            } else {
                Decimal::ONE
            };

            let tx_qty = crate::db_types::str_to_decimal(&tx.quantity);
            let tx_price = crate::db_types::str_to_decimal(&tx.price);
            let tx_fee = crate::db_types::str_to_decimal(&tx.fee);

            if tx.r#type.to_uppercase() == "BUY" {
                let unit_cost_asset = if tx_qty > Decimal::ZERO {
                    (tx_qty * tx_price + tx_fee) / tx_qty
                } else {
                    tx_price
                };
                let unit_cost_base = unit_cost_asset * tx_rate;
                buy_lots.push(InternalLot {
                    source_tx_id: tx.id,
                    date: tx.date,
                    price: tx_price * tx_rate,
                    qty: tx_qty,
                    original_qty: tx_qty,
                    unit_cost: unit_cost_base,
                });
            } else if tx.r#type.to_uppercase() == "SELL" {
                let mut qty_to_sell = tx_qty;
                let sell_unit_proceeds_asset = if tx_qty > Decimal::ZERO {
                    (tx_qty * tx_price - tx_fee) / tx_qty
                } else {
                    tx_price
                };
                let sell_unit_proceeds_base = sell_unit_proceeds_asset * tx_rate;

                let mut eligible_indices: Vec<usize> = buy_lots.iter().enumerate()
                    .filter(|(_, lot)| lot.date <= tx.date && lot.qty > Decimal::ZERO)
                    .map(|(i, _)| i)
                    .collect();

                match strategy.to_uppercase().as_str() {
                    "FIFO" => {
                        eligible_indices.sort_by(|&a, &b| buy_lots[a].date.cmp(&buy_lots[b].date));
                    }
                    "LIFO" => {
                        eligible_indices.sort_by(|&a, &b| buy_lots[b].date.cmp(&buy_lots[a].date));
                    }
                    "HYBRID" => {
                        let mut short_term = Vec::new();
                        let mut long_term = Vec::new();
                        for &idx in &eligible_indices {
                            let lot = &buy_lots[idx];
                            let age = tx.date.signed_duration_since(lot.date).num_days();
                            if age >= 0 && age <= hybrid_threshold_days {
                                short_term.push(idx);
                            } else {
                                long_term.push(idx);
                            }
                        }
                        short_term.sort_by(|&a, &b| buy_lots[b].date.cmp(&buy_lots[a].date));
                        long_term.sort_by(|&a, &b| buy_lots[a].date.cmp(&buy_lots[b].date));
                        eligible_indices = [short_term, long_term].concat();
                    }
                    _ => {
                        eligible_indices.sort_by(|&a, &b| buy_lots[a].date.cmp(&buy_lots[b].date));
                    }
                }

                let mut matches: Vec<LotMatch> = Vec::new();

                for idx in eligible_indices {
                    if qty_to_sell <= Decimal::ZERO {
                        break;
                    }
                    let lot = &mut buy_lots[idx];
                    let matched_qty = qty_to_sell.min(lot.qty);
                    let cost_basis = matched_qty * lot.unit_cost;
                    let proceeds = matched_qty * sell_unit_proceeds_base;
                    let holding_days = tx.date.signed_duration_since(lot.date).num_days();

                    matches.push(LotMatch {
                        buy_transaction_id: lot.source_tx_id,
                        buy_date: lot.date,
                        buy_price: lot.price,
                        unit_cost: lot.unit_cost,
                        matched_qty,
                        // Read before the mutation below: this is what lets the
                        // view tell "drained the lot" from "partially used".
                        lot_qty_before_match: lot.qty,
                        cost_basis,
                        proceeds,
                        pnl: proceeds - cost_basis,
                        holding_days,
                        term: term_for(strategy, holding_days, hybrid_threshold_days),
                    });

                    lot.qty -= matched_qty;
                    qty_to_sell -= matched_qty;
                }

                // Totals are derived from the matches, not carried alongside
                // them, so the per-sell figure and its lot table cannot disagree.
                let matched_qty: Decimal = matches.iter().map(|m| m.matched_qty).sum();
                let cost_basis: Decimal = matches.iter().map(|m| m.cost_basis).sum();
                let proceeds: Decimal = matches.iter().map(|m| m.proceeds).sum();

                breakdowns.push(SellBreakdown {
                    sell_transaction_id: tx.id,
                    sell_date: tx.date,
                    sold_qty: tx_qty,
                    matched_qty,
                    unmatched_qty: tx_qty - matched_qty,
                    proceeds,
                    cost_basis,
                    realized_pnl: proceeds - cost_basis,
                    matches,
                });
            } else if tx.r#type.to_uppercase() == "SPLIT" {
                // Split ratio = quantity : price (shares after : shares before).
                // Existing lots scale in quantity and divide in unit cost so total
                // cost basis is unchanged and no P&L is realized.
                let denominator = tx_price;
                if tx_qty > Decimal::ZERO && denominator > Decimal::ZERO {
                    let ratio = tx_qty / denominator;
                    for lot in buy_lots.iter_mut() {
                        lot.qty *= ratio;
                        lot.original_qty *= ratio;
                        lot.unit_cost /= ratio;
                        lot.price /= ratio;
                    }
                }
            }
        }

        Ok((buy_lots, breakdowns))
    }

    /// `too_many_arguments` is silenced rather than grouping the arguments into a
    /// struct: the tenth is the `SqlitePool` threaded through so `replay`'s FX calls
    /// can read dated closes, and a params struct is a wider refactor than this fix.
    #[allow(clippy::too_many_arguments)]
    pub async fn calculate_lots(
        symbol: &str,
        asset_type: &str,
        transactions: &[Transaction],
        current_price: Decimal,
        asset_currency: &str,
        base_currency: &str,
        currency_service: &CurrencyService,
        strategy: &str,
        hybrid_threshold_days: i64,
        pool: &SqlitePool,
    ) -> Result<AssetTaxSummary> {
        let sorted_txs = Self::sorted_transactions(transactions);

        let current_price_base = if asset_currency != base_currency {
            // The market-value leg marks the position at the price the caller is
            // quoting *now*, so the FX leg has to be the rate that prices now: a
            // current close converted at the rate recorded on the latest transaction
            // date would mark a 2026 price with a 2023 exchange rate. The realized
            // legs in `replay` are each converted at their own transaction date, which
            // is the correct rule for a historical figure and the reason the two legs
            // cannot share one rate lookup.
            let rate = currency_service
                .get_rate(asset_currency, base_currency, Utc::now(), pool)
                .await?
                .rate;
            current_price * rate
        } else {
            current_price
        };

        let (buy_lots, breakdowns) = Self::replay(
            &sorted_txs,
            pool,
            currency_service,
            asset_currency,
            base_currency,
            strategy,
            hybrid_threshold_days,
        )
        .await?;

        // Derived, not accumulated: the asset total is the sum of the per-sell
        // totals by construction, so the two can never drift apart.
        let realized_pnl: Decimal = breakdowns.iter().map(|b| b.realized_pnl).sum();
        let mut open_lots = Vec::new();
        let mut total_remaining_qty = Decimal::ZERO;
        let mut total_cost_basis = Decimal::ZERO;

        for lot in buy_lots {
            if lot.qty > Decimal::ZERO {
                total_remaining_qty += lot.qty;
                let lot_cost = lot.qty * lot.unit_cost;
                total_cost_basis += lot_cost;

                let lot_market_value = lot.qty * current_price_base;
                let latent_gain_loss = lot_market_value - lot_cost;
                let latent_roi = if lot_cost > Decimal::ZERO { latent_gain_loss / lot_cost } else { Decimal::ZERO };

                open_lots.push(TaxLot {
                    buy_transaction_id: lot.source_tx_id,
                    buy_date: lot.date,
                    buy_price: lot.price,
                    original_qty: lot.original_qty,
                    remaining_qty: lot.qty,
                    latent_gain_loss,
                    latent_roi,
                });
            }
        }

        let market_value = total_remaining_qty * current_price_base;
        let unrealized_pnl = market_value - total_cost_basis;
        let unrealized_roi = if total_cost_basis > Decimal::ZERO { unrealized_pnl / total_cost_basis } else { Decimal::ZERO };
        let average_cost = if total_remaining_qty > Decimal::ZERO { total_cost_basis / total_remaining_qty } else { Decimal::ZERO };

        Ok(AssetTaxSummary {
            symbol: symbol.to_string(),
            asset_type: asset_type.to_string(),
            current_shares: total_remaining_qty,
            average_cost,
            current_price: current_price_base,
            total_cost: total_cost_basis,
            market_value,
            unrealized_pnl,
            unrealized_roi,
            realized_pnl,
            tax_lots: open_lots,
        })
    }

    /// The engine's one dated entry point: the open lots and every sell's lot
    /// attribution at `end`, from a single pass.
    ///
    /// `end` filters transactions; `valuation_date` prices them for the market value. A
    /// caller that has no separate valuation date passes `end` for both — which is what
    /// the terminated report does. The two are separate parameters because the
    /// alternative (deriving the FX date from the transaction list, the way the old
    /// `latest_date` did) silently prices a past date with whatever rate the last
    /// transaction happened to carry.
    ///
    /// The `date <= end` filter runs *before* `sorted_transactions`, not after, because
    /// the SPLIT branch rescales every lot in scope: a split dated after `end` would
    /// move the cost basis of a window that closed before it happened.
    ///
    /// `too_many_arguments` is silenced for the same reason as `calculate_lots`: the
    /// tenth is the `SqlitePool` threaded through for `get_rate`.
    #[allow(clippy::too_many_arguments)]
    pub async fn replay_at(
        transactions: &[Transaction],
        end: NaiveDate,
        valuation_date: NaiveDate,
        current_price: Decimal,
        asset_currency: &str,
        base_currency: &str,
        currency_service: &CurrencyService,
        strategy: &str,
        hybrid_threshold_days: i64,
        pool: &SqlitePool,
    ) -> Result<DatedReplay> {
        let in_window: Vec<Transaction> = transactions
            .iter()
            .filter(|tx| tx.date.date_naive() <= end)
            .cloned()
            .collect();
        let sorted_txs = Self::sorted_transactions(&in_window);

        // An asset with nothing dated at or before `end` has no lots and needs no rate:
        // returning here is what keeps a position bought after the window from costing
        // an outbound FX call.
        if sorted_txs.is_empty() {
            return Ok(DatedReplay {
                open_lots: Vec::new(),
                sell_breakdowns: Vec::new(),
                current_price_base: Decimal::ZERO,
                fx_rate_date_used: None,
            });
        }

        let mut fx_rate_date_used = None;
        // Case-insensitive on purpose, to match `get_rate`'s own identity rule
        // (`currency_service.rs:401-403`): an asset stored as "usd" in a "USD" portfolio
        // needs no conversion, and `get_rate` short-circuits it to rate 1 with
        // `date_used = valuation_date`. A plain `!=` here would take the branch and then
        // report `fx_rate_date_used = Some(valuation_date)` — "a conversion happened on
        // this date" when none did. Money is unaffected either way; only the provenance
        // cell would lie. `replay` (:621) and `calculate_lots` (:778) still use a plain
        // `!=`; neither exposes provenance, so the difference is invisible today and is
        // tracked rather than changed here, since :778 is the live undated path.
        let current_price_base = if !asset_currency.eq_ignore_ascii_case(base_currency) {
            // The market-value leg marks the position at the close the caller quoted for
            // `valuation_date`, so it is converted at that date's rate. The realized legs
            // in `replay` are each converted at their own transaction date, which is the
            // correct rule for a historical figure and why the two legs cannot share one
            // lookup.
            let at = currency_service
                .get_rate(
                    asset_currency,
                    base_currency,
                    valuation_date.and_hms_opt(0, 0, 0).unwrap().and_utc(),
                    pool,
                )
                .await?;
            fx_rate_date_used = Some(at.date_used);
            current_price * at.rate
        } else {
            current_price
        };

        let (buy_lots, sell_breakdowns) = Self::replay(
            &sorted_txs,
            pool,
            currency_service,
            asset_currency,
            base_currency,
            strategy,
            hybrid_threshold_days,
        )
        .await?;

        let mut open_lots = Vec::new();
        for lot in buy_lots {
            if lot.qty <= Decimal::ZERO {
                continue;
            }
            let lot_cost = lot.qty * lot.unit_cost;
            let lot_market_value = lot.qty * current_price_base;
            let unrealized_gain_loss = lot_market_value - lot_cost;
            let unrealized_roi = if lot_cost > Decimal::ZERO {
                Some(unrealized_gain_loss / lot_cost)
            } else {
                None
            };
            open_lots.push(DatedLot {
                buy_transaction_id: lot.source_tx_id,
                buy_date: lot.date,
                buy_price: lot.price,
                original_qty: lot.original_qty,
                remaining_qty: lot.qty,
                unit_cost: lot.unit_cost,
                market_value: lot_market_value,
                unrealized_gain_loss,
                unrealized_roi,
            });
        }

        Ok(DatedReplay { open_lots, sell_breakdowns, current_price_base, fx_rate_date_used })
    }

    /// Realized-P&L breakdown for one sell. Replays the whole asset history
    /// rather than a prefix: a sell's matches depend only on transactions at or
    /// before it, so this is identical to a prefix replay and is the same call
    /// `calculate_lots` makes. Takes no current price — realized P&L needs none.
    ///
    /// `too_many_arguments` is silenced for the same reason as `calculate_lots`: the
    /// eighth argument is the `SqlitePool` threaded through for `get_rate`.
    #[allow(clippy::too_many_arguments)]
    pub async fn sell_breakdown(
        sell_transaction_id: i32,
        transactions: &[Transaction],
        asset_currency: &str,
        base_currency: &str,
        currency_service: &CurrencyService,
        strategy: &str,
        hybrid_threshold_days: i64,
        pool: &SqlitePool,
    ) -> Result<Option<SellBreakdown>> {
        let sorted_txs = Self::sorted_transactions(transactions);
        let (_, breakdowns) = Self::replay(
            &sorted_txs,
            pool,
            currency_service,
            asset_currency,
            base_currency,
            strategy,
            hybrid_threshold_days,
        )
        .await?;
        Ok(breakdowns.into_iter().find(|b| b.sell_transaction_id == sell_transaction_id))
    }
}

/// Short/long-term bucketing exists only for HYBRID, where `threshold_days` is
/// a real strategy parameter. Reporting a term under FIFO or LIFO would present
/// a HYBRID tuning knob as a tax fact, so it stays null there.
fn term_for(strategy: &str, holding_days: i64, threshold_days: i64) -> Option<String> {
    if strategy.to_uppercase() != "HYBRID" {
        return None;
    }
    Some(if holding_days <= threshold_days { "SHORT".to_string() } else { "LONG".to_string() })
}
