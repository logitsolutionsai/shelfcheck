//! CSV and TSV reader: streaming, quoted newlines, BOM handling.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::map::{self, ColumnMap};
use crate::model::{Family, Field, Record};

use super::Limits;

struct Column {
    source: Arc<str>,
    canonical: Arc<str>,
}

/// Shopify export fill-down state: the current Handle and the values of the
/// product-level columns from its first row (value, line, column index).
#[derive(Default)]
struct FillDown {
    key_col: Option<usize>,
    cols: Vec<usize>,
    variant_cols: Vec<usize>,
    current: Option<String>,
    saved: HashMap<usize, (String, u64)>,
}

/// A run of line-end bytes (CR and LF) in the file: [start, end), and the
/// offsets of the LFs in it.
struct LineEnds {
    start: u64,
    end: u64,
    lfs: Vec<u64>,
}

/// Passes bytes through and notes every run of line-end bytes. The csv
/// crate gives a record the position where it started reading, which can sit
/// before line ends it then skipped: the LF of a CRLF pair, or blank lines.
/// The runs let the reader count those and report the line the record is
/// really on.
struct LineTracker<R: Read> {
    inner: R,
    offset: u64,
    open: Option<LineEnds>,
    runs: Rc<RefCell<VecDeque<LineEnds>>>,
}

impl<R: Read> Read for LineTracker<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        let mut runs = self.runs.borrow_mut();
        for (i, &b) in buf[..n].iter().enumerate() {
            let at = self.offset + i as u64;
            if b == b'\n' || b == b'\r' {
                let run = self.open.get_or_insert_with(|| LineEnds {
                    start: at,
                    end: at,
                    lfs: Vec::new(),
                });
                if b == b'\n' {
                    run.lfs.push(at);
                }
                run.end = at + 1;
            } else if let Some(run) = self.open.take() {
                runs.push_back(run);
            }
        }
        if n == 0 {
            if let Some(run) = self.open.take() {
                runs.push_back(run);
            }
        }
        self.offset += n as u64;
        Ok(n)
    }
}

/// The line a record is really on: the csv position's line, plus the LFs it
/// skipped between that position and the record's first byte.
fn true_line(runs: &RefCell<VecDeque<LineEnds>>, byte: u64, line: u64) -> u64 {
    let mut runs = runs.borrow_mut();
    while runs.front().is_some_and(|r| r.end <= byte) {
        runs.pop_front();
    }
    match runs.front() {
        Some(r) if r.start <= byte => line + r.lfs.iter().filter(|&&o| o >= byte).count() as u64,
        _ => line,
    }
}

pub struct CsvRecords<R: Read> {
    rdr: ::csv::Reader<LineTracker<R>>,
    runs: Rc<RefCell<VecDeque<LineEnds>>>,
    columns: Vec<Column>,
    file: String,
    feed: usize,
    family: Family,
    limits: Limits,
    rows: u64,
    record: ::csv::StringRecord,
    fill: FillDown,
    done: bool,
}

