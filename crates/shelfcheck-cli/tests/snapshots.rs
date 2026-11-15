//! Snapshot tests (insta) of the demo runs in the README: the terminal output
//! and report.html of `check` and `diff` on the fixtures.
//!
//! Anything that depends on the machine is replaced before comparing:
//! - stage times ("0.62 s");
//! - input hashes, because git may check the fixtures out with other line
//!   ends on another machine;
//! - the embedded font data (70 KB of base64, checked by html.rs tests).
//!
//! The runs use relative paths from the workspace root and a relative output
//! folder, so no absolute path reaches a snapshot. `SOURCE_DATE_EPOCH` fixes
//! the report's run time.
//!
//! After an intended change: `INSTA_UPDATE=always cargo test -p shelfcheck
//! --test snapshots`, then read the diff of `tests/snapshots/*.snap`.

use std::path::Path;
use std::process::Command;

use regex::Regex;

fn workspace() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Run shelfcheck from the workspace root; exit 1 (problems found) expected.
fn run(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_shelfcheck"))
        .current_dir(workspace())
        .env("NO_COLOR", "1")
        .env("SOURCE_DATE_EPOCH", "1794614400")
        .args(args)
        .output()
        .expect("run shelfcheck");
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    assert_eq!(
        out.status.code(),
        Some(1),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

fn normalise_terminal(text: &str) -> String {
    let time = Regex::new(r"(?m)\d+\.\d+ s$").expect("regex");
    time.replace_all(text, "[time] s").into_owned()
}

fn normalise_html(text: &str) -> String {
    let font = Regex::new(r"base64,[A-Za-z0-9+/=]+\)").expect("regex");
    let short_hash = Regex::new(r"\b[0-9a-f]{4}…[0-9a-f]{4}\b").expect("regex");
    let long_hash = Regex::new(r"\b[0-9a-f]{64}\b").expect("regex");
    let text = text.replace("\r\n", "\n");
    let text = font.replace_all(&text, "base64,[font data])");
    let text = short_hash.replace_all(&text, "[hash]");
    long_hash.replace_all(&text, "[sha256]").into_owned()
}

fn report(out: &str) -> String {
    let path = workspace().join(out).join("report.html");
    normalise_html(&std::fs::read_to_string(path).expect("report.html"))
}

#[test]
fn demo_check() {
    let out = "target/insta-demo/check";
    let terminal = run(&[
        "check",
        "fixtures/store-a/products.csv",
        "--out",
        out,
        "--color",
        "never",
    ]);
    insta::assert_snapshot!("check_terminal", normalise_terminal(&terminal));
    insta::assert_snapshot!("check_report_html", report(out));
}

#[test]
fn demo_diff() {
    let out = "target/insta-demo/diff";
    let terminal = run(&[
        "diff",
        "fixtures/store-a-oct/products.csv",
        "fixtures/store-a-nov/products.csv",
        "--store",
        "fixtures/store-a-export.csv",
        "--decisions",
        "target/insta-demo/no-decisions.toml",
        "--out",
        out,
        "--color",
        "never",
    ]);
    insta::assert_snapshot!("diff_terminal", normalise_terminal(&terminal));
    insta::assert_snapshot!("diff_report_html", report(out));
}
