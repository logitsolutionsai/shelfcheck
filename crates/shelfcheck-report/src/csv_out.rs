//! failing-products.csv: one row per failing item, with its formats, rule
//! ids and sentences. An item fails when it has at least one error.

use std::collections::{BTreeMap, BTreeSet};

use shelfcheck_core::error::{Error, Result};
use shelfcheck_core::model::Severity;
use shelfcheck_core::results::{Problem, ResultsFile};

pub const HEADER: [&str; 8] = [
    "item_id",
    "product",
    "file",
    "line",
    "formats",
    "severity",
    "rule_ids",
    "sentences",
];

pub fn render(results: &ResultsFile) -> Result<String> {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(HEADER)
        .map_err(|e| Error::internal(format!("failing-products.csv: {e}")))?;
    if let Some(c) = &results.check {
        // Group by (feed, item); keep first-seen order, which is sorted.
        let mut order: Vec<(usize, String)> = Vec::new();
        let mut groups: BTreeMap<(usize, String), Vec<&Problem>> = BTreeMap::new();
        for p in &c.problems {
            let k = (p.feed, p.item_id.clone());
            if !groups.contains_key(&k) {
                order.push(k.clone());
            }
            groups.entry(k).or_default().push(p);
        }
        for k in order {
            let Some(ps) = groups.get(&k) else { continue };
            if !ps.iter().any(|p| p.severity == Severity::Error) {
                continue;
            }
            let Some(first) = ps.first() else { continue };
            let formats: BTreeSet<&str> = ps
                .iter()
                .flat_map(|p| p.formats.iter().map(|f| f.id()))
                .collect();
            let mut rules: Vec<&str> = Vec::new();
            for p in ps.iter() {
                if !rules.contains(&p.rule.as_str()) {
                    rules.push(p.rule.as_str());
                }
            }
            let sentences: Vec<&str> = ps.iter().map(|p| p.message.as_str()).collect();
            let line = first.location.line.to_string();
            let formats = formats.into_iter().collect::<Vec<_>>().join(";");
            let rules = rules.join(";");
            let sentences = sentences.join(" | ");
            w.write_record([
                first.item_id.as_str(),
                first.product.as_str(),
                first.location.file.as_str(),
                line.as_str(),
                formats.as_str(),
                "error",
                rules.as_str(),
                sentences.as_str(),
            ])
            .map_err(|e| Error::internal(format!("failing-products.csv: {e}")))?;
        }
    }
    let bytes = w
        .into_inner()
        .map_err(|e| Error::internal(format!("failing-products.csv: {e}")))?;
    String::from_utf8(bytes).map_err(|e| Error::internal(format!("failing-products.csv: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_only_when_clean() {
        let text = render(&ResultsFile::new()).expect("csv");
        assert_eq!(
            text,
            "item_id,product,file,line,formats,severity,rule_ids,sentences\n"
        );
    }
}
