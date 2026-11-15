//! The `shelfcheck` binary.

mod cli;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
use shelfcheck_core::check::{self, CheckOptions};
use shelfcheck_core::config::{self, Config};
use shelfcheck_core::diff::{self, DiffOptions, Level, Status, decisions};
use shelfcheck_core::error::{Error, Result};
use shelfcheck_core::model::{FeedFormat, TargetFormat};
use shelfcheck_core::read::{self, ReadOptions};
use shelfcheck_core::results::ResultsFile;
use shelfcheck_core::rules::RuleSet;
use shelfcheck_core::rules::engine::EngineOpts;
use shelfcheck_report::terminal::{self, Style};
use shelfcheck_report::{WriteOptions, read_results, write_all};

use cli::{Cli, ColorChoice, Command, FailOn, RulesAction};

fn main() -> ExitCode {
    let cli = Cli::parse();
    // A panic is a bug: report it as exit 4, never as a clean run, and in one
    // line (where it happened) rather than the default multi-line message.
    std::panic::set_hook(Box::new(|info| {
        let place = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        eprintln!("shelfcheck: internal error{place} (a bug)");
    }));
    let outcome = std::panic::catch_unwind(|| run(&cli));
    let code = match outcome {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => {
            eprintln!("shelfcheck: {e}");
            e.exit_code()
        }
        Err(_) => {
            eprintln!(
                "shelfcheck: internal error (a bug). Please report it with the command you ran."
            );
            4
        }
    };
    ExitCode::from(u8::try_from(code).unwrap_or(4))
}

fn style(cli: &Cli, to_stderr: bool) -> Style {
    let tty = if to_stderr {
        std::io::stderr().is_terminal()
    } else {
        std::io::stdout().is_terminal()
    };
    let no_color = std::env::var_os("NO_COLOR")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let color = match cli.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => tty && !no_color,
    };
    Style { color }
}

/// Human output goes to stdout, or to stderr when --json owns stdout.
fn say(cli: &Cli, text: &str) {
    if cli.json {
        eprint!("{text}");
    } else {
        print!("{text}");
        let _ = std::io::stdout().flush();
    }
}

fn load_config(cli: &Cli) -> Result<Config> {
    match &cli.config {
        Some(p) => Config::load(p, true),
        None => Config::load(Path::new(config::DEFAULT_FILE), false),
    }
}

fn read_options(cfg: &Config, format: Option<&str>) -> Result<ReadOptions> {
    let format = match format {
        Some(f) => Some(FeedFormat::parse(f).ok_or_else(|| {
            Error::input(format!(
                "--format {f}: use openai-csv, openai-tsv, openai-jsonl, google-xml, google-csv, google-tsv, acp, shopify, csv, tsv or jsonl"
            ))
        })?),
        None => None,
    };
    Ok(ReadOptions {
        format,
        custom_map: cfg.map.clone(),
        limits: cfg.limits(),
    })
}

