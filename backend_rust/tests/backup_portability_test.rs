//! The gate that enforces the portability discipline.
//!
//! `scripts/check_backup_portability.sh` is the only thing standing between this
//! module and a Postgres port that turns out to be a rewrite, and a grep gate is
//! exactly the kind of code that rots quietly: widen the banned list, break the
//! line join, or let the column-list test degrade into "does the line contain a
//! paren", and the script keeps printing `Portability gate passed`. Until this file
//! a loosening of the script would have been indistinguishable from a clean
//! repository — the worst failure mode for a lint, because it fails by reporting
//! success. Same reasoning as `access_gate_test.rs`.
//!
//! Every rule gets both directions. A case that must fail proves the rule bites;
//! a case that must pass proves it is not decoration that also refuses correct
//! code, which is how gates get deleted.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/scripts/check_backup_portability.sh"
);

fn run(dir: &Path) -> (i32, String) {
    let out = Command::new("bash")
        .arg(dir.join("scripts/check_backup_portability.sh"))
        .output()
        .expect("the portability gate script runs");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

/// Build a throwaway tree holding `src/backup/<label>.rs`, run the real script
/// against it, and report what it said. The script resolves its own directory, so
/// copying it next to a fake `src/backup` is enough to redirect it.
fn run_tree(label: &str, source: &str) -> (i32, String) {
    let root = std::env::temp_dir().join(format!("portability-gate-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("src/backup")).unwrap();
    fs::copy(SCRIPT, root.join("scripts/check_backup_portability.sh")).unwrap();
    fs::write(root.join("src/backup/fixture.rs"), source).unwrap();

    let out = run(&root);
    let _ = fs::remove_dir_all(&root);
    out
}

#[test]
fn the_real_tree_passes_the_gate() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let (code, text) = run(&repo);
    assert_eq!(code, 0, "gate failed on the repository:\n{text}");
    // The counts are a tripwire in the same spirit as `access_gate_test.rs`: a
    // fifth file or a fifth INSERT means someone has to look at this gate and say
    // so, rather than the script quietly inspecting less than the module holds.
    assert!(
        text.contains("Portability gate passed: 4 file(s) inspected under src/backup"),
        "the gate should account for all four files under src/backup, said:\n{text}"
    );
    assert!(
        text.contains("INSERT statements inspected: 4"),
        "the gate should account for the module's four INSERTs, said:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Rule 1 — no banned construct in SQL the module can send
// ---------------------------------------------------------------------------

/// The pair that makes the comment-line skip honest. `restore.rs` explains a real
/// schema fact with the word `PRAGMA` in prose; the rule is about statements the
/// engine receives, so that must pass. And if the same token in a SQL string also
/// passed, the first half would be proving nothing at all — which is why both
/// directions live in this file.
#[test]
fn a_banned_construct_in_a_sql_string_fails() {
    let (code, text) = run_tree(
        "pragma-in-sql",
        r#"pub async fn leak(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("PRAGMA table_info(assets)").execute(pool).await
}
"#,
    );
    assert_eq!(code, 1, "a PRAGMA in a query string must not pass:\n{text}");
    assert!(
        text.contains("'PRAGMA' has no cross-engine spelling")
            && text.contains("fixture.rs:2"),
        "the failure did not name the pattern and the line:\n{text}"
    );
}

#[test]
fn a_banned_construct_in_a_sql_string_fails_whatever_its_spelling() {
    // SQLite keywords are case-insensitive, so a gate keyed on the upper-case
    // spelling is a gate on one spelling of the bypass.
    let (code, text) = run_tree(
        "pragma-lower-case",
        r#"pub async fn leak(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("pragma foreign_keys = ON").execute(pool).await
}
"#,
    );
    assert_eq!(code, 1, "a lower-case pragma is the same statement:\n{text}");
    assert!(
        text.contains("'pragma' has no cross-engine spelling"),
        "the lower-case spelling was not matched:\n{text}"
    );
}

#[test]
fn a_banned_construct_in_prose_is_not_a_statement() {
    let (code, text) = run_tree(
        "pragma-in-comment",
        r#"// `split_sync` has no foreign key to `assets`, so `PRAGMA foreign_keys` does
// not reach it. This is the sentence in restore.rs that the skip exists for.
pub const Q: &str = "INSERT INTO assets (symbol) VALUES (?)";
"#,
    );
    assert_eq!(
        code, 0,
        "a comment naming a keyword is not SQL the module sends:\n{text}"
    );
}

#[test]
fn a_banned_construct_sharing_a_line_with_code_still_fails() {
    // The skip is per whole line, so it cannot be used as a hiding place: park the
    // token after a `//` on a code line and the gate still sees the line.
    let (code, text) = run_tree(
        "pragma-trailing-comment",
        r#"pub const Q: &str = "PRAGMA table_info(assets)"; // documented, honestly
"#,
    );
    assert_eq!(code, 1, "a trailing comment must not launder a pragma:\n{text}");
    assert!(
        text.contains("'PRAGMA' has no cross-engine spelling"),
        "the token was skipped because the line also holds a comment:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Rule 2 — every INSERT names its columns
// ---------------------------------------------------------------------------

/// The exact construct the spec bans. It is also the case the obvious
/// implementation misses: testing "does the line containing `INSERT INTO` contain
/// a `(`" passes here, because `VALUES (?, ?, ?)` supplies the paren.
#[test]
fn an_insert_without_a_column_list_fails() {
    let (code, text) = run_tree(
        "bare-values",
        r#"pub const Q: &str = "INSERT INTO assets VALUES (?, ?, ?)";
"#,
    );
    assert_eq!(code, 1, "a bare INSERT silently changes meaning:\n{text}");
    assert!(
        text.contains("fixture.rs:1 INSERT without an explicit column list"),
        "the failure did not name the file and line:\n{text}"
    );
}

/// The shape the module actually writes: SQL in a Rust string literal that wraps
/// mid-statement. A line-based rule refuses this — the `(` is on the next line —
/// and a gate that refuses correct code gets deleted.
#[test]
fn a_column_list_that_wraps_to_the_next_line_is_a_column_list() {
    let (code, text) = run_tree(
        "wrapped-column-list",
        r#"pub const Q: &str = "INSERT INTO historical_prices \
     (symbol, date, close_price) VALUES (?, ?, ?) ON CONFLICT (symbol, date) DO NOTHING";
"#,
    );
    assert_eq!(
        code, 0,
        "a wrapped column list is still a column list:\n{text}"
    );
}

#[test]
fn a_wrapped_statement_still_needs_a_column_list() {
    // The mirror image of the case above: without the join this is the one that
    // passes for the wrong reason, and with a naive join it is the one that passes
    // because a paren from somewhere else looked close enough.
    let (code, text) = run_tree(
        "wrapped-bare-values",
        r#"pub const Q: &str = "INSERT INTO historical_prices \
     VALUES (?, ?, ?)";
"#,
    );
    assert_eq!(code, 1, "a bare INSERT must fail however it wraps:\n{text}");
    assert!(
        text.contains("INSERT without an explicit column list"),
        "the wrapped bare INSERT was not reported:\n{text}"
    );
}

#[test]
fn an_insert_that_names_no_table_is_refused() {
    // The branch that fires when the text after `INSERT INTO` is not a table name.
    // With no case for it, a broken version of that branch would be indistinguishable
    // from a working one.
    let (code, text) = run_tree(
        "no-table-name",
        r#"pub const Q: &str = "INSERT INTO";
"#,
    );
    assert_eq!(code, 1, "an INSERT with no table name is not a statement:\n{text}");
    assert!(
        text.contains("INSERT INTO is not followed by a table name"),
        "the malformed INSERT was not reported:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// The gate refuses to report success while inspecting nothing
// ---------------------------------------------------------------------------

#[test]
fn a_tree_with_nothing_to_inspect_fails() {
    let root = std::env::temp_dir().join(format!("portability-gate-empty-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("src/backup")).unwrap();
    fs::copy(SCRIPT, root.join("scripts/check_backup_portability.sh")).unwrap();

    let (code, text) = run(&root);
    let _ = fs::remove_dir_all(&root);
    assert_eq!(code, 1, "a gate that inspects nothing must not pass:\n{text}");
    assert!(
        text.contains("no files inspected"),
        "the empty tree was not reported: {text}"
    );
}

#[test]
fn a_tree_without_a_backup_module_fails() {
    let root =
        std::env::temp_dir().join(format!("portability-gate-missing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::copy(SCRIPT, root.join("scripts/check_backup_portability.sh")).unwrap();

    let (code, text) = run(&root);
    let _ = fs::remove_dir_all(&root);
    assert_eq!(code, 1, "a missing module is not a pass:\n{text}");
    assert!(
        text.contains("src/backup does not exist"),
        "the missing directory was not reported: {text}"
    );
}
