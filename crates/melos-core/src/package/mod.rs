pub mod filter;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
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

    /// The `resolution` field from pubspec.yaml (e.g. "workspace").
    ///
    /// Dart 3.5+ workspaces require each member package to declare
    /// `resolution: workspace`. When set, `pubspec_overrides.yaml` must NOT
    /// be generated because it conflicts with workspace resolution.
    pub resolution: Option<String>,
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
    pub resolution: Option<String>,

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
            resolution: pubspec.resolution,
        })
    }

    /// Whether this package uses Dart workspace resolution (`resolution: workspace`).
    ///
    /// When `true`, `pubspec_overrides.yaml` must NOT be generated because the
    /// Dart workspace resolver handles dependency linking and rejects overrides.
    pub fn uses_workspace_resolution(&self) -> bool {
        self.resolution
            .as_ref()
            .is_some_and(|r| r.eq_ignore_ascii_case("workspace"))
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

/// Directories excluded during package discovery.
///
/// These contain cached dependencies, build artifacts, or IDE files whose
/// `pubspec.yaml` files should never be treated as workspace packages.
/// Matches the ignore patterns used by real Melos:
///   `.dart_tool`, `.symlinks/plugins`, `.fvm`, `.plugin_symlinks`
/// plus additional safety exclusions for `.pub-cache`, `build`, and IDE dirs.
const EXCLUDED_PACKAGE_DIRS: &[&str] = &[
    ".dart_tool",
    ".symlinks",
    ".plugin_symlinks",
    ".pub-cache",
    ".pub",
    ".fvm",
    "build",
    ".idea",
    ".vscode",
];

/// Returns `true` if any component of `path` (relative to `root`) is in
/// [`EXCLUDED_PACKAGE_DIRS`], meaning the path lives inside an artifact
/// directory and should be skipped during package discovery.
fn is_in_excluded_dir(path: &Path, root: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .any(|c| matches!(c, std::path::Component::Normal(s) if EXCLUDED_PACKAGE_DIRS.contains(&s.to_str().unwrap_or(""))))
}

/// Discover all packages in the workspace matching the given glob patterns.
///
/// Glob iteration is sequential (cheap directory matching), but pubspec parsing
/// is parallelized across cores via rayon for faster discovery in large workspaces.
///
/// Directories listed in [`EXCLUDED_PACKAGE_DIRS`] (e.g. `.dart_tool`,
/// `.symlinks`, `build`) are automatically skipped so that cached
/// dependencies and build artifacts are never treated as workspace packages.
pub fn discover_packages(root: &Path, patterns: &[String]) -> Result<Vec<Package>> {
    // Phase 1: collect candidate directories sequentially (glob is fast)
    let mut candidate_dirs: Vec<PathBuf> = Vec::new();

    for pattern in patterns {
        let full_pattern = root.join(pattern).display().to_string();

        for entry in glob::glob(&full_pattern)
            .with_context(|| format!("Invalid glob pattern: {}", pattern))?
        {
            let entry_path = entry.with_context(|| "Failed to read glob entry")?;

            // Skip directories inside artifact/cache directories
            if is_in_excluded_dir(&entry_path, root) {
                continue;
            }

            if entry_path.is_dir() && entry_path.join("pubspec.yaml").exists() {
                candidate_dirs.push(entry_path);
            }
        }
    }

    // Phase 2: parse pubspec.yaml files in parallel
    let mut packages: Vec<Package> = candidate_dirs
        .par_iter()
        .filter_map(|dir| match Package::from_path(dir) {
            Ok(pkg) => Some(pkg),
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse package at {}: {}",
                    dir.display(),
                    e
                );
                None
            }
        })
        .collect();

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
            resolution: None,
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
            resolution: None,
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
            resolution: None,
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
            resolution: None,
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
        assert!(!pkg.dependency_versions.contains_key("flutter"));
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

    #[test]
    fn test_from_path_with_workspace_resolution() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = dir.path().join("ws_pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: ws_pkg\nversion: 1.0.0\nresolution: workspace\n",
        )
        .unwrap();

        let pkg = Package::from_path(&pkg_dir).unwrap();
        assert_eq!(pkg.resolution, Some("workspace".to_string()));
        assert!(pkg.uses_workspace_resolution());
    }

    #[test]
    fn test_uses_workspace_resolution_true() {
        let pkg = Package {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            version: None,
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: Some("workspace".to_string()),
        };
        assert!(pkg.uses_workspace_resolution());
    }

    #[test]
    fn test_uses_workspace_resolution_case_insensitive() {
        let pkg = Package {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            version: None,
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: Some("Workspace".to_string()),
        };
        assert!(pkg.uses_workspace_resolution());
    }

    #[test]
    fn test_uses_workspace_resolution_false_other() {
        let pkg = Package {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            version: None,
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: Some("local".to_string()),
        };
        assert!(!pkg.uses_workspace_resolution());
    }

    #[test]
    fn test_uses_workspace_resolution_none() {
        let pkg = Package {
            name: "test".to_string(),
            path: PathBuf::from("/test"),
            version: None,
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        };
        assert!(!pkg.uses_workspace_resolution());
    }

    // --- is_in_excluded_dir tests ---

    #[test]
    fn test_excluded_dir_dart_tool() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/packages/app/.dart_tool/cache/some_pkg");
        assert!(is_in_excluded_dir(path, root));
    }

    #[test]
    fn test_excluded_dir_symlinks() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/packages/app/.symlinks/plugins/path_provider_windows");
        assert!(is_in_excluded_dir(path, root));
    }

    #[test]
    fn test_excluded_dir_plugin_symlinks() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/packages/app/.plugin_symlinks/some_plugin");
        assert!(is_in_excluded_dir(path, root));
    }

    #[test]
    fn test_excluded_dir_fvm() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/.fvm/versions/3.10.0/packages/sky_engine");
        assert!(is_in_excluded_dir(path, root));
    }

    #[test]
    fn test_excluded_dir_pub_cache() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/packages/app/.pub-cache/hosted/pub.dev/http-1.0.0");
        assert!(is_in_excluded_dir(path, root));
    }

    #[test]
    fn test_excluded_dir_build() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/packages/app/build/flutter_assets");
        assert!(is_in_excluded_dir(path, root));
    }

    #[test]
    fn test_excluded_dir_ide_directories() {
        let root = Path::new("/workspace");
        assert!(is_in_excluded_dir(
            Path::new("/workspace/.idea/libraries"),
            root,
        ));
        assert!(is_in_excluded_dir(
            Path::new("/workspace/.vscode/settings"),
            root,
        ));
    }

    #[test]
    fn test_not_excluded_normal_package() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/packages/core");
        assert!(!is_in_excluded_dir(path, root));
    }

    #[test]
    fn test_not_excluded_nested_package() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/packages/features/auth");
        assert!(!is_in_excluded_dir(path, root));
    }

    #[test]
    fn test_excluded_dir_deeply_nested() {
        let root = Path::new("/workspace");
        // .symlinks deeply nested inside a package
        let path = Path::new("/workspace/packages/app/ios/.symlinks/plugins/url_launcher_ios");
        assert!(is_in_excluded_dir(path, root));
    }

    // --- discover_packages exclusion integration tests ---

    #[test]
    fn test_discover_excludes_dart_tool_packages() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Real package
        let real_pkg = root.join("packages").join("core");
        fs::create_dir_all(&real_pkg).unwrap();
        fs::write(
            real_pkg.join("pubspec.yaml"),
            "name: core\nversion: 1.0.0\n",
        )
        .unwrap();

        // Cached package inside .dart_tool (should be excluded)
        let cached_pkg = root
            .join("packages")
            .join("core")
            .join(".dart_tool")
            .join("package_config")
            .join("cached_dep");
        fs::create_dir_all(&cached_pkg).unwrap();
        fs::write(
            cached_pkg.join("pubspec.yaml"),
            "name: cached_dep\nversion: 0.1.0\n",
        )
        .unwrap();

        let packages = discover_packages(root, &["packages/**".to_string()]).unwrap();
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"core"), "real package should be found");
        assert!(
            !names.contains(&"cached_dep"),
            ".dart_tool packages should be excluded"
        );
    }

    #[test]
    fn test_discover_excludes_symlinks_packages() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Real package
        let real_pkg = root.join("packages").join("app");
        fs::create_dir_all(&real_pkg).unwrap();
        fs::write(real_pkg.join("pubspec.yaml"), "name: app\nversion: 1.0.0\n").unwrap();

        // Symlinked plugin package (should be excluded)
        let symlink_pkg = root
            .join("packages")
            .join("app")
            .join(".symlinks")
            .join("plugins")
            .join("path_provider_windows");
        fs::create_dir_all(&symlink_pkg).unwrap();
        fs::write(
            symlink_pkg.join("pubspec.yaml"),
            "name: path_provider_windows\nversion: 2.0.0\n",
        )
        .unwrap();

        let packages = discover_packages(root, &["packages/**".to_string()]).unwrap();
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"app"));
        assert!(
            !names.contains(&"path_provider_windows"),
            ".symlinks packages should be excluded"
        );
    }

    #[test]
    fn test_discover_excludes_build_dir_packages() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Real package
        let real_pkg = root.join("packages").join("ui");
        fs::create_dir_all(&real_pkg).unwrap();
        fs::write(real_pkg.join("pubspec.yaml"), "name: ui\nversion: 1.0.0\n").unwrap();

        // Build artifact package (should be excluded)
        let build_pkg = root
            .join("packages")
            .join("ui")
            .join("build")
            .join("some_output");
        fs::create_dir_all(&build_pkg).unwrap();
        fs::write(
            build_pkg.join("pubspec.yaml"),
            "name: build_artifact\nversion: 0.0.1\n",
        )
        .unwrap();

        let packages = discover_packages(root, &["packages/**".to_string()]).unwrap();
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"ui"));
        assert!(
            !names.contains(&"build_artifact"),
            "build dir packages should be excluded"
        );
    }

    #[test]
    fn test_discover_excludes_multiple_artifact_dirs() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Real packages
        let pkg_a = root.join("packages").join("pkg_a");
        fs::create_dir_all(&pkg_a).unwrap();
        fs::write(pkg_a.join("pubspec.yaml"), "name: pkg_a\nversion: 1.0.0\n").unwrap();

        let pkg_b = root.join("packages").join("pkg_b");
        fs::create_dir_all(&pkg_b).unwrap();
        fs::write(pkg_b.join("pubspec.yaml"), "name: pkg_b\nversion: 1.0.0\n").unwrap();

        // Various artifact packages that should all be excluded
        for excluded in &[".dart_tool", ".symlinks", ".fvm", ".pub-cache", "build"] {
            let artifact = root
                .join("packages")
                .join("pkg_a")
                .join(excluded)
                .join("fake_pkg");
            fs::create_dir_all(&artifact).unwrap();
            fs::write(
                artifact.join("pubspec.yaml"),
                format!(
                    "name: fake_{}\nversion: 0.0.1\n",
                    excluded.replace(['.', '-'], "_")
                ),
            )
            .unwrap();
        }

        let packages = discover_packages(root, &["packages/**".to_string()]).unwrap();
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();

        assert_eq!(
            names.len(),
            2,
            "only real packages should be found: {:?}",
            names
        );
        assert!(names.contains(&"pkg_a"));
        assert!(names.contains(&"pkg_b"));
    }
}
