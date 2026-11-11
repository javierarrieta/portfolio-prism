//! The CSV the owner archives: its columns, its totals and its bytes.

mod common;

use common::{build_rocket, disabled_config, seed_split_sync, setup_db, DEFAULT_USER};
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;
use rust_decimal::Decimal;
use sqlx::sqlite::SqlitePool;
use std::str::FromStr;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// The CSV as `(header, rows)`, split the plain way: no fixture in this file contains a
/// comma inside a cell except the one that deliberately does.
fn parse(body: &str) -> (Vec<&str>, Vec<Vec<&str>>) {
    let lines: Vec<&str> = body.trim_end_matches("\r\n").split("\r\n").collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    let rows = lines[1..].iter().map(|l| l.split(',').collect()).collect();
    (header, rows)
}

fn cell<'a>(header: &[&str], row: &'a [&'a str], column: &str) -> &'a str {
    let i = header.iter().position(|c| *c == column).unwrap_or_else(|| panic!("no {column} column"));
    row[i]
}

/// One portfolio whose name and symbol both start with a formula character, one sell that
/// leaves an unmatched tail, and one lot still open: every row kind and both cell hazards
/// in one request.
///
/// *** DEVIATION FROM THE BRIEF, RECORDED *** The brief seeded one buy of 60 and one sell
/// of 80. That single match makes `realized_pnl` and `lot_pnl` numerically identical, so
/// `the_lot_pnl_column_sums_to_the_assets_realized_in_window`'s `assert_ne!` — the test
/// that exists to catch sourcing `lot_pnl` from the sell total — cannot hold; it is the
/// same single-match trap that already hid a real money bug on this branch. That fixture
/// also consumes every bought unit, so no `OPEN_LOT` row is produced and
/// `every_row_kind_reaches_the_file_and_nulls_are_empty_cells` cannot see all three kinds.
/// Two buys, a sell that drains both and over-sells, and a later buy fix both while keeping
/// the brief's anchors: an unmatched tail of exactly 20.0 and `realized_in_window` 1800
/// (60 x (130-100) + 60 x (130-130)).
async fn csv_fixture(pool: &SqlitePool) -> i32 {
    let port_id = common::seed_portfolio(pool, DEFAULT_USER, "=cmd|'/c calc'!A1", "USD").await;
    let asset_id = common::seed_asset(pool, port_id, "-SYM", "Minus symbol").await;
    common::seed_transaction_at(pool, asset_id, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    common::seed_transaction_at(pool, asset_id, "BUY", 60.0, 130.0, 0.0, "2024-03-01T00:00:00Z").await;
    common::seed_transaction_at(pool, asset_id, "SELL", 140.0, 130.0, 0.0, "2024-05-01T00:00:00Z").await;
    common::seed_transaction_at(pool, asset_id, "BUY", 30.0, 140.0, 0.0, "2024-06-01T00:00:00Z").await;
    common::seed_daily_prices(pool, "-SYM", "2024-01-01", "2024-06-28", "110.0").await;
    seed_split_sync(pool, "-SYM").await;
    port_id
}

/// A client and the CSV body for `query`, over a freshly seeded fixture. The portfolio id
/// comes back from the seeding helper rather than being assumed.
async fn csv_body(query: &str) -> String {
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?{query}&format=csv"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    resp.into_string().await.unwrap()
}

#[tokio::test]
async fn the_file_is_csv_with_the_servers_own_filename() {
    let body = csv_body("strategy=FIFO&threshold_days=30&from=2024-01-01&to=2024-06-30").await;
    // The body assertion is here so the request is real; the headers are asserted on a
    // second dispatch, because `into_string` consumes the response.
    assert!(body.starts_with("row_type,"), "row 1 is the header row: {body:.80}");
}

#[tokio::test]
async fn the_headers_are_the_columns_order_and_no_comment_row_precedes_them() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?from=2024-01-01&to=2024-06-30&format=csv"))
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    assert_eq!(resp.content_type(), Some(ContentType::new("text", "csv")));
    let disposition = resp.headers().get_one("Content-Disposition").unwrap_or("").to_string();
    assert_eq!(disposition, format!("attachment; filename=\"tax-report-{port_id}-2024-01-01-2024-06-30.csv\""));

    let body = resp.into_string().await.unwrap();
    let (header, _) = parse(&body);
    assert_eq!(header, backend_rust::report_csv::CSV_COLUMNS.to_vec());
    assert!(!body.starts_with('#'), "a leading `#` row maps every column to null on import");
}

#[tokio::test]
async fn the_lot_pnl_column_sums_to_the_assets_realized_in_window() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let json: serde_json::Value = serde_json::from_str(
        &client
            .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&from=2024-01-01&to=2024-06-30"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap(),
    )
    .unwrap();
    let body = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&from=2024-01-01&to=2024-06-30&format=csv"))
        .dispatch()
        .await
        .into_string()
        .await
        .unwrap();

    let (header, rows) = parse(&body);
    let sold: Decimal = rows.iter()
        .filter(|r| cell(&header, r, "row_type") == "SOLD_LOT")
        .map(|r| dec(cell(&header, r, "lot_pnl")))
        .sum();
    // The JSON totals are the authority; the file has to agree with them, in Decimals and
    // not in serialized strings.
    assert_eq!(sold, dec(json["asset_totals"][0]["realized_in_window"].as_str().unwrap()));
    assert_eq!(sold, dec("1800.0"));

    // And the columns that must NOT be summed are the ones the release note names.
    let repeated: Decimal = rows.iter()
        .filter(|r| cell(&header, r, "row_type") == "SOLD_LOT")
        .map(|r| dec(cell(&header, r, "realized_pnl")))
        .sum();
    assert_ne!(repeated, sold, "sell-level columns repeat per lot row; summing them double-counts");
}

