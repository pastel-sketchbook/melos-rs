use std::time::Instant;

use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use crate::events::Event;
use crate::package::Package;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

use super::PackageResults;

/// Options for the clean command (clap-free).
#[derive(Debug, Clone)]
pub struct CleanOpts {
    pub concurrency: usize,
}

/// Paths removed during a deep clean.
pub const DEEP_CLEAN_DIRS: &[&str] = &[".dart_tool", "build"];

/// Files removed during a deep clean.
pub const DEEP_CLEAN_FILES: &[&str] = &["pubspec.lock"];

/// Run clean across packages.
///
/// Flutter packages are cleaned via `flutter clean` through the [`ProcessRunner`].
/// Dart packages are cleaned by removing `build/` and `.dart_tool/` directories,
/// emitting events manually for each package.
///
/// Returns combined [`PackageResults`] from both Flutter and Dart operations.
pub async fn run(
    packages: &[Package],
    workspace: &Workspace,
    opts: &CleanOpts,
    events: Option<&UnboundedSender<Event>>,
) -> Result<PackageResults> {
    let flutter_pkgs: Vec<_> = packages.iter().filter(|p| p.is_flutter).cloned().collect();
    let dart_pkgs: Vec<_> = packages.iter().filter(|p| !p.is_flutter).cloned().collect();

    let mut all_results = Vec::new();

    // Flutter packages: run `flutter clean` via ProcessRunner.
    if !flutter_pkgs.is_empty() {
        if let Some(tx) = events {
            let _ = tx.send(Event::Progress {
                completed: 0,
                total: flutter_pkgs.len(),
                message: "flutter clean...".into(),
            });
        }
        let runner = ProcessRunner::new(opts.concurrency, false);
        let results = runner
            .run_in_packages_with_events(
                &flutter_pkgs,
                "flutter clean",
                &workspace.env_vars(),
                None,
                events,
                &workspace.packages,
            )
            .await?;
        all_results.extend(results);
    }

    // Dart packages: remove build artifacts manually.
    if !dart_pkgs.is_empty() {
        if let Some(tx) = events {
            let _ = tx.send(Event::Progress {
                completed: 0,
                total: dart_pkgs.len(),
                message: "cleaning dart packages...".into(),
            });
        }
        for (i, pkg) in dart_pkgs.iter().enumerate() {
            let start = Instant::now();
            if let Some(tx) = events {
                let _ = tx.send(Event::PackageStarted {
                    name: pkg.name.clone(),
                });
            }

            let mut success = true;
            for dir_name in DEEP_CLEAN_DIRS {
                let dir = pkg.path.join(dir_name);
                if dir.exists() {
                    match std::fs::remove_dir_all(&dir) {
                        Ok(()) => {
                            if let Some(tx) = events {
                                let _ = tx.send(Event::PackageOutput {
                                    name: pkg.name.clone(),
                                    line: format!("removed {dir_name}/"),
                                    is_stderr: false,
                                });
                            }
                        }
                        Err(e) => {
                            success = false;
                            if let Some(tx) = events {
                                let _ = tx.send(Event::PackageOutput {
                                    name: pkg.name.clone(),
                                    line: format!("failed to remove {dir_name}/: {e}"),
                                    is_stderr: true,
                                });
                            }
                        }
                    }
                }
            }

            let duration = start.elapsed();
            if let Some(tx) = events {
                let _ = tx.send(Event::PackageFinished {
                    name: pkg.name.clone(),
                    success,
                    duration,
                });
                let _ = tx.send(Event::Progress {
                    completed: i + 1,
                    total: dart_pkgs.len(),
                    message: "cleaning dart packages...".into(),
                });
            }
            all_results.push((pkg.name.clone(), success));
        }
    }

    Ok(PackageResults::from(all_results))
}

/// Result of attempting to remove a `pubspec_overrides.yaml` from a single package.
#[derive(Debug, Clone, PartialEq)]
pub enum OverrideRemoval {
    /// File was successfully removed.
    Removed,
    /// File existed but removal failed with the given error message.
    Failed(String),
    /// File did not exist (no action taken).
    NotPresent,
}

