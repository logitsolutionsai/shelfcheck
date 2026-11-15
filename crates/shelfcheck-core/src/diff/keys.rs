//! Identity keys, strongest first:
//! 1. a valid GTIN, unique within the feed;
//! 2. brand plus MPN plus the variant option values, unique within the feed;
//! 3. the feed's own id, never trusted alone: it is compared against keys 1
//!    and 2 to detect reuse.
//!
//! Title similarity is a hint for the reviewer and never decides a match.

use std::collections::HashMap;

use crate::attributes;
use crate::gtin;
use crate::model::Record;

/// Normalised GTIN-14, when the record's gtin is valid.
pub fn gtin_of(rec: &Record) -> Option<String> {
    rec.value("gtin").and_then(gtin::to_gtin14)
}

fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// brand | mpn | option values (canonical where parseable), when brand and
/// mpn are both present.
pub fn brand_mpn_of(rec: &Record) -> Option<String> {
    let brand = rec.value("brand")?;
    let mpn = rec.value("mpn")?;
    let mut opts: Vec<String> = rec
        .options
        .iter()
        .map(|(n, v)| {
            let value = match attributes::Attr::from_option_name(n) {
                Some(a) => attributes::parse_value(a, v)
                    .map(|(c, _)| c)
                    .unwrap_or_else(|| norm(v)),
                None => norm(v),
            };
            format!("{}={}", norm(n), value)
        })
        .collect();
    opts.sort();
    Some(format!("{}|{}|{}", norm(brand), norm(mpn), opts.join(";")))
}

/// brand and mpn as written, for evidence.
pub fn brand_mpn_text(rec: &Record) -> Option<(String, String)> {
    Some((
        rec.value("brand")?.to_string(),
        rec.value("mpn")?.to_string(),
    ))
}

/// The product type: product_type, else product_category.
pub fn product_type_of(rec: &Record) -> Option<String> {
    rec.value("product_type")
        .or_else(|| rec.value("product_category"))
        .map(str::to_string)
}

/// Keys of one feed, with only unique keys kept.
pub struct FeedKeys {
    pub gtin: Vec<Option<String>>,
    pub brand_mpn: Vec<Option<String>>,
    pub by_gtin: HashMap<String, usize>,
    pub by_brand_mpn: HashMap<String, usize>,
    pub by_id: HashMap<String, usize>,
}

impl FeedKeys {
    pub fn build(recs: &[Record]) -> Self {
        let raw_g: Vec<Option<String>> = recs.iter().map(gtin_of).collect();
        let raw_m: Vec<Option<String>> = recs.iter().map(brand_mpn_of).collect();
        let mut gc: HashMap<&str, usize> = HashMap::new();
        for g in raw_g.iter().flatten() {
            *gc.entry(g.as_str()).or_default() += 1;
        }
        let mut mc: HashMap<&str, usize> = HashMap::new();
        for m in raw_m.iter().flatten() {
            *mc.entry(m.as_str()).or_default() += 1;
        }
        // An invalid or duplicated GTIN never counts as a key.
        let gtin: Vec<Option<String>> = raw_g
            .iter()
            .map(|g| {
                g.as_ref()
                    .filter(|g| gc.get(g.as_str()) == Some(&1))
                    .cloned()
            })
            .collect();
        let brand_mpn: Vec<Option<String>> = raw_m
            .iter()
            .map(|m| {
                m.as_ref()
                    .filter(|m| mc.get(m.as_str()) == Some(&1))
                    .cloned()
            })
            .collect();
        let mut by_gtin = HashMap::new();
        let mut by_brand_mpn = HashMap::new();
        let mut by_id = HashMap::new();
        for (i, r) in recs.iter().enumerate() {
            if let Some(g) = &gtin[i] {
                by_gtin.insert(g.clone(), i);
            }
            if let Some(m) = &brand_mpn[i] {
                by_brand_mpn.insert(m.clone(), i);
            }
            if let Some(id) = r.value("item_id") {
                by_id.entry(id.to_string()).or_insert(i);
            }
        }
        FeedKeys {
            gtin,
            brand_mpn,
            by_gtin,
            by_brand_mpn,
            by_id,
        }
    }
}

/// Jaro-Winkler similarity of two titles (0 to 1), as text with two
/// decimals. A hint only.
pub fn title_similarity(a: Option<&str>, b: Option<&str>) -> Option<String> {
    let (a, b) = (a?, b?);
    let s = strsim::jaro_winkler(&norm(a), &norm(b));
    // Round to two decimals without float formatting surprises.
    let hundredths = (s * 100.0).round() as i64;
    Some(format!("{}.{:02}", hundredths / 100, hundredths % 100))
}
