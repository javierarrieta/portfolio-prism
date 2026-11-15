//! The gate that keeps the upload path whole.
//!
//! `scripts/check_upload_path.sh` is what stops the proxy in front of the API from
//! being narrower than the API's own bundle cap. That is exactly the kind of rule a
//! grep gate rots on: the two numbers live in different languages, in different
//! directories, and nothing in the Rust tree reads the nginx file. A loosening of the
//! script would look identical to a clean repository, which is the worst failure mode
//! a lint can have — it fails by reporting success. Same reasoning as
//! `backup_portability_test.rs` and `access_gate_test.rs`.
//!
//! This failure is not hypothetical. A 1,284,105-byte bundle — an ordinary export of
//! a few years of price history — was refused by nginx's 1 MiB default with an HTML
//! page, before the backend saw a byte, while the API advertised an 8 MiB cap. The UI
//! could only say the answer carried no readable reason. It could.
//!
//! Every rule gets both directions: a case that must fail proves the rule bites, a
//! case that must pass proves it is not decoration that also refuses correct code.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/scripts/check_upload_path.sh"
);

/// The panel as it ships, minus nothing: three mentions is the definition plus the
/// preview call plus the restore call, which is what rule 2 counts.
const PANEL: &str = "\
function oversizeReason(file, caps) { return null; }
if (oversizeReason(file, caps)) return;
if (oversizeReason(file, caps)) return;
";

fn nginx_with(directive: &str) -> String {
    format!(
        "\
server {{
    listen 80;
    root /usr/share/nginx/html;

    location / {{
        try_files $uri $uri/ /index.html;
    }}

    location /api {{
        proxy_pass http://127.0.0.1:8000;
{directive}
    }}
}}
"
    )
}

/// Run the real script against a throwaway tree and report what it said.
fn run_tree(label: &str, max_bundle: &str, nginx: &str, panel: &str) -> (i32, String) {
    let root = std::env::temp_dir().join(format!("upload-gate-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("backend_rust/scripts")).unwrap();
    fs::create_dir_all(root.join("backend_rust/src/backup")).unwrap();
    fs::create_dir_all(root.join("frontend/src/components")).unwrap();
    fs::copy(SCRIPT, root.join("backend_rust/scripts/check_upload_path.sh")).unwrap();
    fs::write(
        root.join("backend_rust/src/backup/mod.rs"),
        format!("pub const MAX_BUNDLE_BYTES: usize = {max_bundle};\n"),
    )
    .unwrap();
    fs::write(root.join("frontend/nginx.conf"), nginx).unwrap();
    fs::write(root.join("frontend/src/components/BackupPanel.tsx"), panel).unwrap();

    let out = Command::new("bash")
        .arg(root.join("backend_rust/scripts/check_upload_path.sh"))
        .output()
        .expect("the upload-path gate script runs");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = fs::remove_dir_all(&root);
    (out.status.code().unwrap_or(-1), text)
}

fn ok(label: &str, max: &str, nginx: &str) -> (i32, String) {
    run_tree(label, max, nginx, PANEL)
}

#[test]
fn the_real_tree_passes_the_gate() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("bash")
        .arg(repo.join("scripts/check_upload_path.sh"))
        .output()
        .expect("the upload-path gate runs");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code().unwrap_or(-1), 0, "gate failed on the repo:\n{text}");
    // The numbers are asserted, not just the verdict: a script that inspected nothing
    // would also print a pass, and these two lines are what prove it read both files.
    assert!(text.contains("MAX_BUNDLE_BYTES: 8388608 bytes"), "said:\n{text}");
    assert!(text.contains("client_max_body_size: 12m = 12582912 bytes"), "said:\n{text}");
}

#[test]
fn a_proxy_narrower_than_the_api_is_refused() {
    // The shipped bug: nginx's 1 MiB default against an 8 MiB API cap.
    let (code, text) = ok("narrow", "8 * 1024 * 1024", &nginx_with("        client_max_body_size 1m;"));
    assert_eq!(code, 1, "a 1 MiB proxy must not pass an 8 MiB cap, said:\n{text}");
    assert!(text.contains("cannot be re-imported through the UI"), "said:\n{text}");
}

#[test]
fn a_missing_directive_is_refused() {
    let (code, text) = ok("missing", "8 * 1024 * 1024", &nginx_with(""));
    assert_eq!(code, 1, "no directive means nginx's 1 MiB default, said:\n{text}");
    assert!(text.contains("no `client_max_body_size`"), "said:\n{text}");
}

