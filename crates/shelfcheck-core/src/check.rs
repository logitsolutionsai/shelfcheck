//! `shelfcheck check`: read feeds as streams, run the packs in parallel
//! chunks, then the cross-format rules, and summarise.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rayon::prelude::*;

use crate::error::Result;
use crate::gtin;
use crate::images;
use crate::model::{Availability, Location, Record, Severity, TargetFormat};
use crate::price::parse_price;
use crate::read::{self, Input, ReadOptions};
use crate::results::{
    CheckResults, CheckSummary, IgnoredRule, NotChecked, Problem, Readiness, build_fixes,
    pack_infos, percent,
};
use crate::rules::engine::{self, EngineOpts, render};
use crate::rules::{AppliesTo, RuleKind, RuleSet};

/// Records are checked in chunks of this many, each chunk across threads.
pub const CHUNK: usize = 8192;

#[derive(Debug, Clone)]
pub struct CheckOptions {
    pub formats: Vec<TargetFormat>,
    pub read: ReadOptions,
    pub engine: EngineOpts,
    pub images: Option<PathBuf>,
}

impl Default for CheckOptions {
    fn default() -> Self {
        CheckOptions {
            formats: TargetFormat::ALL.to_vec(),
            read: ReadOptions::default(),
            engine: EngineOpts::default(),
            images: None,
        }
    }
}

/// How long each stage took, for the terminal only (never in results.json).
#[derive(Debug, Clone, Default)]
pub struct Timings {
    pub read_and_check: Duration,
    pub cross: Duration,
}

fn bit(f: TargetFormat) -> u8 {
    match f {
        TargetFormat::Openai => 1,
        TargetFormat::Acp => 2,
        TargetFormat::Google => 4,
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    checked: u8,
    errors: u8,
    warnings: u8,
}

struct CrossVal {
    text: String,
    norm: String,
    loc: Location,
}

struct CrossEntry {
    product: String,
    vals: Vec<(String, CrossVal)>,
}

/// The formats each feed is checked against. A format with a feed in its own
/// format checks only those feeds; a format with none checks every feed.
pub fn targets_per_feed(inputs: &[Input], formats: &[TargetFormat]) -> Vec<Vec<TargetFormat>> {
    let mut out = vec![Vec::new(); inputs.len()];
    for &f in formats {
        let native: Vec<usize> = inputs
            .iter()
            .enumerate()
            .filter(|(_, i)| i.format.family.target() == Some(f))
            .map(|(n, _)| n)
            .collect();
        if native.is_empty() {
            for t in out.iter_mut() {
                t.push(f);
            }
        } else {
            for n in native {
                out[n].push(f);
            }
        }
    }
    for t in out.iter_mut() {
        t.sort();
    }
    out
}

fn cross_norm(field: &str, value: &str) -> Option<String> {
    match field {
        "price" => parse_price(value).ok().map(|p| p.canonical()),
        "availability" => Availability::parse(value).map(|a| format!("{a:?}")),
        // One barcode written with and without spaces or dashes is the same
        // barcode (Google ignores them), so compare it with them removed.
        "gtin" => {
            let bare = engine::strip_separators(value.trim());
            gtin::to_gtin14(&bare).or(Some(bare))
        }
        _ => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else {
                Some(v.to_string())
            }
        }
    }
}

