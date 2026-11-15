//! The vendored ACP feed schema (2026-04-17) and the conversion of flat
//! records to ACP products, so feeds in other formats can be checked against
//! the same schema.
//!
//! The schema file is kept unmodified in `rules/acp-feed/vendor/`. Its `$id`
//! is a placeholder (`https://example.com/...`), so it is never fetched: the
//! validator is built from a local wrapper that holds the schema's `$defs`
//! and points at one of them with a local `$ref`.

use std::fmt;

use serde_json::{Map, Value, json};

use crate::error::{Error, Result};
use crate::model::{Availability, Record};
use crate::price::parse_price;

/// The value of `schema =` in a json_schema rule that means the vendored file.
pub const BUILTIN_SCHEMA: &str = "acp-feed/vendor/schema.feed.json";

/// The vendored schema, embedded at build time.
pub const SCHEMA_TEXT: &str = include_str!("../../rules/acp-feed/vendor/schema.feed.json");

/// The ACP spec folder the schema came from.
pub const SCHEMA_VERSION: &str = "2026-04-17";

/// One schema error, flattened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    /// JSON pointer into the instance.
    pub pointer: String,
    /// The JSON Schema keyword that failed ("type", "required", ...).
    pub keyword: String,
    pub message: String,
}

pub struct AcpSchema {
    product: jsonschema::Validator,
    metadata: jsonschema::Validator,
}

impl fmt::Debug for AcpSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AcpSchema({SCHEMA_VERSION})")
    }
}

fn unavailable(msg: impl fmt::Display) -> Error {
    Error::Unavailable {
        path: format!("rules/{BUILTIN_SCHEMA}"),
        reason: format!("the ACP schema validator could not be built: {msg}"),
    }
}

fn build(defs: &Value, def: &str) -> Result<jsonschema::Validator> {
    let wrapper = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": defs.clone(),
        "$ref": format!("#/$defs/{def}"),
    });
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .should_validate_formats(true)
        .build(&wrapper)
        .map_err(unavailable)
}

impl AcpSchema {
    pub fn load() -> Result<Self> {
        let root: Value = serde_json::from_str(SCHEMA_TEXT).map_err(unavailable)?;
        let defs = root
            .get("$defs")
            .ok_or_else(|| unavailable("the schema has no $defs"))?;
        for needed in ["Product", "FeedMetadata"] {
            if defs.get(needed).is_none() {
                return Err(unavailable(format!("the schema has no $defs/{needed}")));
            }
        }
        Ok(AcpSchema {
            product: build(defs, "Product")?,
            metadata: build(defs, "FeedMetadata")?,
        })
    }

    fn validator(&self, def: &str) -> Option<&jsonschema::Validator> {
        match def {
            "Product" => Some(&self.product),
            "FeedMetadata" => Some(&self.metadata),
            _ => None,
        }
    }

    /// Validate against a `$defs` entry. Errors are sorted by pointer, then
    /// keyword, then message, so output never depends on validator order.
    pub fn validate(&self, def: &str, instance: &Value) -> Vec<SchemaError> {
        let Some(v) = self.validator(def) else {
            return vec![SchemaError {
                pointer: String::new(),
                keyword: "schema".to_string(),
                message: format!("no $defs/{def} in the vendored schema"),
            }];
        };
        // Fast path: most records are valid, and is_valid stops at the first
        // failure without building error values.
        if v.is_valid(instance) {
            return Vec::new();
        }
        let mut out: Vec<SchemaError> = v
            .iter_errors(instance)
            .map(|e| SchemaError {
                pointer: e.instance_path().as_str().to_string(),
                keyword: e.kind().keyword().to_string(),
                message: e.to_string(),
            })
            .collect();
        out.sort_by(|a, b| {
            a.pointer
                .cmp(&b.pointer)
                .then_with(|| a.keyword.cmp(&b.keyword))
                .then_with(|| a.message.cmp(&b.message))
        });
        out.dedup();
        out
    }
}