#[test]
fn raising_the_api_cap_alone_is_caught() {
    // The direction that matters going forward: someone widens MAX_BUNDLE_BYTES in
    // Rust, every Rust test still passes, and the UI silently stops importing.
    let (code, text) = ok("raise", "16 * 1024 * 1024", &nginx_with("        client_max_body_size 12m;"));
    assert_eq!(code, 1, "16 MiB over a 12 MiB proxy must fail, said:\n{text}");
    assert!(text.contains("12582912"), "the gate should name both sizes, said:\n{text}");
}

#[test]
fn a_directive_out_of_scope_for_api_is_refused() {
    // Present in the file, in a location the upload never reaches. A file-wide grep
    // would call this a pass; scope is the whole point of rule 1.
    let nginx = "\
server {
    listen 80;
    location /api {
        proxy_pass http://127.0.0.1:8000;
    }
    location /other {
        client_max_body_size 64m;
    }
}
";
    let (code, text) = ok("scope", "8 * 1024 * 1024", nginx);
    assert_eq!(code, 1, "a directive in another location does not cover /api, said:\n{text}");
    assert!(text.contains("no `client_max_body_size`"), "said:\n{text}");
}

#[test]
fn a_server_level_directive_counts() {
    let nginx = "\
server {
    listen 80;
    client_max_body_size 20m;
    location /api {
        proxy_pass http://127.0.0.1:8000;
    }
}
";
    let (code, text) = ok("serverlevel", "8 * 1024 * 1024", nginx);
    assert_eq!(code, 0, "server level applies to every location, said:\n{text}");
}

#[test]
fn exact_and_bare_and_uppercase_sizes_all_parse() {
    // Exactly at the cap is allowed: the API accepts a bundle of MAX_BUNDLE_BYTES.
    let (code, _) = ok("exact", "8 * 1024 * 1024", &nginx_with("        client_max_body_size 8m;"));
    assert_eq!(code, 0, "8m against an 8 MiB cap is equal, not under");
    // A bare number is bytes to nginx.
    let (code, _) = ok(
        "bare",
        "8 * 1024 * 1024",
        &nginx_with("        client_max_body_size 8388608;"),
    );
    assert_eq!(code, 0, "a bare byte count must parse as bytes");
    // Suffixes are case-insensitive.
    let (code, _) = ok("upper", "8 * 1024 * 1024", &nginx_with("        client_max_body_size 64M;"));
    assert_eq!(code, 0, "64M must parse like 64m");
}

#[test]
fn unlimited_is_reported_rather_than_refused() {
    // `0` disables nginx's limit. That is a decision an operator is allowed to make,
    // so the gate says it out loud instead of failing — but it says it.
    let (code, text) = ok("unlimited", "8 * 1024 * 1024", &nginx_with("        client_max_body_size 0;"));
    assert_eq!(code, 0, "unlimited is a choice, said:\n{text}");
    assert!(text.contains("unlimited"), "the gate should announce it, said:\n{text}");
}

#[test]
fn a_panel_that_stopped_refusing_locally_is_refused() {
    let (code, text) = run_tree(
        "panel",
        "8 * 1024 * 1024",
        &nginx_with("        client_max_body_size 12m;"),
        "// nobody checks the size any more\n",
    );
    assert_eq!(code, 1, "rule 2 must bite when the local refusal is gone, said:\n{text}");
    assert!(text.contains("refuses an oversized bundle"), "said:\n{text}");
}

#[test]
fn a_layout_that_moved_is_refused_not_skipped() {
    // Inspecting nothing is the failure this whole file exists to prevent.
    let root = std::env::temp_dir().join(format!("upload-gate-empty-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("backend_rust/scripts")).unwrap();
    fs::copy(SCRIPT, root.join("backend_rust/scripts/check_upload_path.sh")).unwrap();
    let out = Command::new("bash")
        .arg(root.join("backend_rust/scripts/check_upload_path.sh"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(out.status.code().unwrap_or(-1), 1, "missing inputs must fail loudly, said:\n{text}");
    assert!(text.contains("is missing"), "said:\n{text}");
}

#[test]
fn a_constant_that_stopped_being_arithmetic_is_refused() {
    let (code, text) = ok("weird", "MAX_UPLOAD", &nginx_with("        client_max_body_size 12m;"));
    assert_eq!(code, 1, "a non-arithmetic cap must not be silently skipped, said:\n{text}");
    assert!(text.contains("not arithmetic"), "said:\n{text}");
}