#[allow(clippy::needless_range_loop)]
pub fn run(
    paths: &[PathBuf],
    rules: &RuleSet,
    opts: &CheckOptions,
) -> Result<(CheckResults, Timings)> {
    let started = Instant::now();
    let mut inputs = Vec::new();
    let mut iters = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        let (input, it) = read::open(p, i, &opts.read)?;
        inputs.push(input);
        iters.push(it);
    }
    let targets = targets_per_feed(&inputs, &opts.formats);

    let cross_fields: Vec<String> = if inputs.len() > 1 {
        rules
            .packs
            .iter()
            .filter(|p| p.meta.applies_to == AppliesTo::Cross)
            .flat_map(|p| p.rules.iter())
            .filter(|r| r.def.kind == RuleKind::SameAcrossFormats && !rules.is_ignored(&r.def.id))
            .filter_map(|r| r.def.field.clone())
            .collect()
    } else {
        Vec::new()
    };

    let mut problems: Vec<Problem> = Vec::new();
    let mut tally: HashMap<String, Tally> = HashMap::new();
    let mut items: u64 = 0;
    let mut cross: Vec<HashMap<String, CrossEntry>> = Vec::new();
    // --images: items measured, and items with no readable local image.
    let mut images_measured: u64 = 0;
    let mut images_missing: u64 = 0;
    // Fields a "not checked" entry is listed for (when_field), with the
    // format its pack applies to, and how many checked items have them.
    let watch: Vec<(String, Option<TargetFormat>)> = rules
        .packs
        .iter()
        .flat_map(|p| {
            p.not_checked.iter().filter_map(move |n| {
                n.when_field
                    .clone()
                    .map(|f| (f, p.meta.applies_to.target()))
            })
        })
        .collect();
    let mut field_counts: HashMap<String, u64> = HashMap::new();

    for (feed, it) in iters.into_iter().enumerate() {
        let file = inputs[feed].records_path.clone();
        let t = &targets[feed];
        let mask: u8 = t.iter().map(|f| bit(*f)).fold(0, |a, b| a | b);
        let mut cross_map: HashMap<String, CrossEntry> = HashMap::new();

        problems.extend(engine::check_metadata(rules, &inputs[feed], feed, t));

        let mut chunk: Vec<Record> = Vec::with_capacity(CHUNK);
        let mut it = it.peekable();
        let items_before = items;
        loop {
            chunk.clear();
            while chunk.len() < CHUNK {
                match it.next() {
                    Some(r) => chunk.push(r?),
                    None => break,
                }
            }
            if chunk.is_empty() {
                break;
            }
            items += chunk.len() as u64;
            if let Some(dir) = &opts.images {
                if t.contains(&TargetFormat::Google) {
                    for r in chunk.iter_mut() {
                        if !r.has("image_url") {
                            continue;
                        }
                        if images::add_image_fields(r, dir) {
                            images_measured += 1;
                        } else {
                            images_missing += 1;
                        }
                    }
                }
            }
            for (f, target) in &watch {
                if target.is_some_and(|x| !t.contains(&x)) {
                    continue;
                }
                let n = chunk
                    .iter()
                    .filter(|r| r.get(f).is_some_and(|x| x.is_present()))
                    .count() as u64;
                if n > 0 {
                    *field_counts.entry(f.clone()).or_default() += n;
                }
            }
            // Order-preserving parallel map: results come back in record order.
            let per_record: Vec<Vec<Problem>> = chunk
                .par_iter()
                .map(|r| engine::check_record(rules, r, &file, t, &opts.engine))
                .collect();
            for (rec, probs) in chunk.iter().zip(per_record) {
                let entry = tally.entry(rec.product_key()).or_default();
                entry.checked |= mask;
                for p in &probs {
                    let pm: u8 = p.formats.iter().map(|f| bit(*f)).fold(0, |a, b| a | b);
                    match p.severity {
                        Severity::Error => entry.errors |= pm,
                        Severity::Warning => entry.warnings |= pm,
                        Severity::Info => {}
                    }
                }
                problems.extend(probs);
                if !cross_fields.is_empty() {
                    let id = rec.item_id();
                    if rec.has("item_id") && !cross_map.contains_key(&id) {
                        let mut vals = Vec::new();
                        for f in &cross_fields {
                            if let Some(field) = rec.get(f).filter(|x| x.is_present()) {
                                if let Some(norm) = cross_norm(f, field.value()) {
                                    vals.push((
                                        f.clone(),
                                        CrossVal {
                                            text: field.value().to_string(),
                                            norm,
                                            loc: field.location(&file),
                                        },
                                    ));
                                }
                            }
                        }
                        cross_map.insert(
                            id,
                            CrossEntry {
                                product: rec.product_key(),
                                vals,
                            },
                        );
                    }
                }
            }
            if it.peek().is_none() {
                break;
            }
        }
        if items == items_before {
            return Err(crate::error::Error::input(read::no_rows_message(
                &inputs[feed].path,
            )));
        }
        cross.push(cross_map);
    }
    let read_and_check = started.elapsed();

    // Cross-format rules.
    let cross_started = Instant::now();
    if inputs.len() > 1 && !cross_fields.is_empty() {
        let cross_pack = rules
            .packs
            .iter()
            .find(|p| p.meta.applies_to == AppliesTo::Cross);
        if let Some(pack) = cross_pack {
            for k in 1..cross.len() {
                let mut ids: Vec<&String> = cross[k].keys().collect();
                ids.sort();
                for id in ids {
                    let Some(entry) = cross[k].get(id) else {
                        continue;
                    };
                    let Some(anchor_feed) = (0..k).find(|j| cross[*j].contains_key(id)) else {
                        continue;
                    };
                    let Some(anchor) = cross[anchor_feed].get(id) else {
                        continue;
                    };
                    for rule in &pack.rules {
                        if rule.def.kind != RuleKind::SameAcrossFormats
                            || rules.is_ignored(&rule.def.id)
                        {
                            continue;
                        }
                        let Some(field) = rule.def.field.as_deref() else {
                            continue;
                        };
                        let mine = entry.vals.iter().find(|(f, _)| f == field).map(|(_, v)| v);
                        let theirs = anchor.vals.iter().find(|(f, _)| f == field).map(|(_, v)| v);
                        let (Some(a), Some(b)) = (mine, theirs) else {
                            continue;
                        };
                        if a.norm == b.norm {
                            continue;
                        }
                        let mut formats: Vec<TargetFormat> = targets[k].clone();
                        for f in &targets[anchor_feed] {
                            if !formats.contains(f) {
                                formats.push(*f);
                            }
                        }
                        formats.sort();
                        let redacted = opts
                            .engine
                            .redact
                            .iter()
                            .any(|r| r.eq_ignore_ascii_case(field));
                        let show = |s: &str| {
                            if redacted {
                                "[redacted]".to_string()
                            } else {
                                s.to_string()
                            }
                        };
                        let vars = vec![
                            ("value", show(&a.text)),
                            ("other", show(&b.text)),
                            ("other_file", inputs[anchor_feed].records_path.clone()),
                            ("field", field.to_string()),
                        ];
                        let message = render(&rule.def.says, &vars);
                        let pm: u8 = formats.iter().map(|f| bit(*f)).fold(0, |x, y| x | y);
                        tally.entry(entry.product.clone()).or_default().errors |= pm;
                        problems.push(Problem {
                            rule: rule.def.id.clone(),
                            pack: pack.meta.id.clone(),
                            fix: rule.fix_key.clone(),
                            severity: rule.def.severity,
                            formats,
                            item_id: id.clone(),
                            product: entry.product.clone(),
                            feed: k,
                            location: a.loc.clone(),
                            field: Some(field.to_string()),
                            value: if redacted { None } else { Some(a.text.clone()) },
                            expected: if redacted { None } else { Some(b.text.clone()) },
                            message,
                            related: Some(b.loc.clone()),
                        });
                    }
                }
            }
        }
    }
    let cross_time = cross_started.elapsed();

    problems.sort_by_cached_key(Problem::sort_key);

    // Readiness per format.
    let mut readiness = Vec::new();
    for &f in &opts.formats {
        let b = bit(f);
        let checked = tally.values().filter(|t| t.checked & b != 0).count();
        let with_errors = tally
            .values()
            .filter(|t| t.checked & b != 0 && t.errors & b != 0)
            .count();
        let with_warnings = tally
            .values()
            .filter(|t| t.checked & b != 0 && t.warnings & b != 0)
            .count();
        let ready = checked - with_errors;
        let pack = rules.pack_for(f);
        readiness.push(Readiness {
            format: f,
            label: f.long_label().to_string(),
            products: checked,
            ready,
            with_errors,
            with_warnings,
            percent_ready: percent(ready, checked),
            pack: pack.map(|p| p.meta.id.clone()),
            rules_as_of: pack
                .and_then(|p| {
                    p.meta
                        .label
                        .clone()
                        .or_else(|| Some(format!("Checked {}", p.meta.checked)))
                })
                .unwrap_or_default(),
        });
    }

    let products_needing_fixes = tally.values().filter(|t| t.errors != 0).count();
    let summary = CheckSummary {
        products: tally.len(),
        items,
        products_needing_fixes,
        errors: problems
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .count(),
        warnings: problems
            .iter()
            .filter(|p| p.severity == Severity::Warning)
            .count(),
    };

    let fixes = build_fixes(&problems, rules, &opts.formats);

    let ignored: Vec<IgnoredRule> = rules
        .ignored
        .iter()
        .map(|id| IgnoredRule {
            id: id.clone(),
            pack: rules
                .find(id)
                .map(|(p, _)| p.meta.id.clone())
                .unwrap_or_default(),
        })
        .collect();

    let not_checked = not_checked(
        rules,
        opts,
        (images_measured, images_missing),
        &field_counts,
    );

    let results = CheckResults {
        inputs,
        formats: opts.formats.clone(),
        packs: pack_infos(rules),
        summary,
        readiness,
        fixes,
        problems,
        ignored,
        not_checked,
    };
    Ok((
        results,
        Timings {
            read_and_check,
            cross: cross_time,
        },
    ))
}

