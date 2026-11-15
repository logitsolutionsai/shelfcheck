//! The one record model every input format maps to.
//!
//! A `Record` is one item: one row of a flat feed (OpenAI, Google, Shopify
//! export) or one variant of an ACP product. Records that share a `group_id`
//! form a product. Every field keeps the value exactly as written, plus where
//! it came from, so a problem can always quote and point at the original cell.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Where a value sits in its source file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct Location {
    /// The file path as given on the command line.
    pub file: String,
    /// 1-based line. For CSV, the line the row starts on.
    pub line: u64,
    /// 1-based column index (CSV and TSV only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// The column, element or key name as written in the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_name: Option<String>,
    /// JSON pointer inside the line (JSONL and ACP only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

impl Location {
    /// "row 214, column G" style text for people.
    pub fn describe(&self) -> String {
        let mut s = format!("line {}", self.line);
        if let Some(c) = self.column {
            s.push_str(&format!(", column {}", column_letters(c)));
        }
        if let Some(p) = &self.pointer {
            s.push_str(&format!(", {p}"));
        } else if self.column.is_none() {
            if let Some(n) = &self.column_name {
                s.push_str(&format!(", {n}"));
            }
        }
        s
    }
}

/// 1 -> A, 26 -> Z, 27 -> AA.
pub fn column_letters(mut col: u32) -> String {
    let mut out = Vec::new();
    while col > 0 {
        let rem = ((col - 1) % 26) as u8;
        out.push(b'A' + rem);
        col = (col - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// One value as written, with its source position.
#[derive(Debug, Clone)]
pub struct Field {
    /// The value exactly as written (for repeated XML elements and JSON
    /// arrays, the items joined with ", ").
    pub raw: String,
    /// Separate values when the source had them separately (repeated XML
    /// elements, JSON arrays).
    pub items: Option<Vec<String>>,
    /// The source column, element or key name as written.
    pub source: Arc<str>,
    pub line: u64,
    pub column: Option<u32>,
    pub pointer: Option<String>,
}

impl Field {
    pub fn new(raw: impl Into<String>, source: Arc<str>, line: u64) -> Self {
        Field {
            raw: raw.into(),
            items: None,
            source,
            line,
            column: None,
            pointer: None,
        }
    }

    pub fn is_present(&self) -> bool {
        match &self.items {
            Some(items) => items.iter().any(|s| !s.trim().is_empty()),
            None => !self.raw.trim().is_empty(),
        }
    }

    pub fn value(&self) -> &str {
        self.raw.trim()
    }

    /// The separate values: the source items when there are any, otherwise
    /// the raw value split on `sep` (empty parts dropped).
    pub fn parts(&self, sep: Option<&str>) -> Vec<String> {
        if let Some(items) = &self.items {
            return items
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        match sep {
            Some(sep) if !sep.is_empty() => self
                .raw
                .split(sep)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            _ => {
                let v = self.raw.trim();
                if v.is_empty() {
                    Vec::new()
                } else {
                    vec![v.to_string()]
                }
            }
        }
    }

    /// Character count; for multi-item values, the sum over the items.
    pub fn char_len(&self) -> usize {
        match &self.items {
            Some(items) => items.iter().map(|s| s.chars().count()).sum(),
            None => self.raw.chars().count(),
        }
    }

    pub fn location(&self, file: &str) -> Location {
        Location {
            file: file.to_string(),
            line: self.line,
            column: self.column,
            column_name: Some(self.source.to_string()),
            pointer: self.pointer.clone(),
        }
    }
}

/// The family of a feed format: whose field names and spellings it uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Openai,
    Google,
    Acp,
    Shopify,
    Generic,
}

impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Family::Openai => "OpenAI",
            Family::Google => "Google",
            Family::Acp => "ACP",
            Family::Shopify => "Shopify export",
            Family::Generic => "Generic",
        }
    }

    /// The target format this family is native to, if any.
    pub fn target(self) -> Option<TargetFormat> {
        match self {
            Family::Openai => Some(TargetFormat::Openai),
            Family::Google => Some(TargetFormat::Google),
            Family::Acp => Some(TargetFormat::Acp),
            Family::Shopify | Family::Generic => None,
        }
    }
}

