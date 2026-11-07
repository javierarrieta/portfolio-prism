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

#[test]
fn the_real_tree_passes_the_gate() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let (code, text) = run(&repo);
    assert_eq!(code, 0, "gate failed on the repository:\n{text}");
    assert!(
        text.contains("handlers inspected: 15"),
        "the gate should account for all 15 handlers, said:\n{text}"
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
