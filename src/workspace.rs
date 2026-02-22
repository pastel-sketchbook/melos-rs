use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{self, MelosConfig};
use crate::package::{self, Package};

/// Represents a Melos workspace with its config and discovered packages
pub struct Workspace {
    /// Absolute path to the workspace root (where melos.yaml lives)
    pub root_path: PathBuf,

    /// Parsed melos.yaml configuration
    pub config: MelosConfig,

    /// All packages discovered in the workspace
    pub packages: Vec<Package>,
}

impl Workspace {
    /// Find melos.yaml by walking up from the current directory, then load the workspace
    pub fn find_and_load() -> Result<Self> {
        let config_path = find_config_file()?;
        let root_path = config_path
            .parent()
            .context("Config file has no parent directory")?
            .to_path_buf();

        let config = config::parse_config(&config_path)?;
        let packages = package::discover_packages(&root_path, &config.packages)?;

        Ok(Workspace {
            root_path,
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

    /// Build per-package environment variables
    #[allow(dead_code)]
    pub fn package_env_vars(&self, pkg: &Package) -> HashMap<String, String> {
        let mut env = self.env_vars();
        env.insert("MELOS_PACKAGE_NAME".to_string(), pkg.name.clone());
        env.insert(
            "MELOS_PACKAGE_PATH".to_string(),
            pkg.path.display().to_string(),
        );
        if let Some(ref version) = pkg.version {
            env.insert("MELOS_PACKAGE_VERSION".to_string(), version.clone());
        }
        env
    }
}

/// Search for melos.yaml starting from the current directory and walking up
fn find_config_file() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    let mut dir: &Path = &cwd;

    loop {
        let candidate = dir.join("melos.yaml");
        if candidate.exists() {
            return Ok(candidate);
        }

        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }

    anyhow::bail!(
        "Could not find melos.yaml in '{}' or any parent directory",
        cwd.display()
    )
}
