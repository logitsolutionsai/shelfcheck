//! SARIF 2.1.0 for GitHub code scanning, written with our own serde types.
//! One result per problem, located at file, line and column. Open diff
//! changes that need a decision (or are warnings) are added as results too,
//! under rule ids `diff.<kind>`.

use std::collections::BTreeMap;

use serde::Serialize;
use shelfcheck_core::diff::{Level, Status};
use shelfcheck_core::error::{Error, Result};
use shelfcheck_core::model::{Location, Severity};
use shelfcheck_core::results::{ResultsFile, VERSION};

pub const SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";

#[derive(Serialize)]
pub struct Sarif {
    #[serde(rename = "$schema")]
    pub schema: &'static str,
    pub version: &'static str,
    pub runs: Vec<Run>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub tool: Tool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub invocations: Vec<Invocation>,
    pub results: Vec<SarifResult>,
}

/// Only written when results were left out, to say so.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Invocation {
    pub execution_successful: bool,
    pub tool_execution_notifications: Vec<Notification>,
}

#[derive(Serialize)]
pub struct Notification {
    pub level: &'static str,
    pub message: Text,
}

/// GitHub code scanning rejects a run with more than 25,000 results.
pub const MAX_RESULTS: usize = 25_000;

#[derive(Serialize)]
pub struct Tool {
    pub driver: Driver,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Driver {
    pub name: &'static str,
    pub version: &'static str,
    pub information_uri: &'static str,
    pub rules: Vec<ReportingDescriptor>,
}

#[derive(Serialize)]
pub struct Text {
    pub text: String,
}

#[derive(Serialize)]
pub struct Configuration {
    pub level: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportingDescriptor {
    pub id: String,
    pub short_description: Text,
    pub full_description: Text,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_uri: Option<String>,
    pub default_configuration: Configuration,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SarifResult {
    pub rule_id: String,
    pub rule_index: usize,
    pub level: &'static str,
    pub message: Text,
    pub locations: Vec<SarifLocation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_locations: Vec<SarifLocation>,
    pub partial_fingerprints: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SarifLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<usize>,
    pub physical_location: PhysicalLocation,
    /// The cell, such as "column L (gtin)". A CSV column number is a cell
    /// index, not a character column, so it is not put in the region.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Text>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalLocation {
    pub artifact_location: ArtifactLocation,
    pub region: Region,
}

#[derive(Serialize)]
pub struct ArtifactLocation {
    pub uri: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub start_line: u64,
}

fn level(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

/// Percent-encode what a URI path cannot hold as it is (spaces, %, #, ?,
/// non-ASCII), byte by byte.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~!$&'()*+,;=:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A forward-slash URI reference: relative paths stay relative (code
/// scanning resolves them against the repository), absolute paths become
/// file:// URIs, and characters a URI cannot hold are percent-encoded.
pub fn uri(file: &str) -> String {
    let f = file.replace('\\', "/");
    let f = f.strip_prefix("./").unwrap_or(&f);
    let b = f.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/';
    if drive {
        format!("file:///{}", encode_path(f))
    } else if f.starts_with('/') {
        format!("file://{}", encode_path(f))
    } else {
        encode_path(f)
    }
}

fn location(l: &Location, id: Option<usize>) -> SarifLocation {
    let letters = l.column.map(shelfcheck_core::model::column_letters);
    let cell = match (letters, &l.column_name, &l.pointer) {
        (_, _, Some(p)) => Some(format!("JSON pointer {p}")),
        (Some(c), Some(n), None) => Some(format!("column {c} ({n})")),
        (Some(c), None, None) => Some(format!("column {c}")),
        (None, Some(n), None) => Some(n.clone()),
        (None, None, None) => None,
    };
    SarifLocation {
        id,
        physical_location: PhysicalLocation {
            artifact_location: ArtifactLocation { uri: uri(&l.file) },
            region: Region {
                start_line: l.line.max(1),
            },
        },
        message: cell.map(|text| Text { text }),
    }
}

pub fn build(results: &ResultsFile) -> Sarif {
    let mut rules: Vec<ReportingDescriptor> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut out: Vec<SarifResult> = Vec::new();

    if let Some(c) = &results.check {
        // Rule descriptors for every rule that produced a problem, in id order.
        let mut ids: Vec<(&str, Severity, &str)> = c
            .problems
            .iter()
            .map(|p| (p.rule.as_str(), p.severity, p.fix.as_str()))
            .collect();
        ids.sort();
        ids.dedup_by(|a, b| a.0 == b.0);
        for (id, sev, fix) in ids {
            let f = c.fixes.iter().find(|f| f.key == fix);
            let title = f.map(|f| f.title.clone()).unwrap_or_else(|| id.to_string());
            let cite = f.and_then(|f| f.cites.first().cloned()).unwrap_or_default();
            let help = f.and_then(|f| f.links.first().map(|l| l.url.clone()));
            index.insert(id.to_string(), rules.len());
            rules.push(ReportingDescriptor {
                id: id.to_string(),
                short_description: Text { text: title },
                full_description: Text {
                    text: if cite.is_empty() {
                        id.to_string()
                    } else {
                        cite
                    },
                },
                help_uri: help,
                default_configuration: Configuration { level: level(sev) },
            });
        }
        for p in &c.problems {
            let mut fp = BTreeMap::new();
            fp.insert(
                "shelfcheck/v1".to_string(),
                format!(
                    "{}:{}:{}",
                    p.rule,
                    p.item_id,
                    p.field.clone().unwrap_or_default()
                ),
            );
            out.push(SarifResult {
                rule_id: p.rule.clone(),
                rule_index: index.get(&p.rule).copied().unwrap_or(0),
                level: level(p.severity),
                message: Text {
                    text: p.message.clone(),
                },
                locations: vec![location(&p.location, None)],
                related_locations: p.related.iter().map(|l| location(l, Some(1))).collect(),
                partial_fingerprints: fp,
            });
        }
    }

    if let Some(d) = &results.diff {
        for c in d
            .changes
            .iter()
            .filter(|c| c.status == Status::Open && c.level != Level::Info)
        {
            let rule_id = format!("diff.{}", c.kind.id());
            let idx = match index.get(&rule_id) {
                Some(i) => *i,
                None => {
                    let i = rules.len();
                    index.insert(rule_id.clone(), i);
                    rules.push(ReportingDescriptor {
                        id: rule_id.clone(),
                        short_description: Text { text: format!("diff: {}", c.kind.id()) },
                        full_description: Text {
                            text: "An identity change since the last feed. Record a decision with shelfcheck decide.".to_string(),
                        },
                        help_uri: None,
                        default_configuration: Configuration {
                            level: if c.level == Level::Decision { "error" } else { "warning" },
                        },
                    });
                    i
                }
            };
            let loc = c
                .new
                .as_ref()
                .or(c.old.as_ref())
                .map(|s| location(&s.location, None));
            let mut fp = BTreeMap::new();
            fp.insert(
                "shelfcheck/v1".to_string(),
                format!("{}:{}:{}", rule_id, c.item_id, c.fingerprint),
            );
            let mut text = c.sentence.clone();
            if let Some(cmd) = c.commands.first() {
                text.push_str(&format!(" Decide with: {cmd}"));
            }
            out.push(SarifResult {
                rule_id,
                rule_index: idx,
                level: if c.level == Level::Decision {
                    "error"
                } else {
                    "warning"
                },
                message: Text { text },
                locations: loc.into_iter().collect(),
                related_locations: Vec::new(),
                partial_fingerprints: fp,
            });
        }
    }

    // Problems are sorted errors first, so a cut keeps the most serious.
    let mut invocations = Vec::new();
    if out.len() > MAX_RESULTS {
        let left_out = out.len() - MAX_RESULTS;
        out.truncate(MAX_RESULTS);
        invocations.push(Invocation {
            execution_successful: true,
            tool_execution_notifications: vec![Notification {
                level: "warning",
                message: Text {
                    text: format!(
                        "{left_out} more results were left out: GitHub code scanning accepts at most {MAX_RESULTS} per run. results.json and failing-products.csv hold every problem."
                    ),
                },
            }],
        });
    }

    Sarif {
        schema: SCHEMA,
        version: "2.1.0",
        runs: vec![Run {
            invocations,
            tool: Tool {
                driver: Driver {
                    name: "shelfcheck",
                    version: VERSION,
                    information_uri: "https://logitsolutions.ai/guides/ai-in-ecommerce/",
                    rules,
                },
            },
            results: out,
        }],
    }
}

pub fn render(results: &ResultsFile) -> Result<String> {
    let s = serde_json::to_string_pretty(&build(results))
        .map_err(|e| Error::internal(format!("SARIF: {e}")))?;
    Ok(format!("{s}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_are_relative_or_file_uris_and_encoded() {
        assert_eq!(
            uri("fixtures/store-a/products.csv"),
            "fixtures/store-a/products.csv"
        );
        assert_eq!(uri("./feeds/a b.csv"), "feeds/a%20b.csv");
        assert_eq!(
            uri("C:\\Users\\me\\feed file.csv"),
            "file:///C:/Users/me/feed%20file.csv"
        );
        assert_eq!(uri("/home/me/feed.csv"), "file:///home/me/feed.csv");
        assert_eq!(uri("feeds/caf\u{e9}#1.csv"), "feeds/caf%C3%A9%231.csv");
    }

    #[test]
    fn empty_sarif_is_valid_shape() {
        let text = render(&ResultsFile::new()).expect("sarif");
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(v["version"], "2.1.0");
        assert_eq!(v["runs"][0]["tool"]["driver"]["name"], "shelfcheck");
        assert!(
            v["runs"][0]["results"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(false)
        );
    }
}
