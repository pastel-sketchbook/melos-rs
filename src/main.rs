mod cli;
mod commands;
mod config;
mod package;
mod runner;
mod watcher;
mod workspace;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, Verbosity};
use colored::Colorize;

/// Built-in command names that can be overridden by scripts with the same name.
/// Note: `run`, `init`, and `completion` are excluded because they are never overridden.
/// The `bs` alias for `bootstrap` is resolved by clap before reaching our code.
const OVERRIDABLE_COMMANDS: &[&str] = &[
    "analyze",
    "bootstrap",
    "build",
    "clean",
    "exec",
    "format",
    "health",
    "list",
    "pub",
    "publish",
    "test",
    "version",
];

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
    let workspace = match workspace::Workspace::find_and_load(cli.sdk_path.as_deref()) {
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

    // Check for script overrides: if a script has the same name as the built-in
    // command being invoked, run the script instead.
    let result = if let Some(script_name) = get_overridable_command_name(&cli.command)
        && workspace.config.scripts.contains_key(script_name)
    {
        if verbosity == Verbosity::Verbose {
            println!(
                "{} Script '{}' overrides the built-in command",
                "DEBUG".dimmed(),
                script_name,
            );
        }
        let run_args = commands::run::RunArgs {
            script: Some(script_name.to_string()),
            no_select: false,
            list: false,
            json: false,
            include_private: false,
            group: vec![],
            watch: false,
            filters: cli::GlobalFilterArgs::default(),
        };
        commands::run::run(&workspace, run_args).await
    } else {
        match cli.command {
            Commands::Analyze(args) => commands::analyze::run(&workspace, args).await,
            Commands::Bootstrap(args) => commands::bootstrap::run(&workspace, args).await,
            Commands::Build(args) => commands::build::run(&workspace, args).await,
            Commands::Clean(args) => commands::clean::run(&workspace, args).await,
            Commands::Completion(_) => unreachable!("completion handled above"),
            Commands::Exec(args) => commands::exec::run(&workspace, args).await,
            Commands::Format(args) => commands::format::run(&workspace, args).await,
            Commands::Health(args) => commands::health::run(&workspace, args).await,
            Commands::Init(_) => unreachable!("init handled above"),
            Commands::List(args) => commands::list::run(&workspace, args).await,
            Commands::Pub(args) => commands::pub_cmds::run(&workspace, args).await,
            Commands::Publish(args) => commands::publish::run(&workspace, args).await,
            Commands::Run(args) => commands::run::run(&workspace, args).await,
            Commands::Test(args) => commands::test::run(&workspace, args).await,
            Commands::Version(args) => commands::version::run(&workspace, args).await,
        }
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

/// If the CLI command is a built-in that can be overridden by a script,
/// return the command name as a string.
fn get_overridable_command_name(command: &Commands) -> Option<&'static str> {
    let name = match command {
        Commands::Analyze(_) => "analyze",
        Commands::Bootstrap(_) => "bootstrap",
        Commands::Build(_) => "build",
        Commands::Clean(_) => "clean",
        Commands::Exec(_) => "exec",
        Commands::Format(_) => "format",
        Commands::Health(_) => "health",
        Commands::List(_) => "list",
        Commands::Pub(_) => "pub",
        Commands::Publish(_) => "publish",
        Commands::Version(_) => "version",
        Commands::Test(_) => "test",
        // `run`, `init`, `completion` are never overridden
        Commands::Run(_) | Commands::Init(_) | Commands::Completion(_) => return None,
    };

    // Only override if the name is in our overridable list
    if OVERRIDABLE_COMMANDS.contains(&name) {
        Some(name)
    } else {
        None
    }
}
