//! Google Merchant Center XML reader: RSS 2.0 `<item>` or Atom `<entry>`
//! elements with `g:` attributes. Streaming, CDATA and namespaces handled.
//! Values come from element text only; XML attributes are not read, so an
//! Atom `<link href="..."/>` gives no link (use `<g:link>` or RSS `<link>`).
//! No DTDs and no external entities: only the five predefined entities and
//! character references are resolved; anything else is an error.

use std::io::{self, BufRead, Read};
use std::sync::Arc;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{Error, Result};
use crate::map::{self, ColumnMap};
use crate::model::{Family, Field, Record};

use super::Limits;

/// Counts the newlines in the bytes the XML reader has consumed, so every
/// element can be given a line number without holding the file in memory.
pub struct LineCounter<R> {
    inner: R,
    newlines: u64,
}

impl<R> LineCounter<R> {
    pub fn new(inner: R) -> Self {
        LineCounter { inner, newlines: 0 }
    }

    /// The 1-based line of the next unread byte.
    pub fn line(&self) -> u64 {
        self.newlines + 1
    }
}

impl<R: BufRead> Read for LineCounter<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.newlines += buf[..n].iter().filter(|b| **b == b'\n').count() as u64;
        Ok(n)
    }
}

impl<R: BufRead> BufRead for LineCounter<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        let count = match self.inner.fill_buf() {
            Ok(b) => b[..amt.min(b.len())]
                .iter()
                .filter(|c| **c == b'\n')
                .count() as u64,
            Err(_) => 0,
        };
        self.newlines += count;
        self.inner.consume(amt);
    }
}

/// An XML event copied out of the reader's buffer.
enum Ev {
    Start(String),
    End(String),
    Text(String),
    BadRef(String),
    Eof,
    Other,
}

struct Open {
    qname: String,
    line: u64,
    text: String,
    has_children: bool,
}

pub struct XmlRecords<R: BufRead> {
    reader: Reader<LineCounter<R>>,
    buf: Vec<u8>,
    file: String,
    feed: usize,
    family: Family,
    map: ColumnMap,
    limits: Limits,
    rows: u64,
    current: Option<Record>,
    stack: Vec<Open>,
    done: bool,
}

fn local(qname: &str) -> &str {
    match qname.rsplit_once(':') {
        Some((_, l)) => l,
        None => qname,
    }
}

impl<R: BufRead> XmlRecords<R> {
    pub fn new(reader: R, feed: usize, file: String, map: ColumnMap, limits: Limits) -> Self {
        let mut reader = Reader::from_reader(LineCounter::new(reader));
        // No trim_text: entity references arrive as separate events, and
        // trimming each text piece would drop the spaces around them.
        // Values are trimmed when compared; the raw text is kept as written.
        reader.config_mut().expand_empty_elements = true;
        XmlRecords {
            reader,
            buf: Vec::new(),
            file,
            feed,
            family: map.family,
            map,
            limits,
            rows: 0,
            current: None,
            stack: Vec::new(),
            done: false,
        }
    }

    fn line(&self) -> u64 {
        self.reader.get_ref().line()
    }

    fn err(&self, msg: impl std::fmt::Display) -> Error {
        Error::parse(&self.file, self.line(), msg)
    }

    fn push_text(&mut self, t: &str) -> Result<()> {
        if let Some(top) = self.stack.last_mut() {
            top.text.push_str(t);
            if top.text.len() > self.limits.max_cell_bytes {
                let name = top.qname.clone();
                return Err(self.err(format!(
                    "the value of <{name}> is over the {} byte cell limit",
                    self.limits.max_cell_bytes
                )));
            }
        }
        Ok(())
    }

    /// Read one event and copy what we need out of the buffer, so the
    /// buffer borrow ends before the event is handled.
    fn next_event(&mut self) -> Result<Ev> {
        self.buf.clear();
        let line_before = self.reader.get_ref().line();
        let event = self
            .reader
            .read_event_into(&mut self.buf)
            .map_err(|e| Error::parse(&self.file, line_before, format!("invalid XML: {e}")))?;
        let ev = match event {
            Event::Start(e) => Ev::Start(e.name().into_inner().to_string()),
            Event::End(e) => Ev::End(e.name().into_inner().to_string()),
            Event::Text(t) => Ev::Text(t.xml10_content().into_owned()),
            Event::CData(c) => Ev::Text(c.into_inner().into_owned()),
            Event::GeneralRef(r) => {
                if r.is_char_ref() {
                    match r.resolve_char_ref() {
                        Ok(Some(ch)) => Ev::Text(ch.to_string()),
                        Ok(None) | Err(_) => {
                            Ev::BadRef(format!("invalid character reference &{};", &*r))
                        }
                    }
                } else {
                    let name: &str = &r;
                    match quick_xml::escape::resolve_xml_entity(name) {
                        Some(s) => Ev::Text(s.to_string()),
                        None => Ev::BadRef(format!(
                            "unknown entity &{name};. Only the five XML entities are allowed; DTD entities are never expanded"
                        )),
                    }
                }
            }
            Event::Eof => Ev::Eof,
            // Declarations, comments, processing instructions and any
            // DOCTYPE are ignored; DTD content is never processed.
            _ => Ev::Other,
        };
        Ok(ev)
    }

