//! The rule engine: the fixed set of rule kinds, run on one record.

use std::str::FromStr;

use rust_decimal::Decimal;

use super::acp::{field_for_pointer, generic_pointer, to_acp_product};
use super::{AppliesTo, IfState, Pack, Rule, RuleKind, RuleSet};
use crate::gtin;
use crate::model::{Field, Location, Record, TargetFormat};
use crate::price::{check_currency, parse_price};
use crate::read::Input;
use crate::results::Problem;

/// Options that change how problems are written, not what is checked.
#[derive(Debug, Clone, Default)]
pub struct EngineOpts {
    /// Canonical field names whose values never appear in output.
    pub redact: Vec<String>,
}

impl EngineOpts {
    fn redacted(&self, field: Option<&str>) -> bool {
        match field {
            Some(f) => self.redact.iter().any(|r| r.eq_ignore_ascii_case(f)),
            None => false,
        }
    }
}

/// One rule failure before it becomes a Problem.
#[derive(Debug, Default)]
struct Hit {
    location: Option<Location>,
    value: Option<String>,
    expected: Option<String>,
    vars: Vec<(&'static str, String)>,
    /// For schema errors: the fix key suffix and the field the pointer maps to.
    fix_suffix: Option<String>,
    field_override: Option<String>,
    /// For ACP schema errors inside /variants/N: that variant's id.
    item_override: Option<String>,
}

/// Fill a `says` template in one pass. Unknown placeholders are left as
/// written, and text that comes from a value is never expanded again, so a
/// title containing "{expected}" is quoted as it is.
pub fn render(template: &str, vars: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let key = &after[..close];
                match vars.iter().find(|(k, _)| *k == key) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push('{');
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The formats a pack's problems count against for a feed.
fn pack_formats(pack: &Pack, targets: &[TargetFormat]) -> Vec<TargetFormat> {
    match pack.meta.applies_to {
        AppliesTo::All => targets.to_vec(),
        AppliesTo::Cross => Vec::new(),
        other => match other.target() {
            Some(t) if targets.contains(&t) => vec![t],
            _ => Vec::new(),
        },
    }
}

/// Run every applicable rule on one record.
pub fn check_record(
    set: &RuleSet,
    rec: &Record,
    file: &str,
    targets: &[TargetFormat],
    opts: &EngineOpts,
) -> Vec<Problem> {
    let mut out = Vec::new();
    for pack in &set.packs {
        let formats = pack_formats(pack, targets);
        if formats.is_empty() {
            continue;
        }
        let native = pack.meta.applies_to.target().map(|t| t.family()) == Some(rec.family);
        for rule in &pack.rules {
            if set.is_ignored(&rule.def.id) {
                continue;
            }
            if let Some(stripped) = without_separators(rule, rec, &formats) {
                // Some formats ignore spaces and dashes in this field and
                // others do not: check each group on its own value, and
                // merge problems that read the same.
                let (lenient, strict): (Vec<TargetFormat>, Vec<TargetFormat>) = formats
                    .iter()
                    .partition(|f| rule.def.separators_ignored_by.contains(f));
                let start = out.len();
                if !strict.is_empty() {
                    for hit in eval(rule, rec, native, file) {
                        out.push(to_problem(pack, rule, rec, file, &strict, hit, opts));
                    }
                }
                let written = rule
                    .def
                    .field
                    .as_deref()
                    .and_then(|f| rec.value(f))
                    .map(str::to_string);
                for mut hit in eval(rule, &stripped, native, file) {
                    // Quote the value as written, not the stripped one.
                    if hit.value.is_some() {
                        hit.value = written.clone();
                    }
                    let p = to_problem(pack, rule, rec, file, &lenient, hit, opts);
                    match out[start..].iter_mut().find(|q| q.message == p.message) {
                        Some(q) => {
                            q.formats.extend(p.formats);
                            q.formats.sort();
                        }
                        None => out.push(p),
                    }
                }
                continue;
            }
            let hits = match rule.def.kind {
                RuleKind::JsonSchema => schema_hits(set, rule, rec, file),
                RuleKind::SameAcrossFormats => Vec::new(),
                _ => eval(rule, rec, native, file),
            };
            for hit in hits {
                out.push(to_problem(pack, rule, rec, file, &formats, hit, opts));
            }
        }
    }
    out
}

/// Spaces and dashes a format ignores in a value (Google, for gtin).
pub fn strip_separators(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '\u{a0}'))
        .collect()
}

