//! End-to-end runs of the binary: exit codes, output files, decide.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_shelfcheck"));
    c.env("NO_COLOR", "1")
        .env("SOURCE_DATE_EPOCH", "1794614400");
    c
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn code(c: &mut Command) -> (i32, String, String) {
    let out = c.output().expect("run shelfcheck");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn check_finds_problems_and_writes_every_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("out");
    let (c, stdout, stderr) = code(
        bin()
            .arg("check")
            .arg(fixtures().join("store-a/products.csv"))
            .arg("--out")
            .arg(&out),
    );
    assert_eq!(c, 1, "{stdout}{stderr}");
    for f in [
        "results.json",
        "report.html",
        "results.sarif",
        "failing-products.csv",
        "junit.xml",
    ] {
        assert!(out.join(f).is_file(), "{f} written");
    }
    assert!(
        stdout.contains("need fixes before every format accepts them"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Barcodes with a wrong check digit"),
        "{stdout}"
    );
    let html = std::fs::read_to_string(out.join("report.html")).expect("html");
    assert!(html.contains("Fixes, Ranked by Products Affected"));
    assert!(!html.contains("fonts.googleapis.com"));
    let sarif: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("results.sarif")).expect("sarif"))
            .expect("json");
    assert_eq!(sarif["version"], "2.1.0");
}

#[test]
fn exit_codes() {
    let dir = tempfile::tempdir().expect("tempdir");
    // 3: a missing input file.
    let (c, _, _) = code(
        bin()
            .arg("check")
            .arg(dir.path().join("nope.csv"))
            .arg("--out")
            .arg(dir.path().join("o")),
    );
    assert_eq!(c, 3);
    // 2: an unknown rule to ignore.
    let (c, _, _) = code(
        bin()
            .arg("check")
            .arg(fixtures().join("store-a/products.csv"))
            .arg("--ignore")
            .arg("no.such.rule")
            .arg("--out")
            .arg(dir.path().join("o")),
    );
    assert_eq!(c, 2);
    // 2: invalid JSON in a JSONL feed.
    let bad = dir.path().join("bad.jsonl");
    std::fs::write(&bad, "{not json}\n").expect("write");
    let (c, _, _) = code(
        bin()
            .arg("check")
            .arg(&bad)
            .arg("--out")
            .arg(dir.path().join("o")),
    );
    assert_eq!(c, 2);
    // 0: a clean feed.
    let clean = dir.path().join("clean.csv");
    std::fs::write(
        &clean,
        "item_id,title,description,url,brand,seller_name,image_url,availability,price,target_countries,store_country,gtin\n\
         A1,Harbor Mug,A mug.,https://example.com/a1,Harbor,Harbor Shop,https://example.com/a1.jpg,in_stock,12.00 USD,US,US,0012345678905\n",
    )
    .expect("write");
    let (c, stdout, stderr) = code(
        bin()
            .arg("check")
            .arg(&clean)
            .arg("--out")
            .arg(dir.path().join("o")),
    );
    assert_eq!(c, 0, "{stdout}{stderr}");
}

#[test]
fn diff_then_decide_then_quiet() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("out");
    let decisions = dir.path().join("decisions.toml");
    let run_diff = || {
        code(
            bin()
                .arg("diff")
                .arg(fixtures().join("store-a-oct/products.csv"))
                .arg(fixtures().join("store-a-nov/products.csv"))
                .arg("--store")
                .arg(fixtures().join("store-a-export.csv"))
                .arg("--decisions")
                .arg(&decisions)
                .arg("--out")
                .arg(&out),
        )
    };
    let (c, stdout, stderr) = run_diff();
    assert_eq!(c, 1, "{stdout}{stderr}");
    assert!(stdout.contains("4 changes need a decision."), "{stdout}");
    for (id, flag) in [
        ("D1", "--different"),
        ("D2", "--fixed"),
        ("D3", "--same"),
        ("D4", "--same"),
    ] {
        let (c, stdout, stderr) = code(
            bin()
                .arg("decide")
                .arg(id)
                .arg(flag)
                .arg("--by")
                .arg("dev@example.com")
                .arg("--results")
                .arg(&out)
                .arg("--decisions")
                .arg(&decisions),
        );
        assert_eq!(c, 0, "{stdout}{stderr}");
    }
    let text = std::fs::read_to_string(&decisions).expect("decisions written");
    assert!(text.contains("verdict = \"different_product\""));
    let (c, stdout, stderr) = run_diff();
    assert_eq!(c, 0, "{stdout}{stderr}");
    assert!(stdout.contains("No change needs a decision."), "{stdout}");
}

