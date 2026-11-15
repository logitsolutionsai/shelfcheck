//! Hand-made fixtures (`fixtures/hand/`), written during QA on 2026-11-15
//! without `fixtures/generate.py`. The gold file comes from the same script
//! as the fixtures, so it can only show that shelfcheck agrees with the
//! generator. Every expected value here was worked out by hand from the rule
//! sources and the GS1 check-digit method, not from shelfcheck's output.
//!
//! - `openai-tricky.csv`: CRLF line ends, a barcode for every GS1 prefix rule
//!   (and the 977, 978 and 979 prefixes that are not errors), titles of
//!   exactly 150 and 151 accented characters, and awkward prices.
//! - `google-tricky.xml`: Google RSS with CRLF, CDATA and entities.
//! - `openai-tricky.jsonl`: a relative link, a missing description and a
//!   lowercase country code.
//! - `cross-openai.csv` and `cross-google.xml`: the same four items in two
//!   formats.
//! - `openai-variants.csv` (fix pass 2): a variant row without
//!   listing_has_variations (a warning), and barcodes written with dashes and
//!   spaces, which Google ignores and OpenAI does not accept.
//! - `google-qa.csv` (fix pass 2): Google's quoted, comma-separated
//!   question_and_answer form, which v1 does not read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use shelfcheck_core::check::{self, CheckOptions};
use shelfcheck_core::model::TargetFormat;
use shelfcheck_core::results::{CheckResults, Problem};
use shelfcheck_core::rules::RuleSet;

fn hand(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/hand")
        .join(name)
}

fn run(files: &[&str], formats: &[TargetFormat]) -> CheckResults {
    let rules = RuleSet::load(&[], &[]).expect("rules");
    let paths: Vec<PathBuf> = files.iter().map(|f| hand(f)).collect();
    let opts = CheckOptions {
        formats: formats.to_vec(),
        ..CheckOptions::default()
    };
    check::run(&paths, &rules, &opts).expect("check").0
}

/// (item, rule) for every problem from the given packs.
fn hits(res: &CheckResults, packs: &[&str]) -> BTreeSet<(String, String)> {
    res.problems
        .iter()
        .filter(|p| packs.contains(&p.pack.as_str()))
        .map(|p| (p.item_id.clone(), p.rule.clone()))
        .collect()
}

fn set(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn find<'a>(res: &'a CheckResults, item: &str, rule: &str) -> &'a Problem {
    res.problems
        .iter()
        .find(|p| p.item_id == item && p.rule == rule)
        .unwrap_or_else(|| panic!("no {rule} on {item}"))
}

#[test]
fn barcodes_prices_and_unicode_titles() {
    let res = run(&["openai-tricky.csv"], &TargetFormat::ALL);
    let expected = set(&[
        ("B05", "gs1.gtin.restricted_prefix"),    // 20...
        ("B06", "gs1.gtin.restricted_prefix"),    // UPC 4..., read as 04...
        ("B07", "gs1.gtin.prefix_05"),            // warning only
        ("B08", "gs1.gtin.demo_prefix"),          // 952
        ("B09", "gs1.gtin.coupon_prefix"),        // 981
        ("B10", "gs1.gtin.refund_prefix"),        // 980
        ("B11", "gs1.gtin.coupon_prefix"),        // 99
        ("B12", "gs1.gtin.unused_prefix"),        // 0000042
        ("B13", "gs1.gtin.check_digit"),          // 4006381333932, should end in 1
        ("B14", "gs1.gtin.form"),                 // dashes
        ("B15", "gs1.gtin.form"),                 // ISBN-10
        ("B18", "gs1.gtin.restricted_prefix_8"),  // GTIN-8 starting with 2
        ("U02", "openai.title.length"),           // 151 characters
        ("P01", "openai.price.form"),             // 1,299.00 USD
        ("P02", "openai.price.form"),             // €79.99
        ("P03", "openai.price.form"),             // 79.99 usd
        ("P04", "openai.sale_price.below_price"), // 89.99 over 79.99
        ("P05", "openai.sale_price.below_price"), // EUR against USD
    ]);
    assert_eq!(hits(&res, &["gs1-gtin", "openai-product-feed"]), expected);

    // ISBN (978, 979), ISSN (977), GTIN-8, -12 and -14 and exactly 150
    // characters raise nothing anywhere.
    for clean in [
        "B01", "B02", "B03", "B04", "B16", "B17", "B19", "U01", "U03", "P06",
    ] {
        let any: Vec<&str> = res
            .problems
            .iter()
            .filter(|p| p.item_id == clean)
            .map(|p| p.rule.as_str())
            .collect();
        assert!(any.is_empty(), "{clean}: {any:?}");
    }

    // The check digit, its message and the cell: CRLF file, B13 on line 14,
    // gtin is column 13 (M).
    let p = find(&res, "B13", "gs1.gtin.check_digit");
    assert_eq!(p.expected.as_deref(), Some("1"));
    assert_eq!(p.location.line, 14);
    assert_eq!(p.location.column, Some(13));
    assert_eq!(
        p.message,
        "4006381333932 has a wrong check digit. It should end in 1."
    );
    // 151 accented characters is a Google error and an OpenAI warning.
    assert_eq!(find(&res, "U02", "google.title.length").location.line, 22);
    assert_eq!(
        find(&res, "B07", "gs1.gtin.prefix_05").severity,
        shelfcheck_core::model::Severity::Warning
    );
}

