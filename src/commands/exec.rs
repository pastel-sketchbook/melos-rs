use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters;
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
    let packages = apply_filters(&workspace.packages, &filters, Some(&workspace.root_path))?;

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    println!(
        "Running in {} package(s) with concurrency {}:\n",
        packages.len().to_string().cyan(),
        args.concurrency.to_string().cyan()
    );

    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // Execute command in each package (runner handles per-package env vars + colored output)
    let runner = ProcessRunner::new(args.concurrency, args.fail_fast);
    let results = runner
        .run_in_packages(&packages, &cmd_str, &workspace.env_vars())
        .await?;

    // Count failures
    let failed = results.iter().filter(|(_, success)| !success).count();

    if failed > 0 {
        anyhow::bail!("{} package(s) failed", failed);
    }

    Ok(())
}
