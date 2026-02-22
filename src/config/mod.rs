pub mod filter;
pub mod script;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use self::script::ScriptConfig;

/// Top-level melos.yaml configuration
#[derive(Debug, Deserialize)]
pub struct MelosConfig {
    /// Workspace name
    pub name: String,

    /// Package glob patterns
    pub packages: Vec<String>,

    /// Command-level configuration (version hooks, etc.)
    #[serde(default)]
    pub command: Option<CommandConfig>,

    /// Named scripts
    #[serde(default)]
    pub scripts: HashMap<String, ScriptEntry>,
}

/// A script entry can be either a simple string or a full config object
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ScriptEntry {
    /// Simple string command
    Simple(String),
    /// Full script configuration
    Full(ScriptConfig),
}

impl ScriptEntry {
    /// Get the run command string
    pub fn run_command(&self) -> &str {
        match self {
            ScriptEntry::Simple(cmd) => cmd,
            ScriptEntry::Full(config) => &config.run,
        }
    }

    /// Get the description if available
    pub fn description(&self) -> Option<&str> {
        match self {
            ScriptEntry::Simple(_) => None,
            ScriptEntry::Full(config) => config.description.as_deref(),
        }
    }

    /// Get package filters if available
    #[allow(dead_code)]
    pub fn package_filters(&self) -> Option<&filter::PackageFilters> {
        match self {
            ScriptEntry::Simple(_) => None,
            ScriptEntry::Full(config) => config.package_filters.as_ref(),
        }
    }
}

/// Configuration for the `command` section
#[derive(Debug, Deserialize)]
pub struct CommandConfig {
    /// Version command config
    pub version: Option<VersionCommandConfig>,
}

/// Configuration for the `version` command
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct VersionCommandConfig {
    /// Branch to use for versioning
    pub branch: Option<String>,

    /// Hooks configuration
    pub hooks: Option<VersionHooks>,
}

/// Hooks for versioning
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionHooks {
    /// Script to run before committing version changes
    pub pre_commit: Option<String>,
}

/// Parse melos.yaml from a file path
pub fn parse_config(path: &Path) -> Result<MelosConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: MelosConfig = yaml_serde::from_str(&content)
        .with_context(|| format!("Failed to parse melos.yaml: {}", path.display()))?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(config.name, "test_project");
        assert_eq!(config.packages, vec!["packages/**"]);
        assert!(config.scripts.is_empty());
    }

    #[test]
    fn test_parse_config_with_scripts() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
scripts:
  analyze:
    run: flutter analyze .
    description: Run analysis
  format: dart format .
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(config.scripts.len(), 2);

        let analyze = &config.scripts["analyze"];
        assert_eq!(analyze.run_command(), "flutter analyze .");
        assert_eq!(analyze.description(), Some("Run analysis"));

        let format = &config.scripts["format"];
        assert_eq!(format.run_command(), "dart format .");
        assert_eq!(format.description(), None);
    }
}
