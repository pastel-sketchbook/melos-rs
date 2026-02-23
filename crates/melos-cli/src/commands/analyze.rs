use std::sync::Arc;

use anyhow::Result;
use clap::Args;
use colored::Colorize;
use tokio::sync::Semaphore;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use crate::render::{create_progress_bar, spawn_renderer};
use melos_core::commands::analyze::{
    AnalyzeOpts, assemble_dry_run_scan, build_fix_command, format_conflict_warnings,
    parse_dry_run_output,
};
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::runner::shell_command;
use melos_core::workspace::Workspace;

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
    #[arg(long, conflicts_with = "dry_run")]
    pub fix: bool,

    /// Preview fixes with `dart fix --dry-run` (no changes applied, skips analysis)
    #[arg(long, conflicts_with = "fix")]
    pub dry_run: bool,

    /// Apply fixes only for specific diagnostic codes (comma-separated, requires --fix or --dry-run)
    #[arg(long, value_delimiter = ',')]
    pub code: Vec<String>,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Run `dart analyze` across all matching packages
pub async fn run(workspace: &Workspace, args: AnalyzeArgs) -> Result<()> {
    // --code requires --fix or --dry-run
    if !args.code.is_empty() && !args.fix && !args.dry_run {
        anyhow::bail!("--code requires --fix or --dry-run");
    }

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

    let action = if args.dry_run {
        "Previewing fixes for"
    } else if args.fix {
        "Fixing and analyzing"
    } else {
        "Analyzing"
    };

    println!(
        "\n{} {} {} packages...\n",
        "$".cyan(),
        action,
        packages.len()
    );

    for pkg in &packages {
        let sdk = if pkg.is_flutter { "flutter" } else { "dart" };
        println!("  {} {} ({})", "->".cyan(), pkg.name, sdk);
    }
    println!();

    // --dry-run: preview fixes, parse output, display consolidated results
    if args.dry_run {
        let scan = scan_dry_run(
            &packages,
            workspace,
            args.concurrency,
            &args.code,
            "scanning for conflicts",
        )
        .await?;

        if scan.entries.is_empty() {
            println!("{}", "Nothing to fix!".green());
        } else {
            for entry in &scan.entries {
                println!("{}", entry.path);
                for (code, count) in &entry.fixes {
                    let label = if *count == 1 { "fix" } else { "fixes" };
                    println!("  {} \u{2022} {} {}", code, count, label);
                }
                println!();
            }

            match scan.codes.len() {
                1 => println!("To fix this diagnostic, run:"),
                _ => println!("To fix an individual diagnostic, run one of:"),
            }
            for code in &scan.codes {
                println!("  dart fix --apply --code={}", code);
                println!("  melos-rs analyze --fix --code={}", code);
            }
            println!();
            println!("To fix all diagnostics, run:");
            println!("  dart fix --apply");
            println!("  melos-rs analyze --fix");

            if !scan.conflicts.is_empty() {
                println!();
                println!("{}", format_conflict_warnings(&scan.conflicts).yellow());
            }
        }

        println!("\n{}", "Dry run complete. No changes were applied.".green());
        println!();
    }

    // --fix: apply fixes before analysis (with conflict pre-scan)
    if args.fix {
        let mut skip_fix = false;
        if args.code.is_empty() {
            let scan = scan_dry_run(
                &packages,
                workspace,
                args.concurrency,
                &args.code,
                "previewing fixes",
            )
            .await?;
            if !scan.conflicts.is_empty() {
                println!("{}", format_conflict_warnings(&scan.conflicts).yellow());
                println!();
                println!(
                    "{}",
                    "Skipping dart fix --apply to avoid a fix/analyze loop.".yellow()
                );
                println!(
                    "{}",
                    "Use --code=<diagnostic> to apply a specific fix, or resolve the conflict in analysis_options.yaml.".yellow()
                );
                println!();
                skip_fix = true;
            }
        }

        if !skip_fix {
            let fix_cmd = build_fix_command(true, &args.code);
            let (fix_tx, fix_render) = spawn_renderer(packages.len(), "fixing");
            let fix_runner = melos_core::runner::ProcessRunner::new(args.concurrency, false);
            let fix_results = fix_runner
                .run_in_packages_with_events(
                    &packages,
                    &fix_cmd,
                    &workspace.env_vars(),
                    None,
                    Some(&fix_tx),
                    &workspace.packages,
                )
                .await?;
            drop(fix_tx);
            fix_render.await??;

            let fix_failed = fix_results.iter().filter(|(_, success)| !success).count();
            if fix_failed > 0 {
                println!(
                    "{}",
                    format!(
                        "Warning: {} failed in {} package(s), continuing with analysis...",
                        fix_cmd, fix_failed
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
    }

    let opts = AnalyzeOpts {
        concurrency: args.concurrency,
        fatal_warnings: args.fatal_warnings,
        fatal_infos: args.fatal_infos,
        no_fatal: args.no_fatal,
    };

    let (tx, render_handle) = spawn_renderer(packages.len(), "analyzing");
    let results =
        melos_core::commands::analyze::run(&packages, workspace, &opts, Some(&tx)).await?;
    drop(tx);
    render_handle.await??;

    if results.failed() > 0 {
        anyhow::bail!(
            "{} package(s) failed analysis ({} passed)",
            results.failed(),
            results.passed()
        );
    }

    println!(
        "\n{}",
        format!("All {} package(s) passed analysis.", results.passed()).green()
    );
    Ok(())
}

/// Run `dart fix --dry-run` across packages and parse output.
///
/// Returns consolidated file entries, unique diagnostic codes, and any
/// conflicting lint rule pairs detected via the equal-count heuristic.
async fn scan_dry_run(
    packages: &[melos_core::package::Package],
    workspace: &Workspace,
    concurrency: usize,
    codes: &[String],
    progress_label: &str,
) -> Result<melos_core::commands::analyze::DryRunScan> {
    let fix_cmd = build_fix_command(false, codes);

    let semaphore = Arc::new(Semaphore::new(concurrency));
    let pb = create_progress_bar(packages.len() as u64, progress_label);
    let mut handles = Vec::new();

    for pkg in packages {
        let sem = semaphore.clone();
        let cmd = fix_cmd.clone();
        let pkg_path = pkg.path.clone();
        let root_path = workspace.root_path.clone();
        let env = workspace.env_vars();
        let pb = pb.clone();

        handles.push(tokio::spawn(async move {
            // safety: semaphore is never closed in this scope
            let _permit = sem.acquire().await.expect("semaphore closed unexpectedly");
            let (shell, flag) = shell_command();
            let result = tokio::process::Command::new(shell)
                .arg(flag)
                .arg(&cmd)
                .current_dir(&pkg_path)
                .envs(&env)
                .output()
                .await;
            pb.inc(1);

            let prefix = pkg_path
                .strip_prefix(&root_path)
                .unwrap_or(&pkg_path)
                .to_string_lossy()
                .to_string();

            match result {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    parse_dry_run_output(&stdout, &prefix)
                }
                Err(_) => Vec::new(),
            }
        }));
    }

    let mut all_entries = Vec::new();

    for handle in handles {
        let pkg_entries = handle.await?;
        all_entries.extend(pkg_entries);
    }

    pb.finish_and_clear();

    Ok(assemble_dry_run_scan(all_entries, 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    // build_analyze_command, build_fix_command, parse_fix_line, parse_dry_run_output,
    // detect_conflicting_diagnostics, format_conflict_warnings, and pre-scan skip logic
    // tests have moved to melos_core::commands::analyze

    #[test]
    fn test_analyze_args_defaults() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: AnalyzeArgs,
        }

        let cli = TestCli::parse_from(["test"]);
        assert!(!cli.args.fix);
        assert!(!cli.args.dry_run);
        assert!(!cli.args.fatal_warnings);
        assert!(!cli.args.fatal_infos);
        assert!(!cli.args.no_fatal);
    }

    #[test]
    fn test_analyze_args_fix_flag() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: AnalyzeArgs,
        }

        let cli = TestCli::parse_from(["test", "--fix"]);
        assert!(cli.args.fix);
        assert!(!cli.args.dry_run);
    }

    #[test]
    fn test_analyze_args_dry_run_flag() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: AnalyzeArgs,
        }

        let cli = TestCli::parse_from(["test", "--dry-run"]);
        assert!(cli.args.dry_run);
        assert!(!cli.args.fix);
    }

    #[test]
    fn test_analyze_args_fix_and_dry_run_conflict() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: AnalyzeArgs,
        }

        let result = TestCli::try_parse_from(["test", "--fix", "--dry-run"]);
        assert!(result.is_err());
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
        assert!(!cli.args.dry_run);
    }

    #[test]
    fn test_analyze_args_code_with_fix() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: AnalyzeArgs,
        }

        let cli = TestCli::parse_from([
            "test",
            "--fix",
            "--code",
            "deprecated_member_use,unused_import",
        ]);
        assert!(cli.args.fix);
        assert_eq!(
            cli.args.code,
            vec!["deprecated_member_use", "unused_import"]
        );
    }

    #[test]
    fn test_analyze_args_code_with_dry_run() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: AnalyzeArgs,
        }

        let cli = TestCli::parse_from(["test", "--dry-run", "--code", "unnecessary_cast"]);
        assert!(cli.args.dry_run);
        assert_eq!(cli.args.code, vec!["unnecessary_cast"]);
    }
}
