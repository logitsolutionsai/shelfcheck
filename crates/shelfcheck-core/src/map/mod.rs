//! Column maps: from source column, element or key names to canonical fields.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::model::{Family, Field, Record};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapFile {
    map: MapMeta,
    columns: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapMeta {
    id: String,
    family: Family,
    #[serde(default)]
    fill_down_key: Option<String>,
    #[serde(default)]
    fill_down: Vec<String>,
    #[serde(default)]
    variant_columns: Vec<String>,
}

/// A compiled column map.
#[derive(Debug, Clone)]
pub struct ColumnMap {
    pub id: String,
    pub family: Family,
    columns: HashMap<String, Arc<str>>,
    /// Shopify export: the column whose value groups rows (Handle).
    pub fill_down_key: Option<String>,
    /// Columns copied from the first row of a group to the following rows.
    pub fill_down: Vec<String>,
    /// A row with all of these empty is not a variant row (Shopify image rows).
    pub variant_columns: Vec<String>,
}

/// Normalise a source name for lookup: trim, drop a BOM, ignore case and a
/// leading `g:` namespace prefix.
pub fn norm_key(s: &str) -> String {
    let t = s
        .trim()
        .trim_start_matches('\u{feff}')
        .trim()
        .to_ascii_lowercase();
    match t.strip_prefix("g:") {
        Some(rest) => rest.to_string(),
        None => t,
    }
}

impl ColumnMap {
    fn from_toml(text: &str, what: &str) -> Result<Self> {
        let file: MapFile = toml::from_str(text)
            .map_err(|e| Error::internal(format!("built-in map {what} is invalid: {e}")))?;
        let mut columns = HashMap::new();
        for (k, v) in file.columns {
            columns.insert(norm_key(&k), Arc::from(v.as_str()));
        }
        Ok(ColumnMap {
            id: file.map.id,
            family: file.map.family,
            columns,
            fill_down_key: file.map.fill_down_key,
            fill_down: file.map.fill_down,
            variant_columns: file.map.variant_columns,
        })
    }

    /// The built-in map for a family. Generic feeds use the OpenAI names,
    /// then the Google names.
    pub fn builtin(family: Family) -> Result<Self> {
        match family {
            Family::Openai => Self::from_toml(include_str!("openai.toml"), "openai.toml"),
            Family::Google => Self::from_toml(include_str!("google.toml"), "google.toml"),
            Family::Shopify => {
                Self::from_toml(include_str!("shopify-export.toml"), "shopify-export.toml")
            }
            Family::Acp => {
                // ACP is mapped in code (read/acp.rs); the OpenAI names are
                // the canonical names it maps to.
                let mut m = Self::from_toml(include_str!("openai.toml"), "openai.toml")?;
                m.id = "acp".to_string();
                m.family = Family::Acp;
                Ok(m)
            }
            Family::Generic => {
                let mut m = Self::from_toml(include_str!("openai.toml"), "openai.toml")?;
                let g = Self::from_toml(include_str!("google.toml"), "google.toml")?;
                for (k, v) in g.columns {
                    m.columns.entry(k).or_insert(v);
                }
                m.id = "generic".to_string();
                m.family = Family::Generic;
                Ok(m)
            }
        }
    }

    /// Add custom entries (`[map]` in shelfcheck.toml); they win over the
    /// built-in names.
    pub fn with_custom(mut self, custom: &BTreeMap<String, String>) -> Self {
        for (k, v) in custom {
            self.columns.insert(norm_key(k), Arc::from(v.trim()));
        }
        self
    }

    /// The canonical field for a source name. Unknown names are kept under
    /// their own normalised name, so custom rule packs can still use them.
    pub fn canonical(&self, source: &str) -> Arc<str> {
        let k = norm_key(source);
        match self.columns.get(&k) {
            Some(c) => c.clone(),
            None => Arc::from(k.as_str()),
        }
    }

    /// Is this source name one the map knows?
    pub fn knows(&self, source: &str) -> bool {
        self.columns.contains_key(&norm_key(source))
    }

    /// (source, canonical) pairs, sorted, for `shelfcheck init`.
    pub fn entries(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self
            .columns
            .iter()
            .map(|(k, c)| (k.clone(), c.to_string()))
            .collect();
        v.sort();
        v
    }
}

