use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use colored::Colorize;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use tokio::sync::mpsc;

use crate::package::Package;

/// Default debounce duration for file change events.
const DEFAULT_DEBOUNCE_MS: u64 = 500;

/// File extensions that trigger a re-run in Dart/Flutter projects.
const WATCHED_EXTENSIONS: &[&str] = &["dart", "yaml", "json", "arb", "g.dart"];

/// Directories to ignore when watching for changes.
const IGNORED_DIRS: &[&str] = &[
    ".dart_tool",
    "build",
    ".symlinks",
    ".plugin_symlinks",
    "ios/Pods",
    ".fvm",
    ".idea",
    ".vscode",
];

/// A file change event identifying which package was affected.
#[derive(Debug, Clone)]
pub struct PackageChangeEvent {
    /// Name of the package where the change was detected
    pub package_name: String,
}

/// Watch package directories for file changes, emitting debounced events.
///
/// This function blocks the current async task until the watcher is stopped
/// (e.g. by dropping the `shutdown_tx` sender or pressing Ctrl+C).
///
/// # Arguments
/// * `packages` - Packages to watch (their `path` directories are monitored recursively)
/// * `debounce_ms` - Debounce duration in milliseconds (0 uses the default 500ms)
/// * `event_tx` - Channel sender for emitting change events
/// * `shutdown_rx` - Receiver that signals the watcher to stop
pub fn start_watching(
    packages: &[Package],
    debounce_ms: u64,
    event_tx: mpsc::UnboundedSender<PackageChangeEvent>,
    mut shutdown_rx: mpsc::Receiver<()>,
) -> Result<()> {
    let debounce_duration = if debounce_ms == 0 {
        Duration::from_millis(DEFAULT_DEBOUNCE_MS)
    } else {
        Duration::from_millis(debounce_ms)
    };

    // Build a map from watched path -> package name for quick lookup
    let package_paths: Vec<(PathBuf, String)> = packages
        .iter()
        .map(|p| {
            let canonical = p.path.canonicalize().unwrap_or_else(|_| p.path.clone());
            (canonical, p.name.clone())
        })
        .collect();

    // Create the debounced file watcher
    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer =
        new_debouncer(debounce_duration, tx).context("Failed to create file watcher")?;

    // Watch each package directory recursively
    for pkg in packages {
        debouncer
            .watcher()
            .watch(&pkg.path, notify::RecursiveMode::Recursive)
            .with_context(|| format!("Failed to watch directory: {}", pkg.path.display()))?;
    }

    println!(
        "\n{} Watching {} package(s) for changes... (press {} to stop)\n",
        "👀".cyan(),
        packages.len().to_string().cyan(),
        "Ctrl+C".bold(),
    );

    // Process events in a loop until shutdown
    loop {
        // Check for shutdown signal (non-blocking)
        // try_recv returns Ok(()) if a message was sent, or Err(Disconnected) if sender dropped
        match shutdown_rx.try_recv() {
            Ok(()) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
        }

        // Wait for debounced events with a timeout so we can check shutdown
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(events)) => {
                // Collect affected packages, filtering out ignored paths and extensions
                let mut affected_packages: HashSet<String> = HashSet::new();

                for event in events {
                    if event.kind != DebouncedEventKind::Any {
                        continue;
                    }

                    let path = &event.path;

                    // Skip ignored directories
                    if should_ignore_path(path) {
                        continue;
                    }

                    // Skip files without watched extensions
                    if !has_watched_extension(path) {
                        continue;
                    }

                    // Find which package this file belongs to
                    if let Some(pkg_name) = find_owning_package(path, &package_paths) {
                        affected_packages.insert(pkg_name);
                    }
                }

                // Emit events for each affected package
                for package_name in affected_packages {
                    let event = PackageChangeEvent { package_name };
                    if event_tx.send(event).is_err() {
                        // Receiver dropped, stop watching
                        break;
                    }
                }
            }
            Ok(Err(error)) => {
                eprintln!("{} Watch error: {}", "WARN".yellow(), error);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // No events, continue loop
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // Watcher was dropped
                break;
            }
        }
    }

    Ok(())
}