impl<R: Read> CsvRecords<R> {
    pub fn new(
        reader: R,
        delimiter: u8,
        feed: usize,
        file: String,
        map: ColumnMap,
        limits: Limits,
    ) -> Result<Self> {
        let runs = Rc::new(RefCell::new(VecDeque::new()));
        let tracked = LineTracker {
            inner: reader,
            offset: 0,
            open: None,
            runs: runs.clone(),
        };
        let mut rdr = ::csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .flexible(true)
            .has_headers(true)
            .from_reader(tracked);
        let headers = rdr
            .headers()
            .map_err(|e| Error::parse(&file, 1, format!("cannot read the header row: {e}")))?
            .clone();
        let mut columns = Vec::with_capacity(headers.len());
        for h in headers.iter() {
            // csv strips a UTF-8 BOM itself; this also covers a BOM it misses.
            let clean = h.trim_start_matches('\u{feff}').trim();
            columns.push(Column {
                source: Arc::from(clean),
                canonical: map.canonical(clean),
            });
        }
        if !headers.is_empty() && !columns.iter().any(|c| map.knows(&c.source)) {
            return Err(Error::input(format!(
                "{file}: none of the header columns is a feed field shelfcheck knows; pass --format, or map the columns under [map] in shelfcheck.toml"
            )));
        }
        let mut fill = FillDown::default();
        if let Some(key) = &map.fill_down_key {
            let find = |name: &str| {
                columns
                    .iter()
                    .position(|c| c.source.eq_ignore_ascii_case(name))
            };
            fill.key_col = find(key);
            fill.cols = map.fill_down.iter().filter_map(|n| find(n)).collect();
            fill.variant_cols = map.variant_columns.iter().filter_map(|n| find(n)).collect();
        }
        Ok(CsvRecords {
            rdr,
            runs,
            columns,
            file,
            feed,
            family: map.family,
            limits,
            rows: 0,
            record: ::csv::StringRecord::new(),
            fill,
            done: false,
        })
    }

    fn next_record(&mut self) -> Result<Option<Record>> {
        loop {
            let more = self
                .rdr
                .read_record(&mut self.record)
                .map_err(|e| Error::input(format!("{}: {e}", self.file)))?;
            if !more {
                return Ok(None);
            }
            self.rows += 1;
            if self.rows > self.limits.max_rows {
                return Err(Error::input(format!(
                    "{}: more than {} rows, the row limit",
                    self.file, self.limits.max_rows
                )));
            }
            let line = match self.record.position() {
                Some(p) => true_line(&self.runs, p.byte(), p.line()),
                None => self.rows + 1,
            };
            if self.record.iter().all(|v| v.trim().is_empty()) {
                continue;
            }

            // Shopify export: skip rows without variant data, fill down
            // product-level columns.
            let mut filled: HashMap<usize, (String, u64)> = HashMap::new();
            if let Some(kc) = self.fill.key_col {
                let key = self.record.get(kc).unwrap_or("").trim().to_string();
                if self.fill.current.as_deref() != Some(key.as_str()) {
                    self.fill.current = Some(key);
                    self.fill.saved.clear();
                    for &c in &self.fill.cols {
                        if let Some(v) = self.record.get(c) {
                            if !v.trim().is_empty() {
                                self.fill.saved.insert(c, (v.to_string(), line));
                            }
                        }
                    }
                } else {
                    for &c in &self.fill.cols {
                        let empty = self
                            .record
                            .get(c)
                            .map(|v| v.trim().is_empty())
                            .unwrap_or(true);
                        if empty {
                            if let Some(saved) = self.fill.saved.get(&c) {
                                filled.insert(c, saved.clone());
                            }
                        }
                    }
                }
                if !self.fill.variant_cols.is_empty()
                    && self.fill.variant_cols.iter().all(|&c| {
                        self.record
                            .get(c)
                            .map(|v| v.trim().is_empty())
                            .unwrap_or(true)
                    })
                {
                    continue;
                }
            }

            let mut rec = Record::new(self.feed, self.family, line);
            for (i, value) in self.record.iter().enumerate() {
                let Some(col) = self.columns.get(i) else {
                    break;
                };
                if value.len() > self.limits.max_cell_bytes {
                    return Err(Error::input(format!(
                        "{}:{}: the cell in column {} is over the {} byte cell limit",
                        self.file, line, col.source, self.limits.max_cell_bytes
                    )));
                }
                let (raw, fline) = match filled.get(&i) {
                    Some((v, l)) => (v.clone(), *l),
                    None => (value.to_string(), line),
                };
                let mut field = Field::new(raw, col.source.clone(), fline);
                field.column = u32::try_from(i + 1).ok();
                map::insert_field(&mut rec, col.canonical.clone(), field);
            }
            map::extract_options(&mut rec);
            return Ok(Some(rec));
        }
    }
}

impl<R: Read> Iterator for CsvRecords<R> {
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