#[test]
fn google_xml_with_crlf_cdata_and_entities() {
    let res = run(&["google-tricky.xml"], &[TargetFormat::Google]);
    let expected = set(&[
        ("G2", "google.availability.enum"), // pre_order is OpenAI's spelling
        ("G3", "google.required.availability"),
        ("G4", "google.price.form"), // 15,00 EUR
        ("G5", "google.availability_date.required"),
        ("G6", "google.availability_date.backorder"),
        ("G8", "google.apparel.color"),
        ("G8", "google.apparel.size"),
        ("G8", "google.apparel.gender"),
        ("G8", "google.apparel.age_group"),
    ]);
    assert_eq!(hits(&res, &["google-merchant", "gs1-gtin"]), expected);
    // Lines counted by hand in the CRLF file: G2's <item> is on line 17 and
    // its availability element nine lines later; a missing field points at
    // the <item> line (G8 on line 93).
    assert_eq!(
        find(&res, "G2", "google.availability.enum").location.line,
        26
    );
    assert_eq!(find(&res, "G8", "google.apparel.size").location.line, 93);
    assert_eq!(
        find(&res, "G6", "google.availability_date.backorder").severity,
        shelfcheck_core::model::Severity::Warning
    );
    assert_eq!(res.summary.items, 8);
}

#[test]
fn jsonl_with_a_relative_link_and_a_lowercase_country() {
    let res = run(
        &["openai-tricky.jsonl"],
        &[TargetFormat::Openai, TargetFormat::Google],
    );
    let expected = set(&[
        ("J2", "openai.url.absolute"),
        ("J2", "google.link.absolute"),
        ("J3", "openai.required.description"),
        ("J3", "google.required.description"),
        ("J3", "openai.target_countries.form"),
    ]);
    assert_eq!(
        hits(
            &res,
            &["openai-product-feed", "google-merchant", "gs1-gtin"]
        ),
        expected
    );
    assert_eq!(
        find(&res, "J3", "openai.required.description")
            .location
            .line,
        3
    );
}

#[test]
fn cross_format_compares_meaning_not_spelling() {
    let res = run(
        &["cross-openai.csv", "cross-google.xml"],
        &TargetFormat::ALL,
    );
    // X1 price 10.00 against 12.00; X3 brand Handmade against HandMade Co.
    // X2 pre_order and preorder mean the same; X4 is one GTIN written as
    // GTIN-12 and GTIN-13.
    let expected = set(&[("X1", "cross.price"), ("X3", "cross.brand")]);
    assert_eq!(hits(&res, &["cross-format"]), expected);
    // Each feed is checked against its own format: preorder in the Google
    // feed and pre_order in the OpenAI feed are both correct.
    assert!(
        !res.problems
            .iter()
            .any(|p| p.rule.ends_with("availability.enum")),
        "{:?}",
        res.problems
    );
}