/// After mapping: build `record.options` from whichever option sources the
/// record has. Names are kept as written; the first source of a name wins.
pub fn extract_options(rec: &mut Record) {
    let mut opts: Vec<(String, String)> = Vec::new();
    let push = |opts: &mut Vec<(String, String)>, name: &str, value: &str| {
        let n = name.trim();
        let v = value.trim();
        if n.is_empty() || v.is_empty() {
            return;
        }
        if opts.iter().any(|(on, _)| on.eq_ignore_ascii_case(n)) {
            return;
        }
        opts.push((n.to_string(), v.to_string()));
    };

    if let Some(f) = rec.get("variant_dict") {
        for (k, v) in parse_variant_dict(&f.raw) {
            push(&mut opts, &k, &v);
        }
    }
    if let Some(f) = rec.get("variant_option") {
        for part in f.parts(Some(",")) {
            if let Some((k, v)) = part.split_once(':') {
                push(&mut opts, k, v);
            }
        }
    }
    for i in 1..=3 {
        let name = rec.value(&format!("option{i}_name")).map(str::to_string);
        let value = rec.value(&format!("option{i}_value")).map(str::to_string);
        if let (Some(n), Some(v)) = (name, value) {
            // Shopify's placeholder for products without options.
            if n.eq_ignore_ascii_case("title") && v.eq_ignore_ascii_case("default title") {
                continue;
            }
            push(&mut opts, &n, &v);
        }
    }
    for attr in ["color", "size", "material", "pattern"] {
        if let Some(v) = rec.value(attr).map(str::to_string) {
            push(&mut opts, attr, &v);
        }
    }
    if let Some(v) = rec.value("multipack").map(str::to_string) {
        push(&mut opts, "pack quantity", &v);
    }
    // ACP readers fill options directly; keep those first.
    let mut existing = std::mem::take(&mut rec.options);
    for (k, v) in opts {
        if !existing.iter().any(|(ek, _)| ek.eq_ignore_ascii_case(&k)) {
            existing.push((k, v));
        }
    }
    rec.options = existing;
}

/// `variant_dict` as a JSON object (`{"storage": "128 GB"}`), or as
/// `name:value` pairs separated by commas or semicolons.
pub fn parse_variant_dict(raw: &str) -> Vec<(String, String)> {
    let t = raw.trim();
    if t.is_empty() {
        return Vec::new();
    }
    if t.starts_with('{') {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(t) {
            return map
                .into_iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_json::Value::String(s) => s,
                        other => other.to_string(),
                    };
                    (k, s)
                })
                .collect();
        }
        return Vec::new();
    }
    t.split([',', ';'])
        .filter_map(|p| p.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// Insert a field under its canonical name. Repeated sources (repeated XML
/// elements) are collected as items.
pub fn insert_field(rec: &mut Record, canonical: Arc<str>, field: Field) {
    match rec.fields.get_mut(&canonical) {
        Some(existing) => {
            let mut items = existing
                .items
                .take()
                .unwrap_or_else(|| vec![existing.raw.clone()]);
            match field.items {
                Some(more) => items.extend(more),
                None => items.push(field.raw),
            }
            existing.raw = items.join(", ");
            existing.items = Some(items);
        }
        None => {
            rec.fields.insert(canonical, field);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_maps_parse() {
        for f in [
            Family::Openai,
            Family::Google,
            Family::Shopify,
            Family::Acp,
            Family::Generic,
        ] {
            assert!(ColumnMap::builtin(f).is_ok(), "{f:?}");
        }
    }

    #[test]
    fn google_names_with_and_without_prefix() {
        let m = ColumnMap::builtin(Family::Google).expect("map");
        assert_eq!(&*m.canonical("g:id"), "item_id");
        assert_eq!(&*m.canonical("link"), "url");
        assert_eq!(&*m.canonical("G:IMAGE_LINK"), "image_url");
        assert_eq!(&*m.canonical("custom_label_0"), "custom_label_0");
    }

    #[test]
    fn openai_aliases() {
        let m = ColumnMap::builtin(Family::Openai).expect("map");
        assert_eq!(&*m.canonical("sku"), "item_id");
        assert_eq!(&*m.canonical("\u{feff}item_id"), "item_id");
    }

    #[test]
    fn variant_dict_forms() {
        assert_eq!(
            parse_variant_dict(r#"{"storage": "128 GB", "color": "Grey"}"#),
            vec![
                ("color".to_string(), "Grey".to_string()),
                ("storage".to_string(), "128 GB".to_string())
            ]
        );
        assert_eq!(parse_variant_dict("size:M; color:Black").len(), 2);
    }
}
