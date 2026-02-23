use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;

use crate::commands::{
    analyze::AnalyzeArgs, build::BuildArgs, exec::ExecArgs, format::FormatArgs, health::HealthArgs,
    init::InitArgs, list::ListArgs, pub_cmds::PubArgs, publish::PublishArgs, run::RunArgs,
    test::TestArgs, version::VersionArgs,
};

/// melos-rs: A Rust CLI for Flutter/Dart monorepo management
///
/// Drop-in replacement for Melos, built for speed.
#[derive(Parser, Debug)]
#[command(name = "melos-rs", version, about, long_about = None)]
pub struct Cli {
    /// Increase output verbosity (show debug info)
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress non-essential output
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Path to the Dart/Flutter SDK (overrides MELOS_SDK_PATH and sdkPath config)
    #[arg(long, global = true)]
    pub sdk_path: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Verbosity level resolved from --verbose / --quiet flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// --quiet: only errors and essential output
    Quiet,
    /// default: normal output
    Normal,
    /// --verbose: extra debug info
    Verbose,
}

impl Cli {
    /// Resolve the verbosity level from CLI flags
    pub fn verbosity(&self) -> Verbosity {
        match (self.quiet, self.verbose) {
            (true, _) => Verbosity::Quiet,
            (_, true) => Verbosity::Verbose,
            _ => Verbosity::Normal,
        }
    }
}

/// Package filter flags shared across all commands.
///
/// These correspond to the global filters in Melos:
/// <https://melos.invertase.dev/~melos-latest/filters>
#[derive(Args, Debug, Clone, Default)]
pub struct GlobalFilterArgs {
    /// Include only packages with names matching the glob pattern (can be repeated)
    #[arg(long, global = true)]
    pub scope: Vec<String>,

    /// Exclude packages with names matching the glob pattern (can be repeated)
    #[arg(long, global = true)]
    pub ignore: Vec<String>,

    /// Only include packages that have been changed since the given git ref
    #[arg(long, global = true)]
    pub diff: Option<String>,

    /// Alias for --diff
    #[arg(long, global = true, conflicts_with = "diff")]
    pub since: Option<String>,

    /// Only include packages where the given directory exists
    #[arg(long = "dir-exists", global = true)]
    pub dir_exists: Option<String>,

    /// Only include packages where the given file exists
    #[arg(long = "file-exists", global = true)]
    pub file_exists: Option<String>,

    /// Only include Flutter packages
    #[arg(long, global = true)]
    pub flutter: bool,

    /// Only include pure Dart packages (exclude Flutter)
    #[arg(long, global = true)]
    pub no_flutter: bool,

    /// Only include packages that depend on the given package (can be repeated)
    #[arg(long = "depends-on", global = true)]
    pub depends_on: Vec<String>,

    /// Exclude packages that depend on the given package (can be repeated)
    #[arg(long = "no-depends-on", global = true)]
    pub no_depends_on: Vec<String>,

    /// Exclude private packages (publish_to: none)
    #[arg(long, global = true)]
    pub no_private: bool,

    /// Only include packages in the given category (can be repeated)
    #[arg(long, global = true)]
    pub category: Vec<String>,

    /// Also include transitive dependencies of matched packages
    #[arg(long, global = true)]
    pub include_dependencies: bool,

    /// Also include transitive dependents of matched packages
    #[arg(long, global = true)]
    pub include_dependents: bool,

    /// Only include published packages (publish_to is not "none")
    #[arg(long, global = true)]
    pub published: bool,

    /// Only include non-published/private packages
    #[arg(long, global = true, conflicts_with = "published")]
    pub no_published: bool,
}

impl GlobalFilterArgs {
    /// Returns the effective diff ref, preferring --diff over --since
    pub fn effective_diff(&self) -> Option<&str> {
        self.diff.as_deref().or(self.since.as_deref())
    }

    /// Returns the flutter filter: Some(true) for --flutter, Some(false) for --no-flutter, None if neither
    pub fn flutter_filter(&self) -> Option<bool> {
        match (self.flutter, self.no_flutter) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        }
    }

    /// Returns the published filter: Some(true) for --published, Some(false) for --no-published, None if neither
    pub fn published_filter(&self) -> Option<bool> {
        match (self.published, self.no_published) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run static analysis across packages using `dart analyze`
    Analyze(AnalyzeArgs),

    /// Initialize the workspace by linking packages and running `pub get`
    #[command(alias = "bs")]
    Bootstrap(BootstrapArgs),

    /// Build Flutter apps for Android and/or iOS with declarative config
    Build(BuildArgs),

    /// Clean all packages (runs `flutter clean` in each)
    Clean(CleanArgs),

    /// Generate shell completion scripts
    Completion(CompletionArgs),

    /// Execute a command in each package
    Exec(ExecArgs),

    /// Format Dart code across packages using `dart format`
    Format(FormatArgs),

    /// Run workspace health checks (version drift, missing fields, SDK consistency)
    Health(HealthArgs),

    /// Initialize a new Melos workspace
    Init(InitArgs),

    /// List packages in the workspace
    List(ListArgs),

    /// Run pub commands (get, outdated, upgrade) across packages
    Pub(PubArgs),

    /// Publish packages to pub.dev
    Publish(PublishArgs),

    /// Run a script defined in melos.yaml
    Run(RunArgs),

    /// Run tests across packages using `dart test` / `flutter test`
    Test(TestArgs),

    /// Manage package versions
    Version(VersionArgs),

    /// Launch the interactive terminal UI (requires melos-tui binary)
    Tui,
}

/// Arguments for the `bootstrap` command
#[derive(Args, Debug)]
pub struct BootstrapArgs {
    /// Number of concurrent pub get processes
    #[arg(short = 'c', long, default_value_t = 5)]
    pub concurrency: usize,

    /// Enforce the pubspec.lock file (pass --enforce-lockfile to pub get)
    #[arg(long)]
    pub enforce_lockfile: bool,

    /// Disable enforce-lockfile even if configured in melos.yaml
    #[arg(long, conflicts_with = "enforce_lockfile")]
    pub no_enforce_lockfile: bool,

    /// Skip resolving dependencies in example apps
    #[arg(long)]
    pub no_example: bool,

    /// Use cached packages only; do not access the network
    #[arg(long)]
    pub offline: bool,

    /// Show what would be done without actually running pub get
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Arguments for the `clean` command
#[derive(Args, Debug)]
pub struct CleanArgs {
    /// Deep clean: also remove .dart_tool/, build/, and pubspec.lock
    #[arg(long)]
    pub deep: bool,

    /// Show what would be cleaned without actually removing anything
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Arguments for the `completion` command
#[derive(Args, Debug)]
pub struct CompletionArgs {
    /// The shell to generate completions for
    #[arg(value_enum)]
    pub shell: Shell,
}
