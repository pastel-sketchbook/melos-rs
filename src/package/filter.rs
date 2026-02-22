use std::collections::{HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::filter::PackageFilters;
use crate::package::Package;

/// Apply package filters to a list of packages, returning only those that match.
///
/// This is the main filtering entry point used by all commands. It handles:
/// - Glob-based scope/ignore matching on package names
/// - Flutter/Dart filtering
/// - Directory/file existence checks
/// - Dependency-based filtering (depends-on, no-depends-on)
/// - Private package exclusion (no-private)
/// - Git diff-based change detection
/// - Transitive dependency/dependent expansion
pub fn apply_filters(
    packages: &[Package],
    filters: &PackageFilters,
    workspace_root: Option<&Path>,
) -> Result<Vec<Package>> {
    // First pass: apply direct filters
    let mut matched: Vec<Package> = packages
        .iter()
        .filter(|pkg| matches_filters(pkg, filters))
        .cloned()
        .collect();

    // Git diff filter: only keep packages with changed files since the ref
    if let Some(ref diff_ref) = filters.diff
        && let Some(root) = workspace_root
    {
        let changed = changed_packages_since(root, packages, diff_ref)?;
        matched.retain(|pkg| changed.contains(&pkg.name));
    }

    // Expand with transitive dependencies if requested
    if filters.include_dependencies {
        matched = expand_with_dependencies(&matched, packages);
    }

    // Expand with transitive dependents if requested
    if filters.include_dependents {
        matched = expand_with_dependents(&matched, packages);
    }

    Ok(matched)
}

/// Check if a single package matches all the given direct filters (no git/transitive expansion)
fn matches_filters(pkg: &Package, filters: &PackageFilters) -> bool {
    // Scope filter: package name must match at least one scope glob
    if let Some(ref scopes) = filters.scope {
        let matches_any = scopes.iter().any(|pattern| {
            glob::Pattern::new(pattern)
                .map(|p| p.matches(&pkg.name))
                .unwrap_or_else(|_| pkg.name.contains(pattern))
        });
        if !matches_any {
            return false;
        }
    }

    // Ignore filter: package name must NOT match any ignore glob
    if let Some(ref ignores) = filters.ignore {
        let matches_any = ignores.iter().any(|pattern| {
            glob::Pattern::new(pattern)
                .map(|p| p.matches(&pkg.name))
                .unwrap_or_else(|_| pkg.name.contains(pattern))
        });
        if matches_any {
            return false;
        }
    }

    // Flutter filter
    if let Some(flutter) = filters.flutter
        && pkg.is_flutter != flutter
    {
        return false;
    }

    // Directory exists filter
    if let Some(ref dir) = filters.dir_exists
        && !pkg.dir_exists(dir)
    {
        return false;
    }

    // File exists filter
    if let Some(ref file) = filters.file_exists
        && !pkg.file_exists(file)
    {
        return false;
    }

    // Depends-on filter: package must depend on ALL listed packages
    if let Some(ref deps) = filters.depends_on {
        for dep in deps {
            if !pkg.has_dependency(dep) {
                return false;
            }
        }
    }

    // No-depends-on filter: package must NOT depend on any listed package
    if let Some(ref no_deps) = filters.no_depends_on {
        for dep in no_deps {
            if pkg.has_dependency(dep) {
                return false;
            }
        }
    }

    // No-private filter: exclude packages with publish_to: none
    if filters.no_private && pkg.is_private() {
        return false;
    }

    true
}

/// Determine which packages have changed files since a git ref.
///
/// Runs `git diff --name-only <ref>` and maps changed file paths to their
/// containing packages.
fn changed_packages_since(
    workspace_root: &Path,
    packages: &[Package],
    git_ref: &str,
) -> Result<HashSet<String>> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", git_ref])
        .current_dir(workspace_root)
        .output()
        .context("Failed to run git diff")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let changed_files: Vec<&str> = stdout.lines().collect();

    let mut changed_packages = HashSet::new();

    for pkg in packages {
        // Get the package path relative to workspace root
        let rel_path = pkg.path.strip_prefix(workspace_root).unwrap_or(&pkg.path);
        let rel_str = rel_path.to_string_lossy();

        // A package is "changed" if any changed file is under its directory
        for file in &changed_files {
            if file.starts_with(rel_str.as_ref()) {
                changed_packages.insert(pkg.name.clone());
                break;
            }
        }
    }

    Ok(changed_packages)
}