/// "What this run did not check", always present. `images` is (measured,
/// missing) when --images was given; `field_counts` is how many checked
/// items have each `when_field`.
pub fn not_checked(
    rules: &RuleSet,
    opts: &CheckOptions,
    images: (u64, u64),
    field_counts: &HashMap<String, u64>,
) -> Vec<NotChecked> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for p in &rules.packs {
        for n in &p.not_checked {
            if !seen.insert(n.id.clone()) {
                continue;
            }
            if let Some(f) = &n.when_field {
                let count = field_counts.get(f).copied().unwrap_or(0);
                if count == 0 {
                    continue;
                }
                out.push(NotChecked {
                    id: n.id.clone(),
                    title: n.title.clone(),
                    says: render(&n.says, &[("count", count.to_string())]),
                });
                continue;
            }
            let says = match (&opts.images, n.id.as_str()) {
                (Some(dir), "google.images.min_size") => {
                    let (measured, missing) = images;
                    if missing == 0 {
                        // Every image was measured: nothing to say here.
                        continue;
                    }
                    format!(
                        "{measured} images were measured in {}. {missing} items had no readable image there, so Google's 500 x 500 minimum, enforced from 31 January 2027, was not checked for them.",
                        dir.display()
                    )
                }
                _ => n.says.clone(),
            };
            out.push(NotChecked {
                id: n.id.clone(),
                title: n.title.clone(),
                says,
            });
        }
    }
    out.push(NotChecked {
        id: "pages".to_string(),
        title: "Product pages".to_string(),
        says: "Links were checked for form only. No page was fetched.".to_string(),
    });
    out.push(NotChecked {
        id: "ucp".to_string(),
        title: "UCP".to_string(),
        says: "Not in this version. Its official validator checks API responses, not feed files."
            .to_string(),
    });
    out.push(NotChecked {
        id: "shown".to_string(),
        title: "Whether you will be shown".to_string(),
        says: "A clean feed is accepted, not promoted. No tool can promise an assistant will show your products.".to_string(),
    });
    out
}

