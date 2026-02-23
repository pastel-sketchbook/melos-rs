use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use crate::runner::{ProcessRunner, create_progress_bar};
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

    let cmd_str = build_format_command(args.set_exit_if_changed, &args.output, args.line_length);

    let pb = create_progress_bar(packages.len() as u64, "formatting");
    let runner = ProcessRunner::new(args.concurrency, false);
    let results = runner
        .run_in_packages_with_progress(
            &packages,
            &cmd_str,
            &workspace.env_vars(),
            None,
            Some(&pb),
            &workspace.packages,
        )
        .await?;
    pb.finish_and_clear();

    let failed = results.iter().filter(|(_, success)| !success).count();
    let passed = results.len() - failed;

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

/// Build the `dart format` command string from flags.
fn build_format_command(
    set_exit_if_changed: bool,
    output: &str,
    line_length: Option<u32>,
) -> String {
    let mut cmd_parts = vec!["dart".to_string(), "format".to_string()];

    if set_exit_if_changed {
        cmd_parts.push("--set-exit-if-changed".to_string());
    }

    if output != "write" {
        cmd_parts.push(format!("--output={}", output));
    }

    if let Some(line_length) = line_length {
        cmd_parts.push(format!("--line-length={}", line_length));
    }

    // Format the current directory (package root)
    cmd_parts.push(".".to_string());

    cmd_parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_format_command_default() {
        let cmd = build_format_command(false, "write", None);
        assert_eq!(cmd, "dart format .");
    }

    #[test]
    fn test_build_format_command_set_exit_if_changed() {
        let cmd = build_format_command(true, "write", None);
        assert_eq!(cmd, "dart format --set-exit-if-changed .");
    }

    #[test]
    fn test_build_format_command_json_output() {
        let cmd = build_format_command(false, "json", None);
        assert_eq!(cmd, "dart format --output=json .");
    }

    #[test]
    fn test_build_format_command_none_output() {
        let cmd = build_format_command(false, "none", None);
        assert_eq!(cmd, "dart format --output=none .");
    }

    #[test]
    fn test_build_format_command_line_length() {
        let cmd = build_format_command(false, "write", Some(120));
        assert_eq!(cmd, "dart format --line-length=120 .");
    }

    #[test]
    fn test_build_format_command_all_flags() {
        let cmd = build_format_command(true, "json", Some(80));
        assert_eq!(
            cmd,
            "dart format --set-exit-if-changed --output=json --line-length=80 ."
        );
    }

    #[test]
    fn test_build_format_command_write_output_not_added() {
        // "write" is the default and should not be added to the command
        let cmd = build_format_command(false, "write", None);
        assert!(!cmd.contains("--output"));
    }
}
