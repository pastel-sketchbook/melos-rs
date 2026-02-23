use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{self, ConfigSource, MelosConfig};
use crate::package::{self, Package};

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

    /// Effective SDK path (resolved from CLI > env var > config, if any)
    pub sdk_path: Option<String>,

    /// Warnings collected during workspace loading (config validation, nested
    /// workspace discovery, useRootAsPackage issues). The caller is responsible
    /// for presenting these to the user.
    pub warnings: Vec<String>,
}

impl Workspace {
    /// Find melos.yaml or pubspec.yaml (with melos: key) by walking up from the
    /// current directory, then load the workspace.
    ///
    /// Priority: melos.yaml is preferred over pubspec.yaml (the user hasn't
    /// migrated to 7.x yet).
    ///
    /// `sdk_path_override` is the CLI `--sdk-path` value, which takes highest priority.
    pub fn find_and_load(sdk_path_override: Option<&str>) -> Result<Self> {
        let config_source = find_config()?;
        let root_path = config_source
            .path()
            .parent()
            .context("Config file has no parent directory")?
            .to_path_buf();

        let config = config::parse_config(&config_source)?;

        // Run post-parse validation and collect warnings
        let mut warnings = config.validate();

        let mut packages = package::discover_packages(&root_path, &config.packages)?;

        // Discover packages from nested workspaces if enabled
        if config.discover_nested_workspaces == Some(true) {
            let nested = discover_nested_workspace_packages(&root_path, &packages, &mut warnings)?;
            if !nested.is_empty() {
                let new_count = nested.len();
                for pkg in nested {
                    if !packages.iter().any(|p| p.name == pkg.name) {
                        packages.push(pkg);
                    }
                }
                // Re-sort after adding nested packages
                packages.sort_by(|a, b| a.name.cmp(&b.name));
                warnings.push(format!(
                    "Discovered {} package(s) from nested workspaces",
                    new_count
                ));
            }
        }

        // If useRootAsPackage is enabled, include the workspace root as a package
        if config.use_root_as_package == Some(true) {
            let pubspec_path = root_path.join("pubspec.yaml");
            if pubspec_path.exists() {
                match Package::from_path(&root_path) {
                    Ok(root_pkg) => {
                        // Only add if not already discovered (avoid duplicates)
                        if !packages.iter().any(|p| p.path == root_path) {
                            packages.push(root_pkg);
                        }
                    }
                    Err(e) => {
                        warnings.push(format!(
                            "useRootAsPackage is enabled but root pubspec.yaml could not be parsed: {}",
                            e
                        ));
                    }
                }
            } else {
                warnings.push(
                    "useRootAsPackage is enabled but no pubspec.yaml found at workspace root"
                        .to_string(),
                );
            }
        }

        // Apply top-level ignore patterns (global exclusion before any command-level filters)
        if let Some(ref ignore_patterns) = config.ignore {
            packages.retain(|pkg| {
                !ignore_patterns.iter().any(|pattern| {
                    glob::Pattern::new(pattern)
                        .map(|p| p.matches(&pkg.name))
                        .unwrap_or_else(|_| pkg.name.contains(pattern))
                })
            });
        }

        // Resolve SDK path: CLI flag > MELOS_SDK_PATH env var > config sdkPath
        let sdk_path = sdk_path_override
            .map(|s| s.to_string())
            .or_else(|| std::env::var("MELOS_SDK_PATH").ok())
            .or_else(|| config.sdk_path.clone());

        Ok(Workspace {
            root_path,
            config_source,
            config,
            packages,
            sdk_path,
            warnings,
        })
    }