/// Remove `pubspec_overrides.yaml` files from packages (Melos 6.x mode).
///
/// Returns per-package results indicating what happened.
pub fn remove_pubspec_overrides(packages: &[Package]) -> Vec<(String, OverrideRemoval)> {
    packages
        .iter()
        .map(|pkg| {
            let override_path = pkg.path.join("pubspec_overrides.yaml");
            let result = if override_path.exists() {
                match std::fs::remove_file(&override_path) {
                    Ok(()) => OverrideRemoval::Removed,
                    Err(e) => OverrideRemoval::Failed(e.to_string()),
                }
            } else {
                OverrideRemoval::NotPresent
            };
            (pkg.name.clone(), result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_package(name: &str, path: PathBuf) -> Package {
        Package {
            name: name.to_string(),
            path,
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        }
    }

    #[test]
    fn test_deep_clean_dirs_constant() {
        assert!(DEEP_CLEAN_DIRS.contains(&".dart_tool"));
        assert!(DEEP_CLEAN_DIRS.contains(&"build"));
        assert_eq!(DEEP_CLEAN_DIRS.len(), 2);
    }

    #[test]
    fn test_deep_clean_files_constant() {
        assert!(DEEP_CLEAN_FILES.contains(&"pubspec.lock"));
        assert_eq!(DEEP_CLEAN_FILES.len(), 1);
    }

    #[test]
    fn test_remove_pubspec_overrides_removes_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        // Create the override file
        let override_path = pkg_dir.join("pubspec_overrides.yaml");
        std::fs::write(
            &override_path,
            "dependency_overrides:\n  core:\n    path: ../core\n",
        )
        .unwrap();
        assert!(override_path.exists());

        let pkg = make_package("app", pkg_dir);
        let results = remove_pubspec_overrides(&[pkg]);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, OverrideRemoval::Removed);
        assert!(
            !override_path.exists(),
            "pubspec_overrides.yaml should be removed"
        );
    }

    #[test]
    fn test_remove_pubspec_overrides_no_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let pkg = make_package("app", pkg_dir);
        let results = remove_pubspec_overrides(&[pkg]);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, OverrideRemoval::NotPresent);
    }

    #[test]
    fn test_remove_pubspec_overrides_multiple_packages() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_a_dir = dir.path().join("packages").join("a");
        let pkg_b_dir = dir.path().join("packages").join("b");
        std::fs::create_dir_all(&pkg_a_dir).unwrap();
        std::fs::create_dir_all(&pkg_b_dir).unwrap();

        // Only pkg_a has an override file
        let override_a = pkg_a_dir.join("pubspec_overrides.yaml");
        std::fs::write(&override_a, "# overrides").unwrap();

        let pkg_a = make_package("a", pkg_a_dir);
        let pkg_b = make_package("b", pkg_b_dir.clone());

        let results = remove_pubspec_overrides(&[pkg_a, pkg_b]);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, OverrideRemoval::Removed);
        assert_eq!(results[1].1, OverrideRemoval::NotPresent);
        assert!(!override_a.exists(), "a's override should be removed");
        assert!(
            !pkg_b_dir.join("pubspec_overrides.yaml").exists(),
            "b never had one"
        );
    }

    #[test]
    fn test_remove_pubspec_overrides_empty_packages() {
        let results = remove_pubspec_overrides(&[]);
        assert!(results.is_empty());
    }

    #[test]
    fn test_clean_opts_struct_construction() {
        let opts = CleanOpts { concurrency: 8 };
        assert_eq!(opts.concurrency, 8);
    }

    #[tokio::test]
    async fn test_clean_dart_removes_build_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(pkg_dir.join("build")).unwrap();
        std::fs::create_dir_all(pkg_dir.join(".dart_tool")).unwrap();

        let pkg = make_package("app", pkg_dir.clone());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let workspace = crate::workspace::Workspace {
            root_path: dir.path().to_path_buf(),
            config_source: crate::config::ConfigSource::MelosYaml(dir.path().join("melos.yaml")),
            config: crate::config::MelosConfig {
                name: "test".to_string(),
                packages: vec!["packages/**".to_string()],
                repository: None,
                sdk_path: None,
                command: None,
                scripts: HashMap::new(),
                ignore: None,
                categories: HashMap::new(),
                use_root_as_package: None,
                discover_nested_workspaces: None,
            },
            packages: vec![pkg.clone()],
            sdk_path: None,
            warnings: vec![],
        };

        let opts = CleanOpts { concurrency: 1 };
        let results = run(&[pkg], &workspace, &opts, Some(&tx)).await.unwrap();

        assert_eq!(results.results.len(), 1);
        assert!(results.results[0].1, "dart clean should succeed");
        assert!(!pkg_dir.join("build").exists(), "build/ should be removed");
        assert!(
            !pkg_dir.join(".dart_tool").exists(),
            ".dart_tool/ should be removed"
        );

        // Verify events were emitted.
        drop(tx);
        let mut event_count = 0;
        while rx.recv().await.is_some() {
            event_count += 1;
        }
        assert!(event_count > 0, "should emit at least one event");
    }
}
