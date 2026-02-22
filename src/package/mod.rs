pub mod filter;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Represents a Dart/Flutter package found in the workspace
#[derive(Debug, Clone)]
pub struct Package {
    /// Package name from pubspec.yaml
    pub name: String,

    /// Absolute path to the package directory
    pub path: PathBuf,

    /// Package version from pubspec.yaml
    pub version: Option<String>,

    /// Whether this is a Flutter package (has flutter dependency)
    pub is_flutter: bool,

    /// The `publish_to` field from pubspec.yaml (e.g. "none" for private packages)
    pub publish_to: Option<String>,

    /// Dependencies listed in pubspec.yaml
    pub dependencies: Vec<String>,

    /// Dev dependencies listed in pubspec.yaml
    pub dev_dependencies: Vec<String>,

    /// Version constraints for dependencies (dep name -> constraint string, e.g. "^1.0.0").
    ///
    /// Only populated for dependencies that specify a version constraint (string or
    /// mapping with a `version` key). SDK deps, path-only deps, and git-only deps
    /// are excluded.
    pub dependency_versions: HashMap<String, String>,
}

/// Minimal pubspec.yaml structure for parsing
#[derive(Debug, Deserialize)]
pub struct PubspecYaml {
    pub name: String,

    #[serde(default)]
    pub version: Option<String>,

    #[serde(default)]
    pub publish_to: Option<String>,

    #[serde(default)]
    pub dependencies: Option<HashMap<String, yaml_serde::Value>>,

    #[serde(default)]
    pub dev_dependencies: Option<HashMap<String, yaml_serde::Value>>,

    #[serde(default)]
    pub flutter: Option<yaml_serde::Value>,
}

impl Package {
    /// Parse a package from a directory containing pubspec.yaml
    pub fn from_path(path: &Path) -> Result<Self> {
        let pubspec_path = path.join("pubspec.yaml");
        let content = std::fs::read_to_string(&pubspec_path)
            .with_context(|| format!("Failed to read {}", pubspec_path.display()))?;

        let pubspec: PubspecYaml = yaml_serde::from_str(&content)
            .with_context(|| format!("Failed to parse {}", pubspec_path.display()))?;

        let dependencies: Vec<String> = pubspec
            .dependencies
            .as_ref()
            .map(|deps| deps.keys().cloned().collect())
            .unwrap_or_default();

        let dev_dependencies: Vec<String> = pubspec
            .dev_dependencies
            .as_ref()
            .map(|deps| deps.keys().cloned().collect())
            .unwrap_or_default();

        // Extract version constraints from both deps and dev_deps
        let mut dependency_versions = HashMap::new();
        for deps_map in [&pubspec.dependencies, &pubspec.dev_dependencies]
            .iter()
            .copied()
            .flatten()
        {
            for (name, value) in deps_map {
                if let Some(constraint) = extract_version_constraint(value) {
                    dependency_versions.insert(name.clone(), constraint);
                }
            }
        }

        // A package is a Flutter package if it has a `flutter` key at the top level
        // or depends on the `flutter` SDK
        let is_flutter = pubspec.flutter.is_some()
            || dependencies.contains(&"flutter".to_string())
            || pubspec
                .dependencies
                .as_ref()
                .map(|deps| {
                    deps.get("flutter").is_some_and(|v| {
                        // Check for `flutter: sdk: flutter` pattern
                        v.is_mapping()
                    })
                })
                .unwrap_or(false);

        Ok(Package {
            name: pubspec.name,
            path: path.to_path_buf(),
            version: pubspec.version,
            is_flutter,
            publish_to: pubspec.publish_to,
            dependencies,
            dev_dependencies,
            dependency_versions,
        })
    }

    /// Whether this package is private (publish_to: none)
    pub fn is_private(&self) -> bool {
        self.publish_to
            .as_ref()
            .is_some_and(|p| p.eq_ignore_ascii_case("none"))
    }

