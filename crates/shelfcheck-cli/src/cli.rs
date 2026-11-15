//! Command-line interface (clap 4 derive).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "shelfcheck",
    version,
    about = "Checks your product feed against the formats AI shopping assistants read, and tells you what changed since the last feed.",
    after_help = "Exit codes: 0 clean, 1 problems found, 2 invalid input or config, 3 a required file is missing or unreadable, 4 internal error."
)]
pub struct Cli {
    /// Print only the verdict line.
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Write results.json to stdout; the summary goes to stderr.
    #[arg(long, global = true)]
    pub json: bool,

    /// Colour in terminal output. NO_COLOR is honoured.
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,

    /// Config file (default: shelfcheck.toml in the current folder, if any).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FailOn {
    Error,
    Warning,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List what each format would reject or ignore.
    Check(CheckArgs),
    /// Compare two feeds for identity changes.
    Diff(DiffArgs),
    /// Record a decision for a change from the last diff.
    Decide(DecideArgs),
    /// List or describe the rules.
    Rules {
        #[command(subcommand)]
        action: RulesAction,
    },
    /// Write shelfcheck.toml with the detected column map.
    Init(InitArgs),
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Feed files: CSV, TSV, JSONL, Google XML, an ACP bundle folder, or a Shopify export.
    #[arg(required = true)]
    pub feeds: Vec<PathBuf>,

    /// Formats to check against.
    #[arg(long, value_delimiter = ',')]
    pub formats: Vec<String>,

    /// Read every feed as this input format (openai-csv, google-xml, google-tsv, acp, shopify, csv, tsv, jsonl).
    #[arg(long)]
    pub format: Option<String>,

    /// Folder of product images, for Google's image size rule.
    #[arg(long)]
    pub images: Option<PathBuf>,

    /// Folder of local rule packs (repeatable).
    #[arg(long = "rules")]
    pub rules: Vec<PathBuf>,

    /// Turn a rule off (repeatable). Every ignore is reported.
    #[arg(long)]
    pub ignore: Vec<String>,

    /// Fields whose values never appear in the report or SARIF.
    #[arg(long, value_delimiter = ',')]
    pub redact: Vec<String>,

    /// Output folder.
    #[arg(long, default_value = "shelfcheck-results")]
    pub out: PathBuf,

    /// Make warnings count for the exit code.
    #[arg(long, value_enum, default_value_t = FailOn::Error)]
    pub fail_on: FailOn,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    pub old: PathBuf,
    pub new: PathBuf,

    /// A store export (Shopify product CSV) to check for stale titles and options.
    #[arg(long)]
    pub store: Option<PathBuf>,

    /// Decisions file.
    #[arg(long, default_value = "shelfcheck-decisions.toml")]
    pub decisions: PathBuf,

    /// Read both feeds as this input format.
    #[arg(long)]
    pub format: Option<String>,

    /// Output folder.
    #[arg(long, default_value = "shelfcheck-results")]
    pub out: PathBuf,

    /// Make warnings count for the exit code.
    #[arg(long, value_enum, default_value_t = FailOn::Error)]
    pub fail_on: FailOn,
}

#[derive(Debug, Args)]
#[group(id = "verdict", required = true, multiple = false)]
pub struct VerdictArgs {
    /// The same product: the change is expected.
    #[arg(long)]
    pub same: bool,
    /// A different product.
    #[arg(long)]
    pub different: bool,
    /// A data error, now fixed.
    #[arg(long)]
    pub fixed: bool,
}

#[derive(Debug, Args)]
pub struct DecideArgs {
    /// The change id from the last diff, such as D1.
    pub change_id: String,

    #[command(flatten)]
    pub verdict: VerdictArgs,

    /// A note stored with the decision.
    #[arg(long)]
    pub note: Option<String>,

    /// Who decided (default: SHELFCHECK_BY, then the user name).
    #[arg(long)]
    pub by: Option<String>,

    /// Folder holding the last diff's results.json.
    #[arg(long, default_value = "shelfcheck-results")]
    pub results: PathBuf,

    /// Decisions file.
    #[arg(long, default_value = "shelfcheck-decisions.toml")]
    pub decisions: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum RulesAction {
    /// Every rule, with its pack, kind and severity.
    List {
        /// Include local packs from this folder.
        #[arg(long = "rules")]
        rules: Vec<PathBuf>,
    },
    /// One rule in full.
    Describe {
        rule_id: String,
        #[arg(long = "rules")]
        rules: Vec<PathBuf>,
    },
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// A feed to detect the format and columns from.
    pub feed: Option<PathBuf>,

    /// Overwrite an existing shelfcheck.toml.
    #[arg(long)]
    pub force: bool,
}
