pub mod filter;
pub mod script;

use std::collections::HashMap;
use std::fmt;

use anyhow::{Context, Result};
use serde::Deserialize;

use self::script::{ExecEntry, ScriptConfig};
use crate::workspace::ConfigSource;

/// Top-level melos.yaml configuration
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MelosConfig {
    /// Workspace name
    pub name: String,

    /// Package glob patterns
    pub packages: Vec<String>,

    /// Repository URL or object for changelog commit links
    #[serde(default)]
    pub repository: Option<RepositoryConfig>,

    /// Custom Dart/Flutter SDK path. Overrides the default SDK resolution.
    ///
    /// Can also be set via the `MELOS_SDK_PATH` env var or the `--sdk-path` CLI flag.
    /// Priority: CLI flag > env var > config file.
    #[serde(default)]
    pub sdk_path: Option<String>,

    /// Command-level configuration (version hooks, etc.)
    #[serde(default)]
    pub command: Option<CommandConfig>,

    /// Named scripts
    #[serde(default)]
    pub scripts: HashMap<String, ScriptEntry>,

    /// Global ignore patterns: packages matching these globs are excluded from all commands.
    ///
    /// Applied during workspace loading before any command-level filters.
    #[serde(default)]
    pub ignore: Option<Vec<String>>,

    /// Category definitions: category_name -> list of package name glob patterns
    #[serde(default)]
    pub categories: HashMap<String, Vec<String>>,

    /// When true, include the workspace root directory as a package.
    ///
    /// The root must contain a `pubspec.yaml`. Useful for workspaces where the
    /// root is itself a publishable Dart/Flutter package.
    #[serde(default)]
    pub use_root_as_package: Option<bool>,

    /// When true, recursively discover packages inside nested Dart workspaces.
    ///
    /// After initial package discovery, any discovered pubspec.yaml with a
    /// `workspace:` field is treated as a nested workspace root, and its
    /// listed workspace paths are also scanned for packages.
    #[serde(default)]
    pub discover_nested_workspaces: Option<bool>,
}

impl MelosConfig {
    /// Validate the config and return a list of warnings for suspicious patterns.
    ///
    /// This is a post-parse validation step — the config has already been
    /// successfully deserialized. These warnings help catch typos and
    /// misconfiguration that serde silently accepts.
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        // Check for empty packages list
        if self.packages.is_empty() {
            warnings.push(
                "No package paths configured. No packages will be discovered. \
                 Add glob patterns to the `packages` field."
                    .to_string(),
            );
        }

        // Check scripts for common issues
        for (name, entry) in &self.scripts {
            // Scripts with steps or exec config don't need a traditional run command
            let has_steps = entry.steps().is_some();
            let has_exec_config = entry.has_exec_config();

            if let Some(cmd) = entry.run_command() {
                // Warn about exec-style scripts missing `--` separator
                if is_exec_style(cmd) && !cmd.contains(" -- ") {
                    warnings.push(format!(
                        "Script '{}' looks like an exec command but has no `--` separator. \
                         The command may not be parsed correctly. \
                         Expected format: `melos exec [flags] -- <command>`",
                        name
                    ));
                }

                // Warn about empty run commands (only if no exec shorthand or steps)
                if cmd.trim().is_empty() && !has_exec_config && !has_steps {
                    warnings.push(format!("Script '{}' has an empty `run` command.", name));
                }
            } else if !has_exec_config && !has_steps {
                // No run command and no exec/steps config
                warnings.push(format!(
                    "Script '{}' has no `run`, `exec`, or `steps` defined.",
                    name
                ));
            }

            // Check for references to categories that don't exist
            if let Some(filters) = entry.package_filters()
                && let Some(ref cats) = filters.category
            {
                for cat in cats {
                    if !self.categories.contains_key(cat) {
                        warnings.push(format!(
                            "Script '{}' references category '{}' which is not defined in `categories`. \
                             Available categories: {}",
                            name,
                            cat,
                            if self.categories.is_empty() {
                                "(none)".to_string()
                            } else {
                                self.categories.keys().cloned().collect::<Vec<_>>().join(", ")
                            }
                        ));
                    }
                }
            }
        }

        warnings
    }
}

