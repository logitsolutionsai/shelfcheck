//! Terminal output, following the "Terminal Output" board: one line per
//! stage with the time at the right edge, problems last, the verdict just
//! above the file paths. Symbols carry the meaning (✗ failed, ▲ needs a
//! decision, ✓ passed); colour only adds to them.

use std::collections::BTreeSet;
use std::time::Duration;

use shelfcheck_core::diff::{ChangeKind, DiffResults, Level, Status};
use shelfcheck_core::model::Severity;
use shelfcheck_core::results::{CheckResults, PackInfo, VERSION};

use crate::{Written, pack_stamp, plural, shown_packs, thousands};

/// ANSI styling, or none.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub color: bool,
}

impl Style {
    fn paint(&self, rgb: (u8, u8, u8), s: &str) -> String {
        if self.color {
            format!("\x1b[38;2;{};{};{}m{s}\x1b[0m", rgb.0, rgb.1, rgb.2)
        } else {
            s.to_string()
        }
    }
    pub fn muted(&self, s: &str) -> String {
        self.paint((0x8b, 0x97, 0xa8), s)
    }
    pub fn fail(&self, s: &str) -> String {
        self.paint((0xf9, 0x70, 0x66), s)
    }
    pub fn caution(&self, s: &str) -> String {
        self.paint((0xfd, 0xb0, 0x22), s)
    }
    pub fn link(&self, s: &str) -> String {
        self.paint((0x5b, 0x9b, 0xff), s)
    }
    pub fn strong(&self, s: &str) -> String {
        if self.color {
            format!("\x1b[1m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

const WIDTH: usize = 80;

/// "read      OpenAI CSV · 1,400 products              0.21 s"
fn stage(style: &Style, name: &str, text: &str, time: Option<Duration>) -> String {
    let left = format!("{name:<10}{text}");
    match time {
        Some(t) => {
            let secs = format!("{:.2} s", t.as_secs_f64());
            let pad = WIDTH
                .saturating_sub(left.chars().count() + secs.len())
                .max(2);
            format!(
                "{}{text}{}{}",
                style.muted(&format!("{name:<10}")),
                " ".repeat(pad),
                style.muted(&secs)
            )
        }
        None => format!("{}{text}", style.muted(&format!("{name:<10}"))),
    }
}

fn header(style: &Style, packs: &[PackInfo]) -> String {
    let stamps: Vec<String> = shown_packs(packs)
        .iter()
        .map(|p| format!("{} {}", p.short, pack_stamp(p)))
        .collect();
    style.muted(&format!(
        "shelfcheck {VERSION} · rules {}",
        stamps.join(" · ")
    ))
}

/// Exactly `n` characters: padded, or cut with an ellipsis.
fn fit(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len <= n {
        format!("{s}{}", " ".repeat(n - len))
    } else {
        let cut: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
            + &" ".repeat(n.saturating_sub(cut.trim_end().chars().count() + 1))
    }
}

fn pad_right(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len >= n {
        format!("{s}  ")
    } else {
        format!("{s}{}", " ".repeat(n - len))
    }
}

pub fn render_check(
    c: &CheckResults,
    elapsed: Duration,
    written: Option<&Written>,
    style: &Style,
) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push(header(style, &c.packs));
    out.push(String::new());
    let labels: Vec<String> = c.inputs.iter().map(|i| i.format_label.clone()).collect();
    out.push(stage(
        style,
        "read",
        &format!(
            "{} · {} products, {} variants",
            labels.join(", "),
            thousands(c.summary.products as u64),
            thousands(c.summary.items)
        ),
        Some(elapsed),
    ));
    out.push(String::new());
    // Column widths come from the numbers, so a large feed stays aligned.
    let cells: Vec<(String, String, String, String)> = c
        .readiness
        .iter()
        .map(|r| {
            (
                thousands(r.ready as u64),
                format!("{}%", r.percent_ready),
                thousands(r.with_errors as u64),
                thousands(r.with_warnings as u64),
            )
        })
        .collect();
    let w_ready = cells.iter().map(|x| x.0.len()).max().unwrap_or(0).max(5);
    let w_pct = cells.iter().map(|x| x.1.len()).max().unwrap_or(0).max(6);
    let w_err = cells.iter().map(|x| x.2.len()).max().unwrap_or(0).max(6);
    out.push(style.muted(&format!(
        "{:<10}{:>w_ready$}   {:w_pct$}   {:<w_err$}   warnings",
        "", "ready", "", "errors"
    )));
    for (r, (ready, pct, errors, warnings)) in c.readiness.iter().zip(&cells) {
        let errors_cell = format!("{errors:<w_err$}");
        out.push(format!(
            "{}{ready:>w_ready$}   {pct:>w_pct$}   {}   {warnings}",
            style.muted(&format!("{:<10}", r.format.label())),
            if r.with_errors > 0 {
                style.fail(&errors_cell)
            } else {
                errors_cell
            },
        ));
    }
    out.push(String::new());
    const TITLE_MAX: usize = 44;
    let w_title = c
        .fixes
        .iter()
        .take(3)
        .map(|f| f.title.chars().count())
        .max()
        .unwrap_or(0)
        .min(TITLE_MAX);
    let w_count = c
        .fixes
        .iter()
        .take(3)
        .map(|f| thousands(f.products as u64).len())
        .max()
        .unwrap_or(0)
        .max(4);
    for f in c.fixes.iter().take(3) {
        let sym = if f.severity == Severity::Error {
            style.fail("✗")
        } else {
            style.caution("▲")
        };
        let failing: Vec<&str> = f
            .formats
            .iter()
            .filter(|s| {
                matches!(
                    s.status,
                    shelfcheck_core::results::FixStatus::Error
                        | shelfcheck_core::results::FixStatus::Warning
                )
            })
            .map(|s| s.format.label())
            .collect();
        let detail = format!(
            "{} · e.g. {}, line {}",
            if f.cross {
                "Cross-format".to_string()
            } else {
                failing.join(", ")
            },
            f.example.item_id,
            f.example.location.line
        );
        out.push(format!(
            "{sym} {:>w_count$}  {}  {}",
            thousands(f.products as u64),
            fit(&f.title, w_title),
            detail
        ));
    }
    if c.fixes.len() > 3 {
        let rest: BTreeSet<&str> = c.fixes.iter().skip(3).map(|f| f.key.as_str()).collect();
        let products: BTreeSet<&str> = c
            .problems
            .iter()
            .filter(|p| rest.contains(p.fix.as_str()))
            .map(|p| p.product.as_str())
            .collect();
        out.push(style.muted(&format!(
            "    + {} more {}, {} {}. All of them are in the report.",
            c.fixes.len() - 3,
            plural(c.fixes.len() - 3, "fix", "fixes"),
            thousands(products.len() as u64),
            plural(products.len(), "product", "products")
        )));
    }
    if !c.ignored.is_empty() {
        let ids: Vec<&str> = c.ignored.iter().map(|i| i.id.as_str()).collect();
        out.push(style.muted(&format!("    ignored: {}", ids.join(", "))));
    }
    if !c.fixes.is_empty() || !c.ignored.is_empty() {
        out.push(String::new());
    }
    let verdict = if c.summary.products_needing_fixes == 0 {
        format!(
            "{} All {} products pass every format checked.",
            style.muted("✓"),
            thousands(c.summary.products as u64)
        )
    } else {
        format!(
            "{} of {} products need fixes before every format accepts them.",
            thousands(c.summary.products_needing_fixes as u64),
            thousands(c.summary.products as u64)
        )
    };
    out.push(style.strong(&verdict));
    if let Some(w) = written {
        out.push(paths_line(style, w));
    }
    out.join("\n") + "\n"
}

fn paths_line(style: &Style, w: &Written) -> String {
    format!(
        "{}{}   {}results.sarif   {}failing-products.csv",
        style.muted(&format!("{:<10}", "report")),
        w.report_html.to_string_lossy().replace('\\', "/"),
        style.muted("sarif "),
        style.muted("csv "),
    )
}

pub fn render_diff(
    d: &DiffResults,
    elapsed: Duration,
    written: Option<&Written>,
    style: &Style,
) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push(style.muted(&format!(
        "shelfcheck {VERSION} · decisions {}",
        d.decisions_file
    )));
    out.push(String::new());
    let s = &d.stats;
    out.push(stage(
        style,
        "match",
        &format!(
            "{} in both · {} by gtin · {} by brand and mpn · {} by id only",
            thousands(s.in_both as u64),
            thousands(s.by_gtin as u64),
            thousands(s.by_brand_mpn as u64),
            thousands(s.by_id as u64)
        ),
        Some(elapsed),
    ));
    out.push(String::new());
    let mut any = false;
    for c in d.changes.iter().filter(|c| c.level != Level::Info) {
        any = true;
        let head = format!(
            "{} {:<3} {}",
            if c.status == Status::Decided {
                "✓"
            } else {
                "▲"
            },
            c.id,
            c.kind.id()
        );
        let text = if c.status == Status::Decided {
            let v = c.decision.as_ref().map(|d| d.verdict.id()).unwrap_or("");
            format!("decided: {v}")
        } else {
            c.sentence.clone()
        };
        let head_padded = pad_right(&head, 35);
        let item_padded = pad_right(&c.item_id, 14);
        let painted = match (c.status, c.level) {
            (Status::Decided, _) => style.muted(&head_padded),
            (_, Level::Decision) => style.strong(&style.caution(&head_padded)),
            _ => style.caution(&head_padded),
        };
        out.push(format!("{painted}{item_padded}{text}"));
    }
    let mut info: Vec<String> = Vec::new();
    for kind in [
        ChangeKind::PriceOnly,
        ChangeKind::StockOnly,
        ChangeKind::New,
        ChangeKind::Retired,
    ] {
        let n = d.changes.iter().filter(|c| c.kind == kind).count();
        if n > 0 {
            let label = match kind {
                ChangeKind::PriceOnly => "price-only",
                ChangeKind::StockOnly => "stock-only",
                ChangeKind::New => "new",
                _ => "retired",
            };
            info.push(format!("{} {label}", thousands(n as u64)));
        }
    }
    if !info.is_empty() {
        out.push(style.muted(&format!("      {}", info.join(" · "))));
        any = true;
    }
    if any {
        out.push(String::new());
    }
    let n = d.summary.needs_decision;
    out.push(style.strong(&match n {
        0 => "No change needs a decision.".to_string(),
        1 => "1 change needs a decision.".to_string(),
        n => format!("{n} changes need a decision."),
    }));
    let cmds: Vec<String> = d
        .changes
        .iter()
        .filter(|c| c.level == Level::Decision && c.status == Status::Open)
        .filter_map(|c| c.commands.first().cloned())
        .collect();
    if !cmds.is_empty() {
        out.push(style.link(&cmds.join("    ")));
    }
    if let Some(w) = written {
        out.push(paths_line(style, w));
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_right_aligns_time() {
        let s = stage(
            &Style { color: false },
            "read",
            "OpenAI CSV",
            Some(Duration::from_millis(210)),
        );
        assert!(s.starts_with("read      OpenAI CSV"));
        assert!(s.ends_with("0.21 s"));
        assert_eq!(s.chars().count(), WIDTH);
    }

    #[test]
    fn fit_pads_and_cuts_to_one_width() {
        assert_eq!(fit("abc", 5), "abc  ");
        let cut = fit("availability has a value the format does not accept", 20);
        assert_eq!(cut.chars().count(), 20);
        assert!(cut.ends_with('…') || cut.trim_end().ends_with('…'));
    }
}