/// Convenience for tests and the CLI: check one feed with defaults.
pub fn check_paths(paths: &[&Path]) -> Result<CheckResults> {
    let rules = RuleSet::load(&[], &[])?;
    let owned: Vec<PathBuf> = paths.iter().map(|p| p.to_path_buf()).collect();
    Ok(run(&owned, &rules, &CheckOptions::default())?.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Container, Family, FeedFormat};

    fn input(family: Family, container: Container) -> Input {
        Input {
            path: "f".to_string(),
            records_path: "f".to_string(),
            format: FeedFormat::new(family, container),
            format_label: String::new(),
            sha256: String::new(),
            bytes: 0,
            acp_metadata: None,
            metadata_path: None,
        }
    }

    #[test]
    fn native_feeds_take_their_format() {
        let one = vec![input(Family::Openai, Container::Csv)];
        assert_eq!(
            targets_per_feed(&one, &TargetFormat::ALL),
            vec![TargetFormat::ALL.to_vec()]
        );
        let three = vec![
            input(Family::Openai, Container::Csv),
            input(Family::Google, Container::Xml),
            input(Family::Acp, Container::AcpBundle),
        ];
        assert_eq!(
            targets_per_feed(&three, &TargetFormat::ALL),
            vec![
                vec![TargetFormat::Openai],
                vec![TargetFormat::Google],
                vec![TargetFormat::Acp]
            ]
        );
        let shopify = vec![
            input(Family::Shopify, Container::Csv),
            input(Family::Google, Container::Xml),
        ];
        assert_eq!(
            targets_per_feed(&shopify, &TargetFormat::ALL),
            vec![
                vec![TargetFormat::Openai, TargetFormat::Acp],
                vec![
                    TargetFormat::Openai,
                    TargetFormat::Acp,
                    TargetFormat::Google
                ]
            ]
        );
    }
}
