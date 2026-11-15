//! ACP feed reader: `metadata.json` plus `products.jsonl`, one Product per
//! line. Each variant becomes one record; the product's original JSON is kept
//! on its first variant so the schema is checked once per line.

use std::collections::VecDeque;
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rust_decimal::Decimal;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::map;
use crate::model::{Family, Field, Record};
use crate::price::minor_units;

use super::Limits;

/// Where the bundle's files are.
pub struct Bundle {
    pub metadata: Option<PathBuf>,
    pub products: PathBuf,
}

impl Bundle {
    /// Accept the bundle folder, its metadata.json, or its products.jsonl.
    pub fn locate(path: &Path) -> Result<Self> {
        let dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if name == "metadata.json" {
                path.parent().map(Path::to_path_buf).unwrap_or_default()
            } else {
                // A products file given directly.
                let metadata = path
                    .parent()
                    .map(|p| p.join("metadata.json"))
                    .filter(|p| p.is_file());
                return Ok(Bundle {
                    metadata,
                    products: path.to_path_buf(),
                });
            }
        };
        let products = dir.join("products.jsonl");
        if !products.is_file() {
            return Err(Error::Unavailable {
                path: products.display().to_string(),
                reason: "an ACP bundle needs products.jsonl".to_string(),
            });
        }
        let metadata = Some(dir.join("metadata.json")).filter(|p| p.is_file());
        Ok(Bundle { metadata, products })
    }

    pub fn read_metadata(&self) -> Result<Option<Value>> {
        let Some(p) = &self.metadata else {
            return Ok(None);
        };
        let text = fs::read_to_string(p).map_err(|e| Error::io(p, &e))?;
        let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| Error::parse(&p.display().to_string(), 1, format!("not JSON: {e}")))?;
        Ok(Some(v))
    }
}

pub struct AcpRecords<R: BufRead> {
    reader: R,
    buf: String,
    line: u64,
    file: String,
    feed: usize,
    limits: Limits,
    rows: u64,
    pending: VecDeque<Record>,
    done: bool,
}

fn s(v: &Value) -> Option<&str> {
    v.as_str()
}

fn field(rec: &mut Record, name: &str, raw: String, source: &str, line: u64, pointer: String) {
    let mut f = Field::new(raw, Arc::from(source), line);
    f.pointer = Some(pointer);
    map::insert_field(rec, Arc::from(name), f);
}

/// `{amount, currency}` in minor units -> "79.99 USD". A non-integer amount
/// is kept as written ("79.99 USD"); the schema reports it.
pub fn price_text(price: &Value) -> Option<String> {
    let currency = price.get("currency").and_then(s)?;
    let amount = price.get("amount")?;
    if let Some(minor) = amount.as_i64() {
        let exp = minor_units(currency).unwrap_or(2);
        return Some(format!("{} {currency}", Decimal::new(minor, exp)));
    }
    if amount.is_number() {
        return Some(format!("{amount} {currency}"));
    }
    None
}

fn description_text(d: &Value) -> Option<(String, &'static str)> {
    for key in ["plain", "markdown", "html"] {
        if let Some(t) = d.get(key).and_then(s) {
            return Some((t.to_string(), key));
        }
    }
    None
}

