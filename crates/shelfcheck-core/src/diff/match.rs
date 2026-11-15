//! Matching old records to new ones, strongest key first.

use serde::{Deserialize, Serialize};

use super::keys::{FeedKeys, brand_mpn_text, gtin_of, product_type_of};
use crate::model::Record;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Gtin,
    BrandMpn,
    Id,
}

impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Method::Gtin => "gtin",
            Method::BrandMpn => "brand and mpn",
            Method::Id => "id only",
        }
    }
}

/// Evidence that two records with the same id are different products.
#[derive(Debug, Clone, Default)]
pub struct Reuse {
    pub gtin: Option<(String, String)>,
    pub brand_mpn: Option<((String, String), (String, String))>,
    pub product_type: Option<(String, String)>,
}

impl Reuse {
    pub fn any(&self) -> bool {
        self.gtin.is_some() || self.brand_mpn.is_some() || self.product_type.is_some()
    }
}

pub fn reuse_evidence(o: &Record, n: &Record) -> Reuse {
    let mut r = Reuse::default();
    if let (Some(a), Some(b)) = (gtin_of(o), gtin_of(n)) {
        if a != b {
            r.gtin = Some((
                o.value("gtin").unwrap_or_default().to_string(),
                n.value("gtin").unwrap_or_default().to_string(),
            ));
        }
    }
    if let (Some(a), Some(b)) = (brand_mpn_text(o), brand_mpn_text(n)) {
        if !a.0.eq_ignore_ascii_case(&b.0) || !a.1.eq_ignore_ascii_case(&b.1) {
            r.brand_mpn = Some((a, b));
        }
    }
    if let (Some(a), Some(b)) = (product_type_of(o), product_type_of(n)) {
        if !a.trim().eq_ignore_ascii_case(b.trim()) {
            r.product_type = Some((a, b));
        }
    }
    r
}

pub struct Matching {
    /// (old index, new index, method), in new-feed order.
    pub pairs: Vec<(usize, usize, Method)>,
    /// (old index, new index, evidence): same id, different product.
    pub reused: Vec<(usize, usize, Reuse)>,
    pub old_used: Vec<bool>,
    pub new_used: Vec<bool>,
}

#[allow(clippy::needless_range_loop)]
pub fn match_feeds(old: &[Record], new: &[Record]) -> Matching {
    let ok = FeedKeys::build(old);
    let nk = FeedKeys::build(new);
    let mut old_m: Vec<Option<(usize, Method)>> = vec![None; old.len()];
    let mut new_m: Vec<Option<(usize, Method)>> = vec![None; new.len()];

    // 1. GTIN.
    for ni in 0..new.len() {
        let Some(g) = &nk.gtin[ni] else { continue };
        if let Some(&oi) = ok.by_gtin.get(g) {
            if old_m[oi].is_none() {
                old_m[oi] = Some((ni, Method::Gtin));
                new_m[ni] = Some((oi, Method::Gtin));
            }
        }
    }
    // 2. Brand, MPN and options.
    for ni in 0..new.len() {
        if new_m[ni].is_some() {
            continue;
        }
        let Some(m) = &nk.brand_mpn[ni] else { continue };
        if let Some(&oi) = ok.by_brand_mpn.get(m) {
            if old_m[oi].is_none() {
                old_m[oi] = Some((ni, Method::BrandMpn));
                new_m[ni] = Some((oi, Method::BrandMpn));
            }
        }
    }
    // 3. Same id: never trusted alone. With contradicting evidence it is a
    //    reused id; without, the two are paired by id.
    let mut reused = Vec::new();
    let mut old_used = vec![false; old.len()];
    let mut new_used = vec![false; new.len()];
    for ni in 0..new.len() {
        let Some(id) = new[ni].value("item_id") else {
            continue;
        };
        let Some(&oi) = ok.by_id.get(id) else {
            continue;
        };
        if new_m[ni].map(|(o, _)| o == oi).unwrap_or(false) {
            continue; // already the same product by a strong key
        }
        let ev = reuse_evidence(&old[oi], &new[ni]);
        if ev.any() {
            reused.push((oi, ni, ev));
            old_used[oi] = true;
            new_used[ni] = true;
        } else if new_m[ni].is_none() && old_m[oi].is_none() {
            old_m[oi] = Some((ni, Method::Id));
            new_m[ni] = Some((oi, Method::Id));
        }
    }

    let mut pairs = Vec::new();
    for (ni, m) in new_m.iter().enumerate() {
        if let Some((oi, method)) = m {
            pairs.push((*oi, ni, *method));
            old_used[*oi] = true;
            new_used[ni] = true;
        }
    }
    Matching {
        pairs,
        reused,
        old_used,
        new_used,
    }
}
