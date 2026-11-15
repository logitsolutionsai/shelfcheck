//! report.html: one offline file, rendered from a minijinja template with
//! the fonts embedded.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value as J, json};
use shelfcheck_core::diff::{Change, ChangeKind, DiffResults, Level, Snapshot, Status};
use shelfcheck_core::error::{Error, Result};
use shelfcheck_core::results::{CheckResults, Fix, FixStatus, ResultsFile, VERSION};

use crate::fonts;
use crate::{WriteOptions, pack_stamp, plural, shown_packs, thousands};

pub const TEMPLATE: &str = include_str!("../templates/report.html");

/// The first and last four characters of a hash: "4b1d…e802".
pub fn short_hash(h: &str) -> String {
    if h.len() <= 8 {
        return h.to_string();
    }
    format!("{}…{}", &h[..4], &h[h.len() - 4..])
}

/// "Error in all three", "Google error, OpenAI warning", "ACP error".
pub fn fix_summary(f: &Fix, checked: usize) -> String {
    let errors: Vec<&str> = f
        .formats
        .iter()
        .filter(|s| s.status == FixStatus::Error)
        .map(|s| s.format.label())
        .collect();
    let warnings: Vec<&str> = f
        .formats
        .iter()
        .filter(|s| s.status == FixStatus::Warning)
        .map(|s| s.format.label())
        .collect();
    if f.cross {
        return "Cross-format error".to_string();
    }
    if checked > 1 && errors.len() == checked {
        return if checked == 3 {
            "Error in all three".to_string()
        } else {
            format!("Error in all {checked}")
        };
    }
    let mut parts = Vec::new();
    if !errors.is_empty() {
        parts.push(format!("{} error", errors.join(", ")));
    }
    if !warnings.is_empty() {
        parts.push(format!("{} warning", warnings.join(", ")));
    }
    parts.join(", ")
}

fn status_text(s: FixStatus) -> &'static str {
    s.label()
}

fn fix_view(f: &Fix) -> J {
    let formats: Vec<J> = f
        .formats
        .iter()
        .filter(|s| s.status != FixStatus::NotChecked)
        .map(|s| {
            json!({
                "label": s.format.label(),
                "status": match s.status {
                    FixStatus::Error => "error",
                    FixStatus::Warning => "warning",
                    FixStatus::Passes => "passes",
                    FixStatus::NotUsed => "not_used",
                    FixStatus::NotChecked => "not_checked",
                },
                "text": status_text(s.status),
            })
        })
        .collect();
    let e = &f.example;
    // The sentence often quotes the value already ("5000000001553 has a
    // wrong check digit"); then the value is not shown twice.
    let value = e
        .value
        .clone()
        .filter(|v| !v.is_empty() && !e.message.contains(v.as_str()));
    json!({
        "rank": f.rank,
        "title": f.title,
        "detail": f.detail,
        "formats": if f.cross { Vec::new() } else { formats },
        "example": {
            "item": e.item_id,
            "field": e.field,
            "value": value,
            "message": e.message,
            "where": e.location.describe(),
        },
        "links": f.links.iter().map(|l| json!({"label": l.label, "url": l.url})).collect::<Vec<_>>(),
        "cites": f.cites.first().cloned().unwrap_or_default(),
        "products": thousands(f.products as u64),
        "unit": plural(f.products, "product", "products"),
    })
}

