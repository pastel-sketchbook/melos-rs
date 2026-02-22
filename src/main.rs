mod cli;
mod commands;
mod config;
mod package;
mod runner;
mod workspace;

use anyhow::Result;
use cli::{Cli, Commands, Verbosity};
use clap::Parser;
use colored::Colorize;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let verbosity = cli.verbosity();

    // `init` and `completion` don't require an existing workspace — handle them early
    if let Commands::Init(args) = cli.command {
        return match commands::init::run(args) {
            Ok(()) => {
                if verbosity != Verbosity::Quiet {
                    println!("\n{}", "SUCCESS".green().bold());
                }
                Ok(())
            }
            Err(e) => {
                eprintln!("\n{} {}", "FAILED".red().bold(), e);
                std::process::exit(1);
            }
        };
    }

    if let Commands::Completion(args) = cli.command {
        clap_complete::generate(
            args.shell,
            &mut <Cli as clap::CommandFactory>::command(),
            "melos-rs",
            &mut std::io::stdout(),
        );
        return Ok(());
    }

    // Find and load workspace
    let workspace = match workspace::Workspace::find_and_load() {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("{} Failed to load workspace: {}", "ERROR".red().bold(), e);
            std::process::exit(1);
        }
    };

    if verbosity != Verbosity::Quiet {
        let config_mode = if workspace.config_source.is_legacy() {
            "melos.yaml"
        } else {
            "pubspec.yaml"
        };

        println!(
            "{} {} ({}) [{}]",
            "melos-rs".cyan().bold(),
            workspace.config.name.bold(),
            workspace.root_path.display(),
            config_mode.dimmed()
        );
    }

    if verbosity == Verbosity::Verbose {
        println!(
            "{} {} packages discovered, config from {}",
            "DEBUG".dimmed(),
            workspace.packages.len(),
            workspace.config_source.path().display()
        );
    }

    let result = match cli.command {
        Commands::Bootstrap(args) => commands::bootstrap::run(&workspace, args).await,
        Commands::Clean(args) => commands::clean::run(&workspace, args).await,
        Commands::Completion(_) => unreachable!("completion handled above"),
        Commands::Exec(args) => commands::exec::run(&workspace, args).await,
        Commands::Format(args) => commands::format::run(&workspace, args).await,
        Commands::Init(_) => unreachable!("init handled above"),
        Commands::List(args) => commands::list::run(&workspace, args).await,
        Commands::Publish(args) => commands::publish::run(&workspace, args).await,
        Commands::Run(args) => commands::run::run(&workspace, args).await,
        Commands::Version(args) => commands::version::run(&workspace, args).await,
    };

    match result {
        Ok(()) => {
            if verbosity != Verbosity::Quiet {
                println!("\n{}", "SUCCESS".green().bold());
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("\n{} {}", "FAILED".red().bold(), e);
            std::process::exit(1);
        }
    }
}