/// The container a feed is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    Csv,
    Tsv,
    Jsonl,
    Xml,
    AcpBundle,
}

impl Container {
    pub fn label(self) -> &'static str {
        match self {
            Container::Csv => "CSV",
            Container::Tsv => "TSV",
            Container::Jsonl => "JSONL",
            Container::Xml => "XML",
            Container::AcpBundle => "bundle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FeedFormat {
    pub family: Family,
    pub container: Container,
}

impl FeedFormat {
    pub fn new(family: Family, container: Container) -> Self {
        FeedFormat { family, container }
    }

    /// Parse a `--format` value such as `openai-csv`, `google-xml`, `acp`,
    /// `shopify`, `csv`, `tsv` or `jsonl`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let (fam, cont) = match s.split_once('-') {
            Some((f, c)) => (f.to_string(), Some(c.to_string())),
            None => (s.clone(), None),
        };
        let container = |c: Option<String>, default: Container| -> Option<Container> {
            match c.as_deref() {
                None => Some(default),
                Some("csv") => Some(Container::Csv),
                Some("tsv") | Some("txt") => Some(Container::Tsv),
                Some("jsonl") | Some("ndjson") => Some(Container::Jsonl),
                Some("xml") => Some(Container::Xml),
                _ => None,
            }
        };
        match fam.as_str() {
            "openai" => Some(FeedFormat::new(
                Family::Openai,
                container(cont, Container::Csv)?,
            )),
            "google" => Some(FeedFormat::new(
                Family::Google,
                container(cont, Container::Xml)?,
            )),
            "acp" => Some(FeedFormat::new(Family::Acp, Container::AcpBundle)),
            "shopify" => Some(FeedFormat::new(Family::Shopify, Container::Csv)),
            "csv" if cont.is_none() => Some(FeedFormat::new(Family::Generic, Container::Csv)),
            "tsv" if cont.is_none() => Some(FeedFormat::new(Family::Generic, Container::Tsv)),
            "jsonl" if cont.is_none() => Some(FeedFormat::new(Family::Generic, Container::Jsonl)),
            _ => None,
        }
    }

    pub fn id(&self) -> String {
        match (self.family, self.container) {
            (Family::Acp, _) => "acp".to_string(),
            (Family::Shopify, _) => "shopify".to_string(),
            (f, c) => format!(
                "{}-{}",
                match f {
                    Family::Openai => "openai",
                    Family::Google => "google",
                    _ => "generic",
                },
                match c {
                    Container::Csv => "csv",
                    Container::Tsv => "tsv",
                    Container::Jsonl => "jsonl",
                    Container::Xml => "xml",
                    Container::AcpBundle => "acp",
                }
            ),
        }
    }
}

impl fmt::Display for FeedFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.family {
            Family::Acp => write!(f, "ACP bundle"),
            Family::Shopify => write!(f, "Shopify export CSV"),
            Family::Generic => write!(f, "{}", self.container.label()),
            fam => write!(f, "{} {}", fam.label(), self.container.label()),
        }
    }
}

/// The formats shelfcheck checks against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetFormat {
    Openai,
    Acp,
    Google,
}

