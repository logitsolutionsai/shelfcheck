//! Format detection by file name, headers or root element. `--format`
//! overrides it.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::error::{Error, Result};
use crate::map::norm_key;
use crate::model::{Container, Family, FeedFormat};

const OPENAI_MARKERS: &[&str] = &[
    "item_id",
    "target_countries",
    "store_country",
    "seller_name",
    "image_url",
    "variant_dict",
    "is_eligible_checkout",
    "group_id",
    "listing_has_variations",
    "additional_image_urls",
];

const GOOGLE_MARKERS: &[&str] = &[
    "link",
    "image_link",
    "item_group_id",
    "additional_image_link",
    "google_product_category",
    "identifier_exists",
    "variant_option",
];

/// Decide the family from a list of column or key names.
pub fn family_from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Family {
    let raw: Vec<String> = names
        .into_iter()
        .map(|s| s.trim().trim_start_matches('\u{feff}').to_string())
        .collect();
    let lower: Vec<String> = raw.iter().map(|s| s.to_ascii_lowercase()).collect();
    if lower.iter().any(|h| h == "handle")
        && lower
            .iter()
            .any(|h| h == "variant sku" || h == "variant price" || h == "variant barcode")
    {
        return Family::Shopify;
    }
    let normed: Vec<String> = raw.iter().map(|s| norm_key(s)).collect();
    let prefixed = lower.iter().filter(|h| h.starts_with("g:")).count();
    let openai = normed
        .iter()
        .filter(|h| OPENAI_MARKERS.contains(&h.as_str()))
        .count();
    let google = normed
        .iter()
        .filter(|h| GOOGLE_MARKERS.contains(&h.as_str()))
        .count()
        + prefixed;
    if openai == 0 && google == 0 {
        Family::Generic
    } else if google > openai {
        Family::Google
    } else {
        Family::Openai
    }
}

fn first_line(path: &Path) -> Result<String> {
    let f = File::open(path).map_err(|e| Error::io(path, &e))?;
    let mut r = BufReader::new(f);
    let mut line = String::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line).map_err(|e| {
            Error::input(format!(
                "{}: cannot read the first line as UTF-8: {e}",
                path.display()
            ))
        })?;
        if n == 0 {
            return Ok(String::new());
        }
        if !line.trim().is_empty() {
            return Ok(line
                .trim_start_matches('\u{feff}')
                .trim_end_matches(['\r', '\n'])
                .to_string());
        }
    }
}

fn head_bytes(path: &Path, n: usize) -> Result<Vec<u8>> {
    let f = File::open(path).map_err(|e| Error::io(path, &e))?;
    let mut buf = Vec::with_capacity(n);
    f.take(n as u64)
        .read_to_end(&mut buf)
        .map_err(|e| Error::io(path, &e))?;
    Ok(buf)
}

/// Split a CSV header line. Good enough for detection; the real reader
/// handles quoting fully.
fn split_header(line: &str, delim: char) -> Vec<String> {
    line.split(delim)
        .map(|s| s.trim().trim_matches('"').to_string())
        .collect()
}

pub fn detect(path: &Path) -> Result<FeedFormat> {
    if path.is_dir() {
        if path.join("products.jsonl").is_file() {
            return Ok(FeedFormat::new(Family::Acp, Container::AcpBundle));
        }
        return Err(Error::input(format!(
            "{}: a folder is read as an ACP bundle, but it has no products.jsonl",
            path.display()
        )));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name == "metadata.json" {
        return Ok(FeedFormat::new(Family::Acp, Container::AcpBundle));
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let container = match ext.as_str() {
        "xml" | "rss" | "atom" => Container::Xml,
        "jsonl" | "ndjson" => Container::Jsonl,
        "tsv" | "tab" => Container::Tsv,
        "csv" => Container::Csv,
        _ => {
            let head = head_bytes(path, 4096)?;
            let text = String::from_utf8_lossy(&head);
            let t = text.trim_start_matches('\u{feff}').trim_start();
            if t.starts_with('<') {
                Container::Xml
            } else if t.starts_with('{') {
                Container::Jsonl
            } else if t.lines().next().map(|l| l.contains('\t')).unwrap_or(false) {
                Container::Tsv
            } else {
                Container::Csv
            }
        }
    };
    match container {
        Container::Xml => {
            let head = head_bytes(path, 8192)?;
            let text = String::from_utf8_lossy(&head);
            if text.contains("<rss") || text.contains("<feed") {
                Ok(FeedFormat::new(Family::Google, Container::Xml))
            } else {
                Err(Error::input(format!(
                    "{}: XML feeds are read as Google Merchant Center RSS 2.0 or Atom, but no <rss> or <feed> root was found",
                    path.display()
                )))
            }
        }
        Container::Jsonl => {
            let line = first_line(path)?;
            if line.is_empty() {
                return Ok(FeedFormat::new(Family::Generic, Container::Jsonl));
            }
            let v: serde_json::Value = serde_json::from_str(&line).map_err(|e| {
                Error::parse(&path.display().to_string(), 1, format!("not JSON: {e}"))
            })?;
            let Some(obj) = v.as_object() else {
                return Err(Error::parse(
                    &path.display().to_string(),
                    1,
                    "each line must be a JSON object",
                ));
            };
            if obj.get("variants").map(|v| v.is_array()).unwrap_or(false) {
                return Ok(FeedFormat::new(Family::Acp, Container::AcpBundle));
            }
            let fam = family_from_names(obj.keys().map(|k| k.as_str()));
            Ok(FeedFormat::new(fam, Container::Jsonl))
        }
        Container::Csv | Container::Tsv => {
            let line = first_line(path)?;
            if line.contains(';') && !line.contains(',') && !line.contains('\t') {
                return Err(Error::input(format!(
                    "{}: the header row is separated by semicolons; shelfcheck reads comma-separated CSV and tab-separated TSV. Export the feed with commas or tabs",
                    path.display()
                )));
            }
            let container = if line.contains('\t') {
                Container::Tsv
            } else {
                container
            };
            let delim = if container == Container::Tsv {
                '\t'
            } else {
                ','
            };
            let headers = split_header(&line, delim);
            let fam = family_from_names(headers.iter().map(|s| s.as_str()));
            Ok(FeedFormat::new(fam, container))
        }
        Container::AcpBundle => Ok(FeedFormat::new(Family::Acp, Container::AcpBundle)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_from_headers() {
        assert_eq!(
            family_from_names(["item_id", "title", "target_countries"]),
            Family::Openai
        );
        assert_eq!(
            family_from_names(["id", "title", "link", "image_link"]),
            Family::Google
        );
        assert_eq!(family_from_names(["g:id", "g:title"]), Family::Google);
        assert_eq!(
            family_from_names(["Handle", "Title", "Variant SKU"]),
            Family::Shopify
        );
        assert_eq!(family_from_names(["sku", "name"]), Family::Generic);
        assert_eq!(
            family_from_names(["\u{feff}item_id", "title"]),
            Family::Openai
        );
    }
}
