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

/// Every construct Rule 1 must ban, spelled the way the script's array spells it.
///
/// Pinned from two sides: `every_banned_construct_is_reported` requires a report
/// line for each of these, and `the_script_bans_exactly_the_pinned_set` requires
/// the script's own array to hold exactly this set. Before both, eleven of the
/// fourteen had no fixture at all and deleting one from the array was invisible.
const BANNED: [&str; 14] = [
    "PRAGMA",
    "pragma_",
    "INSERT OR REPLACE",
    "INSERT OR IGNORE",
    "INSERT OR ROLLBACK",
    "INSERT OR ABORT",
    "INSERT OR FAIL",
    "AUTOINCREMENT",
    "last_insert_rowid",
    "strftime",
    "julianday",
    "datetime(",
    "IFNULL",
    "GROUP_CONCAT",
];

fn run(dir: &Path) -> (i32, String) {
    let out = Command::new("bash")
        .arg(dir.join("scripts/check_backup_portability.sh"))
        .output()
        .expect("the portability gate script runs");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

/// Build a throwaway tree holding `src/backup/fixture.rs`, run the real script
/// against it, and report what it said. The script resolves its own directory, so
/// copying it next to a fake `src/backup` is enough to redirect it.
fn run_tree(label: &str, source: &str) -> (i32, String) {
    let root = tree(label);
    fs::write(root.join("src/backup/fixture.rs"), source).unwrap();

    let out = run(&root);
    let _ = fs::remove_dir_all(&root);
    out
}

/// An empty throwaway tree: `scripts/` holding the real script, and `src/backup/`.
fn tree(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("portability-gate-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("src/backup")).unwrap();
    fs::copy(SCRIPT, root.join("scripts/check_backup_portability.sh")).unwrap();
    root
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
    // Per file, not just in total: a scanner that silently produced nothing for
    // three of the four files would still add up to four here.
    assert!(
        text.contains("inspected src/backup/restore.rs: 4 INSERT statement(s)"),
        "restore.rs holds the module's four INSERTs, said:\n{text}"
    );
    for file in ["export.rs", "mod.rs", "schema.rs"] {
        assert!(
            text.contains(&format!("inspected src/backup/{file}: 0 INSERT statement(s)")),
            "the scanner reported nothing for {file}, said:\n{text}"
        );
    }
}

#[test]
fn the_gate_counts_the_statements_it_inspected() {
    let (code, text) = run_tree(
        "counted",
        r#"pub const A: &str = "INSERT INTO assets (symbol) VALUES (?)";
pub const B: &str = "INSERT INTO portfolios (name) VALUES (?)";
"#,
    );
    assert_eq!(code, 0, "two well-formed INSERTs must pass:\n{text}");
    assert!(
        text.contains("inspected src/backup/fixture.rs: 2 INSERT statement(s)")
            && text.contains("INSERT statements inspected: 2"),
        "the count does not match the fixture:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Rule 1 — no banned construct in SQL the module can send
// ---------------------------------------------------------------------------

/// One fixture, one statement per banned construct, in the upper-case spelling the
/// module's SQL actually uses. Every entry of the array has to show up in the
/// output: a construct that is in the array but not reported is a construct the
/// gate does not ban, whatever the array claims.
#[test]
fn every_banned_construct_is_reported() {
    let mut source = String::new();
    for (i, pattern) in BANNED.iter().enumerate() {
        source.push_str(&format!(
            "pub const C{i}: &str = \"SELECT {} FROM t\";\n",
            pattern.to_uppercase()
        ));
    }
    let (code, text) = run_tree("all-banned", &source);
    assert_eq!(code, 1, "a fixture holding every banned construct must fail:\n{text}");
    for pattern in BANNED {
        assert!(
            text.contains(&format!("'{pattern}' has no cross-engine spelling")),
            "the gate did not report {pattern}:\n{text}"
        );
    }
}

/// The array is the rule. If it drifts from the set this file pins — an entry
/// deleted, a new one added without a fixture — the gate's verdict stops meaning
/// what the spec says it means, so say so loudly here instead.
#[test]
fn the_script_bans_exactly_the_pinned_set() {
    let script = fs::read_to_string(SCRIPT).expect("the gate script is readable");
    let body = script
        .split("BANNED=(")
        .nth(1)
        .expect("the script declares a BANNED array")
        .split(')')
        .next()
        .expect("the BANNED array is closed");
    let mut found: Vec<String> = body
        .lines()
        .filter_map(|l| l.trim().strip_prefix('"')?.strip_suffix('"').map(str::to_owned))
        .collect();
    let mut expected: Vec<String> = BANNED.iter().map(|s| s.to_string()).collect();
    found.sort();
    expected.sort();
    assert_eq!(
        found, expected,
        "the script's BANNED array and this file's pinned set have diverged"
    );
}

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
fn a_banned_construct_is_caught_whatever_its_spelling() {
    // SQLite keywords are case-insensitive, so a gate keyed on one spelling is a
    // gate on one spelling of the bypass. The module's house style is upper-case
    // SQL keywords, so `STRFTIME` is the spelling real code writes and `Pragma`
    // is the one a case-sensitive grep on a lower-case list would wave through.
    let (code, text) = run_tree(
        "spellings",
        r#"pub const LOWER: &str = "pragma foreign_keys = ON";
pub const MIXED: &str = "Pragma foreign_keys = ON";
pub const UPPER: &str = "SELECT STRFTIME('%Y-%m', date) FROM historical_prices";
pub const MIXED2: &str = "SELECT IfNull(close_price, 0) FROM historical_prices";
pub const UPPER2: &str = "SELECT LAST_INSERT_ROWID(), JULIANDAY(date), DATETIME(date)";
"#,
    );
    assert_eq!(code, 1, "one construct, five spellings:\n{text}");
    for pattern in ["PRAGMA", "strftime", "IFNULL", "last_insert_rowid", "julianday", "datetime("] {
        assert!(
            text.contains(&format!("'{pattern}' has no cross-engine spelling")),
            "{pattern} slipped through on its spelling:\n{text}"
        );
    }
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

/// `INSERT OR ROLLBACK` is the same SQLite-only family as `INSERT OR REPLACE` and
/// was missing from both the spec's list and the array. It is invisible to Rule 2
/// as well: `INSERT OR ROLLBACK INTO t VALUES (...)` holds no `INSERT INTO`, so
/// before this the whole statement passed.
#[test]
fn the_other_insert_or_variants_are_banned() {
    let (code, text) = run_tree(
        "insert-or-family",
        r#"pub const A: &str = "INSERT OR ROLLBACK INTO assets VALUES (?, ?)";
pub const B: &str = "INSERT OR ABORT INTO assets VALUES (?, ?)";
pub const C: &str = "INSERT OR FAIL INTO assets VALUES (?, ?)";
"#,
    );
    assert_eq!(code, 1, "the INSERT OR family is SQLite-only:\n{text}");
    for pattern in ["INSERT OR ROLLBACK", "INSERT OR ABORT", "INSERT OR FAIL"] {
        assert!(
            text.contains(&format!("'{pattern}' has no cross-engine spelling")),
            "{pattern} was not reported:\n{text}"
        );
    }
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
    let root = tree("empty-dir");

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
    let root = tree("missing-dir");
    fs::remove_dir_all(root.join("src/backup")).unwrap();

    let (code, text) = run(&root);
    let _ = fs::remove_dir_all(&root);
    assert_eq!(code, 1, "a missing module is not a pass:\n{text}");
    assert!(
        text.contains("src/backup does not exist"),
        "the missing directory was not reported: {text}"
    );
}

/// A path named `*.rs` that is not a file. `awk` warns and skips it while still
/// exiting 0, so without the guard it contributes zero statements and the gate
/// prints `passed`.
#[test]
fn a_path_named_rs_that_is_not_a_file_fails_the_gate() {
    let root = tree("not-a-file");
    fs::create_dir(root.join("src/backup/not_a_file.rs")).unwrap();

    let (code, text) = run(&root);
    let _ = fs::remove_dir_all(&root);
    assert_eq!(code, 1, "a directory is not a source file:\n{text}");
    assert!(
        text.contains("is not a readable regular file"),
        "the non-file path was not reported:\n{text}"
    );
}

/// A regular file the scanner cannot open, which is the same failure one step on:
/// `awk` never reads it, so it must not be allowed to look like a file with no
/// INSERTs in it.
#[test]
fn a_file_the_scanner_cannot_read_fails_the_gate() {
    let root = tree("unreadable-file");
    let path = root.join("src/backup/unreadable.rs");
    fs::write(&path, "pub const Q: &str = \"INSERT INTO assets (a) VALUES (?)\";\n").unwrap();
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    if fs::read_to_string(&path).is_ok() {
        // Running as root, where no file is unreadable and there is nothing to
        // assert. The guard above is not skipped, and the mutation evidence in the
        // task report covers this branch.
        let _ = fs::remove_dir_all(&root);
        return;
    }

    let (code, text) = run(&root);
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o644));
    let _ = fs::remove_dir_all(&root);
    assert_eq!(
        code, 1,
        "a file the scanner produced no verdict for must not pass:\n{text}"
    );
    assert!(
        text.contains("the INSERT scanner produced no verdict"),
        "the missing sentinel was not reported:\n{text}"
    );
}
