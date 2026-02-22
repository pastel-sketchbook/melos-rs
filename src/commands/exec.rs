use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::{apply_filters_with_categories, topological_sort};
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Arguments for the `exec` command
#[derive(Args, Debug)]
pub struct ExecArgs {
    /// Command to execute in each package
    #[arg(trailing_var_arg = true, required = true)]
    pub command: Vec<String>,

    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "5")]
    pub concurrency: usize,

    /// Stop execution on first failure
    #[arg(long)]
    pub fail_fast: bool,

    /// Execute packages in dependency order (topological sort)
    #[arg(long)]
    pub order_dependents: bool,

    /// Timeout per package in seconds (0 = no timeout)
    #[arg(long, default_value = "0")]
    pub timeout: u64,

    /// Print commands without executing them
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Execute a command across all matching packages
pub async fn run(workspace: &Workspace, args: ExecArgs) -> Result<()> {
    let cmd_str = args.command.join(" ");
    println!(
        "\n{} Running '{}' in packages...\n",
        "$".cyan(),
        cmd_str.bold()
    );

    // Apply filters from CLI flags
    let filters: PackageFilters = (&args.filters).into();
    let mut packages = apply_filters_with_categories(&workspace.packages, &filters, Some(&workspace.root_path), &workspace.config.categories)?;

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    // Apply topological sort if requested
    if args.order_dependents {
        packages = topological_sort(&packages);
        println!(
            "{} Packages ordered by dependencies (topological sort)\n",
            "i".blue()
        );
    }

    let timeout_display = if args.timeout > 0 {
        format!(", timeout {}s", args.timeout)
    } else {
        String::new()
    };

    println!(
        "Running in {} package(s) with concurrency {}{}:\n",
        packages.len().to_string().cyan(),
        args.concurrency.to_string().cyan(),
        timeout_display,
    );

    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // Dry-run mode: show what would be executed without running
    if args.dry_run {
        println!("{}", "DRY RUN — no commands were executed.".yellow().bold());
        return Ok(());
    }

    // Build timeout Duration (0 means no timeout)
    let timeout = if args.timeout > 0 {
        Some(std::time::Duration::from_secs(args.timeout))
    } else {
        None
    };

    // Execute command in each package (runner handles per-package env vars + colored output)
    let runner = ProcessRunner::new(args.concurrency, args.fail_fast);
    let results = runner
        .run_in_packages(&packages, &cmd_str, &workspace.env_vars(), timeout)
        .await?;

    // Count failures
    let failed = results.iter().filter(|(_, success)| !success).count();

    if failed > 0 {
        anyhow::bail!("{} package(s) failed", failed);
    }

    Ok(())
}
