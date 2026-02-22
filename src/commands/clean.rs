use anyhow::Result;
use colored::Colorize;

use crate::cli::CleanArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Clean all packages by running `flutter clean`
pub async fn run(workspace: &Workspace, args: CleanArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let all_filtered = apply_filters(&workspace.packages, &filters, Some(&workspace.root_path))?;

    println!(
        "\n{} Cleaning {} packages...\n",
        "$".cyan(),
        all_filtered.len()
    );

    if all_filtered.is_empty() {
        println!("{}", "No packages found in workspace.".yellow());
        return Ok(());
    }

    let flutter_packages: Vec<_> = all_filtered
        .iter()
        .filter(|p| p.is_flutter)
        .cloned()
        .collect();

    if flutter_packages.is_empty() {
        println!("{}", "No Flutter packages to clean.".yellow());
        return Ok(());
    }

    let runner = ProcessRunner::new(1, false);
    let results = runner
        .run_in_packages(
            &flutter_packages,
            "flutter clean",
            &workspace.env_vars(),
        )
        .await?;

    let mut failed = 0;
    for (name, success) in &results {
        if *success {
            println!("  {} {}", "CLEANED".green(), name);
        } else {
            println!("  {} {}", "FAILED".red(), name);
            failed += 1;
        }
    }

    if failed > 0 {
        anyhow::bail!("{} package(s) failed to clean", failed);
    }

    Ok(())
}
