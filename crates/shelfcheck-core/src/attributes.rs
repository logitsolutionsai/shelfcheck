//! Critical attribute parsing for `diff`: capacity, size, colour, material
//! and pack quantity.
//!
//! Attributes come from variant options first, and from the title only when
//! the record has no options at all. A value that cannot be parsed is
//! unknown, never guessed, and an unknown value is never reported as changed.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::OnceLock;

use regex::Regex;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attr {
    Capacity,
    Size,
    Colour,
    Material,
    PackQuantity,
}

impl Attr {
    pub fn id(self) -> &'static str {
        match self {
            Attr::Capacity => "capacity",
            Attr::Size => "size",
            Attr::Colour => "colour",
            Attr::Material => "material",
            Attr::PackQuantity => "pack_quantity",
        }
    }

    /// Which attribute an option name describes, if any.
    pub fn from_option_name(name: &str) -> Option<Attr> {
        let n = name.trim().to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
        if has(&["storage", "capacity", "memory", "hard drive", "ssd"]) {
            Some(Attr::Capacity)
        } else if has(&["colour", "color"]) {
            Some(Attr::Colour)
        } else if has(&["material", "fabric"]) {
            Some(Attr::Material)
        } else if has(&["pack", "quantity", "count", "multipack"]) {
            Some(Attr::PackQuantity)
        } else if has(&["size", "dimension", "length", "width", "height"]) {
            Some(Attr::Size)
        } else {
            None
        }
    }
}

/// A parsed attribute: what was written, and the canonical form compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttrValue {
    /// The option name, or the attribute id when parsed from the title.
    pub name: String,
    /// The value as written.
    pub text: String,
    /// Canonical form; two values are equal when these are equal.
    pub canonical: String,
    /// Unit as written (capacity and dimensions), for the decimal and binary caveat.
    pub unit: Option<String>,
    /// "option" or "title".
    pub from: String,
}

pub type Attributes = BTreeMap<Attr, AttrValue>;

/// Parse the critical attributes of a record.
pub fn parse(options: &[(String, String)], title: Option<&str>) -> Attributes {
    let mut out = Attributes::new();
    if !options.is_empty() {
        for (name, value) in options {
            let Some(attr) = Attr::from_option_name(name) else {
                continue;
            };
            if out.contains_key(&attr) {
                continue;
            }
            if let Some(v) = parse_value(attr, value) {
                out.insert(
                    attr,
                    AttrValue {
                        name: name.trim().to_string(),
                        text: value.trim().to_string(),
                        canonical: v.0,
                        unit: v.1,
                        from: "option".to_string(),
                    },
                );
            }
        }
        return out;
    }
    if let Some(t) = title {
        for attr in [
            Attr::Capacity,
            Attr::Size,
            Attr::PackQuantity,
            Attr::Colour,
            Attr::Material,
        ] {
            if let Some((text, canonical, unit)) = find_in_title(attr, t) {
                out.insert(
                    attr,
                    AttrValue {
                        name: attr.id().to_string(),
                        text,
                        canonical,
                        unit,
                        from: "title".to_string(),
                    },
                );
            }
        }
    }
    out
}

/// Parse one option value: (canonical, unit).
pub fn parse_value(attr: Attr, value: &str) -> Option<(String, Option<String>)> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    match attr {
        Attr::Capacity => capacity(v).map(|(c, u)| (c, Some(u))),
        Attr::Size => {
            if let Some((c, u)) = dimensions(v) {
                Some((c, Some(u)))
            } else {
                apparel_size(v).map(|c| (c, None))
            }
        }
        Attr::Colour => Some((colour(v), None)),
        Attr::Material => Some((material(v), None)),
        Attr::PackQuantity => pack_quantity(v).map(|c| (c, None)),
    }
}