/// Map one ACP product line to records, one per variant.
pub fn product_records(product: Arc<Value>, feed: usize, line: u64) -> Vec<Record> {
    let pid = product.get("id").and_then(s).map(str::to_string);
    let variants = product
        .get("variants")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    if variants.is_empty() {
        let mut rec = Record::new(feed, Family::Acp, line);
        if let Some(id) = &pid {
            field(
                &mut rec,
                "item_id",
                id.clone(),
                "id",
                line,
                "/id".to_string(),
            );
            field(
                &mut rec,
                "group_id",
                id.clone(),
                "id",
                line,
                "/id".to_string(),
            );
        }
        rec.native_json = Some(product.clone());
        out.push(rec);
        return out;
    }
    for (i, v) in variants.iter().enumerate() {
        let p = |rest: &str| format!("/variants/{i}/{rest}");
        let mut rec = Record::new(feed, Family::Acp, line);
        if let Some(id) = v.get("id").and_then(s) {
            field(
                &mut rec,
                "item_id",
                id.to_string(),
                "variants.id",
                line,
                p("id"),
            );
        }
        if let Some(id) = &pid {
            field(
                &mut rec,
                "group_id",
                id.clone(),
                "id",
                line,
                "/id".to_string(),
            );
        }
        match v.get("title").and_then(s) {
            Some(t) => field(
                &mut rec,
                "title",
                t.to_string(),
                "variants.title",
                line,
                p("title"),
            ),
            None => {
                if let Some(t) = product.get("title").and_then(s) {
                    field(
                        &mut rec,
                        "title",
                        t.to_string(),
                        "title",
                        line,
                        "/title".to_string(),
                    );
                }
            }
        }
        if let Some((d, key)) = v.get("description").and_then(description_text) {
            field(
                &mut rec,
                "description",
                d,
                "variants.description",
                line,
                p(&format!("description/{key}")),
            );
        } else if let Some((d, key)) = product.get("description").and_then(description_text) {
            field(
                &mut rec,
                "description",
                d,
                "description",
                line,
                format!("/description/{key}"),
            );
        }
        if let Some(u) = v.get("url").and_then(s) {
            field(
                &mut rec,
                "url",
                u.to_string(),
                "variants.url",
                line,
                p("url"),
            );
        } else if let Some(u) = product.get("url").and_then(s) {
            field(
                &mut rec,
                "url",
                u.to_string(),
                "url",
                line,
                "/url".to_string(),
            );
        }
        if let Some(barcodes) = v.get("barcodes").and_then(Value::as_array) {
            if let Some((j, b)) = barcodes
                .iter()
                .enumerate()
                .find(|(_, b)| b.get("value").and_then(s).is_some())
            {
                if let Some(val) = b.get("value").and_then(s) {
                    field(
                        &mut rec,
                        "gtin",
                        val.to_string(),
                        "variants.barcodes.value",
                        line,
                        p(&format!("barcodes/{j}/value")),
                    );
                }
            }
        }
        if let Some(price) = v.get("price") {
            if let Some(t) = price_text(price) {
                field(
                    &mut rec,
                    "price",
                    t,
                    "variants.price",
                    line,
                    p("price/amount"),
                );
            }
        }
        if let Some(price) = v.get("list_price") {
            if let Some(t) = price_text(price) {
                field(
                    &mut rec,
                    "list_price",
                    t,
                    "variants.list_price",
                    line,
                    p("list_price/amount"),
                );
            }
        }
        if let Some(a) = v.get("availability") {
            if let Some(st) = a.get("status").and_then(s) {
                field(
                    &mut rec,
                    "availability",
                    st.to_string(),
                    "variants.availability.status",
                    line,
                    p("availability/status"),
                );
            } else if let Some(av) = a.get("available").and_then(Value::as_bool) {
                let st = if av { "in_stock" } else { "out_of_stock" };
                field(
                    &mut rec,
                    "availability",
                    st.to_string(),
                    "variants.availability.available",
                    line,
                    p("availability/available"),
                );
            }
        }
        if let Some(c) = v
            .get("condition")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(s)
        {
            field(
                &mut rec,
                "condition",
                c.to_string(),
                "variants.condition",
                line,
                p("condition/0"),
            );
        }
        if let Some(c) = v
            .get("categories")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        {
            if let Some(val) = c.get("value").and_then(s) {
                field(
                    &mut rec,
                    "product_category",
                    val.to_string(),
                    "variants.categories.value",
                    line,
                    p("categories/0/value"),
                );
            }
        }
        let media = v
            .get("media")
            .and_then(Value::as_array)
            .map(|m| (m.clone(), true))
            .or_else(|| {
                product
                    .get("media")
                    .and_then(Value::as_array)
                    .map(|m| (m.clone(), false))
            });
        if let Some((media, own)) = media {
            let base = if own {
                format!("/variants/{i}/media")
            } else {
                "/media".to_string()
            };
            let mut extra = Vec::new();
            for (j, m) in media.iter().enumerate() {
                let Some(url) = m.get("url").and_then(s) else {
                    continue;
                };
                if j == 0 {
                    field(
                        &mut rec,
                        "image_url",
                        url.to_string(),
                        "media.url",
                        line,
                        format!("{base}/0/url"),
                    );
                } else {
                    extra.push(url.to_string());
                }
            }
            if !extra.is_empty() {
                let mut f = Field::new(extra.join(", "), Arc::from("media.url"), line);
                f.items = Some(extra);
                f.pointer = Some(format!("{base}/1/url"));
                map::insert_field(&mut rec, Arc::from("additional_image_urls"), f);
            }
        }
        if let Some(seller) = v.get("seller") {
            if let Some(name) = seller.get("name").and_then(s) {
                field(
                    &mut rec,
                    "seller_name",
                    name.to_string(),
                    "variants.seller.name",
                    line,
                    p("seller/name"),
                );
            }
        }
        if let Some(opts) = v.get("variant_options").and_then(Value::as_array) {
            for o in opts {
                if let (Some(n), Some(val)) =
                    (o.get("name").and_then(s), o.get("value").and_then(s))
                {
                    rec.options.push((n.to_string(), val.to_string()));
                }
            }
        }
        if i == 0 {
            rec.native_json = Some(product.clone());
        }
        map::extract_options(&mut rec);
        out.push(rec);
    }
    out
}

