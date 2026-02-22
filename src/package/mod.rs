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

    /// Dependencies listed in pubspec.yaml
    pub dependencies: Vec<String>,

    /// Dev dependencies listed in pubspec.yaml
    pub dev_dependencies: Vec<String>,
}

/// Minimal pubspec.yaml structure for parsing
#[derive(Debug, Deserialize)]
pub struct PubspecYaml {
    pub name: String,

    #[serde(default)]
    pub version: Option<String>,

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
            dependencies,
            dev_dependencies,
        })
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
