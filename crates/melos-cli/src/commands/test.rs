use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use melos_core::commands::test::TestOpts;
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::workspace::Workspace;

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

    let opts = TestOpts {
        concurrency: args.concurrency,
        fail_fast: args.fail_fast,
        coverage: args.coverage,
        test_randomize_ordering_seed: args.test_randomize_ordering_seed,
        update_goldens: args.update_goldens,
        no_run: args.no_run,
        extra_args: args.extra_args,
    };

    let (tx, render_handle) = crate::render::spawn_renderer(testable_packages.len(), "testing");
    let results =
        melos_core::commands::test::run(&testable_packages, workspace, &opts, Some(&tx)).await?;
    drop(tx);
    render_handle.await??;

    let failed = results.failed();
    let passed = results.passed();

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