/// Check if a command string looks like an exec-style command
fn is_exec_style(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    trimmed.contains("melos exec") || trimmed.contains("melos-rs exec")
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
    /// Get the run command string.
    ///
    /// Returns `None` for:
    /// - `Simple("")` (shouldn't happen but handled)
    /// - `Full` scripts with exec string shorthand (command is in `exec` field)
    /// - `Full` scripts with only `steps` (no single command)
    ///
    /// For exec string shorthand, use `exec_command()` instead.
    pub fn run_command(&self) -> Option<&str> {
        match self {
            ScriptEntry::Simple(cmd) => Some(cmd),
            ScriptEntry::Full(config) => {
                // If exec is a string shorthand, the "run command" concept doesn't apply
                if matches!(config.exec, Some(ExecEntry::Command(_))) && config.run.is_empty() {
                    return None;
                }
                // If steps are present and run is empty, there's no single run command
                if config.steps.is_some() && config.run.is_empty() {
                    return None;
                }
                if config.run.is_empty() {
                    return None;
                }
                Some(&config.run)
            }
        }
    }

    /// Get the exec command (the command to run in each package).
    ///
    /// Returns the effective command to execute per-package:
    /// - Exec string shorthand: returns the exec string
    /// - Exec object + run: returns the `run` string
    /// - `melos exec` in run command: returns `None` (handled by string parsing)
    pub fn exec_command(&self) -> Option<&str> {
        match self {
            ScriptEntry::Simple(_) => None,
            ScriptEntry::Full(config) => match &config.exec {
                Some(ExecEntry::Command(cmd)) => Some(cmd),
                Some(ExecEntry::Options(_)) => {
                    if config.run.is_empty() {
                        None
                    } else {
                        Some(&config.run)
                    }
                }
                None => None,
            },
        }
    }

    /// Whether this script has exec configuration (either string or object form).
    pub fn has_exec_config(&self) -> bool {
        match self {
            ScriptEntry::Simple(_) => false,
            ScriptEntry::Full(config) => config.exec.is_some(),
        }
    }

    /// Get exec options from the config (concurrency, fail_fast, order_dependents).
    ///
    /// Returns `None` if no exec object config is present.
    pub fn exec_options(&self) -> Option<&script::ExecOptions> {
        match self {
            ScriptEntry::Simple(_) => None,
            ScriptEntry::Full(config) => match &config.exec {
                Some(ExecEntry::Options(opts)) => Some(opts),
                _ => None,
            },
        }
    }

    /// Get steps if this is a multi-step script.
    pub fn steps(&self) -> Option<&[String]> {
        match self {
            ScriptEntry::Simple(_) => None,
            ScriptEntry::Full(config) => config.steps.as_deref(),
        }
    }

    /// Whether this script is private (hidden from interactive selection and `run --list`).
    pub fn is_private(&self) -> bool {
        match self {
            ScriptEntry::Simple(_) => false,
            ScriptEntry::Full(config) => config.private.unwrap_or(false),
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

    /// Get the groups this script belongs to.
    ///
    /// Returns `None` for simple scripts or full scripts without groups.
    pub fn groups(&self) -> Option<&[String]> {
        match self {
            ScriptEntry::Simple(_) => None,
            ScriptEntry::Full(config) => config.groups.as_deref(),
        }
    }

    /// Check whether this script belongs to a given group.
    pub fn in_group(&self, group: &str) -> bool {
        self.groups()
            .is_some_and(|groups| groups.iter().any(|g| g == group))
    }
}

// ---------------------------------------------------------------------------
// Repository configuration
// ---------------------------------------------------------------------------

/// Minimal percent-encoding for URL query parameters.
///
/// Encodes characters that are not unreserved per RFC 3986 (letters, digits,
/// `-`, `.`, `_`, `~`). This is sufficient for tag names and titles in release
/// URLs without adding a dependency on a URL-encoding crate.
pub(crate) fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

/// Repository URL or structured config for changelog commit links.
///
/// Supports two forms:
///   - Simple URL string: `repository: https://github.com/org/repo`
///   - Object form: `repository: { type: github, origin: ..., owner: ..., name: ... }`
#[derive(Debug, Clone)]
pub struct RepositoryConfig {
    /// The full URL to the repository (e.g., https://github.com/invertase/melos)
    pub url: String,
}

impl RepositoryConfig {
    /// Get the commit URL for a given commit hash.
    /// Returns a URL like `https://github.com/org/repo/commit/<hash>`.
    pub fn commit_url(&self, hash: &str) -> String {
        let base = self.url.trim_end_matches('/');
        // GitHub/GitLab/Bitbucket all use /commit/<hash>
        format!("{}/commit/{}", base, hash)
    }

    /// Get a prefilled release creation page URL for a given tag and title.
    ///
    /// GitHub format: `https://github.com/owner/repo/releases/new?tag=<tag>&title=<title>`
    pub fn release_url(&self, tag: &str, title: &str) -> String {
        let base = self.url.trim_end_matches('/');
        let encoded_tag = url_encode(tag);
        let encoded_title = url_encode(title);
        format!(
            "{}/releases/new?tag={}&title={}",
            base, encoded_tag, encoded_title
        )
    }
}

impl<'de> serde::Deserialize<'de> for RepositoryConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        struct RepoVisitor;

        impl<'de> de::Visitor<'de> for RepoVisitor {
            type Value = RepositoryConfig;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter
                    .write_str("a repository URL string or an object with type/origin/owner/name")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> {
                Ok(RepositoryConfig { url: v.to_string() })
            }

            fn visit_map<M: de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut repo_type: Option<String> = None;
                let mut origin: Option<String> = None;
                let mut owner: Option<String> = None;
                let mut name: Option<String> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "type" => repo_type = Some(map.next_value()?),
                        "origin" => origin = Some(map.next_value()?),
                        "owner" => owner = Some(map.next_value()?),
                        "name" => name = Some(map.next_value()?),
                        _ => {
                            let _: yaml_serde::Value = map.next_value()?;
                        }
                    }
                }

                let owner = owner.ok_or_else(|| de::Error::missing_field("owner"))?;
                let name = name.ok_or_else(|| de::Error::missing_field("name"))?;

                let base_url = match origin {
                    Some(ref o) => o.trim_end_matches('/').to_string(),
                    None => {
                        let host = match repo_type.as_deref() {
                            Some("gitlab") => "https://gitlab.com",
                            Some("bitbucket") => "https://bitbucket.org",
                            Some("azure") => "https://dev.azure.com",
                            _ => "https://github.com", // default to github
                        };
                        host.to_string()
                    }
                };

                Ok(RepositoryConfig {
                    url: format!("{}/{}/{}", base_url, owner, name),
                })
            }
        }

        deserializer.deserialize_any(RepoVisitor)
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

    /// Publish command config
    pub publish: Option<PublishCommandConfig>,

    /// Test command config
    pub test: Option<TestCommandConfig>,
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

    /// Whether to push commits and tags to remote after versioning
    #[serde(default = "default_true_opt")]
    pub git_push: Option<bool>,

    /// Whether to fetch tags from remote before versioning (ensures accurate tag-based analysis)
    #[serde(default)]
    pub fetch_tags: Option<bool>,

    /// Coordinated versioning: keep all packages at the same version
    #[serde(default)]
    pub coordinated: Option<bool>,

    /// Whether to print release URL links after versioning (requires `repository` config).
    ///
    /// Generates prefilled GitHub/GitLab release creation page links for each versioned package.
    #[serde(default)]
    pub release_url: Option<bool>,

    /// Aggregate changelog definitions: generate additional CHANGELOG files that
    /// aggregate commits from filtered package subsets.
    ///
    /// Example:
    /// ```yaml
    /// command:
    ///   version:
    ///     changelogs:
    ///       - path: CHANGELOG_APPS.md
    ///         packageFilters:
    ///           scope: ["app_*"]
    ///         description: "Changes in application packages"
    /// ```
    #[serde(default)]
    pub changelogs: Option<Vec<AggregateChangelogConfig>>,

    /// Structured commit body inclusion rules.
    ///
    /// Replaces the simpler `changelogConfig.includeCommitBody` with fine-grained
    /// control over when commit bodies are included in changelogs.
    #[serde(default)]
    pub changelog_commit_bodies: Option<ChangelogCommitBodiesConfig>,

    /// Changelog formatting options (e.g., whether to include the date in headers).
    #[serde(default)]
    pub changelog_format: Option<ChangelogFormatConfig>,

    /// When true, update git tag references in dependent packages' pubspec.yaml
    /// that use git dependencies with `ref:` pointing to a tag of a bumped package.
    #[serde(default)]
    pub update_git_tag_refs: Option<bool>,
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

    /// Whether to push commits and tags to remote
    pub fn should_git_push(&self) -> bool {
        self.git_push.unwrap_or(true)
    }

    /// Whether coordinated versioning is enabled
    pub fn is_coordinated(&self) -> bool {
        self.coordinated.unwrap_or(false)
    }

    /// Whether to fetch tags from remote before versioning
    pub fn should_fetch_tags(&self) -> bool {
        self.fetch_tags.unwrap_or(false)
    }

    /// Whether to print release URL links after versioning
    pub fn should_release_url(&self) -> bool {
        self.release_url.unwrap_or(false)
    }

    /// Whether to update git tag refs in dependent packages
    pub fn should_update_git_tag_refs(&self) -> bool {
        self.update_git_tag_refs.unwrap_or(false)
    }

    /// Whether to include the date in changelog version headers (default: false per Melos docs)
    pub fn should_include_date(&self) -> bool {
        self.changelog_format
            .as_ref()
            .and_then(|f| f.include_date)
            .unwrap_or(false)
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

    /// Only include these conventional commit types in the changelog.
    /// If set, commits with types not in this list are excluded.
    /// Example: ["feat", "fix", "perf"]
    #[serde(default)]
    pub include_types: Option<Vec<String>>,

    /// Exclude these conventional commit types from the changelog.
    /// Applied after include_types (if both set, include_types takes precedence).
    /// Example: ["chore", "ci", "build"]
    #[serde(default)]
    pub exclude_types: Option<Vec<String>>,
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

/// Aggregate changelog configuration.
///
/// Defines an additional CHANGELOG file that aggregates commits from a subset
/// of packages (filtered by `packageFilters`).
///
/// YAML example:
/// ```yaml
/// changelogs:
///   - path: CHANGELOG_APPS.md
///     packageFilters:
///       scope: ["app_*"]
///     description: "Changes in application packages"
/// ```
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AggregateChangelogConfig {
    /// Path to the aggregate changelog file, relative to workspace root.
    pub path: String,

    /// Package filters to select which packages' commits are included.
    #[serde(default)]
    pub package_filters: Option<filter::PackageFilters>,

    /// Optional description text placed at the top of the changelog file.
    #[serde(default)]
    pub description: Option<String>,
}

/// Structured commit body inclusion rules for changelogs.
///
/// YAML example:
/// ```yaml
/// changelogCommitBodies:
///   include: true
///   onlyBreaking: true  # default: true — only include bodies for breaking changes
/// ```
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogCommitBodiesConfig {
    /// Whether to include commit bodies at all.
    #[serde(default)]
    pub include: bool,

    /// When true (default), only include commit bodies for breaking changes.
    #[serde(default = "default_true")]
    pub only_breaking: bool,
}

/// Changelog formatting options.
///
/// YAML example:
/// ```yaml
/// changelogFormat:
///   includeDate: true
/// ```
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogFormatConfig {
    /// Whether to include the date in changelog version headers.
    /// Default: false (per Melos docs).
    #[serde(default)]
    pub include_date: Option<bool>,
}

/// Configuration for the `bootstrap` command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapCommandConfig {
    /// Run `pub get` in parallel
    #[serde(default)]
    pub run_pub_get_in_parallel: Option<bool>,

    /// When true, validate that workspace packages' version constraints on
    /// sibling packages are satisfied by the sibling's actual version before
    /// running `pub get`. This catches constraint mismatches early — important
    /// because published packages won't have the local path overrides.
    #[serde(default)]
    pub enforce_versions_for_dependency_resolution: Option<bool>,

    /// Pass --enforce-lockfile to pub get
    #[serde(default)]
    pub enforce_lockfile: Option<bool>,

    /// Pass --offline to pub get when true
    #[serde(default)]
    pub run_pub_get_offline: Option<bool>,

    /// Additional paths to include as dependency overrides in pubspec_overrides.yaml.
    ///
    /// Each path is resolved relative to the workspace root and scanned for
    /// packages whose names match workspace dependencies. Matched packages are
    /// added as `dependency_overrides` entries alongside sibling packages.
    #[serde(default)]
    pub dependency_override_paths: Option<Vec<String>>,

    /// Shared environment SDK constraints to sync across all packages.
    ///
    /// Example:
    /// ```yaml
    /// environment:
    ///   sdk: ">=3.0.0 <4.0.0"
    ///   flutter: ">=3.0.0 <4.0.0"
    /// ```
    #[serde(default)]
    pub environment: Option<HashMap<String, String>>,

    /// Shared dependencies to sync across all packages.
    ///
    /// If a package lists one of these dependencies, its version constraint
    /// will be updated to match the one defined here during bootstrap.
    #[serde(default)]
    pub dependencies: Option<HashMap<String, yaml_serde::Value>>,

    /// Shared dev_dependencies to sync across all packages.
    ///
    /// Same behavior as `dependencies` but for dev_dependencies.
    #[serde(default)]
    pub dev_dependencies: Option<HashMap<String, yaml_serde::Value>>,

    /// Lifecycle hooks (pre/post)
    #[serde(default)]
    pub hooks: Option<BootstrapHooks>,
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
    /// Script to run before cleaning
    pub pre: Option<String>,

    /// Script to run after cleaning
    pub post: Option<String>,
}

