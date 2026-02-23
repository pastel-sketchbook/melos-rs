use anyhow::Result;
use colored::Colorize;

use crate::cli::BootstrapArgs;
use crate::filter_ext::package_filters_from_args;
use melos_core::commands::bootstrap::{
    build_pub_get_command, config_dependency_override_paths, config_enforce_lockfile,
    config_enforce_versions, config_run_pub_get_offline, effective_concurrency,
    generate_pubspec_overrides, sync_shared_dependencies,
};
use melos_core::package::filter::{apply_filters_with_categories, topological_sort};
use melos_core::runner::ProcessRunner;
use melos_core::workspace::Workspace;

/// Bootstrap the workspace: link local packages and run `pub get` in each package
pub async fn run(workspace: &Workspace, args: BootstrapArgs) -> Result<()> {
    let filters = package_filters_from_args(&args.filters);
    let filtered = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    // Topological sort ensures dependencies are bootstrapped before dependents
    let packages = topological_sort(&filtered);

    let concurrency = effective_concurrency(workspace, args.concurrency);

    // Merge CLI flags with config flags
    let enforce_lockfile = if args.no_enforce_lockfile {
        false
    } else {
        args.enforce_lockfile || config_enforce_lockfile(workspace)
    };

    let offline = args.offline || config_run_pub_get_offline(workspace);

    println!(
        "\n{} Bootstrapping {} packages (concurrency: {}, dependency order)...\n",
        "$".cyan(),
        packages.len(),
        concurrency
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

    if args.dry_run {
        println!(
            "{}",
            "DRY RUN — no packages were bootstrapped.".yellow().bold()
        );
        return Ok(());
    }

    if let Some(pre_hook) = workspace.hook("bootstrap", "pre") {
        crate::runner::run_lifecycle_hook(pre_hook, "pre-bootstrap", &workspace.root_path, &[])
            .await?;
    }

    // In 6.x mode, generate pubspec_overrides.yaml for local package linking.
    if workspace.config_source.is_legacy() {
        let all_workspace_resolution = packages.iter().all(|p| p.uses_workspace_resolution());

        if all_workspace_resolution && !packages.is_empty() {
            println!(
                "  {} All packages use workspace resolution — skipping pubspec_overrides.yaml generation\n",
                "i".blue()
            );
        } else {
            let override_paths = config_dependency_override_paths(workspace);
            let result = generate_pubspec_overrides(
                &packages,
                &workspace.packages,
                &override_paths,
                &workspace.root_path,
            )?;

            for warning in &result.warnings {
                eprintln!("  {} {}", "WARN".yellow(), warning);
            }

            if result.extra_package_count > 0 {
                println!(
                    "  {} Found {} extra package(s) from dependencyOverridePaths",
                    "i".blue(),
                    result.extra_package_count
                );
            }

            for pkg in &packages {
                if pkg.uses_workspace_resolution() {
                    continue;
                }
                let local_dep_count = pkg
                    .dependencies
                    .iter()
                    .chain(pkg.dev_dependencies.iter())
                    .filter(|dep| workspace.packages.iter().any(|p| &p.name == *dep))
                    .count();
                if local_dep_count > 0 {
                    println!(
                        "  {} Generated pubspec_overrides.yaml for {} ({} local dep{})",
                        "LINK".cyan(),
                        pkg.name,
                        local_dep_count,
                        if local_dep_count == 1 { "" } else { "s" }
                    );
                }
            }

            if result.generated > 0 {
                println!(
                    "\n  {} Linked {} package{} via pubspec_overrides.yaml\n",
                    "OK".green(),
                    result.generated,
                    if result.generated == 1 { "" } else { "s" }
                );
            }
        }
    }

    // Validate version constraints if configured
    if config_enforce_versions(workspace) {
        let violations =
            melos_core::commands::bootstrap::enforce_versions(&packages, &workspace.packages)?;
        if violations.is_empty() {
            println!(
                "  {} All workspace dependency version constraints satisfied.\n",
                "OK".green()
            );
        } else {
            let msg = format!(
                "Version constraint violations found ({} issue{}):\n{}\n\n\
                 Update the version constraints in pubspec.yaml to match the workspace packages' actual versions.",
                violations.len(),
                if violations.len() == 1 { "" } else { "s" },
                violations.join("\n")
            );
            anyhow::bail!(msg);
        }
    }

    // Sync shared dependencies if configured
    let synced = sync_shared_dependencies(&packages, workspace)?;
    if synced > 0 {
        println!(
            "  {} Synced shared dependencies in {} package{}",
            "OK".green(),
            synced,
            if synced == 1 { "" } else { "s" }
        );
    }

    let flutter_cmd = build_pub_get_command("flutter", enforce_lockfile, args.no_example, offline);
    let dart_cmd = build_pub_get_command("dart", enforce_lockfile, args.no_example, offline);

    let flutter_packages: Vec<_> = packages.iter().filter(|p| p.is_flutter).cloned().collect();
    let dart_packages: Vec<_> = packages.iter().filter(|p| !p.is_flutter).cloned().collect();

    let total = flutter_packages.len() + dart_packages.len();
    let (tx, render_handle) = crate::render::spawn_renderer(total, "bootstrapping");

    let mut bail_msg: Option<String> = None;

    if !flutter_packages.is_empty() {
        let _ = tx.send(melos_core::events::Event::Progress {
            completed: 0,
            total: 0,
            message: "flutter pub get...".into(),
        });
        let runner = ProcessRunner::new(concurrency, true);
        let results = runner
            .run_in_packages_with_events(
                &flutter_packages,
                &flutter_cmd,
                &workspace.env_vars(),
                None,
                Some(&tx),
                &workspace.packages,
            )
            .await?;

        for (name, success) in &results {
            if !success {
                bail_msg = Some(format!("flutter pub get failed in package '{}'", name));
                break;
            }
        }
    }

    if bail_msg.is_none() && !dart_packages.is_empty() {
        let _ = tx.send(melos_core::events::Event::Progress {
            completed: 0,
            total: 0,
            message: "dart pub get...".into(),
        });
        let runner = ProcessRunner::new(concurrency, true);
        let results = runner
            .run_in_packages_with_events(
                &dart_packages,
                &dart_cmd,
                &workspace.env_vars(),
                None,
                Some(&tx),
                &workspace.packages,
            )
            .await?;

        for (name, success) in &results {
            if !success {
                bail_msg = Some(format!("dart pub get failed in package '{}'", name));
                break;
            }
        }
    }

    drop(tx);
    render_handle.await??;

    if let Some(msg) = bail_msg {
        anyhow::bail!(msg);
    }

    if let Some(post_hook) = workspace.hook("bootstrap", "post") {
        crate::runner::run_lifecycle_hook(post_hook, "post-bootstrap", &workspace.root_path, &[])
            .await?;
    }

    println!(
        "\n{}",
        format!("All {} package(s) bootstrapped.", packages.len()).green()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    // All pure logic tests have moved to melos_core::commands::bootstrap.
    // CLI tests remain here if they test BootstrapArgs clap parsing.
    // Currently no CLI-specific tests remain.
}
