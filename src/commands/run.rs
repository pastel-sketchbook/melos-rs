use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use anyhow::{bail, Result};
use clap::Args;
use colored::Colorize;

use crate::config::ScriptEntry;
use crate::package::filter::apply_filters;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Arguments for the `run` command
#[derive(Args, Debug)]
pub struct RunArgs {
    /// Name of the script to run (omit for interactive selection)
    pub script: Option<String>,

    /// Skip interactive selection (fail if script not found)
    #[arg(long)]
    pub no_select: bool,
}

/// Execute a named script from the melos.yaml scripts section
pub async fn run(workspace: &Workspace, args: RunArgs) -> Result<()> {
    let script_name = match args.script {
        Some(name) => name,
        None if args.no_select => {
            bail!("No script name provided and --no-select is set");
        }
        None => select_script_interactive(workspace)?,
    };

    let script = workspace
        .config
        .scripts
        .get(&script_name)
        .ok_or_else(|| anyhow::anyhow!("Script '{}' not found in melos.yaml", script_name))?;

    let run_command = script.run_command();

    if let Some(desc) = script.description() {
        println!("\n{} {}", "Description:".dimmed(), desc.trim());
    }

    println!(
        "\n{} Running script '{}'...\n",
        "$".cyan(),
        script_name.bold()
    );

    // Build env vars with MELOS_ROOT_PATH and any script-level env in the future
    let env_vars = workspace.env_vars();

    // Substitute env vars in the command string (e.g. $MELOS_ROOT_PATH)
    let substituted = substitute_env_vars(run_command, &env_vars);

    // Check if this script has an exec-style command (runs in each package)
    if is_exec_command(&substituted) {
        run_exec_script(workspace, script, &substituted, &env_vars).await?;
    } else {
        // Parse the run command, expanding melos -> melos-rs references
        let expanded = expand_command(&substituted)?;

        // Execute the expanded command(s) at the workspace root
        for cmd in expanded {
            println!("{} {}", ">".dimmed(), cmd.dimmed());

            let status = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .current_dir(&workspace.root_path)
                .envs(&env_vars)
                .status()
                .await?;

            if !status.success() {
                bail!(
                    "Script '{}' failed with exit code: {}",
                    script_name,
                    status.code().unwrap_or(-1)
                );
            }
        }
    }

    Ok(())
}

/// Run a script that uses `melos exec` style execution across packages.
///
/// If the script has `packageFilters`, they are applied to narrow down packages.
async fn run_exec_script(
    workspace: &Workspace,
    script: &ScriptEntry,
    command: &str,
    env_vars: &HashMap<String, String>,
) -> Result<()> {
    // Apply script-level packageFilters if present
    let packages = if let Some(filters) = script.package_filters() {
        apply_filters(&workspace.packages, filters, Some(&workspace.root_path))?
    } else {
        workspace.packages.clone()
    };

    if packages.is_empty() {
        println!("{}", "No packages matched the script's filters.".yellow());
        return Ok(());
    }

    println!(
        "Running in {} package(s):\n",
        packages.len().to_string().cyan()
    );
    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // Extract the actual command after `melos exec` / `melos-rs exec`
    let actual_cmd = extract_exec_command(command);

    // Parse concurrency from exec flags (e.g., `-c 5`)
    let concurrency = extract_exec_concurrency(command);
    let fail_fast = command.contains("--fail-fast");

    let runner = ProcessRunner::new(concurrency, fail_fast);
    let results = runner
        .run_in_packages(&packages, &actual_cmd, env_vars)
        .await?;

    let failed = results.iter().filter(|(_, success)| !success).count();
    if failed > 0 {
        bail!("{} package(s) failed", failed);
    }

    Ok(())
}

/// Prompt the user to select a script interactively from available scripts
fn select_script_interactive(workspace: &Workspace) -> Result<String> {
    let scripts: Vec<(&String, &ScriptEntry)> = workspace.config.scripts.iter().collect();

    if scripts.is_empty() {
        bail!("No scripts defined in melos.yaml");
    }

    println!("\n{}\n", "Select a script to run:".bold());
    let mut sorted_scripts: Vec<_> = scripts.iter().collect();
    sorted_scripts.sort_by_key(|(name, _)| *name);

    for (i, (name, entry)) in sorted_scripts.iter().enumerate() {
        let desc = entry
            .description()
            .map(|d| format!(" - {}", d.trim().dimmed()))
            .unwrap_or_default();
        println!("  {} {}{}", format!("[{}]", i + 1).cyan(), name, desc);
    }

    print!("\n{} ", "Enter number or name:".bold());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin()
        .lock()
        .read_line(&mut input)?;
    let input = input.trim();

    // Try as number first
    if let Ok(num) = input.parse::<usize>() {
        if num >= 1 && num <= sorted_scripts.len() {
            return Ok(sorted_scripts[num - 1].0.to_string());
        }
        bail!("Invalid selection: {}", num);
    }

    // Try as name
    if workspace.config.scripts.contains_key(input) {
        return Ok(input.to_string());
    }

    bail!("Script '{}' not found", input);
}

