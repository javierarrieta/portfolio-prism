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

/// The UTF-8 BOM, which the server writes and the app's encoding spec requires (Excel picks
/// its codepage from it). Pinned here rather than assumed: a dropped BOM mojibake-s every
/// non-ASCII `portfolio_name` and `symbol` on a Windows install, and nothing else would notice.
fn strip_bom(body: &str) -> &str {
    body.strip_prefix('\u{FEFF}')
        .unwrap_or_else(|| panic!("file must open with the UTF-8 BOM (EF BB BF), got: {body:.40}"))
}

/// The CSV as `(header, rows)`, split the plain way: no fixture in this file contains a
/// comma inside a cell except the one that deliberately does.
fn parse(body: &str) -> (Vec<&str>, Vec<Vec<&str>>) {
    let body = strip_bom(body);
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

/// The claim this whole feature rests on is that the CSV is a **projection** of the same
/// payload the JSON serves -- no number re-derived. That has to be true of the dates too, and
/// it was not: `to_rfc3339()` renders UTC as `+00:00` while serde renders the same instant as
/// `Z`. Same instant, different text, so a preparer reconciling the two surfaces saw two
/// different strings for one field. Pinned here because nothing else would notice.
#[tokio::test]
async fn a_date_cell_reads_the_same_text_as_the_json_field_it_projects() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let q = "strategy=FIFO&threshold_days=30&from=2024-01-01&to=2024-06-30";

    let json: serde_json::Value = serde_json::from_str(
        &client
            .get(format!("/api/portfolios/{port_id}/tax-summary?{q}"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap(),
    )
    .unwrap();
    let csv = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?{q}&format=csv"))
        .dispatch()
        .await
        .into_string()
        .await
        .unwrap();

    let mut lines = csv.trim_end().split("\r\n");
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    let idx = header
        .iter()
        .position(|c| *c == "sell_date")
        .expect("sell_date column");
    let first = lines.next().unwrap();
    let cell = first.split(',').nth(idx).unwrap().trim_matches('"');

    let json_date = json["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["sell_date"].is_string())
        .expect("a row with a sell_date")["sell_date"]
        .as_str()
        .unwrap();
    assert_eq!(cell, json_date, "the CSV cell must read exactly what the JSON field reads");
    assert!(json_date.ends_with('Z'), "and the JSON form is the Z one: {json_date}");
}

#[tokio::test]
async fn the_file_is_csv_with_the_servers_own_filename() {
    let body = csv_body("strategy=FIFO&threshold_days=30&from=2024-01-01&to=2024-06-30").await;
    // The body assertion is here so the request is real; the headers are asserted on a
    // second dispatch, because `into_string` consumes the response.
    assert!(
        strip_bom(&body).starts_with("row_type,"),
        "row 1 is the header row: {body:.80}"
    );
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
async fn a_to_only_filename_names_the_window_applied_not_the_parameters_typed() {
    // Two documents claimed an absent `from` renders as the literal `inception`. It does not:
    // `analytics.rs` uses `window.from.or(first_in_scope)`, so the filename carries the
    // portfolio's first transaction on or before `to`. `inception` is the no-transactions
    // case only. This pins the common branch; the next test pins the rare one.
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;   // first transaction is 2024-01-01
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?to=2024-06-30&format=csv"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    assert_eq!(
        resp.headers().get_one("Content-Disposition").unwrap_or(""),
        format!("attachment; filename=\"tax-report-{port_id}-2024-01-01-2024-06-30.csv\""),
        "an absent `from` must name the first transaction in scope, not `inception`",
    );
}

#[tokio::test]
async fn a_window_with_no_transaction_in_scope_is_the_only_inception_filename() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;   // nothing on or before 2023-12-31
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?to=2023-12-31&format=csv"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    assert_eq!(
        resp.headers().get_one("Content-Disposition").unwrap_or(""),
        format!("attachment; filename=\"tax-report-{port_id}-inception-2023-12-31.csv\""),
    );
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
    // (An earlier version of this test asserted `!body.contains("e-")` as an f64 guard. It
    // could not fail: no value in any fixture is small or large enough to force exponent
    // notation, and neither would a plausible f64 regression produce one. Removed rather than
    // kept as decoration -- the real guarantee is `Decimal::to_string` in `decimal_cell` plus
    // the wire-form pins in `tax_report_test.rs`.)
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
    // Determinism alone is not evidence of CSV-ness: with the `format=csv` arm deleted, both
    // dispatches would return the same JSON and this test would still pass. Pin what was
    // actually fetched.
    assert!(
        strip_bom(&first).starts_with("row_type,"),
        "this test must be comparing CSV, not two identical JSON bodies"
    );
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
        .get(format!("/api/portfolios/{port_id}/tax-summary?from=2024-01-01&to=2024-06-30&format=pdf"))
        .dispatch()
        .await;
    assert_eq!(unknown.status(), Status::UnprocessableEntity);

    // `json` is what the route already returns, so naming it is a no-op, not a 422. It used
    // to be a 422 -- pinned here -- which meant a client that always appends `format=json`
    // was refused for asking for the default.
    // Bounded on both ends: `from` alone runs through today, and today is past the seeded
    // series, so an open-ended window here would try the provider and answer 503.
    let named_default = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?from=2024-01-01&to=2024-06-30&format=json"))
        .dispatch()
        .await;
    assert_eq!(named_default.status(), Status::Ok);
    assert_eq!(named_default.content_type(), Some(ContentType::JSON));

    let upper = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?from=2024-01-01&to=2024-06-30&format=CSV"))
        .dispatch()
        .await;
    assert_eq!(upper.status(), Status::Ok, "the two format names are matched case-insensitively");
}

/// Task 5 review Important #1. The controller's first date fix used
/// `SecondsFormat::Secs`, which matches serde only for whole-second timestamps. chrono's
/// serde impl uses `AutoSi` (sub-second digits when present), and `POST /transactions`
/// accepts fractional seconds, so a stored `...T12:00:00.123456Z` rendered as
/// `...T12:00:00Z` in the CSV -- the projection claim, false again, with precision dropped.
/// Measured before fixing:
///   serde (AutoSi) = "2024-12-01T12:00:00.123456Z"
///   Secs, true     =  2024-12-01T12:00:00Z
///   AutoSi, true   =  2024-12-01T12:00:00.123456Z
/// The first assertion is the anti-vacuity guard: if the storage layer dropped the fraction
/// on the way in, this test would be comparing two truncated strings and proving nothing.
#[tokio::test]
async fn a_sub_second_transaction_date_still_projects_exactly() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = common::seed_portfolio(&pool, DEFAULT_USER, "Sub second", "USD").await;
    let asset_id = common::seed_asset(&pool, port_id, "SUBSEC", "sub").await;
    common::seed_transaction_at(&pool, asset_id, "BUY", 10.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    common::seed_transaction_at(&pool, asset_id, "SELL", 4.0, 130.0, 0.0, "2024-05-01T00:00:00.123456Z").await;
    common::seed_daily_prices(&pool, "SUBSEC", "2024-01-01", "2024-06-28", "110.0").await;
    seed_split_sync(&pool, "SUBSEC").await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let q = "strategy=FIFO&threshold_days=30&from=2024-01-01&to=2024-06-30";

    let json: serde_json::Value = serde_json::from_str(
        &client
            .get(format!("/api/portfolios/{port_id}/tax-summary?{q}"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap(),
    )
    .unwrap();
    let sell_date = json["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["sell_date"].is_string())
        .unwrap()["sell_date"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        sell_date.contains(".123456"),
        "the fixture must actually round-trip sub-second precision or this proves nothing: {sell_date}"
    );

    let csv = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?{q}&format=csv"))
        .dispatch()
        .await
        .into_string()
        .await
        .unwrap();
    assert!(
        csv.contains(&sell_date),
        "the CSV must carry the same text the JSON carries, fraction and all (JSON: {sell_date})"
    );
}

/// Task 5 review Important #2: the whole RFC 4180 quoting path was untested -- the fixture
/// is comma-free by design, so `text_cell`'s `contains([',', '"', '\n', '\r'])` and its
/// `replace('"', "\"\"")` were invisible to the suite. Dropping any one of them would have
/// left every test green.
///
/// *** One trigger per asset. *** An earlier version of this test used a single symbol
/// `A,B"C` and asserted the fully-quoted form -- and it PASSED with the comma removed from
/// the trigger list, because the embedded `"` forced quoting on its own. A test that passes
/// when the thing it exists to guard is deleted is worse than no test, so each trigger gets
/// its own asset here and nothing overlaps.
#[tokio::test]
async fn each_rfc4180_trigger_forces_quoting_on_its_own() {
    // (symbol, the quoted form that must appear, which trigger it isolates)
    let cases = [
        ("X,Y", "\"X,Y\"", "embedded comma"),
        ("Q\"Z", "\"Q\"\"Z\"", "embedded double quote, doubled"),
    ];
    for (symbol, expected, trigger) in cases {
        let pool = setup_db(&disabled_config()).await;
        let port_id = common::seed_portfolio(&pool, DEFAULT_USER, "Escaping", "USD").await;
        let asset_id = common::seed_asset(&pool, port_id, symbol, "nasty").await;
        common::seed_transaction_at(&pool, asset_id, "BUY", 10.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
        common::seed_transaction_at(&pool, asset_id, "SELL", 4.0, 130.0, 0.0, "2024-05-01T00:00:00Z").await;
        common::seed_daily_prices(&pool, symbol, "2024-01-01", "2024-06-28", "110.0").await;
        seed_split_sync(&pool, symbol).await;
        let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

        let csv = client
            .get(format!(
                "/api/portfolios/{port_id}/tax-summary?strategy=FIFO&threshold_days=30&from=2024-01-01&to=2024-06-30&format=csv"
            ))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap();

        assert!(
            csv.contains(expected),
            "{trigger}: expected {expected} in the file, got: {csv}"
        );
        let arity = backend_rust::report_csv::CSV_COLUMNS.len();
        for (i, line) in csv.trim_end().split("\r\n").enumerate() {
            assert!(
                line.split(',').count() >= arity,
                "{trigger}: row {i} has fewer than {arity} cells -- the {trigger} escaped its quoting"
            );
        }
    }
}

/// The header-only decision (brief correction 5), previously unpinned. A window in which
/// every in-scope asset produces no rows yields a file with a header and nothing else, while
/// the JSON still carries the asset's cumulative figures. Accepted deliberately: such an
/// asset's *window* figures are all genuinely zero, and the only number the file drops is
/// `realized_cumulative`, which is not a window figure. Pinned so the shape is a decision
/// rather than an accident someone "fixes" by inventing a fourth row kind.
#[tokio::test]
async fn a_window_with_no_rows_is_a_header_and_nothing_else() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = common::seed_portfolio(&pool, DEFAULT_USER, "Quiet window", "USD").await;
    let asset_id = common::seed_asset(&pool, port_id, "QUIET", "quiet").await;
    common::seed_transaction_at(&pool, asset_id, "BUY", 10.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    // Fully sold before the window opens: no sells inside it, and nothing left open.
    common::seed_transaction_at(&pool, asset_id, "SELL", 10.0, 130.0, 0.0, "2024-02-01T00:00:00Z").await;
    common::seed_daily_prices(&pool, "QUIET", "2024-01-01", "2024-06-28", "110.0").await;
    seed_split_sync(&pool, "QUIET").await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let q = "strategy=FIFO&threshold_days=30&from=2024-03-01&to=2024-06-28";

    let csv = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?{q}&format=csv"))
        .dispatch()
        .await
        .into_string()
        .await
        .unwrap();
    let body = strip_bom(&csv);
    let lines: Vec<&str> = body.trim_end_matches("\r\n").split("\r\n").collect();
    assert_eq!(lines.len(), 1, "header only, no data rows: {body}");
    assert!(lines[0].starts_with("row_type,"));

    // And the JSON really does carry the cumulative figure the file omits -- otherwise this
    // test would be pinning an empty case against an empty payload, which proves nothing.
    let json: serde_json::Value = serde_json::from_str(
        &client
            .get(format!("/api/portfolios/{port_id}/tax-summary?{q}"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json["rows"].as_array().unwrap().len(), 0);
    let totals = &json["asset_totals"][0];
    assert_eq!(totals["realized_in_window"], "0", "the window figure is a true zero");
    assert_ne!(
        totals["realized_cumulative"], "0",
        "the omitted figure must actually be non-zero or the omission is vacuous"
    );
}
