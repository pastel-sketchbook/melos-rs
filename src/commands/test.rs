use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters_with_categories;
use crate::runner::{ProcessRunner, create_progress_bar};
use crate::workspace::Workspace;

/// Arguments for the `test` command
#[derive(Args, Debug)]
pub struct TestArgs {
    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "1")]
    pub concurrency: usize,

    /// Abort on first test failure
    #[arg(long)]
    pub fail_fast: bool,

    /// Collect code coverage information
    #[arg(long)]
    pub coverage: bool,

    /// Test randomization seed (0 = random each run)
    #[arg(long)]
    pub test_randomize_ordering_seed: Option<String>,

    /// Update golden files (passes --update-goldens to flutter test)
    #[arg(long)]
    pub update_goldens: bool,

    /// Do not run tests; only list available test files
    #[arg(long)]
    pub no_run: bool,

    /// Additional arguments passed to the test runner (after --)
    #[arg(last = true)]
    pub extra_args: Vec<String>,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Run `dart test` / `flutter test` across all matching packages
pub async fn run(workspace: &Workspace, args: TestArgs) -> Result<()> {
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

    // Only include packages that actually have a test directory
    let testable_packages: Vec<_> = packages
        .into_iter()
        .filter(|pkg| pkg.path.join("test").is_dir())
        .collect();

    if testable_packages.is_empty() {
        println!("{}", "No packages with test/ directory found.".yellow());
        return Ok(());
    }

    if let Some(pre_hook) = workspace.hook("test", "pre") {
        crate::runner::run_lifecycle_hook(pre_hook, "pre-test", &workspace.root_path, &[]).await?;
    }

    println!(
        "\n{} Running tests in {} package(s)...\n",
        "$".cyan(),
        testable_packages.len()
    );

    for pkg in &testable_packages {
        let sdk = if pkg.is_flutter { "flutter" } else { "dart" };
        println!("  {} {} ({})", "->".cyan(), pkg.name, sdk);
    }
    println!();

    let flutter_pkgs: Vec<_> = testable_packages
        .iter()
        .filter(|p| p.is_flutter)
        .cloned()
        .collect();
    let dart_pkgs: Vec<_> = testable_packages
        .iter()
        .filter(|p| !p.is_flutter)
        .cloned()
        .collect();

    let extra_flags = build_extra_flags(&args);

    let pb = create_progress_bar(testable_packages.len() as u64, "testing");
    let runner = ProcessRunner::new(args.concurrency, args.fail_fast);
    let mut all_results = Vec::new();

    if !flutter_pkgs.is_empty() {
        let cmd = build_test_command("flutter", &extra_flags, &args.extra_args);
        pb.set_message("flutter test...");
        let results = runner
            .run_in_packages_with_progress(
                &flutter_pkgs,
                &cmd,
                &workspace.env_vars(),
                None,
                Some(&pb),
                &workspace.packages,
            )
            .await?;
        all_results.extend(results);
    }

    if !dart_pkgs.is_empty() {
        let cmd = build_test_command("dart", &extra_flags, &args.extra_args);
        pb.set_message("dart test...");
        let results = runner
            .run_in_packages_with_progress(
                &dart_pkgs,
                &cmd,
                &workspace.env_vars(),
                None,
                Some(&pb),
                &workspace.packages,
            )
            .await?;
        all_results.extend(results);
    }

    pb.finish_and_clear();

    let failed = all_results.iter().filter(|(_, success)| !success).count();
    let passed = all_results.len() - failed;

    if failed > 0 {
        anyhow::bail!("{} package(s) failed testing ({} passed)", failed, passed);
    }

    println!(
        "\n{}",
        format!("All {} package(s) passed testing.", passed).green()
    );

    if let Some(post_hook) = workspace.hook("test", "post") {
        crate::runner::run_lifecycle_hook(post_hook, "post-test", &workspace.root_path, &[])
            .await?;
    }

    Ok(())
}

/// Build the extra flags string from test args (coverage, randomize, no-run)
fn build_extra_flags(args: &TestArgs) -> Vec<String> {
    let mut flags = Vec::new();

    if args.coverage {
        flags.push("--coverage".to_string());
    }

    if let Some(ref seed) = args.test_randomize_ordering_seed {
        flags.push(format!("--test-randomize-ordering-seed={}", seed));
    }

    if args.no_run {
        flags.push("--no-run".to_string());
    }

    if args.update_goldens {
        flags.push("--update-goldens".to_string());
    }

    flags
}

/// Build the full test command string
fn build_test_command(sdk: &str, extra_flags: &[String], extra_args: &[String]) -> String {
    let mut parts = vec![sdk.to_string(), "test".to_string()];
    parts.extend(extra_flags.iter().cloned());
    parts.extend(extra_args.iter().cloned());
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_test_command_default() {
        let cmd = build_test_command("dart", &[], &[]);
        assert_eq!(cmd, "dart test");
    }

    #[test]
    fn test_build_test_command_flutter_with_coverage() {
        let flags = vec!["--coverage".to_string()];
        let cmd = build_test_command("flutter", &flags, &[]);
        assert_eq!(cmd, "flutter test --coverage");
    }

    #[test]
    fn test_build_test_command_with_all_flags() {
        let flags = vec![
            "--coverage".to_string(),
            "--test-randomize-ordering-seed=42".to_string(),
            "--no-run".to_string(),
        ];
        let extra = vec!["--reporter=expanded".to_string()];
        let cmd = build_test_command("dart", &flags, &extra);
        assert_eq!(
            cmd,
            "dart test --coverage --test-randomize-ordering-seed=42 --no-run --reporter=expanded"
        );
    }

    #[test]
    fn test_build_extra_flags_empty() {
        let args = TestArgs {
            concurrency: 1,
            fail_fast: false,
            coverage: false,
            test_randomize_ordering_seed: None,
            no_run: false,
            update_goldens: false,
            extra_args: vec![],
            filters: GlobalFilterArgs::default(),
        };
        let flags = build_extra_flags(&args);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_build_extra_flags_all() {
        let args = TestArgs {
            concurrency: 5,
            fail_fast: true,
            coverage: true,
            test_randomize_ordering_seed: Some("0".to_string()),
            no_run: true,
            update_goldens: true,
            extra_args: vec![],
            filters: GlobalFilterArgs::default(),
        };
        let flags = build_extra_flags(&args);
        assert_eq!(flags.len(), 4);
        assert!(flags.contains(&"--coverage".to_string()));
        assert!(flags.contains(&"--test-randomize-ordering-seed=0".to_string()));
        assert!(flags.contains(&"--no-run".to_string()));
        assert!(flags.contains(&"--update-goldens".to_string()));
    }

    #[test]
    fn test_build_extra_flags_update_goldens_only() {
        let args = TestArgs {
            concurrency: 1,
            fail_fast: false,
            coverage: false,
            test_randomize_ordering_seed: None,
            no_run: false,
            update_goldens: true,
            extra_args: vec![],
            filters: GlobalFilterArgs::default(),
        };
        let flags = build_extra_flags(&args);
        assert_eq!(flags, vec!["--update-goldens"]);
    }

    #[test]
    fn test_build_test_command_with_update_goldens() {
        let flags = vec!["--update-goldens".to_string()];
        let cmd = build_test_command("flutter", &flags, &[]);
        assert_eq!(cmd, "flutter test --update-goldens");
    }
}
