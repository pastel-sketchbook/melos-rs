use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use colored::{Color, Colorize};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;

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

/// Run a lifecycle hook shell command in the workspace root.
///
/// Prints the hook label and command, executes via the platform shell, and
/// bails if the command exits with a non-zero status. Extra environment
/// variables (e.g. `MELOS_PUBLISH_DRY_RUN`) can be passed via `extra_env`.
pub async fn run_lifecycle_hook(
    hook_cmd: &str,
    label: &str,
    root_path: &Path,
    extra_env: &[(&str, &str)],
) -> Result<()> {
    println!("\n{} Running {} hook: {}", "$".cyan(), label, hook_cmd);
    let (shell, shell_flag) = shell_command();
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg(shell_flag).arg(hook_cmd).current_dir(root_path);
    for &(key, val) in extra_env {
        cmd.env(key, val);
    }
    let status = cmd.status().await?;

    if !status.success() {
        anyhow::bail!(
            "{} hook failed with exit code: {}",
            label,
            status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

/// Create a styled progress bar for package processing.
///
/// Uses a consistent style across all commands:
/// `{spinner} [{bar}] {pos}/{len} {msg}`
pub fn create_progress_bar(total: u64, message: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("=> "),
    );
    pb.set_message(message.to_string());
    pb
}

/// Colors assigned to packages for distinguishing concurrent output
const PKG_COLORS: &[Color] = &[
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Red,
    Color::BrightCyan,
    Color::BrightGreen,
    Color::BrightYellow,
    Color::BrightBlue,
];

/// Process runner that executes shell commands in package directories
/// with configurable concurrency and fail-fast behavior
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

    /// Run a command in each package directory, respecting concurrency limits.
    ///
    /// Per-package env vars (MELOS_PACKAGE_NAME, MELOS_PACKAGE_VERSION,
    /// MELOS_PACKAGE_PATH, and MELOS_PARENT_PACKAGE_*) are automatically
    /// injected alongside workspace env vars.
    /// Each package gets colored output prefixing to distinguish concurrent output.
    ///
    /// If `timeout` is `Some(duration)`, each command is killed after the duration elapses.
    ///
    /// `all_packages` is the full workspace package list, used for parent package detection.
    /// If empty, parent package env vars are not set.
    ///
    /// Returns a vec of (package_name, success) results.
    pub async fn run_in_packages(
        &self,
        packages: &[Package],
        command: &str,
        env_vars: &HashMap<String, String>,
        timeout: Option<Duration>,
        all_packages: &[Package],
    ) -> Result<Vec<(String, bool)>> {
        self.run_in_packages_with_progress(packages, command, env_vars, timeout, None, all_packages)
            .await
    }

    /// Like [`run_in_packages`] but accepts an optional [`ProgressBar`] that is
    /// incremented in real time as each package command completes.
    pub async fn run_in_packages_with_progress(
        &self,
        packages: &[Package],
        command: &str,
        env_vars: &HashMap<String, String>,
        timeout: Option<Duration>,
        progress: Option<&ProgressBar>,
        all_packages: &[Package],
    ) -> Result<Vec<(String, bool)>> {
        let semaphore = std::sync::Arc::new(Semaphore::new(self.concurrency));
        let results = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Mutex used to serialize output blocks so concurrent package output
        // does not interleave.
        let output_lock = std::sync::Arc::new(std::sync::Mutex::new(()));

        let mut handles = Vec::new();

        for (idx, pkg) in packages.iter().enumerate() {
            if self.fail_fast && failed.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            let sem = semaphore.clone();
            let results = results.clone();
            let failed = failed.clone();
            let output_lock = output_lock.clone();
            let fail_fast = self.fail_fast;
            let command = command.to_string();
            let pkg_name = pkg.name.clone();
            let pkg_path = pkg.path.clone();
            let color = PKG_COLORS[idx % PKG_COLORS.len()];

            let env = build_package_env(env_vars, pkg, all_packages);
            let pb = progress.cloned();

            let handle = tokio::spawn(async move {
                // The semaphore is never closed, so acquire always succeeds.
                let _permit = sem.acquire().await.expect("semaphore closed unexpectedly");

                // Skip if already failed and fail-fast is enabled
                if fail_fast && failed.load(std::sync::atomic::Ordering::Relaxed) {
                    results.lock().await.push((pkg_name.clone(), false));
                    return;
                }

                let prefix = format!("[{}]", pkg_name).color(color).bold();
                println!("{} running...", prefix);

                let (shell, shell_flag) = shell_command();
                let child = tokio::process::Command::new(shell)
                    .arg(shell_flag)
                    .arg(&command)
                    .current_dir(&pkg_path)
                    .envs(&env)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn();

                // Collect output and determine success.
                // stdout/stderr are buffered and printed atomically after completion.
                let (success, stdout_buf, stderr_buf) = match child {
                    Ok(child) => {
                        if let Some(dur) = timeout {
                            match tokio::time::timeout(dur, child.wait_with_output()).await {
                                Ok(Ok(output)) => {
                                    (output.status.success(), output.stdout, output.stderr)
                                }
                                Ok(Err(e)) => {
                                    let msg = format!("{} {} {}\n", prefix, "ERROR".red(), e);
                                    (false, Vec::new(), msg.into_bytes())
                                }
                                Err(_) => {
                                    let msg = format!(
                                        "{} {} timed out after {}s\n",
                                        prefix,
                                        "TIMEOUT".red().bold(),
                                        dur.as_secs()
                                    );
                                    (false, Vec::new(), msg.into_bytes())
                                }
                            }
                        } else {
                            match child.wait_with_output().await {
                                Ok(output) => {
                                    (output.status.success(), output.stdout, output.stderr)
                                }
                                Err(e) => {
                                    let msg = format!("{} {} {}\n", prefix, "ERROR".red(), e);
                                    (false, Vec::new(), msg.into_bytes())
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("{} {} {}\n", prefix, "ERROR".red(), e);
                        (false, Vec::new(), msg.into_bytes())
                    }
                };

                // Atomically print the entire output block under the lock
                {
                    // The lock is never poisoned in practice since we
                    // never panic while holding it; using expect for clarity.
                    let _guard = output_lock.lock().expect("output lock poisoned");

                    if !stdout_buf.is_empty() {
                        for line in String::from_utf8_lossy(&stdout_buf).lines() {
                            println!("{} {}", prefix, line);
                        }
                    }
                    if !stderr_buf.is_empty() {
                        for line in String::from_utf8_lossy(&stderr_buf).lines() {
                            eprintln!("{} {}", prefix, line);
                        }
                    }

                    if success {
                        println!("{} {}", prefix, "SUCCESS".green());
                    } else {
                        eprintln!("{} {}", prefix, "FAILED".red());
                    }
                }

                if !success {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                }

                results.lock().await.push((pkg_name, success));
                if let Some(ref pb) = pb {
                    pb.inc(1);
                }
            });

            handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in handles {
            handle.await?;
        }

        let results = results.lock().await;
        Ok(results.clone())
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
        }
    }

    // ── shell_command tests ─────────────────────────────────────────────

    #[test]
    fn test_shell_command_returns_platform_appropriate_values() {
        let (shell, flag) = super::shell_command();
        if cfg!(target_os = "windows") {
            assert_eq!(shell, "cmd");
            assert_eq!(flag, "/C");
        } else {
            assert_eq!(shell, "sh");
            assert_eq!(flag, "-c");
        }
    }

    // ── find_parent_package tests ──────────────────────────────────────

    #[test]
    fn test_find_parent_package_example_child() {
        let parent = make_pkg("my_lib", "/workspace/packages/my_lib");
        let child = make_pkg("my_lib_example", "/workspace/packages/my_lib/example");
        let all = vec![parent.clone(), child.clone()];

        let result = find_parent_package(&child, &all);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "my_lib");
    }

    #[test]
    fn test_find_parent_package_bare_example() {
        let parent = make_pkg("my_lib", "/workspace/packages/my_lib");
        let child = make_pkg("example", "/workspace/packages/my_lib/example");
        let all = vec![parent.clone(), child.clone()];

        let result = find_parent_package(&child, &all);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "my_lib");
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
        assert_eq!(result.unwrap().name, "app_feature");
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

    // ── build_package_env tests ────────────────────────────────────────

    #[test]
    fn test_build_package_env_basic() {
        let mut ws_env = HashMap::new();
        ws_env.insert("MELOS_ROOT_PATH".to_string(), "/workspace".to_string());

        let pkg = make_pkg("core", "/workspace/packages/core");
        let env = build_package_env(&ws_env, &pkg, &[]);

        assert_eq!(env.get("MELOS_PACKAGE_NAME").unwrap(), "core");
        assert_eq!(
            env.get("MELOS_PACKAGE_PATH").unwrap(),
            "/workspace/packages/core"
        );
        assert_eq!(env.get("MELOS_PACKAGE_VERSION").unwrap(), "1.0.0");
        assert_eq!(env.get("MELOS_ROOT_PATH").unwrap(), "/workspace");
        // No parent env vars
        assert!(env.get("MELOS_PARENT_PACKAGE_NAME").is_none());
    }

    #[test]
    fn test_build_package_env_with_parent() {
        let ws_env = HashMap::new();
        let parent = make_pkg("my_lib", "/workspace/packages/my_lib");
        let child = make_pkg("my_lib_example", "/workspace/packages/my_lib/example");
        let all = vec![parent.clone(), child.clone()];

        let env = build_package_env(&ws_env, &child, &all);

        assert_eq!(env.get("MELOS_PARENT_PACKAGE_NAME").unwrap(), "my_lib");
        assert_eq!(
            env.get("MELOS_PARENT_PACKAGE_PATH").unwrap(),
            "/workspace/packages/my_lib"
        );
        assert_eq!(env.get("MELOS_PARENT_PACKAGE_VERSION").unwrap(), "1.0.0");
    }

    #[test]
    fn test_build_package_env_no_parent_for_non_example() {
        let ws_env = HashMap::new();
        let pkg = make_pkg("core", "/workspace/packages/core");
        let all = vec![pkg.clone()];

        let env = build_package_env(&ws_env, &pkg, &all);
        assert!(env.get("MELOS_PARENT_PACKAGE_NAME").is_none());
    }

    #[test]
    fn test_build_package_env_no_version() {
        let ws_env = HashMap::new();
        let mut pkg = make_pkg("core", "/workspace/packages/core");
        pkg.version = None;

        let env = build_package_env(&ws_env, &pkg, &[]);
        assert!(env.get("MELOS_PACKAGE_VERSION").is_none());
    }
}
