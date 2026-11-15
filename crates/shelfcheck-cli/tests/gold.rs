//! Gold tests: `check` and `diff` on the fixtures must equal gold.json
//! exactly. No missed problems, no extra ones.
//!
//! gold.json is written by fixtures/generate.py from the seeds, not from
//! shelfcheck's own output.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use shelfcheck_core::check::{self, CheckOptions};
use shelfcheck_core::diff::{self, DiffOptions};
use shelfcheck_core::model::TargetFormat;
use shelfcheck_core::rules::RuleSet;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn gold() -> Value {
    let text = std::fs::read_to_string(fixtures().join("gold.json"))
        .expect("fixtures/gold.json; run python fixtures/generate.py");
    serde_json::from_str(&text).expect("gold.json is JSON")
}

/// The part of a path after "fixtures/".
fn rel(file: &str) -> String {
    let f = file.replace('\\', "/");
    match f.rsplit_once("fixtures/") {
        Some((_, r)) => r.to_string(),
        None => f,
    }
}

fn s(v: &Value) -> String {
    match v {
        Value::Null => "-".to_string(),
        Value::String(x) => x.clone(),
        other => other.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn key(
    rule: &str,
    file: &str,
    line: u64,
    column: &str,
    pointer: &str,
    item: &str,
    formats: &str,
    expected: &str,
) -> String {
    // The expected digit is compared for check-digit problems only.
    let expected = if rule == "gs1.gtin.check_digit" {
        expected
    } else {
        "-"
    };
    format!("{rule}|{file}|{line}|{column}|{pointer}|{item}|{formats}|{expected}")
}

fn compare(name: &str, mut want: Vec<String>, mut got: Vec<String>) {
    want.sort();
    got.sort();
    if want == got {
        return;
    }
    let missing: Vec<&String> = want.iter().filter(|w| !got.contains(w)).take(15).collect();
    let extra: Vec<&String> = got.iter().filter(|g| !want.contains(g)).take(15).collect();
    panic!(
        "{name}: {} expected, {} found.\nmissed (first 15):\n  {}\nextra (first 15):\n  {}",
        want.len(),
        got.len(),
        missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  "),
        extra
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

#[test]
fn check_cases_match_gold() {
    let g = gold();
    let rules = RuleSet::load(&[], &[]).expect("rules");
    for case in g["check"].as_array().expect("check cases") {
        let name = case["name"].as_str().unwrap_or("?");
        let inputs: Vec<PathBuf> = case["inputs"]
            .as_array()
            .expect("inputs")
            .iter()
            .map(|p| fixtures().join(p.as_str().expect("path")))
            .collect();
        let formats: Vec<TargetFormat> = case["formats"]
            .as_array()
            .expect("formats")
            .iter()
            .filter_map(|f| f.as_str().and_then(TargetFormat::parse))
            .collect();
        let opts = CheckOptions {
            formats,
            ..CheckOptions::default()
        };
        let (res, _) = check::run(&inputs, &rules, &opts).expect("check runs");

        let want: Vec<String> = case["problems"]
            .as_array()
            .expect("problems")
            .iter()
            .map(|p| {
                let formats: Vec<String> = p["formats"]
                    .as_array()
                    .expect("formats")
                    .iter()
                    .map(s)
                    .collect();
                key(
                    &s(&p["rule"]),
                    &s(&p["file"]),
                    p["line"].as_u64().expect("line"),
                    &s(&p["column"]),
                    &s(&p["pointer"]),
                    &s(&p["item_id"]),
                    &formats.join(","),
                    &s(&p["expected"]),
                )
            })
            .collect();
        let got: Vec<String> = res
            .problems
            .iter()
            .map(|p| {
                let formats: Vec<&str> = p.formats.iter().map(|f| f.id()).collect();
                key(
                    &p.rule,
                    &rel(&p.location.file),
                    p.location.line,
                    &p.location
                        .column
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    p.location.pointer.as_deref().unwrap_or("-"),
                    &p.item_id,
                    &formats.join(","),
                    p.expected.as_deref().unwrap_or("-"),
                )
            })
            .collect();
        compare(name, want, got);
        assert_eq!(
            res.summary.products as u64,
            case["summary"]["products"].as_u64().expect("products"),
            "{name}: products"
        );
        assert_eq!(
            res.summary.items,
            case["summary"]["items"].as_u64().expect("items"),
            "{name}: items"
        );
    }
}

#[test]
fn diff_cases_match_gold() {
    let g = gold();
    for case in g["diff"].as_array().expect("diff cases") {
        let name = case["name"].as_str().unwrap_or("?");
        let f = |k: &str| fixtures().join(case[k].as_str().expect("path"));
        let store = f("store");
        let decisions = match case["decisions"].as_str() {
            Some(d) => fixtures().join(d),
            // A path that does not exist: no decisions yet.
            None => fixtures().join("no-decisions-here.toml"),
        };
        let res = diff::run(
            &f("old"),
            &f("new"),
            Some(&store),
            &decisions,
            &DiffOptions::default(),
        )
        .expect("diff runs");
        let want: Vec<String> = case["changes"]
            .as_array()
            .expect("changes")
            .iter()
            .map(|c| {
                format!(
                    "{}|{}|{}|{}|{}",
                    s(&c["id"]),
                    s(&c["kind"]),
                    s(&c["item_id"]),
                    s(&c["fingerprint"]),
                    s(&c["status"])
                )
            })
            .collect();
        let got: Vec<String> = res
            .changes
            .iter()
            .map(|c| {
                let status = match c.status {
                    diff::Status::Open => "open",
                    diff::Status::Decided => "decided",
                };
                format!(
                    "{}|{}|{}|{}|{status}",
                    c.id,
                    c.kind.id(),
                    c.item_id,
                    c.fingerprint
                )
            })
            .collect();
        compare(name, want, got);
        let st = &case["stats"];
        let stats: BTreeMap<&str, usize> = [
            ("old_items", res.stats.old_items),
            ("new_items", res.stats.new_items),
            ("in_both", res.stats.in_both),
            ("by_gtin", res.stats.by_gtin),
            ("by_brand_mpn", res.stats.by_brand_mpn),
            ("by_id", res.stats.by_id),
        ]
        .into_iter()
        .collect();
        for (k, v) in stats {
            assert_eq!(st[k].as_u64(), Some(v as u64), "{name}: stats.{k}");
        }
        assert_eq!(
            res.summary.needs_decision as u64,
            case["needs_decision"].as_u64().expect("needs_decision"),
            "{name}: needs_decision"
        );
    }
}

/// Property: reordering rows never changes the results.
#[test]
fn reordering_rows_keeps_results() {
    let src = fixtures().join("store-a-nov/products.csv");
    let text = std::fs::read_to_string(src).expect("fixture");
    let mut lines: Vec<&str> = text.lines().collect();
    let header = lines.remove(0);
    lines.reverse();
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("a.csv");
    let b = dir.path().join("b.csv");
    std::fs::write(&a, &text).expect("write");
    std::fs::write(&b, format!("{header}\n{}\n", lines.join("\n"))).expect("write");
    let rules = RuleSet::load(&[], &[]).expect("rules");
    let ra = check::run(&[a], &rules, &CheckOptions::default())
        .expect("check a")
        .0;
    let rb = check::run(&[b], &rules, &CheckOptions::default())
        .expect("check b")
        .0;
    let strip = |r: &shelfcheck_core::results::CheckResults| {
        let mut v: Vec<(String, String, String)> = r
            .problems
            .iter()
            .map(|p| (p.rule.clone(), p.item_id.clone(), p.message.clone()))
            .collect();
        v.sort();
        v
    };
    assert_eq!(strip(&ra), strip(&rb));
    assert_eq!(
        ra.summary.products_needing_fixes,
        rb.summary.products_needing_fixes
    );
}
