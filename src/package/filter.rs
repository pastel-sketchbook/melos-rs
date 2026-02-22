use crate::config::filter::PackageFilters;
use crate::package::Package;

/// Apply package filters to a list of packages, returning only those that match
pub fn apply_filters(packages: &[Package], filters: &PackageFilters) -> Vec<Package> {
    packages
        .iter()
        .filter(|pkg| matches_filters(pkg, filters))
        .cloned()
        .collect()
}

/// Check if a single package matches all the given filters
fn matches_filters(pkg: &Package, filters: &PackageFilters) -> bool {
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

    // Depends-on filter
    if let Some(ref deps) = filters.depends_on {
        for dep in deps {
            if !pkg.has_dependency(dep) {
                return false;
            }
        }
    }

    // Scope filter (include only these package names)
    if let Some(ref scope) = filters.scope
        && !scope.contains(&pkg.name)
    {
        return false;
    }

    // Ignore filter (exclude these package names)
    if let Some(ref ignore) = filters.ignore
        && ignore.contains(&pkg.name)
    {
        return false;
    }

    true
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
        ignore: None,
        scope: None,
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
            dependencies: deps.into_iter().map(String::from).collect(),
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

        let result = apply_filters(&packages, &filters);
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

        let result = apply_filters(&packages, &filters);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "app");
    }
}
