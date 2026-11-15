//! Feed readers. Every reader streams records one at a time.

pub mod acp;
pub mod csv;
pub mod detect;
pub mod jsonl;
pub mod xml;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::map::ColumnMap;
use crate::model::{Container, Family, FeedFormat, Record};

/// Safety limits (section 11). All configurable lower.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "Limits::default_file_bytes")]
    pub max_file_bytes: u64,
    #[serde(default = "Limits::default_rows")]
    pub max_rows: u64,
    #[serde(default = "Limits::default_cell_bytes")]
    pub max_cell_bytes: usize,
}

impl Limits {
    fn default_file_bytes() -> u64 {
        2 * 1024 * 1024 * 1024
    }
    fn default_rows() -> u64 {
        5_000_000
    }
    fn default_cell_bytes() -> usize {
        100 * 1024
    }

    /// The configured limits, never above the defaults.
    pub fn clamp(self) -> Self {
        let d = Limits::default();
        Limits {
            max_file_bytes: self.max_file_bytes.min(d.max_file_bytes),
            max_rows: self.max_rows.min(d.max_rows),
            max_cell_bytes: self.max_cell_bytes.min(d.max_cell_bytes),
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_bytes: Self::default_file_bytes(),
            max_rows: Self::default_rows(),
            max_cell_bytes: Self::default_cell_bytes(),
        }
    }
}

/// One input feed, as recorded in the results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Input {
    /// The path as given.
    pub path: String,
    /// The file the records come from (products.jsonl for an ACP bundle);
    /// problem locations use this path.
    pub records_path: String,
    pub format: FeedFormat,
    /// Human label, such as "OpenAI CSV".
    pub format_label: String,
    /// SHA-256 of the file (for an ACP bundle: metadata.json then products.jsonl).
    pub sha256: String,
    pub bytes: u64,
    /// For an ACP bundle, the parsed metadata.json, checked separately.
    #[serde(skip)]
    pub acp_metadata: Option<serde_json::Value>,
    /// For an ACP bundle, the metadata.json path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<String>,
}

pub type RecordIter = Box<dyn Iterator<Item = Result<Record>>>;

/// Options shared by every reader.
#[derive(Debug, Clone, Default)]
pub struct ReadOptions {
    pub format: Option<FeedFormat>,
    pub custom_map: BTreeMap<String, String>,
    pub limits: Limits,
}

fn display_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn open_file(path: &Path, limits: &Limits) -> Result<(File, u64)> {
    let f = File::open(path).map_err(|e| Error::io(path, &e))?;
    let len = f.metadata().map_err(|e| Error::io(path, &e))?.len();
    if len > limits.max_file_bytes {
        return Err(Error::input(format!(
            "{}: the file is {} bytes, over the {} byte limit",
            path.display(),
            len,
            limits.max_file_bytes
        )));
    }
    Ok((f, len))
}

/// SHA-256 of one or more files, streamed.
pub fn hash_files(paths: &[&Path]) -> Result<String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    for p in paths {
        let mut f = File::open(p).map_err(|e| Error::io(p, &e))?;
        loop {
            let n = f.read(&mut buf).map_err(|e| Error::io(p, &e))?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
    }
    Ok(format!("{:x}", h.finalize()))
}

/// Open a feed: detect its format (or use the override), and return the
/// input description plus a streaming record iterator.
pub fn open(path: &Path, feed: usize, opts: &ReadOptions) -> Result<(Input, RecordIter)> {
    if !path.exists() {
        return Err(Error::Unavailable {
            path: path.display().to_string(),
            reason: "no such file or directory".to_string(),
        });
    }
    let format = match opts.format {
        Some(f) => f,
        None => detect::detect(path)?,
    };
    let shown = display_path(path);
    let map = ColumnMap::builtin(format.family)?.with_custom(&opts.custom_map);

    if format.family == Family::Acp || format.container == Container::AcpBundle {
        let bundle = self::acp::Bundle::locate(path)?;
        let mut hashed: Vec<&Path> = Vec::new();
        if let Some(m) = &bundle.metadata {
            hashed.push(m.as_path());
        }
        hashed.push(bundle.products.as_path());
        let (file, bytes) = open_file(&bundle.products, &opts.limits)?;
        let sha256 = hash_files(&hashed)?;
        let metadata = bundle.read_metadata()?;
        let products_shown = display_path(&bundle.products);
        let input = Input {
            path: shown,
            records_path: products_shown.clone(),
            format: FeedFormat::new(Family::Acp, Container::AcpBundle),
            format_label: FeedFormat::new(Family::Acp, Container::AcpBundle).to_string(),
            sha256,
            bytes,
            acp_metadata: metadata,
            metadata_path: bundle.metadata.as_ref().map(|p| display_path(p)),
        };
        let it =
            self::acp::AcpRecords::new(BufReader::new(file), feed, products_shown, opts.limits);
        return Ok((input, Box::new(it)));
    }

    // Check the size limit before reading the file to hash it.
    let (file, bytes) = open_file(path, &opts.limits)?;
    let sha256 = hash_files(&[path])?;
    let input = Input {
        path: shown.clone(),
        records_path: shown.clone(),
        format,
        format_label: format.to_string(),
        sha256,
        bytes,
        acp_metadata: None,
        metadata_path: None,
    };
    let it: RecordIter = match format.container {
        Container::Csv | Container::Tsv => {
            let delim = if format.container == Container::Tsv {
                b'\t'
            } else {
                b','
            };
            Box::new(self::csv::CsvRecords::new(
                file,
                delim,
                feed,
                shown,
                map,
                opts.limits,
            )?)
        }
        Container::Jsonl => Box::new(self::jsonl::JsonlRecords::new(
            BufReader::new(file),
            feed,
            shown,
            map,
            opts.limits,
        )),
        Container::Xml => Box::new(self::xml::XmlRecords::new(
            BufReader::new(file),
            feed,
            shown,
            map,
            opts.limits,
        )),
        Container::AcpBundle => {
            return Err(Error::internal("ACP bundle reached the flat-file branch"));
        }
    };
    Ok((input, it))
}

/// The error for a feed with no product rows: an empty file, a header row
/// only, or a file that is not a feed. Checking it would report every
/// format as passing, which is never the right answer.
pub fn no_rows_message(path: &str) -> String {
    format!(
        "{path}: no product rows were found (the file is empty, has only a header row, or is not a feed)"
    )
}

/// Read every record of a feed into memory (used by `diff`). A feed with
/// no records is invalid input.
pub fn read_all(path: &Path, feed: usize, opts: &ReadOptions) -> Result<(Input, Vec<Record>)> {
    let (input, it) = open(path, feed, opts)?;
    let mut out = Vec::new();
    for r in it {
        out.push(r?);
    }
    if out.is_empty() {
        return Err(Error::input(no_rows_message(&input.path)));
    }
    Ok((input, out))
}
