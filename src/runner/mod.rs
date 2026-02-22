use std::collections::HashMap;

use anyhow::Result;
use colored::{Color, Colorize};
use tokio::sync::Semaphore;

use crate::package::Package;

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
    /// Returns a vec of (package_name, success) results.
    pub async fn run_in_packages(
        &self,
        packages: &[Package],
        command: &str,
        env_vars: &HashMap<String, String>,
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

            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                // Skip if already failed and fail-fast is enabled
                if fail_fast && failed.load(std::sync::atomic::Ordering::Relaxed) {
                    results.lock().await.push((pkg_name.clone(), false));
                    return;
                }

                let prefix = format!("[{}]", pkg_name).color(color).bold();
                println!("{} running...", prefix);

                let result = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .current_dir(&pkg_path)
                    .envs(&env)
                    .stdout(std::process::Stdio::inherit())
                    .stderr(std::process::Stdio::inherit())
                    .status()
                    .await;

                let success = match result {
                    Ok(status) => status.success(),
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