/// A copy of the record with the rule's field stripped of spaces and dashes,
/// when one of `formats` ignores them and the value has any. None otherwise,
/// which is the common case and costs one scan of the value.
fn without_separators(rule: &Rule, rec: &Record, formats: &[TargetFormat]) -> Option<Record> {
    if rule.def.separators_ignored_by.is_empty()
        || !formats
            .iter()
            .any(|f| rule.def.separators_ignored_by.contains(f))
    {
        return None;
    }
    let name = rule.def.field.as_deref()?;
    let field = rec.get(name)?;
    if !field.raw.contains([' ', '-', '\u{a0}']) || field.raw.trim().is_empty() {
        return None;
    }
    let stripped = strip_separators(field.raw.trim());
    if stripped == field.raw.trim() {
        return None;
    }
    let mut copy = rec.clone();
    let mut f = field.clone();
    f.raw = stripped;
    copy.set(std::sync::Arc::from(name), f);
    Some(copy)
}

fn to_problem(
    pack: &Pack,
    rule: &Rule,
    rec: &Record,
    file: &str,
    formats: &[TargetFormat],
    hit: Hit,
    opts: &EngineOpts,
) -> Problem {
    let field = hit
        .field_override
        .clone()
        .or_else(|| rule.def.field.clone());
    let redacted = opts.redacted(field.as_deref());
    let mut vars = hit.vars;
    // Redaction covers every placeholder that can quote a feed value: the
    // rule's own field, and the other field a rule compares with.
    let than_redacted = opts.redacted(rule.def.than.as_deref());
    let if_redacted = opts.redacted(rule.def.if_field.as_deref());
    for (k, v) in vars.iter_mut() {
        let hide = match *k {
            // Price and currency reasons quote the currency code.
            "reason" => redacted && matches!(rule.def.kind, RuleKind::Price | RuleKind::Currency),
            "prefix" | "error" => redacted,
            "other" => than_redacted,
            "if_value" => if_redacted,
            _ => false,
        };
        if hide {
            *v = "[redacted]".to_string();
        }
    }
    let expected = if redacted { None } else { hit.expected.clone() };
    let shown_value = if redacted {
        Some("[redacted]".to_string())
    } else {
        hit.value.clone()
    };
    vars.push(("value", shown_value.clone().unwrap_or_default()));
    vars.push((
        "expected",
        expected.clone().unwrap_or_else(|| {
            if redacted {
                "[redacted]".to_string()
            } else {
                String::new()
            }
        }),
    ));
    vars.push(("field", field.clone().unwrap_or_default()));
    if let Some(m) = rule.def.max_len.or(rule.def.max_items) {
        vars.push(("max", m.to_string()));
    }
    let message = render(&rule.def.says, &vars);
    let fix = match &hit.fix_suffix {
        Some(s) => format!("{}:{s}", rule.fix_key),
        None => rule.fix_key.clone(),
    };
    let location = hit
        .location
        .unwrap_or_else(|| rec.location_for(file, field.as_deref()));
    Problem {
        rule: rule.def.id.clone(),
        pack: pack.meta.id.clone(),
        fix,
        severity: rule.def.severity,
        formats: formats.to_vec(),
        item_id: hit.item_override.unwrap_or_else(|| rec.item_id()),
        product: rec.product_key(),
        feed: rec.feed,
        location,
        field,
        value: if redacted { None } else { hit.value },
        expected,
        message,
        related: None,
    }
}

/// Location of a field, or of the record when the field is absent.
fn loc(rec: &Record, file: &str, name: &str) -> Location {
    match rec.get(name) {
        Some(f) => f.location(file),
        None => rec.location_for(file, Some(name)),
    }
}

