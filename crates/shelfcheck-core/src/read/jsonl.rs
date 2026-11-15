//! JSONL reader: one JSON object per line, flat OpenAI or Google keys.

use std::io::BufRead;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::map::{self, ColumnMap};
use crate::model::{Family, Field, Record};

use super::Limits;

pub struct JsonlRecords<R: BufRead> {
    reader: R,
    buf: String,
    line: u64,
    file: String,
    feed: usize,
    family: Family,
    map: ColumnMap,
    limits: Limits,
    rows: u64,
    done: bool,
}

/// Escape a key for a JSON pointer segment (RFC 6901).
pub fn pointer_segment(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// A JSON value as feed text: strings as is, numbers and booleans as JSON,
/// arrays as separate items, objects as JSON text.
pub fn value_to_field(
    v: &serde_json::Value,
    source: Arc<str>,
    line: u64,
    pointer: String,
) -> Option<Field> {
    let mut field = match v {
        serde_json::Value::Null => return None,
        serde_json::Value::String(s) => Field::new(s.clone(), source, line),
        serde_json::Value::Bool(b) => Field::new(b.to_string(), source, line),
        serde_json::Value::Number(n) => Field::new(n.to_string(), source, line),
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|i| match i {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect();
            let mut f = Field::new(parts.join(", "), source, line);
            f.items = Some(parts);
            f
        }
        serde_json::Value::Object(_) => Field::new(v.to_string(), source, line),
    };
    field.pointer = Some(pointer);
    Some(field)
}

impl<R: BufRead> JsonlRecords<R> {
    pub fn new(reader: R, feed: usize, file: String, map: ColumnMap, limits: Limits) -> Self {
        JsonlRecords {
            reader,
            buf: String::new(),
            line: 0,
            file,
            feed,
            family: map.family,
            map,
            limits,
            rows: 0,
            done: false,
        }
    }

    fn next_record(&mut self) -> Result<Option<Record>> {
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
                return Ok(None);
            }
            self.line += 1;
            let text = self.buf.trim_start_matches('\u{feff}').trim();
            if text.is_empty() {
                continue;
            }
            self.rows += 1;
            if self.rows > self.limits.max_rows {
                return Err(Error::input(format!(
                    "{}: more than {} rows, the row limit",
                    self.file, self.limits.max_rows
                )));
            }
            let v: serde_json::Value = serde_json::from_str(text)
                .map_err(|e| Error::parse(&self.file, self.line, format!("not JSON: {e}")))?;
            let serde_json::Value::Object(obj) = v else {
                return Err(Error::parse(
                    &self.file,
                    self.line,
                    "each line must be a JSON object",
                ));
            };
            let mut rec = Record::new(self.feed, self.family, self.line);
            for (k, val) in &obj {
                let source: Arc<str> = Arc::from(k.as_str());
                let canonical = self.map.canonical(k);
                if let Some(field) =
                    value_to_field(val, source, self.line, format!("/{}", pointer_segment(k)))
                {
                    if field.raw.len() > self.limits.max_cell_bytes {
                        return Err(Error::input(format!(
                            "{}:{}: the value of {k} is over the {} byte cell limit",
                            self.file, self.line, self.limits.max_cell_bytes
                        )));
                    }
                    map::insert_field(&mut rec, canonical, field);
                }
            }
            map::extract_options(&mut rec);
            return Ok(Some(rec));
        }
    }
}

impl<R: BufRead> Iterator for JsonlRecords<R> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.next_record() {
            Ok(Some(r)) => Some(Ok(r)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_flat_objects() {
        let text = "{\"id\": \"A1\", \"price\": \"79.99 USD\", \"variant_dict\": {\"storage\": \"128 GB\"}, \"additional_image_urls\": [\"https://x/1.jpg\", \"https://x/2.jpg\"]}\n\n{\"item_id\": \"A2\", \"gtin\": \"00012345678905\"}\n";
        let map = ColumnMap::builtin(Family::Openai).expect("map");
        let recs: Vec<Record> =
            JsonlRecords::new(text.as_bytes(), 0, "t.jsonl".into(), map, Limits::default())
                .collect::<Result<Vec<_>>>()
                .expect("rows");
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].value("item_id"), Some("A1"));
        assert_eq!(
            recs[0].options,
            vec![("storage".to_string(), "128 GB".to_string())]
        );
        assert_eq!(
            recs[0]
                .get("additional_image_urls")
                .and_then(|f| f.items.clone())
                .map(|v| v.len()),
            Some(2)
        );
        assert_eq!(recs[1].line, 3);
        assert_eq!(
            recs[1]
                .get("gtin")
                .and_then(|f| f.pointer.clone())
                .as_deref(),
            Some("/gtin")
        );
    }
}
