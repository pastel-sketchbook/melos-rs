use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{self, MelosConfig};
use crate::package::{self, Package};

/// How the workspace configuration was found
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// Melos 3.x-6.x: configuration lives in a standalone `melos.yaml`
    MelosYaml(PathBuf),

    /// Melos 7.x: configuration lives under the `melos:` key in root `pubspec.yaml`
    PubspecYaml(PathBuf),
}

impl ConfigSource {
    /// The path to the config file
    pub fn path(&self) -> &Path {
        match self {
            ConfigSource::MelosYaml(p) | ConfigSource::PubspecYaml(p) => p,
        }
    }

    /// Whether this is the legacy 6.x format (melos.yaml)
    pub fn is_legacy(&self) -> bool {
        matches!(self, ConfigSource::MelosYaml(_))
    }
}

/// Represents a Melos workspace with its config and discovered packages
pub struct Workspace {
    /// Absolute path to the workspace root (where config file lives)
    pub root_path: PathBuf,

    /// How the config was found (melos.yaml vs pubspec.yaml)
    pub config_source: ConfigSource,

    /// Parsed configuration
    pub config: MelosConfig,

    /// All packages discovered in the workspace
    pub packages: Vec<Package>,
}

impl Workspace {
    /// Find melos.yaml or pubspec.yaml (with melos: key) by walking up from the
    /// current directory, then load the workspace.
    ///
    /// Priority: melos.yaml is preferred over pubspec.yaml (the user hasn't
    /// migrated to 7.x yet).
    pub fn find_and_load() -> Result<Self> {
        let config_source = find_config()?;
        let root_path = config_source
            .path()
            .parent()
            .context("Config file has no parent directory")?
            .to_path_buf();

        let config = config::parse_config(&config_source)?;
        let packages = package::discover_packages(&root_path, &config.packages)?;

        Ok(Workspace {
            root_path,
            config_source,
            config,
            packages,
        })
    }

    /// Build environment variables that are available to scripts and commands
    ///
    /// Melos provides these env vars:
    ///   MELOS_ROOT_PATH - absolute path to the workspace root
    ///   MELOS_PACKAGE_NAME - (set per-package during exec)
    ///   MELOS_PACKAGE_PATH - (set per-package during exec)
    ///   MELOS_PACKAGE_VERSION - (set per-package during exec)
    pub fn env_vars(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert(
            "MELOS_ROOT_PATH".to_string(),
            self.root_path.display().to_string(),
        );
        env
    }
}

/// Search for workspace config starting from the current directory and walking up.
///
/// For each directory we check:
/// 1. `melos.yaml` — if found, use 6.x mode (preferred)
/// 2. `pubspec.yaml` containing a top-level `melos:` key — use 7.x mode
///
/// If both exist in the same directory, `melos.yaml` wins (user hasn't migrated).
fn find_config() -> Result<ConfigSource> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    let mut dir: &Path = &cwd;

    loop {
        // Prefer melos.yaml (6.x)
        let melos_yaml = dir.join("melos.yaml");
        if melos_yaml.exists() {
            return Ok(ConfigSource::MelosYaml(melos_yaml));
        }

        // Fall back to pubspec.yaml with melos: key (7.x)
        let pubspec_yaml = dir.join("pubspec.yaml");
        if pubspec_yaml.exists() && pubspec_has_melos_key(&pubspec_yaml) {
            return Ok(ConfigSource::PubspecYaml(pubspec_yaml));
        }

        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }

    anyhow::bail!(
        "Could not find melos.yaml or pubspec.yaml (with melos: key) in '{}' or any parent directory.\n\
         \n\
         Hint: Create a melos.yaml (Melos 6.x) or add a `melos:` section to your root pubspec.yaml (Melos 7.x).",
        cwd.display()
    )
}

/// Check whether a pubspec.yaml file contains a top-level `melos:` key.
///
/// We do a quick YAML parse to a generic Value rather than a full MelosConfig
/// parse — this keeps the detection lightweight and avoids validation errors
/// at the discovery stage.
fn pubspec_has_melos_key(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };

    let Ok(value) = yaml_serde::from_str::<yaml_serde::Value>(&content) else {
        return false;
    };

    value
        .as_mapping()
        .is_some_and(|m| m.contains_key(yaml_serde::Value::String("melos".to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_pubspec_has_melos_key_positive() {
        let dir = TempDir::new().unwrap();
        let pubspec = dir.path().join("pubspec.yaml");
        fs::write(
            &pubspec,
            "name: my_workspace\nmelos:\n  name: my_workspace\n  scripts: {}\n",
        )
        .unwrap();
        assert!(pubspec_has_melos_key(&pubspec));
    }

    #[test]
    fn test_pubspec_has_melos_key_negative() {
        let dir = TempDir::new().unwrap();
        let pubspec = dir.path().join("pubspec.yaml");
        fs::write(&pubspec, "name: my_package\nversion: 1.0.0\n").unwrap();
        assert!(!pubspec_has_melos_key(&pubspec));
    }

    #[test]
    fn test_pubspec_has_melos_key_missing_file() {
        let path = PathBuf::from("/nonexistent/pubspec.yaml");
        assert!(!pubspec_has_melos_key(&path));
    }

    #[test]
    fn test_config_source_is_legacy() {
        let melos = ConfigSource::MelosYaml(PathBuf::from("melos.yaml"));
        assert!(melos.is_legacy());

        let pubspec = ConfigSource::PubspecYaml(PathBuf::from("pubspec.yaml"));
        assert!(!pubspec.is_legacy());
    }
}
