//! `shelfcheck diff`: identity changes between two feeds.

pub mod changes;
pub mod decisions;
pub mod keys;
#[path = "match.rs"]
pub mod matching;

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::Location;
use crate::read::{self, Input, ReadOptions};
use decisions::Decisions;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    // Report order.
    IdReused,
    CriticalAttributeChanged,
    IdChanged,
    StoreStale,
    GroupChanged,
    OfferIdHasPrice,
    DuplicateId,
    New,
    Retired,
    PriceOnly,
    StockOnly,
}

impl ChangeKind {
    pub const ALL: [ChangeKind; 11] = [
        ChangeKind::IdReused,
        ChangeKind::CriticalAttributeChanged,
        ChangeKind::IdChanged,
        ChangeKind::StoreStale,
        ChangeKind::GroupChanged,
        ChangeKind::OfferIdHasPrice,
        ChangeKind::DuplicateId,
        ChangeKind::New,
        ChangeKind::Retired,
        ChangeKind::PriceOnly,
        ChangeKind::StockOnly,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ChangeKind::IdReused => "id_reused",
            ChangeKind::CriticalAttributeChanged => "critical_attribute_changed",
            ChangeKind::IdChanged => "id_changed",
            ChangeKind::StoreStale => "store_stale",
            ChangeKind::GroupChanged => "group_changed",
            ChangeKind::OfferIdHasPrice => "offer_id_has_price",
            ChangeKind::DuplicateId => "duplicate_id",
            ChangeKind::New => "new",
            ChangeKind::Retired => "retired",
            ChangeKind::PriceOnly => "price_only",
            ChangeKind::StockOnly => "stock_only",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }

    pub fn level(self) -> Level {
        match self {
            ChangeKind::IdReused | ChangeKind::CriticalAttributeChanged | ChangeKind::IdChanged => {
                Level::Decision
            }
            ChangeKind::StoreStale
            | ChangeKind::GroupChanged
            | ChangeKind::OfferIdHasPrice
            | ChangeKind::DuplicateId => Level::Warning,
            ChangeKind::New
            | ChangeKind::Retired
            | ChangeKind::PriceOnly
            | ChangeKind::StockOnly => Level::Info,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Needs a decision from a person.
    Decision,
    Warning,
    Info,
}

impl Level {
    pub fn prefix(self) -> &'static str {
        match self {
            Level::Decision => "D",
            Level::Warning => "W",
            Level::Info => "I",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    Decided,
}

/// The fields a reviewer needs from one side of a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gtin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<(String, String)>,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttrChange {
    pub attribute: String,
    /// The option name as written, or the attribute when read from the title.
    pub name: String,
    pub old: String,
    pub new: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRef {
    pub id: String,
    pub verdict: decisions::Verdict,
    pub by: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// D1, W1, I1...: decisions, warnings, information.
    pub id: String,
    pub kind: ChangeKind,
    pub level: Level,
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_item_id: Option<String>,
    pub fingerprint: String,
    pub sentence: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<AttrChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<matching::Method>,
    /// Jaro-Winkler title similarity, two decimals. A hint only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_similarity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<Snapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<Snapshot>,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<DecisionRef>,
    /// The exact `shelfcheck decide` commands for this change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchStats {
    pub old_items: usize,
    pub new_items: usize,
    pub in_both: usize,
    pub by_gtin: usize,
    pub by_brand_mpn: usize,
    pub by_id: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummary {
    /// Changes that need a decision and have none yet.
    pub needs_decision: usize,
    pub decided: usize,
    pub warnings: usize,
    pub info: usize,
    pub by_kind: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffResults {
    pub old: Input,
    pub new: Input,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<Input>,
    pub decisions_file: String,
    pub stats: MatchStats,
    pub summary: DiffSummary,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, Default)]
pub struct DiffOptions {
    pub read: ReadOptions,
    /// Extra flags every suggested `shelfcheck decide` command needs, such as
    /// `--results out` when results are not in the default folder.
    pub decide_flags: String,
}

/// Run `diff` on files.
pub fn run(
    old: &Path,
    new: &Path,
    store: Option<&Path>,
    decisions_path: &Path,
    opts: &DiffOptions,
) -> Result<DiffResults> {
    let (old_input, old_recs) = read::read_all(old, 0, &opts.read)?;
    let (new_input, new_recs) = read::read_all(new, 1, &opts.read)?;
    let (store_input, store_recs) = match store {
        Some(p) => {
            let (i, r) = read::read_all(p, 2, &opts.read)?;
            (Some(i), Some(r))
        }
        None => (None, None),
    };
    let decisions = Decisions::load(decisions_path)?;
    let files = changes::Files {
        old: old_input.records_path.clone(),
        new: new_input.records_path.clone(),
        store: store_input.as_ref().map(|i| i.records_path.clone()),
    };
    let (stats, list) = changes::compute(&old_recs, &new_recs, store_recs.as_deref(), &files);
    let list = changes::finish(list, &decisions, &opts.decide_flags);
    let summary = summarize(&list);
    Ok(DiffResults {
        old: old_input,
        new: new_input,
        store: store_input,
        decisions_file: decisions_path.to_string_lossy().replace('\\', "/"),
        stats,
        summary,
        changes: list,
    })
}

pub fn summarize(list: &[Change]) -> DiffSummary {
    let mut s = DiffSummary::default();
    for c in list {
        *s.by_kind.entry(c.kind.id().to_string()).or_default() += 1;
        match (c.level, c.status) {
            (Level::Decision, Status::Open) => s.needs_decision += 1,
            (_, Status::Decided) => s.decided += 1,
            (Level::Warning, Status::Open) => s.warnings += 1,
            (Level::Info, Status::Open) => s.info += 1,
        }
    }
    s
}
