use anyhow::Result;
use colored::Colorize;

use crate::cli::CleanArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Paths removed during a deep clean
const DEEP_CLEAN_DIRS: &[&str] = &[".dart_tool", "build"];
const DEEP_CLEAN_FILES: &[&str] = &["pubspec.lock"];

/// Clean all packages
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

    // Run `flutter clean` in Flutter packages
    let flutter_packages: Vec<_> = all_filtered
        .iter()
        .filter(|p| p.is_flutter)
        .cloned()
        .collect();

    let dart_packages: Vec<_> = all_filtered
        .iter()
        .filter(|p| !p.is_flutter)
        .cloned()
        .collect();

    let mut failed = 0u32;

    if !flutter_packages.is_empty() {
        println!("{}", "Running flutter clean...".dimmed());
        let runner = ProcessRunner::new(1, false);
        let results = runner
            .run_in_packages(
                &flutter_packages,
                "flutter clean",
                &workspace.env_vars(),
            )
            .await?;

        for (name, success) in &results {
            if *success {
                println!("  {} {}", "CLEANED".green(), name);
            } else {
                println!("  {} {}", "FAILED".red(), name);
                failed += 1;
            }
        }
    }

    // For pure Dart packages, remove build artifacts
    if !dart_packages.is_empty() {
        println!("{}", "Cleaning pure Dart packages...".dimmed());
        for pkg in &dart_packages {
            // Remove build/ directory if present
            let build_dir = pkg.path.join("build");
            if build_dir.exists()
                && let Err(e) = std::fs::remove_dir_all(&build_dir)
            {
                eprintln!(
                    "  {} Failed to remove {}: {}",
                    "WARN".yellow(),
                    build_dir.display(),
                    e
                );
                failed += 1;
            }

            // Remove .dart_tool/ directory if present
            let dart_tool_dir = pkg.path.join(".dart_tool");
            if dart_tool_dir.exists()
                && let Err(e) = std::fs::remove_dir_all(&dart_tool_dir)
            {
                eprintln!(
                    "  {} Failed to remove {}: {}",
                    "WARN".yellow(),
                    dart_tool_dir.display(),
                    e
                );
                failed += 1;
            }

            println!("  {} {}", "CLEANED".green(), pkg.name);
        }
    }

    // Deep clean: remove additional artifacts from ALL packages
    if args.deep {
        println!("\n{}", "Deep cleaning...".dimmed());
        for pkg in &all_filtered {
            for dir_name in DEEP_CLEAN_DIRS {
                let dir_path = pkg.path.join(dir_name);
                if dir_path.exists() {
                    if let Err(e) = std::fs::remove_dir_all(&dir_path) {
                        eprintln!(
                            "  {} Failed to remove {}: {}",
                            "WARN".yellow(),
                            dir_path.display(),
                            e
                        );
                    } else {
                        println!(
                            "  {} Removed {}/{}",
                            "OK".green(),
                            pkg.name,
                            dir_name
                        );
                    }
                }
            }

            for file_name in DEEP_CLEAN_FILES {
                let file_path = pkg.path.join(file_name);
                if file_path.exists() {
                    if let Err(e) = std::fs::remove_file(&file_path) {
                        eprintln!(
                            "  {} Failed to remove {}: {}",
                            "WARN".yellow(),
                            file_path.display(),
                            e
                        );
                    } else {
                        println!(
                            "  {} Removed {}/{}",
                            "OK".green(),
                            pkg.name,
                            file_name
                        );
                    }
                }
            }
        }
    }

    if failed > 0 {
        anyhow::bail!("{} package(s) failed to clean", failed);
    }

    println!("\n{}", "All packages cleaned.".green());
    Ok(())
}