/// Configuration for the `test` command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCommandConfig {
    /// Lifecycle hooks (pre/post)
    pub hooks: Option<TestHooks>,
}

/// Hooks for the test command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestHooks {
    /// Script to run before testing
    pub pre: Option<String>,

    /// Script to run after testing
    pub post: Option<String>,
}

/// Hooks for the bootstrap command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapHooks {
    /// Script to run before bootstrapping
    pub pre: Option<String>,

    /// Script to run after bootstrapping
    pub post: Option<String>,
}

/// Configuration for the `publish` command
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishCommandConfig {
    /// Lifecycle hooks (pre/post)
    #[serde(default)]
    pub hooks: Option<PublishHooks>,
}

/// Hooks for the publish command
///
/// The pre-hook runs before `melos publish` and the post-hook runs after.
/// Both run only once, even if multiple packages are published.
/// The `MELOS_PUBLISH_DRY_RUN` env var is set to `true` or `false` so hooks
/// can detect dry-run mode.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishHooks {
    /// Script to run before publishing
    pub pre: Option<String>,

    /// Script to run after publishing
    pub post: Option<String>,
}

fn default_true_opt() -> Option<bool> {
    Some(true)
}

fn default_true() -> bool {
    true
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
#[serde(rename_all = "camelCase")]
struct MelosSection {
    /// Override workspace name (optional; defaults to pubspec `name`)
    #[serde(default)]
    name: Option<String>,

    /// Package glob patterns (optional in 7.x; falls back to `workspace:` paths)
    #[serde(default)]
    packages: Option<Vec<String>>,

    /// Repository URL or object for changelog commit links
    #[serde(default)]
    repository: Option<RepositoryConfig>,

    /// Custom Dart/Flutter SDK path
    #[serde(default)]
    sdk_path: Option<String>,

    /// Command-level configuration
    #[serde(default)]
    command: Option<CommandConfig>,

    /// Named scripts
    #[serde(default)]
    scripts: HashMap<String, ScriptEntry>,

    /// Global ignore patterns
    #[serde(default)]
    ignore: Option<Vec<String>>,

    /// Category definitions
    #[serde(default)]
    categories: HashMap<String, Vec<String>>,

    /// When true, include the workspace root directory as a package.
    #[serde(default)]
    use_root_as_package: Option<bool>,

    /// When true, recursively discover nested workspaces.
    #[serde(default)]
    discover_nested_workspaces: Option<bool>,
}
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
                repository: wrapper.melos.repository,
                sdk_path: wrapper.melos.sdk_path,
                command: wrapper.melos.command,
                scripts: wrapper.melos.scripts,
                ignore: wrapper.melos.ignore,
                categories: wrapper.melos.categories,
                use_root_as_package: wrapper.melos.use_root_as_package,
                discover_nested_workspaces: wrapper.melos.discover_nested_workspaces,
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
        assert_eq!(analyze.run_command(), Some("flutter analyze ."));
        assert_eq!(analyze.description(), Some("Run analysis"));
        assert!(analyze.env().is_empty());

        let format = &config.scripts["format"];
        assert_eq!(format.run_command(), Some("dart format ."));
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
        assert_eq!(build.run_command(), Some("dart run build_runner build"));

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

    // ── Config validation tests ─────────────────────────────────────────

    #[test]
    fn test_validate_empty_packages() {
        let config = MelosConfig {
            name: "test".to_string(),
            packages: vec![],
            repository: None,
            sdk_path: None,
            command: None,
            scripts: HashMap::new(),
            ignore: None,
            categories: HashMap::new(),
            use_root_as_package: None,
            discover_nested_workspaces: None,
        };
        let warnings = config.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("No package paths configured"));
    }

    #[test]
    fn test_validate_exec_missing_separator() {
        let mut scripts = HashMap::new();
        scripts.insert(
            "test".to_string(),
            ScriptEntry::Simple("melos exec flutter test".to_string()),
        );
        let config = MelosConfig {
            name: "test".to_string(),
            packages: vec!["packages/**".to_string()],
            repository: None,
            sdk_path: None,
            command: None,
            scripts,
            ignore: None,
            categories: HashMap::new(),
            use_root_as_package: None,
            discover_nested_workspaces: None,
        };
        let warnings = config.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no `--` separator"));
    }

    #[test]
    fn test_validate_exec_with_separator_ok() {
        let mut scripts = HashMap::new();
        scripts.insert(
            "test".to_string(),
            ScriptEntry::Simple("melos exec -- flutter test".to_string()),
        );
        let config = MelosConfig {
            name: "test".to_string(),
            packages: vec!["packages/**".to_string()],
            repository: None,
            sdk_path: None,
            command: None,
            scripts,
            ignore: None,
            categories: HashMap::new(),
            use_root_as_package: None,
            discover_nested_workspaces: None,
        };
        let warnings = config.validate();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_validate_empty_run_command() {
        let mut scripts = HashMap::new();
        scripts.insert("empty".to_string(), ScriptEntry::Simple("  ".to_string()));
        let config = MelosConfig {
            name: "test".to_string(),
            packages: vec!["packages/**".to_string()],
            repository: None,
            sdk_path: None,
            command: None,
            scripts,
            ignore: None,
            categories: HashMap::new(),
            use_root_as_package: None,
            discover_nested_workspaces: None,
        };
        let warnings = config.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("empty `run` command"));
    }

    #[test]
    fn test_validate_undefined_category_reference() {
        let mut scripts = HashMap::new();
        scripts.insert(
            "test".to_string(),
            ScriptEntry::Full(Box::new(ScriptConfig {
                run: "flutter test".to_string(),
                exec: None,
                steps: None,
                private: None,
                description: None,
                package_filters: Some(filter::PackageFilters {
                    category: Some(vec!["nonexistent".to_string()]),
                    ..Default::default()
                }),
                env: HashMap::new(),
                groups: None,
            })),
        );
        let config = MelosConfig {
            name: "test".to_string(),
            packages: vec!["packages/**".to_string()],
            repository: None,
            sdk_path: None,
            command: None,
            scripts,
            ignore: None,
            categories: HashMap::new(),
            use_root_as_package: None,
            discover_nested_workspaces: None,
        };
        let warnings = config.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("category 'nonexistent'"));
        assert!(warnings[0].contains("not defined"));
    }

    #[test]
    fn test_validate_valid_config_no_warnings() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
