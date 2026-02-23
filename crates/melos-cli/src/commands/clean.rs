use anyhow::Result;
use colored::Colorize;

use crate::cli::CleanArgs;
use crate::filter_ext::package_filters_from_args;
use crate::runner::{ProcessRunner, create_progress_bar};
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::workspace::Workspace;

/// Paths removed during a deep clean
const DEEP_CLEAN_DIRS: &[&str] = &[".dart_tool", "build"];
const DEEP_CLEAN_FILES: &[&str] = &["pubspec.lock"];

/// Clean all packages
pub async fn run(workspace: &Workspace, args: CleanArgs) -> Result<()> {
    let filters = package_filters_from_args(&args.filters);
    let all_filtered = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    println!(
        "\n{} Cleaning {} packages...\n",
        "$".cyan(),
        all_filtered.len()
    );

    if all_filtered.is_empty() {
        println!("{}", "No packages found in workspace.".yellow());
        return Ok(());
    }

    // Dry-run mode: show what would be cleaned without running
    if args.dry_run {
        for pkg in &all_filtered {
            let pkg_type = if pkg.is_flutter { "flutter" } else { "dart" };
            println!("  {} {} ({})", "->".cyan(), pkg.name, pkg_type.dimmed());
        }
        if args.deep {
            println!(
                "\n  {} Deep clean would also remove: {}, {}",
                "i".blue(),
                DEEP_CLEAN_DIRS.join(", "),
                DEEP_CLEAN_FILES.join(", ")
            );
        }
        println!(
            "\n{}",
            "DRY RUN — no packages were cleaned.".yellow().bold()
        );
        return Ok(());
    }

    if let Some(pre_hook) = workspace.hook("clean", "pre") {
        crate::runner::run_lifecycle_hook(pre_hook, "pre-clean", &workspace.root_path, &[]).await?;
    }

    // In 6.x mode, remove generated pubspec_overrides.yaml files
    if workspace.config_source.is_legacy() {
        remove_pubspec_overrides(&all_filtered);
    }

    // Flutter packages need `flutter clean`; pure Dart packages just need artifacts removed
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
    let pb = create_progress_bar(all_filtered.len() as u64, "cleaning");

    if !flutter_packages.is_empty() {
        pb.set_message("flutter clean...");
        let runner = ProcessRunner::new(1, false);
        let results = runner
            .run_in_packages_with_progress(
                &flutter_packages,
                "flutter clean",
                &workspace.env_vars(),
                None,
                Some(&pb),
                &workspace.packages,
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
        pb.set_message("cleaning dart packages...");
        println!("{}", "Cleaning pure Dart packages...".dimmed());
        for pkg in &dart_packages {
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
            pb.inc(1);
        }
    }

    pb.finish_and_clear();

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
                        println!("  {} Removed {}/{}", "OK".green(), pkg.name, dir_name);
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
                        println!("  {} Removed {}/{}", "OK".green(), pkg.name, file_name);
                    }
                }
            }
        }
    }

    let total = all_filtered.len();
    if failed > 0 {
        let passed = total - failed as usize;
        anyhow::bail!("{} package(s) failed cleaning ({} passed)", failed, passed);
    }

    println!(
        "\n{}",
        format!("All {} package(s) passed cleaning.", total).green()
    );

    if let Some(post_hook) = workspace.hook("clean", "post") {
        crate::runner::run_lifecycle_hook(post_hook, "post-clean", &workspace.root_path, &[])
            .await?;
    }

    Ok(())
}

/// Remove `pubspec_overrides.yaml` files from packages (Melos 6.x mode).
///
/// These files are generated by `melos bootstrap` and should be cleaned up.
fn remove_pubspec_overrides(packages: &[melos_core::package::Package]) {
    let mut removed = 0u32;

    for pkg in packages {
        let override_path = pkg.path.join("pubspec_overrides.yaml");
        if override_path.exists() {
            match std::fs::remove_file(&override_path) {
                Ok(()) => {
                    println!(
                        "  {} Removed pubspec_overrides.yaml from {}",
                        "OK".green(),
                        pkg.name
                    );
                    removed += 1;
                }
                Err(e) => {
                    eprintln!(
                        "  {} Failed to remove pubspec_overrides.yaml from {}: {}",
                        "WARN".yellow(),
                        pkg.name,
                        e
                    );
                }
            }
        }
    }

    if removed > 0 {
        println!(
            "  {} Removed {} pubspec_overrides.yaml file{}\n",
            "OK".green(),
            removed,
            if removed == 1 { "" } else { "s" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use melos_core::package::Package;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_package(name: &str, path: PathBuf) -> Package {
        Package {
            name: name.to_string(),
            path,
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        }
    }

    #[test]
    fn test_deep_clean_dirs_constant() {
        assert!(DEEP_CLEAN_DIRS.contains(&".dart_tool"));
        assert!(DEEP_CLEAN_DIRS.contains(&"build"));
        assert_eq!(DEEP_CLEAN_DIRS.len(), 2);
    }

    #[test]
    fn test_deep_clean_files_constant() {
        assert!(DEEP_CLEAN_FILES.contains(&"pubspec.lock"));
        assert_eq!(DEEP_CLEAN_FILES.len(), 1);
    }

    #[test]
    fn test_remove_pubspec_overrides_removes_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        // Create the override file
        let override_path = pkg_dir.join("pubspec_overrides.yaml");
        std::fs::write(
            &override_path,
            "dependency_overrides:\n  core:\n    path: ../core\n",
        )
        .unwrap();
        assert!(override_path.exists());

        let pkg = make_package("app", pkg_dir.clone());
        remove_pubspec_overrides(&[pkg]);

        assert!(
            !override_path.exists(),
            "pubspec_overrides.yaml should be removed"
        );
    }

    #[test]
    fn test_remove_pubspec_overrides_no_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let override_path = pkg_dir.join("pubspec_overrides.yaml");
        assert!(!override_path.exists());

        let pkg = make_package("app", pkg_dir.clone());
        // Should not panic when file doesn't exist
        remove_pubspec_overrides(&[pkg]);
    }

    #[test]
    fn test_remove_pubspec_overrides_multiple_packages() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_a_dir = dir.path().join("packages").join("a");
        let pkg_b_dir = dir.path().join("packages").join("b");
        std::fs::create_dir_all(&pkg_a_dir).unwrap();
        std::fs::create_dir_all(&pkg_b_dir).unwrap();

        // Only pkg_a has an override file
        let override_a = pkg_a_dir.join("pubspec_overrides.yaml");
        std::fs::write(&override_a, "# overrides").unwrap();

        let pkg_a = make_package("a", pkg_a_dir);
        let pkg_b = make_package("b", pkg_b_dir.clone());

        remove_pubspec_overrides(&[pkg_a, pkg_b]);

        assert!(!override_a.exists(), "a's override should be removed");
        assert!(
            !pkg_b_dir.join("pubspec_overrides.yaml").exists(),
            "b never had one"
        );
    }

    #[test]
    fn test_remove_pubspec_overrides_empty_packages() {
        // Should not panic with empty list
        remove_pubspec_overrides(&[]);
    }
}