    fn next_record(&mut self) -> Result<Option<Record>> {
        loop {
            let ev = self.next_event()?;
            let line = self.line();
            match ev {
                Ev::Start(qname) => {
                    if self.current.is_none() {
                        let l = local(&qname);
                        if l == "item" || l == "entry" {
                            self.rows += 1;
                            if self.rows > self.limits.max_rows {
                                return Err(Error::input(format!(
                                    "{}: more than {} items, the row limit",
                                    self.file, self.limits.max_rows
                                )));
                            }
                            self.current = Some(Record::new(self.feed, self.family, line));
                            self.stack.clear();
                        }
                    } else {
                        if let Some(parent) = self.stack.last_mut() {
                            parent.has_children = true;
                        }
                        self.stack.push(Open {
                            qname,
                            line,
                            text: String::new(),
                            has_children: false,
                        });
                    }
                }
                Ev::Text(text) => {
                    if self.current.is_some() {
                        self.push_text(&text)?;
                    }
                }
                Ev::BadRef(msg) => {
                    if self.current.is_some() {
                        return Err(self.err(msg));
                    }
                }
                Ev::End(qname) => {
                    if self.current.is_none() {
                        continue;
                    }
                    match self.stack.pop() {
                        Some(open) => {
                            // Only direct children of the item become fields;
                            // nested groups such as <g:shipping> are skipped.
                            if self.stack.is_empty() && !open.has_children {
                                let source: Arc<str> = Arc::from(open.qname.as_str());
                                let canonical = self.map.canonical(local(&open.qname));
                                let field = Field::new(open.text, source, open.line);
                                if let Some(rec) = self.current.as_mut() {
                                    map::insert_field(rec, canonical, field);
                                }
                            } else if self.stack.is_empty()
                                && local(&open.qname) == "question_and_answer"
                            {
                                // Not read in this version, but its presence
                                // is kept so the run can say it was not read.
                                let source: Arc<str> = Arc::from(open.qname.as_str());
                                let canonical = self.map.canonical(local(&open.qname));
                                let field =
                                    Field::new("(nested group, not read)", source, open.line);
                                if let Some(rec) = self.current.as_mut() {
                                    map::insert_field(rec, canonical, field);
                                }
                            }
                        }
                        None => {
                            let l = local(&qname);
                            if l == "item" || l == "entry" {
                                if let Some(mut rec) = self.current.take() {
                                    map::extract_options(&mut rec);
                                    return Ok(Some(rec));
                                }
                            }
                        }
                    }
                }
                Ev::Eof => {
                    if self.current.is_some() {
                        return Err(self.err("the file ended inside an item"));
                    }
                    return Ok(None);
                }
                Ev::Other => {}
            }
        }
    }
}

impl<R: BufRead> Iterator for XmlRecords<R> {
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

    fn read(text: &str) -> Vec<Record> {
        let map = ColumnMap::builtin(Family::Google).expect("map");
        XmlRecords::new(text.as_bytes(), 0, "t.xml".into(), map, Limits::default())
            .collect::<Result<Vec<_>>>()
            .expect("items")
    }

    #[test]
    fn rss_items_with_namespace_cdata_and_entities() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:g="http://base.google.com/ns/1.0">
<channel>
<title>Store</title>
<item>
<g:id>TEE-BLK-M</g:id>
<title><![CDATA[Tee & Co <Black>]]></title>
<link>https://example.com/tee?a=1&amp;b=2</link>
<g:price>15.00 USD</g:price>
<g:additional_image_link>https://example.com/1.jpg</g:additional_image_link>
<g:additional_image_link>https://example.com/2.jpg</g:additional_image_link>
<g:shipping><g:country>US</g:country></g:shipping>
<g:gtin/>
</item>
<item>
<g:id>MUG-1</g:id>
</item>
</channel>
</rss>
"#;
        let recs = read(text);
        assert_eq!(recs.len(), 2);
        let r = &recs[0];
        assert_eq!(r.line, 5);
        assert_eq!(r.value("item_id"), Some("TEE-BLK-M"));
        assert_eq!(r.get("item_id").map(|f| f.line), Some(6));
        assert_eq!(
            r.get("item_id").map(|f| f.source.to_string()).as_deref(),
            Some("g:id")
        );
        assert_eq!(r.value("title"), Some("Tee & Co <Black>"));
        assert_eq!(r.value("url"), Some("https://example.com/tee?a=1&b=2"));
        assert_eq!(
            r.get("additional_image_urls")
                .and_then(|f| f.items.clone())
                .map(|v| v.len()),
            Some(2)
        );
        assert!(r.get("shipping").is_none());
        assert!(r.get("gtin").map(|f| !f.is_present()).unwrap_or(false));
        assert_eq!(recs[1].line, 15);
    }

    #[test]
    fn unknown_entities_are_rejected() {
        let text = "<rss><channel><item><g:id>&ext;</g:id></item></channel></rss>";
        let map = ColumnMap::builtin(Family::Google).expect("map");
        let out: Result<Vec<Record>> =
            XmlRecords::new(text.as_bytes(), 0, "t.xml".into(), map, Limits::default()).collect();
        assert!(out.is_err());
    }
}
