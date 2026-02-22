use anyhow::Result;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};

use crate::cli::BootstrapArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters_with_categories;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Bootstrap the workspace: run `flutter pub get` / `dart pub get` in each package
pub async fn run(workspace: &Workspace, args: BootstrapArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters_with_categories(&workspace.packages, &filters, Some(&workspace.root_path), &workspace.config.categories)?;

    println!(
        "\n{} Bootstrapping {} packages (concurrency: {})...\n",
        "$".cyan(),
        packages.len(),
        args.concurrency
    );

    if packages.is_empty() {
        println!("{}", "No packages found in workspace.".yellow());
        return Ok(());
    }

    for pkg in &packages {
        let pkg_type = if pkg.is_flutter { "flutter" } else { "dart" };
        println!("  {} {} ({})", "->".cyan(), pkg.name, pkg_type.dimmed());
    }
    println!();

    let flutter_packages: Vec<_> = packages.iter().filter(|p| p.is_flutter).cloned().collect();
    let dart_packages: Vec<_> = packages.iter().filter(|p| !p.is_flutter).cloned().collect();

    let total = flutter_packages.len() + dart_packages.len();
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("=> "),
    );

    // Bootstrap Flutter packages in parallel
    if !flutter_packages.is_empty() {
        pb.set_message("flutter pub get...");
        let runner = ProcessRunner::new(args.concurrency, true);
        let results = runner
            .run_in_packages(
                &flutter_packages,
                "flutter pub get",
                &workspace.env_vars(),
            )
            .await?;

        for (name, success) in &results {
            pb.inc(1);
            if !success {
                pb.finish_and_clear();
                anyhow::bail!("flutter pub get failed in package '{}'", name);
            }
        }
    }

    // Bootstrap Dart packages in parallel
    if !dart_packages.is_empty() {
        pb.set_message("dart pub get...");
        let runner = ProcessRunner::new(args.concurrency, true);
        let results = runner
            .run_in_packages(&dart_packages, "dart pub get", &workspace.env_vars())
            .await?;

        for (name, success) in &results {
            pb.inc(1);
            if !success {
                pb.finish_and_clear();
                anyhow::bail!("dart pub get failed in package '{}'", name);
            }
        }
    }

    pb.finish_and_clear();
    println!("\n{}", "All packages bootstrapped.".green());
    Ok(())
}
