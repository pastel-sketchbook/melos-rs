pub mod filter;
pub mod script;

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use self::script::ScriptConfig;
use crate::workspace::ConfigSource;

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

    /// Category definitions: category_name -> list of package name glob patterns
    #[serde(default)]
    pub categories: HashMap<String, Vec<String>>,
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
    pub fn package_filters(&self) -> Option<&filter::PackageFilters> {
        match self {
            ScriptEntry::Simple(_) => None,
            ScriptEntry::Full(config) => config.package_filters.as_ref(),
        }
    }

    /// Get script-level environment variables
    pub fn env(&self) -> &HashMap<String, String> {
        static EMPTY: std::sync::LazyLock<HashMap<String, String>> =
            std::sync::LazyLock::new(HashMap::new);
        match self {
            ScriptEntry::Simple(_) => &EMPTY,
            ScriptEntry::Full(config) => &config.env,
        }
    }
}

/// Configuration for the `command` section
#[derive(Debug, Deserialize)]
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
pub struct BootstrapCommandConfig {
    /// Run `pub get` in parallel
    #[serde(default)]
    pub run_pub_get_in_parallel: Option<bool>,

    /// Enforce versions for dependency resolution
    #[serde(default)]
    #[allow(dead_code)]
    pub enforce_versions_for_dependency_resolution: Option<bool>,
}

/// Configuration for the `clean` command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanCommandConfig {
    /// Additional hooks
    pub hooks: Option<CleanHooks>,
}

/// Hooks for the clean command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanHooks {
    /// Script to run after cleaning
    pub post: Option<String>,
}

fn default_true_opt() -> Option<bool> {
    Some(true)
}

/// Wrapper struct for Melos 7.x format: `pubspec.yaml` with a `melos:` key.
///
/// The top-level pubspec fields we need are `name` and `workspace`.
/// Everything else (scripts, command config, categories) lives under `melos:`.
#[derive(Debug, Deserialize)]
struct PubspecWithMelos {
    /// Top-level `name` from pubspec.yaml
    name: String,

    /// Dart workspace paths (replaces `packages` globs in 7.x)
    #[serde(default)]
    workspace: Option<Vec<String>>,

    /// The `melos:` section containing melos-specific config
    melos: MelosSection,
}

/// The `melos:` section inside a Melos 7.x root pubspec.yaml.
///
/// All the familiar melos.yaml fields except `name` (taken from pubspec top-level)
/// and `packages` (taken from `workspace:` field).
#[derive(Debug, Deserialize)]
struct MelosSection {
    /// Override workspace name (optional; defaults to pubspec `name`)
    #[serde(default)]
    name: Option<String>,

    /// Package glob patterns (optional in 7.x; falls back to `workspace:` paths)
    #[serde(default)]
    packages: Option<Vec<String>>,

    /// Command-level configuration
    #[serde(default)]
    command: Option<CommandConfig>,

    /// Named scripts
    #[serde(default)]
    scripts: HashMap<String, ScriptEntry>,

    /// Category definitions
    #[serde(default)]
    categories: HashMap<String, Vec<String>>,
}

/// Parse workspace config from the given config source.
///
/// - **6.x (`melos.yaml`)**: Direct deserialization to `MelosConfig`.
/// - **7.x (`pubspec.yaml`)**: Deserialize wrapper, then assemble `MelosConfig`
///   from pubspec top-level fields + the `melos:` section.
pub fn parse_config(source: &ConfigSource) -> Result<MelosConfig> {
    let path = source.path();
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    match source {
        ConfigSource::MelosYaml(_) => {
            let config: MelosConfig = yaml_serde::from_str(&content)
                .with_context(|| format!("Failed to parse melos.yaml: {}", path.display()))?;
            Ok(config)
        }
        ConfigSource::PubspecYaml(_) => {
            let wrapper: PubspecWithMelos = yaml_serde::from_str(&content).with_context(|| {
                format!(
                    "Failed to parse melos config from pubspec.yaml: {}",
                    path.display()
                )
            })?;

            // Name: prefer melos.name override, then pubspec top-level name
            let name = wrapper.melos.name.unwrap_or(wrapper.name);

            // Packages: prefer melos.packages, then workspace: paths converted to globs.
            // Dart workspace paths are explicit directory paths (e.g. "packages/core"),
            // but discover_packages expects glob patterns. We append "/**" if the path
            // doesn't already contain a glob character.
            let packages = if let Some(pkgs) = wrapper.melos.packages {
                pkgs
            } else if let Some(ws_paths) = wrapper.workspace {
                // Dart workspace lists explicit package paths, not globs.
                // Keep them as-is — discover_packages will match the directory.
                ws_paths
            } else {
                anyhow::bail!(
                    "No package paths found in pubspec.yaml: neither `melos.packages` nor `workspace:` is set.\n\
                     \n\
                     Hint: Add a `workspace:` field listing your package paths, or add `packages:` under `melos:`."
                );
            };

            Ok(MelosConfig {
                name,
                packages,
                command: wrapper.melos.command,
                scripts: wrapper.melos.scripts,
                categories: wrapper.melos.categories,
            })
        }
    }
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
        assert!(config.categories.is_empty());
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
        assert!(analyze.env().is_empty());

        let format = &config.scripts["format"];
        assert_eq!(format.run_command(), "dart format .");
        assert_eq!(format.description(), None);
    }

    #[test]
    fn test_parse_script_with_env() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
