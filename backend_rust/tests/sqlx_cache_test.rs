//! The gate that keeps the prepare cache honest has to be testable without either of
//! the things it normally needs.
//!
//! `scripts/check_sqlx_cache.sh` shells out to exactly two external programs — `sqlx`
//! (for `migrate run`) and `cargo sqlx prepare --check` — and everything else it does
//! is shell builtins, `grep`, `ls` and `wc`. So the properties that matter can be pinned
//! with PATH shims in a temporary directory, the same trick
//! `tests/access_gate_test.rs` uses with the real script copied next to a throwaway
//! tree: no database, no `sqlx-cli`, no network.
//!
//! What is pinned, and why each one is worth a test:
//!
//! * **It refuses instead of skipping.** A gate whose tool is missing has to fail. If it
//!   exited 0 when `sqlx-cli` was not installed, the CI image losing the binary would
//!   read as a clean build — the failure mode every gate here is written against.
//! * **`migrate run` happens before `prepare --check`.** `prepare --check` describes the
//!   *live* database, so an unmigrated one makes it fail for the wrong reason and sends
//!   someone to regenerate a cache that was already correct. Until this file that
//!   ordering was enforced only by a comment in the script.
//! * **Drift names the query.** `sqlx-cli`'s own bail message names a
//!   `query-<sha256>.json` file, which is useless to a reader; the script's value is
//!   that it prints the SQL inside that file.
//! * **A dead cache entry is surfaced.** `prepare --check` only *warns* about `.sqlx`
//!   entries nothing regenerates and still exits 0, so the cache can accumulate junk and
//!   stay green. Failing on that would fail on a cosmetic, but swallowing it would let the
//!   gate claim more honesty than it has.
//! * **The residue is reported.** The count of `// macro-exempt:` markers is the number
//!   the migration wants to reach zero *deliberately*. Pinning it here means adding an
//!   exemption is an edit someone has to make in a test.
//!
//! What is deliberately **not** covered: the real `prepare --check` comparison itself,
//! which needs a live database and `sqlx-cli`. That half was verified by hand against
//! the development cluster — two mutations, a live `ALTER TABLE` and a hand-stale cache
//! entry — and both transcripts are recorded in the script's header.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/check_sqlx_cache.sh");

/// The script's shebang is `/usr/bin/env bash`, which needs `PATH` to find `bash` — and
/// two of these tests deliberately run it with a `PATH` holding nothing else.
const BASH: &str = "/bin/bash";

/// A cache file the throwaway tree pretends to hold. The sha is arbitrary; what matters
/// is that the drift message names *this* file, so the SQL printed back has to have
/// come from it.
const FAKE_SHA: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
const FAKE_SQL: &str = "SELECT id, name FROM portfolios WHERE id = $1";

/// sqlx-cli 0.9.0's actual failure text (`prepare.rs`'s `bail!`), reproduced verbatim
/// apart from the sha. Anything else would test the script against a message that does
/// not exist.
const BAIL_TEXT: &str = "prepare check failed: one or more query files differ";

/// `sqlx-cli`'s one-directional warning: dead cache entries, exit status still 0.
const UNUSED_TEXT: &str = "potentially unused queries found in .sqlx";

/// What the real tree's residue is right now. If this changes, say why in the commit
/// that changes it — that is the whole point of the assertion.
const REAL_RESIDUE: usize = 10;

/// The URL the shims are told about. Nothing connects to it; it only has to be non-empty
/// enough to get past the script's first guard.
const SHIM_URL: &str = "postgres://shim@127.0.0.1:1/shim_does_not_connect";

fn bash() -> &'static str {
    if Path::new(BASH).exists() {
        BASH
    } else {
        "bash"
    }
}

/// One scratch directory per test, holding both the shims and the throwaway tree, so
/// that cleaning up is one `remove_dir_all` of a path this module made.
fn base(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sqlx-cache-{}-{label}", std::process::id()))
}

/// A directory of PATH shims standing in for the two programs the script needs.
///
/// `cargo` is shimmed rather than `cargo-sqlx` because the script calls `cargo sqlx
/// prepare …`, and shimming `cargo` means the test never runs the real toolchain. A
/// `cargo-sqlx` file is still created because the script's own presence check looks for
/// that name — shimming it away would test the wrong thing.
struct Shims {
    dir: PathBuf,
    log: PathBuf,
}

impl Shims {
    fn new(label: &str) -> Self {
        let root = base(label);
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("bin");
        fs::create_dir_all(&dir).unwrap();
        let shims = Self {
            dir,
            log: root.join("calls.log"),
        };
        shims.write("sqlx", "exit 0");
        shims.write("cargo-sqlx", "exit 0");
        shims.write("cargo", "exit 0");
        shims
    }