#[tokio::test]
async fn every_row_kind_reaches_the_file_and_nulls_are_empty_cells() {
    let body = csv_body("strategy=FIFO&from=2024-01-01&to=2024-06-30").await;
    let (header, rows) = parse(&body);

    let kinds: Vec<&str> = rows.iter().map(|r| cell(&header, r, "row_type")).collect();
    assert!(kinds.contains(&"SOLD_LOT"), "{kinds:?}");
    assert!(kinds.contains(&"OPEN_LOT"), "{kinds:?}");
    assert!(kinds.contains(&"UNMATCHED"), "{kinds:?}");

    let unmatched = rows.iter().find(|r| cell(&header, r, "row_type") == "UNMATCHED").unwrap();
    assert_eq!(cell(&header, unmatched, "unmatched_qty"), "20.0");
    assert_eq!(cell(&header, unmatched, "lot_pnl"), "", "nulls are empty cells, not `0` or `n/a`");
    assert_eq!(cell(&header, unmatched, "market_value"), "");

    // A comma-free fixture, so the only way to see quoting is this cell.
    assert!(
        body.contains("\"'=cmd|'/c calc'!A1\""),
        "a free-text cell with a formula lead is quoted and apostrophe-guarded",
    );
    assert!(body.contains("'-SYM"), "and so is a symbol");
    assert!(!body.contains("e-"), "money is written verbatim and never in exponent notation");
}

#[tokio::test]
async fn two_dispatches_over_unchanged_data_are_byte_identical() {
    // Goal 4, and the reason the file carries no generated-at column: a timestamp would
    // make this unreachable and would make every regenerated file a different file.
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let path = format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&from=2024-01-01&to=2024-06-30&format=csv");

    let first = client.get(&path).dispatch().await.into_string().await.unwrap();
    let second = client.get(&path).dispatch().await.into_string().await.unwrap();
    assert_eq!(first, second);
}

#[tokio::test]
async fn format_csv_without_a_window_is_a_422_naming_format() {
    // The legacy summary shape has no rows to write, and answering with "everything,
    // priced at the latest close" would quietly answer a different question than the file
    // claims to answer.
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?format=csv"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::UnprocessableEntity);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert!(body["error"].as_str().unwrap().contains("format"));

    let unknown = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?from=2024-01-01&format=json"))
        .dispatch()
        .await;
    assert_eq!(unknown.status(), Status::UnprocessableEntity);
}