scripts:
  test: flutter test
  build:
    run: melos exec -- dart build
categories:
  apps:
    - app_*
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let warnings = config.validate();
        assert!(
            warnings.is_empty(),
            "Expected no warnings, got: {:?}",
            warnings
        );
    }

    // -----------------------------------------------------------------------
    // RepositoryConfig tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_repository_config_from_url_string() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
repository: https://github.com/invertase/melos
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let repo = config.repository.unwrap();
        assert_eq!(repo.url, "https://github.com/invertase/melos");
    }

    #[test]
    fn test_repository_config_from_object_github() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
repository:
  type: github
  owner: invertase
  name: melos
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let repo = config.repository.unwrap();
        assert_eq!(repo.url, "https://github.com/invertase/melos");
    }

    #[test]
    fn test_repository_config_from_object_gitlab() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
repository:
  type: gitlab
  owner: myorg
  name: myrepo
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let repo = config.repository.unwrap();
        assert_eq!(repo.url, "https://gitlab.com/myorg/myrepo");
    }

    #[test]
    fn test_repository_config_from_object_custom_origin() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
repository:
  type: github
  origin: https://git.internal.io
  owner: team
  name: project
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let repo = config.repository.unwrap();
        assert_eq!(repo.url, "https://git.internal.io/team/project");
    }

    #[test]
    fn test_repository_config_default_type_is_github() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