    fn read(text: &str, family: Family, delim: u8) -> Vec<Record> {
        let map = ColumnMap::builtin(family).expect("map");
        CsvRecords::new(
            text.as_bytes(),
            delim,
            0,
            "t.csv".to_string(),
            map,
            Limits::default(),
        )
        .expect("open")
        .collect::<Result<Vec<_>>>()
        .expect("rows")
    }

    #[test]
    fn bom_and_quoted_newlines() {
        let text =
            "\u{feff}item_id,title,description\nA1,\"Mug\",\"line one\nline two\"\nA2,Cup,plain\n";
        let recs = read(text, Family::Openai, b',');
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].value("item_id"), Some("A1"));
        assert_eq!(recs[0].value("description"), Some("line one\nline two"));
        assert_eq!(recs[0].line, 2);
        // The second record starts after the quoted newline.
        assert_eq!(recs[1].line, 4);
        assert_eq!(recs[1].get("title").and_then(|f| f.column), Some(2));
    }

    #[test]
    fn crlf_line_numbers() {
        // Hand-counted: header on line 1, A1 on 2, A2 on 3 (its description
        // runs onto line 4), A3 on 5.
        let text = "item_id,description\r\nA1,one\r\nA2,\"two\r\nlines\"\r\nA3,three\r\n";
        let recs = read(text, Family::Openai, b',');
        let lines: Vec<u64> = recs.iter().map(|r| r.line).collect();
        assert_eq!(lines, vec![2, 3, 5]);
        assert_eq!(recs[1].value("description"), Some("two\r\nlines"));
        // The same file with LF only.
        let lf = text.replace("\r\n", "\n");
        let recs = read(&lf, Family::Openai, b',');
        let lines: Vec<u64> = recs.iter().map(|r| r.line).collect();
        assert_eq!(lines, vec![2, 3, 5]);
        // Blank lines between rows are skipped but still counted.
        for gap in [
            "item_id,description\r\nA1,one\r\n\r\nA2,two\r\n",
            "item_id,description\nA1,one\n\n\nA2,two\n",
        ] {
            let recs = read(gap, Family::Openai, b',');
            let lines: Vec<u64> = recs.iter().map(|r| r.line).collect();
            let expected = if gap.contains('\r') { 4 } else { 5 };
            assert_eq!(lines, vec![2, expected], "{gap:?}");
        }
    }

    #[test]
    fn tsv_reads() {
        let recs = read(
            "id\ttitle\tlink\nG1\tLamp\thttps://example.com/l\n",
            Family::Google,
            b'\t',
        );
        assert_eq!(recs[0].value("item_id"), Some("G1"));
        assert_eq!(recs[0].value("url"), Some("https://example.com/l"));
    }

    #[test]
    fn values_kept_as_written() {
        let recs = read("item_id,gtin\nA1, 00012345678905 \n", Family::Openai, b',');
        let f = recs[0].get("gtin").expect("gtin");
        assert_eq!(f.raw, " 00012345678905 ");
        assert_eq!(f.value(), "00012345678905");
    }

    #[test]
    fn shopify_fill_down_and_image_rows() {
        let text = "Handle,Title,Vendor,Option1 Name,Option1 Value,Variant SKU,Variant Price,Variant Barcode,Image Src\n\
tab-s,\"Slate Tab S, 128 GB, Grey\",Slate,Storage,128 GB,TAB-S-128,399.00,5012345678900,https://x/1.jpg\n\
tab-s,,,,256 GB,TAB-S-256,449.00,5012345678917,\n\
tab-s,,,,,,,,https://x/2.jpg\n";
        let recs = read(text, Family::Shopify, b',');
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[1].value("title"), Some("Slate Tab S, 128 GB, Grey"));
        assert_eq!(recs[1].value("brand"), Some("Slate"));
        assert_eq!(
            recs[1].options,
            vec![("Storage".to_string(), "256 GB".to_string())]
        );
        assert_eq!(recs[1].value("group_id"), Some("tab-s"));
    }
}
