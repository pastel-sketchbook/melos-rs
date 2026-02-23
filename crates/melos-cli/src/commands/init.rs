use std::io::{self, BufRead, Write};

use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;

use melos_core::commands::init::{create_dir_if_missing, write_7x_config, write_legacy_config};

/// Arguments for the `init` command
#[derive(Args, Debug)]
pub struct InitArgs {
    /// Workspace name (defaults to current directory name)
    pub name: Option<String>,

    /// Directory to create the workspace in
    #[arg(short = 'd', long)]
    pub directory: Option<String>,

    /// Additional package glob patterns (comma-separated, can be repeated)
    #[arg(short = 'p', long)]
    pub packages: Vec<String>,

    /// Use legacy 6.x format (melos.yaml) instead of 7.x (pubspec.yaml with melos: key)
    #[arg(long)]
    pub legacy: bool,
}

/// Initialize a new Melos workspace
pub fn run(args: InitArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;

    // Resolve workspace name
    let workspace_name = match args.name {
        Some(name) => name,
        None => prompt_with_default(
            "Workspace name",
            cwd.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("my_workspace"),
        )?,
    };

    // Resolve target directory
    let cwd_name = cwd.file_name().and_then(|s| s.to_str()).unwrap_or("");

    let default_dir = if workspace_name == cwd_name {
        ".".to_string()
    } else {
        workspace_name.clone()
    };

    let target_dir_str = match args.directory {
        Some(dir) => dir,
        None => prompt_with_default("Directory", &default_dir)?,
    };

    let target_dir = if target_dir_str == "." {
        cwd.clone()
    } else {
        cwd.join(&target_dir_str)
    };

    // Ask about apps directory
    let include_apps = prompt_yes_no("Include an 'apps' directory?", true)?;

    let mut package_patterns: Vec<String> = vec!["packages/*".to_string()];
    if include_apps {
        package_patterns.push("apps/*".to_string());
    }
    // Add user-specified patterns (expand comma-separated values)
    for pattern in &args.packages {
        for p in pattern.split(',') {
            let trimmed = p.trim().to_string();
            if !trimmed.is_empty() && !package_patterns.contains(&trimmed) {
                package_patterns.push(trimmed);
            }
        }
    }

    println!(
        "\n{} Initializing workspace '{}' in '{}'...\n",
        "$".cyan(),
        workspace_name.bold(),
        target_dir.display()
    );

    create_dir_if_missing(&target_dir)?;
    create_dir_if_missing(&target_dir.join("packages"))?;
    if include_apps {
        create_dir_if_missing(&target_dir.join("apps"))?;
    }

    // Generate config files
    if args.legacy {
        write_legacy_config(&target_dir, &workspace_name, &package_patterns)?;
    } else {
        write_7x_config(&target_dir, &workspace_name, &package_patterns)?;
    }

    println!("{}", "Created:".green().bold());
    print_tree(&target_dir, &cwd, args.legacy, include_apps);

    println!(
        "\n{} Run '{}' to bootstrap your workspace.",
        "i".blue(),
        "melos-rs bootstrap".cyan()
    );

    Ok(())
}

/// Print a simple directory tree showing what was created
fn print_tree(dir: &std::path::Path, cwd: &std::path::Path, legacy: bool, include_apps: bool) {
    let display_dir = pathdiff::diff_paths(dir, cwd).unwrap_or_else(|| dir.to_path_buf());

    println!("  {}/", display_dir.display().to_string().bold());
    if legacy {
        println!("  {}", "├── melos.yaml".dimmed());
    }
    println!("  {}", "├── pubspec.yaml".dimmed());
    println!("  {}", "├── packages/".dimmed());
    if include_apps {
        println!("  {}", "└── apps/".dimmed());
    }
}

/// Prompt the user for input with a default value
fn prompt_with_default(label: &str, default: &str) -> Result<String> {
    print!("{} [{}]: ", label.bold(), default.dimmed());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    let trimmed = input.trim();

    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

/// Prompt for yes/no with a default
fn prompt_yes_no(question: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    print!("{} [{}]: ", question.bold(), hint.dimmed());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();

    if trimmed.is_empty() {
        Ok(default_yes)
    } else {
        Ok(trimmed == "y" || trimmed == "yes")
    }
}