scripts:
  build:
    run: dart run build_runner build
    description: Run code generation
    env:
      DART_DEFINE: "flavor=production"
      API_URL: "https://api.example.com"
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let build = &config.scripts["build"];
        assert_eq!(build.run_command(), "dart run build_runner build");

        let env = build.env();
        assert_eq!(env.len(), 2);
        assert_eq!(env["DART_DEFINE"], "flavor=production");
        assert_eq!(env["API_URL"], "https://api.example.com");
    }

    #[test]
    fn test_parse_config_with_categories() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
categories:
  apps:
    - app_*
    - "*_app"
  libraries:
    - core_*
    - utils
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(config.categories.len(), 2);
        assert_eq!(
            config.categories["apps"],
            vec!["app_*".to_string(), "*_app".to_string()]
        );
        assert_eq!(
            config.categories["libraries"],
            vec!["core_*".to_string(), "utils".to_string()]
        );
    }

    // ── 7.x pubspec.yaml format tests ───────────────────────────────────

    #[test]
    fn test_parse_7x_pubspec_with_melos_section() {
        let yaml = r#"
name: my_workspace
workspace:
  - packages/core
  - packages/app
melos:
  scripts:
    analyze: dart analyze .
  categories:
    libs:
      - core
"#;
        let wrapper: PubspecWithMelos = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(wrapper.name, "my_workspace");
        assert_eq!(
            wrapper.workspace,
            Some(vec![
                "packages/core".to_string(),
                "packages/app".to_string()
            ])
        );
        assert_eq!(wrapper.melos.scripts.len(), 1);
        assert!(wrapper.melos.name.is_none());
    }

    #[test]
    fn test_parse_7x_with_melos_name_override() {
        let yaml = r#"
name: pubspec_name
melos:
  name: custom_workspace_name
  packages:
    - packages/**
"#;
        let wrapper: PubspecWithMelos = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(wrapper.melos.name.as_deref(), Some("custom_workspace_name"));
    }

    #[test]
    fn test_parse_7x_packages_from_workspace_field() {
        let yaml = r#"
name: my_workspace
workspace:
  - packages/core
  - packages/app
melos:
  scripts: {}
"#;
        let wrapper: PubspecWithMelos = yaml_serde::from_str(yaml).unwrap();

        // Simulate what parse_config does: fall back to workspace paths
        let packages = wrapper
            .melos
            .packages
            .unwrap_or_else(|| wrapper.workspace.unwrap_or_default());
        assert_eq!(
            packages,
            vec!["packages/core".to_string(), "packages/app".to_string()]
        );
    }

    #[test]
    fn test_parse_7x_melos_packages_override() {
        let yaml = r#"
name: my_workspace
workspace:
  - packages/core
  - packages/app
melos:
  packages:
    - packages/**
    - tools/**
"#;
        let wrapper: PubspecWithMelos = yaml_serde::from_str(yaml).unwrap();
        // melos.packages should take precedence over workspace:
        assert_eq!(
            wrapper.melos.packages,
            Some(vec!["packages/**".to_string(), "tools/**".to_string()])
        );
    }
}
