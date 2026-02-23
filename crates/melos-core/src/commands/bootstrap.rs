use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::BootstrapCommandConfig;
use crate::package::Package;
use crate::workspace::Workspace;

/// Extract the bootstrap command config from the workspace, if present.
pub fn bootstrap_config(workspace: &Workspace) -> Option<&BootstrapCommandConfig> {
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
pub fn effective_concurrency(workspace: &Workspace, cli_concurrency: usize) -> usize {
    match bootstrap_config(workspace).and_then(|b| b.run_pub_get_in_parallel) {
        Some(false) => 1,
        _ => cli_concurrency,
    }
}

/// Check if `enforce_lockfile` is set in bootstrap config.
pub fn config_enforce_lockfile(workspace: &Workspace) -> bool {
    bootstrap_config(workspace)
        .and_then(|b| b.enforce_lockfile)
        .unwrap_or(false)
}

/// Check if `run_pub_get_offline` is set in bootstrap config.
pub fn config_run_pub_get_offline(workspace: &Workspace) -> bool {
    bootstrap_config(workspace)
        .and_then(|b| b.run_pub_get_offline)
        .unwrap_or(false)
}

/// Get `dependencyOverridePaths` from bootstrap config, if set.
pub fn config_dependency_override_paths(workspace: &Workspace) -> Vec<String> {
    bootstrap_config(workspace)
        .and_then(|b| b.dependency_override_paths.clone())
        .unwrap_or_default()
}

/// Check if `enforce_versions_for_dependency_resolution` is set in bootstrap config.
pub fn config_enforce_versions(workspace: &Workspace) -> bool {
    bootstrap_config(workspace)
        .and_then(|b| b.enforce_versions_for_dependency_resolution)
        .unwrap_or(false)
}

/// Convert a YAML value (from shared deps config) to a version constraint string.
///
/// Supports:
/// - String: `"^1.0.0"` -> `Some("^1.0.0")`
/// - Null: -> `Some("any")` (null means accept any version)
/// - Other (mapping, etc.): -> `None` (complex deps like git can't be synced as simple strings)
pub fn yaml_value_to_constraint(value: &yaml_serde::Value) -> Option<String> {
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
pub fn sync_yaml_section(
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

/// Build the `pub get` command string with optional flags.
pub fn build_pub_get_command(
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

/// Validate that workspace packages' version constraints on sibling packages are
/// satisfied by the siblings' actual versions.
///
/// Returns `Ok(())` if all constraints are satisfied, or a list of violation
/// messages if any are not.
pub fn enforce_versions(
    packages: &[Package],
    all_workspace_packages: &[Package],
) -> Result<Vec<String>> {
    let workspace_map: HashMap<&str, &Package> = all_workspace_packages
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();

    let mut violations = Vec::new();

    for pkg in packages {
        for dep_name in pkg.dependencies.iter().chain(pkg.dev_dependencies.iter()) {
            let Some(sibling) = workspace_map.get(dep_name.as_str()) else {
                continue;
            };

            let Some(constraint_str) = pkg.dependency_versions.get(dep_name) else {
                continue;
            };

            let Some(ref sibling_version_str) = sibling.version else {
                continue;
            };

            let constraint = match semver::VersionReq::parse(constraint_str) {
                Ok(req) => req,
                Err(_) => continue,
            };

            // Dart versions may have +buildNumber; strip it for semver parsing
            let version_for_semver = sibling_version_str
                .split('+')
                .next()
                .unwrap_or(sibling_version_str);
            let sibling_version = match semver::Version::parse(version_for_semver) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if !constraint.matches(&sibling_version) {
                violations.push(format!(
                    "  {} depends on {} {} but workspace has {}",
                    pkg.name, dep_name, constraint_str, sibling_version_str
                ));
            }
        }
    }

    Ok(violations)
}

/// Sync shared dependency versions from bootstrap config into each package's pubspec.yaml.
///
/// Returns the number of packages whose pubspec.yaml was updated.
pub fn sync_shared_dependencies(packages: &[Package], workspace: &Workspace) -> Result<u32> {
    let bc = bootstrap_config(workspace);

    let shared_env = bc.and_then(|b| b.environment.as_ref());
    let shared_deps = bc.and_then(|b| b.dependencies.as_ref());
    let shared_dev_deps = bc.and_then(|b| b.dev_dependencies.as_ref());

    if shared_env.is_none() && shared_deps.is_none() && shared_dev_deps.is_none() {
        return Ok(0);
    }

    let mut synced_count = 0u32;

    for pkg in packages {
        let pubspec_path = pkg.path.join("pubspec.yaml");
        let content = std::fs::read_to_string(&pubspec_path)
            .with_context(|| format!("Failed to read pubspec.yaml for '{}'", pkg.name))?;

        let mut changed = false;
        let mut lines: Vec<String> = content.lines().map(String::from).collect();

        if let Some(env) = shared_env {
            changed |= sync_yaml_section(&mut lines, "environment", env);
        }

        if let Some(deps) = shared_deps {
            let string_deps: HashMap<String, String> = deps
                .iter()
                .filter_map(|(k, v)| yaml_value_to_constraint(v).map(|c| (k.clone(), c)))
                .collect();
            changed |= sync_yaml_section(&mut lines, "dependencies", &string_deps);
        }

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

    Ok(synced_count)
}

/// Result from generating pubspec_overrides.yaml files.
pub struct PubspecOverridesResult {
    /// Number of packages that got pubspec_overrides.yaml generated.
    pub generated: u32,
    /// Number of extra packages found from dependencyOverridePaths.
    pub extra_package_count: usize,
    /// Warnings for missing override paths.
    pub warnings: Vec<String>,
}

/// Generate `pubspec_overrides.yaml` files for local package linking (Melos 6.x mode).
///
/// For each package that depends on other workspace packages, creates a
/// `pubspec_overrides.yaml` with `dependency_overrides:` entries pointing to
/// the sibling package via a relative path.
pub fn generate_pubspec_overrides(
    packages: &[Package],
    all_workspace_packages: &[Package],
    dependency_override_paths: &[String],
    workspace_root: &Path,
) -> Result<PubspecOverridesResult> {
    let mut warnings = Vec::new();

    // Discover extra packages from dependencyOverridePaths
    let mut extra_packages = Vec::new();
    for override_path_str in dependency_override_paths {
        let override_dir = workspace_root.join(override_path_str);
        if !override_dir.exists() {
            warnings.push(format!(
                "dependencyOverridePaths: '{}' does not exist, skipping",
                override_path_str
            ));
            continue;
        }
        match Package::from_path(&override_dir) {
            Ok(pkg) => {
                extra_packages.push(pkg);
            }
            Err(_) => {
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

    let extra_package_count = extra_packages.len();

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
        // Skip packages that use Dart workspace resolution
        if pkg.uses_workspace_resolution() {
            continue;
        }

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
    }

    Ok(PubspecOverridesResult {
        generated,
        extra_package_count,
        warnings,
    })
}

/// Build the YAML content for a `pubspec_overrides.yaml` file.
pub fn build_pubspec_overrides_content(local_deps: &[&Package], pkg_path: &Path) -> Result<String> {
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

    use crate::config::{BootstrapCommandConfig, CommandConfig, ConfigSource, MelosConfig};
    use crate::workspace::Workspace;

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
                    build: None,
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
            warnings: vec![],
        }
    }

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

    // -- build_pubspec_overrides_content tests --

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

        let alpha_pos = content.find("alpha:").unwrap();
        let zebra_pos = content.find("zebra:").unwrap();
        assert!(
            alpha_pos < zebra_pos,
            "Dependencies should be sorted by name"
        );
    }

    // -- effective_concurrency tests --

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

    // -- build_pub_get_command tests --

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

    // -- config_enforce_lockfile tests --

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

    // -- enforce_versions tests --

    #[test]
    fn test_enforce_versions_all_satisfied() {
        let core = make_versioned_package("core", "1.2.3", vec![], vec![]);
        let app = make_versioned_package("app", "1.0.0", vec!["core"], vec![("core", "^1.0.0")]);
        let all = vec![core.clone(), app.clone()];
        let result = enforce_versions(&[app], &all).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_enforce_versions_violation() {
        let core = make_versioned_package("core", "2.0.0", vec![], vec![]);
        let app = make_versioned_package("app", "1.0.0", vec!["core"], vec![("core", "^1.0.0")]);
        let all = vec![core.clone(), app.clone()];
        let violations = enforce_versions(&[app], &all).unwrap();
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("app"));
        assert!(violations[0].contains("core"));
        assert!(violations[0].contains("^1.0.0"));
        assert!(violations[0].contains("2.0.0"));
    }

    #[test]
    fn test_enforce_versions_no_constraint_skipped() {
        let core = make_versioned_package("core", "2.0.0", vec![], vec![]);
        let app = make_versioned_package("app", "1.0.0", vec!["core"], vec![]);
        let all = vec![core.clone(), app.clone()];
        let result = enforce_versions(&[app], &all).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_enforce_versions_non_workspace_dep_skipped() {
        let app = make_versioned_package("app", "1.0.0", vec!["http"], vec![("http", "^0.13.0")]);
        let all = vec![app.clone()];
        let result = enforce_versions(&[app], &all).unwrap();
        assert!(result.is_empty());
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

    // -- runPubGetOffline config tests --

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

    // -- dependencyOverridePaths config tests --

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

    // -- generate_pubspec_overrides tests --

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

    // -- sync_yaml_section tests --

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
        assert_eq!(lines[4], "  http: ^0.13.0");
    }

    #[test]
    fn test_sync_yaml_section_only_matches_existing_keys() {
        let mut lines: Vec<String> =
            vec!["dependencies:".to_string(), "  http: ^0.13.0".to_string()];
        let mut values = HashMap::new();
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

    // -- yaml_value_to_constraint tests --

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

    // -- sync_shared_dependencies integration test --

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
                    build: None,
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
            warnings: vec![],
        };

        let result = sync_shared_dependencies(&[app], &ws).unwrap();
        assert_eq!(result, 1);

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
        let result = sync_shared_dependencies(&[app], &ws).unwrap();
        assert_eq!(result, 0);

        let content = std::fs::read_to_string(pkg_dir.join("pubspec.yaml")).unwrap();
        assert!(content.contains("http: ^0.13.0"));
    }

    // -- workspace resolution tests --

    #[test]
    fn test_generate_pubspec_overrides_skips_workspace_resolution() {
        let dir = tempfile::TempDir::new().unwrap();

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

        let overrides_path = app_dir.join("pubspec_overrides.yaml");
        assert!(
            !overrides_path.exists(),
            "pubspec_overrides.yaml should NOT be generated for workspace-resolved packages"
        );
    }

    #[test]
    fn test_generate_pubspec_overrides_mixed_resolution() {
        let dir = tempfile::TempDir::new().unwrap();

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

        assert!(
            !app_dir.join("pubspec_overrides.yaml").exists(),
            "workspace-resolved app should not get pubspec_overrides.yaml"
        );

        assert!(
            legacy_dir.join("pubspec_overrides.yaml").exists(),
            "legacy app should get pubspec_overrides.yaml"
        );
        let content = std::fs::read_to_string(legacy_dir.join("pubspec_overrides.yaml")).unwrap();
        assert!(content.contains("core:"));
    }
}