/// `diff-old.csv` against `diff-new.csv` with `diff-store.csv`: one item
/// per change kind, worked out by hand.
#[test]
fn diff_finds_every_change_kind_by_hand() {
    use shelfcheck_core::diff::{self, DiffOptions};
    let dir = tempfile::tempdir().expect("tempdir");
    let d = diff::run(
        &hand("diff-old.csv"),
        &hand("diff-new.csv"),
        Some(&hand("diff-store.csv")),
        &dir.path().join("none.toml"),
        &DiffOptions::default(),
    )
    .expect("diff");
    let got: BTreeSet<(String, String)> = d
        .changes
        .iter()
        .map(|c| (c.kind.id().to_string(), c.item_id.clone()))
        .collect();
    let expected = set(&[
        // Same id, a different gtin and product type: a phone became a case.
        ("id_reused", "R1"),
        // No gtin on either side: the id alone does not make them the same
        // product, because brand and mpn differ.
        ("id_reused", "N1"),
        ("critical_attribute_changed", "C1"), // 128 GB to 256 GB, same gtin
        ("store_stale", "C1"),                // store title still 128 GB
        ("id_changed", "MUG-1"),              // same gtin, new id MUG-1B
        ("group_changed", "GRP1"),
        ("duplicate_id", "D1"),
        ("price_only", "P1"),
        ("new", "NEW1"),
        ("retired", "RET1"),
        // No barcode on either side: the same id, brand Handmade and MPN
        // SL-9, with storage 128 GB in the old feed and 256 GB in the new.
        ("critical_attribute_changed", "S1"),
        // Not changes: C2 (0.128 TB is 128 GB) and K1 (BLK is Black).
    ]);
    assert_eq!(got, expected);
    let s1 = d
        .changes
        .iter()
        .find(|c| c.kind.id() == "critical_attribute_changed" && c.item_id == "S1")
        .expect("S1");
    assert_eq!(s1.fingerprint, "capacity:128 GB->256 GB");
    assert_eq!(
        s1.sentence,
        "S1 kept its id, brand and MPN, but its storage changed from 128 GB to 256 GB."
    );
    let c1 = d
        .changes
        .iter()
        .find(|c| c.kind.id() == "critical_attribute_changed" && c.item_id == "C1")
        .expect("C1");
    assert_eq!(c1.fingerprint, "capacity:128 GB->256 GB");
    let mug = d
        .changes
        .iter()
        .find(|c| c.kind.id() == "id_changed")
        .expect("MUG-1");
    assert_eq!(mug.new_item_id.as_deref(), Some("MUG-1B"));
    assert_eq!(d.summary.needs_decision, 5);
}

/// Decision 2026-11-15: a variant row without listing_has_variations is a
/// warning, because OpenAI keeps the row ("Omitted, empty, or false: no
/// variant options are used"). Barcodes: OpenAI says "no spaces or dashes";
/// Google says "spaces and dashes are accepted, but ignored".
#[test]
fn variant_flag_is_a_warning_and_google_ignores_barcode_separators() {
    use shelfcheck_core::model::Severity;
    let res = run(&["openai-variants.csv"], &TargetFormat::ALL);
    let expected = set(&[
        ("V1-S", "openai.variants.listing_has_variations"),
        ("V4", "gs1.gtin.form"),
        ("V5", "gs1.gtin.form"),
        ("V5", "gs1.gtin.check_digit"),
    ]);
    assert_eq!(hits(&res, &["gs1-gtin", "openai-product-feed"]), expected);

    let v1 = find(&res, "V1-S", "openai.variants.listing_has_variations");
    assert_eq!(v1.severity, Severity::Warning);
    // A warning never blocks: TEE1, CAP1 and V3 are ready for OpenAI; V4 and
    // V5 are not (their barcodes have dashes or spaces).
    let openai = res
        .readiness
        .iter()
        .find(|r| r.format == TargetFormat::Openai)
        .expect("openai");
    assert_eq!((openai.products, openai.ready), (5, 3));

    // 400-6381-333931 is 4006381333931 to Google, a valid EAN-13: an error for
    // OpenAI and ACP only.
    let v4 = find(&res, "V4", "gs1.gtin.form");
    assert_eq!(v4.formats, vec![TargetFormat::Openai, TargetFormat::Acp]);
    // 400 6381 333932 is 4006381333932 to Google: a wrong check digit there,
    // quoted as written. OpenAI and ACP reject the spaces first.
    let v5 = find(&res, "V5", "gs1.gtin.check_digit");
    assert_eq!(v5.formats, vec![TargetFormat::Google]);
    assert_eq!(v5.expected.as_deref(), Some("1"));
    assert_eq!(
        v5.message,
        "400 6381 333932 has a wrong check digit. It should end in 1."
    );
    assert_eq!(
        find(&res, "V5", "gs1.gtin.form").formats,
        vec![TargetFormat::Openai, TargetFormat::Acp]
    );
}

/// Decision 2026-11-15: Google's quoted, comma-separated question_and_answer
/// form is not read in v1. It raises no problem, and the run says, in one
/// line, that it was not read.
#[test]
fn question_and_answer_is_listed_as_not_read() {
    let res = run(&["google-qa.csv"], &[TargetFormat::Google]);
    assert!(
        !res.problems
            .iter()
            .any(|p| p.field.as_deref() == Some("question_and_answer")),
        "{:?}",
        res.problems
    );
    let note = res
        .not_checked
        .iter()
        .find(|n| n.id == "google.question_and_answer")
        .expect("a not-checked line for question_and_answer");
    assert!(
        note.says
            .starts_with("question_and_answer was not read on 1 of the items checked."),
        "{}",
        note.says
    );
    assert!(!note.says.contains('\n'));

    // A feed without the attribute gets no such line.
    let res = run(&["cross-google.xml"], &[TargetFormat::Google]);
    assert!(
        !res.not_checked
            .iter()
            .any(|n| n.id == "google.question_and_answer")
    );
}
