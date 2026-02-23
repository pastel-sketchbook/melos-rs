use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use melos_core::runner::shell_command;

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
