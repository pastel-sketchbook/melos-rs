use anyhow::Result;
use colored::Colorize;

use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Bootstrap the workspace: run `flutter pub get` (or `dart pub get`) in each package
pub async fn run(workspace: &Workspace) -> Result<()> {
    println!(
        "\n{} Bootstrapping {} packages...\n",
        "$".cyan(),
        workspace.packages.len()
    );

    if workspace.packages.is_empty() {
        println!("{}", "No packages found in workspace.".yellow());
        return Ok(());
    }

    for pkg in &workspace.packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    let runner = ProcessRunner::new(1, true);

    // Run `flutter pub get` for Flutter packages, `dart pub get` for pure Dart packages
    let flutter_packages: Vec<_> = workspace
        .packages
        .iter()
        .filter(|p| p.is_flutter)
        .cloned()
        .collect();

    let dart_packages: Vec<_> = workspace
        .packages
        .iter()
        .filter(|p| !p.is_flutter)
        .cloned()
        .collect();

    if !flutter_packages.is_empty() {
        println!("{}", "Running flutter pub get...".dimmed());
        let results = runner
            .run_in_packages(
                &flutter_packages,
                "flutter pub get",
                &workspace.env_vars(),
            )
            .await?;

        for (name, success) in &results {
            if !success {
                anyhow::bail!("flutter pub get failed in package '{}'", name);
            }
        }
    }

    if !dart_packages.is_empty() {
        println!("{}", "Running dart pub get...".dimmed());
        let results = runner
            .run_in_packages(&dart_packages, "dart pub get", &workspace.env_vars())
            .await?;

        for (name, success) in &results {
            if !success {
                anyhow::bail!("dart pub get failed in package '{}'", name);
            }
        }
    }

    println!("\n{}", "All packages bootstrapped.".green());
    Ok(())
}
