use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Semaphore;
use tokio::sync::mpsc::UnboundedSender;

use crate::events::Event;
use crate::package::Package;

/// Return the platform-appropriate shell executable and flag for running commands.
///
/// On Windows, returns `("cmd", "/C")` to invoke `cmd.exe /C <command>`.
/// On Unix-like systems, returns `("sh", "-c")` to invoke `sh -c <command>`.
pub fn shell_command() -> (&'static str, &'static str) {
    if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    }
}

/// Process runner that executes shell commands in package directories
/// with configurable concurrency and fail-fast behavior.
pub struct ProcessRunner {
    /// Maximum concurrent processes
    concurrency: usize,
    /// Whether to stop on first failure
    fail_fast: bool,
}

impl ProcessRunner {
    pub fn new(concurrency: usize, fail_fast: bool) -> Self {
        Self {
            concurrency: concurrency.max(1),
            fail_fast,
        }
    }

    /// Run a command in each package directory without event emission.
    ///
    /// Equivalent to calling [`run_in_packages_with_events`] with no event sender.
    /// When no sender is provided, the runner produces no output.
    pub async fn run_in_packages(
        &self,
        packages: &[Package],
        command: &str,
        env_vars: &HashMap<String, String>,
        timeout: Option<Duration>,
        all_packages: &[Package],
    ) -> Result<Vec<(String, bool)>> {
        self.run_in_packages_with_events(packages, command, env_vars, timeout, None, all_packages)
            .await
    }

