use anyhow::Result;
use colored::Colorize;

use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Clean all packages by running `flutter clean`
pub async fn run(workspace: &Workspace) -> Result<()> {
    println!(
        "\n{} Cleaning {} packages...\n",
        "$".cyan(),
        workspace.packages.len()
    );

    if workspace.packages.is_empty() {
        println!("{}", "No packages found in workspace.".yellow());
        return Ok(());
    }

    let flutter_packages: Vec<_> = workspace
        .packages
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