/// Check if a file path should be ignored (build artifacts, IDE files, etc.)
fn should_ignore_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    IGNORED_DIRS.iter().any(|dir| path_str.contains(dir))
}

/// Check if a file has one of the watched extensions.
fn has_watched_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| WATCHED_EXTENSIONS.contains(&ext))
        .unwrap_or(false)
}

/// Find which package owns a given file path by checking if the file
/// is under any of the watched package directories.
fn find_owning_package(file_path: &Path, package_paths: &[(PathBuf, String)]) -> Option<String> {
    let canonical = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());

    // Find the most specific (longest path) matching package
    package_paths
        .iter()
        .filter(|(pkg_path, _)| canonical.starts_with(pkg_path))
        .max_by_key(|(pkg_path, _)| pkg_path.as_os_str().len())
        .map(|(_, name)| name.clone())
}

/// Format a set of changed package names for display.
pub fn format_changed_packages(names: &HashSet<String>) -> String {
    let mut sorted: Vec<_> = names.iter().collect();
    sorted.sort();
    sorted
        .iter()
        .map(|n| n.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_should_ignore_path_dart_tool() {
        assert!(should_ignore_path(Path::new(
            "/workspace/packages/foo/.dart_tool/package_config.json"
        )));
    }

    #[test]
    fn test_should_ignore_path_build() {
        assert!(should_ignore_path(Path::new(
            "/workspace/packages/foo/build/outputs/app.apk"
        )));
    }

    #[test]
    fn test_should_ignore_path_normal() {
        assert!(!should_ignore_path(Path::new(
            "/workspace/packages/foo/lib/main.dart"
        )));
    }

    #[test]
    fn test_should_ignore_path_ide() {
        assert!(should_ignore_path(Path::new(
            "/workspace/packages/foo/.idea/workspace.xml"
        )));
        assert!(should_ignore_path(Path::new(
            "/workspace/packages/foo/.vscode/settings.json"
        )));
    }

    #[test]
    fn test_has_watched_extension_dart() {
        assert!(has_watched_extension(Path::new("lib/main.dart")));
    }

    #[test]
    fn test_has_watched_extension_yaml() {
        assert!(has_watched_extension(Path::new("pubspec.yaml")));
    }

    #[test]
    fn test_has_watched_extension_json() {
        assert!(has_watched_extension(Path::new("analysis_options.json")));
    }

    #[test]
    fn test_has_watched_extension_arb() {
        assert!(has_watched_extension(Path::new("lib/l10n/app_en.arb")));
    }

    #[test]
    fn test_has_watched_extension_unknown() {
        assert!(!has_watched_extension(Path::new("README.md")));
        assert!(!has_watched_extension(Path::new("Makefile")));
    }

    #[test]
    fn test_has_watched_extension_no_ext() {
        assert!(!has_watched_extension(Path::new("Dockerfile")));
    }

    #[test]
    fn test_find_owning_package_basic() {
        let packages = vec![
            (PathBuf::from("/workspace/packages/foo"), "foo".to_string()),
            (PathBuf::from("/workspace/packages/bar"), "bar".to_string()),
        ];

        assert_eq!(
            find_owning_package(
                Path::new("/workspace/packages/foo/lib/main.dart"),
                &packages
            ),
            Some("foo".to_string())
        );
        assert_eq!(
            find_owning_package(
                Path::new("/workspace/packages/bar/test/test.dart"),
                &packages
            ),
            Some("bar".to_string())
        );
    }

    #[test]
    fn test_find_owning_package_nested() {
        // When packages are nested, the most specific (longest) path wins
        let packages = vec![
            (PathBuf::from("/workspace/packages"), "root".to_string()),
            (PathBuf::from("/workspace/packages/foo"), "foo".to_string()),
        ];

        assert_eq!(
            find_owning_package(
                Path::new("/workspace/packages/foo/lib/main.dart"),
                &packages
            ),
            Some("foo".to_string())
        );
    }

    #[test]
    fn test_find_owning_package_not_found() {
        let packages = vec![(PathBuf::from("/workspace/packages/foo"), "foo".to_string())];

        assert_eq!(
            find_owning_package(Path::new("/other/path/main.dart"), &packages),
            None
        );
    }

    #[test]
    fn test_format_changed_packages() {
        let mut names = HashSet::new();
        names.insert("bar".to_string());
        names.insert("foo".to_string());
        names.insert("baz".to_string());
        assert_eq!(format_changed_packages(&names), "bar, baz, foo");
    }

    #[test]
    fn test_format_changed_packages_single() {
        let mut names = HashSet::new();
        names.insert("foo".to_string());
        assert_eq!(format_changed_packages(&names), "foo");
    }

    #[test]
    fn test_format_changed_packages_empty() {
        let names = HashSet::new();
        assert_eq!(format_changed_packages(&names), "");
    }

    #[test]
    fn test_should_ignore_path_symlinks() {
        assert!(should_ignore_path(Path::new(
            "/workspace/packages/foo/.symlinks/plugins/bar"
        )));
        assert!(should_ignore_path(Path::new(
            "/workspace/packages/foo/.plugin_symlinks/plugin"
        )));
    }

    #[test]
    fn test_should_ignore_path_fvm() {
        assert!(should_ignore_path(Path::new(
            "/workspace/packages/foo/.fvm/flutter_sdk"
        )));
    }

    #[test]
    fn test_should_ignore_path_ios_pods() {
        assert!(should_ignore_path(Path::new(
            "/workspace/packages/foo/ios/Pods/Firebase"
        )));
    }

    #[test]
    fn test_has_watched_extension_g_dart() {
        assert!(has_watched_extension(Path::new(
            "lib/models/user.g.dart"
        )));
    }

    #[tokio::test]
    async fn test_start_watching_detects_file_change() {
        use std::fs;
        use std::collections::HashMap;

        // Create a temp directory structure mimicking a package
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("my_package");
        let lib_dir = pkg_dir.join("lib");
        fs::create_dir_all(&lib_dir).unwrap();

        // Write an initial file
        fs::write(lib_dir.join("main.dart"), "void main() {}").unwrap();

        let package = Package {
            name: "my_package".to_string(),
            path: pkg_dir.clone(),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
        };

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

        let packages = vec![package];
        let packages_clone = packages.clone();

        let watcher_handle = tokio::task::spawn_blocking(move || {
            start_watching(&packages_clone, 100, event_tx, shutdown_rx)
        });

        // Give the watcher a moment to initialize
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Write a new file to trigger a change
        fs::write(lib_dir.join("widget.dart"), "class MyWidget {}").unwrap();

        // Wait for the event (with timeout)
        let event = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            event_rx.recv(),
        )
        .await;

        assert!(event.is_ok(), "Should receive an event within 5 seconds");
        let event = event.unwrap().expect("Should have an event");
        assert_eq!(event.package_name, "my_package");

        // Signal shutdown and wait for the watcher to exit
        drop(shutdown_tx);
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            watcher_handle,
        ).await;
    }

    #[tokio::test]
    async fn test_watcher_ignores_non_dart_files() {
        use std::fs;
        use std::collections::HashMap;

        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("my_package");
        let lib_dir = pkg_dir.join("lib");
        fs::create_dir_all(&lib_dir).unwrap();

        let package = Package {
            name: "my_package".to_string(),
            path: pkg_dir.clone(),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
        };

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

        let packages = vec![package];
        let packages_clone = packages.clone();

        let watcher_handle = tokio::task::spawn_blocking(move || {
            start_watching(&packages_clone, 100, event_tx, shutdown_rx)
        });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Write a non-watched file (markdown)
        fs::write(pkg_dir.join("README.md"), "# My Package").unwrap();

        // Wait briefly — should NOT get an event
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            event_rx.recv(),
        )
        .await;

        assert!(
            result.is_err(),
            "Should NOT receive an event for non-watched file extension"
        );

        // Signal shutdown and wait for the watcher to exit
        drop(shutdown_tx);
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            watcher_handle,
        ).await;
    }
}
