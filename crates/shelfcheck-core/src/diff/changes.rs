//! Change rules (section 8): what changed between matched records, and
//! which changes need a person to decide.

use std::collections::{BTreeMap, HashMap};

use super::decisions::Decisions;
use super::keys::{brand_mpn_text, gtin_of, product_type_of, title_similarity};
use super::matching::{Method, Reuse, match_feeds};
use super::{AttrChange, Change, ChangeKind, DecisionRef, MatchStats, Snapshot, Status};
use crate::attributes::{self, Attr, Attributes};
use crate::model::{Availability, Record};
use crate::price::parse_price;
use rust_decimal::Decimal;

/// The record files, for locations.
pub struct Files {
    pub old: String,
    pub new: String,
    pub store: Option<String>,
}

pub fn snapshot(rec: &Record, file: &str) -> Snapshot {
    let v = |n: &str| rec.value(n).map(str::to_string);
    Snapshot {
        item_id: rec.item_id(),
        title: v("title"),
        gtin: v("gtin"),
        brand: v("brand"),
        mpn: v("mpn"),
        product_type: product_type_of(rec),
        group_id: v("group_id"),
        price: v("price"),
        availability: v("availability"),
        options: rec.options.clone(),
        location: rec.location_for(file, None),
    }
}

fn attrs(rec: &Record) -> Attributes {
    attributes::parse(&rec.options, rec.value("title"))
}

fn blank(kind: ChangeKind, item_id: String, fingerprint: String, sentence: String) -> Change {
    Change {
        id: String::new(),
        kind,
        level: kind.level(),
        item_id,
        new_item_id: None,
        fingerprint,
        sentence,
        evidence: Vec::new(),
        attributes: Vec::new(),
        matched_by: None,
        title_similarity: None,
        old: None,
        new: None,
        status: Status::Open,
        decision: None,
        commands: Vec::new(),
    }
}

/// Keep the first record of each id; report ids that appear more than once.
fn dedupe(recs: &[Record]) -> (Vec<&Record>, BTreeMap<String, Vec<u64>>) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut lines: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut out = Vec::new();
    for r in recs {
        match r.value("item_id") {
            Some(id) => {
                lines.entry(id.to_string()).or_default().push(r.line);
                if seen.contains_key(id) {
                    continue;
                }
                seen.insert(id.to_string(), out.len());
                out.push(r);
            }
            None => out.push(r),
        }
    }
    lines.retain(|_, v| v.len() > 1);
    (out, lines)
}

/// The fields that make "only price changed" true when they are equal.
fn same_content(o: &Record, n: &Record) -> bool {
    for f in [
        "title",
        "brand",
        "mpn",
        "url",
        "image_url",
        "group_id",
        "product_type",
        "product_category",
        "description",
    ] {
        if o.value(f) != n.value(f) {
            return false;
        }
    }
    if gtin_of(o) != gtin_of(n) {
        return false;
    }
    let (ao, an) = (attrs(o), attrs(n));
    let canon = |a: &Attributes| {
        a.iter()
            .map(|(k, v)| (*k, v.canonical.clone()))
            .collect::<Vec<_>>()
    };
    canon(&ao) == canon(&an)
}

fn price_norm(v: Option<&str>) -> Option<String> {
    v.map(|s| {
        parse_price(s)
            .map(|p| p.canonical())
            .unwrap_or_else(|_| s.to_string())
    })
}

fn avail_norm(v: Option<&str>) -> Option<String> {
    v.map(|s| {
        Availability::parse(s)
            .map(|a| format!("{a:?}"))
            .unwrap_or_else(|| s.to_string())
    })
}

const UNIT_NOTE: &str = "Compared in decimal units (1 GB = 1,000 MB). Some sellers mean binary units (1 GiB = 1,024 MiB), so check what the store means.";

fn attr_changes(o: &Record, n: &Record) -> Vec<AttrChange> {
    let (ao, an) = (attrs(o), attrs(n));
    let mut out = Vec::new();
    for (attr, ov) in &ao {
        let Some(nv) = an.get(attr) else { continue };
        if ov.canonical != nv.canonical {
            let note = match (attr, &ov.unit, &nv.unit) {
                (Attr::Capacity, Some(a), Some(b)) if a != b => Some(UNIT_NOTE.to_string()),
                _ => None,
            };
            out.push(AttrChange {
                attribute: attr.id().to_string(),
                name: nv.name.clone(),
                old: ov.text.clone(),
                new: nv.text.clone(),
                note,
            });
        }
    }
    out
}