    /// Run a command in each package directory, emitting events for progress tracking.
    ///
    /// Per-package env vars (MELOS_PACKAGE_NAME, MELOS_PACKAGE_VERSION,
    /// MELOS_PACKAGE_PATH, and MELOS_PARENT_PACKAGE_*) are automatically
    /// injected alongside workspace env vars.
    ///
    /// If `timeout` is `Some(duration)`, each command is killed after the duration elapses.
    ///
    /// `all_packages` is the full workspace package list, used for parent package detection.
    /// If empty, parent package env vars are not set.
    ///
    /// Returns a vec of (package_name, success) results.
    pub async fn run_in_packages_with_events(
        &self,
        packages: &[Package],
        command: &str,
        env_vars: &HashMap<String, String>,
        timeout: Option<Duration>,
        events: Option<&UnboundedSender<Event>>,
        all_packages: &[Package],
    ) -> Result<Vec<(String, bool)>> {
        let semaphore = std::sync::Arc::new(Semaphore::new(self.concurrency));
        let results = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let mut handles = Vec::new();

        for pkg in packages {
            if self.fail_fast && failed.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            let sem = semaphore.clone();
            let results = results.clone();
            let failed = failed.clone();
            let fail_fast = self.fail_fast;
            let command = command.to_string();
            let pkg_name = pkg.name.clone();
            let pkg_path = pkg.path.clone();
            let tx = events.cloned();

            let env = build_package_env(env_vars, pkg, all_packages);

            let handle = tokio::spawn(async move {
                // safety: the semaphore is never closed, so acquire always succeeds
                let _permit = sem.acquire().await.expect("semaphore closed unexpectedly");

                // Skip if already failed and fail-fast is enabled
                if fail_fast && failed.load(std::sync::atomic::Ordering::Relaxed) {
                    results.lock().await.push((pkg_name.clone(), false));
                    return;
                }

                emit(
                    &tx,
                    Event::PackageStarted {
                        name: pkg_name.clone(),
                    },
                );

                let start = std::time::Instant::now();
                let (shell, shell_flag) = shell_command();
                let child = tokio::process::Command::new(shell)
                    .arg(shell_flag)
                    .arg(&command)
                    .current_dir(&pkg_path)
                    .envs(&env)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn();

                let success = match child {
                    Ok(mut child) => {
                        // Take stdout/stderr handles for streaming.
                        // safety: we set Stdio::piped() above so these are always Some
                        let stdout = child.stdout.take().expect("stdout piped");
                        let stderr = child.stderr.take().expect("stderr piped");

                        let stdout_tx = tx.clone();
                        let stderr_tx = tx.clone();
                        let stdout_name = pkg_name.clone();
                        let stderr_name = pkg_name.clone();

                        // Stream stdout lines as they arrive.
                        let stdout_task = tokio::spawn(async move {
                            let reader = BufReader::new(stdout);
                            let mut lines = reader.lines();
                            while let Ok(Some(line)) = lines.next_line().await {
                                emit(
                                    &stdout_tx,
                                    Event::PackageOutput {
                                        name: stdout_name.clone(),
                                        line,
                                        is_stderr: false,
                                    },
                                );
                            }
                        });

                        // Stream stderr lines as they arrive.
                        let stderr_task = tokio::spawn(async move {
                            let reader = BufReader::new(stderr);
                            let mut lines = reader.lines();
                            while let Ok(Some(line)) = lines.next_line().await {
                                emit(
                                    &stderr_tx,
                                    Event::PackageOutput {
                                        name: stderr_name.clone(),
                                        line,
                                        is_stderr: true,
                                    },
                                );
                            }
                        });

                        // Wait for the process to exit, optionally with a timeout.
                        let status = if let Some(dur) = timeout {
                            match tokio::time::timeout(dur, child.wait()).await {
                                Ok(Ok(s)) => Some(s),
                                Ok(Err(e)) => {
                                    emit(
                                        &tx,
                                        Event::PackageOutput {
                                            name: pkg_name.clone(),
                                            line: format!("ERROR: {}", e),
                                            is_stderr: true,
                                        },
                                    );
                                    None
                                }
                                Err(_) => {
                                    emit(
                                        &tx,
                                        Event::PackageOutput {
                                            name: pkg_name.clone(),
                                            line: format!(
                                                "TIMEOUT: timed out after {}s",
                                                dur.as_secs()
                                            ),
                                            is_stderr: true,
                                        },
                                    );
                                    None
                                }
                            }
                        } else {
                            match child.wait().await {
                                Ok(s) => Some(s),
                                Err(e) => {
                                    emit(
                                        &tx,
                                        Event::PackageOutput {
                                            name: pkg_name.clone(),
                                            line: format!("ERROR: {}", e),
                                            is_stderr: true,
                                        },
                                    );
                                    None
                                }
                            }
                        };

                        // Ensure streaming tasks finish before we emit PackageFinished.
                        let _ = stdout_task.await;
                        let _ = stderr_task.await;

                        status.is_some_and(|s| s.success())
                    }
                    Err(e) => {
                        emit(
                            &tx,
                            Event::PackageOutput {
                                name: pkg_name.clone(),
                                line: format!("ERROR: {}", e),
                                is_stderr: true,
                            },
                        );
                        false
                    }
                };

                let duration = start.elapsed();

                emit(
                    &tx,
                    Event::PackageFinished {
                        name: pkg_name.clone(),
                        success,
                        duration,
                    },
                );

                if !success {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                }

                results.lock().await.push((pkg_name, success));
            });

            handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in handles {
            handle.await.context("Package task panicked")?;
        }

        let results = results.lock().await;
        Ok(results.clone())
    }
}

/// Send an event if the transmitter is present, ignoring send errors
/// (the receiver may have been dropped).
fn emit(tx: &Option<UnboundedSender<Event>>, event: Event) {
    if let Some(tx) = tx {
        let _ = tx.send(event);
    }
}

/// Build environment variables for a specific package, merging workspace-level
/// vars with per-package Melos env vars.
///
/// Also sets `MELOS_PARENT_PACKAGE_*` vars when the package is an "example"
/// child of another workspace package (name ends with `example` and its path
/// is a subdirectory of the parent's path).
fn build_package_env(
    workspace_env: &HashMap<String, String>,
    pkg: &Package,
    all_packages: &[Package],
) -> HashMap<String, String> {
    let mut env = workspace_env.clone();
    env.insert("MELOS_PACKAGE_NAME".to_string(), pkg.name.clone());
    env.insert(
        "MELOS_PACKAGE_PATH".to_string(),
        pkg.path.display().to_string(),
    );
    if let Some(ref version) = pkg.version {
        env.insert("MELOS_PACKAGE_VERSION".to_string(), version.clone());
    }

    // Detect parent package: if the current package name ends with "example"
    // and its directory is a child of another package's directory, set parent env vars.
    if let Some(parent) = find_parent_package(pkg, all_packages) {
        env.insert("MELOS_PARENT_PACKAGE_NAME".to_string(), parent.name.clone());
        env.insert(
            "MELOS_PARENT_PACKAGE_PATH".to_string(),
            parent.path.display().to_string(),
        );
        if let Some(ref version) = parent.version {
            env.insert("MELOS_PARENT_PACKAGE_VERSION".to_string(), version.clone());
        }
    }

    env
}

/// Find the parent package for an example package.
///
/// A package is considered a child if:
/// 1. Its name ends with `example` (e.g., `my_pkg_example` or just `example`)
/// 2. Its path is a subdirectory of another workspace package's path
///
/// If multiple candidates match, the one with the longest (deepest) path wins.
fn find_parent_package<'a>(pkg: &Package, all_packages: &'a [Package]) -> Option<&'a Package> {
    if !pkg.name.ends_with("example") {
        return None;
    }

    let mut best: Option<&'a Package> = None;

    for candidate in all_packages {
        // Don't match ourselves
        if candidate.name == pkg.name {
            continue;
        }

        // Check if pkg's path is under candidate's path
        if pkg.path.starts_with(&candidate.path) {
            match best {
                None => best = Some(candidate),
                Some(current_best) => {
                    // Pick the deepest (most specific) parent
                    if candidate.path.as_os_str().len() > current_best.path.as_os_str().len() {
                        best = Some(candidate);
                    }
                }
            }
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_pkg(name: &str, path: &str) -> Package {
        Package {
            name: name.to_string(),
            path: PathBuf::from(path),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        }
    }

    // -- shell_command tests --

    #[test]
    fn test_shell_command_returns_platform_appropriate_values() {
        let (shell, flag) = shell_command();
        if cfg!(target_os = "windows") {
            assert_eq!(shell, "cmd");
            assert_eq!(flag, "/C");
        } else {
            assert_eq!(shell, "sh");
            assert_eq!(flag, "-c");
        }
    }

    // -- find_parent_package tests --

    #[test]
    fn test_find_parent_package_example_child() {
        let parent = make_pkg("my_lib", "/workspace/packages/my_lib");
        let child = make_pkg("my_lib_example", "/workspace/packages/my_lib/example");
        let all = vec![parent.clone(), child.clone()];

        let result = find_parent_package(&child, &all);
        assert!(result.is_some());
        assert_eq!(result.expect("should find parent").name, "my_lib");
    }

    #[test]
    fn test_find_parent_package_bare_example() {
        let parent = make_pkg("my_lib", "/workspace/packages/my_lib");
        let child = make_pkg("example", "/workspace/packages/my_lib/example");
        let all = vec![parent.clone(), child.clone()];

        let result = find_parent_package(&child, &all);
        assert!(result.is_some());
        assert_eq!(result.expect("should find parent").name, "my_lib");
    }

    #[test]
    fn test_find_parent_package_not_example() {
        let pkg = make_pkg("my_lib", "/workspace/packages/my_lib");
        let other = make_pkg("core", "/workspace/packages/core");
        let all = vec![pkg.clone(), other.clone()];

        assert!(find_parent_package(&pkg, &all).is_none());
    }

    #[test]
    fn test_find_parent_package_deepest_wins() {
        let root = make_pkg("app", "/workspace/packages/app");
        let inner = make_pkg("app_feature", "/workspace/packages/app/feature");
        let child = make_pkg(
            "app_feature_example",
            "/workspace/packages/app/feature/example",
        );
        let all = vec![root.clone(), inner.clone(), child.clone()];

        let result = find_parent_package(&child, &all);
        assert!(result.is_some());
        assert_eq!(
            result.expect("should find deepest parent").name,
            "app_feature"
        );
    }

    #[test]
    fn test_find_parent_package_no_match_when_not_under_parent_path() {
        let parent = make_pkg("my_lib", "/workspace/packages/my_lib");
        let child = make_pkg("other_example", "/workspace/packages/other/example");
        let all = vec![parent.clone(), child.clone()];

        // child name ends with "example" but its path is NOT under my_lib's path
        assert!(find_parent_package(&child, &all).is_none());
    }

    #[test]
    fn test_find_parent_package_empty_packages() {
        let child = make_pkg("example", "/workspace/packages/example");
        assert!(find_parent_package(&child, &[]).is_none());
    }

    // -- build_package_env tests --

    #[test]
    fn test_build_package_env_basic() {
        let mut ws_env = HashMap::new();
        ws_env.insert("MELOS_ROOT_PATH".to_string(), "/workspace".to_string());

        let pkg = make_pkg("core", "/workspace/packages/core");
        let env = build_package_env(&ws_env, &pkg, &[]);

        assert_eq!(
            env.get("MELOS_PACKAGE_NAME")
                .expect("should have MELOS_PACKAGE_NAME"),
            "core"
        );
        assert_eq!(
            env.get("MELOS_PACKAGE_PATH")
                .expect("should have MELOS_PACKAGE_PATH"),
            "/workspace/packages/core"
        );
        assert_eq!(
            env.get("MELOS_PACKAGE_VERSION")
                .expect("should have MELOS_PACKAGE_VERSION"),
            "1.0.0"
        );
        assert_eq!(
            env.get("MELOS_ROOT_PATH")
                .expect("should have MELOS_ROOT_PATH"),
            "/workspace"
        );
        // No parent env vars
        assert!(!env.contains_key("MELOS_PARENT_PACKAGE_NAME"));
    }

    #[test]
    fn test_build_package_env_with_parent() {
        let ws_env = HashMap::new();
        let parent = make_pkg("my_lib", "/workspace/packages/my_lib");
        let child = make_pkg("my_lib_example", "/workspace/packages/my_lib/example");
        let all = vec![parent.clone(), child.clone()];

        let env = build_package_env(&ws_env, &child, &all);

        assert_eq!(
            env.get("MELOS_PARENT_PACKAGE_NAME")
                .expect("should have parent name"),
            "my_lib"
        );
        assert_eq!(
            env.get("MELOS_PARENT_PACKAGE_PATH")
                .expect("should have parent path"),
            "/workspace/packages/my_lib"
        );
        assert_eq!(
            env.get("MELOS_PARENT_PACKAGE_VERSION")
                .expect("should have parent version"),
            "1.0.0"
        );
    }

    #[test]
    fn test_build_package_env_no_parent_for_non_example() {
        let ws_env = HashMap::new();
        let pkg = make_pkg("core", "/workspace/packages/core");
        let all = vec![pkg.clone()];

        let env = build_package_env(&ws_env, &pkg, &all);
        assert!(!env.contains_key("MELOS_PARENT_PACKAGE_NAME"));
    }

    #[test]
    fn test_build_package_env_no_version() {
        let ws_env = HashMap::new();
        let mut pkg = make_pkg("core", "/workspace/packages/core");
        pkg.version = None;

        let env = build_package_env(&ws_env, &pkg, &[]);
        assert!(!env.contains_key("MELOS_PACKAGE_VERSION"));
    }
}
