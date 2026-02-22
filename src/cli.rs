use clap::{Args, Parser, Subcommand};

use crate::commands::{
    exec::ExecArgs, format::FormatArgs, list::ListArgs, publish::PublishArgs, run::RunArgs,
    version::VersionArgs,
};

/// melos-rs: A Rust CLI for Flutter/Dart monorepo management
///
/// Drop-in replacement for Melos, built for speed.
#[derive(Parser, Debug)]
#[command(name = "melos-rs", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
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
}

impl GlobalFilterArgs {
    /// Returns the effective diff ref, preferring --diff over --since
    pub fn effective_diff(&self) -> Option<&str> {
        self.diff.as_deref().or(self.since.as_deref())
    }

    /// Returns the flutter filter: Some(true) for --flutter, Some(false) for --no-flutter, None if neither
    pub fn flutter_filter(&self) -> Option<bool> {
        if self.flutter {
            Some(true)
        } else if self.no_flutter {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize the workspace by linking packages and running `pub get`
    Bootstrap(BootstrapArgs),

    /// Clean all packages (runs `flutter clean` in each)
    Clean(CleanArgs),

    /// Execute a command in each package
    Exec(ExecArgs),

    /// Format Dart code across packages using `dart format`
    Format(FormatArgs),

    /// List packages in the workspace
    List(ListArgs),

    /// Publish packages to pub.dev
    Publish(PublishArgs),

    /// Run a script defined in melos.yaml
    Run(RunArgs),

    /// Manage package versions
    Version(VersionArgs),
}

/// Arguments for the `bootstrap` command
#[derive(Args, Debug)]
pub struct BootstrapArgs {
    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Arguments for the `clean` command
#[derive(Args, Debug)]
pub struct CleanArgs {
    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}