fn re(cell: &'static OnceLock<Option<Regex>>, pattern: &str) -> Option<&'static Regex> {
    cell.get_or_init(|| Regex::new(pattern).ok()).as_ref()
}

static CAPACITY_RE: OnceLock<Option<Regex>> = OnceLock::new();
static DIMS_RE: OnceLock<Option<Regex>> = OnceLock::new();
static PACK_RE: OnceLock<Option<Regex>> = OnceLock::new();

const CAPACITY_PATTERN: &str = r"(?i)\b(\d+(?:\.\d+)?)\s*(kb|mb|gb|tb|pb)\b";
const DIMS_PATTERN: &str = r#"(?i)(\d+(?:\.\d+)?)(?:\s*[x×]\s*(\d+(?:\.\d+)?))?(?:\s*[x×]\s*(\d+(?:\.\d+)?))?\s*(mm|cm|m|inches|inch|in|")(?:\b|$|\s)"#;
const PACK_PATTERN: &str =
    r"(?i)(?:\bpack of\s*(\d+)\b|\b(\d+)\s*-?\s*(?:pack|pk|pcs|pieces|count|ct)\b)";

/// Capacity in bytes, decimal units: 128 GB equals 0.128 TB.
fn capacity(v: &str) -> Option<(String, String)> {
    let caps = re(&CAPACITY_RE, CAPACITY_PATTERN)?.captures(v)?;
    let n = Decimal::from_str(caps.get(1)?.as_str()).ok()?;
    let unit = caps.get(2)?.as_str().to_ascii_uppercase();
    let factor: i64 = match unit.as_str() {
        "KB" => 1_000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "TB" => 1_000_000_000_000,
        "PB" => 1_000_000_000_000_000,
        _ => return None,
    };
    let bytes = n.checked_mul(Decimal::from(factor))?.normalize();
    Some((format!("bytes:{bytes}"), unit))
}

/// Dimensions in millimetres: 30 cm equals 300 mm.
fn dimensions(v: &str) -> Option<(String, String)> {
    let caps = re(&DIMS_RE, DIMS_PATTERN)?.captures(v)?;
    let unit_raw = caps.get(4)?.as_str().to_ascii_lowercase();
    let (factor, unit) = match unit_raw.as_str() {
        "mm" => (Decimal::from(1i64), "mm"),
        "cm" => (Decimal::from(10i64), "cm"),
        "m" => (Decimal::from(1000i64), "m"),
        "in" | "inch" | "inches" | "\"" => (Decimal::new(254, 1), "in"),
        _ => return None,
    };
    let mut dims = Vec::new();
    for i in 1..=3 {
        if let Some(m) = caps.get(i) {
            let n = Decimal::from_str(m.as_str()).ok()?;
            dims.push(n.checked_mul(factor)?.normalize().to_string());
        }
    }
    Some((format!("mm:{}", dims.join("x")), unit.to_string()))
}

/// Apparel sizes: XS, S, M, L, XL, XXL (= 2XL) and the spelled-out forms,
/// plus plain numbers (shoe and dress sizes) compared as written.
fn apparel_size(v: &str) -> Option<String> {
    let n = v.trim().to_ascii_lowercase().replace(['-', '_'], " ");
    let n = n.split_whitespace().collect::<Vec<_>>().join(" ");
    let canon = match n.as_str() {
        "xxs" | "2xs" => "XXS",
        "xs" | "extra small" | "x small" => "XS",
        "s" | "small" | "sm" => "S",
        "m" | "medium" | "med" | "md" => "M",
        "l" | "large" | "lg" => "L",
        "xl" | "extra large" | "x large" => "XL",
        "xxl" | "2xl" | "xx large" => "XXL",
        "xxxl" | "3xl" | "xxx large" => "XXXL",
        "4xl" | "xxxxl" => "4XL",
        other => {
            if !other.is_empty() && other.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return Decimal::from_str(other)
                    .ok()
                    .map(|d| format!("size:{}", d.normalize()));
            }
            return Some(format!("text:{other}"));
        }
    };
    Some(format!("apparel:{canon}"))
}

fn pack_quantity(v: &str) -> Option<String> {
    let t = v.trim();
    if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
        return t.parse::<u64>().ok().map(|n| format!("pack:{n}"));
    }
    let caps = re(&PACK_RE, PACK_PATTERN)?.captures(t)?;
    let n = caps
        .get(1)
        .or_else(|| caps.get(2))?
        .as_str()
        .parse::<u64>()
        .ok()?;
    Some(format!("pack:{n}"))
}

/// The colour alias table, embedded from `map/colours.toml`.
fn colour_table() -> &'static HashMap<String, String> {
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let parsed: BTreeMap<String, Vec<String>> =
            toml::from_str(include_str!("map/colours.toml")).unwrap_or_default();
        let mut m = HashMap::new();
        for (canon, aliases) in parsed {
            m.insert(canon.to_ascii_lowercase(), canon.clone());
            for a in aliases {
                m.insert(a.to_ascii_lowercase(), canon.clone());
            }
        }
        m
    })
}

/// Canonical colour: the table's colour when listed, else the value itself
/// ignoring case and spacing.
fn colour(v: &str) -> String {
    let key = v
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match colour_table().get(&key) {
        Some(c) => format!("colour:{c}"),
        None => format!("text:{key}"),
    }
}

const MATERIALS: &[(&str, &str)] = &[
    ("stainless steel", "stainless steel"),
    ("aluminium", "aluminium"),
    ("aluminum", "aluminium"),
    ("cotton", "cotton"),
    ("polyester", "polyester"),
    ("wool", "wool"),
    ("leather", "leather"),
    ("steel", "steel"),
    ("glass", "glass"),
    ("ceramic", "ceramic"),
    ("porcelain", "porcelain"),
    ("plastic", "plastic"),
    ("silicone", "silicone"),
    ("bamboo", "bamboo"),
    ("linen", "linen"),
    ("silk", "silk"),
    ("wood", "wood"),
    ("oak", "oak"),
];

fn material(v: &str) -> String {
    let key = v.trim().to_ascii_lowercase();
    for (word, canon) in MATERIALS {
        if key == *word {
            return format!("material:{canon}");
        }
    }
    format!("text:{key}")
}