fn eq(a: &str, b: &str, ignore_case: bool) -> bool {
    if ignore_case {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn condition(rule: &Rule, rec: &Record) -> (bool, String) {
    // `if_field`, or the first of `if_fields` that has a value.
    let v = match rule.def.if_field.as_deref() {
        Some(if_field) => rec.value(if_field),
        None if !rule.def.if_fields.is_empty() => {
            rule.def.if_fields.iter().find_map(|f| rec.value(f))
        }
        None => return (false, String::new()),
    };
    let in_values = |x: &str| {
        rule.def
            .if_values
            .iter()
            .any(|w| w.eq_ignore_ascii_case(x.trim()))
    };
    // A category path matches a value when it is that value or sits below
    // it: "Apparel & Accessories > Clothing > Shirts" is under
    // "Apparel & Accessories > Clothing", but "Apparel & Accessories >
    // Clothing Accessories" is not.
    let starts = |x: &str| {
        let lx = x.trim().to_ascii_lowercase();
        rule.def.if_values.iter().any(|w| {
            let lw = w.trim().to_ascii_lowercase();
            match lx.strip_prefix(lw.as_str()) {
                Some(rest) => rest.is_empty() || rest.trim_start().starts_with('>'),
                None => false,
            }
        })
    };
    let ok = match rule.def.if_state {
        IfState::Present => v.is_some(),
        IfState::Missing => v.is_none(),
        IfState::In => v.map(in_values).unwrap_or(false),
        IfState::MissingOrIn => v.map(in_values).unwrap_or(true),
        IfState::StartsWith => v.map(starts).unwrap_or(false),
    };
    (ok, v.unwrap_or("").to_string())
}

/// Why a URL is not a public absolute http(s) URL, if it is not.
pub fn url_reason(u: &str) -> Option<String> {
    let t = u.trim();
    let lower = t.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else if lower.starts_with("//") {
        return Some("has no scheme; start it with https://".to_string());
    } else if lower.contains("://") {
        return Some("is not an http or https URL".to_string());
    } else {
        return Some("is not an absolute URL; start it with https://".to_string());
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Some("has no host".to_string());
    }
    if t.chars().any(char::is_whitespace) {
        return Some("contains spaces; encode them as %20".to_string());
    }
    None
}

fn parts_of(rule: &Rule, f: &Field) -> Vec<String> {
    if rule.def.each {
        f.parts(Some(rule.def.separator.as_deref().unwrap_or(",")))
    } else {
        let v = f.value();
        if v.is_empty() {
            Vec::new()
        } else {
            vec![v.to_string()]
        }
    }
}

fn hit_at(rec: &Record, file: &str, field: &str, value: Option<String>) -> Hit {
    Hit {
        location: Some(loc(rec, file, field)),
        value,
        ..Hit::default()
    }
}

fn eval(rule: &Rule, rec: &Record, native: bool, file: &str) -> Vec<Hit> {
    let d = &rule.def;
    let Some(name) = d.field.as_deref() else {
        return Vec::new();
    };
    let field = rec.get(name).filter(|f| f.is_present());

    match d.kind {
        RuleKind::Required => {
            if field.is_none() {
                return vec![hit_at(rec, file, name, None)];
            }
            return Vec::new();
        }
        RuleKind::RequiredIf => {
            let (cond, if_value) = condition(rule, rec);
            if cond && field.is_none() {
                let mut h = hit_at(rec, file, name, None);
                h.vars.push(("if_value", if_value));
                return vec![h];
            }
            return Vec::new();
        }
        _ => {}
    }

    // Every other kind checks a value that is there; a missing value is the
    // business of the required kinds.
    let Some(f) = field else {
        return Vec::new();
    };
    let v = f.value();
    let h = |value: &str| hit_at(rec, file, name, Some(value.to_string()));

    match d.kind {
        RuleKind::MaxLen => {
            let max = d.max_len.unwrap_or(usize::MAX);
            if d.each {
                // With pair_separator, each part is "question:answer" and
                // each side has the limit (Google question_and_answer).
                let pieces: Vec<String> = match d.pair_separator.as_deref() {
                    Some(sep) if !sep.is_empty() => parts_of(rule, f)
                        .iter()
                        .flat_map(|p| match p.split_once(sep) {
                            Some((q, a)) => vec![q.trim().to_string(), a.trim().to_string()],
                            None => vec![p.clone()],
                        })
                        .collect(),
                    _ => parts_of(rule, f),
                };
                for part in pieces {
                    let n = part.chars().count();
                    if n > max {
                        let mut x = h(&part);
                        x.vars.push(("count", n.to_string()));
                        return vec![x];
                    }
                }
                Vec::new()
            } else {
                let n = match &f.items {
                    Some(items) => items.iter().map(|s| s.trim().chars().count()).sum(),
                    None => v.chars().count(),
                };
                if n > max {
                    let mut x = h(v);
                    x.vars.push(("count", n.to_string()));
                    vec![x]
                } else {
                    Vec::new()
                }
            }
        }
        RuleKind::Enum => {
            for part in parts_of(rule, f) {
                let ok = d.values.iter().any(|a| eq(a, &part, d.ignore_case))
                    || (!native && d.foreign_values.iter().any(|a| eq(a, &part, d.ignore_case)));
                if !ok {
                    let mut x = h(&part);
                    x.vars.push(("values", d.values.join(", ")));
                    return vec![x];
                }
            }
            Vec::new()
        }
        RuleKind::Pattern => {
            let Some(re) = &rule.regex else {
                return Vec::new();
            };
            for part in parts_of(rule, f) {
                let m = re.is_match(&part);
                if m == d.must_not_match {
                    return vec![h(&part)];
                }
            }
            Vec::new()
        }
        RuleKind::Price => match parse_price(v) {
            Err(reason) => {
                let mut x = h(v);
                x.vars.push(("reason", reason));
                vec![x]
            }
            Ok(p) => {
                if d.positive && p.amount <= Decimal::ZERO {
                    let mut x = h(v);
                    x.vars.push(("reason", "is not above 0".to_string()));
                    vec![x]
                } else {
                    Vec::new()
                }
            }
        },
        RuleKind::Currency => match check_currency(v) {
            Err(reason) => {
                let mut x = h(v);
                x.vars.push(("reason", reason));
                vec![x]
            }
            Ok(()) => Vec::new(),
        },
        RuleKind::UrlAbsolute => {
            for part in parts_of(rule, f) {
                if let Some(reason) = url_reason(&part) {
                    let mut x = h(&part);
                    x.vars.push(("reason", reason));
                    return vec![x];
                }
            }
            Vec::new()
        }
        RuleKind::Gtin => match d.check.as_deref() {
            Some("check_digit") => match gtin::wrong_check_digit(v) {
                Some(expected) => {
                    let mut x = h(v);
                    x.expected = Some(expected.to_string());
                    vec![x]
                }
                None => Vec::new(),
            },
            _ => match gtin::check_form(v) {
                Err(e) => {
                    let mut x = h(v);
                    x.vars.push(("reason", e.reason()));
                    vec![x]
                }
                Ok(()) => Vec::new(),
            },
        },
        RuleKind::GtinPrefix => {
            if !gtin::is_valid(v) {
                return Vec::new();
            }
            if !d.lengths.is_empty() && !d.lengths.contains(&v.len()) {
                return Vec::new();
            }
            let Some(form) = gtin::prefix_form(v) else {
                return Vec::new();
            };
            match gtin::prefix_matches(&form, &d.prefixes, &rule.ranges) {
                Some(prefix) => {
                    let mut x = h(v);
                    x.vars.push(("prefix", prefix));
                    vec![x]
                }
                None => Vec::new(),
            }
        }
        RuleKind::LessThanField => {
            let Some(than) = d.than.as_deref() else {
                return Vec::new();
            };
            let Some(other) = rec.value(than) else {
                return Vec::new();
            };
            let (Ok(a), Ok(b)) = (parse_price(v), parse_price(other)) else {
                return Vec::new();
            };
            if a.currency != b.currency || a.amount >= b.amount {
                let mut x = h(v);
                x.vars.push(("other", other.to_string()));
                vec![x]
            } else {
                Vec::new()
            }
        }
        RuleKind::ListMax => {
            let n = f.parts(Some(d.separator.as_deref().unwrap_or(","))).len();
            let max = d.max_items.unwrap_or(usize::MAX);
            if n > max {
                let mut x = h(v);
                x.vars.push(("count", n.to_string()));
                vec![x]
            } else {
                Vec::new()
            }
        }
        RuleKind::Range => {
            let reason = match Decimal::from_str(v) {
                Err(_) => Some("is not a number".to_string()),
                Ok(n) => {
                    if d.min.map(|m| n < Decimal::from(m)).unwrap_or(false) {
                        Some(format!("is below {}", d.min.unwrap_or(0)))
                    } else if d.max.map(|m| n > Decimal::from(m)).unwrap_or(false) {
                        Some(format!("is above {}", d.max.unwrap_or(0)))
                    } else {
                        None
                    }
                }
            };
            match reason {
                Some(r) => {
                    let mut x = h(v);
                    x.vars.push(("reason", r));
                    vec![x]
                }
                None => Vec::new(),
            }
        }
        RuleKind::Required
        | RuleKind::RequiredIf
        | RuleKind::JsonSchema
        | RuleKind::SameAcrossFormats => Vec::new(),
    }
}

/// ACP schema errors for one record: the original product JSON for ACP
/// input (once per line), or the converted product for any other format.
fn schema_hits(set: &RuleSet, rule: &Rule, rec: &Record, file: &str) -> Vec<Hit> {
    if rule.def.target.as_deref() == Some("metadata") {
        return Vec::new();
    }
    let Some(schema) = &set.acp else {
        return Vec::new();
    };
    let def = rule.def.def.as_deref().unwrap_or("Product");
    let mut out = Vec::new();
    if rec.family == crate::model::Family::Acp {
        let Some(json) = &rec.native_json else {
            return out;
        };
        for e in schema.validate(def, json) {
            let generic = generic_pointer(&e.pointer);
            out.push(Hit {
                location: Some(Location {
                    file: file.to_string(),
                    line: rec.line,
                    column: None,
                    column_name: None,
                    pointer: Some(if e.pointer.is_empty() {
                        "/".to_string()
                    } else {
                        e.pointer.clone()
                    }),
                }),
                value: json_value_at(json, &e.pointer),
                vars: vec![
                    (
                        "path",
                        if e.pointer.is_empty() {
                            "/".to_string()
                        } else {
                            e.pointer.clone()
                        },
                    ),
                    ("error", e.message.clone()),
                ],
                fix_suffix: Some(format!("{}:{generic}", e.keyword)),
                item_override: variant_id_at(json, &e.pointer),
                ..Hit::default()
            });
        }
        return out;
    }
    let (json, origins) = to_acp_product(rec);
    for e in schema.validate(def, &json) {
        let generic = generic_pointer(&e.pointer);
        let field = field_for_pointer(&origins, &e.pointer);
        let (location, value) = match &field {
            Some(name) => (loc(rec, file, name), rec.value(name).map(str::to_string)),
            None => (rec.location_for(file, None), None),
        };
        out.push(Hit {
            location: Some(location),
            value,
            vars: vec![("path", e.pointer.clone()), ("error", e.message.clone())],
            fix_suffix: Some(format!("{}:{generic}", e.keyword)),
            field_override: field,
            ..Hit::default()
        });
    }
    out
}

/// The id of the variant a pointer such as /variants/2/price/amount is in.
fn variant_id_at(json: &serde_json::Value, pointer: &str) -> Option<String> {
    let rest = pointer.strip_prefix("/variants/")?;
    let idx: usize = rest.split('/').next()?.parse().ok()?;
    json.get("variants")?
        .get(idx)?
        .get("id")?
        .as_str()
        .map(str::to_string)
}

fn json_value_at(json: &serde_json::Value, pointer: &str) -> Option<String> {
    let v = json.pointer(pointer)?;
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => None,
        other => Some(other.to_string()),
    }
}

/// ACP bundle metadata.json against $defs/FeedMetadata.
pub fn check_metadata(
    set: &RuleSet,
    input: &Input,
    feed: usize,
    targets: &[TargetFormat],
) -> Vec<Problem> {
    let mut out = Vec::new();
    let Some(meta) = &input.acp_metadata else {
        return out;
    };
    let Some(schema) = &set.acp else { return out };
    let file = input
        .metadata_path
        .clone()
        .unwrap_or_else(|| input.path.clone());
    for pack in &set.packs {
        let formats = pack_formats(pack, targets);
        if formats.is_empty() {
            continue;
        }
        for rule in &pack.rules {
            if rule.def.kind != RuleKind::JsonSchema
                || rule.def.target.as_deref() != Some("metadata")
            {
                continue;
            }
            if set.is_ignored(&rule.def.id) {
                continue;
            }
            let def = rule.def.def.as_deref().unwrap_or("FeedMetadata");
            for e in schema.validate(def, meta) {
                let generic = generic_pointer(&e.pointer);
                let path = if e.pointer.is_empty() {
                    "/".to_string()
                } else {
                    e.pointer.clone()
                };
                let vars = vec![("path", path.clone()), ("error", e.message.clone())];
                out.push(Problem {
                    rule: rule.def.id.clone(),
                    pack: pack.meta.id.clone(),
                    fix: format!("{}:{}:{generic}", rule.fix_key, e.keyword),
                    severity: rule.def.severity,
                    formats: formats.clone(),
                    item_id: "(metadata.json)".to_string(),
                    product: "(metadata.json)".to_string(),
                    feed,
                    location: Location {
                        file: file.clone(),
                        line: 1,
                        column: None,
                        column_name: None,
                        pointer: Some(path),
                    },
                    field: None,
                    value: json_value_at(meta, &e.pointer),
                    expected: None,
                    message: render(&rule.def.says, &vars),
                    related: None,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Family;
    use std::sync::Arc;

    fn rec(family: Family, pairs: &[(&str, &str)]) -> Record {
        let mut r = Record::new(0, family, 2);
        for (i, (k, v)) in pairs.iter().enumerate() {
            let mut f = Field::new(*v, Arc::from(*k), 2);
            f.column = Some(i as u32 + 1);
            r.set(Arc::from(*k), f);
        }
        crate::map::extract_options(&mut r);
        r
    }

    fn rules_hit(r: &Record, targets: &[TargetFormat]) -> Vec<String> {
        let set = RuleSet::load(&[], &[]).expect("rules");
        let mut ids: Vec<String> = check_record(&set, r, "t.csv", targets, &EngineOpts::default())
            .into_iter()
            .map(|p| p.rule)
            .collect();
        ids.sort();
        ids
    }

    fn clean() -> Vec<(&'static str, &'static str)> {
        vec![
            ("item_id", "A1"),
            ("title", "Harbor Mug"),
            ("description", "A mug."),
            ("url", "https://example.com/a1"),
            ("brand", "Harbor"),
            ("seller_name", "Harbor Shop"),
            ("image_url", "https://example.com/a1.jpg"),
            ("availability", "in_stock"),
            ("price", "12.00 USD"),
            ("target_countries", "US"),
            ("store_country", "US"),
            ("gtin", "0012345678905"),
        ]
    }

    #[test]
    fn clean_record_passes_every_format() {
        let r = rec(Family::Openai, &clean());
        assert_eq!(rules_hit(&r, &TargetFormat::ALL), Vec::<String>::new());
    }

    #[test]
    fn each_required_field_is_reported() {
        let mut pairs = clean();
        pairs.retain(|(k, _)| *k != "target_countries");
        let r = rec(Family::Openai, &pairs);
        assert_eq!(
            rules_hit(&r, &[TargetFormat::Openai]),
            vec!["openai.required.target_countries"]
        );
    }

    #[test]
    fn preorder_is_native_spelling_only() {
        let mut pairs = clean();
        for p in pairs.iter_mut() {
            if p.0 == "availability" {
                p.1 = "preorder";
            }
        }
        pairs.push(("availability_date", "2026-12-01"));
        let r = rec(Family::Openai, &pairs);
        assert_eq!(
            rules_hit(&r, &TargetFormat::ALL),
            vec!["openai.availability.enum"]
        );
        let g = rec(Family::Google, &pairs);
        // In a Google feed, preorder is Google's own spelling, and OpenAI
        // accepts it as foreign spelling a converter maps.
        assert_eq!(rules_hit(&g, &TargetFormat::ALL), Vec::<String>::new());
    }

    #[test]
    fn check_digit_and_prefixes() {
        let mut pairs = clean();
        for p in pairs.iter_mut() {
            if p.0 == "gtin" {
                p.1 = "0012345678904";
            }
        }
        let r = rec(Family::Openai, &pairs);
        let set = RuleSet::load(&[], &[]).expect("rules");
        let probs = check_record(
            &set,
            &r,
            "t.csv",
            &TargetFormat::ALL,
            &EngineOpts::default(),
        );
        assert_eq!(probs.len(), 1);
        assert_eq!(probs[0].rule, "gs1.gtin.check_digit");
        assert_eq!(probs[0].expected.as_deref(), Some("5"));
        assert_eq!(probs[0].formats, TargetFormat::ALL.to_vec());
        assert_eq!(
            probs[0].message,
            "0012345678904 has a wrong check digit. It should end in 5."
        );
        assert_eq!(probs[0].location.column, Some(12));
    }

    #[test]
    fn sale_price_and_lengths() {
        let mut pairs = clean();
        pairs.push(("sale_price", "12.00 USD"));
        let long = "x".repeat(151);
        let mut r = rec(Family::Openai, &pairs);
        r.set(Arc::from("title"), Field::new(long, Arc::from("title"), 2));
        let ids = rules_hit(&r, &TargetFormat::ALL);
        assert_eq!(
            ids,
            vec![
                "google.title.length",
                "openai.sale_price.below_price",
                "openai.title.length"
            ]
        );
    }

    fn local_set() -> RuleSet {
        let text = r#"
[pack]
id = "local"
name = "Local"
short = "Local"
applies_to = "google"
source = "test"
version = "1"
checked = "2026-11-14"

[[rule]]
id = "t.list"
field = "variant_option"
kind = "list_max"
max_items = 2
separator = ","
severity = "error"
says = "{count} values, max {max}"
cites = "t"

[[rule]]
id = "t.range"
field = "popularity_rank"
kind = "range"
min = 0
max = 100
severity = "error"
says = "{value} {reason}"
cites = "t"

[[rule]]
id = "t.pattern"
field = "related_product"
kind = "pattern"
regex = '^[^:]+:[^:]+:.+$'
each = true
separator = ","
severity = "error"
says = "{value} is not a:b:c"
cites = "t"

[[rule]]
id = "t.currency"
field = "currency"
kind = "currency"
severity = "warning"
says = "{value} {reason}"
cites = "t"
"#;
        let pack = Pack::parse(text, "local.toml").expect("pack");
        RuleSet {
            packs: vec![pack],
            ignored: std::collections::BTreeSet::new(),
            acp: None,
        }
    }

    #[test]
    fn list_range_pattern_currency_kinds() {
        let set = local_set();
        let bad = rec(
            Family::Google,
            &[
                ("variant_option", "Color:Red, Size:M, Memory:8 GB"),
                ("popularity_rank", "101"),
                ("related_product", "accessory:gtin:00012345678905, bad"),
                ("currency", "usd"),
            ],
        );
        let probs = check_record(
            &set,
            &bad,
            "t.csv",
            &[TargetFormat::Google],
            &EngineOpts::default(),
        );
        let msgs: Vec<(String, String)> = probs
            .iter()
            .map(|p| (p.rule.clone(), p.message.clone()))
            .collect();
        assert_eq!(
            msgs,
            vec![
                ("t.list".to_string(), "3 values, max 2".to_string()),
                ("t.range".to_string(), "101 is above 100".to_string()),
                ("t.pattern".to_string(), "bad is not a:b:c".to_string()),
                (
                    "t.currency".to_string(),
                    "usd has currency usd; ISO 4217 codes are uppercase (USD)".to_string()
                ),
            ]
        );
        let good = rec(
            Family::Google,
            &[
                ("variant_option", "Color:Red"),
                ("popularity_rank", "100"),
                ("related_product", "accessory:gtin:1"),
                ("currency", "USD"),
            ],
        );
        assert!(
            check_record(
                &set,
                &good,
                "t.csv",
                &[TargetFormat::Google],
                &EngineOpts::default()
            )
            .is_empty()
        );
        // A pack for another format does not run.
        assert!(
            check_record(
                &set,
                &bad,
                "t.csv",
                &[TargetFormat::Openai],
                &EngineOpts::default()
            )
            .is_empty()
        );
    }

    #[test]
    fn redaction_hides_values() {
        let set = local_set();
        let bad = rec(Family::Google, &[("popularity_rank", "101")]);
        let opts = EngineOpts {
            redact: vec!["popularity_rank".to_string()],
        };
        let probs = check_record(&set, &bad, "t.csv", &[TargetFormat::Google], &opts);
        assert_eq!(probs.len(), 1);
        assert_eq!(probs[0].value, None);
        assert_eq!(probs[0].message, "[redacted] is above 100");
    }

    /// QA 2026-11-15: OpenAI's weight field is `weight` (the rule read
    /// `item_weight`, which no feed has), and dimensions_unit goes with
    /// length, width or height, not with the dimensions object, which
    /// carries its own unit.
    #[test]
    fn weight_and_dimension_units() {
        let mut pairs = clean();
        pairs.push(("weight", "1.2"));
        pairs.push(("length", "20"));
        let r = rec(Family::Openai, &pairs);
        assert_eq!(
            rules_hit(&r, &[TargetFormat::Openai]),
            vec![
                "openai.dimensions_unit.required",
                "openai.item_weight_unit.required"
            ]
        );
        let mut ok = clean();
        ok.push(("weight", "1.2"));
        ok.push(("item_weight_unit", "kg"));
        ok.push((
            "dimensions",
            "{\"length\":\"20\",\"width\":\"10\",\"unit\":\"cm\"}",
        ));
        let r = rec(Family::Openai, &ok);
        assert_eq!(rules_hit(&r, &[TargetFormat::Openai]), Vec::<String>::new());
    }

    /// QA 2026-11-15: apparel categories match on a path boundary, so
    /// "Clothing Accessories" is not "Clothing". Fix pass 2: question_and_answer
    /// is not read in v1 (Google's quoted, comma-separated form), so no value
    /// of it raises a problem; the run lists it as not checked instead.
    #[test]
    fn google_pairs_and_category_paths() {
        let mut pairs = clean();
        let long_pair = format!("{}:{}", "q".repeat(900), "a".repeat(900));
        pairs.push(("question_and_answer", long_pair.as_str()));
        pairs.push((
            "product_category",
            "Apparel & Accessories > Clothing Accessories > Belts",
        ));
        pairs.push(("color", "Brown"));
        pairs.push(("gender", "unisex"));
        pairs.push(("age_group", "adult"));
        let r = rec(Family::Google, &pairs);
        assert_eq!(rules_hit(&r, &[TargetFormat::Google]), Vec::<String>::new());

        let mut bad = clean();
        let long_q = format!("{}:yes", "q".repeat(1001));
        bad.push(("question_and_answer", long_q.as_str()));
        bad.push((
            "product_category",
            "Apparel & Accessories > Clothing > Shirts",
        ));
        bad.push(("color", "Brown"));
        bad.push(("gender", "unisex"));
        bad.push(("age_group", "adult"));
        let r = rec(Family::Google, &bad);
        assert_eq!(
            rules_hit(&r, &[TargetFormat::Google]),
            vec!["google.apparel.size"]
        );
    }

    #[test]
    fn urls() {
        assert!(url_reason("https://example.com/x").is_none());
        assert!(url_reason("/x").is_some());
        assert!(url_reason("ftp://example.com/x").is_some());
        assert!(url_reason("https:///x").is_some());
        assert!(url_reason("https://example.com/a b").is_some());
    }

    #[test]
    fn render_fills_placeholders() {
        assert_eq!(
            render(
                "{value} should end in {expected}",
                &[("value", "1".into()), ("expected", "2".into())]
            ),
            "1 should end in 2"
        );
        assert_eq!(render("{unknown}", &[]), "{unknown}");
        // A value is never expanded again, whatever it contains.
        assert_eq!(
            render(
                "{value} should end in {expected}",
                &[("value", "{expected}".into()), ("expected", "5".into())]
            ),
            "{expected} should end in 5"
        );
        assert_eq!(render("a { b", &[]), "a { b");
    }
}
