//! The gate that enforces the gate.
//!
//! `scripts/check_access_gate.sh` is the mechanism that makes "every handler asks
//! who the caller is" true for code nobody has read yet. Until this file it had no
//! test at all: a regression in the script would have been indistinguishable from a
//! clean repository, which is the worst possible failure mode for a lint — it fails
//! by reporting success.
//!
//! Two directions are pinned. The real tree must pass, or the gate is noise. And a
//! tree that *should* fail must fail, or the gate is decoration. The second half
//! builds a throwaway source tree in a temporary directory and runs the same script
//! against it; the script resolves its own directory, so copying it next to a fake
//! `src/api_routes` is enough to redirect it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/check_access_gate.sh");

/// A tree whose handlers each commit one of the sins the rules exist to catch.
///
/// Three distinct defects, because each was a real hole in an earlier version of the
/// script and they fail for unrelated reasons:
/// * `late_guard` calls `require` **after** the id query — the old rule checked
///   presence, not order, and a guard that runs after the read guards nothing;
/// * `comment_only` names `AuthedUser` in a comment — the old rule matched the whole
///   body, so prose satisfied it;
/// * `sneaky` is an `OPTIONS` handler — the old attribute pattern did not include
///   `options`, so the handler was invisible to all three rules.
const BAD_HANDLER: &str = r#"use crate::access::require;
use crate::auth::guard::AuthedUser;

#[get("/late")]
pub async fn late_guard(user: AuthedUser, pool: &State<SqlitePool>) -> Json<Value> {
    let row = sqlx::query("SELECT * FROM portfolios WHERE id = ?").bind(1);
    require(&pool, user.id, 1, Access::Owner).await;
    Json(json!({}))
}

#[post("/comment")]
pub async fn comment_only(pool: &State<SqlitePool>) -> Json<Value> {
    // AuthedUser mentioned in a comment must not satisfy Rule C
    let row = sqlx::query("SELECT * FROM assets WHERE id = ?").bind(1);
    Json(json!({}))
}

#[options("/sneaky")]
pub async fn sneaky(pool: &State<SqlitePool>) -> Json<Value> {
    let row = sqlx::query("SELECT * FROM transactions WHERE id = ?").bind(1);
    Json(json!({}))
}
"#;

