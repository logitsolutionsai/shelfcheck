//! The decisions file, `shelfcheck-decisions.toml`, meant to be committed.
//! A decided change with the same kind, item and fingerprint stays quiet in
//! later runs; a new fingerprint for the same item is a new change.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const DEFAULT_FILE: &str = "shelfcheck-decisions.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    SameProduct,
    DifferentProduct,
    DataErrorFixed,
}

impl Verdict {
    pub fn id(self) -> &'static str {
        match self {
            Verdict::SameProduct => "same_product",
            Verdict::DifferentProduct => "different_product",
            Verdict::DataErrorFixed => "data_error_fixed",
        }
    }

    /// The `shelfcheck decide` flag for this verdict.
    pub fn flag(self) -> &'static str {
        match self {
            Verdict::SameProduct => "--same",
            Verdict::DifferentProduct => "--different",
            Verdict::DataErrorFixed => "--fixed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    /// The change id when the decision was made (D1, W2...). A label only;
    /// matching uses kind, item and fingerprint.
    pub id: String,
    pub change: String,
    pub item_id: String,
    pub fingerprint: String,
    pub verdict: Verdict,
    pub by: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decisions {
    #[serde(default, rename = "decision")]
    pub decisions: Vec<Decision>,
}

impl Decisions {
    /// Read the file; a missing file is an empty set.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Decisions::default());
        }
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, &e))?;
        toml::from_str(&text)
            .map_err(|e| Error::input(format!("{}: invalid decisions file: {e}", path.display())))
    }

    pub fn find(&self, change: &str, item_id: &str, fingerprint: &str) -> Option<&Decision> {
        self.decisions
            .iter()
            .find(|d| d.change == change && d.item_id == item_id && d.fingerprint == fingerprint)
    }

    /// Add or replace the decision for the same change, item and fingerprint.
    pub fn upsert(&mut self, d: Decision) {
        match self.decisions.iter_mut().find(|x| {
            x.change == d.change && x.item_id == d.item_id && x.fingerprint == d.fingerprint
        }) {
            Some(existing) => *existing = d,
            None => self.decisions.push(d),
        }
    }

    pub fn to_toml(&self) -> Result<String> {
        let body = toml::to_string(self)
            .map_err(|e| Error::internal(format!("cannot write decisions: {e}")))?;
        Ok(format!(
            "# shelfcheck decisions. Commit this file. Each entry silences one change\n# with the same kind, item and fingerprint. Written by `shelfcheck decide`.\n\n{body}"
        ))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = self.to_toml()?;
        std::fs::write(path, text).map_err(|e| Error::io(path, &e))
    }
}

/// Today's date (UTC) as YYYY-MM-DD, from the system clock.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    civil_date((secs / 86_400) as i64)
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
pub fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        // 2026-11-14 is day 20,771 since the epoch.
        assert_eq!(civil_date(20_771), "2026-11-14");
        assert_eq!(civil_date(-1), "1969-12-31");
    }

    #[test]
    fn roundtrip_and_upsert() {
        let mut ds = Decisions::default();
        let d = Decision {
            id: "D1".into(),
            change: "id_reused".into(),
            item_id: "PHN-X2-BLK".into(),
            fingerprint: "gtin:0194253012344->0194253098768".into(),
            verdict: Verdict::DifferentProduct,
            by: "dev@example.com".into(),
            at: "2026-11-14".into(),
            note: None,
        };
        ds.upsert(d.clone());
        let mut d2 = d.clone();
        d2.verdict = Verdict::SameProduct;
        ds.upsert(d2);
        assert_eq!(ds.decisions.len(), 1);
        let text = ds.to_toml().expect("toml");
        assert!(text.contains("[[decision]]"));
        let back: Decisions = toml::from_str(&text).expect("parse");
        assert_eq!(back, ds);
        assert!(
            back.find(
                "id_reused",
                "PHN-X2-BLK",
                "gtin:0194253012344->0194253098768"
            )
            .is_some()
        );
    }
}
