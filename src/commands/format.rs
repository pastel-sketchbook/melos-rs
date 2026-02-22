use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Arguments for the `format` command
#[derive(Args, Debug)]
pub struct FormatArgs {
    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "5")]
    pub concurrency: usize,

    /// Set exit code if formatting changes are needed (useful for CI)
    #[arg(long)]
    pub set_exit_if_changed: bool,

    /// Output format: write (default), json, none
    #[arg(short, long, default_value = "write")]
    pub output: String,

    /// Line length (default: 80)
    #[arg(short = 'l', long)]
    pub line_length: Option<u32>,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Format Dart code across all matching packages using `dart format`
pub async fn run(workspace: &Workspace, args: FormatArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters(&workspace.packages, &filters, Some(&workspace.root_path))?;

    println!(
        "\n{} Formatting {} packages...\n",
        "$".cyan(),
        packages.len()
    );

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // Build the dart format command with flags
    let mut cmd_parts = vec!["dart".to_string(), "format".to_string()];

    if args.set_exit_if_changed {
        cmd_parts.push("--set-exit-if-changed".to_string());
    }

    if args.output != "write" {
        cmd_parts.push(format!("--output={}", args.output));
    }

    if let Some(line_length) = args.line_length {
        cmd_parts.push(format!("--line-length={}", line_length));
    }

    // Format the current directory (package root)
    cmd_parts.push(".".to_string());

    let cmd_str = cmd_parts.join(" ");

    let runner = ProcessRunner::new(args.concurrency, false);
    let results = runner
        .run_in_packages(&packages, &cmd_str, &workspace.env_vars())
        .await?;

    let failed = results.iter().filter(|(_, success)| !success).count();

    if failed > 0 {
        if args.set_exit_if_changed {
            anyhow::bail!(
                "{} package(s) have formatting changes. Run `melos-rs format` to fix.",
                failed
            );
        }
        anyhow::bail!("{} package(s) failed to format", failed);
    }

    println!("\n{}", "All packages formatted.".green());
    Ok(())
}
