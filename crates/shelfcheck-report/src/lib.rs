//! shelfcheck outputs: report.html, results.json, results.sarif,
//! failing-products.csv, junit.xml, and the terminal summary.

pub mod csv_out;
pub mod fonts;
pub mod html;
pub mod junit;
pub mod sarif;
pub mod terminal;

use std::fs;
use std::path::{Path, PathBuf};

use shelfcheck_core::error::{Error, Result};
use shelfcheck_core::results::{PackInfo, ResultsFile};

/// 1400 -> "1,400".
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "product" or "products".
pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        one.to_string()
    } else {
        many.to_string()
    }
}

/// The short date or version a pack is known by: OpenAI 2026-11-14,
/// ACP 2026-04-17, GS1 26.0.
pub fn pack_stamp(p: &PackInfo) -> String {
    match p.id.as_str() {
        "acp-feed" => p
            .version
            .split_whitespace()
            .next()
            .unwrap_or(&p.checked)
            .to_string(),
        "gs1-gtin" => p
            .version
            .split_whitespace()
            .next_back()
            .unwrap_or(&p.checked)
            .to_string(),
        _ => p.checked.clone(),
    }
}

/// Packs shown in headers and footers (not the cross-format pack).
pub fn shown_packs(packs: &[PackInfo]) -> Vec<&PackInfo> {
    packs.iter().filter(|p| p.id != "cross-format").collect()
}

/// Where each output went.
#[derive(Debug, Clone)]
pub struct Written {
    pub dir: PathBuf,
    pub results_json: PathBuf,
    pub report_html: PathBuf,
    pub sarif: PathBuf,
    pub failing_csv: PathBuf,
    pub junit: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    /// Shown in the report header ("run 2026-11-14T06:30:04Z"); never in
    /// results.json.
    pub run_at: Option<String>,
    /// `--fail-on warning`: warnings fail JUnit cases too.
    pub fail_on_warning: bool,
}

fn write(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).map_err(|e| Error::io(path, &e))
}

/// Write every output file into `dir`.
pub fn write_all(dir: &Path, results: &ResultsFile, opts: &WriteOptions) -> Result<Written> {
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, &e))?;
    let w = Written {
        dir: dir.to_path_buf(),
        results_json: dir.join("results.json"),
        report_html: dir.join("report.html"),
        sarif: dir.join("results.sarif"),
        failing_csv: dir.join("failing-products.csv"),
        junit: dir.join("junit.xml"),
    };
    let json = serde_json::to_string_pretty(results)
        .map_err(|e| Error::internal(format!("results.json: {e}")))?;
    write(&w.results_json, &format!("{json}\n"))?;
    write(&w.report_html, &html::render(results, opts)?)?;
    write(&w.sarif, &sarif::render(results)?)?;
    write(&w.failing_csv, &csv_out::render(results)?)?;
    write(&w.junit, &junit::render(results, opts.fail_on_warning)?)?;
    Ok(w)
}

/// Read an earlier results.json (for `decide`, and to keep a check and a
/// diff together in one report).
pub fn read_results(dir: &Path) -> Result<Option<ResultsFile>> {
    let p = dir.join("results.json");
    if !p.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&p).map_err(|e| Error::io(&p, &e))?;
    let r: ResultsFile = serde_json::from_str(&text).map_err(|e| {
        Error::input(format!(
            "{}: not a shelfcheck results file: {e}",
            p.display()
        ))
    })?;
    Ok(Some(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1400), "1,400");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }
}
