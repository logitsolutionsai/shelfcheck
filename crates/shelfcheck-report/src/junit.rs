//! junit.xml: one testcase per rule pack and one per change kind.

use quick_junit::{NonSuccessKind, Report, TestCase, TestCaseStatus, TestSuite};
use shelfcheck_core::diff::{ChangeKind, Level, Status};
use shelfcheck_core::error::{Error, Result};
use shelfcheck_core::model::Severity;
use shelfcheck_core::results::ResultsFile;

fn failed(message: String, description: String) -> TestCaseStatus {
    let mut s = TestCaseStatus::non_success(NonSuccessKind::Failure);
    s.set_message(message);
    s.set_description(description);
    s
}

pub fn build(results: &ResultsFile, fail_on_warning: bool) -> Report {
    let mut report = Report::new("shelfcheck");
    if let Some(c) = &results.check {
        let mut suite = TestSuite::new("shelfcheck check");
        for pack in &c.packs {
            let problems: Vec<_> = c.problems.iter().filter(|p| p.pack == pack.id).collect();
            let errors = problems
                .iter()
                .filter(|p| p.severity == Severity::Error)
                .count();
            let warnings = problems
                .iter()
                .filter(|p| p.severity == Severity::Warning)
                .count();
            let fails = errors > 0 || (fail_on_warning && warnings > 0);
            let status = if fails {
                let lines: Vec<String> = problems
                    .iter()
                    .take(50)
                    .map(|p| {
                        format!(
                            "{}:{} {} {}",
                            p.location.file, p.location.line, p.rule, p.message
                        )
                    })
                    .collect();
                failed(
                    format!(
                        "{errors} errors and {warnings} warnings from {} (rules as of {})",
                        pack.name, pack.checked
                    ),
                    lines.join("\n"),
                )
            } else {
                TestCaseStatus::success()
            };
            let mut case = TestCase::new(pack.id.clone(), status);
            case.set_classname("shelfcheck.check");
            suite.add_test_case(case);
        }
        report.add_test_suite(suite);
    }
    if let Some(d) = &results.diff {
        let mut suite = TestSuite::new("shelfcheck diff");
        for kind in ChangeKind::ALL {
            let open: Vec<_> = d
                .changes
                .iter()
                .filter(|c| c.kind == kind && c.status == Status::Open)
                .collect();
            let fails = match kind.level() {
                Level::Decision => !open.is_empty(),
                Level::Warning => fail_on_warning && !open.is_empty(),
                Level::Info => false,
            };
            let status = if fails {
                let lines: Vec<String> = open
                    .iter()
                    .map(|c| {
                        format!(
                            "{} {}: {} ({})",
                            c.id,
                            c.item_id,
                            c.sentence,
                            c.commands.first().cloned().unwrap_or_default()
                        )
                    })
                    .collect();
                failed(
                    format!("{} open {} changes", open.len(), kind.id()),
                    lines.join("\n"),
                )
            } else {
                TestCaseStatus::success()
            };
            let mut case = TestCase::new(kind.id(), status);
            case.set_classname("shelfcheck.diff");
            suite.add_test_case(case);
        }
        report.add_test_suite(suite);
    }
    report
}

pub fn render(results: &ResultsFile, fail_on_warning: bool) -> Result<String> {
    build(results, fail_on_warning)
        .to_string()
        .map_err(|e| Error::internal(format!("junit.xml: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_serializes() {
        let xml = render(&ResultsFile::new(), false).expect("junit");
        assert!(xml.contains("<testsuites"));
    }
}
