use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::filter::PackageFilters;
use crate::package::Package;

/// Apply package filters without category definitions.
///
/// Convenience wrapper for `apply_filters_with_categories` when categories
/// are not available (e.g., in tests or when filtering without a full config).
#[cfg(test)]
pub fn apply_filters(
    packages: &[Package],
    filters: &PackageFilters,
    workspace_root: Option<&Path>,
) -> Result<Vec<Package>> {
    apply_filters_with_categories(packages, filters, workspace_root, &HashMap::new())
}

/// Apply package filters with category definitions from melos.yaml.
///
/// `categories` maps category names to lists of package name glob patterns.
pub fn apply_filters_with_categories(
    packages: &[Package],
    filters: &PackageFilters,
    workspace_root: Option<&Path>,
    categories: &HashMap<String, Vec<String>>,
) -> Result<Vec<Package>> {
    // Resolve category filter into a set of matching package names
    let category_names: Option<HashSet<String>> =
        resolve_category_packages(packages, filters, categories);

    // First pass: apply direct filters
    let mut matched: Vec<Package> = packages
        .iter()
        .filter(|pkg| {
            matches_filters(pkg, filters)
                && category_names
                    .as_ref()
                    .is_none_or(|names| names.contains(&pkg.name))
        })
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

/// Resolve category filter into a set of package names that belong to any of the requested categories.
///
/// Returns `None` if no category filter is set (meaning no category restriction).
/// Returns `Some(set)` with matching package names if a category filter is active.
fn resolve_category_packages(
    packages: &[Package],
    filters: &PackageFilters,
    categories: &HashMap<String, Vec<String>>,
) -> Option<HashSet<String>> {
    let category_filter = filters.category.as_ref()?;
    if category_filter.is_empty() {
        return None;
    }

    let mut matching = HashSet::new();

    for requested_category in category_filter {
        if let Some(patterns) = categories.get(requested_category) {
            for pkg in packages {
                let in_category = patterns.iter().any(|pattern| {
                    glob::Pattern::new(pattern)
                        .map(|p| p.matches(&pkg.name))
                        .unwrap_or_else(|_| pkg.name.contains(pattern))
                });
                if in_category {
                    matching.insert(pkg.name.clone());
                }
            }
        }
    }

    Some(matching)
}

/// Topological sort of packages by their dependency relationships.
///
/// Returns packages in dependency order: packages with no local dependencies come first,
/// followed by packages that depend on them, etc. This is useful for `--order-dependents`
/// in exec and for bootstrap (ensuring dependencies are bootstrapped before dependents).
///
/// Uses Kahn's algorithm. If there are cycles, the cyclic packages are appended
/// at the end (not silently dropped).
pub fn topological_sort(packages: &[Package]) -> Vec<Package> {
    let known: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    let pkg_map: HashMap<&str, &Package> = packages.iter().map(|p| (p.name.as_str(), p)).collect();

    // Build adjacency list and in-degree map
    // Edge direction: dependency -> dependent (so deps come first in sort)
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();

    for pkg in packages {
        dependents.entry(pkg.name.as_str()).or_default();
        in_degree.entry(pkg.name.as_str()).or_insert(0);

        for dep in pkg.dependencies.iter().chain(pkg.dev_dependencies.iter()) {
            if known.contains(dep.as_str()) {
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(pkg.name.as_str());
                *in_degree.entry(pkg.name.as_str()).or_insert(0) += 1;
            }
        }
    }

    // Kahn's algorithm
    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|&(_, &deg)| deg == 0)
        .map(|(&name, _)| name)
        .collect();

    // Sort the initial queue for deterministic output
    let mut sorted_queue: Vec<&str> = queue.drain(..).collect();
    sorted_queue.sort();
    queue.extend(sorted_queue);

    let mut result: Vec<Package> = Vec::with_capacity(packages.len());

    while let Some(node) = queue.pop_front() {
        if let Some(&pkg) = pkg_map.get(node) {
            result.push(pkg.clone());
        }

        // Collect and sort neighbors for deterministic output
        if let Some(neighbors) = dependents.get(node) {
            let mut ready = Vec::new();
            for &neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(neighbor) {
                    *deg -= 1;
                    if *deg == 0 {
                        ready.push(neighbor);
                    }
                }
            }
            ready.sort();
            queue.extend(ready);
        }
    }

    // Append any remaining packages (part of cycles) to avoid dropping them
    if result.len() < packages.len() {
        let in_result: HashSet<String> = result.iter().map(|p| p.name.clone()).collect();
        let remaining: Vec<Package> = packages
            .iter()
            .filter(|p| !in_result.contains(&p.name))
            .cloned()
            .collect();
        result.extend(remaining);
    }

    result
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

    #[test]
    fn test_category_filter() {
        let packages = vec![
            make_package("app_main", true, vec!["flutter"]),
            make_package("app_settings", true, vec!["flutter"]),
            make_package("core_lib", false, vec![]),
            make_package("utils", false, vec![]),
        ];

        let mut categories = HashMap::new();
        categories.insert(
            "apps".to_string(),
            vec!["app_*".to_string()],
        );
        categories.insert(
            "libraries".to_string(),
            vec!["core_*".to_string(), "utils".to_string()],
        );

        // Filter to "apps" category only
        let filters = PackageFilters {
            category: Some(vec!["apps".to_string()]),
            ..Default::default()
        };

        let result =
            apply_filters_with_categories(&packages, &filters, None, &categories).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "app_main");
        assert_eq!(result[1].name, "app_settings");

        // Filter to "libraries" category
        let filters = PackageFilters {
            category: Some(vec!["libraries".to_string()]),
            ..Default::default()
        };
        let result =
            apply_filters_with_categories(&packages, &filters, None, &categories).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "core_lib");
        assert_eq!(result[1].name, "utils");

        // Filter to nonexistent category -> empty result
        let filters = PackageFilters {
            category: Some(vec!["nonexistent".to_string()]),
            ..Default::default()
        };
        let result =
            apply_filters_with_categories(&packages, &filters, None, &categories).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_topological_sort_basic() {
        // utils has no deps, core depends on utils, app depends on core
        let packages = vec![
            make_package("app", false, vec!["core"]),
            make_package("core", false, vec!["utils"]),
            make_package("utils", false, vec![]),
        ];

        let sorted = topological_sort(&packages);
        let names: Vec<&str> = sorted.iter().map(|p| p.name.as_str()).collect();
        // utils must come before core, core must come before app
        assert_eq!(names, vec!["utils", "core", "app"]);
    }

    #[test]
    fn test_topological_sort_independent() {
        // All independent packages - sorted alphabetically (deterministic)
        let packages = vec![
            make_package("charlie", false, vec![]),
            make_package("alpha", false, vec![]),
            make_package("bravo", false, vec![]),
        ];

        let sorted = topological_sort(&packages);
        let names: Vec<&str> = sorted.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn test_topological_sort_with_cycle() {
        // a -> b -> a (cycle), c is independent
        let packages = vec![
            make_package("a", false, vec!["b"]),
            make_package("b", false, vec!["a"]),
            make_package("c", false, vec![]),
        ];

        let sorted = topological_sort(&packages);
        // c has no deps so comes first; a and b are cyclic but still included
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].name, "c");
        // The cyclic packages a, b are appended at the end
        let cyclic: Vec<&str> = sorted[1..].iter().map(|p| p.name.as_str()).collect();
        assert!(cyclic.contains(&"a"));
        assert!(cyclic.contains(&"b"));
    }
}