fn has_word(haystack_lower: &str, word: &str) -> bool {
    let bytes = haystack_lower.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack_lower[start..].find(word) {
        let i = start + pos;
        let j = i + word.len();
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after_ok = j >= bytes.len() || !bytes[j].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = i + word.len().max(1);
        if start >= haystack_lower.len() {
            break;
        }
    }
    false
}

/// Find an attribute in a title: (text as written, canonical, unit).
fn find_in_title(attr: Attr, title: &str) -> Option<(String, String, Option<String>)> {
    match attr {
        Attr::Capacity => {
            let m = re(&CAPACITY_RE, CAPACITY_PATTERN)?.find(title)?;
            let (c, u) = capacity(m.as_str())?;
            Some((m.as_str().to_string(), c, Some(u)))
        }
        Attr::Size => {
            let m = re(&DIMS_RE, DIMS_PATTERN)?.find(title)?;
            let text = m.as_str().trim().to_string();
            let (c, u) = dimensions(&text)?;
            Some((text, c, Some(u)))
        }
        Attr::PackQuantity => {
            let m = re(&PACK_RE, PACK_PATTERN)?.find(title)?;
            let text = m.as_str().to_string();
            Some((text.clone(), pack_quantity(&text)?, None))
        }
        Attr::Colour => {
            let lower = title.to_ascii_lowercase();
            // Longest aliases first, so "navy blue" wins over "blue".
            let mut aliases: Vec<(&String, &String)> = colour_table().iter().collect();
            aliases.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(b.0)));
            for (alias, canon) in aliases {
                if alias.len() >= 3 && has_word(&lower, alias) {
                    return Some((alias.clone(), format!("colour:{canon}"), None));
                }
            }
            None
        }
        Attr::Material => {
            let lower = title.to_ascii_lowercase();
            for (word, canon) in MATERIALS {
                if has_word(&lower, word) {
                    return Some((word.to_string(), format!("material:{canon}"), None));
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(n: &str, v: &str) -> Vec<(String, String)> {
        vec![(n.to_string(), v.to_string())]
    }

    #[test]
    fn colour_table_parses() {
        assert!(colour_table().len() > 20);
    }

    #[test]
    fn capacity_is_decimal() {
        let a = parse(&opt("storage", "128 GB"), None);
        let b = parse(&opt("Storage", "0.128 TB"), None);
        let c = parse(&opt("storage", "256GB"), None);
        assert_eq!(a[&Attr::Capacity].canonical, b[&Attr::Capacity].canonical);
        assert_ne!(a[&Attr::Capacity].canonical, c[&Attr::Capacity].canonical);
        assert_eq!(b[&Attr::Capacity].unit.as_deref(), Some("TB"));
    }

    #[test]
    fn sizes_convert_units() {
        let a = parse(&opt("size", "30 cm"), None);
        let b = parse(&opt("size", "300 mm"), None);
        assert_eq!(a[&Attr::Size].canonical, b[&Attr::Size].canonical);
        let c = parse(&opt("Size", "12 in"), None);
        assert_eq!(c[&Attr::Size].canonical, "mm:304.8");
        let d = parse(&opt("size", "30 x 40 cm"), None);
        assert_eq!(d[&Attr::Size].canonical, "mm:300x400");
        let m1 = parse(&opt("size", "M"), None);
        let m2 = parse(&opt("Size", "Medium"), None);
        assert_eq!(m1[&Attr::Size].canonical, m2[&Attr::Size].canonical);
        let x1 = parse(&opt("size", "XXL"), None);
        let x2 = parse(&opt("size", "2XL"), None);
        assert_eq!(x1[&Attr::Size].canonical, x2[&Attr::Size].canonical);
    }

    #[test]
    fn colours_need_the_table() {
        let a = parse(&opt("color", "Black"), None);
        let b = parse(&opt("colour", "BLK"), None);
        let c = parse(&opt("color", "Midnight"), None);
        assert_eq!(a[&Attr::Colour].canonical, b[&Attr::Colour].canonical);
        assert_ne!(a[&Attr::Colour].canonical, c[&Attr::Colour].canonical);
    }

    #[test]
    fn pack_quantity_by_keyword() {
        assert_eq!(pack_quantity("Pack of 6").as_deref(), Some("pack:6"));
        assert_eq!(pack_quantity("6-pack").as_deref(), Some("pack:6"));
        assert_eq!(pack_quantity("12 ct").as_deref(), Some("pack:12"));
        assert_eq!(pack_quantity("3").as_deref(), Some("pack:3"));
        assert_eq!(pack_quantity("some"), None);
    }

    #[test]
    fn title_used_only_without_options() {
        let t = Some("Slate Tab S, 128 GB, Grey");
        let from_title = parse(&[], t);
        assert_eq!(from_title[&Attr::Capacity].text, "128 GB");
        assert_eq!(from_title[&Attr::Colour].canonical, "colour:grey");
        let with_opts = parse(&opt("color", "Grey"), t);
        assert!(!with_opts.contains_key(&Attr::Capacity));
    }

    #[test]
    fn unparseable_is_unknown() {
        let a = parse(&opt("storage", "lots"), None);
        assert!(!a.contains_key(&Attr::Capacity));
    }
}
