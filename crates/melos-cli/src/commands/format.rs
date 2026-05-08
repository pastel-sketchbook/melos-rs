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

    /// Output format: write, json, none (defaults to config value or "write")
    #[arg(short, long)]
    pub output: Option<String>,

    /// Line length (defaults to config value or dart format default)
    #[arg(short = 'l', long)]
    pub line_length: Option<u32>,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Resolve format options by merging CLI args over config defaults.
///
/// Priority: CLI flag > `command.format` config > built-in default.
fn resolve_format_opts(workspace: &Workspace, args: &FormatArgs) -> FormatOpts {
    let cfg = workspace
        .config
        .command
        .as_ref()
        .and_then(|c| c.format.as_ref());

    let set_exit_if_changed = if args.set_exit_if_changed {
        true
    } else {
        cfg.and_then(|c| c.set_exit_if_changed).unwrap_or(false)
    };

    let output = args
        .output
        .clone()
        .or_else(|| cfg.and_then(|c| c.output.clone()))
        .unwrap_or_else(|| "write".to_string());

    let line_length = args
        .line_length
        .or_else(|| cfg.and_then(|c| c.line_length));

    FormatOpts {
        concurrency: args.concurrency,
        set_exit_if_changed,
        output,
        line_length,
    }
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

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    if let Some(pre_hook) = workspace.hook("format", "pre") {
        crate::runner::run_lifecycle_hook(pre_hook, "pre-format", &workspace.root_path, &[])
            .await?;
    }

    println!(
        "\n{} Formatting {} packages...\n",
        "$".cyan(),
        packages.len()
    );

    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    let opts = resolve_format_opts(workspace, &args);

    let (tx, render_handle) = crate::render::spawn_renderer(packages.len(), "formatting");
    let results = melos_core::commands::format::run(&packages, workspace, &opts, Some(&tx)).await?;
    drop(tx);
    render_handle.await??;

    let failed = results.failed();
    let passed = results.passed();

    if failed > 0 {
        if opts.set_exit_if_changed {
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

    if let Some(post_hook) = workspace.hook("format", "post") {
        crate::runner::run_lifecycle_hook(post_hook, "post-format", &workspace.root_path, &[])
            .await?;
    }

    Ok(())
}