    /// Check if this package has a given dependency (in deps or dev_deps)
    pub fn has_dependency(&self, dep: &str) -> bool {
        self.dependencies.contains(&dep.to_string())
            || self.dev_dependencies.contains(&dep.to_string())
    }

    /// Check if a file exists relative to this package's directory
    pub fn file_exists(&self, relative_path: &str) -> bool {
        self.path.join(relative_path).is_file()
    }

    /// Check if a directory exists relative to this package's directory
    pub fn dir_exists(&self, relative_path: &str) -> bool {
        self.path.join(relative_path).is_dir()
    }
}

/// Extract a version constraint string from a YAML dependency value.
///
/// Supports:
///   - String value: `"^1.0.0"` -> `Some("^1.0.0")`
///   - Mapping with `version` key: `{version: "^1.0.0", path: "../core"}` -> `Some("^1.0.0")`
///   - SDK/path-only/git-only deps: -> `None`
fn extract_version_constraint(value: &yaml_serde::Value) -> Option<String> {
    match value {
        yaml_serde::Value::String(s) => {
            let trimmed = s.trim();
            // Skip "any" — it's not a real constraint
            if trimmed.is_empty() || trimmed == "any" {
                return None;
            }
            Some(trimmed.to_string())
        }
        yaml_serde::Value::Mapping(map) => {
            // Check for a "version" key in the mapping
            let version_key = yaml_serde::Value::String("version".to_string());
            if let Some(yaml_serde::Value::String(v)) = map.get(&version_key) {
                let trimmed = v.trim();
                if !trimmed.is_empty() && trimmed != "any" {
                    return Some(trimmed.to_string());
                }
            }
            None
        }
        _ => None,
    }
}