/// Expand a matched set of packages to also include their transitive dependencies.
///
/// For each matched package, walks its `dependencies` and `dev_dependencies` to find
/// other workspace packages that are depended on, recursively.
fn expand_with_dependencies(matched: &[Package], all_packages: &[Package]) -> Vec<Package> {
    let all_by_name: std::collections::HashMap<&str, &Package> =
        all_packages.iter().map(|p| (p.name.as_str(), p)).collect();

    let mut result_names: HashSet<String> = matched.iter().map(|p| p.name.clone()).collect();
    let mut queue: VecDeque<String> = matched.iter().map(|p| p.name.clone()).collect();

    while let Some(name) = queue.pop_front() {
        if let Some(pkg) = all_by_name.get(name.as_str()) {
            for dep in pkg.dependencies.iter().chain(pkg.dev_dependencies.iter()) {
                if all_by_name.contains_key(dep.as_str()) && result_names.insert(dep.clone()) {
                    queue.push_back(dep.clone());
                }
            }
        }
    }

    all_packages
        .iter()
        .filter(|p| result_names.contains(&p.name))
        .cloned()
        .collect()
}

/// Expand a matched set of packages to also include their transitive dependents.
///
/// For each matched package, finds all workspace packages that (transitively)
/// depend on it.
fn expand_with_dependents(matched: &[Package], all_packages: &[Package]) -> Vec<Package> {
    let mut result_names: HashSet<String> = matched.iter().map(|p| p.name.clone()).collect();
    let mut changed = true;

    // Fixed-point iteration: keep adding dependents until no new ones are found
    while changed {
        changed = false;
        for pkg in all_packages {
            if result_names.contains(&pkg.name) {
                continue;
            }
            // If this package depends on any package in our result set, include it
            let depends_on_matched = pkg
                .dependencies
                .iter()
                .chain(pkg.dev_dependencies.iter())
                .any(|dep| result_names.contains(dep));

            if depends_on_matched {
                result_names.insert(pkg.name.clone());
                changed = true;
            }
        }
    }

    all_packages
        .iter()
        .filter(|p| result_names.contains(&p.name))
        .cloned()
        .collect()
}