fn store_lookup<'a>(store: &'a [Record], id: &str, gtin: Option<&str>) -> Option<&'a Record> {
    store
        .iter()
        .find(|r| r.value("item_id") == Some(id))
        .or_else(|| gtin.and_then(|g| store.iter().find(|r| gtin_of(r).as_deref() == Some(g))))
}

/// Does the store row still describe the old value of a changed attribute?
/// Returns (where, text) for each stale place: "title" or the option name.
fn stale_places(
    row: &Record,
    attr: Attr,
    old_canonical: &str,
    new_canonical: &str,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, value) in &row.options {
        if Attr::from_option_name(name) == Some(attr) {
            if let Some((c, _)) = attributes::parse_value(attr, value) {
                if c == old_canonical && c != new_canonical {
                    out.push((format!("option {name}"), value.clone()));
                }
            }
        }
    }
    if let Some(title) = row.value("title") {
        let from_title = attributes::parse(&[], Some(title));
        if let Some(v) = from_title.get(&attr) {
            if v.canonical == old_canonical && v.canonical != new_canonical {
                out.push(("title".to_string(), title.to_string()));
            }
        }
    }
    out
}

fn reuse_change(o: &Record, n: &Record, ev: &Reuse, files: &Files) -> Change {
    let id = n.item_id();
    let fingerprint = if let Some((a, b)) = &ev.gtin {
        format!("gtin:{a}->{b}")
    } else if let Some(((ab, am), (bb, bm))) = &ev.brand_mpn {
        format!("mpn:{ab}/{am}->{bb}/{bm}")
    } else if let Some((a, b)) = &ev.product_type {
        format!("type:{a}->{b}")
    } else {
        "id".to_string()
    };
    let sentence = match &ev.product_type {
        Some((a, b)) => format!("{id} was {a} and is now {b}."),
        None => format!(
            "{id} now names a different product: \"{}\" before, \"{}\" now.",
            o.value("title").unwrap_or(""),
            n.value("title").unwrap_or("")
        ),
    };
    let mut c = blank(ChangeKind::IdReused, id, fingerprint, sentence);
    if let Some((a, b)) = &ev.gtin {
        c.evidence.push(format!("gtin changed from {a} to {b}"));
    }
    if let Some(((ab, am), (bb, bm))) = &ev.brand_mpn {
        c.evidence
            .push(format!("brand and MPN changed from {ab} {am} to {bb} {bm}"));
    }
    if let Some((a, b)) = &ev.product_type {
        c.evidence
            .push(format!("product type changed from {a} to {b}"));
    }
    c.title_similarity = title_similarity(o.value("title"), n.value("title"));
    if let Some(s) = &c.title_similarity {
        c.evidence.push(format!(
            "Title similarity is {s}, shown as a hint only: the barcode and MPN decide."
        ));
    }
    c.evidence
        .push("OpenAI says never to reuse an item_id for a different item.".to_string());
    c.matched_by = Some(Method::Id);
    c.old = Some(snapshot(o, &files.old));
    c.new = Some(snapshot(n, &files.new));
    c
}