    /// Extract a lifecycle hook command for a given command and phase.
    ///
    /// `command` is one of `"bootstrap"`, `"build"`, `"clean"`, `"test"`, `"publish"`.
    /// `phase` is `"pre"` or `"post"`.
    ///
    /// Returns `None` if no hook is configured for the given command/phase.
    ///
    /// This centralises the four-level `Option` chain
    /// (`config.command → command_config → hooks → phase`) that was previously
    /// duplicated at every hook call-site.
    pub fn hook(&self, command: &str, phase: &str) -> Option<&str> {
        let cmd = self.config.command.as_ref()?;
        match command {
            "bootstrap" => {
                let h = cmd.bootstrap.as_ref()?.hooks.as_ref()?;
                match phase {
                    "pre" => h.pre.as_deref(),
                    "post" => h.post.as_deref(),
                    _ => None,
                }
            }
            "build" => {
                let h = cmd.build.as_ref()?.hooks.as_ref()?;
                match phase {
                    "pre" => h.pre.as_deref(),
                    "post" => h.post.as_deref(),
                    _ => None,
                }
            }
            "clean" => {
                let h = cmd.clean.as_ref()?.hooks.as_ref()?;
                match phase {
                    "pre" => h.pre.as_deref(),
                    "post" => h.post.as_deref(),
                    _ => None,
                }
            }
            "test" => {
                let h = cmd.test.as_ref()?.hooks.as_ref()?;
                match phase {
                    "pre" => h.pre.as_deref(),
                    "post" => h.post.as_deref(),
                    _ => None,
                }
            }
            "publish" => {
                let h = cmd.publish.as_ref()?.hooks.as_ref()?;
                match phase {
                    "pre" => h.pre.as_deref(),
                    "post" => h.post.as_deref(),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Build environment variables that are available to scripts and commands
    ///
    /// Melos provides these env vars:
    ///   MELOS_ROOT_PATH - absolute path to the workspace root
    ///   MELOS_SDK_PATH  - custom SDK path (if configured)
    ///   MELOS_PACKAGE_NAME - (set per-package during exec)
    ///   MELOS_PACKAGE_PATH - (set per-package during exec)
    ///   MELOS_PACKAGE_VERSION - (set per-package during exec)
    ///
    /// When `sdk_path` is set, `{sdk_path}/bin` is prepended to `PATH` so that
    /// `dart` and `flutter` executables from that SDK are found by child processes.
    pub fn env_vars(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert(
            "MELOS_ROOT_PATH".to_string(),
            self.root_path.display().to_string(),
        );
        if let Some(ref sdk_path) = self.sdk_path {
            env.insert("MELOS_SDK_PATH".to_string(), sdk_path.clone());

            // Prepend sdk_path/bin to PATH so child processes find dart/flutter
            let sdk_bin = std::path::Path::new(sdk_path).join("bin");
            let separator = if cfg!(target_os = "windows") {
                ";"
            } else {
                ":"
            };
            let new_path = match std::env::var("PATH") {
                Ok(current) => format!("{}{}{}", sdk_bin.display(), separator, current),
                Err(_) => sdk_bin.display().to_string(),
            };
            env.insert("PATH".to_string(), new_path);
        }
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

/// Discover packages from nested Dart workspaces.
///
/// Scans each already-discovered package's `pubspec.yaml` for a `workspace:` field.
/// If found, treats it as a nested workspace root and discovers packages from the
/// listed workspace paths. This is recursive — nested workspaces can themselves
/// contain nested workspaces.
fn discover_nested_workspace_packages(
    _root_path: &Path,
    existing_packages: &[Package],
    warnings: &mut Vec<String>,
) -> Result<Vec<Package>> {
    let mut nested_packages = Vec::new();
    let mut visited_roots = std::collections::HashSet::new();

    // Collect paths from existing packages that might be nested workspace roots
    let mut candidates: Vec<PathBuf> = existing_packages.iter().map(|p| p.path.clone()).collect();

    while let Some(pkg_path) = candidates.pop() {
        if visited_roots.contains(&pkg_path) {
            continue;
        }
        visited_roots.insert(pkg_path.clone());

        let pubspec_path = pkg_path.join("pubspec.yaml");
        if !pubspec_path.exists() {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&pubspec_path) else {
            continue;
        };

        let Ok(value) = yaml_serde::from_str::<yaml_serde::Value>(&content) else {
            continue;
        };

        // Check for a `workspace:` field listing nested package paths
        let workspace_paths = value
            .as_mapping()
            .and_then(|m| m.get(yaml_serde::Value::String("workspace".to_string())))
            .and_then(|v| v.as_sequence());

        let Some(ws_paths) = workspace_paths else {
            continue;
        };

        for ws_path_value in ws_paths {
            let Some(ws_path_str) = ws_path_value.as_str() else {
                continue;
            };

            let nested_dir = pkg_path.join(ws_path_str);
            if !nested_dir.is_dir() {
                continue;
            }

            let nested_pubspec = nested_dir.join("pubspec.yaml");
            if nested_pubspec.exists() {
                match Package::from_path(&nested_dir) {
                    Ok(pkg) => {
                        // Add as a candidate for further nested workspace discovery
                        candidates.push(pkg.path.clone());
                        nested_packages.push(pkg);
                    }
                    Err(e) => {
                        warnings.push(format!(
                            "Failed to parse nested workspace package at {}: {}",
                            nested_dir.display(),
                            e
                        ));
                    }
                }
            }
        }
    }

    Ok(nested_packages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    use crate::config::{
        BootstrapCommandConfig, BootstrapHooks, CleanCommandConfig, CleanHooks, CommandConfig,
        MelosConfig, PublishCommandConfig, PublishHooks, TestCommandConfig, TestHooks,
    };

    /// Helper to build a minimal workspace with optional command config.
    fn make_workspace_with_commands(command: Option<CommandConfig>) -> Workspace {
        Workspace {
            root_path: PathBuf::from("/workspace"),
            config_source: ConfigSource::MelosYaml(PathBuf::from("/workspace/melos.yaml")),
            config: MelosConfig {
                name: "test".to_string(),
                packages: vec!["packages/**".to_string()],
                repository: None,
                sdk_path: None,
                command,
                scripts: HashMap::new(),
                ignore: None,
                categories: HashMap::new(),
                use_root_as_package: None,
                discover_nested_workspaces: None,
            },
            packages: vec![],
            sdk_path: None,
            warnings: vec![],
        }
    }

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

    #[test]
    fn test_discover_nested_workspace_packages_finds_nested() {
        let dir = TempDir::new().unwrap();

        // Create a "host" package with a workspace: field
        let host_dir = dir.path().join("host_pkg");
        fs::create_dir_all(&host_dir).unwrap();
        fs::write(
            host_dir.join("pubspec.yaml"),
            "name: host_pkg\nversion: 1.0.0\nworkspace:\n  - sub_a\n  - sub_b\n",
        )
        .unwrap();

        // Create nested packages
        let sub_a = host_dir.join("sub_a");
        fs::create_dir_all(&sub_a).unwrap();
        fs::write(sub_a.join("pubspec.yaml"), "name: sub_a\nversion: 0.1.0\n").unwrap();

        let sub_b = host_dir.join("sub_b");
        fs::create_dir_all(&sub_b).unwrap();
        fs::write(sub_b.join("pubspec.yaml"), "name: sub_b\nversion: 0.2.0\n").unwrap();

        let host_pkg = Package::from_path(&host_dir).unwrap();
        let mut warnings = vec![];
        let nested =
            discover_nested_workspace_packages(dir.path(), &[host_pkg], &mut warnings).unwrap();

        assert_eq!(nested.len(), 2);
        let names: Vec<&str> = nested.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"sub_a"));
        assert!(names.contains(&"sub_b"));
    }

    #[test]
    fn test_discover_nested_workspace_packages_no_workspace_field() {
        let dir = TempDir::new().unwrap();

        // Create a regular package without workspace: field
        let pkg_dir = dir.path().join("regular_pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: regular_pkg\nversion: 1.0.0\n",
        )
        .unwrap();

        let pkg = Package::from_path(&pkg_dir).unwrap();
        let mut warnings = vec![];
        let nested = discover_nested_workspace_packages(dir.path(), &[pkg], &mut warnings).unwrap();
        assert!(nested.is_empty());
    }

    #[test]
    fn test_discover_nested_workspace_packages_missing_nested_dir() {
        let dir = TempDir::new().unwrap();

        // Create a package with workspace: field pointing to non-existent directory
        let pkg_dir = dir.path().join("host_pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("pubspec.yaml"),
            "name: host_pkg\nversion: 1.0.0\nworkspace:\n  - nonexistent\n",
        )
        .unwrap();

        let pkg = Package::from_path(&pkg_dir).unwrap();
        let mut warnings = vec![];
        let nested = discover_nested_workspace_packages(dir.path(), &[pkg], &mut warnings).unwrap();
        assert!(nested.is_empty());
    }

    // -----------------------------------------------------------------------
    // Workspace::hook() tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_hook_no_command_config() {
        let ws = make_workspace_with_commands(None);
        assert!(ws.hook("bootstrap", "pre").is_none());
        assert!(ws.hook("clean", "post").is_none());
    }

    #[test]
    fn test_hook_bootstrap_pre() {
        let ws = make_workspace_with_commands(Some(CommandConfig {
            version: None,
            bootstrap: Some(BootstrapCommandConfig {
                run_pub_get_in_parallel: None,
                enforce_versions_for_dependency_resolution: None,
                enforce_lockfile: None,
                run_pub_get_offline: None,
                dependency_override_paths: None,
                environment: None,
                dependencies: None,
                dev_dependencies: None,
                hooks: Some(BootstrapHooks {
                    pre: Some("echo pre-bootstrap".to_string()),
                    post: None,
                }),
            }),
            build: None,
            clean: None,
            publish: None,
            test: None,
        }));
        assert_eq!(ws.hook("bootstrap", "pre"), Some("echo pre-bootstrap"));
        assert!(ws.hook("bootstrap", "post").is_none());
    }

    #[test]
    fn test_hook_clean_post() {
        let ws = make_workspace_with_commands(Some(CommandConfig {
            version: None,
            bootstrap: None,
            build: None,
            clean: Some(CleanCommandConfig {
                hooks: Some(CleanHooks {
                    pre: None,
                    post: Some("echo post-clean".to_string()),
                }),
            }),
            publish: None,
            test: None,
        }));
        assert!(ws.hook("clean", "pre").is_none());
        assert_eq!(ws.hook("clean", "post"), Some("echo post-clean"));
    }

    #[test]
    fn test_hook_test_both() {
        let ws = make_workspace_with_commands(Some(CommandConfig {
            version: None,
            bootstrap: None,
            build: None,
            clean: None,
            publish: None,
            test: Some(TestCommandConfig {
                hooks: Some(TestHooks {
                    pre: Some("echo pre-test".to_string()),
                    post: Some("echo post-test".to_string()),
                }),
            }),
        }));
        assert_eq!(ws.hook("test", "pre"), Some("echo pre-test"));
        assert_eq!(ws.hook("test", "post"), Some("echo post-test"));
    }

    #[test]
    fn test_hook_publish_pre() {
        let ws = make_workspace_with_commands(Some(CommandConfig {
            version: None,
            bootstrap: None,
            build: None,
            clean: None,
            publish: Some(PublishCommandConfig {
                hooks: Some(PublishHooks {
                    pre: Some("echo pre-publish".to_string()),
                    post: None,
                }),
            }),
            test: None,
        }));
        assert_eq!(ws.hook("publish", "pre"), Some("echo pre-publish"));
        assert!(ws.hook("publish", "post").is_none());
    }

    #[test]
    fn test_hook_unknown_command() {
        let ws = make_workspace_with_commands(Some(CommandConfig {
            version: None,
            bootstrap: None,
            build: None,
            clean: None,
            publish: None,
            test: None,
        }));
        assert!(ws.hook("unknown", "pre").is_none());
    }

    #[test]
    fn test_hook_unknown_phase() {
        let ws = make_workspace_with_commands(Some(CommandConfig {
            version: None,
            bootstrap: None,
            build: None,
            clean: Some(CleanCommandConfig {
                hooks: Some(CleanHooks {
                    pre: Some("echo pre".to_string()),
                    post: Some("echo post".to_string()),
                }),
            }),
            publish: None,
            test: None,
        }));
        assert!(ws.hook("clean", "during").is_none());
    }

    #[test]
    fn test_hook_no_hooks_configured() {
        let ws = make_workspace_with_commands(Some(CommandConfig {
            version: None,
            bootstrap: None,
            build: None,
            clean: Some(CleanCommandConfig { hooks: None }),
            publish: None,
            test: None,
        }));
        assert!(ws.hook("clean", "pre").is_none());
        assert!(ws.hook("clean", "post").is_none());
    }

    // -----------------------------------------------------------------------
    // Workspace::env_vars() tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_env_vars_without_sdk_path() {
        let ws = make_workspace_with_commands(None);
        let env = ws.env_vars();
        assert_eq!(env.get("MELOS_ROOT_PATH").unwrap(), "/workspace");
        assert!(!env.contains_key("MELOS_SDK_PATH"));
        assert!(!env.contains_key("PATH"));
    }

    #[test]
    fn test_env_vars_with_sdk_path_prepends_bin_to_path() {
        let mut ws = make_workspace_with_commands(None);
        ws.sdk_path = Some("/opt/flutter".to_string());

        let env = ws.env_vars();
        assert_eq!(env.get("MELOS_SDK_PATH").unwrap(), "/opt/flutter");

        let path = env
            .get("PATH")
            .expect("PATH should be set when sdk_path is configured");
        assert!(
            path.starts_with("/opt/flutter/bin"),
            "PATH should start with sdk_path/bin, got: {path}"
        );
    }
}