    /// Replace one shim's behaviour. Everything before `extra` is the argv log line.
    fn write(&self, name: &str, extra: &str) {
        let body = format!("#!/bin/bash\necho \"{name} $*\" >> \"$SHIM_LOG\"\n{extra}\n");
        let path = self.dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Drop a program so `command -v` cannot find it, leaving the rest of the `PATH`
    /// empty on purpose: if the guard did not fire, the script would die on a missing
    /// `ls` with status 127 rather than the 1 these tests assert.
    fn hide(&self, name: &str) {
        fs::remove_file(self.dir.join(name)).unwrap();
    }

    fn path(&self, keep_real_path: bool) -> std::ffi::OsString {
        let mut dirs = vec![self.dir.clone()];
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        if keep_real_path {
            dirs.extend(
                std::env::split_paths(&inherited).filter(|p| !p.as_os_str().is_empty()),
            );
        }
        std::env::join_paths(dirs).unwrap()
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// A throwaway tree the copied script can be pointed at: `scripts/`, a `migrations/`
/// directory, and one `.sqlx` cache file whose SQL the script can be asked to print.
fn tree(label: &str) -> PathBuf {
    let root = base(label).join("tree");
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("migrations")).unwrap();
    fs::create_dir_all(root.join(".sqlx")).unwrap();
    fs::copy(SCRIPT, root.join("scripts/check_sqlx_cache.sh")).unwrap();
    fs::write(root.join("migrations/0001_init.sql"), "-- throwaway\n").unwrap();
    fs::write(
        root.join(format!(".sqlx/query-{FAKE_SHA}.json")),
        format!(
            "{{\n  \"db_name\": \"PostgreSQL\",\n  \"query\": \"{FAKE_SQL}\",\n  \
             \"describe\": null,\n  \"hash\": \"{FAKE_SHA}\"\n}}\n"
        ),
    )
    .unwrap();
    root
}

struct Run {
    code: i32,
    text: String,
}

fn run_with(root: &Path, shims: &Shims, database_url: Option<&str>, keep_real_path: bool) -> Run {
    let mut cmd = Command::new(bash());
    cmd.arg(root.join("scripts/check_sqlx_cache.sh"))
        .current_dir(root)
        .env("SHIM_LOG", &shims.log)
        .env("PATH", shims.path(keep_real_path));
    match database_url {
        Some(url) => {
            cmd.env("DATABASE_URL", url);
        }
        None => {
            cmd.env_remove("DATABASE_URL");
        }
    }
    let out = cmd.output().expect("the gate script runs");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Run {
        code: out.status.code().unwrap_or(-1),
        text,
    }
}

/// The integer a `"<label>: <n>"` output line reported, so the assertions read as
/// numbers rather than as substring guesses.
fn reported(run: &Run, label: &str) -> Option<usize> {
    run.text
        .lines()
        .find(|line| line.starts_with(label))
        .and_then(|line| line.rsplit(':').next())
        .and_then(|tail| tail.trim().parse::<usize>().ok())
}

// ---------------------------------------------------------------------------
// (a) and (b): it refuses instead of skipping.
// ---------------------------------------------------------------------------

#[test]
fn it_refuses_without_a_database_url() {
    let label = "no-url";
    let shims = Shims::new(label);
    let root = tree(label);
    let run = run_with(&root, &shims, None, true);
    let calls = shims.calls();
    let _ = fs::remove_dir_all(base(label));

    assert_eq!(
        run.code, 1,
        "an unset DATABASE_URL must not look like a pass:\n{}",
        run.text
    );
    assert!(
        run.text.contains("DATABASE_URL"),
        "the message should name what is missing:\n{}",
        run.text
    );
    assert!(
        calls.is_empty(),
        "it must refuse before running anything, but it ran:\n{calls:?}"
    );
}

#[test]
fn it_refuses_when_sqlx_is_missing_and_says_how_to_install_it() {
    let label = "no-sqlx";
    let shims = Shims::new(label);
    shims.hide("sqlx");
    let root = tree(label);
    let run = run_with(&root, &shims, Some(SHIM_URL), false);
    let calls = shims.calls();
    let _ = fs::remove_dir_all(base(label));

    assert_eq!(
        run.code, 1,
        "a missing sqlx must fail the gate, not skip it:\n{}",
        run.text
    );
    assert!(
        run.text.contains("cargo install sqlx-cli"),
        "the gate should name the install command:\n{}",
        run.text
    );
    assert!(calls.is_empty(), "it ran something anyway:\n{calls:?}");
}

#[test]
fn it_refuses_when_cargo_sqlx_is_missing_even_though_sqlx_is_present() {
    // The two binaries come from one package but are two separate `command -v` checks,
    // and a half-installed toolchain is exactly the state that would otherwise slip
    // through a guard that only looked for the first one.
    let label = "no-cargo-sqlx";
    let shims = Shims::new(label);
    shims.hide("cargo-sqlx");
    let root = tree(label);
    let run = run_with(&root, &shims, Some(SHIM_URL), false);
    let _ = fs::remove_dir_all(base(label));

    assert_eq!(
        run.code, 1,
        "a missing cargo-sqlx must fail the gate:\n{}",
        run.text
    );
    assert!(
        run.text.contains("cargo-sqlx") && run.text.contains("cargo install sqlx-cli"),
        "it should name the missing binary and the fix:\n{}",
        run.text
    );
}

// ---------------------------------------------------------------------------
// (c): the ordering the header calls load-bearing.
// ---------------------------------------------------------------------------

#[test]
fn it_migrates_before_it_checks_the_cache() {
    let label = "order";
    let shims = Shims::new(label);
    let root = tree(label);
    let run = run_with(&root, &shims, Some(SHIM_URL), true);
    let calls = shims.calls();
    let _ = fs::remove_dir_all(base(label));

    assert_eq!(run.code, 0, "a clean tree should pass:\n{}", run.text);
    assert!(run.text.contains("GATE PASSED"), "said:\n{}", run.text);

    let migrated = calls
        .iter()
        .position(|c| c.contains("migrate run") && c.contains("--no-dotenv"))
        .expect("`sqlx migrate run --no-dotenv` was never invoked; calls were:\n{calls:?}");
    let checked = calls
        .iter()
        .position(|c| c.contains("prepare --check -- --all-targets"))
        .expect("`cargo sqlx prepare --check -- --all-targets` was never invoked; calls were:\n{calls:?}");

    assert!(
        migrated < checked,
        "`prepare --check` describes the live database, so an unmigrated one fails it for \
         the wrong reason. Order was:\n{calls:?}"
    );
    assert_eq!(
        calls.len(),
        2,
        "the script should run exactly two tools:\n{calls:?}"
    );
}

// ---------------------------------------------------------------------------
// (d): drift names the query, not just the file.
// ---------------------------------------------------------------------------

#[test]
fn drift_exits_one_and_prints_the_sql_from_the_named_cache_file() {
    let label = "drift";
    let shims = Shims::new(label);
    shims.write(
        "cargo",
        &format!(
            "printf '%s\\n' '{BAIL_TEXT} (query-{FAKE_SHA}.json); you should re-run sqlx prepare' >&2\nexit 1"
        ),
    );
    let root = tree(label);
    let run = run_with(&root, &shims, Some(SHIM_URL), true);
    let _ = fs::remove_dir_all(base(label));

    assert_eq!(run.code, 1, "drift has to fail the build:\n{}", run.text);
    assert!(run.text.contains("GATE FAILED"), "said:\n{}", run.text);
    assert!(
        run.text.contains(BAIL_TEXT),
        "sqlx-cli's own message should still be visible:\n{}",
        run.text
    );
    assert!(
        run.text.contains(FAKE_SQL),
        "the point of the script is turning a sha256 into the query that drifted; \
         `{FAKE_SQL}` was not printed:\n{}",
        run.text
    );
}

#[test]
fn dead_cache_entries_are_surfaced_even_though_sqlx_only_warns() {
    // `prepare --check` prints this and still exits 0, so the cache can accumulate
    // entries nothing regenerates. The gate cannot fail on it without failing on a
    // cosmetic, but it must not swallow it either.
    let label = "dead";
    let shims = Shims::new(label);
    shims.write("cargo", &format!("echo 'warning: {UNUSED_TEXT}'\nexit 0"));
    let root = tree(label);
    let run = run_with(&root, &shims, Some(SHIM_URL), true);
    let _ = fs::remove_dir_all(base(label));

    assert_eq!(run.code, 0, "a warning is not a failure:\n{}", run.text);
    assert!(
        run.text.contains("NOTE:") && run.text.contains(UNUSED_TEXT),
        "the warning should be reported as a NOTE with the way to fix it:\n{}",
        run.text
    );
    assert!(
        run.text.contains("cargo sqlx prepare -- --all-targets"),
        "the NOTE should say how to prune:\n{}",
        run.text
    );
}

// ---------------------------------------------------------------------------
// The residue count, against the real tree.
// ---------------------------------------------------------------------------

#[test]
fn the_real_tree_reports_its_macro_exempt_residue() {
    // Run the *real* script, from the real directory, with the shims in place so it
    // touches no database. This is the only place the number the migration is steering
    // toward zero is asserted anywhere.
    let label = "real";
    let shims = Shims::new(label);
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let run = run_with(&repo, &shims, Some(SHIM_URL), true);
    let _ = fs::remove_dir_all(base(label));

    assert_eq!(run.code, 0, "the real tree should pass:\n{}", run.text);
    assert_eq!(
        reported(&run, "macro-exempt runtime queries in src/"),
        Some(REAL_RESIDUE),
        "the residue moved. Each `// macro-exempt:` marker is a query that stayed runtime \
         SQL on purpose; changing the count is allowed, but it has to be argued in the \
         commit that changes it and in this constant. Said:\n{}",
        run.text
    );
    assert!(
        reported(&run, "cached queries:").is_some_and(|n| n > 50),
        "the cache should be found and counted; said:\n{}",
        run.text
    );
}
