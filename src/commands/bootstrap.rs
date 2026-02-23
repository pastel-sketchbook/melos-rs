use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;

use crate::cli::BootstrapArgs;
use crate::config::filter::PackageFilters;
use crate::package::Package;
use crate::package::filter::{apply_filters_with_categories, topological_sort};
use crate::runner::{ProcessRunner, create_progress_bar};
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

    // Merge CLI flags with config flags to determine extra pub get arguments.
    // --no-enforce-lockfile overrides both --enforce-lockfile and config.
    let enforce_lockfile = if args.no_enforce_lockfile {
        false
    } else {
        args.enforce_lockfile || config_enforce_lockfile(workspace)
    };

    // CLI --offline overrides config runPubGetOffline
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

    if let Some(pre_hook) = workspace.hook("bootstrap", "pre") {
        crate::runner::run_lifecycle_hook(pre_hook, "pre-bootstrap", &workspace.root_path, &[])
            .await?;
    }

    // In 6.x mode, generate pubspec_overrides.yaml for local package linking.
    // Skip entirely if ALL packages use workspace resolution (Dart 3.5+ workspaces
    // handle linking natively and reject pubspec_overrides.yaml).
    if workspace.config_source.is_legacy() {
        let all_workspace_resolution = packages.iter().all(|p| p.uses_workspace_resolution());

        if all_workspace_resolution && !packages.is_empty() {
            println!(
                "  {} All packages use workspace resolution — skipping pubspec_overrides.yaml generation\n",
                "i".blue()
            );
        } else {
            let override_paths = config_dependency_override_paths(workspace);
            generate_pubspec_overrides(
                &packages,
                &workspace.packages,
                &override_paths,
                &workspace.root_path,
            )?;
        }
    }

    // Validate version constraints if configured
    if config_enforce_versions(workspace) {
        enforce_versions(&packages, &workspace.packages)?;
    }

    // Sync shared dependencies if configured
    sync_shared_dependencies(&packages, workspace)?;

    let flutter_cmd = build_pub_get_command("flutter", enforce_lockfile, args.no_example, offline);
    let dart_cmd = build_pub_get_command("dart", enforce_lockfile, args.no_example, offline);

    let flutter_packages: Vec<_> = packages.iter().filter(|p| p.is_flutter).cloned().collect();
    let dart_packages: Vec<_> = packages.iter().filter(|p| !p.is_flutter).cloned().collect();

    let total = flutter_packages.len() + dart_packages.len();
    let pb = create_progress_bar(total as u64, "bootstrapping");

    // Bootstrap Flutter packages in parallel
    if !flutter_packages.is_empty() {
        pb.set_message("flutter pub get...");
        let runner = ProcessRunner::new(concurrency, true);
        let results = runner
            .run_in_packages(
                &flutter_packages,
                &flutter_cmd,
                &workspace.env_vars(),
                None,
                &workspace.packages,
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
            .run_in_packages(
                &dart_packages,
                &dart_cmd,
                &workspace.env_vars(),
                None,
                &workspace.packages,
            )
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

    if let Some(post_hook) = workspace.hook("bootstrap", "post") {
        crate::runner::run_lifecycle_hook(post_hook, "post-bootstrap", &workspace.root_path, &[])
            .await?;
    }

    println!("\n{}", "All packages bootstrapped.".green());
    Ok(())
}

/// Extract the bootstrap command config from the workspace, if present.
fn bootstrap_config(workspace: &Workspace) -> Option<&crate::config::BootstrapCommandConfig> {
    workspace
        .config
        .command
        .as_ref()
        .and_then(|c| c.bootstrap.as_ref())
}

/// Determine effective concurrency for bootstrap.
///
/// If the config has `command.bootstrap.run_pub_get_in_parallel: false`,
/// concurrency is forced to 1. Otherwise, the CLI `-c N` value is used.
fn effective_concurrency(workspace: &Workspace, cli_concurrency: usize) -> usize {
    match bootstrap_config(workspace).and_then(|b| b.run_pub_get_in_parallel) {
        Some(false) => 1,
        _ => cli_concurrency,
    }
}

/// Check if `enforce_lockfile` is set in bootstrap config.
fn config_enforce_lockfile(workspace: &Workspace) -> bool {
    bootstrap_config(workspace)
        .and_then(|b| b.enforce_lockfile)
        .unwrap_or(false)
}

/// Check if `run_pub_get_offline` is set in bootstrap config.
fn config_run_pub_get_offline(workspace: &Workspace) -> bool {
    bootstrap_config(workspace)
        .and_then(|b| b.run_pub_get_offline)
        .unwrap_or(false)
}

/// Get `dependencyOverridePaths` from bootstrap config, if set.
fn config_dependency_override_paths(workspace: &Workspace) -> Vec<String> {
    bootstrap_config(workspace)
        .and_then(|b| b.dependency_override_paths.clone())
        .unwrap_or_default()
}

/// Check if `enforce_versions_for_dependency_resolution` is set in bootstrap config.
fn config_enforce_versions(workspace: &Workspace) -> bool {
    bootstrap_config(workspace)
        .and_then(|b| b.enforce_versions_for_dependency_resolution)
        .unwrap_or(false)
}

/// Sync shared dependency versions from bootstrap config into each package's pubspec.yaml.
///
/// If the bootstrap config defines `environment`, `dependencies`, or `dev_dependencies`,
/// this function reads each package's pubspec.yaml and updates matching entries to use
/// the version constraints from the shared config. The pubspec.yaml is rewritten only
/// if changes were made.
///
/// This mirrors Melos's `command.bootstrap.dependencies` / `dev_dependencies` /
/// `environment` feature that keeps dependency versions consistent across all packages.
fn sync_shared_dependencies(packages: &[Package], workspace: &Workspace) -> Result<()> {
    let bc = bootstrap_config(workspace);

    let shared_env = bc.and_then(|b| b.environment.as_ref());
    let shared_deps = bc.and_then(|b| b.dependencies.as_ref());
    let shared_dev_deps = bc.and_then(|b| b.dev_dependencies.as_ref());

    // Nothing to sync?
    if shared_env.is_none() && shared_deps.is_none() && shared_dev_deps.is_none() {
        return Ok(());
    }

    let mut synced_count = 0u32;

    for pkg in packages {
        let pubspec_path = pkg.path.join("pubspec.yaml");
        let content = std::fs::read_to_string(&pubspec_path)
            .with_context(|| format!("Failed to read pubspec.yaml for '{}'", pkg.name))?;

        let mut changed = false;
        let mut lines: Vec<String> = content.lines().map(String::from).collect();

        // Sync environment SDK constraints
        if let Some(env) = shared_env {
            changed |= sync_yaml_section(&mut lines, "environment", env);
        }

        // Sync dependencies
        if let Some(deps) = shared_deps {
            let string_deps: HashMap<String, String> = deps
                .iter()
                .filter_map(|(k, v)| yaml_value_to_constraint(v).map(|c| (k.clone(), c)))
                .collect();
            changed |= sync_yaml_section(&mut lines, "dependencies", &string_deps);
        }

        // Sync dev_dependencies
        if let Some(dev_deps) = shared_dev_deps {
            let string_deps: HashMap<String, String> = dev_deps
                .iter()
                .filter_map(|(k, v)| yaml_value_to_constraint(v).map(|c| (k.clone(), c)))
                .collect();
            changed |= sync_yaml_section(&mut lines, "dev_dependencies", &string_deps);
        }

        if changed {
            let new_content = lines.join("\n") + "\n";
            std::fs::write(&pubspec_path, new_content).with_context(|| {
                format!("Failed to write updated pubspec.yaml for '{}'", pkg.name)
            })?;
            synced_count += 1;
        }
    }

    if synced_count > 0 {
        println!(
            "  {} Synced shared dependencies in {} package{}",
            "OK".green(),
            synced_count,
            if synced_count == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

/// Convert a YAML value (from shared deps config) to a version constraint string.
///
/// Supports:
/// - String: `"^1.0.0"` -> `Some("^1.0.0")`
/// - Null: -> `Some("any")` (null means accept any version)
/// - Other (mapping, etc.): -> `None` (complex deps like git can't be synced as simple strings)
fn yaml_value_to_constraint(value: &yaml_serde::Value) -> Option<String> {
    match value {
        yaml_serde::Value::String(s) => Some(s.clone()),
        yaml_serde::Value::Null => Some("any".to_string()),
        _ => None, // Complex deps (git, path, etc.) are not synced
    }
}

/// Sync version constraints for entries in a YAML section (e.g., `dependencies:`)
/// by doing line-level text replacement.
///
/// This approach avoids a full YAML parse-modify-serialize cycle which would lose
/// comments and formatting. It finds the section header (e.g., `dependencies:`),
/// then looks for lines matching `  <key>: <old_value>` and replaces them with
/// `  <key>: <new_value>`.
///
/// Returns `true` if any lines were modified.
fn sync_yaml_section(
    lines: &mut [String],
    section: &str,
    values: &HashMap<String, String>,
) -> bool {
    if values.is_empty() {
        return false;
    }

    let section_header = format!("{}:", section);
    let mut in_section = false;
    let mut changed = false;

    for line in lines.iter_mut() {
        let trimmed = line.trim();

        // Check if we're entering the target section
        if trimmed == section_header {
            in_section = true;
            continue;
        }

        // If we hit another top-level key (no leading whitespace), exit the section
        if in_section && !trimmed.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
            in_section = false;
        }

        if !in_section {
            continue;
        }

        // Check if this line is a simple `  key: value` entry
        // (2-space indent, which is standard YAML for pubspec.yaml)
        for (key, new_value) in values {
            if trimmed.starts_with(&format!("{}:", key)) {
                // Determine current indentation
                let indent = line.len() - line.trim_start().len();
                let indent_str: String = line.chars().take(indent).collect();
                let new_line = format!("{}{}: {}", indent_str, key, new_value);
                if *line != new_line {
                    *line = new_line;
                    changed = true;
                }
                break;
            }
        }
    }

    changed
}

/// Validate that workspace packages' version constraints on sibling packages are
/// satisfied by the siblings' actual versions.
///
/// This catches cases like package `app` depending on `core: ^1.0.0` while
/// workspace package `core` is at version `2.0.0`. Such mismatches would cause
/// failures when packages are published (without local path overrides).
fn enforce_versions(packages: &[Package], all_workspace_packages: &[Package]) -> Result<()> {
    let workspace_map: HashMap<&str, &Package> = all_workspace_packages
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();

    let mut violations = Vec::new();

    for pkg in packages {
        // Check all dependencies that are also workspace packages
        for dep_name in pkg.dependencies.iter().chain(pkg.dev_dependencies.iter()) {
            let Some(sibling) = workspace_map.get(dep_name.as_str()) else {
                continue; // Not a workspace package
            };

            let Some(constraint_str) = pkg.dependency_versions.get(dep_name) else {
                continue; // No version constraint (path-only, SDK, etc.)
            };

            let Some(ref sibling_version_str) = sibling.version else {
                continue; // Sibling has no version
            };

            // Parse the version constraint and sibling version using semver
            let constraint = match semver::VersionReq::parse(constraint_str) {
                Ok(req) => req,
                Err(_) => {
                    // Can't parse constraint — skip (e.g. unusual Dart constraint syntax)
                    continue;
                }
            };

            // Dart versions may have +buildNumber; strip it for semver parsing
            let version_for_semver = sibling_version_str
                .split('+')
                .next()
                .unwrap_or(sibling_version_str);
            let sibling_version = match semver::Version::parse(version_for_semver) {
                Ok(v) => v,
                Err(_) => {
                    continue; // Can't parse version — skip
                }
            };

            if !constraint.matches(&sibling_version) {
                violations.push(format!(
                    "  {} depends on {} {} but workspace has {}",
                    pkg.name, dep_name, constraint_str, sibling_version_str
                ));
            }
        }
    }

    if violations.is_empty() {
        println!(
            "  {} All workspace dependency version constraints satisfied.\n",
            "OK".green()
        );
        return Ok(());
    }

    let msg = format!(
        "Version constraint violations found ({} issue{}):\n{}\n\n\
         Update the version constraints in pubspec.yaml to match the workspace packages' actual versions.",
        violations.len(),
        if violations.len() == 1 { "" } else { "s" },
        violations.join("\n")
    );
    anyhow::bail!(msg);
}

/// Build the `pub get` command string with optional flags.
fn build_pub_get_command(
    sdk: &str,
    enforce_lockfile: bool,
    no_example: bool,
    offline: bool,
) -> String {
    let mut cmd = format!("{} pub get", sdk);
    if enforce_lockfile {
        cmd.push_str(" --enforce-lockfile");
    }
    if no_example {
        cmd.push_str(" --no-example");
    }
    if offline {
        cmd.push_str(" --offline");
    }
    cmd
}

/// Generate `pubspec_overrides.yaml` files for local package linking (Melos 6.x mode).
///
/// For each package that depends on other workspace packages, we create a
/// `pubspec_overrides.yaml` with `dependency_overrides:` entries pointing to
/// the sibling package via a relative path.
///
/// If `dependency_override_paths` is non-empty, packages discovered in those
/// directories are also used as override sources (for deps that match by name).
///
/// This allows `pub get` to resolve workspace packages locally without
/// modifying the actual `pubspec.yaml`.
fn generate_pubspec_overrides(
    packages: &[Package],
    all_workspace_packages: &[Package],
    dependency_override_paths: &[String],
    workspace_root: &Path,
) -> Result<()> {
    // Discover extra packages from dependencyOverridePaths
    let mut extra_packages = Vec::new();
    for override_path_str in dependency_override_paths {
        let override_dir = workspace_root.join(override_path_str);
        if !override_dir.exists() {
            eprintln!(
                "  {} dependencyOverridePaths: '{}' does not exist, skipping",
                "WARN".yellow(),
                override_path_str
            );
            continue;
        }
        // Try to parse as a single package directory
        match Package::from_path(&override_dir) {
            Ok(pkg) => {
                extra_packages.push(pkg);
            }
            Err(_) => {
                // Not a package — try scanning immediate subdirectories
                if let Ok(entries) = std::fs::read_dir(&override_dir) {
                    for entry in entries.flatten() {
                        if entry.file_type().is_ok_and(|ft| ft.is_dir())
                            && let Ok(pkg) = Package::from_path(&entry.path())
                        {
                            extra_packages.push(pkg);
                        }
                    }
                }
            }
        }
    }

    if !extra_packages.is_empty() {
        println!(
            "  {} Found {} extra package(s) from dependencyOverridePaths",
            "i".blue(),
            extra_packages.len()
        );
    }

    // Build a combined set of all override sources
    let all_override_sources: Vec<&Package> = all_workspace_packages
        .iter()
        .chain(extra_packages.iter())
        .collect();

    let override_names: HashSet<&str> = all_override_sources
        .iter()
        .map(|p| p.name.as_str())
        .collect();

    let mut generated = 0u32;

    for pkg in packages {
        // Skip packages that use Dart workspace resolution — generating
        // pubspec_overrides.yaml would cause `pub get` to fail with:
        // "Cannot override workspace packages."
        if pkg.uses_workspace_resolution() {
            continue;
        }

        // Find all dependencies (regular + dev) that match override sources
        let local_deps: Vec<&Package> = pkg
            .dependencies
            .iter()
            .chain(pkg.dev_dependencies.iter())
            .filter(|dep| override_names.contains(dep.as_str()))
            .filter_map(|dep| {
                all_override_sources
                    .iter()
                    .find(|p| p.name == **dep)
                    .copied()
            })
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
    let mut content =
        String::from("# Generated by melos-rs. Do not edit.\ndependency_overrides:\n");

    // Sort deps by name for deterministic output
    let mut sorted_deps: Vec<&&Package> = local_deps.iter().collect();
    sorted_deps.sort_by_key(|p| &p.name);

    for dep in sorted_deps {
        let relative =
            pathdiff::diff_paths(&dep.path, pkg_path).unwrap_or_else(|| dep.path.clone());
        let relative_str = relative.display().to_string();

        content.push_str(&format!("  {}:\n", dep.name));
        content.push_str(&format!("    path: {}\n", relative_str));
    }

    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::config::{BootstrapCommandConfig, CommandConfig, MelosConfig};
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
            dependency_versions: HashMap::new(),
            resolution: None,
        }
    }

    fn make_workspace(bootstrap_config: Option<BootstrapCommandConfig>) -> Workspace {
        Workspace {
            root_path: PathBuf::from("/workspace"),
            config_source: ConfigSource::MelosYaml(PathBuf::from("/workspace/melos.yaml")),
            config: MelosConfig {
                name: "test".to_string(),
                packages: vec!["packages/**".to_string()],
                repository: None,
                sdk_path: None,
                command: Some(CommandConfig {
                    version: None,
                    bootstrap: bootstrap_config,
                    clean: None,
                    publish: None,
                    test: None,
                }),
                scripts: HashMap::new(),
                ignore: None,
                categories: HashMap::new(),
                use_root_as_package: None,
                discover_nested_workspaces: None,
            },
            packages: vec![],
            sdk_path: None,
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
        assert!(
            alpha_pos < zebra_pos,
            "Dependencies should be sorted by name"
        );
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
            enforce_lockfile: None,
            run_pub_get_offline: None,
            dependency_override_paths: None,
            environment: None,
            dependencies: None,
            dev_dependencies: None,
            hooks: None,
        }));
        assert_eq!(effective_concurrency(&ws, 5), 1);
    }

    #[test]
    fn test_effective_concurrency_parallel_true_uses_cli() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: Some(true),
            enforce_versions_for_dependency_resolution: None,
            enforce_lockfile: None,
            run_pub_get_offline: None,
            dependency_override_paths: None,
            environment: None,
            dependencies: None,
            dev_dependencies: None,
            hooks: None,
        }));
        assert_eq!(effective_concurrency(&ws, 8), 8);
    }

    #[test]
    fn test_effective_concurrency_parallel_none_uses_cli() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: None,
            enforce_versions_for_dependency_resolution: None,
            enforce_lockfile: None,
            run_pub_get_offline: None,
            dependency_override_paths: None,
            environment: None,
            dependencies: None,
            dev_dependencies: None,
            hooks: None,
        }));
        assert_eq!(effective_concurrency(&ws, 3), 3);
    }

    // -----------------------------------------------------------------------
    // build_pub_get_command tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_pub_get_command_default() {
        let cmd = build_pub_get_command("flutter", false, false, false);
        assert_eq!(cmd, "flutter pub get");
    }

    #[test]
    fn test_build_pub_get_command_enforce_lockfile() {
        let cmd = build_pub_get_command("dart", true, false, false);
        assert_eq!(cmd, "dart pub get --enforce-lockfile");
    }

    #[test]
    fn test_build_pub_get_command_no_example() {
        let cmd = build_pub_get_command("flutter", false, true, false);
        assert_eq!(cmd, "flutter pub get --no-example");
    }

    #[test]
    fn test_build_pub_get_command_offline() {
        let cmd = build_pub_get_command("dart", false, false, true);
        assert_eq!(cmd, "dart pub get --offline");
    }

    #[test]
    fn test_build_pub_get_command_all_flags() {
        let cmd = build_pub_get_command("flutter", true, true, true);
        assert_eq!(
            cmd,
            "flutter pub get --enforce-lockfile --no-example --offline"
        );
    }

    // -----------------------------------------------------------------------
    // config_enforce_lockfile tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_config_enforce_lockfile_true() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: None,
            enforce_versions_for_dependency_resolution: None,
            enforce_lockfile: Some(true),
            run_pub_get_offline: None,
            dependency_override_paths: None,
            environment: None,
            dependencies: None,
            dev_dependencies: None,
            hooks: None,
        }));
        assert!(config_enforce_lockfile(&ws));
    }

    #[test]
    fn test_config_enforce_lockfile_false() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: None,
            enforce_versions_for_dependency_resolution: None,
            enforce_lockfile: Some(false),
            run_pub_get_offline: None,
            dependency_override_paths: None,
            environment: None,
            dependencies: None,
            dev_dependencies: None,
            hooks: None,
        }));
        assert!(!config_enforce_lockfile(&ws));
    }

    #[test]
    fn test_config_enforce_lockfile_none() {
        let ws = make_workspace(None);
        assert!(!config_enforce_lockfile(&ws));
    }

    // -----------------------------------------------------------------------
    // enforce_versions tests
    // -----------------------------------------------------------------------

    fn make_versioned_package(
        name: &str,
        version: &str,
        deps: Vec<&str>,
        dep_versions: Vec<(&str, &str)>,
    ) -> Package {
        Package {
            name: name.to_string(),
            path: PathBuf::from(format!("/workspace/packages/{}", name)),
            version: Some(version.to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: deps.into_iter().map(String::from).collect(),
            dev_dependencies: vec![],
            dependency_versions: dep_versions
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            resolution: None,
        }
    }

    #[test]
    fn test_enforce_versions_all_satisfied() {
        let core = make_versioned_package("core", "1.2.3", vec![], vec![]);
        let app = make_versioned_package("app", "1.0.0", vec!["core"], vec![("core", "^1.0.0")]);
        let all = vec![core.clone(), app.clone()];
        assert!(enforce_versions(&[app], &all).is_ok());
    }

    #[test]
    fn test_enforce_versions_violation() {
        let core = make_versioned_package("core", "2.0.0", vec![], vec![]);
        let app = make_versioned_package("app", "1.0.0", vec!["core"], vec![("core", "^1.0.0")]);
        let all = vec![core.clone(), app.clone()];
        let result = enforce_versions(&[app], &all);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("app"));
        assert!(err.contains("core"));
        assert!(err.contains("^1.0.0"));
        assert!(err.contains("2.0.0"));
    }

    #[test]
    fn test_enforce_versions_no_constraint_skipped() {
        // If there's no version constraint (path-only dep), it's fine
        let core = make_versioned_package("core", "2.0.0", vec![], vec![]);
        let app = make_versioned_package("app", "1.0.0", vec!["core"], vec![]);
        let all = vec![core.clone(), app.clone()];
        assert!(enforce_versions(&[app], &all).is_ok());
    }

    #[test]
    fn test_enforce_versions_non_workspace_dep_skipped() {
        // External deps should not be checked
        let app = make_versioned_package("app", "1.0.0", vec!["http"], vec![("http", "^0.13.0")]);
        let all = vec![app.clone()];
        assert!(enforce_versions(&[app], &all).is_ok());
    }

    #[test]
    fn test_config_enforce_versions_default() {
        let ws = make_workspace(None);
        assert!(!config_enforce_versions(&ws));
    }

    #[test]
    fn test_config_enforce_versions_true() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: None,
            enforce_versions_for_dependency_resolution: Some(true),
            enforce_lockfile: None,
            run_pub_get_offline: None,
            dependency_override_paths: None,
            environment: None,
            dependencies: None,
            dev_dependencies: None,
            hooks: None,
        }));
        assert!(config_enforce_versions(&ws));
    }

    // -----------------------------------------------------------------------
    // runPubGetOffline config tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_config_run_pub_get_offline_true() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: None,
            enforce_versions_for_dependency_resolution: None,
            enforce_lockfile: None,
            run_pub_get_offline: Some(true),
            dependency_override_paths: None,
            environment: None,
            dependencies: None,
            dev_dependencies: None,
            hooks: None,
        }));
        assert!(config_run_pub_get_offline(&ws));
    }

    #[test]
    fn test_config_run_pub_get_offline_false() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: None,
            enforce_versions_for_dependency_resolution: None,
            enforce_lockfile: None,
            run_pub_get_offline: Some(false),
            dependency_override_paths: None,
            environment: None,
            dependencies: None,
            dev_dependencies: None,
            hooks: None,
        }));
        assert!(!config_run_pub_get_offline(&ws));
    }

    #[test]
    fn test_config_run_pub_get_offline_none() {
        let ws = make_workspace(None);
        assert!(!config_run_pub_get_offline(&ws));
    }

    // -----------------------------------------------------------------------
    // dependencyOverridePaths config tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_config_dependency_override_paths_some() {
        let ws = make_workspace(Some(BootstrapCommandConfig {
            run_pub_get_in_parallel: None,
            enforce_versions_for_dependency_resolution: None,
            enforce_lockfile: None,
            run_pub_get_offline: None,
            dependency_override_paths: Some(vec![
                "../external".to_string(),
                "../other".to_string(),
            ]),
            environment: None,
            dependencies: None,
            dev_dependencies: None,
            hooks: None,
        }));
        let paths = config_dependency_override_paths(&ws);
        assert_eq!(paths, vec!["../external", "../other"]);
    }

    #[test]
    fn test_config_dependency_override_paths_none() {
        let ws = make_workspace(None);
        let paths = config_dependency_override_paths(&ws);
        assert!(paths.is_empty());
    }

    // -----------------------------------------------------------------------
    // generate_pubspec_overrides with dependency_override_paths tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_generate_pubspec_overrides_with_extra_packages() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let app = Package {
            name: "app".to_string(),
            path: pkg_dir.clone(),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec!["core".to_string(), "external_lib".to_string()],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        };

        let core = make_package(
            "core",
            &dir.path().join("packages/core").to_string_lossy(),
            vec![],
        );

        // Create an external package directory
        let ext_dir = dir.path().join("external").join("external_lib");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("pubspec.yaml"),
            "name: external_lib\nversion: 2.0.0\nenvironment:\n  sdk: '>=3.0.0 <4.0.0'\n",
        )
        .unwrap();

        let result =
            generate_pubspec_overrides(&[app], &[core], &["external".to_string()], dir.path());
        assert!(result.is_ok());

        let overrides_path = pkg_dir.join("pubspec_overrides.yaml");
        assert!(overrides_path.exists());
        let content = std::fs::read_to_string(&overrides_path).unwrap();
        assert!(content.contains("core:"));
        assert!(content.contains("external_lib:"));
    }

    #[test]
    fn test_generate_pubspec_overrides_no_extra_paths() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let app = Package {
            name: "app".to_string(),
            path: pkg_dir.clone(),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec!["core".to_string()],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        };

        let core_dir = dir.path().join("packages").join("core");
        std::fs::create_dir_all(&core_dir).unwrap();
        let core = make_package("core", &core_dir.to_string_lossy(), vec![]);

        let result = generate_pubspec_overrides(&[app], &[core], &[], dir.path());
        assert!(result.is_ok());

        let overrides_path = pkg_dir.join("pubspec_overrides.yaml");
        assert!(overrides_path.exists());
        let content = std::fs::read_to_string(&overrides_path).unwrap();
        assert!(content.contains("core:"));
    }

    // -----------------------------------------------------------------------
    // sync_yaml_section tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_sync_yaml_section_updates_dependency() {
        let mut lines: Vec<String> = vec![
            "name: my_app".to_string(),
            "version: 1.0.0".to_string(),
            "dependencies:".to_string(),
            "  http: ^0.13.0".to_string(),
            "  intl: ^0.17.0".to_string(),
            "dev_dependencies:".to_string(),
            "  test: ^1.0.0".to_string(),
        ];
        let mut values = HashMap::new();
        values.insert("http".to_string(), "^1.0.0".to_string());

        let changed = sync_yaml_section(&mut lines, "dependencies", &values);
        assert!(changed);
        assert_eq!(lines[3], "  http: ^1.0.0");
        // intl should be unchanged
        assert_eq!(lines[4], "  intl: ^0.17.0");
    }

    #[test]
    fn test_sync_yaml_section_no_change_if_same() {
        let mut lines: Vec<String> =
            vec!["dependencies:".to_string(), "  http: ^1.0.0".to_string()];
        let mut values = HashMap::new();
        values.insert("http".to_string(), "^1.0.0".to_string());

        let changed = sync_yaml_section(&mut lines, "dependencies", &values);
        assert!(!changed);
    }

    #[test]
    fn test_sync_yaml_section_environment() {
        let mut lines: Vec<String> = vec![
            "name: my_app".to_string(),
            "environment:".to_string(),
            "  sdk: '>=2.0.0 <3.0.0'".to_string(),
            "dependencies:".to_string(),
            "  http: ^0.13.0".to_string(),
        ];
        let mut values = HashMap::new();
        values.insert("sdk".to_string(), "'>=3.0.0 <4.0.0'".to_string());

        let changed = sync_yaml_section(&mut lines, "environment", &values);
        assert!(changed);
        assert_eq!(lines[2], "  sdk: '>=3.0.0 <4.0.0'");
        // dependencies section should be unaffected
        assert_eq!(lines[4], "  http: ^0.13.0");
    }

    #[test]
    fn test_sync_yaml_section_only_matches_existing_keys() {
        let mut lines: Vec<String> =
            vec!["dependencies:".to_string(), "  http: ^0.13.0".to_string()];
        let mut values = HashMap::new();
        // This key doesn't exist in the file, so no change
        values.insert("dio".to_string(), "^5.0.0".to_string());

        let changed = sync_yaml_section(&mut lines, "dependencies", &values);
        assert!(!changed);
    }

    #[test]
    fn test_sync_yaml_section_empty_values() {
        let mut lines: Vec<String> =
            vec!["dependencies:".to_string(), "  http: ^0.13.0".to_string()];
        let values = HashMap::new();

        let changed = sync_yaml_section(&mut lines, "dependencies", &values);
        assert!(!changed);
    }

    // -----------------------------------------------------------------------
    // yaml_value_to_constraint tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_yaml_value_to_constraint_string() {
        let v = yaml_serde::Value::String("^1.0.0".to_string());
        assert_eq!(yaml_value_to_constraint(&v), Some("^1.0.0".to_string()));
    }

    #[test]
    fn test_yaml_value_to_constraint_null() {
        let v = yaml_serde::Value::Null;
        assert_eq!(yaml_value_to_constraint(&v), Some("any".to_string()));
    }

    #[test]
    fn test_yaml_value_to_constraint_mapping_returns_none() {
        let mut map = yaml_serde::Mapping::new();
        map.insert(
            yaml_serde::Value::String("git".to_string()),
            yaml_serde::Value::String("https://github.com/org/repo".to_string()),
        );
        let v = yaml_serde::Value::Mapping(map);
        assert_eq!(yaml_value_to_constraint(&v), None);
    }

    // -----------------------------------------------------------------------
    // sync_shared_dependencies integration test
    // -----------------------------------------------------------------------

    #[test]
    fn test_sync_shared_dependencies_updates_pubspec() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: app\nversion: 1.0.0\nenvironment:\n  sdk: '>=2.0.0 <3.0.0'\ndependencies:\n  http: ^0.13.0\n  intl: ^0.17.0\ndev_dependencies:\n  test: ^1.0.0\n",
        ).unwrap();

        let app = Package {
            name: "app".to_string(),
            path: pkg_dir.clone(),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec!["http".to_string(), "intl".to_string()],
            dev_dependencies: vec!["test".to_string()],
            dependency_versions: HashMap::new(),
            resolution: None,
        };

        let mut shared_deps = HashMap::new();
        shared_deps.insert(
            "http".to_string(),
            yaml_serde::Value::String("^1.0.0".to_string()),
        );

        let mut shared_dev_deps = HashMap::new();
        shared_dev_deps.insert(
            "test".to_string(),
            yaml_serde::Value::String("^2.0.0".to_string()),
        );

        let mut shared_env = HashMap::new();
        shared_env.insert("sdk".to_string(), "'>=3.0.0 <4.0.0'".to_string());

        let ws = Workspace {
            root_path: dir.path().to_path_buf(),
            config_source: ConfigSource::MelosYaml(dir.path().join("melos.yaml")),
            config: MelosConfig {
                name: "test".to_string(),
                packages: vec!["packages/**".to_string()],
                repository: None,
                sdk_path: None,
                command: Some(CommandConfig {
                    version: None,
                    bootstrap: Some(BootstrapCommandConfig {
                        run_pub_get_in_parallel: None,
                        enforce_versions_for_dependency_resolution: None,
                        enforce_lockfile: None,
                        run_pub_get_offline: None,
                        dependency_override_paths: None,
                        environment: Some(shared_env),
                        dependencies: Some(shared_deps),
                        dev_dependencies: Some(shared_dev_deps),
                        hooks: None,
                    }),
                    clean: None,
                    publish: None,
                    test: None,
                }),
                scripts: HashMap::new(),
                ignore: None,
                categories: HashMap::new(),
                use_root_as_package: None,
                discover_nested_workspaces: None,
            },
            packages: vec![app.clone()],
            sdk_path: None,
        };

        let result = sync_shared_dependencies(&[app], &ws);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(pkg_dir.join("pubspec.yaml")).unwrap();
        assert!(
            content.contains("http: ^1.0.0"),
            "http should be updated to ^1.0.0, got:\n{}",
            content
        );
        assert!(
            content.contains("test: ^2.0.0"),
            "test should be updated to ^2.0.0, got:\n{}",
            content
        );
        assert!(
            content.contains("sdk: '>=3.0.0 <4.0.0'"),
            "sdk should be updated, got:\n{}",
            content
        );
        // intl should remain unchanged
        assert!(
            content.contains("intl: ^0.17.0"),
            "intl should be unchanged, got:\n{}",
            content
        );
    }

    #[test]
    fn test_sync_shared_dependencies_no_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: app\nversion: 1.0.0\ndependencies:\n  http: ^0.13.0\n",
        )
        .unwrap();

        let app = Package {
            name: "app".to_string(),
            path: pkg_dir.clone(),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec!["http".to_string()],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        };

        let ws = make_workspace(None);
        let result = sync_shared_dependencies(&[app], &ws);
        assert!(result.is_ok());

        // File should be unchanged
        let content = std::fs::read_to_string(pkg_dir.join("pubspec.yaml")).unwrap();
        assert!(content.contains("http: ^0.13.0"));
    }

    // -----------------------------------------------------------------------
    // workspace resolution tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_generate_pubspec_overrides_skips_workspace_resolution() {
        let dir = tempfile::TempDir::new().unwrap();

        // app uses workspace resolution — should NOT get pubspec_overrides.yaml
        let app_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&app_dir).unwrap();

        let app = Package {
            name: "app".to_string(),
            path: app_dir.clone(),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec!["core".to_string()],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: Some("workspace".to_string()),
        };

        let core_dir = dir.path().join("packages").join("core");
        std::fs::create_dir_all(&core_dir).unwrap();
        let core = make_package("core", &core_dir.to_string_lossy(), vec![]);

        let result = generate_pubspec_overrides(&[app], &[core], &[], dir.path());
        assert!(result.is_ok());

        // No pubspec_overrides.yaml should be generated for workspace-resolved package
        let overrides_path = app_dir.join("pubspec_overrides.yaml");
        assert!(
            !overrides_path.exists(),
            "pubspec_overrides.yaml should NOT be generated for workspace-resolved packages"
        );
    }

    #[test]
    fn test_generate_pubspec_overrides_mixed_resolution() {
        let dir = tempfile::TempDir::new().unwrap();

        // app uses workspace resolution — skipped
        let app_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&app_dir).unwrap();

        let app = Package {
            name: "app".to_string(),
            path: app_dir.clone(),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec!["core".to_string()],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: Some("workspace".to_string()),
        };

        // legacy_app does NOT use workspace resolution — should get overrides
        let legacy_dir = dir.path().join("packages").join("legacy_app");
        std::fs::create_dir_all(&legacy_dir).unwrap();

        let legacy_app = Package {
            name: "legacy_app".to_string(),
            path: legacy_dir.clone(),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec!["core".to_string()],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        };

        let core_dir = dir.path().join("packages").join("core");
        std::fs::create_dir_all(&core_dir).unwrap();
        let core = make_package("core", &core_dir.to_string_lossy(), vec![]);

        let result = generate_pubspec_overrides(&[app, legacy_app], &[core], &[], dir.path());
        assert!(result.is_ok());

        // workspace-resolved app: no overrides
        assert!(
            !app_dir.join("pubspec_overrides.yaml").exists(),
            "workspace-resolved app should not get pubspec_overrides.yaml"
        );

        // legacy app: should get overrides
        assert!(
            legacy_dir.join("pubspec_overrides.yaml").exists(),
            "legacy app should get pubspec_overrides.yaml"
        );
        let content = std::fs::read_to_string(legacy_dir.join("pubspec_overrides.yaml")).unwrap();
        assert!(content.contains("core:"));
    }
}