repository:
  owner: myowner
  name: myrepo
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let repo = config.repository.unwrap();
        assert_eq!(repo.url, "https://github.com/myowner/myrepo");
    }

    #[test]
    fn test_repository_config_commit_url() {
        let repo = RepositoryConfig {
            url: "https://github.com/org/repo".to_string(),
        };
        assert_eq!(
            repo.commit_url("abc1234"),
            "https://github.com/org/repo/commit/abc1234"
        );
    }

    #[test]
    fn test_repository_config_commit_url_trailing_slash() {
        let repo = RepositoryConfig {
            url: "https://github.com/org/repo/".to_string(),
        };
        assert_eq!(
            repo.commit_url("def5678"),
            "https://github.com/org/repo/commit/def5678"
        );
    }

    #[test]
    fn test_repository_config_absent() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        assert!(config.repository.is_none());
    }

    #[test]
    fn test_repository_config_7x_pubspec() {
        // Verify the MelosSection correctly parses the repository field,
        // which is what the 7.x config path uses
        let section: MelosSection = yaml_serde::from_str(
            r#"
repository: https://github.com/myorg/myapp
packages:
  - packages/**
"#,
        )
        .unwrap();
        assert!(section.repository.is_some());
        assert_eq!(
            section.repository.unwrap().url,
            "https://github.com/myorg/myapp"
        );
    }

    // -----------------------------------------------------------------------
    // ScriptEntry method tests (exec config, steps, private)
    // -----------------------------------------------------------------------

    #[test]
    fn test_script_entry_exec_string_shorthand() {
        let yaml = r#"
name: test
packages:
  - packages/**
scripts:
  test:
    exec: "flutter test"
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let entry = &config.scripts["test"];

        // exec_command returns the exec string
        assert_eq!(entry.exec_command(), Some("flutter test"));
        // run_command returns None (no run field with exec shorthand)
        assert!(entry.run_command().is_none());
        // has_exec_config is true
        assert!(entry.has_exec_config());
        // no exec options (it's a string, not an object)
        assert!(entry.exec_options().is_none());
    }

    #[test]
    fn test_script_entry_exec_object_with_run() {
        let yaml = r#"
name: test
packages:
  - packages/**
scripts:
  test:
    run: flutter test
    exec:
      concurrency: 3
      failFast: true
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let entry = &config.scripts["test"];

        // exec_command returns the run field (command comes from run when exec is object)
        assert_eq!(entry.exec_command(), Some("flutter test"));
        // run_command also returns the run field
        assert_eq!(entry.run_command(), Some("flutter test"));
        // has exec config
        assert!(entry.has_exec_config());
        // exec options are available
        let opts = entry.exec_options().unwrap();
        assert_eq!(opts.concurrency, Some(3));
        assert!(opts.fail_fast);
        assert!(!opts.order_dependents);
    }

    #[test]
    fn test_script_entry_steps() {
        let yaml = r#"
name: test
packages:
  - packages/**
scripts:
  check:
    steps:
      - analyze
      - "dart format --set-exit-if-changed ."
      - test:unit
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let entry = &config.scripts["check"];

        let steps = entry.steps().unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0], "analyze");
        assert_eq!(steps[1], "dart format --set-exit-if-changed .");
        assert_eq!(steps[2], "test:unit");
        // run_command returns None for steps-only scripts
        assert!(entry.run_command().is_none());
    }

    #[test]
    fn test_script_entry_private() {
        let yaml = r#"
name: test
packages:
  - packages/**
scripts:
  internal:
    run: echo internal
    private: true
  public:
    run: echo public
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        assert!(config.scripts["internal"].is_private());
        assert!(!config.scripts["public"].is_private());
    }

    #[test]
    fn test_script_entry_simple_is_not_private() {
        let entry = ScriptEntry::Simple("echo hello".to_string());
        assert!(!entry.is_private());
        assert!(entry.steps().is_none());
        assert!(entry.exec_command().is_none());
        assert!(!entry.has_exec_config());
    }

    #[test]
    fn test_validate_steps_no_empty_run_warning() {
        // A script with steps but no run should NOT produce an empty run warning
        let mut scripts = HashMap::new();
        scripts.insert(
            "check".to_string(),
            ScriptEntry::Full(Box::new(ScriptConfig {
                run: String::new(),
                exec: None,
                steps: Some(vec!["analyze".to_string(), "test".to_string()]),
                private: None,
                description: None,
                package_filters: None,
                env: HashMap::new(),
                groups: None,
            })),
        );
        let config = MelosConfig {
            name: "test".to_string(),
            packages: vec!["packages/**".to_string()],
            repository: None,
            sdk_path: None,
            command: None,
            scripts,
            ignore: None,
            categories: HashMap::new(),
            use_root_as_package: None,
            discover_nested_workspaces: None,
        };
        let warnings = config.validate();
        assert!(
            warnings.is_empty(),
            "Expected no warnings, got: {:?}",
            warnings
        );
    }

    #[test]
    fn test_validate_exec_shorthand_no_empty_run_warning() {
        // A script with exec string shorthand but no run should NOT produce a warning
        let mut scripts = HashMap::new();
        scripts.insert(
            "test".to_string(),
            ScriptEntry::Full(Box::new(ScriptConfig {
                run: String::new(),
                exec: Some(script::ExecEntry::Command("flutter test".to_string())),
                steps: None,
                private: None,
                description: None,
                package_filters: None,
                env: HashMap::new(),
                groups: None,
            })),
        );
        let config = MelosConfig {
            name: "test".to_string(),
            packages: vec!["packages/**".to_string()],
            repository: None,
            sdk_path: None,
            command: None,
            scripts,
            ignore: None,
            categories: HashMap::new(),
            use_root_as_package: None,
            discover_nested_workspaces: None,
        };
        let warnings = config.validate();
        assert!(
            warnings.is_empty(),
            "Expected no warnings, got: {:?}",
            warnings
        );
    }

    // -----------------------------------------------------------------------
    // Top-level ignore config tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_top_level_ignore() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
ignore:
  - "*_example"
  - internal_*
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let ignore = config.ignore.unwrap();
        assert_eq!(ignore.len(), 2);
        assert_eq!(ignore[0], "*_example");
        assert_eq!(ignore[1], "internal_*");
    }

    #[test]
    fn test_parse_no_top_level_ignore() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        assert!(config.ignore.is_none());
    }

    // -----------------------------------------------------------------------
    // Publish hooks config tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_publish_hooks() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  publish:
    hooks:
      pre: dart pub run build_runner build
      post: dart pub run build_runner clean
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let publish = config.command.unwrap().publish.unwrap();
        let hooks = publish.hooks.unwrap();
        assert_eq!(
            hooks.pre.as_deref(),
            Some("dart pub run build_runner build")
        );
        assert_eq!(
            hooks.post.as_deref(),
            Some("dart pub run build_runner clean")
        );
    }

    #[test]
    fn test_parse_publish_hooks_pre_only() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  publish:
    hooks:
      pre: echo before
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let publish = config.command.unwrap().publish.unwrap();
        let hooks = publish.hooks.unwrap();
        assert_eq!(hooks.pre.as_deref(), Some("echo before"));
        assert!(hooks.post.is_none());
    }

    #[test]
    fn test_parse_no_publish_config() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    branch: main
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        assert!(config.command.unwrap().publish.is_none());
    }

    // -----------------------------------------------------------------------
    // SDK path config tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_sdk_path() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
sdkPath: /opt/flutter/sdk
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(config.sdk_path.as_deref(), Some("/opt/flutter/sdk"));
    }

    #[test]
    fn test_parse_no_sdk_path() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        assert!(config.sdk_path.is_none());
    }

    #[test]
    fn test_parse_7x_sdk_path() {
        let yaml = r#"
name: my_workspace
workspace:
  - packages/core
melos:
  sdkPath: /usr/local/flutter
"#;
        let wrapper: PubspecWithMelos = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(
            wrapper.melos.sdk_path.as_deref(),
            Some("/usr/local/flutter")
        );
    }

    // -----------------------------------------------------------------------
    // Script groups tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_script_groups() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
scripts:
  analyze:
    run: dart analyze .
    groups:
      - ci
      - quality
  test:
    run: flutter test
    groups:
      - ci
  format: dart format .
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();

        let analyze = &config.scripts["analyze"];
        assert_eq!(
            analyze.groups(),
            Some(vec!["ci".to_string(), "quality".to_string()].as_slice())
        );
        assert!(analyze.in_group("ci"));
        assert!(analyze.in_group("quality"));
        assert!(!analyze.in_group("build"));

        let test = &config.scripts["test"];
        assert_eq!(test.groups(), Some(vec!["ci".to_string()].as_slice()));
        assert!(test.in_group("ci"));

        let format = &config.scripts["format"];
        assert!(format.groups().is_none());
        assert!(!format.in_group("ci"));
    }

    #[test]
    fn test_script_entry_simple_has_no_groups() {
        let entry = ScriptEntry::Simple("echo hello".to_string());
        assert!(entry.groups().is_none());
        assert!(!entry.in_group("anything"));
    }

    // -----------------------------------------------------------------------
    // Script override: overridable command names
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_all_command_configs_together() {
        // Ensure all command configs can coexist
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    branch: main
  bootstrap:
    runPubGetInParallel: true
  clean:
    hooks:
      pre: echo pre-clean
  publish:
    hooks:
      pre: echo pre-publish
      post: echo post-publish
"#;
        let config: MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let cmd = config.command.unwrap();
        assert!(cmd.version.is_some());
        assert!(cmd.bootstrap.is_some());
        assert!(cmd.clean.is_some());
        assert!(cmd.publish.is_some());
        let publish = cmd.publish.unwrap();
        let hooks = publish.hooks.unwrap();
        assert_eq!(hooks.pre.as_deref(), Some("echo pre-publish"));
        assert_eq!(hooks.post.as_deref(), Some("echo post-publish"));
    }
}
