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
    /// Full script configuration (boxed to reduce enum size)
    Full(Box<ScriptConfig>),
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
#[allow(dead_code)]
pub struct CommandConfig {
    /// Version command config
    pub version: Option<VersionCommandConfig>,

    /// Bootstrap command config
    pub bootstrap: Option<BootstrapCommandConfig>,

    /// Clean command config
    pub clean: Option<CleanCommandConfig>,
}

/// Configuration for the `version` command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct VersionCommandConfig {
    /// Branch to use for versioning (validates current branch matches)
    pub branch: Option<String>,

    /// Commit message template. Supports {new_version} and {package_name} placeholders.
    pub message: Option<String>,

    /// Whether to include scopes in conventional commit changelogs
    #[serde(default)]
    pub include_scopes: Option<bool>,

    /// Whether to create a tag for the versioned commit
    #[serde(default = "default_true_opt")]
    pub tag_release: Option<bool>,

    /// Whether to generate/update changelogs
    #[serde(default = "default_true_opt")]
    pub changelog: Option<bool>,

    /// Changelog configuration
    pub changelog_config: Option<ChangelogConfig>,

    /// Hooks configuration
    pub hooks: Option<VersionHooks>,

    /// Link to packages on pub.dev in changelogs
    #[serde(default)]
    pub link_to_commits: Option<bool>,

    /// Workspace-level changelog (aggregates all package changes)
    #[serde(default = "default_true_opt")]
    pub workspace_changelog: Option<bool>,
}

impl VersionCommandConfig {
    /// Returns the commit message template, with a sensible default
    pub fn message_template(&self) -> &str {
        self.message
            .as_deref()
            .unwrap_or("chore(release): publish packages")
    }

    /// Whether changelogs should be generated
    pub fn should_changelog(&self) -> bool {
        self.changelog.unwrap_or(true)
    }

    /// Whether tags should be created
    pub fn should_tag(&self) -> bool {
        self.tag_release.unwrap_or(true)
    }

    /// Whether a workspace-level CHANGELOG should be generated
    pub fn should_workspace_changelog(&self) -> bool {
        self.workspace_changelog.unwrap_or(true)
    }
}

/// Changelog-specific configuration
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ChangelogConfig {
    /// Include commit bodies in changelog
    #[serde(default)]
    pub include_commit_body: Option<bool>,

    /// Include commit IDs (short hash) in changelog entries
    #[serde(default)]
    pub include_commit_id: Option<bool>,
}

/// Hooks for versioning
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionHooks {
    /// Script to run before committing version changes
    pub pre_commit: Option<String>,

    /// Script to run after committing version changes
    pub post_commit: Option<String>,
}

/// Configuration for the `bootstrap` command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct BootstrapCommandConfig {
    /// Run `pub get` in parallel
    #[serde(default)]
    pub run_pub_get_in_parallel: Option<bool>,

    /// Enforce versions for dependency resolution
    #[serde(default)]
    pub enforce_versions_for_dependency_resolution: Option<bool>,
}

/// Configuration for the `clean` command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CleanCommandConfig {
    /// Additional hooks
    pub hooks: Option<CleanHooks>,
}

/// Hooks for the clean command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CleanHooks {
    /// Script to run after cleaning
    pub post: Option<String>,
}

fn default_true_opt() -> Option<bool> {
    Some(true)
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
    fn test_parse_version_command_config() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    branch: main
    message: "chore: release {new_version}"
    changelog: true
    includeScopes: true
    linkToCommits: true
    workspaceChangelog: true
    hooks:
      preCommit: dart format .
      postCommit: echo done
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert_eq!(version_config.branch.as_deref(), Some("main"));
        assert_eq!(
            version_config.message_template(),
            "chore: release {new_version}"
        );
        assert!(version_config.should_changelog());
        assert!(version_config.should_tag());
        assert_eq!(version_config.include_scopes, Some(true));
        assert_eq!(version_config.link_to_commits, Some(true));
        let hooks = version_config.hooks.unwrap();
        assert_eq!(hooks.pre_commit.as_deref(), Some("dart format ."));
        assert_eq!(hooks.post_commit.as_deref(), Some("echo done"));
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
