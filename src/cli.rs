use clap::{Parser, Subcommand};

use crate::commands::{exec::ExecArgs, list::ListArgs, version::VersionArgs};

/// melos-rs: A Rust CLI for Flutter/Dart monorepo management
///
/// Drop-in replacement for Melos, built for speed.
#[derive(Parser, Debug)]
#[command(name = "melos-rs", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize the workspace by linking packages and running `pub get`
    Bootstrap,

    /// Clean all packages (runs `flutter clean` in each)
    Clean,

    /// Execute a command in each package
    Exec(ExecArgs),

    /// List packages in the workspace
    List(ListArgs),

    /// Run a script defined in melos.yaml
    Run {
        /// Name of the script to run
        script: String,
    },

    /// Manage package versions
    Version(VersionArgs),
}