#[allow(clippy::type_complexity)]
fn snapshot_rows(old: &Snapshot, new: &Snapshot) -> Vec<J> {
    let opts = |s: &Snapshot| {
        s.options
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let fields: Vec<(&str, Option<String>, Option<String>, bool, bool)> = vec![
        ("title", old.title.clone(), new.title.clone(), false, true),
        ("gtin", old.gtin.clone(), new.gtin.clone(), true, true),
        ("brand", old.brand.clone(), new.brand.clone(), false, false),
        ("mpn", old.mpn.clone(), new.mpn.clone(), true, false),
        (
            "product type",
            old.product_type.clone(),
            new.product_type.clone(),
            false,
            false,
        ),
        (
            "options",
            Some(opts(old)).filter(|s| !s.is_empty()),
            Some(opts(new)).filter(|s| !s.is_empty()),
            false,
            false,
        ),
        (
            "group id",
            old.group_id.clone(),
            new.group_id.clone(),
            true,
            false,
        ),
        ("price", old.price.clone(), new.price.clone(), false, false),
        (
            "availability",
            old.availability.clone(),
            new.availability.clone(),
            true,
            false,
        ),
        (
            "id",
            Some(old.item_id.clone()),
            Some(new.item_id.clone()),
            true,
            false,
        ),
    ];
    fields
        .into_iter()
        .filter(|(_, a, b, _, always)| (a.is_some() || b.is_some()) && (*always || a != b))
        .map(|(name, a, b, mono, _)| {
            json!({"name": name, "old": a.unwrap_or_default(), "new": b.unwrap_or_default(), "mono": mono})
        })
        .collect()
}

fn change_view(c: &Change) -> J {
    let rows = match (&c.old, &c.new) {
        (Some(o), Some(n)) if c.kind != ChangeKind::StoreStale => snapshot_rows(o, n),
        _ => Vec::new(),
    };
    let item = match &c.new_item_id {
        Some(n) => format!("{} → {n}", c.item_id),
        None => c.item_id.clone(),
    };
    let evidence: Vec<J> = c
        .evidence
        .iter()
        .map(|e| json!({"text": e, "stale": e.ends_with("(stale)")}))
        .collect();
    let (verdict, by, at) = match &c.decision {
        Some(d) => (d.verdict.id().replace('_', " "), d.by.clone(), d.at.clone()),
        None => (String::new(), String::new(), String::new()),
    };
    json!({
        "id": c.id,
        "level": match c.level { Level::Decision => "decision", Level::Warning => "warning", Level::Info => "info" },
        "decided": c.status == Status::Decided,
        "verdict": verdict,
        "by": by,
        "at": at,
        "kind": c.kind.id(),
        "item": item,
        "sentence": c.sentence,
        "rows": rows,
        "evidence": evidence,
        "commands": c.commands,
    })
}

fn info_label(k: ChangeKind) -> &'static str {
    match k {
        ChangeKind::PriceOnly => "price-only",
        ChangeKind::StockOnly => "stock-only",
        ChangeKind::New => "new",
        ChangeKind::Retired => "retired",
        other => other.id(),
    }
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Column labels for the old and new feed: the file names, or, when both
/// are called the same (feeds/oct/products.csv, feeds/nov/products.csv),
/// the shortest trailing paths that tell them apart.
fn side_labels(old: &str, new: &str) -> (String, String) {
    let a: Vec<&str> = old.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = new.split('/').filter(|s| !s.is_empty()).collect();
    for n in 1..=a.len().max(b.len()) {
        let ta = a[a.len().saturating_sub(n)..].join("/");
        let tb = b[b.len().saturating_sub(n)..].join("/");
        if ta != tb {
            return (ta, tb);
        }
    }
    (
        format!("old: {}", file_name(old)),
        format!("new: {}", file_name(new)),
    )
}

fn diff_view(d: &DiffResults) -> J {
    let open_decisions: Vec<&Change> = d
        .changes
        .iter()
        .filter(|c| c.level == Level::Decision && c.status == Status::Open)
        .collect();
    let open_warnings: Vec<&Change> = d
        .changes
        .iter()
        .filter(|c| c.level == Level::Warning && c.status == Status::Open)
        .collect();
    let decided: Vec<&Change> = d
        .changes
        .iter()
        .filter(|c| c.level != Level::Info && c.status == Status::Decided)
        .collect();
    let info: Vec<&Change> = d
        .changes
        .iter()
        .filter(|c| c.level == Level::Info)
        .collect();

    let mut main: Vec<J> = Vec::new();
    for c in open_decisions
        .iter()
        .chain(open_warnings.iter())
        .chain(decided.iter())
    {
        main.push(change_view(c));
    }
    let mut info_counts: BTreeMap<usize, (String, usize)> = BTreeMap::new();
    for c in &info {
        let pos = ChangeKind::ALL
            .iter()
            .position(|k| *k == c.kind)
            .unwrap_or(0);
        let e = info_counts
            .entry(pos)
            .or_insert_with(|| (info_label(c.kind).to_string(), 0));
        e.1 += 1;
    }
    let info_parts: Vec<String> = info_counts
        .values()
        .map(|(l, n)| format!("{} {l}", thousands(*n as u64)))
        .collect();
    let mut counts = vec![format!("{} need a decision", open_decisions.len())];
    if !open_warnings.is_empty() {
        counts.push(format!(
            "{} {}",
            open_warnings.len(),
            plural(open_warnings.len(), "warning", "warnings")
        ));
    }
    if !decided.is_empty() {
        counts.push(format!("{} decided", decided.len()));
    }
    counts.extend(info_parts.iter().cloned());
    let s = &d.stats;
    json!({
        "old_file": d.old.path,
        "store_file": d.store.as_ref().map(|i| i.path.clone()),
        "old_label": side_labels(&d.old.path, &d.new.path).0,
        "new_label": side_labels(&d.old.path, &d.new.path).1,
        "counts": format!("{}.", counts.join(" · ")),
        "match_line": format!(
            "{} in both feeds: {} by gtin, {} by brand and mpn, {} by id only",
            thousands(s.in_both as u64), thousands(s.by_gtin as u64), thousands(s.by_brand_mpn as u64), thousands(s.by_id as u64)
        ),
        "main": main,
        "info": info.iter().map(|c| json!({"id": c.id, "kind": c.kind.id(), "item": c.item_id, "sentence": c.sentence})).collect::<Vec<_>>(),
        "info_summary": format!(
            "{} {} that need nothing from you: {}",
            thousands(info.len() as u64),
            plural(info.len(), "change", "changes"),
            info_parts.join(", ")
        ),
    })
}

#[allow(clippy::type_complexity)]
fn check_parts(c: &CheckResults) -> (String, String, Vec<J>, Vec<J>, Vec<J>, Vec<String>) {
    let total = c.summary.products;
    let need = c.summary.products_needing_fixes;
    let verdict = if need == 0 {
        format!(
            "All {} products pass every format checked",
            thousands(total as u64)
        )
    } else {
        format!(
            "{} of {} {} need fixes before every format accepts them",
            thousands(need as u64),
            thousands(total as u64),
            plural(total, "product", "products")
        )
    };
    let lede = if c.fixes.is_empty() {
        String::new()
    } else if c.fixes.len() <= 3 {
        format!(
            "{} {} cover every problem found.",
            c.fixes.len(),
            plural(c.fixes.len(), "fix", "fixes")
        )
    } else {
        let top: BTreeSet<&str> = c.fixes.iter().take(3).map(|f| f.key.as_str()).collect();
        let covered: BTreeSet<&str> = c
            .problems
            .iter()
            .filter(|p| top.contains(p.fix.as_str()))
            .map(|p| p.product.as_str())
            .collect();
        format!(
            "{} fixes cover every problem found, and the first three cover {} {}.",
            c.fixes.len(),
            thousands(covered.len() as u64),
            plural(covered.len(), "product", "products")
        )
    };
    let readiness = c
        .readiness
        .iter()
        .map(|r| {
            json!({
                "label": r.label,
                "ready": thousands(r.ready as u64),
                "percent": r.percent_ready,
                "with_errors": thousands(r.with_errors as u64),
                "with_warnings": thousands(r.with_warnings as u64),
                "rules_as_of": r.rules_as_of,
            })
        })
        .collect();
    let checked = c.formats.len();
    let top: Vec<J> = c.fixes.iter().take(3).map(fix_view).collect();
    let more: Vec<J> = c
        .fixes
        .iter()
        .skip(3)
        .map(|f| {
            json!({
                "rank": f.rank,
                "title": f.title,
                "summary": fix_summary(f, checked),
                "severity": f.severity.label(),
                "products": thousands(f.products as u64),
                "unit": plural(f.products, "product", "products"),
            })
        })
        .collect();
    let ignored = c.ignored.iter().map(|i| i.id.clone()).collect();
    (verdict, lede, readiness, top, more, ignored)
}

/// The template context, as JSON. Public so tests can check it.
pub fn context(results: &ResultsFile, opts: &WriteOptions) -> J {
    let check = results.check.as_ref();
    let diff = results.diff.as_ref();
    let mut verdict = String::new();
    let mut lede = String::new();
    let mut readiness = Vec::new();
    let mut top = Vec::new();
    let mut more = Vec::new();
    let mut ignored = Vec::new();
    if let Some(c) = check {
        let parts = check_parts(c);
        verdict = parts.0;
        lede = parts.1;
        readiness = parts.2;
        top = parts.3;
        more = parts.4;
        ignored = parts.5;
    }
    if let Some(d) = diff {
        let n = d.summary.needs_decision;
        let sentence = match n {
            0 => "No identity change since the last feed needs a decision.".to_string(),
            1 => "Since the last feed, one identity change also needs a decision from you."
                .to_string(),
            n => {
                format!("Since the last feed, {n} identity changes also need a decision from you.")
            }
        };
        if check.is_some() {
            lede = if lede.is_empty() {
                sentence
            } else {
                format!("{lede} {sentence}")
            };
        } else {
            verdict = match n {
                0 => "No change since the last feed needs a decision".to_string(),
                1 => "1 change since the last feed needs a decision".to_string(),
                n => format!("{n} changes since the last feed need a decision"),
            };
        }
    }

    let inputs_line = match (check, diff) {
        (Some(c), _) => {
            let paths: Vec<String> = c.inputs.iter().map(|i| i.path.clone()).collect();
            format!(
                "{} · {} products, {} variants",
                paths.join(", "),
                thousands(c.summary.products as u64),
                thousands(c.summary.items)
            )
        }
        (None, Some(d)) => format!("{} against {}", d.new.path, d.old.path),
        (None, None) => String::new(),
    };

    let mut hashes: Vec<String> = Vec::new();
    if let Some(c) = check {
        for i in &c.inputs {
            hashes.push(short_hash(&i.sha256));
        }
    }
    if let Some(d) = diff {
        for i in [Some(&d.old), Some(&d.new), d.store.as_ref()]
            .into_iter()
            .flatten()
        {
            let h = short_hash(&i.sha256);
            if !hashes.contains(&h) {
                hashes.push(h);
            }
        }
    }

    let packs_line = check
        .map(|c| {
            shown_packs(&c.packs)
                .iter()
                .map(|p| format!("{} {}", p.short, pack_stamp(p)))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let not_checked: Vec<J> = match check {
        Some(c) => c
            .not_checked
            .iter()
            .map(|n| json!({"title": n.title, "says": n.says}))
            .collect(),
        None => vec![
            json!({"title": "Format rules", "says": "This run compared two feeds only. Run shelfcheck check for the OpenAI, ACP, Google and GS1 rules."}),
            json!({"title": "Probabilistic matching", "says": "Items were matched on exact keys only: a valid GTIN, or brand, MPN and options. Title similarity is shown as a hint and never decides a match."}),
            json!({"title": "Whether you will be shown", "says": "A clean feed is accepted, not promoted. No tool can promise an assistant will show your products."}),
        ],
    };

    let more_from = if more.is_empty() { 0 } else { 4 };
    let more_to = 3 + more.len();
    json!({
        "version": VERSION,
        "run_at": opts.run_at,
        "inputs_line": inputs_line,
        "verdict": verdict,
        "lede": lede,
        "readiness": readiness,
        "has_check": check.is_some(),
        "fixes": check.map(|c| !c.fixes.is_empty()).unwrap_or(false),
        "top_fixes": top,
        "more_fixes": more,
        "more_from": more_from,
        "more_to": more_to,
        "ignored": ignored,
        "has_diff": diff.is_some(),
        "diff": diff.map(diff_view),
        "not_checked": not_checked,
        "packs_line": packs_line,
        "hashes_line": hashes.join(", "),
        "fonts": {
            "embedded": true,
            "sans": fonts::base64(fonts::GEIST),
            "mono": fonts::base64(fonts::GEIST_MONO),
        },
    })
}

/// serde_json to minijinja values, built from plain `From` conversions so no
/// minijinja serde feature is needed.
pub fn to_value(v: &J) -> minijinja::Value {
    match v {
        J::Null => minijinja::Value::from(()),
        J::Bool(b) => minijinja::Value::from(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => minijinja::Value::from(i),
            None => minijinja::Value::from(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => minijinja::Value::from(s.as_str()),
        J::Array(a) => minijinja::Value::from(a.iter().map(to_value).collect::<Vec<_>>()),
        J::Object(o) => {
            let m: BTreeMap<String, minijinja::Value> =
                o.iter().map(|(k, v)| (k.clone(), to_value(v))).collect();
            minijinja::Value::from(m)
        }
    }
}

pub fn render(results: &ResultsFile, opts: &WriteOptions) -> Result<String> {
    let ctx = to_value(&context(results, opts));
    let mut env = minijinja::Environment::new();
    env.add_template("report.html", TEMPLATE)
        .map_err(|e| Error::internal(format!("report template: {e}")))?;
    let tmpl = env
        .get_template("report.html")
        .map_err(|e| Error::internal(format!("report template: {e}")))?;
    tmpl.render(ctx)
        .map_err(|e| Error::internal(format!("report.html: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_results_render() {
        let html = render(&ResultsFile::new(), &WriteOptions::default()).expect("render");
        assert!(html.contains("What This Run Did Not Check"));
        assert!(html.contains("Built by Logit Solutions AI"));
        assert!(html.contains("font/woff2;base64,d09GMg"));
        // QA 2026-11-15: the base64 sits in a <style> element, where HTML
        // entities are not decoded, so it must be written unescaped (an
        // escaped "/" as "&#x2f;" broke both fonts).
        assert!(html.contains(&fonts::base64(fonts::GEIST)));
        assert!(html.contains(&fonts::base64(fonts::GEIST_MONO)));
        // Offline: no external fonts, scripts or styles.
        assert!(!html.contains("fonts.googleapis.com"));
        assert!(!html.contains("<script src"));
        assert!(!html.contains("<link "));
    }

    #[test]
    fn side_labels_tell_same_named_feeds_apart() {
        assert_eq!(
            side_labels(
                "fixtures/store-a-oct/products.csv",
                "fixtures/store-a-nov/products.csv"
            ),
            (
                "store-a-oct/products.csv".to_string(),
                "store-a-nov/products.csv".to_string()
            )
        );
        assert_eq!(
            side_labels("feeds/old.csv", "feeds/new.csv"),
            ("old.csv".to_string(), "new.csv".to_string())
        );
        assert_eq!(
            side_labels("a.csv", "a.csv"),
            ("old: a.csv".to_string(), "new: a.csv".to_string())
        );
    }

    #[test]
    fn hashes_are_shortened() {
        assert_eq!(short_hash("4b1d00000000e802"), "4b1d…e802");
    }
}