/// Every change between the two feeds, unsorted and without ids.
pub fn compute(
    old_all: &[Record],
    new_all: &[Record],
    store: Option<&[Record]>,
    files: &Files,
) -> (MatchStats, Vec<Change>) {
    let (old_d, _) = dedupe(old_all);
    let (new_d, new_dupes) = dedupe(new_all);
    let old: Vec<Record> = old_d.into_iter().cloned().collect();
    let new: Vec<Record> = new_d.into_iter().cloned().collect();
    let m = match_feeds(&old, &new);
    let mut out = Vec::new();
    let mut stats = MatchStats {
        old_items: old.len(),
        new_items: new.len(),
        ..MatchStats::default()
    };

    for (oi, ni, ev) in &m.reused {
        out.push(reuse_change(&old[*oi], &new[*ni], ev, files));
    }

    for &(oi, ni, method) in &m.pairs {
        let (o, n) = (&old[oi], &new[ni]);
        stats.in_both += 1;
        match method {
            Method::Gtin => stats.by_gtin += 1,
            Method::BrandMpn => stats.by_brand_mpn += 1,
            Method::Id => stats.by_id += 1,
        }
        let (oid, nid) = (o.value("item_id"), n.value("item_id"));
        if let (Some(a), Some(b)) = (oid, nid) {
            if a != b {
                let (what, evidence) = match method {
                    Method::Gtin => (
                        "barcode",
                        format!("Same gtin {}", n.value("gtin").unwrap_or_default()),
                    ),
                    _ => (
                        "brand, MPN and options",
                        format!(
                            "Same brand, MPN and options ({}, {})",
                            n.value("brand").unwrap_or_default(),
                            n.value("mpn").unwrap_or_default()
                        ),
                    ),
                };
                let mut c = blank(
                    ChangeKind::IdChanged,
                    a.to_string(),
                    format!("id:{a}->{b}"),
                    format!("{a} now has a new id, {b}; the {what} did not change."),
                );
                c.new_item_id = Some(b.to_string());
                c.evidence.push(evidence);
                c.evidence.push(
                    "OpenAI asks you to keep item_id stable when other fields change.".to_string(),
                );
                c.matched_by = Some(method);
                c.old = Some(snapshot(o, &files.old));
                c.new = Some(snapshot(n, &files.new));
                out.push(c);
                continue;
            }
        }
        let id = n.item_id();

        // Critical attributes: same id and same GTIN. With no valid GTIN on
        // either side, the identity fallback (key 2) stands in: the same id
        // with the same brand and MPN, whose option values changed.
        let same_gtin = gtin_of(o).is_some() && gtin_of(o) == gtin_of(n);
        let same_brand_mpn = gtin_of(o).is_none()
            && gtin_of(n).is_none()
            && match (brand_mpn_text(o), brand_mpn_text(n)) {
                (Some(a), Some(b)) => {
                    a.0.trim().eq_ignore_ascii_case(b.0.trim())
                        && a.1.trim().eq_ignore_ascii_case(b.1.trim())
                }
                _ => false,
            };
        if same_gtin || same_brand_mpn {
            let changes = attr_changes(o, n);
            if !changes.is_empty() {
                let fingerprint = changes
                    .iter()
                    .map(|a| format!("{}:{}->{}", a.attribute, a.old, a.new))
                    .collect::<Vec<_>>()
                    .join(";");
                let described = changes
                    .iter()
                    .map(|a| format!("{} changed from {} to {}", a.name, a.old, a.new))
                    .collect::<Vec<_>>()
                    .join(", and its ");
                let mut c = blank(
                    ChangeKind::CriticalAttributeChanged,
                    id.clone(),
                    fingerprint,
                    if same_gtin {
                        format!("{id} kept its id and barcode, but its {described}.")
                    } else {
                        format!("{id} kept its id, brand and MPN, but its {described}.")
                    },
                );
                for a in &changes {
                    c.evidence.push(if same_gtin {
                        format!(
                            "{} {} -> {}, gtin {} both times",
                            a.name,
                            a.old,
                            a.new,
                            n.value("gtin").unwrap_or_default()
                        )
                    } else {
                        format!(
                            "{} {} -> {}, brand {} and MPN {} both times, no barcode",
                            a.name,
                            a.old,
                            a.new,
                            n.value("brand").unwrap_or_default(),
                            n.value("mpn").unwrap_or_default()
                        )
                    });
                    if let Some(note) = &a.note {
                        c.evidence.push(note.clone());
                    }
                }
                c.evidence.push(if same_gtin {
                    "Either the new feed is wrong, or the id and barcode now belong to a different variant.".to_string()
                } else {
                    "Either the new feed is wrong, or the id now belongs to a different variant.".to_string()
                });
                c.attributes = changes.clone();
                c.matched_by = Some(method);
                c.old = Some(snapshot(o, &files.old));
                c.new = Some(snapshot(n, &files.new));

                if let (Some(store), Some(store_file)) = (store, files.store.as_deref()) {
                    let gt = gtin_of(n);
                    if let Some(row) = store_lookup(store, &id, gt.as_deref()) {
                        let (ao, an) = (attrs(o), attrs(n));
                        for a in &changes {
                            let Some(attr) = attributes_of(&a.attribute) else {
                                continue;
                            };
                            let (Some(ov), Some(nv)) = (ao.get(&attr), an.get(&attr)) else {
                                continue;
                            };
                            for (place, text) in
                                stale_places(row, attr, &ov.canonical, &nv.canonical)
                            {
                                c.evidence.push(format!(
                                    "Store export: {place} still reads \"{text}\" (stale)"
                                ));
                                let mut s = blank(
                                    ChangeKind::StoreStale,
                                    id.clone(),
                                    format!("{place}:{}:{}", a.attribute, a.old),
                                    format!(
                                        "The store {place} for {id} still says {}: \"{text}\".",
                                        a.old
                                    ),
                                );
                                s.evidence
                                    .push(format!("The new feed says {} {}.", a.name, a.new));
                                s.new = Some(snapshot(row, store_file));
                                out.push(s);
                            }
                        }
                    }
                }
                out.push(c);
            }
        }

        // Group moves.
        if let (Some(a), Some(b)) = (o.value("group_id"), n.value("group_id")) {
            if a != b {
                let mut c = blank(
                    ChangeKind::GroupChanged,
                    id.clone(),
                    format!("group:{a}->{b}"),
                    format!("{id} moved from group {a} to group {b}."),
                );
                c.evidence
                    .push("OpenAI asks you to keep group_id stable.".to_string());
                c.old = Some(snapshot(o, &files.old));
                c.new = Some(snapshot(n, &files.new));
                out.push(c);
            }
        }

        // Price and stock only.
        if same_content(o, n) {
            let (po, pn) = (price_norm(o.value("price")), price_norm(n.value("price")));
            if po != pn {
                let (a, b) = (
                    o.value("price").unwrap_or(""),
                    n.value("price").unwrap_or(""),
                );
                let mut c = blank(
                    ChangeKind::PriceOnly,
                    id.clone(),
                    format!("price:{a}->{b}"),
                    format!("{id}: price {a} to {b}."),
                );
                c.old = Some(snapshot(o, &files.old));
                c.new = Some(snapshot(n, &files.new));
                out.push(c);
            }
            let (ao, an) = (
                avail_norm(o.value("availability")),
                avail_norm(n.value("availability")),
            );
            if ao != an {
                let (a, b) = (
                    o.value("availability").unwrap_or(""),
                    n.value("availability").unwrap_or(""),
                );
                let mut c = blank(
                    ChangeKind::StockOnly,
                    id.clone(),
                    format!("availability:{a}->{b}"),
                    format!("{id}: availability {a} to {b}."),
                );
                c.old = Some(snapshot(o, &files.old));
                c.new = Some(snapshot(n, &files.new));
                out.push(c);
            }
        }
    }

    // Offer ids that contain the price (new feed).
    for n in &new {
        if let Some(c) = offer_has_price(n, &files.new) {
            out.push(c);
        }
    }

    // Duplicate ids in the new feed.
    for (id, lines) in &new_dupes {
        let mut c = blank(
            ChangeKind::DuplicateId,
            id.clone(),
            format!("rows:{}", lines.len()),
            format!(
                "{id} appears on {} rows of the new feed; the first is used.",
                lines.len()
            ),
        );
        c.evidence.push(format!(
            "Lines {}",
            lines
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push(c);
    }

    // New and retired.
    for (ni, n) in new.iter().enumerate() {
        if !m.new_used[ni] {
            let id = n.item_id();
            let mut c = blank(
                ChangeKind::New,
                id.clone(),
                "new".to_string(),
                format!("{id} is new."),
            );
            c.new = Some(snapshot(n, &files.new));
            out.push(c);
        }
    }
    for (oi, o) in old.iter().enumerate() {
        if !m.old_used[oi] {
            let id = o.item_id();
            let mut c = blank(
                ChangeKind::Retired,
                id.clone(),
                "retired".to_string(),
                format!("{id} is no longer in the feed."),
            );
            c.old = Some(snapshot(o, &files.old));
            out.push(c);
        }
    }
    (stats, out)
}

fn attributes_of(id: &str) -> Option<Attr> {
    [
        Attr::Capacity,
        Attr::Size,
        Attr::Colour,
        Attr::Material,
        Attr::PackQuantity,
    ]
    .into_iter()
    .find(|a| a.id() == id)
}

/// An offer id that contains the item's price, as 49.99 or 4999.
fn offer_has_price(n: &Record, file: &str) -> Option<Change> {
    let offer = n.value("offer_id")?;
    let price = parse_price(n.value("price")?).ok()?;
    let plain = price.amount.normalize().to_string();
    // The amount in hundredths, as digits: 49.99 -> "4999".
    let digits = price
        .amount
        .checked_mul(Decimal::from(100i64))
        .map(|d| d.trunc().normalize().to_string())
        .unwrap_or_default();
    let tokens: Vec<&str> = offer
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .collect();
    let hit = tokens
        .iter()
        .any(|t| (digits.len() >= 3 && *t == digits) || (plain.contains('.') && *t == plain));
    if !hit {
        return None;
    }
    let id = n.item_id();
    let mut c = blank(
        ChangeKind::OfferIdHasPrice,
        id.clone(),
        format!("offer:{offer}"),
        format!(
            "The offer id {offer} contains the price {}.",
            n.value("price").unwrap_or_default()
        ),
    );
    c.evidence.push(
        "OpenAI says never to put a price in an offer id: it changes whenever the price does."
            .to_string(),
    );
    c.new = Some(snapshot(n, file));
    Some(c)
}

/// Sort, number, apply decisions and add the decide commands.
pub fn finish(mut list: Vec<Change>, decisions: &Decisions, decide_flags: &str) -> Vec<Change> {
    list.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.item_id.cmp(&b.item_id))
            .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });
    let mut counters: BTreeMap<&'static str, usize> = BTreeMap::new();
    for c in list.iter_mut() {
        let prefix = c.level.prefix();
        let n = counters.entry(prefix).or_default();
        *n += 1;
        c.id = format!("{prefix}{n}");
        if let Some(d) = decisions.find(c.kind.id(), &c.item_id, &c.fingerprint) {
            c.status = Status::Decided;
            c.decision = Some(DecisionRef {
                id: d.id.clone(),
                verdict: d.verdict,
                by: d.by.clone(),
                at: d.at.clone(),
            });
        }
        let flags: &[&str] = match c.kind {
            ChangeKind::IdReused => &["--different", "--same"],
            ChangeKind::CriticalAttributeChanged => &["--fixed", "--different"],
            ChangeKind::IdChanged => &["--same", "--different"],
            ChangeKind::StoreStale | ChangeKind::OfferIdHasPrice | ChangeKind::DuplicateId => {
                &["--fixed"]
            }
            ChangeKind::GroupChanged => &["--same"],
            ChangeKind::New
            | ChangeKind::Retired
            | ChangeKind::PriceOnly
            | ChangeKind::StockOnly => &[],
        };
        c.commands = flags
            .iter()
            .map(|f| format!("shelfcheck decide {} {f}{decide_flags}", c.id))
            .collect();
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Family, Field};
    use std::sync::Arc;

    fn rec(line: u64, pairs: &[(&str, &str)]) -> Record {
        let mut r = Record::new(0, Family::Openai, line);
        for (k, v) in pairs {
            r.set(Arc::from(*k), Field::new(*v, Arc::from(*k), line));
        }
        crate::map::extract_options(&mut r);
        r
    }

    fn files() -> Files {
        Files {
            old: "old.csv".into(),
            new: "new.csv".into(),
            store: Some("store.csv".into()),
        }
    }

    fn kinds(list: &[Change]) -> Vec<(String, String, String)> {
        let mut v: Vec<_> = list
            .iter()
            .map(|c| {
                (
                    c.kind.id().to_string(),
                    c.item_id.clone(),
                    c.fingerprint.clone(),
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn the_three_walkthrough_changes() {
        let old = vec![
            rec(
                2,
                &[
                    ("item_id", "PHN-X2-BLK"),
                    ("title", "Nova X2 Phone, 128 GB, Black"),
                    ("gtin", "0194253012344"),
                    ("brand", "Nova"),
                    ("mpn", "NX2-128-BLK"),
                    ("product_category", "Phones"),
                ],
            ),
            rec(
                3,
                &[
                    ("item_id", "TAB-S-128"),
                    ("title", "Slate Tab S, 128 GB, Grey"),
                    ("gtin", "5012345678900"),
                    ("variant_dict", r#"{"storage":"128 GB","color":"Grey"}"#),
                ],
            ),
            rec(
                4,
                &[
                    ("item_id", "MUG-12-WHT"),
                    ("title", "Harbor Mug 12 oz, White"),
                    ("gtin", "0012345678905"),
                    ("brand", "Harbor & Co"),
                    ("mpn", "HC-MUG-12"),
                ],
            ),
        ];
        let new = vec![
            rec(
                2,
                &[
                    ("item_id", "PHN-X2-BLK"),
                    ("title", "Nova X2 Phone Case, Black"),
                    ("gtin", "0194253098768"),
                    ("brand", "Nova"),
                    ("mpn", "NX2-CASE-BLK"),
                    ("product_category", "Phone cases"),
                ],
            ),
            rec(
                3,
                &[
                    ("item_id", "TAB-S-128"),
                    ("title", "Slate Tab S, 256 GB, Grey"),
                    ("gtin", "5012345678900"),
                    ("variant_dict", r#"{"storage":"256 GB","color":"Grey"}"#),
                ],
            ),
            rec(
                4,
                &[
                    ("item_id", "MUG-12OZ-WHT"),
                    ("title", "Harbor Mug 12 oz, White"),
                    ("gtin", "0012345678905"),
                    ("brand", "Harbor & Co"),
                    ("mpn", "HC-MUG-12"),
                ],
            ),
        ];
        let store = vec![rec(
            2,
            &[
                ("item_id", "TAB-S-128"),
                ("title", "Slate Tab S, 128 GB, Grey"),
                ("option1_name", "Storage"),
                ("option1_value", "256 GB"),
            ],
        )];
        let (stats, list) = compute(&old, &new, Some(&store), &files());
        assert_eq!(stats.by_gtin, 2);
        assert_eq!(
            kinds(&list),
            vec![
                (
                    "critical_attribute_changed".into(),
                    "TAB-S-128".into(),
                    "capacity:128 GB->256 GB".into()
                ),
                (
                    "id_changed".into(),
                    "MUG-12-WHT".into(),
                    "id:MUG-12-WHT->MUG-12OZ-WHT".into()
                ),
                (
                    "id_reused".into(),
                    "PHN-X2-BLK".into(),
                    "gtin:0194253012344->0194253098768".into()
                ),
                (
                    "store_stale".into(),
                    "TAB-S-128".into(),
                    "title:capacity:128 GB".into()
                ),
            ]
        );
        let done = finish(list, &Decisions::default(), "");
        let d1 = done.iter().find(|c| c.id == "D1").map(|c| c.kind);
        assert_eq!(d1, Some(ChangeKind::IdReused));
        assert!(done.iter().any(|c| {
            c.commands
                .contains(&"shelfcheck decide D1 --different".to_string())
        }));
    }

    #[test]
    fn aliases_and_units_are_not_changes() {
        let old = vec![
            rec(
                2,
                &[
                    ("item_id", "TEE-BLK-M"),
                    ("gtin", "0012345678912"),
                    ("variant_dict", r#"{"color":"Black","size":"M"}"#),
                ],
            ),
            rec(
                3,
                &[
                    ("item_id", "FRAME-30"),
                    ("gtin", "0012345678929"),
                    ("variant_dict", r#"{"size":"30 cm"}"#),
                ],
            ),
        ];
        let new = vec![
            rec(
                2,
                &[
                    ("item_id", "TEE-BLK-M"),
                    ("gtin", "0012345678912"),
                    ("variant_dict", r#"{"color":"BLK","size":"M"}"#),
                ],
            ),
            rec(
                3,
                &[
                    ("item_id", "FRAME-30"),
                    ("gtin", "0012345678929"),
                    ("variant_dict", r#"{"size":"300 mm"}"#),
                ],
            ),
        ];
        let (_, list) = compute(&old, &new, None, &files());
        assert!(list.is_empty(), "{list:?}");
    }

    #[test]
    fn offer_ids_with_prices() {
        let n = rec(
            2,
            &[
                ("item_id", "BAG-1"),
                ("offer_id", "BAG-1-4999"),
                ("price", "49.99 USD"),
            ],
        );
        assert!(offer_has_price(&n, "f").is_some());
        let ok = rec(
            2,
            &[
                ("item_id", "BAG-1"),
                ("offer_id", "BAG-1-BLUE"),
                ("price", "49.99 USD"),
            ],
        );
        assert!(offer_has_price(&ok, "f").is_none());
    }
}