/// Replace array indexes in a pointer with `*`, for grouping fixes.
pub fn generic_pointer(p: &str) -> String {
    p.split('/')
        .map(|seg| {
            if !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()) {
                "*"
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The ACP availability spelling for a meaning.
fn acp_status(a: Availability) -> &'static str {
    match a {
        Availability::InStock => "in_stock",
        Availability::OutOfStock => "out_of_stock",
        Availability::PreOrder => "preorder",
        Availability::Backorder => "backorder",
        Availability::Unknown => "unknown",
        Availability::LimitedStock => "limited_stock",
        Availability::Discontinued => "discontinued",
    }
}

/// A flat record as the one-variant ACP product a straightforward converter
/// would write, plus which canonical field each JSON pointer came from.
/// Values are copied as written; nothing is invented or repaired, so the
/// schema sees the same problems the feed has.
pub fn to_acp_product(rec: &Record) -> (Value, Vec<(String, String)>) {
    let mut origins: Vec<(String, String)> = Vec::new();
    let mut variant = Map::new();
    let mut put = |variant: &mut Map<String, Value>, key: &str, value: Value, field: &str| {
        variant.insert(key.to_string(), value);
        origins.push((format!("/variants/0/{key}"), field.to_string()));
    };
    if let Some(v) = rec.value("item_id") {
        put(&mut variant, "id", json!(v), "item_id");
    }
    if let Some(v) = rec.value("title") {
        put(&mut variant, "title", json!(v), "title");
    }
    if let Some(v) = rec.value("description") {
        put(
            &mut variant,
            "description",
            json!({ "plain": v }),
            "description",
        );
    }
    if let Some(v) = rec.value("url") {
        put(&mut variant, "url", json!(v), "url");
    }
    if let Some(v) = rec.value("gtin") {
        put(
            &mut variant,
            "barcodes",
            json!([{ "type": "GTIN", "value": v }]),
            "gtin",
        );
    }
    if let Some(v) = rec.value("price") {
        if let Ok(p) = parse_price(v) {
            let amount = match p.to_minor() {
                Some(m) => json!(m),
                // Not a whole number of minor units: keep the decimal, so the
                // schema reports it rather than a rounded value.
                None => serde_json::from_str::<Value>(&p.amount.to_string()).unwrap_or(Value::Null),
            };
            put(
                &mut variant,
                "price",
                json!({ "amount": amount, "currency": p.currency }),
                "price",
            );
        }
    }
    if let Some(v) = rec.value("availability") {
        let (status, available) = match Availability::parse(v) {
            Some(a) => (
                acp_status(a).to_string(),
                matches!(a, Availability::InStock | Availability::LimitedStock),
            ),
            None => (v.to_string(), false),
        };
        put(
            &mut variant,
            "availability",
            json!({ "available": available, "status": status }),
            "availability",
        );
    }
    if let Some(v) = rec.value("condition") {
        put(&mut variant, "condition", json!([v]), "condition");
    }
    if let Some(v) = rec.value("product_category") {
        put(
            &mut variant,
            "categories",
            json!([{ "value": v }]),
            "product_category",
        );
    }
    if !rec.options.is_empty() {
        let opts: Vec<Value> = rec
            .options
            .iter()
            .map(|(n, v)| json!({ "name": n, "value": v }))
            .collect();
        put(
            &mut variant,
            "variant_options",
            Value::Array(opts),
            "variant_dict",
        );
    }
    let mut media = Vec::new();
    if let Some(v) = rec.value("image_url") {
        media.push(json!({ "type": "image", "url": v }));
    }
    let mut extra_start = None;
    if let Some(f) = rec.get("additional_image_urls") {
        extra_start = Some(media.len());
        for u in f.parts(Some(",")) {
            media.push(json!({ "type": "image", "url": u }));
        }
    }
    if !media.is_empty() {
        variant.insert("media".to_string(), Value::Array(media));
        origins.push(("/variants/0/media/0".to_string(), "image_url".to_string()));
        if let Some(start) = extra_start {
            origins.push((
                format!("/variants/0/media/{start}"),
                "additional_image_urls".to_string(),
            ));
        }
    }
    let mut seller = Map::new();
    if let Some(v) = rec.value("seller_name") {
        seller.insert("name".to_string(), json!(v));
        origins.push((
            "/variants/0/seller/name".to_string(),
            "seller_name".to_string(),
        ));
    }
    let mut links = Vec::new();
    for (field, kind) in [
        ("seller_privacy_policy", "privacy_policy"),
        ("seller_tos", "terms_of_service"),
    ] {
        if let Some(v) = rec.value(field) {
            origins.push((
                format!("/variants/0/seller/links/{}", links.len()),
                field.to_string(),
            ));
            links.push(json!({ "type": kind, "url": v }));
        }
    }
    if !links.is_empty() {
        seller.insert("links".to_string(), Value::Array(links));
    }
    if !seller.is_empty() {
        variant.insert("seller".to_string(), Value::Object(seller));
    }
    let mut product = Map::new();
    product.insert("id".to_string(), json!(rec.product_key()));
    origins.push(("/id".to_string(), "group_id".to_string()));
    product.insert("variants".to_string(), json!([Value::Object(variant)]));
    origins.push(("/variants/0".to_string(), "item_id".to_string()));
    (Value::Object(product), origins)
}

/// The canonical field a pointer into a converted product came from:
/// the longest matching origin prefix. Media items past the first image
/// belong to additional_image_urls.
pub fn field_for_pointer(origins: &[(String, String)], pointer: &str) -> Option<String> {
    let mut best: Option<&(String, String)> = None;
    for o in origins {
        let matches = pointer == o.0 || pointer.starts_with(&format!("{}/", o.0));
        if matches && best.map(|b| o.0.len() > b.0.len()).unwrap_or(true) {
            best = Some(o);
        }
    }
    if let Some(rest) = pointer.strip_prefix("/variants/0/media/") {
        let idx: usize = rest
            .split('/')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if idx > 0 {
            if let Some(o) = origins.iter().find(|o| o.1 == "additional_image_urls") {
                return Some(o.1.clone());
            }
        }
    }
    best.map(|b| b.1.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Family, Field};
    use std::sync::Arc;

    fn rec(pairs: &[(&str, &str)]) -> Record {
        let mut r = Record::new(0, Family::Openai, 2);
        for (k, v) in pairs {
            r.set(Arc::from(*k), Field::new(*v, Arc::from(*k), 2));
        }
        r
    }

    #[test]
    fn schema_loads_and_accepts_a_clean_product() {
        let schema = AcpSchema::load().expect("schema");
        let r = rec(&[
            ("item_id", "V1"),
            ("title", "Lamp"),
            ("url", "https://example.com/lamp"),
            ("gtin", "00012345678905"),
            ("price", "79.99 USD"),
            ("availability", "pre_order"),
            ("image_url", "https://example.com/lamp.jpg"),
            ("seller_name", "Shop"),
        ]);
        let (p, _) = to_acp_product(&r);
        assert_eq!(schema.validate("Product", &p), Vec::<SchemaError>::new());
    }

    #[test]
    fn decimal_amount_and_relative_url_fail() {
        let schema = AcpSchema::load().expect("schema");
        let bad = json!({"id": "P", "variants": [{"id": "V", "title": "T", "price": {"amount": 79.99, "currency": "USD"}, "url": "/relative"}]});
        let errs = schema.validate("Product", &bad);
        assert!(errs.iter().any(|e| e.pointer == "/variants/0/price/amount"));
        assert!(errs.iter().any(|e| e.pointer == "/variants/0/url"));
    }

    #[test]
    fn pointers_map_back_to_fields() {
        let r = rec(&[
            ("item_id", "V1"),
            ("title", "T"),
            ("price", "79.999 USD"),
            ("image_url", "a"),
            ("additional_image_urls", "b,c"),
        ]);
        let (_, origins) = to_acp_product(&r);
        assert_eq!(
            field_for_pointer(&origins, "/variants/0/price/amount").as_deref(),
            Some("price")
        );
        assert_eq!(
            field_for_pointer(&origins, "/variants/0/media/0/url").as_deref(),
            Some("image_url")
        );
        assert_eq!(
            field_for_pointer(&origins, "/variants/0/media/2/url").as_deref(),
            Some("additional_image_urls")
        );
        assert_eq!(
            field_for_pointer(&origins, "/variants/0").as_deref(),
            Some("item_id")
        );
        assert_eq!(
            generic_pointer("/variants/12/media/0/url"),
            "/variants/*/media/*/url"
        );
    }
}