impl TargetFormat {
    pub const ALL: [TargetFormat; 3] = [
        TargetFormat::Openai,
        TargetFormat::Acp,
        TargetFormat::Google,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TargetFormat::Openai => "OpenAI",
            TargetFormat::Acp => "ACP",
            TargetFormat::Google => "Google",
        }
    }

    pub fn long_label(self) -> &'static str {
        match self {
            TargetFormat::Openai => "OpenAI product feed",
            TargetFormat::Acp => "ACP feed schema",
            TargetFormat::Google => "Google Merchant Center",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            TargetFormat::Openai => "openai",
            TargetFormat::Acp => "acp",
            TargetFormat::Google => "google",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "openai" => Some(TargetFormat::Openai),
            "acp" => Some(TargetFormat::Acp),
            "google" | "gmc" => Some(TargetFormat::Google),
            _ => None,
        }
    }

    pub fn family(self) -> Family {
        match self {
            TargetFormat::Openai => Family::Openai,
            TargetFormat::Acp => Family::Acp,
            TargetFormat::Google => Family::Google,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    // Order matters: Error sorts first.
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// What an availability value means, whatever the spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    InStock,
    OutOfStock,
    PreOrder,
    Backorder,
    Unknown,
    LimitedStock,
    Discontinued,
}

impl Availability {
    /// Every spelling used by the three formats, plus common space-separated
    /// variants Google also documents.
    pub fn parse(s: &str) -> Option<Self> {
        let v = s.trim().to_ascii_lowercase();
        match v.as_str() {
            "in_stock" | "in stock" | "instock" => Some(Availability::InStock),
            "out_of_stock" | "out of stock" | "outofstock" => Some(Availability::OutOfStock),
            "pre_order" | "preorder" | "pre-order" | "pre order" => Some(Availability::PreOrder),
            "backorder" | "back_order" | "back order" => Some(Availability::Backorder),
            "unknown" => Some(Availability::Unknown),
            "limited_stock" => Some(Availability::LimitedStock),
            "discontinued" => Some(Availability::Discontinued),
            _ => None,
        }
    }
}

/// One item from a feed.
#[derive(Debug, Clone)]
pub struct Record {
    /// Index of the input feed this record came from.
    pub feed: usize,
    /// The format family of that feed.
    pub family: Family,
    /// 1-based line where the record starts.
    pub line: u64,
    /// Canonical field name -> value. Canonical names are the OpenAI field
    /// names (`item_id`, `url`, `image_url`...), plus Google-only attributes
    /// under their Google names.
    pub fields: BTreeMap<Arc<str>, Field>,
    /// Variant options as (name, value), from `variant_dict`,
    /// `variant_option`, Shopify option columns, ACP `variant_options`, or the
    /// `color`, `size`, `material` and `pattern` attributes.
    pub options: Vec<(String, String)>,
    /// For ACP input: the original product JSON, attached to the first
    /// variant of each product line so the schema is checked once per line.
    pub native_json: Option<Arc<serde_json::Value>>,
}

impl Record {
    pub fn new(feed: usize, family: Family, line: u64) -> Self {
        Record {
            feed,
            family,
            line,
            fields: BTreeMap::new(),
            options: Vec::new(),
            native_json: None,
        }
    }

    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.get(name)
    }

    /// The trimmed value of a field, if present and not empty.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.fields
            .get(name)
            .filter(|f| f.is_present())
            .map(|f| f.value())
    }

    pub fn has(&self, name: &str) -> bool {
        self.value(name).is_some()
    }

    pub fn set(&mut self, name: Arc<str>, field: Field) {
        self.fields.insert(name, field);
    }

    pub fn item_id(&self) -> String {
        self.value("item_id")
            .map(str::to_string)
            .unwrap_or_else(|| format!("(line {})", self.line))
    }

    /// The product this item belongs to: its group id, or its own id.
    pub fn product_key(&self) -> String {
        match self.value("group_id") {
            Some(g) => g.to_string(),
            None => self.item_id(),
        }
    }

    /// A location for the record itself, or for a missing field.
    pub fn location_for(&self, file: &str, field_hint: Option<&str>) -> Location {
        Location {
            file: file.to_string(),
            line: self.line,
            column: None,
            column_name: field_hint.map(str::to_string),
            pointer: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters_work() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(7), "G");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(27), "AA");
        assert_eq!(column_letters(52), "AZ");
        assert_eq!(column_letters(53), "BA");
    }

    #[test]
    fn availability_spellings_share_a_meaning() {
        assert_eq!(
            Availability::parse("pre_order"),
            Availability::parse("preorder")
        );
        assert_eq!(Availability::parse("in_stock"), Some(Availability::InStock));
        assert_eq!(Availability::parse("maybe"), None);
    }

    #[test]
    fn format_parse_roundtrip() {
        for s in [
            "openai-csv",
            "google-xml",
            "google-tsv",
            "acp",
            "shopify",
            "openai-jsonl",
        ] {
            let f = FeedFormat::parse(s).map(|f| f.id());
            assert_eq!(f.as_deref(), Some(s));
        }
    }

    #[test]
    fn field_parts_split_and_trim() {
        let f = Field::new("a, b,,c ", Arc::from("x"), 1);
        assert_eq!(f.parts(Some(",")), vec!["a", "b", "c"]);
        assert_eq!(f.parts(None), vec!["a, b,,c"]);
    }
}