/// The run time for the report header: SOURCE_DATE_EPOCH when set (for
/// reproducible reports), else now.
fn run_at() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });
    let day = decisions::civil_date((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{day}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn run(cli: &Cli) -> Result<i32> {
    match &cli.command {
        Command::Check(a) => cmd_check(cli, a),
        Command::Diff(a) => cmd_diff(cli, a),
        Command::Decide(a) => cmd_decide(cli, a),
        Command::Rules { action } => cmd_rules(action),
        Command::Init(a) => cmd_init(a),
    }
}

fn cmd_check(cli: &Cli, a: &cli::CheckArgs) -> Result<i32> {
    let cfg = load_config(cli)?;
    let mut formats = Vec::new();
    let wanted: Vec<String> = if a.formats.is_empty() {
        cfg.formats.clone()
    } else {
        a.formats.clone()
    };
    for f in &wanted {
        let t = TargetFormat::parse(f)
            .ok_or_else(|| Error::input(format!("--formats {f}: use openai, acp or google")))?;
        if !formats.contains(&t) {
            formats.push(t);
        }
    }
    if formats.is_empty() {
        formats = TargetFormat::ALL.to_vec();
    }
    formats.sort();
    let mut rule_dirs: Vec<PathBuf> = cfg.rules.iter().map(PathBuf::from).collect();
    rule_dirs.extend(a.rules.iter().cloned());
    let dirs: Vec<&Path> = rule_dirs.iter().map(|p| p.as_path()).collect();
    let mut ignore = cfg.ignore.clone();
    ignore.extend(a.ignore.iter().cloned());
    let rules = RuleSet::load(&dirs, &ignore)?;
    let mut redact = cfg.redact.clone();
    redact.extend(a.redact.iter().cloned());
    if let Some(dir) = &a.images {
        if !dir.is_dir() {
            return Err(Error::Unavailable {
                path: dir.display().to_string(),
                reason: "--images needs a folder".to_string(),
            });
        }
    }
    let opts = CheckOptions {
        formats,
        read: read_options(&cfg, a.format.as_deref())?,
        engine: EngineOpts { redact },
        images: a.images.clone(),
    };
    let started = Instant::now();
    let (results, _timings) = check::run(&a.feeds, &rules, &opts)?;
    let elapsed = started.elapsed();

    // Keep an earlier diff of the same new feed, so one report shows both.
    let mut file = ResultsFile::new();
    if let Ok(Some(prev)) = read_results(&a.out) {
        if let Some(d) = prev.diff {
            if results.inputs.iter().any(|i| i.sha256 == d.new.sha256) {
                file.diff = Some(d);
            }
        }
    }
    let errors = results.summary.errors;
    let warnings = results.summary.warnings;
    file.check = Some(results);
    let written = write_all(
        &a.out,
        &file,
        &WriteOptions {
            run_at: Some(run_at()),
            fail_on_warning: a.fail_on == FailOn::Warning,
        },
    )?;
    if cli.json {
        let json =
            serde_json::to_string_pretty(&file).map_err(|e| Error::internal(e.to_string()))?;
        println!("{json}");
    }
    if let Some(c) = &file.check {
        let st = style(cli, cli.json);
        let text = terminal::render_check(c, elapsed, Some(&written), &st);
        if cli.quiet {
            let verdict = text.lines().rev().nth(1).unwrap_or("").to_string();
            say(cli, &format!("{verdict}\n"));
        } else {
            say(cli, &text);
        }
    }
    let fail = errors > 0 || (a.fail_on == FailOn::Warning && warnings > 0);
    Ok(if fail { 1 } else { 0 })
}

fn cmd_diff(cli: &Cli, a: &cli::DiffArgs) -> Result<i32> {
    let cfg = load_config(cli)?;
    let mut flags = String::new();
    if a.out != Path::new("shelfcheck-results") {
        flags.push_str(&format!(" --results {}", shell_quote(&a.out)));
    }
    if a.decisions != Path::new(decisions::DEFAULT_FILE) {
        flags.push_str(&format!(" --decisions {}", shell_quote(&a.decisions)));
    }
    let opts = DiffOptions {
        read: read_options(&cfg, a.format.as_deref())?,
        decide_flags: flags,
    };
    let started = Instant::now();
    let d = diff::run(&a.old, &a.new, a.store.as_deref(), &a.decisions, &opts)?;
    let elapsed = started.elapsed();

    let mut file = ResultsFile::new();
    if let Ok(Some(prev)) = read_results(&a.out) {
        if let Some(c) = prev.check {
            if c.inputs.iter().any(|i| i.sha256 == d.new.sha256) {
                file.check = Some(c);
            }
        }
    }
    let open_decisions = d.summary.needs_decision;
    let open_warnings = d
        .changes
        .iter()
        .filter(|c| c.level == Level::Warning && c.status == Status::Open)
        .count();
    file.diff = Some(d);
    let written = write_all(
        &a.out,
        &file,
        &WriteOptions {
            run_at: Some(run_at()),
            fail_on_warning: a.fail_on == FailOn::Warning,
        },
    )?;
    if cli.json {
        let json =
            serde_json::to_string_pretty(&file).map_err(|e| Error::internal(e.to_string()))?;
        println!("{json}");
    }
    if let Some(d) = &file.diff {
        let st = style(cli, cli.json);
        let text = terminal::render_diff(d, elapsed, Some(&written), &st);
        if cli.quiet {
            let n = d.summary.needs_decision;
            say(
                cli,
                &format!(
                    "{n} {} need a decision.\n",
                    if n == 1 { "change" } else { "changes" }
                ),
            );
        } else {
            say(cli, &text);
        }
    }
    let fail = open_decisions > 0 || (a.fail_on == FailOn::Warning && open_warnings > 0);
    Ok(if fail { 1 } else { 0 })
}

/// A path as a shell argument: quoted when it holds a space or a character a
/// shell would read, so the suggested `shelfcheck decide` commands can be
/// pasted as they are.
fn shell_quote(p: &Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-:".contains(c));
    if plain {
        s
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

fn who(arg: &Option<String>) -> String {
    if let Some(b) = arg {
        return b.clone();
    }
    for var in ["SHELFCHECK_BY", "USER", "USERNAME"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return v;
            }
        }
    }
    "unknown".to_string()
}

fn cmd_decide(cli: &Cli, a: &cli::DecideArgs) -> Result<i32> {
    let prev = read_results(&a.results)?.ok_or_else(|| Error::Unavailable {
        path: a.results.join("results.json").display().to_string(),
        reason: "no results from an earlier diff; run shelfcheck diff first".to_string(),
    })?;
    let d = prev.diff.ok_or_else(|| {
        Error::input(format!(
            "{}: the results hold no diff; run shelfcheck diff first",
            a.results.display()
        ))
    })?;
    let id = a.change_id.trim().to_ascii_uppercase();
    let change = d
        .changes
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| Error::input(format!("no change {id} in the last diff")))?;
    if change.level == Level::Info {
        return Err(Error::input(format!(
            "{id} is information only and needs no decision"
        )));
    }
    let verdict = if a.verdict.same {
        decisions::Verdict::SameProduct
    } else if a.verdict.different {
        decisions::Verdict::DifferentProduct
    } else {
        decisions::Verdict::DataErrorFixed
    };
    let mut ds = decisions::Decisions::load(&a.decisions)?;
    ds.upsert(decisions::Decision {
        id: change.id.clone(),
        change: change.kind.id().to_string(),
        item_id: change.item_id.clone(),
        fingerprint: change.fingerprint.clone(),
        verdict,
        by: who(&a.by),
        at: decisions::today(),
        note: a.note.clone(),
    });
    ds.save(&a.decisions)?;
    say(
        cli,
        &format!(
            "Recorded {} ({}, {}) as {} in {}. The next diff keeps it quiet while its fingerprint is the same.\n",
            change.id,
            change.kind.id(),
            change.item_id,
            verdict.id(),
            a.decisions.display()
        ),
    );
    Ok(0)
}

fn cmd_rules(action: &RulesAction) -> Result<i32> {
    match action {
        RulesAction::List { rules } => {
            let dirs: Vec<&Path> = rules.iter().map(|p| p.as_path()).collect();
            let set = RuleSet::load(&dirs, &[])?;
            let mut out = String::new();
            for p in &set.packs {
                out.push_str(&format!(
                    "{} ({}, checked {})\n",
                    p.meta.name, p.meta.version, p.meta.checked
                ));
                for r in &p.rules {
                    out.push_str(&format!(
                        "  {:<44} {:<8} {}\n",
                        r.def.id,
                        r.def.severity.label(),
                        r.def.kind.id()
                    ));
                }
                out.push('\n');
            }
            print!("{out}");
            Ok(0)
        }
        RulesAction::Describe { rule_id, rules } => {
            let dirs: Vec<&Path> = rules.iter().map(|p| p.as_path()).collect();
            let set = RuleSet::load(&dirs, &[])?;
            let (p, r) = set.find(rule_id).ok_or_else(|| {
                Error::input(format!("no rule {rule_id} (see shelfcheck rules list)"))
            })?;
            let mut out = String::new();
            out.push_str(&format!("{}\n", r.def.id));
            out.push_str(&format!(
                "  pack      {} ({}, checked {})\n",
                p.meta.name, p.meta.version, p.meta.checked
            ));
            out.push_str(&format!("  kind      {}\n", r.def.kind.id()));
            if let Some(f) = &r.def.field {
                out.push_str(&format!("  field     {f}\n"));
            }
            out.push_str(&format!("  severity  {}\n", r.def.severity.label()));
            out.push_str(&format!("  says      {}\n", r.def.says));
            out.push_str(&format!("  cites     {}\n", r.def.cites));
            let links: Vec<String> = if r.def.links.is_empty() {
                vec![p.meta.source.clone()]
            } else {
                r.def
                    .links
                    .iter()
                    .map(|l| format!("{}: {}", l.label, l.url))
                    .collect()
            };
            for l in links {
                out.push_str(&format!("  source    {l}\n"));
            }
            print!("{out}");
            Ok(0)
        }
    }
}

fn cmd_init(a: &cli::InitArgs) -> Result<i32> {
    let target = Path::new(config::DEFAULT_FILE);
    if target.exists() && !a.force {
        return Err(Error::input(format!(
            "{} already exists; use --force to overwrite it",
            target.display()
        )));
    }
    let mut text = String::from(
        "# shelfcheck configuration, written by `shelfcheck init`.\n\
         # formats = [\"openai\", \"acp\", \"google\"]\n\
         # ignore = []          # rule ids to turn off; every ignore is reported\n\
         # redact = []          # fields whose values never appear in reports\n\
         # rules = []           # folders of local rule packs\n\n",
    );
    let mut unknown: Vec<String> = Vec::new();
    if let Some(feed) = &a.feed {
        let opts = ReadOptions::default();
        let (input, mut it) = read::open(feed, 0, &opts)?;
        text.push_str(&format!(
            "# {} was detected as {}.\n",
            input.path, input.format_label
        ));
        if let Some(first) = it.next() {
            let rec = first?;
            let map = shelfcheck_core::map::ColumnMap::builtin(input.format.family)?;
            let mut sources: Vec<String> =
                rec.fields.values().map(|f| f.source.to_string()).collect();
            sources.sort();
            sources.dedup();
            for s in sources {
                if !map.knows(&s) {
                    unknown.push(s);
                }
            }
        }
    }
    text.push_str("\n[map]\n# Source column = canonical field (the OpenAI field names).\n");
    if unknown.is_empty() {
        text.push_str("# Every column is known to the built-in map.\n");
    } else {
        text.push_str("# Columns the built-in map does not know. Map the ones you need:\n");
        for u in unknown {
            text.push_str(&format!("# \"{}\" = \"\"\n", u.replace('"', "\\\"")));
        }
    }
    std::fs::write(target, text).map_err(|e| Error::io(target, &e))?;
    println!("Wrote {}.", target.display());
    Ok(0)
}
