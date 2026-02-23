use anyhow::Result;
use colored::Colorize;

use crate::cli::CleanArgs;
use crate::filter_ext::package_filters_from_args;
use melos_core::commands::clean::{DEEP_CLEAN_DIRS, DEEP_CLEAN_FILES, OverrideRemoval};
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::runner::ProcessRunner;
use melos_core::workspace::Workspace;

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
        let results = melos_core::commands::clean::remove_pubspec_overrides(&all_filtered);
        let mut removed = 0u32;
        for (name, result) in &results {
            match result {
                OverrideRemoval::Removed => {
                    println!(
                        "  {} Removed pubspec_overrides.yaml from {}",
                        "OK".green(),
                        name
                    );
                    removed += 1;
                }
                OverrideRemoval::Failed(e) => {
                    eprintln!(
                        "  {} Failed to remove pubspec_overrides.yaml from {}: {}",
                        "WARN".yellow(),
                        name,
                        e
                    );
                }
                OverrideRemoval::NotPresent => {}
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

    if !flutter_packages.is_empty() {
        let (tx, render_handle) =
            crate::render::spawn_renderer(flutter_packages.len(), "flutter clean...");
        let runner = ProcessRunner::new(1, false);
        let results = runner
            .run_in_packages_with_events(
                &flutter_packages,
                "flutter clean",
                &workspace.env_vars(),
                None,
                Some(&tx),
                &workspace.packages,
            )
            .await?;
        drop(tx);
        render_handle.await??;

        for (name, success) in &results {
            if *success {
                println!("  {} {}", "CLEANED".green(), name);
            } else {
                println!("  {} {}", "FAILED".red(), name);
                failed += 1;
            }
        }
    }

    if !dart_packages.is_empty() {
        let pb = crate::render::create_progress_bar(
            dart_packages.len() as u64,
            "cleaning dart packages...",
        );
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
        pb.finish_and_clear();
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
