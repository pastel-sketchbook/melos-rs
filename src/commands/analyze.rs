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

    /// Run `dart fix --apply` in each package before analyzing
    #[arg(long)]
    pub fix: bool,

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

    // Run dart fix --apply before analysis if --fix was requested
    if args.fix {
        println!(
            "{} Running {} in {} packages...\n",
            "$".cyan(),
            "dart fix --apply".bold(),
            packages.len()
        );

        let fix_pb = create_progress_bar(packages.len() as u64, "fixing");
        let fix_runner = ProcessRunner::new(args.concurrency, false);
        let fix_results = fix_runner
            .run_in_packages_with_progress(
                &packages,
                FIX_COMMAND,
                &workspace.env_vars(),
                None,
                Some(&fix_pb),
                &workspace.packages,
            )
            .await?;
        fix_pb.finish_and_clear();

        let fix_failed = fix_results.iter().filter(|(_, success)| !success).count();
        if fix_failed > 0 {
            println!(
                "{}",
                format!(
                    "Warning: dart fix --apply failed in {} package(s), continuing with analysis...",
                    fix_failed
                )
                .yellow()
            );
        } else {
            println!(
                "{}",
                format!("Applied fixes in {} package(s).", fix_results.len()).green()
            );
        }
        println!();
    }

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
    let passed = results.len() - failed;

    if failed > 0 {
        anyhow::bail!("{} package(s) failed analysis ({} passed)", failed, passed);
    }

    println!(
        "\n{}",
        format!("All {} package(s) passed analysis.", passed).green()
    );
    Ok(())
}

/// The command string used for `dart fix --apply`.
const FIX_COMMAND: &str = "dart fix --apply";

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

    #[test]
    fn test_fix_command_constant() {
        assert_eq!(FIX_COMMAND, "dart fix --apply");
    }

    #[test]
    fn test_analyze_args_has_fix_field() {
        // Verify the fix field exists and defaults correctly via clap
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: AnalyzeArgs,
        }

        // Without --fix
        let cli = TestCli::parse_from(["test"]);
        assert!(!cli.args.fix);

        // With --fix
        let cli = TestCli::parse_from(["test", "--fix"]);
        assert!(cli.args.fix);
    }

    #[test]
    fn test_analyze_args_fix_with_fatal_warnings() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: AnalyzeArgs,
        }

        let cli = TestCli::parse_from(["test", "--fix", "--fatal-warnings"]);
        assert!(cli.args.fix);
        assert!(cli.args.fatal_warnings);
        assert!(!cli.args.fatal_infos);
        assert!(!cli.args.no_fatal);
    }
}
