use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use clap::Args;
use colored::Colorize;
use regex::Regex;
use tokio::sync::Semaphore;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters_with_categories;
use crate::runner::{ProcessRunner, create_progress_bar, shell_command};
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
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // --dry-run: preview fixes, parse output, display consolidated results
    if args.dry_run {
        let fix_cmd = build_fix_command(false, &args.code);

        let semaphore = Arc::new(Semaphore::new(args.concurrency));
        let pb = create_progress_bar(packages.len() as u64, "previewing fixes");
        let mut handles = Vec::new();

        for pkg in &packages {
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

        let mut all_entries: Vec<DryRunFileEntry> = Vec::new();
        let mut all_codes: BTreeSet<String> = BTreeSet::new();

        for handle in handles {
            let entries = handle.await?;
            for entry in &entries {
                for (code, _) in &entry.fixes {
                    all_codes.insert(code.clone());
                }
            }
            all_entries.extend(entries);
        }

        pb.finish_and_clear();

        // Sort by path for deterministic output across concurrent runs
        all_entries.sort_by(|a, b| a.path.cmp(&b.path));

        if all_entries.is_empty() {
            println!("{}", "Nothing to fix!".green());
        } else {
            for entry in &all_entries {
                println!("{}", entry.path);
                for (code, count) in &entry.fixes {
                    let label = if *count == 1 { "fix" } else { "fixes" };
                    println!("  {} \u{2022} {} {}", code, count, label);
                }
                println!();
            }

            match all_codes.len() {
                1 => println!("To fix this diagnostic, run:"),
                _ => println!("To fix an individual diagnostic, run one of:"),
            }
            for code in &all_codes {
                println!("  dart fix --apply --code={}", code);
                println!("  melos-rs analyze --fix --code={}", code);
            }
            println!();
            println!("To fix all diagnostics, run:");
            println!("  dart fix --apply");
            println!("  melos-rs analyze --fix");
        }

        println!("\n{}", "Dry run complete. No changes were applied.".green());
        return Ok(());
    }

    // --fix: apply fixes before analysis
    if args.fix {
        let fix_cmd = build_fix_command(true, &args.code);
        let fix_pb = create_progress_bar(packages.len() as u64, "fixing");
        let fix_runner = ProcessRunner::new(args.concurrency, false);
        let fix_results = fix_runner
            .run_in_packages_with_progress(
                &packages,
                &fix_cmd,
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

/// Build a `dart fix` command string.
///
/// - `apply`: true for `--apply`, false for `--dry-run`
/// - `codes`: optional diagnostic codes to restrict fixes to
fn build_fix_command(apply: bool, codes: &[String]) -> String {
    let mut parts = vec!["dart".to_string(), "fix".to_string()];
    parts.push(if apply {
        "--apply".to_string()
    } else {
        "--dry-run".to_string()
    });
    for code in codes {
        parts.push(format!("--code={code}"));
    }
    parts.join(" ")
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

/// A file entry parsed from `dart fix --dry-run` output.
#[derive(Debug)]
struct DryRunFileEntry {
    /// Path relative to workspace root (e.g., "packages/ui/lib/foo.dart")
    path: String,
    /// Diagnostic fixes: (code, count)
    fixes: Vec<(String, usize)>,
}

/// Regex for diagnostic fix lines from `dart fix --dry-run` output.
///
/// Matches patterns like:
///   `omit_local_variable_types - 2 fixes`        (Dart SDK 3.x)
///   `omit_local_variable_types \u{2022} 2 fixes`  (older SDKs)
///
/// Captures: (1) diagnostic code, (2) fix count.
/// The separator is matched as any non-word, non-digit character sequence.
static FIX_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // safety: this regex is valid and tested
    Regex::new(r"^(\w+)\s+\S+\s+(\d+)\s+fix(?:es)?$").expect("valid regex")
});

/// Parse a single diagnostic fix line from `dart fix --dry-run` output.
///
/// Returns `(code, count)` on success.
fn parse_fix_line(line: &str) -> Option<(String, usize)> {
    let caps = FIX_LINE_RE.captures(line)?;
    let code = caps[1].to_string();
    let count: usize = caps[2].parse().ok()?;
    Some((code, count))
}

/// Parse `dart fix --dry-run` stdout into file entries.
///
/// Each file path is prefixed with `pkg_prefix` (the package's relative path
/// from the workspace root) to produce workspace-relative paths.
///
/// Example output from Dart SDK 3.x:
/// ```text
/// Computing fixes in ui (dry run)...
///
/// 12 proposed fixes in 2 files.
///
/// lib/shared/router.helper.dart
///   omit_local_variable_types - 4 fixes
///   specify_nonobvious_local_variable_types - 4 fixes
///
/// lib/utils/register_fonts.dart
///   omit_local_variable_types - 2 fixes
///
/// To fix an individual diagnostic, run one of:
///   dart fix --apply --code=omit_local_variable_types
/// ```
fn parse_dry_run_output(stdout: &str, pkg_prefix: &str) -> Vec<DryRunFileEntry> {
    let mut entries = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_fixes: Vec<(String, usize)> = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();

        // Skip blank lines and known non-file lines; flush any pending entry
        if trimmed.is_empty()
            || trimmed.starts_with("Computing fixes")
            || trimmed.starts_with("Nothing to fix")
            || trimmed.starts_with("To fix")
            || trimmed.contains("fixes in")
            || trimmed.contains("fix in")
        {
            if let Some(path) = current_path.take()
                && !current_fixes.is_empty()
            {
                entries.push(DryRunFileEntry {
                    path,
                    fixes: std::mem::take(&mut current_fixes),
                });
            }
            continue;
        }

        // Indented line = diagnostic fix (or dart fix suggestion, which the regex skips)
        if line.starts_with("  ") {
            if let Some((code, count)) = parse_fix_line(trimmed) {
                current_fixes.push((code, count));
            }
        } else if trimmed.ends_with(".dart") {
            // Non-indented Dart file path = start of new file entry
            if let Some(path) = current_path.take()
                && !current_fixes.is_empty()
            {
                entries.push(DryRunFileEntry {
                    path,
                    fixes: std::mem::take(&mut current_fixes),
                });
            }
            current_path = Some(format!("{}/{}", pkg_prefix, trimmed));
        }
        // Any other non-indented line is ignored (future-proofing)
    }

    // Flush last entry
    if let Some(path) = current_path
        && !current_fixes.is_empty()
    {
        entries.push(DryRunFileEntry {
            path,
            fixes: current_fixes,
        });
    }

    entries
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
    fn test_build_fix_command_apply() {
        let cmd = build_fix_command(true, &[]);
        assert_eq!(cmd, "dart fix --apply");
    }

    #[test]
    fn test_build_fix_command_dry_run() {
        let cmd = build_fix_command(false, &[]);
        assert_eq!(cmd, "dart fix --dry-run");
    }

    #[test]
    fn test_build_fix_command_apply_with_codes() {
        let codes = vec![
            "deprecated_member_use".to_string(),
            "unused_import".to_string(),
        ];
        let cmd = build_fix_command(true, &codes);
        assert_eq!(
            cmd,
            "dart fix --apply --code=deprecated_member_use --code=unused_import"
        );
    }

    #[test]
    fn test_build_fix_command_dry_run_with_single_code() {
        let codes = vec!["unnecessary_cast".to_string()];
        let cmd = build_fix_command(false, &codes);
        assert_eq!(cmd, "dart fix --dry-run --code=unnecessary_cast");
    }

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

        // --fix and --dry-run are mutually exclusive
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

    // ── parse_fix_line tests ───────────────────────────────────────────

    #[test]
    fn test_parse_fix_line_dash_separator() {
        let result = parse_fix_line("omit_local_variable_types - 4 fixes");
        assert_eq!(result, Some(("omit_local_variable_types".to_string(), 4)));
    }

    #[test]
    fn test_parse_fix_line_bullet_separator() {
        let result = parse_fix_line("omit_local_variable_types \u{2022} 4 fixes");
        assert_eq!(result, Some(("omit_local_variable_types".to_string(), 4)));
    }

    #[test]
    fn test_parse_fix_line_single_fix() {
        let result = parse_fix_line("unused_import - 1 fix");
        assert_eq!(result, Some(("unused_import".to_string(), 1)));
    }

    #[test]
    fn test_parse_fix_line_dart_fix_command() {
        // Footer lines like "dart fix --apply --code=foo" must not match
        assert!(parse_fix_line("dart fix --apply --code=foo").is_none());
    }

    #[test]
    fn test_parse_fix_line_empty() {
        assert!(parse_fix_line("").is_none());
    }

    // ── parse_dry_run_output tests ─────────────────────────────────────

    #[test]
    fn test_parse_dry_run_output_dart3_format() {
        // Real output from Dart SDK 3.x: summary before files, dash separator
        let stdout = "\
Computing fixes in ui (dry run)...

112 proposed fixes in 13 files.

lib/app.router.dart
  omit_local_variable_types - 2 fixes
  specify_nonobvious_local_variable_types - 2 fixes

lib/main.dart
  omit_local_variable_types - 4 fixes
  specify_nonobvious_local_variable_types - 4 fixes

To fix an individual diagnostic, run one of:
  dart fix --apply --code=omit_local_variable_types
  dart fix --apply --code=specify_nonobvious_local_variable_types

To fix all diagnostics, run:
  dart fix --apply";

        let entries = parse_dry_run_output(stdout, "packages/ui");
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].path, "packages/ui/lib/app.router.dart");
        assert_eq!(entries[0].fixes.len(), 2);
        assert_eq!(
            entries[0].fixes[0],
            ("omit_local_variable_types".to_string(), 2)
        );
        assert_eq!(
            entries[0].fixes[1],
            ("specify_nonobvious_local_variable_types".to_string(), 2)
        );

        assert_eq!(entries[1].path, "packages/ui/lib/main.dart");
        assert_eq!(entries[1].fixes.len(), 2);
        assert_eq!(
            entries[1].fixes[0],
            ("omit_local_variable_types".to_string(), 4)
        );
    }

    #[test]
    fn test_parse_dry_run_output_bullet_separator() {
        // Older SDK format: summary after files, bullet separator
        let stdout = "\
Computing fixes in /workspace/packages/ui...

lib/shared/router.helper.dart
  omit_local_variable_types \u{2022} 4 fixes
  specify_nonobvious_local_variable_types \u{2022} 4 fixes

lib/utils/register_fonts.dart
  omit_local_variable_types \u{2022} 2 fixes
  specify_nonobvious_local_variable_types \u{2022} 2 fixes

12 fixes in 2 files.

To fix an individual diagnostic, run one of:
  dart fix --apply --code=omit_local_variable_types
  dart fix --apply --code=specify_nonobvious_local_variable_types

To fix all diagnostics, run:
  dart fix --apply";

        let entries = parse_dry_run_output(stdout, "packages/ui");
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].path, "packages/ui/lib/shared/router.helper.dart");
        assert_eq!(entries[0].fixes.len(), 2);
        assert_eq!(
            entries[0].fixes[0],
            ("omit_local_variable_types".to_string(), 4)
        );

        assert_eq!(entries[1].path, "packages/ui/lib/utils/register_fonts.dart");
        assert_eq!(entries[1].fixes.len(), 2);
    }

    #[test]
    fn test_parse_dry_run_output_single_file() {
        let stdout = "\
Computing fixes in core (dry run)...

1 proposed fix in 1 file.

lib/src/utils.dart
  unnecessary_cast - 1 fix";

        let entries = parse_dry_run_output(stdout, "packages/core");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "packages/core/lib/src/utils.dart");
        assert_eq!(entries[0].fixes, vec![("unnecessary_cast".to_string(), 1)]);
    }

    #[test]
    fn test_parse_dry_run_output_nothing_to_fix() {
        let stdout = "Computing fixes in core (dry run)...\nNothing to fix!";
        let entries = parse_dry_run_output(stdout, "packages/core");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_dry_run_output_empty() {
        let entries = parse_dry_run_output("", "packages/core");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_dry_run_output_skips_footer_suggestions() {
        let stdout = "\
lib/foo.dart
  unused_import - 3 fixes

3 fixes in 1 file.

To fix an individual diagnostic, run one of:
  dart fix --apply --code=unused_import

To fix all diagnostics, run:
  dart fix --apply";

        let entries = parse_dry_run_output(stdout, "pkg");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "pkg/lib/foo.dart");
        assert_eq!(entries[0].fixes, vec![("unused_import".to_string(), 3)]);
    }

    #[test]
    fn test_parse_dry_run_output_ignores_non_dart_lines() {
        // If the output contains unexpected non-dart lines, they are ignored
        let stdout = "\
Computing fixes in ui (dry run)...

Some unexpected line here

lib/foo.dart
  unused_import - 1 fix";

        let entries = parse_dry_run_output(stdout, "pkg");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "pkg/lib/foo.dart");
    }
}
