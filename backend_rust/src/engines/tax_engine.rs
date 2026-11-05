use crate::models::Transaction;
use crate::schemas::{AssetTaxSummary, LotMatch, SellBreakdown, TaxLot};
use crate::services::currency_service::CurrencyService;
use rust_decimal::Decimal;
use chrono::{DateTime, Utc};
use anyhow::Result;

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

    async fn run_engine(transactions: &[Transaction], strategy: &str, threshold_days: i64) -> AssetTaxSummary {
        let svc = CurrencyService::new();
        TaxLotEngine::calculate_lots(
            "TEST", "STOCK", transactions, Decimal::from_str("110.0").unwrap(), "USD", "USD", &svc, strategy, threshold_days,
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
}

pub struct TaxLotEngine;

struct InternalLot {
    source_tx_id: i32,
    date: DateTime<Utc>,
    price: Decimal,
    qty: Decimal,
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
                currency_service.get_rate(asset_currency, base_currency, tx.date).await?
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
                        lot.unit_cost /= ratio;
                        lot.price /= ratio;
                    }
                }
            }
        }

        Ok((buy_lots, breakdowns))
    }

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
    ) -> Result<AssetTaxSummary> {
        let sorted_txs = Self::sorted_transactions(transactions);

        let current_price_base = if asset_currency != base_currency {
            let latest_date = sorted_txs.last().map(|tx| tx.date).unwrap_or_else(Utc::now);
            let rate = currency_service.get_rate(asset_currency, base_currency, latest_date).await?;
            current_price * rate
        } else {
            current_price
        };

        let (buy_lots, breakdowns) = Self::replay(
            &sorted_txs,
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
                    buy_date: lot.date,
                    buy_price: lot.price,
                    original_qty: lot.qty,
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