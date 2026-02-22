use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};

use crate::cli::BootstrapArgs;
use crate::config::filter::PackageFilters;
use crate::package::Package;
use crate::package::filter::{apply_filters_with_categories, topological_sort};
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Bootstrap the workspace: link local packages and run `pub get` in each package
pub async fn run(workspace: &Workspace, args: BootstrapArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let filtered = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    // Topological sort ensures dependencies are bootstrapped before dependents
    let packages = topological_sort(&filtered);

    // Determine effective concurrency: config `run_pub_get_in_parallel: false` forces 1,
    // otherwise CLI `-c N` (default 5) applies.
    let concurrency = effective_concurrency(workspace, args.concurrency);

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

    // In 6.x mode, generate pubspec_overrides.yaml for local package linking
    if workspace.config_source.is_legacy() {
        generate_pubspec_overrides(&packages, &workspace.packages)?;
    }

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
        let runner = ProcessRunner::new(concurrency, true);
        let results = runner
            .run_in_packages(
                &flutter_packages,
                "flutter pub get",
                &workspace.env_vars(),
                None,
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
        let runner = ProcessRunner::new(concurrency, true);
        let results = runner
            .run_in_packages(&dart_packages, "dart pub get", &workspace.env_vars(), None)
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

/// Determine effective concurrency for bootstrap.
///
/// If the config has `command.bootstrap.run_pub_get_in_parallel: false`,
/// concurrency is forced to 1. Otherwise, the CLI `-c N` value is used.
fn effective_concurrency(workspace: &Workspace, cli_concurrency: usize) -> usize {
    let parallel = workspace
        .config
        .command
        .as_ref()
        .and_then(|c| c.bootstrap.as_ref())
        .and_then(|b| b.run_pub_get_in_parallel);

    match parallel {
        Some(false) => 1,
        _ => cli_concurrency,
    }
}

/// Generate `pubspec_overrides.yaml` files for local package linking (Melos 6.x mode).
///
/// For each package that depends on other workspace packages, we create a
/// `pubspec_overrides.yaml` with `dependency_overrides:` entries pointing to
/// the sibling package via a relative path.
///
/// This allows `pub get` to resolve workspace packages locally without
/// modifying the actual `pubspec.yaml`.
fn generate_pubspec_overrides(packages: &[Package], all_workspace_packages: &[Package]) -> Result<()> {
    let workspace_names: HashSet<&str> = all_workspace_packages
        .iter()
        .map(|p| p.name.as_str())
        .collect();

    let mut generated = 0u32;

    for pkg in packages {
        // Find all dependencies (regular + dev) that are workspace packages
        let local_deps: Vec<&Package> = pkg
            .dependencies
            .iter()
            .chain(pkg.dev_dependencies.iter())
            .filter(|dep| workspace_names.contains(dep.as_str()))
            .filter_map(|dep| all_workspace_packages.iter().find(|p| p.name == *dep))
            .collect();

        let override_path = pkg.path.join("pubspec_overrides.yaml");

        if local_deps.is_empty() {
            // Remove stale override file if no local deps
            if override_path.exists() {
                std::fs::remove_file(&override_path).with_context(|| {
                    format!(
                        "Failed to remove stale pubspec_overrides.yaml in {}",
                        pkg.name
                    )
                })?;
            }
            continue;
        }

        let content = build_pubspec_overrides_content(&local_deps, &pkg.path)?;
        std::fs::write(&override_path, &content).with_context(|| {
            format!(
                "Failed to write pubspec_overrides.yaml for package '{}'",
                pkg.name
            )
        })?;

        generated += 1;
        println!(
            "  {} Generated pubspec_overrides.yaml for {} ({} local dep{})",
            "LINK".cyan(),
            pkg.name,
            local_deps.len(),
            if local_deps.len() == 1 { "" } else { "s" }
        );
    }

    if generated > 0 {
        println!(
            "\n  {} Linked {} package{} via pubspec_overrides.yaml\n",
            "OK".green(),
            generated,
            if generated == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

/// Build the YAML content for a `pubspec_overrides.yaml` file.
///
/// Output format:
/// ```yaml
/// # Generated by melos-rs. Do not edit.
/// dependency_overrides:
///   core:
///     path: ../core
///   utils:
///     path: ../../shared/utils
/// ```
fn build_pubspec_overrides_content(local_deps: &[&Package], pkg_path: &Path) -> Result<String> {
    let mut content = String::from("# Generated by melos-rs. Do not edit.\ndependency_overrides:\n");

    // Sort deps by name for deterministic output
    let mut sorted_deps: Vec<&&Package> = local_deps.iter().collect();
    sorted_deps.sort_by_key(|p| &p.name);

    for dep in sorted_deps {
        let relative = pathdiff::diff_paths(&dep.path, pkg_path).unwrap_or_else(|| dep.path.clone());
        let relative_str = relative.display().to_string();

        content.push_str(&format!("  {}:\n", dep.name));
        content.push_str(&format!("    path: {}\n", relative_str));
    }

    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::collections::HashMap;

    use crate::config::{
        BootstrapCommandConfig, CommandConfig, MelosConfig,
    };
    use crate::workspace::{ConfigSource, Workspace};

    fn make_package(name: &str, path: &str, deps: Vec<&str>) -> Package {
        Package {
            name: name.to_string(),
            path: PathBuf::from(path),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: deps.into_iter().map(String::from).collect(),
            dev_dependencies: vec![],
        }
    }

    fn make_workspace(bootstrap_config: Option<BootstrapCommandConfig>) -> Workspace {
        Workspace {
            root_path: PathBuf::from("/workspace"),
            config_source: ConfigSource::MelosYaml(PathBuf::from("/workspace/melos.yaml")),
            config: MelosConfig {
                name: "test".to_string(),
                packages: vec!["packages/**".to_string()],
                command: Some(CommandConfig {
                    version: None,
                    bootstrap: bootstrap_config,
                    clean: None,
                }),
                scripts: HashMap::new(),
                categories: HashMap::new(),
            },
            packages: vec![],
        }
    }

    #[test]
    fn test_build_pubspec_overrides_content() {
        let core = make_package("core", "/workspace/packages/core", vec![]);
        let utils = make_package("utils", "/workspace/packages/utils", vec![]);
        let app_path = PathBuf::from("/workspace/packages/app");

        let deps: Vec<&Package> = vec![&core, &utils];
        let content = build_pubspec_overrides_content(&deps, &app_path).unwrap();

        assert!(content.contains("# Generated by melos-rs"));
        assert!(content.contains("dependency_overrides:"));
        assert!(content.contains("  core:"));
        assert!(content.contains("    path: ../core"));
        assert!(content.contains("  utils:"));
        assert!(content.contains("    path: ../utils"));
    }

    #[test]
    fn test_build_pubspec_overrides_sorted() {
        let zebra = make_package("zebra", "/workspace/packages/zebra", vec![]);
        let alpha = make_package("alpha", "/workspace/packages/alpha", vec![]);
        let app_path = PathBuf::from("/workspace/packages/app");

        let deps: Vec<&Package> = vec![&zebra, &alpha];
        let content = build_pubspec_overrides_content(&deps, &app_path).unwrap();

        // alpha should come before zebra (sorted)
        let alpha_pos = content.find("alpha:").unwrap();
        let zebra_pos = content.find("zebra:").unwrap();
        assert!(alpha_pos < zebra_pos, "Dependencies should be sorted by name");
    }

    #[test]
    fn test_effective_concurrency_default() {
        let ws = make_workspace(None);
        assert_eq!(effective_concurrency(&ws, 5), 5);
    }

    #[test]
    fn test_effective_concurrency_parallel_false_forces_one() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: Some(false),
            enforce_versions_for_dependency_resolution: None,
        }));
        assert_eq!(effective_concurrency(&ws, 5), 1);
    }

    #[test]
    fn test_effective_concurrency_parallel_true_uses_cli() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: Some(true),
            enforce_versions_for_dependency_resolution: None,
        }));
        assert_eq!(effective_concurrency(&ws, 8), 8);
    }

    #[test]
    fn test_effective_concurrency_parallel_none_uses_cli() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: None,
            enforce_versions_for_dependency_resolution: None,
        }));
        assert_eq!(effective_concurrency(&ws, 3), 3);
    }
}