fn run(dir: &Path) -> (i32, String) {
    let out = Command::new("bash")
        .arg(dir.join("scripts/check_access_gate.sh"))
        .output()
        .expect("the gate script runs");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

/// Build a throwaway tree holding one handler file, run the real script against it,
/// and report what it said. `label` keeps parallel tests out of each other's
/// directory.
fn run_tree(label: &str, handler: &str) -> (i32, String) {
    let root = std::env::temp_dir().join(format!("access-gate-{}-{label}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("src/api_routes")).unwrap();
    fs::copy(SCRIPT, root.join("scripts/check_access_gate.sh")).unwrap();
    fs::write(root.join("src/api_routes/backup.rs"), handler).unwrap();

    let out = run(&root);
    let _ = fs::remove_dir_all(&root);
    out
}

#[test]
fn the_real_tree_passes_the_gate() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let (code, text) = run(&repo);
    assert_eq!(code, 0, "gate failed on the repository:\n{text}");
    assert!(
        text.contains("handlers inspected: 18"),
        "the gate should account for all 18 handlers, said:\n{text}"
    );
}

#[test]
fn the_gate_rejects_a_handler_that_checks_after_reading() {
    let root = std::env::temp_dir().join(format!("access-gate-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("src/api_routes")).unwrap();
    fs::copy(SCRIPT, root.join("scripts/check_access_gate.sh")).unwrap();
    fs::write(root.join("src/api_routes/bad.rs"), BAD_HANDLER).unwrap();

    let (code, text) = run(&root);
    let _ = fs::remove_dir_all(&root);

    assert_eq!(code, 1, "a tree this bad must not pass the gate:\n{text}");
    assert!(
        text.contains("LATE-GUARD"),
        "order violation missed:\n{text}"
    );
    assert!(
        text.contains("NO-CALLER"),
        "a comment naming AuthedUser satisfied Rule C:\n{text}"
    );
    assert!(
        text.contains("sneaky"),
        "an OPTIONS handler was invisible to the gate:\n{text}"
    );
    // Line numbers come from the file, not from a counter that only advances inside
    // the first handler; a wrong number sends someone to the wrong place.
    assert!(
        text.contains("bad.rs:4") && text.contains("bad.rs:11") && text.contains("bad.rs:18"),
        "reported line numbers do not match the file:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Rule A's per-handler marker.
//
// Rule A's invariant is per handler — a handler that resolves a caller-supplied id
// goes through `access` — so the waiver has to be per handler too. These cases are
// the difference between a marker and a bypass.
// ---------------------------------------------------------------------------

/// One handler that has nothing to send to the chokepoint, marked, and one that
/// resolves an id and is not marked. If the marker waived the *file* rather than
/// the handler it names, this tree would pass — which is the only reason it exists.
const PARTIALLY_MARKED: &str = r#"use crate::auth::guard::AuthedUser;

// access-gate: global-config (no caller-supplied id)
#[get("/capabilities")]
pub async fn capabilities(user: AuthedUser) -> Json<Value> {
    Json(json!({}))
}

#[get("/by-id/<id>")]
pub async fn by_id(user: AuthedUser, pool: &State<SqlitePool>, id: i32) -> Json<Value> {
    let row = sqlx::query("SELECT * FROM portfolios WHERE id = ?").bind(id);
    Json(json!({}))
}
"#;

/// A file whose only handler carries the marker: the shape `api_routes/backup.rs`
/// has in the real tree, pinned here so the real tree is not the only proof.
const ONLY_MARKED: &str = r#"use crate::auth::guard::AuthedUser;

// access-gate: global-config (no caller-supplied id)
#[get("/capabilities")]
pub async fn capabilities(user: AuthedUser) -> Json<Value> {
    Json(json!({}))
}
"#;

/// The same marker with a blank line between it and the attribute. A waiver you can
/// park anywhere in a file is a waiver you can hide.
const MISPLACED_MARKER: &str = r#"use crate::auth::guard::AuthedUser;

// access-gate: global-config (no caller-supplied id)

#[get("/capabilities")]
pub async fn capabilities(user: AuthedUser) -> Json<Value> {
    Json(json!({}))
}
"#;

/// The copy-paste this gate exists to catch: a handler whose route takes an id and
/// whose marker claims it does not. Rule B cannot see it — the query is
/// `WHERE portfolio_id`, not `WHERE id`, which the script admits as a blind spot —
/// so the marker cross-check is the only thing standing here.
const CONTRADICTORY_MARKER: &str = r#"use crate::auth::guard::AuthedUser;

// access-gate: global-config (no caller-supplied id)
#[get("/export/<portfolio_id>")]
pub async fn export(user: AuthedUser, pool: &State<SqlitePool>, portfolio_id: i32) -> Json<Value> {
    let row = sqlx::query("SELECT * FROM portfolios WHERE portfolio_id = ?").bind(portfolio_id);
    Json(json!({}))
}
"#;

/// The same defect with a parameter nobody would think to call `id`. The marker's
/// claim is "no caller-supplied id", not "no id-named id", so a check keyed on the
/// spelling `id` is a check on one spelling of the bypass rather than on the bypass.
const ID_LESS_PARAMETER: &str = r#"use crate::auth::guard::AuthedUser;

// access-gate: global-config (no caller-supplied id)
#[get("/backup/export/<portfolio>")]
pub async fn export(user: AuthedUser, pool: &State<SqlitePool>, portfolio: i32) -> Json<Value> {
    let row = sqlx::query("SELECT * FROM portfolios WHERE portfolio_id = ?").bind(portfolio);
    Json(json!({}))
}
"#;

/// The same route attribute split across lines, which is what `rustfmt` does to a
/// long one. Testing only the line that opened the attribute would see `#[get(` and
/// no parameter at all.
const WRAPPED_ATTRIBUTE: &str = r#"use crate::auth::guard::AuthedUser;

// access-gate: global-config (no caller-supplied id)
#[get(
    "/export/<id>"
)]
pub async fn export(user: AuthedUser, pool: &State<SqlitePool>, id: i32) -> Json<Value> {
    let row = sqlx::query("SELECT * FROM portfolios WHERE id = ?").bind(id);
    Json(json!({}))
}
"#;

#[test]
fn the_marker_waives_the_handler_it_names_and_no_other() {
    let (code, text) = run_tree("marked", PARTIALLY_MARKED);
    assert!(
        text.contains("waived") && text.contains("capabilities"),
        "the marked handler was not reported as waived:\n{text}"
    );
    assert!(
        text.contains("NO-CHOKEPOINT") && text.contains("by_id"),
        "an unmarked handler slipped through Rule A because a neighbour was marked:\n{text}"
    );
    assert_eq!(
        code, 1,
        "the unmarked id-resolving handler must still fail the gate:\n{text}"
    );
}

#[test]
fn a_handler_that_resolves_no_id_passes_with_the_marker() {
    let (code, text) = run_tree("only-marked", ONLY_MARKED);
    assert_eq!(
        code, 0,
        "a marked handler with no caller-supplied id must pass:\n{text}"
    );
    assert!(
        text.contains("waived") && text.contains("capabilities"),
        "the waiver was honoured silently; the gate must print what it waived:\n{text}"
    );
}

#[test]
fn a_marker_in_the_wrong_place_waives_nothing() {
    let (code, text) = run_tree("misplaced", MISPLACED_MARKER);
    assert!(
        !text.contains("waived"),
        "a marker that is not the line above the attribute was honoured:\n{text}"
    );
    assert_eq!(code, 1, "a misplaced marker must fail, not pass quietly:\n{text}");
}

#[test]
fn a_marker_on_a_route_that_takes_an_id_is_refused() {
    // The marker's whole claim is "no caller-supplied id". A route attribute naming
    // one contradicts it, and honouring the marker here would open Rule A for
    // exactly the handler that needs it — Tasks 4-6 land in this same file.
    let (code, text) = run_tree("contradicts", CONTRADICTORY_MARKER);
    assert!(
        text.contains("MARKER-CONTRADICTS") && text.contains("export"),
        "a marker claiming no caller-supplied id was honoured on a route that has one:\n{text}"
    );
    assert!(
        !text.contains("waived"),
        "the contradictory marker was honoured anyway:\n{text}"
    );
    assert_eq!(code, 1, "a false marker must fail the gate:\n{text}");
}

#[test]
fn a_marker_is_refused_whatever_the_parameter_is_called() {
    // `<portfolio>`, `<key>`, `<target>`: caller-supplied just as much as `<id>`, and
    // Rule B cannot see the query either (`WHERE portfolio_id` is its blind spot),
    // so a name-keyed cross-check would leave nothing standing at all.
    let (code, text) = run_tree("id-less-param", ID_LESS_PARAMETER);
    assert!(
        text.contains("MARKER-CONTRADICTS") && text.contains("export"),
        "a marker was honoured on a route whose parameter name lacks the letters id:\n{text}"
    );
    assert!(
        !text.contains("waived"),
        "the contradictory marker was honoured anyway:\n{text}"
    );
    assert_eq!(code, 1, "a parameter name must not be a way past the gate:\n{text}");
}

#[test]
fn a_marker_is_refused_on_a_wrapped_route_attribute() {
    // The parameter is on the line after `#[get(`, where rustfmt puts it once the
    // path is long. Reading only the opening line sees no parameter.
    let (code, text) = run_tree("wrapped", WRAPPED_ATTRIBUTE);
    assert!(
        text.contains("MARKER-CONTRADICTS") && text.contains("export"),
        "a parameter on a continuation line of the attribute was not seen:\n{text}"
    );
    assert_eq!(code, 1, "a wrapped attribute must not hide a contradiction:\n{text}");
}
