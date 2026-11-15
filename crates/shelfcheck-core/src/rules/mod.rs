//! Rule packs as data. Each rule is a TOML entry using one of a fixed set of
//! kinds implemented in Rust (engine.rs). There is no expression language and
//! no eval; regexes are compiled from rule packs only, never from feed data.

pub mod acp;
pub mod builtin;
pub mod engine;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{Severity, TargetFormat};

/// Which formats a pack's problems count against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppliesTo {
    Openai,
    Acp,
    Google,
    /// Every format the feed is checked against (GS1 barcode rules).
    All,
    /// Cross-format consistency, checked across feeds.
    Cross,
}

impl AppliesTo {
    pub fn target(self) -> Option<TargetFormat> {
        match self {
            AppliesTo::Openai => Some(TargetFormat::Openai),
            AppliesTo::Acp => Some(TargetFormat::Acp),
            AppliesTo::Google => Some(TargetFormat::Google),
            AppliesTo::All | AppliesTo::Cross => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackMeta {
    pub id: String,
    /// "OpenAI product feed"
    pub name: String,
    /// "OpenAI"
    pub short: String,
    pub applies_to: AppliesTo,
    pub source: String,
    pub version: String,
    /// The date the rules were checked against the source, YYYY-MM-DD.
    pub checked: String,
    /// Text for the readiness table's "Rules as of" column.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Required,
    RequiredIf,
    MaxLen,
    Enum,
    Pattern,
    Price,
    Currency,
    UrlAbsolute,
    Gtin,
    GtinPrefix,
    LessThanField,
    JsonSchema,
    ListMax,
    Range,
    SameAcrossFormats,
}

impl RuleKind {
    pub fn id(self) -> &'static str {
        match self {
            RuleKind::Required => "required",
            RuleKind::RequiredIf => "required_if",
            RuleKind::MaxLen => "max_len",
            RuleKind::Enum => "enum",
            RuleKind::Pattern => "pattern",
            RuleKind::Price => "price",
            RuleKind::Currency => "currency",
            RuleKind::UrlAbsolute => "url_absolute",
            RuleKind::Gtin => "gtin",
            RuleKind::GtinPrefix => "gtin_prefix",
            RuleKind::LessThanField => "less_than_field",
            RuleKind::JsonSchema => "json_schema",
            RuleKind::ListMax => "list_max",
            RuleKind::Range => "range",
            RuleKind::SameAcrossFormats => "same_across_formats",
        }
    }
}

/// The condition of a `required_if` rule, on `if_field`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IfState {
    /// `if_field` has a value.
    #[default]
    Present,
    /// `if_field` is missing or empty.
    Missing,
    /// `if_field` is one of `if_values` (ignoring case).
    In,
    /// `if_field` is missing, or one of `if_values`.
    MissingOrIn,
    /// `if_field` starts with one of `if_values` (ignoring case).
    StartsWith,
}

/// When a rule runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum When {
    /// Only when the field has a value (default for every kind but required).
    #[default]
    Present,
    /// Always.
    Always,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDef {
    pub id: String,
    pub kind: RuleKind,
    #[serde(default)]
    pub field: Option<String>,
    pub severity: Severity,
    /// Sentence template: {value} {expected} {field} {source} {other} {max}
    /// {count} {reason} {values} {prefix} {path} {error}.
    pub says: String,
    /// The source's own wording of the rule.
    pub cites: String,
    /// Links to the source; default is the pack source.
    #[serde(default)]
    pub links: Vec<Link>,
    /// Groups rules from several packs into one fix in the report.
    #[serde(default)]
    pub fix: Option<String>,
    /// Fix heading.
    #[serde(default)]
    pub title: Option<String>,
    /// One more sentence of explanation for the report.
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub when: When,

    // required_if
    #[serde(default)]
    pub if_field: Option<String>,
    /// Instead of `if_field`: the condition looks at the first of these
    /// fields that has a value (for "required with any of length, width or
    /// height").
    #[serde(default)]
    pub if_fields: Vec<String>,
    #[serde(default)]
    pub if_state: IfState,
    #[serde(default)]
    pub if_values: Vec<String>,

    // max_len (characters), list_max (items)
    #[serde(default)]
    pub max_len: Option<usize>,
    #[serde(default)]
    pub max_items: Option<usize>,

    // range (inclusive, integers)
    #[serde(default)]
    pub min: Option<i64>,
    #[serde(default)]
    pub max: Option<i64>,

    // enum
    #[serde(default)]
    pub values: Vec<String>,
    /// Spellings accepted when the feed is in another format's spelling,
    /// because a converter maps them (`pre_order` for Google's `preorder`).
    #[serde(default)]
    pub foreign_values: Vec<String>,
    #[serde(default)]
    pub ignore_case: bool,

    // pattern
    #[serde(default)]
    pub regex: Option<String>,
    #[serde(default)]
    pub must_not_match: bool,

    // every per-value kind: check each part, split on `separator`
    #[serde(default)]
    pub each: bool,
    #[serde(default)]
    pub separator: Option<String>,
    /// max_len with each: split every part once on this, and check both
    /// sides (question:answer pairs).
    #[serde(default)]
    pub pair_separator: Option<String>,

    // price
    #[serde(default)]
    pub positive: bool,

    // less_than_field
    #[serde(default)]
    pub than: Option<String>,

    // gtin: "form" or "check_digit"
    #[serde(default)]
    pub check: Option<String>,

    // gtin_prefix
    #[serde(default)]
    pub prefixes: Vec<String>,
    #[serde(default)]
    pub ranges: Vec<Vec<String>>,
    #[serde(default)]
    pub lengths: Vec<usize>,

    // Formats that ignore spaces and dashes in this rule's field: for them
    // the value is checked with those removed (Google: gtin "spaces and
    // dashes are accepted, but ignored"). Every other format checks the
    // value as written.
    #[serde(default)]
    pub separators_ignored_by: Vec<TargetFormat>,

    // json_schema: the $defs entry to validate against; "product" records
    // or the bundle "metadata".
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub def: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixDef {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotCheckedDef {
    pub id: String,
    pub title: String,
    pub says: String,
    /// List this entry only when some checked item has this field; `{count}`
    /// in `says` becomes the number of such items.
    #[serde(default)]
    pub when_field: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    pub pack: PackMeta,
    #[serde(default, rename = "rule")]
    pub rules: Vec<RuleDef>,
    #[serde(default, rename = "fix")]
    pub fixes: Vec<FixDef>,
    #[serde(default, rename = "not_checked")]
    pub not_checked: Vec<NotCheckedDef>,
}

/// A rule ready to run.
#[derive(Debug, Clone)]
pub struct Rule {
    pub def: RuleDef,
    pub regex: Option<Regex>,
    pub ranges: Vec<(String, String)>,
    pub fix_key: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct Pack {
    pub meta: PackMeta,
    pub rules: Vec<Rule>,
    pub fixes: BTreeMap<String, FixDef>,
    pub not_checked: Vec<NotCheckedDef>,
    /// "built-in" or the pack file path.
    pub origin: String,
}

fn bad(origin: &str, rule: &str, msg: &str) -> Error {
    Error::input(format!("{origin}: rule {rule}: {msg}"))
}

impl Pack {
    pub fn parse(text: &str, origin: &str) -> Result<Self> {
        let file: PackFile = toml::from_str(text)
            .map_err(|e| Error::input(format!("{origin}: invalid rule pack: {e}")))?;
        let mut rules = Vec::with_capacity(file.rules.len());
        for def in file.rules {
            rules.push(compile(def, origin, file.pack.applies_to)?);
        }
        let fixes = file.fixes.into_iter().map(|f| (f.key.clone(), f)).collect();
        Ok(Pack {
            meta: file.pack,
            rules,
            fixes,
            not_checked: file.not_checked,
            origin: origin.to_string(),
        })
    }
}

fn default_title(def: &RuleDef) -> String {
    let field = def.field.as_deref().unwrap_or("value");
    match def.kind {
        RuleKind::Required | RuleKind::RequiredIf => format!("{field} is missing"),
        RuleKind::MaxLen => format!("{field} is too long"),
        RuleKind::Enum => format!("{field} values the format rejects"),
        RuleKind::Price => format!("{field} is not written as amount and currency"),
        RuleKind::Currency => format!("{field} is not an ISO 4217 code"),
        RuleKind::UrlAbsolute => format!("{field} is not an absolute http(s) URL"),
        RuleKind::Gtin => format!("{field} is not a valid GTIN"),
        RuleKind::GtinPrefix => {
            format!("{field} uses a prefix that is not for products sold publicly")
        }
        RuleKind::LessThanField => format!(
            "{field} is not below {}",
            def.than.as_deref().unwrap_or("the other field")
        ),
        RuleKind::ListMax => format!("{field} has too many values"),
        RuleKind::Range => format!("{field} is out of range"),
        RuleKind::Pattern => format!("{field} is not in the expected form"),
        RuleKind::JsonSchema => "ACP schema".to_string(),
        RuleKind::SameAcrossFormats => format!("{field} differs between feeds"),
    }
}

fn compile(def: RuleDef, origin: &str, applies_to: AppliesTo) -> Result<Rule> {
    let id = def.id.clone();
    let needs_field = !matches!(def.kind, RuleKind::JsonSchema);
    if needs_field && def.field.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(bad(origin, &id, "needs a field"));
    }
    match def.kind {
        RuleKind::RequiredIf if def.if_field.is_none() && def.if_fields.is_empty() => {
            return Err(bad(origin, &id, "required_if needs if_field or if_fields"));
        }
        RuleKind::RequiredIf if def.if_field.is_some() && !def.if_fields.is_empty() => {
            return Err(bad(origin, &id, "use if_field or if_fields, not both"));
        }
        RuleKind::RequiredIf
            if matches!(
                def.if_state,
                IfState::In | IfState::MissingOrIn | IfState::StartsWith
            ) && def.if_values.is_empty() =>
        {
            return Err(bad(origin, &id, "this if_state needs if_values"));
        }
        RuleKind::MaxLen if def.max_len.is_none() => {
            return Err(bad(origin, &id, "max_len needs max_len"));
        }
        RuleKind::ListMax if def.max_items.is_none() => {
            return Err(bad(origin, &id, "list_max needs max_items"));
        }
        RuleKind::Range if def.min.is_none() && def.max.is_none() => {
            return Err(bad(origin, &id, "range needs min or max"));
        }
        RuleKind::Enum if def.values.is_empty() => {
            return Err(bad(origin, &id, "enum needs values"));
        }
        RuleKind::Pattern if def.regex.is_none() => {
            return Err(bad(origin, &id, "pattern needs regex"));
        }
        RuleKind::LessThanField if def.than.is_none() => {
            return Err(bad(origin, &id, "less_than_field needs than"));
        }
        RuleKind::GtinPrefix if def.prefixes.is_empty() && def.ranges.is_empty() => {
            return Err(bad(origin, &id, "gtin_prefix needs prefixes or ranges"));
        }
        RuleKind::Gtin => match def.check.as_deref() {
            Some("form") | Some("check_digit") | None => {}
            Some(other) => {
                return Err(bad(
                    origin,
                    &id,
                    &format!("unknown gtin check {other}; use form or check_digit"),
                ));
            }
        },
        RuleKind::JsonSchema => {
            if def.def.is_none() {
                return Err(bad(origin, &id, "json_schema needs def"));
            }
            if def.schema.as_deref() != Some(acp::BUILTIN_SCHEMA) {
                return Err(bad(
                    origin,
                    &id,
                    &format!(
                        "json_schema supports only the vendored schema, schema = \"{}\"",
                        acp::BUILTIN_SCHEMA
                    ),
                ));
            }
        }
        RuleKind::SameAcrossFormats if applies_to != AppliesTo::Cross => {
            return Err(bad(
                origin,
                &id,
                "same_across_formats rules belong in a pack with applies_to = \"cross\"",
            ));
        }
        _ => {}
    }
    let regex = match &def.regex {
        Some(r) => {
            Some(Regex::new(r).map_err(|e| bad(origin, &id, &format!("invalid regex: {e}")))?)
        }
        None => None,
    };
    let mut ranges = Vec::new();
    for r in &def.ranges {
        match r.as_slice() {
            [lo, hi] if lo.len() == hi.len() && !lo.is_empty() => {
                ranges.push((lo.clone(), hi.clone()))
            }
            _ => {
                return Err(bad(
                    origin,
                    &id,
                    "each range is [low, high] with equal lengths",
                ));
            }
        }
    }
    let fix_key = def.fix.clone().unwrap_or_else(|| def.id.clone());
    let title = def.title.clone().unwrap_or_else(|| default_title(&def));
    Ok(Rule {
        def,
        regex,
        ranges,
        fix_key,
        title,
    })
}

/// Every pack in use, plus the ignore list.
#[derive(Debug)]
pub struct RuleSet {
    pub packs: Vec<Pack>,
    pub ignored: BTreeSet<String>,
    pub acp: Option<acp::AcpSchema>,
}

impl RuleSet {
    /// The built-in packs, plus local packs from `--rules` folders, minus
    /// ignored rules. An ignore that names no rule is an error.
    pub fn load(extra_dirs: &[&Path], ignore: &[String]) -> Result<Self> {
        let mut packs = builtin::packs()?;
        for dir in extra_dirs {
            packs.extend(load_dir(dir)?);
        }
        let mut seen = BTreeSet::new();
        for p in &packs {
            for r in &p.rules {
                if !seen.insert(r.def.id.clone()) {
                    return Err(Error::input(format!(
                        "{}: rule id {} is used twice",
                        p.origin, r.def.id
                    )));
                }
            }
        }
        let mut ignored = BTreeSet::new();
        for id in ignore {
            let id = id.trim().to_string();
            if !seen.contains(&id) {
                return Err(Error::input(format!(
                    "--ignore {id}: no rule has that id (see shelfcheck rules list)"
                )));
            }
            ignored.insert(id);
        }
        let needs_acp = packs
            .iter()
            .any(|p| p.rules.iter().any(|r| r.def.kind == RuleKind::JsonSchema));
        let acp = if needs_acp {
            Some(acp::AcpSchema::load()?)
        } else {
            None
        };
        Ok(RuleSet {
            packs,
            ignored,
            acp,
        })
    }

    pub fn is_ignored(&self, id: &str) -> bool {
        self.ignored.contains(id)
    }

    pub fn find(&self, id: &str) -> Option<(&Pack, &Rule)> {
        self.packs
            .iter()
            .find_map(|p| p.rules.iter().find(|r| r.def.id == id).map(|r| (p, r)))
    }

    /// The pack that holds a target format's rules.
    pub fn pack_for(&self, f: TargetFormat) -> Option<&Pack> {
        self.packs
            .iter()
            .find(|p| p.meta.applies_to.target() == Some(f) && p.origin == "built-in")
    }

    /// Fix title and detail overrides from every pack.
    pub fn fix_def(&self, key: &str) -> Option<&FixDef> {
        self.packs.iter().find_map(|p| p.fixes.get(key))
    }
}

fn load_dir(dir: &Path) -> Result<Vec<Pack>> {
    let rd = std::fs::read_dir(dir).map_err(|e| Error::io(dir, &e))?;
    let mut files = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| Error::io(dir, &e))?;
        let p = entry.path();
        if p.extension().map(|e| e == "toml").unwrap_or(false) {
            files.push(p);
        }
    }
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| Error::io(&f, &e))?;
        out.push(Pack::parse(&text, &f.display().to_string())?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_packs_load() {
        let set = RuleSet::load(&[], &[]).expect("built-in packs load");
        assert!(set.find("gs1.gtin.check_digit").is_some());
        assert!(set.pack_for(TargetFormat::Openai).is_some());
        assert!(set.pack_for(TargetFormat::Acp).is_some());
        assert!(set.pack_for(TargetFormat::Google).is_some());
        assert!(set.acp.is_some());
    }

    #[test]
    fn unknown_ignore_is_an_error() {
        let err = RuleSet::load(&[], &["no.such.rule".to_string()]).err();
        assert_eq!(err.map(|e| e.exit_code()), Some(2));
    }

    #[test]
    fn bad_rule_is_rejected() {
        let text = r#"
[pack]
id = "x"
name = "X"
short = "X"
applies_to = "openai"
source = "local"
version = "1"
checked = "2026-11-14"

[[rule]]
id = "x.len"
field = "title"
kind = "max_len"
severity = "error"
says = "too long"
cites = "local"
"#;
        assert!(Pack::parse(text, "x.toml").is_err());
        let typo = text.replace("kind = \"max_len\"", "kind = \"max_len\"\nmax_lenn = 3");
        assert!(Pack::parse(&typo, "x.toml").is_err());
    }
}
