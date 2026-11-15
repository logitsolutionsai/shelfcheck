//! `shelfcheck.toml`: custom column maps, ignores, redaction and limits.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::read::Limits;

pub const DEFAULT_FILE: &str = "shelfcheck.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Formats to check: openai, acp, google.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<String>,
    /// Rule ids to turn off. Every ignore is reported.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
    /// Fields whose values never appear in the report or SARIF.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redact: Vec<String>,
    /// Folders of local rule packs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
    /// Source column -> canonical field.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub map: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
}

impl Config {
    /// Load a config file. `required` decides whether a missing file is an
    /// error (an explicit --config) or an empty config (the default name).
    pub fn load(path: &Path, required: bool) -> Result<Self> {
        if !path.exists() {
            if required {
                return Err(Error::Unavailable {
                    path: path.display().to_string(),
                    reason: "config file not found".to_string(),
                });
            }
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, &e))?;
        toml::from_str(&text)
            .map_err(|e| Error::input(format!("{}: invalid config: {e}", path.display())))
    }

    pub fn limits(&self) -> Limits {
        self.limits.unwrap_or_default().clamp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_rejects_typos() {
        let c: Config = toml::from_str(
            "ignore = [\"openai.title.length\"]\nredact = [\"cost\"]\n[map]\n\"My SKU\" = \"item_id\"\n[limits]\nmax_rows = 10\n",
        )
        .expect("config");
        assert_eq!(c.map.get("My SKU").map(String::as_str), Some("item_id"));
        assert_eq!(c.limits().max_rows, 10);
        assert!(toml::from_str::<Config>("ignorr = []").is_err());
    }
}