impl<R: BufRead> AcpRecords<R> {
    pub fn new(reader: R, feed: usize, file: String, limits: Limits) -> Self {
        AcpRecords {
            reader,
            buf: String::new(),
            line: 0,
            file,
            feed,
            limits,
            rows: 0,
            pending: VecDeque::new(),
            done: false,
        }
    }

    fn fill(&mut self) -> Result<bool> {
        loop {
            self.buf.clear();
            let n = self.reader.read_line(&mut self.buf).map_err(|e| {
                Error::parse(
                    &self.file,
                    self.line + 1,
                    format!("cannot read the line as UTF-8: {e}"),
                )
            })?;
            if n == 0 {
                return Ok(false);
            }
            self.line += 1;
            let text = self.buf.trim_start_matches('\u{feff}').trim();
            if text.is_empty() {
                continue;
            }
            if text.len() > self.limits.max_cell_bytes.saturating_mul(10) {
                return Err(Error::input(format!(
                    "{}:{}: the product line is over {} bytes",
                    self.file,
                    self.line,
                    self.limits.max_cell_bytes.saturating_mul(10)
                )));
            }
            let v: Value = serde_json::from_str(text)
                .map_err(|e| Error::parse(&self.file, self.line, format!("not JSON: {e}")))?;
            let recs = product_records(Arc::new(v), self.feed, self.line);
            self.rows += recs.len() as u64;
            if self.rows > self.limits.max_rows {
                return Err(Error::input(format!(
                    "{}: more than {} rows, the row limit",
                    self.file, self.limits.max_rows
                )));
            }
            self.pending.extend(recs);
            return Ok(true);
        }
    }
}

impl<R: BufRead> Iterator for AcpRecords<R> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(r) = self.pending.pop_front() {
                return Some(Ok(r));
            }
            if self.done {
                return None;
            }
            match self.fill() {
                Ok(true) => continue,
                Ok(false) => {
                    self.done = true;
                    return None;
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_become_records() {
        let line = r#"{"id":"P1","title":"Lamp","variants":[{"id":"V1","title":"Lamp White","barcodes":[{"type":"GTIN","value":"00012345678905"}],"price":{"amount":7999,"currency":"USD"},"availability":{"available":true,"status":"in_stock"},"variant_options":[{"name":"color","value":"White"}]},{"id":"V2","title":"Lamp Black","price":{"amount":79.99,"currency":"USD"}}]}"#;
        let text = format!("{line}\n");
        let recs: Vec<Record> =
            AcpRecords::new(text.as_bytes(), 0, "p.jsonl".into(), Limits::default())
                .collect::<Result<Vec<_>>>()
                .expect("rows");
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].value("item_id"), Some("V1"));
        assert_eq!(recs[0].value("group_id"), Some("P1"));
        assert_eq!(recs[0].value("price"), Some("79.99 USD"));
        assert_eq!(
            recs[0]
                .get("gtin")
                .and_then(|f| f.pointer.clone())
                .as_deref(),
            Some("/variants/0/barcodes/0/value")
        );
        assert!(recs[0].native_json.is_some());
        assert!(recs[1].native_json.is_none());
        assert_eq!(recs[1].value("price"), Some("79.99 USD"));
        assert_eq!(
            recs[0].options,
            vec![("color".to_string(), "White".to_string())]
        );
    }
}
