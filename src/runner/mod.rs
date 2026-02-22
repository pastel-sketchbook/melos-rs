use std::collections::HashMap;

use anyhow::Result;
use colored::Colorize;
use tokio::sync::Semaphore;

use crate::package::Package;

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

    /// Run a command in each package directory, respecting concurrency limits
    ///
    /// Returns a vec of (package_name, success) results in order
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

        for pkg in packages {
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
            let env = env_vars.clone();

            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                // Skip if already failed and fail-fast is enabled
                if fail_fast && failed.load(std::sync::atomic::Ordering::Relaxed) {
                    results
                        .lock()
                        .await
                        .push((pkg_name.clone(), false));
                    return;
                }

                println!("{} {} {}", "▶".cyan(), pkg_name.bold(), "...".dimmed());

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
                        eprintln!(
                            "  {} Failed to execute in {}: {}",
                            "ERROR".red(),
                            pkg_name,
                            e
                        );
                        false
                    }
                };

                if !success {
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
