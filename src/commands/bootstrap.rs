use anyhow::Result;
use colored::Colorize;

use crate::cli::BootstrapArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Bootstrap the workspace: run `flutter pub get` (or `dart pub get`) in each package
pub async fn run(workspace: &Workspace, args: BootstrapArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters(&workspace.packages, &filters, Some(&workspace.root_path))?;

    println!(
        "\n{} Bootstrapping {} packages...\n",
        "$".cyan(),
        packages.len()
    );

    if packages.is_empty() {
        println!("{}", "No packages found in workspace.".yellow());
        return Ok(());
    }

    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    let runner = ProcessRunner::new(1, true);

    // Run `flutter pub get` for Flutter packages, `dart pub get` for pure Dart packages
    let flutter_packages: Vec<_> = packages.iter().filter(|p| p.is_flutter).cloned().collect();

    let dart_packages: Vec<_> = packages.iter().filter(|p| !p.is_flutter).cloned().collect();

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
