use anyhow::{bail, Result};
use colored::Colorize;

use crate::workspace::Workspace;

/// Execute a named script from the melos.yaml scripts section
pub async fn run(workspace: &Workspace, script_name: &str) -> Result<()> {
    let script = workspace
        .config
        .scripts
        .get(script_name)
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

    // Parse the run command to handle `melos run <other_script>` and `melos exec` references
    let expanded = expand_command(run_command, workspace)?;

    // Execute the expanded command(s)
    for cmd in expanded {
        println!("{} {}", ">".dimmed(), cmd.dimmed());

        let status = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .current_dir(&workspace.root_path)
            .envs(workspace.env_vars())
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

    Ok(())
}

/// Expand a run command, resolving `melos run <X>` references to the actual
/// melos-rs binary, and splitting `&&` chains into separate commands.
///
/// For example:
///   "melos run generate:dart && melos run generate:flutter"
/// becomes:
///   ["melos-rs run generate:dart", "melos-rs run generate:flutter"]
fn expand_command(command: &str, _workspace: &Workspace) -> Result<Vec<String>> {
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
    use crate::workspace::Workspace;
    use std::path::PathBuf;

    fn dummy_workspace() -> Workspace {
        Workspace {
            root_path: PathBuf::from("/tmp/test"),
            config: crate::config::MelosConfig {
                name: "test".to_string(),
                packages: vec![],
                command: None,
                scripts: Default::default(),
            },
            packages: vec![],
        }
    }

    #[test]
    fn test_expand_simple_command() {
        let ws = dummy_workspace();
        let result = expand_command("flutter analyze .", &ws).unwrap();
        assert_eq!(result, vec!["flutter analyze ."]);
    }

    #[test]
    fn test_expand_chained_command() {
        let ws = dummy_workspace();
        let result =
            expand_command("melos run generate:dart && melos run generate:flutter", &ws).unwrap();
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
        let ws = dummy_workspace();
        let result = expand_command("melos exec -c 1 -- flutter analyze .", &ws).unwrap();
        assert_eq!(result, vec!["melos-rs exec -c 1 -- flutter analyze ."]);
    }
}
