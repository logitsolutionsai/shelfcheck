//! Results: every problem and change, with source locations. This is the
//! content of `results.json`; the schema is in docs/results-schema.json.
//!
//! Nothing here holds a timestamp, so the same input always gives the same
//! file.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::diff::DiffResults;
use crate::model::{Location, Severity, TargetFormat};
use crate::read::Input;
use crate::rules::{AppliesTo, Link, RuleSet};

pub const TOOL: &str = "shelfcheck";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Problem {
    pub rule: String,
    pub pack: String,
    /// The fix group this problem belongs to.
    pub fix: String,
    pub severity: Severity,
    /// The formats this problem counts against.
    pub formats: Vec<TargetFormat>,
    pub item_id: String,
    /// The product (group id, or the item id).
    pub product: String,
    /// Index into `inputs`.
    pub feed: usize,
    pub location: Location,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The value as written (absent when missing or redacted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    pub message: String,
    /// For cross-format problems: the cell in the other feed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related: Option<Location>,
}

impl Problem {
    /// Stable sort key: feed, line, column, pointer, rule.
    #[allow(clippy::type_complexity)]
    pub fn sort_key(&self) -> (usize, String, u64, u32, String, String) {
        (
            self.feed,
            self.location.file.clone(),
            self.location.line,
            self.location.column.unwrap_or(0),
            self.location.pointer.clone().unwrap_or_default(),
            self.rule.clone(),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
    pub short: String,
    pub applies_to: AppliesTo,
    pub source: String,
    pub version: String,
    pub checked: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub origin: String,
    pub rules: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Readiness {
    pub format: TargetFormat,
    pub label: String,
    /// Products checked for this format.
    pub products: usize,
    /// Products with no errors for this format. Warnings never block.
    pub ready: usize,
    pub with_errors: usize,
    pub with_warnings: usize,
    /// Ready share, one decimal, as text ("87.9").
    pub percent_ready: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<String>,
    pub rules_as_of: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FixStatus {
    Error,
    Warning,
    Passes,
    NotUsed,
    NotChecked,
}

impl FixStatus {
    pub fn label(self) -> &'static str {
        match self {
            FixStatus::Error => "Error",
            FixStatus::Warning => "Warning",
            FixStatus::Passes => "passes",
            FixStatus::NotUsed => "not used",
            FixStatus::NotChecked => "not checked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FixFormat {
    pub format: TargetFormat,
    pub status: FixStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fix {
    pub rank: usize,
    pub key: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The worst severity in the group.
    pub severity: Severity,
    /// Distinct products affected.
    pub products: usize,
    pub problems: usize,
    pub formats: Vec<FixFormat>,
    /// Cross-format fixes have no per-format status.
    pub cross: bool,
    pub rules: Vec<String>,
    pub cites: Vec<String>,
    pub links: Vec<Link>,
    pub example: Problem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotChecked {
    pub id: String,
    pub title: String,
    pub says: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IgnoredRule {
    pub id: String,
    pub pack: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckSummary {
    /// Distinct products across every feed.
    pub products: usize,
    /// Records (items or variants) across every feed.
    pub items: u64,
    /// Products with at least one error in any format.
    pub products_needing_fixes: usize,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckResults {
    pub inputs: Vec<Input>,
    pub formats: Vec<TargetFormat>,
    pub packs: Vec<PackInfo>,
    pub summary: CheckSummary,
    pub readiness: Vec<Readiness>,
    pub fixes: Vec<Fix>,
    pub problems: Vec<Problem>,
    pub ignored: Vec<IgnoredRule>,
    pub not_checked: Vec<NotChecked>,
}

/// The whole of results.json. A `check` and a `diff` run into the same
/// folder are kept together when the diff's new feed is a checked feed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultsFile {
    pub tool: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<CheckResults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<DiffResults>,
}

impl ResultsFile {
    pub fn new() -> Self {
        ResultsFile {
            tool: TOOL.to_string(),
            version: VERSION.to_string(),
            check: None,
            diff: None,
        }
    }
}

impl Default for ResultsFile {
    fn default() -> Self {
        Self::new()
    }
}

pub fn pack_infos(rules: &RuleSet) -> Vec<PackInfo> {
    rules
        .packs
        .iter()
        .map(|p| PackInfo {
            id: p.meta.id.clone(),
            name: p.meta.name.clone(),
            short: p.meta.short.clone(),
            applies_to: p.meta.applies_to,
            source: p.meta.source.clone(),
            version: p.meta.version.clone(),
            checked: p.meta.checked.clone(),
            label: p.meta.label.clone(),
            origin: p.origin.clone(),
            rules: p.rules.len(),
            notes: p.meta.notes.clone(),
        })
        .collect()
}

/// One decimal, as text: 1231 of 1400 -> "87.9".
pub fn percent(n: usize, d: usize) -> String {
    if d == 0 {
        return "0.0".to_string();
    }
    // Integer arithmetic, rounded half up, so output never depends on floats.
    let tenths = (n as u128 * 1000 + d as u128 / 2) / d as u128;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Group problems into fixes, ranked by products affected.
pub fn build_fixes(problems: &[Problem], rules: &RuleSet, formats: &[TargetFormat]) -> Vec<Fix> {
    let mut groups: BTreeMap<String, Vec<&Problem>> = BTreeMap::new();
    for p in problems {
        groups.entry(p.fix.clone()).or_default().push(p);
    }
    let mut fixes: Vec<Fix> = Vec::new();
    for (key, ps) in groups {
        let Some(first) = ps.first() else { continue };
        let products: BTreeSet<&str> = ps.iter().map(|p| p.product.as_str()).collect();
        let severity = ps
            .iter()
            .map(|p| p.severity)
            .min()
            .unwrap_or(Severity::Info);
        let rule_ids: BTreeSet<&str> = ps.iter().map(|p| p.rule.as_str()).collect();
        // Title and detail: a pack [[fix]] entry, else the first rule in pack order.
        let mut title = None;
        let mut detail = None;
        let mut cites = Vec::new();
        let mut links: Vec<Link> = Vec::new();
        let mut field = first.field.clone();
        for pack in &rules.packs {
            for r in &pack.rules {
                if rule_ids.contains(r.def.id.as_str()) {
                    if title.is_none() {
                        title = Some(r.title.clone());
                        detail = r.def.detail.clone();
                        if field.is_none() {
                            field = r.def.field.clone();
                        }
                    }
                    if !cites.contains(&r.def.cites) {
                        cites.push(r.def.cites.clone());
                    }
                    for l in &r.def.links {
                        if !links.iter().any(|x| x.url == l.url && x.label == l.label) {
                            links.push(l.clone());
                        }
                    }
                }
            }
        }
        if let Some(fd) = rules.fix_def(&key) {
            title = Some(fd.title.clone());
            if fd.detail.is_some() {
                detail = fd.detail.clone();
            }
        } else if let Some(rest) = key
            .strip_prefix("acp.schema:")
            .or_else(|| key.strip_prefix("acp.metadata:"))
        {
            // Schema fixes are grouped by keyword and path: "type:/variants/*/price/amount".
            let (keyword, path) = rest.split_once(':').unwrap_or((rest, ""));
            let path = if path.is_empty() { "/" } else { path };
            let what = if key.starts_with("acp.metadata:") {
                "ACP metadata.json"
            } else {
                "ACP schema"
            };
            title = Some(format!("{what}: {path} fails the {keyword} check"));
        }
        let title = title.unwrap_or_else(|| first.message.clone());
        let cross = ps.iter().all(|p| p.pack == "cross-format");
        let mut fformats = Vec::new();
        for f in TargetFormat::ALL {
            let status = if !formats.contains(&f) {
                FixStatus::NotChecked
            } else {
                let worst = ps
                    .iter()
                    .filter(|p| p.formats.contains(&f))
                    .map(|p| p.severity)
                    .min();
                match worst {
                    Some(Severity::Error) => FixStatus::Error,
                    Some(Severity::Warning) | Some(Severity::Info) => FixStatus::Warning,
                    None => {
                        let used = rules.pack_for(f).map(|pack| {
                            pack.rules.iter().any(|r| {
                                r.fix_key == key || (field.is_some() && r.def.field == field)
                            })
                        });
                        let all_formats_pack = ps.iter().any(|p| {
                            rules.packs.iter().any(|pk| {
                                pk.meta.id == p.pack && pk.meta.applies_to == AppliesTo::All
                            })
                        });
                        if used.unwrap_or(false) || all_formats_pack {
                            FixStatus::Passes
                        } else {
                            FixStatus::NotUsed
                        }
                    }
                }
            };
            fformats.push(FixFormat { format: f, status });
        }
        fixes.push(Fix {
            rank: 0,
            key,
            title,
            detail,
            severity,
            products: products.len(),
            problems: ps.len(),
            formats: fformats,
            cross,
            rules: rule_ids.into_iter().map(str::to_string).collect(),
            cites,
            links,
            example: (*first).clone(),
        });
    }
    fixes.sort_by(|a, b| {
        b.products
            .cmp(&a.products)
            .then_with(|| a.severity.cmp(&b.severity))
            .then_with(|| a.key.cmp(&b.key))
    });
    for (i, f) in fixes.iter_mut().enumerate() {
        f.rank = i + 1;
    }
    fixes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_one_decimal() {
        assert_eq!(percent(1231, 1400), "87.9");
        assert_eq!(percent(1302, 1400), "93.0");
        assert_eq!(percent(0, 0), "0.0");
        assert_eq!(percent(1, 3), "33.3");
        assert_eq!(percent(2, 3), "66.7");
    }
}