/// Check if a command is an exec-style command (runs in each package)
fn is_exec_command(command: &str) -> bool {
    let trimmed = command.trim();
    trimmed.starts_with("melos exec")
        || trimmed.starts_with("melos-rs exec")
        || trimmed.contains("melos exec")
        || trimmed.contains("melos-rs exec")
}

/// Extract the actual command from a `melos exec -- <command>` string
fn extract_exec_command(command: &str) -> String {
    // Look for `--` separator
    if let Some(pos) = command.find("--") {
        let after_separator = &command[pos + 2..];
        return after_separator.trim().to_string();
    }

    // Fallback: strip `melos exec` / `melos-rs exec` prefix and flags
    let stripped = command
        .replace("melos-rs exec", "")
        .replace("melos exec", "");

    // Remove known flags like -c N, --fail-fast
    let parts: Vec<&str> = stripped.split_whitespace().collect();
    let mut result = Vec::new();
    let mut skip_next = false;

    for part in &parts {
        if skip_next {
            skip_next = false;
            continue;
        }
        if *part == "-c" || *part == "--concurrency" {
            skip_next = true;
            continue;
        }
        if *part == "--fail-fast" {
            continue;
        }
        result.push(*part);
    }

    result.join(" ").trim().to_string()
}

/// Extract concurrency value from exec flags (e.g., `-c 5`)
fn extract_exec_concurrency(command: &str) -> usize {
    let parts: Vec<&str> = command.split_whitespace().collect();
    for (i, part) in parts.iter().enumerate() {
        if (*part == "-c" || *part == "--concurrency")
            && i + 1 < parts.len()
            && let Ok(n) = parts[i + 1].parse::<usize>()
        {
            return n;
        }
    }
    5 // Melos default
}

/// Substitute environment variables in a command string.
///
/// Replaces `$VAR_NAME` and `${VAR_NAME}` with their values from the env map.
fn substitute_env_vars(command: &str, env: &HashMap<String, String>) -> String {
    let mut result = command.to_string();

    for (key, value) in env {
        // Replace ${VAR} form
        result = result.replace(&format!("${{{}}}", key), value);
        // Replace $VAR form (only when followed by non-alphanumeric or end)
        let pattern = format!("${}", key);
        if result.contains(&pattern) {
            // Simple replacement - works for most cases
            result = result.replace(&pattern, value);
        }
    }

    result
}

/// Expand a run command, resolving `melos run <X>` references to the actual
/// melos-rs binary, and splitting `&&` chains into separate commands.
///
/// For example:
///   "melos run generate:dart && melos run generate:flutter"
/// becomes:
///   ["melos-rs run generate:dart", "melos-rs run generate:flutter"]
fn expand_command(command: &str) -> Result<Vec<String>> {
    let trimmed = command.trim();

    // Split on `&&` to handle chained commands
    let parts: Vec<String> = trimmed
        .split("&&")
        .map(|part| {
            let part = part.trim().to_string();
            // Replace `melos run` with `melos-rs run` so it calls back into ourselves
            // Replace `melos exec` with `melos-rs exec`
            let part = part.replace("melos exec", "melos-rs exec");
            let part = part.replace("melos run", "melos-rs run");
            part.replace("melos version", "melos-rs version")
        })
        .collect();

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_expand_simple_command() {
        let result = expand_command("flutter analyze .").unwrap();
        assert_eq!(result, vec!["flutter analyze ."]);
    }

    #[test]
    fn test_expand_chained_command() {
        let result =
            expand_command("melos run generate:dart && melos run generate:flutter").unwrap();
        assert_eq!(
            result,
            vec![
                "melos-rs run generate:dart",
                "melos-rs run generate:flutter"
            ]
        );
    }

    #[test]
    fn test_expand_exec_command() {
        let result = expand_command("melos exec -c 1 -- flutter analyze .").unwrap();
        assert_eq!(result, vec!["melos-rs exec -c 1 -- flutter analyze ."]);
    }

    #[test]
    fn test_substitute_env_vars() {
        let mut env = HashMap::new();
        env.insert("MELOS_ROOT_PATH".to_string(), "/workspace".to_string());

        assert_eq!(
            substitute_env_vars("echo $MELOS_ROOT_PATH", &env),
            "echo /workspace"
        );
        assert_eq!(
            substitute_env_vars("echo ${MELOS_ROOT_PATH}/bin", &env),
            "echo /workspace/bin"
        );
    }

    #[test]
    fn test_is_exec_command() {
        assert!(is_exec_command("melos exec -- flutter test"));
        assert!(is_exec_command("melos-rs exec -c 5 -- dart test"));
        assert!(!is_exec_command("flutter analyze ."));
        assert!(!is_exec_command("dart format ."));
    }

    #[test]
    fn test_extract_exec_command() {
        assert_eq!(
            extract_exec_command("melos exec -- flutter test"),
            "flutter test"
        );
        assert_eq!(
            extract_exec_command("melos exec -c 5 -- dart analyze ."),
            "dart analyze ."
        );
    }

    #[test]
    fn test_extract_exec_concurrency() {
        assert_eq!(
            extract_exec_concurrency("melos exec -c 3 -- flutter test"),
            3
        );
        assert_eq!(
            extract_exec_concurrency("melos exec -- flutter test"),
            5 // default
        );
    }
}