#[test]
fn rules_list_and_describe() {
    let (c, stdout, _) = code(bin().arg("rules").arg("list"));
    assert_eq!(c, 0);
    assert!(stdout.contains("openai.required.target_countries"));
    let (c, stdout, _) = code(
        bin()
            .arg("rules")
            .arg("describe")
            .arg("gs1.gtin.check_digit"),
    );
    assert_eq!(c, 0);
    assert!(stdout.contains("check digit"));
}

/// Bad input gives exit 2 or 3 and one clear line on stderr, never a panic
/// and never a clean "0 products pass" run.
#[test]
fn bad_input_exits_with_one_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let write = |name: &str, body: &[u8]| {
        let p = dir.path().join(name);
        std::fs::write(&p, body).expect("write");
        p
    };
    let header = b"item_id,title,description,url,brand,seller_name,image_url,availability,price,target_countries,store_country\n";
    let cases: Vec<(PathBuf, i32, &str)> = vec![
        (write("empty.csv", b""), 2, "no product rows"),
        (write("header-only.csv", header), 2, "no product rows"),
        (
            write("semicolons.csv", b"item_id;title;price\nA1;Mug;1.00 USD\n"),
            2,
            "semicolons",
        ),
        (
            write("notes.txt", b"hello world\n"),
            2,
            "none of the header columns",
        ),
        (
            write(
                "broken.xml",
                b"<?xml version=\"1.0\"?>\n<rss><channel><item><g:id>1</g:id>\n</channel></rss>\n",
            ),
            2,
            "invalid XML",
        ),
        (
            write("broken.jsonl", b"{\"item_id\":\"A\"}\n{oops\n"),
            2,
            "broken.jsonl:2",
        ),
        (dir.path().join("missing.csv"), 3, "no such file"),
    ];
    for (path, want, says) in cases {
        let (c, stdout, stderr) = code(
            bin()
                .arg("check")
                .arg(&path)
                .arg("--out")
                .arg(dir.path().join("out")),
        );
        assert_eq!(c, want, "{}: {stdout}{stderr}", path.display());
        assert!(stderr.contains(says), "{}: {stderr}", path.display());
        assert_eq!(stderr.trim_end().lines().count(), 1, "{stderr}");
        assert!(!stderr.contains("panicked"), "{stderr}");
    }
}

#[test]
fn bad_decisions_file_is_invalid_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    let decisions = dir.path().join("decisions.toml");
    std::fs::write(
        &decisions,
        "[[decision]]\nid = \"D1\"\nchange = \"id_reused\"\nitem_id = \"X\"\nfingerprint = \"f\"\nverdict = \"maybe\"\nby = \"a\"\nat = \"2026-11-15\"\n",
    )
    .expect("write");
    let (c, _, stderr) = code(
        bin()
            .arg("diff")
            .arg(fixtures().join("store-a-oct/products.csv"))
            .arg(fixtures().join("store-a-nov/products.csv"))
            .arg("--decisions")
            .arg(&decisions)
            .arg("--out")
            .arg(dir.path().join("out")),
    );
    assert_eq!(c, 2, "{stderr}");
    assert!(stderr.contains("invalid decisions file"), "{stderr}");
}

/// The suggested decide command is pasted as it is, so an output folder
/// with a space is quoted.
#[test]
fn decide_commands_quote_paths_with_spaces() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("my results");
    let (c, stdout, stderr) = code(
        bin()
            .arg("diff")
            .arg(fixtures().join("store-a-oct/products.csv"))
            .arg(fixtures().join("store-a-nov/products.csv"))
            .arg("--decisions")
            .arg(dir.path().join("d.toml"))
            .arg("--out")
            .arg(&out),
    );
    assert_eq!(c, 1, "{stdout}{stderr}");
    let json = std::fs::read_to_string(out.join("results.json")).expect("results");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let cmd = v["diff"]["changes"][0]["commands"][0]
        .as_str()
        .expect("command")
        .to_string();
    assert!(cmd.contains("--results \""), "{cmd}");
    assert!(cmd.contains("my results\""), "{cmd}");
}
