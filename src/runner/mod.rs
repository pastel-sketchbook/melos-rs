use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use colored::{Color, Colorize};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;

use crate::package::Package;

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
    /// MELOS_PACKAGE_PATH) are automatically injected alongside workspace env vars.
    /// Each package gets colored output prefixing to distinguish concurrent output.
    ///
    /// If `timeout` is `Some(duration)`, each command is killed after the duration elapses.
    ///
    /// Returns a vec of (package_name, success) results.
    pub async fn run_in_packages(
        &self,
        packages: &[Package],
        command: &str,
        env_vars: &HashMap<String, String>,
        timeout: Option<Duration>,
    ) -> Result<Vec<(String, bool)>> {
        self.run_in_packages_with_progress(packages, command, env_vars, timeout, None)
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
    ) -> Result<Vec<(String, bool)>> {
        let semaphore = std::sync::Arc::new(Semaphore::new(self.concurrency));
        let results = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let mut handles = Vec::new();

        for (idx, pkg) in packages.iter().enumerate() {
            // Check if we should stop early
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
            let color = PKG_COLORS[idx % PKG_COLORS.len()];

            // Build per-package env vars merged with workspace vars
            let env = build_package_env(env_vars, pkg);
            let pb = progress.cloned();

            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                // Skip if already failed and fail-fast is enabled
                if fail_fast && failed.load(std::sync::atomic::Ordering::Relaxed) {
                    results.lock().await.push((pkg_name.clone(), false));
                    return;
                }

                let prefix = format!("[{}]", pkg_name).color(color).bold();
                println!("{} running...", prefix);

                let child = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .current_dir(&pkg_path)
                    .envs(&env)
                    .stdout(std::process::Stdio::inherit())
                    .stderr(std::process::Stdio::inherit())
                    .spawn();

                let success = match child {
                    Ok(child) => {
                        if let Some(dur) = timeout {
                            // Apply timeout: wait for the child or kill it
                            match tokio::time::timeout(dur, child.wait_with_output()).await {
                                Ok(Ok(output)) => output.status.success(),
                                Ok(Err(e)) => {
                                    eprintln!("{} {} {}", prefix, "ERROR".red(), e);
                                    false
                                }
                                Err(_) => {
                                    eprintln!(
                                        "{} {} timed out after {}s",
                                        prefix,
                                        "TIMEOUT".red().bold(),
                                        dur.as_secs()
                                    );
                                    false
                                }
                            }
                        } else {
                            // No timeout: wait normally
                            match child.wait_with_output().await {
                                Ok(output) => output.status.success(),
                                Err(e) => {
                                    eprintln!("{} {} {}", prefix, "ERROR".red(), e);
                                    false
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("{} {} {}", prefix, "ERROR".red(), e);
                        false
                    }
                };

                if success {
                    println!("{} {}", prefix, "SUCCESS".green());
                } else {
                    eprintln!("{} {}", prefix, "FAILED".red());
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
fn build_package_env(
    workspace_env: &HashMap<String, String>,
    pkg: &Package,
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
    env
}