/// Discover all packages in the workspace matching the given glob patterns
pub fn discover_packages(root: &Path, patterns: &[String]) -> Result<Vec<Package>> {
    let mut packages = Vec::new();

    for pattern in patterns {
        let full_pattern = root.join(pattern).display().to_string();

        // Glob matches directories; each should contain a pubspec.yaml
        for entry in glob::glob(&full_pattern)
            .with_context(|| format!("Invalid glob pattern: {}", pattern))?
        {
            let entry_path = entry.with_context(|| "Failed to read glob entry")?;

            if entry_path.is_dir() {
                let pubspec = entry_path.join("pubspec.yaml");
                if pubspec.exists() {
                    match Package::from_path(&entry_path) {
                        Ok(pkg) => packages.push(pkg),
                        Err(e) => {
                            eprintln!(
                                "Warning: Failed to parse package at {}: {}",
                                entry_path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    // Sort by name for deterministic ordering
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_from_path_dart_package() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = dir.path().join("my_package");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: my_package\nversion: 1.2.3\ndependencies:\n  http: ^0.13.0\n",
        )
        .unwrap();

        let pkg = Package::from_path(&pkg_dir).unwrap();
        assert_eq!(pkg.name, "my_package");
        assert_eq!(pkg.version, Some("1.2.3".to_string()));
        assert!(!pkg.is_flutter);
        assert!(!pkg.is_private());
        assert!(pkg.dependencies.contains(&"http".to_string()));
    }

    #[test]
    fn test_from_path_flutter_package() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = dir.path().join("flutter_app");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: flutter_app\nversion: 2.0.0\ndependencies:\n  flutter:\n    sdk: flutter\nflutter:\n  uses-material-design: true\n",
        )
        .unwrap();

        let pkg = Package::from_path(&pkg_dir).unwrap();
        assert_eq!(pkg.name, "flutter_app");
        assert!(pkg.is_flutter);
    }

    #[test]
    fn test_from_path_private_package() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = dir.path().join("private_pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: private_pkg\nversion: 0.0.1\npublish_to: none\n",
        )
        .unwrap();

        let pkg = Package::from_path(&pkg_dir).unwrap();
        assert!(pkg.is_private());
        assert_eq!(pkg.publish_to, Some("none".to_string()));
    }

    #[test]
    fn test_is_private_case_insensitive() {
        let pkg = Package {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            version: None,
            is_flutter: false,
            publish_to: Some("NONE".to_string()),
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
        };
        assert!(pkg.is_private());
    }

    #[test]
    fn test_is_private_not_none() {
        let pkg = Package {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            version: None,
            is_flutter: false,
            publish_to: Some("https://pub.dev".to_string()),
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
        };
        assert!(!pkg.is_private());
    }

    #[test]
    fn test_is_private_no_publish_to() {
        let pkg = Package {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            version: None,
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
        };
        assert!(!pkg.is_private());
    }

    #[test]
    fn test_has_dependency() {
        let pkg = Package {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            version: None,
            is_flutter: false,
            publish_to: None,
            dependencies: vec!["http".to_string()],
            dev_dependencies: vec!["test".to_string()],
            dependency_versions: HashMap::new(),
        };
        assert!(pkg.has_dependency("http"));
        assert!(pkg.has_dependency("test"));
        assert!(!pkg.has_dependency("dio"));
    }

    #[test]
    fn test_from_path_with_dev_dependencies() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = dir.path().join("pkg_with_devdeps");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: pkg_with_devdeps\nversion: 1.0.0\ndependencies:\n  http: ^0.13.0\ndev_dependencies:\n  test: ^1.0.0\n  mockito: ^5.0.0\n",
        )
        .unwrap();

        let pkg = Package::from_path(&pkg_dir).unwrap();
        assert_eq!(pkg.dependencies.len(), 1);
        assert_eq!(pkg.dev_dependencies.len(), 2);
        assert!(pkg.has_dependency("test"));
        assert!(pkg.has_dependency("mockito"));
    }

    #[test]
    fn test_from_path_missing_pubspec() {
        let dir = TempDir::new().unwrap();
        let result = Package::from_path(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_dependency_versions_parsed() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = dir.path().join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: pkg\nversion: 1.0.0\ndependencies:\n  http: ^0.13.0\n  core:\n    version: ^2.0.0\n    path: ../core\n  flutter:\n    sdk: flutter\ndev_dependencies:\n  test: ^1.0.0\n",
        )
        .unwrap();

        let pkg = Package::from_path(&pkg_dir).unwrap();
        assert_eq!(
            pkg.dependency_versions.get("http").map(|s| s.as_str()),
            Some("^0.13.0")
        );
        assert_eq!(
            pkg.dependency_versions.get("core").map(|s| s.as_str()),
            Some("^2.0.0")
        );
        assert_eq!(
            pkg.dependency_versions.get("test").map(|s| s.as_str()),
            Some("^1.0.0")
        );
        // flutter SDK dep should have no version constraint
        assert!(pkg.dependency_versions.get("flutter").is_none());
    }

    #[test]
    fn test_extract_version_constraint_string() {
        let val = yaml_serde::Value::String("^1.0.0".to_string());
        assert_eq!(extract_version_constraint(&val), Some("^1.0.0".to_string()));
    }

    #[test]
    fn test_extract_version_constraint_any() {
        let val = yaml_serde::Value::String("any".to_string());
        assert_eq!(extract_version_constraint(&val), None);
    }

    #[test]
    fn test_extract_version_constraint_mapping_with_version() {
        let mut map = yaml_serde::Mapping::new();
        map.insert(
            yaml_serde::Value::String("version".to_string()),
            yaml_serde::Value::String("^2.0.0".to_string()),
        );
        map.insert(
            yaml_serde::Value::String("path".to_string()),
            yaml_serde::Value::String("../core".to_string()),
        );
        let val = yaml_serde::Value::Mapping(map);
        assert_eq!(extract_version_constraint(&val), Some("^2.0.0".to_string()));
    }

    #[test]
    fn test_extract_version_constraint_sdk_dep() {
        let mut map = yaml_serde::Mapping::new();
        map.insert(
            yaml_serde::Value::String("sdk".to_string()),
            yaml_serde::Value::String("flutter".to_string()),
        );
        let val = yaml_serde::Value::Mapping(map);
        assert_eq!(extract_version_constraint(&val), None);
    }
}