/// Parse CLI-style exec flags into PackageFilters
///
/// Handles flags like:
///   --depends-on="build_runner"
///   --flutter / --no-flutter
///   --file-exists="pubspec.yaml"
///   --dir-exists="test"
#[allow(dead_code)]
pub fn filters_from_exec_flags(
    depends_on: &Option<String>,
    flutter: Option<bool>,
    file_exists: &Option<String>,
    dir_exists: &Option<String>,
) -> PackageFilters {
    PackageFilters {
        flutter,
        dir_exists: dir_exists.clone(),
        file_exists: file_exists.clone(),
        depends_on: depends_on
            .as_ref()
            .map(|d| d.split(',').map(|s| s.trim().to_string()).collect()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_package(name: &str, is_flutter: bool, deps: Vec<&str>) -> Package {
        Package {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/packages/{}", name)),
            version: Some("1.0.0".to_string()),
            is_flutter,
            publish_to: None,
            dependencies: deps.into_iter().map(String::from).collect(),
            dev_dependencies: vec![],
        }
    }

    fn make_private_package(name: &str) -> Package {
        Package {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/packages/{}", name)),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: Some("none".to_string()),
            dependencies: vec![],
            dev_dependencies: vec![],
        }
    }

    #[test]
    fn test_flutter_filter() {
        let packages = vec![
            make_package("app", true, vec!["flutter"]),
            make_package("core", false, vec![]),
        ];

        let filters = PackageFilters {
            flutter: Some(true),
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "app");
    }

    #[test]
    fn test_depends_on_filter() {
        let packages = vec![
            make_package("app", true, vec!["build_runner", "flutter"]),
            make_package("core", false, vec!["json_annotation"]),
        ];

        let filters = PackageFilters {
            depends_on: Some(vec!["build_runner".to_string()]),
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "app");
    }

    #[test]
    fn test_scope_glob_filter() {
        let packages = vec![
            make_package("my_app", false, vec![]),
            make_package("my_core", false, vec![]),
            make_package("other_lib", false, vec![]),
        ];

        let filters = PackageFilters {
            scope: Some(vec!["my_*".to_string()]),
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "my_app");
        assert_eq!(result[1].name, "my_core");
    }

    #[test]
    fn test_scope_multiple_globs() {
        let packages = vec![
            make_package("app_main", false, vec![]),
            make_package("core_lib", false, vec![]),
            make_package("utils", false, vec![]),
        ];

        let filters = PackageFilters {
            scope: Some(vec!["app_*".to_string(), "core_*".to_string()]),
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "app_main");
        assert_eq!(result[1].name, "core_lib");
    }

    #[test]
    fn test_ignore_glob_filter() {
        let packages = vec![
            make_package("app", false, vec![]),
            make_package("app_test", false, vec![]),
            make_package("core", false, vec![]),
        ];

        let filters = PackageFilters {
            ignore: Some(vec!["*_test".to_string()]),
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "app");
        assert_eq!(result[1].name, "core");
    }

    #[test]
    fn test_no_private_filter() {
        let packages = vec![
            make_package("public_pkg", false, vec![]),
            make_private_package("private_pkg"),
        ];

        let filters = PackageFilters {
            no_private: true,
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "public_pkg");
    }

    #[test]
    fn test_no_depends_on_filter() {
        let packages = vec![
            make_package("app", true, vec!["flutter", "http"]),
            make_package("core", false, vec!["http"]),
            make_package("utils", false, vec![]),
        ];

        let filters = PackageFilters {
            no_depends_on: Some(vec!["flutter".to_string()]),
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "core");
        assert_eq!(result[1].name, "utils");
    }

    #[test]
    fn test_combined_filters() {
        let packages = vec![
            make_package("my_app", true, vec!["flutter"]),
            make_package("my_core", false, vec![]),
            make_private_package("my_internal"),
            make_package("other_lib", false, vec![]),
        ];

        let filters = PackageFilters {
            scope: Some(vec!["my_*".to_string()]),
            no_private: true,
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "my_app");
        assert_eq!(result[1].name, "my_core");
    }

    #[test]
    fn test_include_dependencies() {
        // app depends on core, core depends on utils
        let packages = vec![
            Package {
                name: "app".to_string(),
                path: PathBuf::from("/tmp/packages/app"),
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                publish_to: None,
                dependencies: vec!["core".to_string()],
                dev_dependencies: vec![],
            },
            Package {
                name: "core".to_string(),
                path: PathBuf::from("/tmp/packages/core"),
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                publish_to: None,
                dependencies: vec!["utils".to_string()],
                dev_dependencies: vec![],
            },
            Package {
                name: "utils".to_string(),
                path: PathBuf::from("/tmp/packages/utils"),
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                publish_to: None,
                dependencies: vec![],
                dev_dependencies: vec![],
            },
            Package {
                name: "unrelated".to_string(),
                path: PathBuf::from("/tmp/packages/unrelated"),
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                publish_to: None,
                dependencies: vec![],
                dev_dependencies: vec![],
            },
        ];

        // Scope to just "app", but include_dependencies should pull in core and utils
        let filters = PackageFilters {
            scope: Some(vec!["app".to_string()]),
            include_dependencies: true,
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        let names: Vec<&str> = result.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"app"));
        assert!(names.contains(&"core"));
        assert!(names.contains(&"utils"));
        assert!(!names.contains(&"unrelated"));
    }

    #[test]
    fn test_include_dependents() {
        // app depends on core, core depends on utils
        let packages = vec![
            Package {
                name: "app".to_string(),
                path: PathBuf::from("/tmp/packages/app"),
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                publish_to: None,
                dependencies: vec!["core".to_string()],
                dev_dependencies: vec![],
            },
            Package {
                name: "core".to_string(),
                path: PathBuf::from("/tmp/packages/core"),
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                publish_to: None,
                dependencies: vec!["utils".to_string()],
                dev_dependencies: vec![],
            },
            Package {
                name: "utils".to_string(),
                path: PathBuf::from("/tmp/packages/utils"),
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                publish_to: None,
                dependencies: vec![],
                dev_dependencies: vec![],
            },
            Package {
                name: "unrelated".to_string(),
                path: PathBuf::from("/tmp/packages/unrelated"),
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                publish_to: None,
                dependencies: vec![],
                dev_dependencies: vec![],
            },
        ];

        // Scope to just "utils", but include_dependents should pull in core and app
        let filters = PackageFilters {
            scope: Some(vec!["utils".to_string()]),
            include_dependents: true,
            ..Default::default()
        };

        let result = apply_filters(&packages, &filters, None).unwrap();
        let names: Vec<&str> = result.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"utils"));
        assert!(names.contains(&"core"));
        assert!(names.contains(&"app"));
        assert!(!names.contains(&"unrelated"));
    }
}
