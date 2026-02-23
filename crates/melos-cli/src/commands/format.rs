use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use melos_core::commands::format::FormatOpts;
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::workspace::Workspace;

/// Arguments for the `format` command
#[derive(Args, Debug)]
pub struct FormatArgs {
    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "1")]
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
    let filters = package_filters_from_args(&args.filters);
    let packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

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

    let opts = FormatOpts {
        concurrency: args.concurrency,
        set_exit_if_changed: args.set_exit_if_changed,
        output: args.output.clone(),
        line_length: args.line_length,
    };

    let (tx, render_handle) = crate::render::spawn_renderer(packages.len(), "formatting");
    let results = melos_core::commands::format::run(&packages, workspace, &opts, Some(&tx)).await?;
    drop(tx);
    render_handle.await??;

    let failed = results.failed();
    let passed = results.passed();

    if failed > 0 {
        if args.set_exit_if_changed {
            anyhow::bail!(
                "{} package(s) have formatting changes ({} passed). Run `melos-rs format` to fix.",
                failed,
                passed
            );
        }
        anyhow::bail!(
            "{} package(s) failed formatting ({} passed)",
            failed,
            passed
        );
    }

    println!(
        "\n{}",
        format!("All {} package(s) passed formatting.", passed).green()
    );
    Ok(())
}
