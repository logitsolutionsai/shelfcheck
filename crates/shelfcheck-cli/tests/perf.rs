//! Performance: a generated 1,000,000-row OpenAI CSV, checked against all
//! three formats. Run in release mode:
//!
//!     cargo test --release -p shelfcheck --test perf -- --ignored --nocapture
//!
//! The target is under 10 seconds on a 2024 laptop. The result is printed
//! and written to target/perf-1m.txt; it is not asserted, because CI
//! machines vary. Set SHELFCHECK_PERF_MAX_SECS to make it a hard limit.

use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use shelfcheck_core::check::{self, CheckOptions};
use shelfcheck_core::rules::RuleSet;

fn check_digit(body: &str) -> u32 {
    let mut sum = 0;
    for (i, c) in body.chars().rev().enumerate() {
        let d = c.to_digit(10).unwrap_or(0);
        sum += if i % 2 == 0 { d * 3 } else { d };
    }
    (10 - sum % 10) % 10
}

/// Write `rows` rows; every 50th row carries one problem.
pub fn generate(path: &Path, rows: usize) -> std::io::Result<()> {
    let f = std::fs::File::create(path)?;
    let mut w = BufWriter::with_capacity(1 << 20, f);
    writeln!(
        w,
        "item_id,title,description,url,brand,seller_name,image_url,availability,price,target_countries,store_country,gtin,mpn,group_id,listing_has_variations,variant_dict"
    )?;
    for i in 0..rows {
        let group = i / 3;
        let body = format!("50{:010}", i + 1);
        let mut gtin = format!("{body}{}", check_digit(&body));
        let mut target = "US";
        let mut avail = "in_stock";
        match i % 150 {
            0 => {
                // Wrong check digit.
                let last = gtin.pop().and_then(|c| c.to_digit(10)).unwrap_or(0);
                gtin.push(char::from_digit((last + 1) % 10, 10).unwrap_or('0'));
            }
            50 => target = "",
            100 => avail = "preorder",
            _ => {}
        }
        writeln!(
            w,
            "SKU-{i},Perf Item {i},A generated row for the performance test.,https://perf.example/p/{group}?v={i},Perfbrand,Perf Store,https://perf.example/img/{i}.jpg,{avail},{}.99 USD,{target},US,{gtin},PB-{i},G{group},true,\"{{\"\"size\"\": \"\"{}\"\"}}\"",
            10 + i % 90,
            ["S", "M", "L"][i % 3]
        )?;
    }
    w.flush()
}

#[test]
#[ignore]
fn one_million_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("perf-1m.csv");
    let rows = 1_000_000;
    generate(&path, rows).expect("generate");
    let rules = RuleSet::load(&[], &[]).expect("rules");
    let started = Instant::now();
    let (res, _) = check::run(&[path], &rules, &CheckOptions::default()).expect("check");
    let secs = started.elapsed().as_secs_f64();
    assert_eq!(res.summary.items, rows as u64);
    let line = format!(
        "{rows} rows checked against OpenAI, ACP, Google and GS1 in {secs:.2} s ({:.0} rows per second), {} problems, {} threads",
        rows as f64 / secs,
        res.problems.len(),
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    );
    println!("{line}");
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let _ = std::fs::create_dir_all(&target);
    let _ = std::fs::write(target.join("perf-1m.txt"), format!("{line}\n"));
    if let Ok(max) = std::env::var("SHELFCHECK_PERF_MAX_SECS") {
        if let Ok(max) = max.parse::<f64>() {
            assert!(secs <= max, "{secs:.2} s is over the {max} s limit");
        }
    }
}

/// Where the time goes on 200,000 rows: reading alone, then each rule pack
/// alone on the records already in memory, on one thread. Run in release:
///
///     cargo test --release -p shelfcheck --test perf phase_breakdown -- --ignored --nocapture
#[test]
#[ignore]
fn phase_breakdown() {
    use shelfcheck_core::model::TargetFormat;
    use shelfcheck_core::read::{self, ReadOptions};
    use shelfcheck_core::rules::engine::{self, EngineOpts};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("perf-200k.csv");
    generate(&path, 200_000).expect("generate");

    let started = Instant::now();
    let (_, recs) = read::read_all(&path, 0, &ReadOptions::default()).expect("read");
    println!(
        "read 200,000 rows: {:.2} s",
        started.elapsed().as_secs_f64()
    );

    let file = path.display().to_string();
    for (label, ignore) in [
        ("all packs", vec![]),
        (
            "all but the ACP schema",
            vec!["acp.schema.product".to_string()],
        ),
    ] {
        let rules = RuleSet::load(&[], &ignore).expect("rules");
        let started = Instant::now();
        let mut n = 0usize;
        for r in &recs {
            n += engine::check_record(&rules, r, &file, &TargetFormat::ALL, &EngineOpts::default())
                .len();
        }
        println!(
            "{label}, one thread: {:.2} s, {n} problems",
            started.elapsed().as_secs_f64()
        );
    }
    for f in TargetFormat::ALL {
        let rules = RuleSet::load(&[], &[]).expect("rules");
        let started = Instant::now();
        for r in &recs {
            let _ = engine::check_record(&rules, r, &file, &[f], &EngineOpts::default());
        }
        println!(
            "{} (with GS1), one thread: {:.2} s",
            f.label(),
            started.elapsed().as_secs_f64()
        );
    }
}
