//! results.sarif must validate against the official SARIF 2.1.0 JSON schema
//! (OASIS, sarif-schema-2.1.0.json, errata 01).
//!
//! The schema is not vendored: OASIS publishes it under its IPR policy, not an
//! open-source licence. The test downloads it once with `curl` into Cargo's
//! test scratch folder, or reads the file named by `SARIF_SCHEMA`. Offline,
//! with no copy, the test prints why and passes without checking.

use std::path::{Path, PathBuf};
use std::process::Command;

const SCHEMA_URL: &str =
    "https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json";

fn schema() -> Option<serde_json::Value> {
    let path = match std::env::var_os("SARIF_SCHEMA") {
        Some(p) => PathBuf::from(p),
        None => {
            let p = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sarif-schema-2.1.0.json");
            if !p.is_file() {
                let ok = Command::new("curl")
                    .args(["-fsSL", "--max-time", "30", "-o"])
                    .arg(&p)
                    .arg(SCHEMA_URL)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if !ok {
                    let _ = std::fs::remove_file(&p);
                    eprintln!("SARIF schema not available offline ({SCHEMA_URL}); not checked");
                    return None;
                }
            }
            p
        }
    };
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

fn sarif_of(args: &[&str], out: &Path) -> serde_json::Value {
    let ws = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = Command::new(env!("CARGO_BIN_EXE_shelfcheck"))
        .current_dir(&ws)
        .env("NO_COLOR", "1")
        .args(args)
        .arg("--out")
        .arg(out)
        .arg("--quiet")
        .status()
        .expect("run shelfcheck");
    assert_eq!(status.code(), Some(1));
    let text = std::fs::read_to_string(out.join("results.sarif")).expect("results.sarif");
    serde_json::from_str(&text).expect("SARIF is JSON")
}

#[test]
fn sarif_validates_against_the_official_schema() {
    let Some(schema) = schema() else {
        return;
    };
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft4)
        .should_validate_formats(true)
        .build(&schema)
        .expect("the SARIF schema compiles");
    // The validator is live: a wrong version and a result without a message
    // are both rejected.
    let bad = serde_json::json!({
        "version": "9.9.9",
        "runs": [{"tool": {"driver": {"name": "x"}}, "results": [{"ruleId": "r"}]}]
    });
    assert!(validator.iter_errors(&bad).count() >= 2);

    let dir = tempfile::tempdir().expect("tempdir");
    let runs = [
        (
            vec!["check", "fixtures/store-a/products.csv"],
            dir.path().join("check"),
        ),
        (
            vec![
                "check",
                "fixtures/store-a/products.csv",
                "fixtures/store-a/products.xml",
                "fixtures/store-a/acp",
            ],
            dir.path().join("three"),
        ),
        (
            vec![
                "diff",
                "fixtures/store-a-oct/products.csv",
                "fixtures/store-a-nov/products.csv",
                "--store",
                "fixtures/store-a-export.csv",
            ],
            dir.path().join("diff"),
        ),
    ];
    for (args, out) in runs {
        let sarif = sarif_of(&args, &out);
        let errors: Vec<String> = validator
            .iter_errors(&sarif)
            .take(10)
            .map(|e| format!("{} at {}", e, e.instance_path()))
            .collect();
        assert!(errors.is_empty(), "{args:?}: {errors:#?}");
        assert!(
            sarif["runs"][0]["results"]
                .as_array()
                .is_some_and(|r| !r.is_empty())
        );
    }
}
