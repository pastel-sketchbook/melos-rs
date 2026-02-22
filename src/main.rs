mod cli;
mod commands;
mod config;
mod package;
mod runner;
mod workspace;

use anyhow::Result;
use cli::{Cli, Commands};
use clap::Parser;
use colored::Colorize;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Find and load workspace
    let workspace = match workspace::Workspace::find_and_load() {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!(
                "{} Failed to load melos.yaml: {}",
                "ERROR".red().bold(),
                e
            );
            std::process::exit(1);
        }
    };

    println!(
        "{} {} ({})",
        "melos-rs".cyan().bold(),
        workspace.config.name.bold(),
        workspace.root_path.display()
    );

    let result = match cli.command {
        Commands::Bootstrap(args) => commands::bootstrap::run(&workspace, args).await,
        Commands::Clean(args) => commands::clean::run(&workspace, args).await,
        Commands::Exec(args) => commands::exec::run(&workspace, args).await,
        Commands::List(args) => commands::list::run(&workspace, args).await,
        Commands::Run { script } => commands::run::run(&workspace, &script).await,
        Commands::Version(args) => commands::version::run(&workspace, args).await,
    };

    match result {
        Ok(()) => {
            println!("\n{}", "SUCCESS".green().bold());
            Ok(())
        }
        Err(e) => {
            eprintln!("\n{} {}", "FAILED".red().bold(), e);
            std::process::exit(1);
        }
    }
}
