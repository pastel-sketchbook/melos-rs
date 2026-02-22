use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters_with_categories;
use crate::runner::{ProcessRunner, create_progress_bar};
use crate::workspace::Workspace;

/// Arguments for the `analyze` command
#[derive(Args, Debug)]
pub struct AnalyzeArgs {
    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "5")]
    pub concurrency: usize,

    /// Report fatal warnings as errors
    #[arg(long)]
    pub fatal_warnings: bool,

    /// Treat info-level issues as fatal
    #[arg(long)]
    pub fatal_infos: bool,

    /// Don't treat warnings or info as fatal (overrides --fatal-warnings and --fatal-infos)
    #[arg(long)]
    pub no_fatal: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Run `dart analyze` across all matching packages
pub async fn run(workspace: &Workspace, args: AnalyzeArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
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

    println!(
        "\n{} Analyzing {} packages...\n",
        "$".cyan(),
        packages.len()
    );

    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    let cmd_str = build_analyze_command(args.fatal_warnings, args.fatal_infos, args.no_fatal);

    let pb = create_progress_bar(packages.len() as u64, "analyzing");
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

    if failed > 0 {
        anyhow::bail!("{} package(s) have analysis issues", failed);
    }

    println!("\n{}", "All packages passed analysis.".green());
    Ok(())
}

/// Build the `dart analyze` command string from flags.
fn build_analyze_command(fatal_warnings: bool, fatal_infos: bool, no_fatal: bool) -> String {
    let mut cmd_parts = vec!["dart".to_string(), "analyze".to_string()];

    if !no_fatal {
        if fatal_warnings {
            cmd_parts.push("--fatal-warnings".to_string());
        }
        if fatal_infos {
            cmd_parts.push("--fatal-infos".to_string());
        }
    } else {
        cmd_parts.push("--no-fatal-warnings".to_string());
        cmd_parts.push("--no-fatal-infos".to_string());
    }

    // Analyze the current directory (package root)
    cmd_parts.push(".".to_string());

    cmd_parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_analyze_command_default() {
        let cmd = build_analyze_command(false, false, false);
        assert_eq!(cmd, "dart analyze .");
    }

    #[test]
    fn test_build_analyze_command_fatal_warnings() {
        let cmd = build_analyze_command(true, false, false);
        assert_eq!(cmd, "dart analyze --fatal-warnings .");
    }

    #[test]
    fn test_build_analyze_command_fatal_infos() {
        let cmd = build_analyze_command(false, true, false);
        assert_eq!(cmd, "dart analyze --fatal-infos .");
    }

    #[test]
    fn test_build_analyze_command_both_fatal() {
        let cmd = build_analyze_command(true, true, false);
        assert_eq!(cmd, "dart analyze --fatal-warnings --fatal-infos .");
    }

    #[test]
    fn test_build_analyze_command_no_fatal_overrides() {
        // --no-fatal overrides both --fatal-warnings and --fatal-infos
        let cmd = build_analyze_command(true, true, true);
        assert_eq!(cmd, "dart analyze --no-fatal-warnings --no-fatal-infos .");
    }

    #[test]
    fn test_build_analyze_command_no_fatal_alone() {
        let cmd = build_analyze_command(false, false, true);
        assert_eq!(cmd, "dart analyze --no-fatal-warnings --no-fatal-infos .");
    }
}
