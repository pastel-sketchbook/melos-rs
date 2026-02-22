use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::time::Duration;

use anyhow::{bail, Result};
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::ScriptEntry;
use crate::config::filter::PackageFilters;
use crate::package::filter::{apply_filters_with_categories, topological_sort};
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Maximum recursion depth for nested script references
const MAX_SCRIPT_DEPTH: usize = 16;

/// Arguments for the `run` command
#[derive(Args, Debug)]
pub struct RunArgs {
    /// Name of the script to run (omit for interactive selection)
    pub script: Option<String>,

    /// Skip interactive selection (fail if script not found)
    #[arg(long)]
    pub no_select: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
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

    let cli_filters: PackageFilters = (&args.filters).into();
    let mut visited = HashSet::new();
    run_script_recursive(workspace, &script_name, &cli_filters, &mut visited, 0).await
}

/// Recursively execute a named script, resolving nested `melos run <X>` references.
///
/// When a script's expanded command is `melos-rs run <other_script>` and that
/// script exists in the config, it is executed inline instead of shelling out.
/// A visited set tracks the call chain to detect and prevent cycles.
async fn run_script_recursive(
    workspace: &Workspace,
    script_name: &str,
    cli_filters: &PackageFilters,
    visited: &mut HashSet<String>,
    depth: usize,
) -> Result<()> {
    if depth > MAX_SCRIPT_DEPTH {
        bail!(
            "Script recursion depth exceeded ({} levels). Check for deeply nested 'melos run' references.",
            MAX_SCRIPT_DEPTH
        );
    }

    if !visited.insert(script_name.to_string()) {
        let chain: Vec<_> = visited.iter().cloned().collect();
        bail!(
            "Circular script reference detected: '{}' -> [{}] -> '{}'",
            script_name,
            chain.join(" -> "),
            script_name
        );
    }

    let script = workspace
        .config
        .scripts
        .get(script_name)
        .ok_or_else(|| anyhow::anyhow!("Script '{}' not found in config", script_name))?;

    let run_command = script.run_command();

    if let Some(desc) = script.description() {
        println!("\n{} {}", "Description:".dimmed(), desc.trim());
    }

    let indent = "  ".repeat(depth);
    println!(
        "\n{}{} Running script '{}'...\n",
        indent,
        "$".cyan(),
        script_name.bold()
    );

    // Build env vars with MELOS_ROOT_PATH and any script-level env
    let mut env_vars = workspace.env_vars();
    // Merge script-level env vars (they take precedence over workspace vars)
    env_vars.extend(script.env().iter().map(|(k, v)| (k.clone(), v.clone())));

    // Substitute env vars in the command string (e.g. $MELOS_ROOT_PATH)
    let substituted = substitute_env_vars(run_command, &env_vars);

    // Check if this script has an exec-style command (runs in each package)
    if is_exec_command(&substituted) {
        run_exec_script(workspace, script, &substituted, &env_vars, cli_filters).await?;
    } else {
        // Parse the run command, expanding melos -> melos-rs references
        let expanded = expand_command(&substituted)?;

        // Execute the expanded command(s) at the workspace root
        for cmd in &expanded {
            // Check if this command is a `melos-rs run <script>` reference
            // to a script defined in the config — if so, execute it inline
            if let Some(ref_name) = extract_melos_run_script_name(cmd)
                && workspace.config.scripts.contains_key(ref_name)
            {
                Box::pin(run_script_recursive(
                    workspace,
                    ref_name,
                    cli_filters,
                    visited,
                    depth + 1,
                ))
                .await?;
                continue;
            }

            println!("{}{} {}", indent, ">".dimmed(), cmd.dimmed());

            let status = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
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

    // Remove from visited so the same script can appear in separate chains
    // (e.g. A -> B, A -> C -> B is fine; A -> B -> A is a cycle)
    visited.remove(script_name);

    Ok(())
}

/// Parsed exec flags extracted from a `melos exec [flags] -- <command>` string
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecFlags {
    concurrency: usize,
    fail_fast: bool,
    order_dependents: bool,
    timeout: Option<Duration>,
    dry_run: bool,
}

impl Default for ExecFlags {
    fn default() -> Self {
        Self {
            concurrency: 5, // Melos default
            fail_fast: false,
            order_dependents: false,
            timeout: None,
            dry_run: false,
        }
    }
}

/// Parse all exec flags from a `melos exec [flags] -- <command>` string.
///
/// Recognizes: `-c N` / `--concurrency N`, `--fail-fast`, `--order-dependents`,
/// `--timeout N`, `--dry-run`.
fn parse_exec_flags(command: &str) -> ExecFlags {
    let mut flags = ExecFlags::default();
    let parts: Vec<&str> = command.split_whitespace().collect();

    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "-c" | "--concurrency" => {
                if i + 1 < parts.len() {
                    if let Ok(n) = parts[i + 1].parse::<usize>() {
                        flags.concurrency = n;
                    }
                    i += 1;
                }
            }
            "--fail-fast" => flags.fail_fast = true,
            "--order-dependents" => flags.order_dependents = true,
            "--dry-run" => flags.dry_run = true,
            "--timeout" => {
                if i + 1 < parts.len() {
                    if let Ok(secs) = parts[i + 1].parse::<u64>()
                        && secs > 0
                    {
                        flags.timeout = Some(Duration::from_secs(secs));
                    }
                    i += 1;
                }
            }
            "--" => break, // Stop parsing flags at separator
            _ => {}
        }
        i += 1;
    }

    flags
}

/// Run a script that uses `melos exec` style execution across packages.
///
/// If the script has `packageFilters`, they are merged with CLI filters.
/// CLI filters take precedence when both are set (e.g. `--scope` narrows
/// down even if the script already specifies a scope).
async fn run_exec_script(
    workspace: &Workspace,
    script: &ScriptEntry,
    command: &str,
    env_vars: &HashMap<String, String>,
    cli_filters: &PackageFilters,
) -> Result<()> {
    // Merge script-level packageFilters with CLI filters
    let filters = if let Some(script_filters) = script.package_filters() {
        script_filters.merge(cli_filters)
    } else {
        cli_filters.clone()
    };

    let mut packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    if packages.is_empty() {
        println!("{}", "No packages matched the script's filters.".yellow());
        return Ok(());
    }

    // Parse all exec flags from the command string
    let flags = parse_exec_flags(command);

    // Apply topological sort if requested
    if flags.order_dependents {
        packages = topological_sort(&packages);
        println!(
            "{} Packages ordered by dependencies (topological sort)\n",
            "i".blue()
        );
    }

    let timeout_display = flags
        .timeout
        .map(|d| format!(", timeout {}s", d.as_secs()))
        .unwrap_or_default();

    println!(
        "Running in {} package(s) with concurrency {}{}:\n",
        packages.len().to_string().cyan(),
        flags.concurrency.to_string().cyan(),
        timeout_display,
    );
    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // Dry-run mode: show what would be executed without running
    if flags.dry_run {
        println!("{}", "DRY RUN — no commands were executed.".yellow().bold());
        return Ok(());
    }

    // Extract the actual command after `melos exec` / `melos-rs exec`
    let actual_cmd = extract_exec_command(command);

    let runner = ProcessRunner::new(flags.concurrency, flags.fail_fast);
    let results = runner
        .run_in_packages(&packages, &actual_cmd, env_vars, flags.timeout)
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
    // Look for standalone `--` separator (space-delimited token, not just any `--` prefix)
    let parts: Vec<&str> = command.split_whitespace().collect();
    if let Some(pos) = parts.iter().position(|&p| p == "--") {
        return parts[pos + 1..].join(" ");
    }

    // Fallback: strip `melos exec` / `melos-rs exec` prefix and all known flags
    let stripped = command
        .replace("melos-rs exec", "")
        .replace("melos exec", "");

    // Remove known flags like -c N, --fail-fast, --order-dependents, --timeout N, --dry-run
    let parts: Vec<&str> = stripped.split_whitespace().collect();
    let mut result = Vec::new();
    let mut skip_next = false;

    for part in &parts {
        if skip_next {
            skip_next = false;
            continue;
        }
        if *part == "-c" || *part == "--concurrency" || *part == "--timeout" {
            skip_next = true;
            continue;
        }
        if *part == "--fail-fast" || *part == "--order-dependents" || *part == "--dry-run" {
            continue;
        }
        result.push(*part);
    }

    result.join(" ").trim().to_string()
}

/// Extract the script name from a `melos-rs run <name>` or `melos run <name>` command.
///
/// Returns `Some(script_name)` if the command is a simple `melos[-rs] run <name>` invocation
/// with no extra flags or arguments after the script name.
/// Returns `None` if the command doesn't match or has trailing args.
fn extract_melos_run_script_name(command: &str) -> Option<&str> {
    let trimmed = command.trim();

    // Try `melos-rs run <name>` first, then `melos run <name>`
    let rest = trimmed
        .strip_prefix("melos-rs run ")
        .or_else(|| trimmed.strip_prefix("melos run "))?;

    let rest = rest.trim();

    // The script name must be a single token (no spaces, no extra args)
    if rest.is_empty() || rest.contains(' ') {
        return None;
    }

    Some(rest)
}

/// Substitute environment variables in a command string.
///
/// Replaces `${VAR_NAME}` (braced form) and `$VAR_NAME` (bare form) with their
/// values from the env map. The bare `$VAR` form uses word-boundary matching to
/// avoid replacing `$MELOS_ROOT_PATH` when `$MELOS_ROOT` is also defined.
fn substitute_env_vars(command: &str, env: &HashMap<String, String>) -> String {
    let mut result = command.to_string();

    // Sort keys by length descending so longer variable names are replaced first.
    // This prevents `$MELOS_ROOT` from matching before `$MELOS_ROOT_PATH`.
    let mut sorted_keys: Vec<&String> = env.keys().collect();
    sorted_keys.sort_by_key(|k| std::cmp::Reverse(k.len()));

    for key in sorted_keys {
        let value = &env[key];
        // Replace ${VAR} form (always safe - braces delimit the name)
        result = result.replace(&format!("${{{}}}", key), value);

        // Replace $VAR form with word-boundary awareness:
        // Match $KEY only when NOT followed by another alphanumeric or underscore.
        // Since the regex crate doesn't support lookahead, we use a replacement
        // closure that checks the character after the match.
        let pattern = format!(r"\${}", regex::escape(key));
        if let Ok(re) = regex::Regex::new(&pattern) {
            let bytes = result.clone();
            let bytes = bytes.as_bytes();
            result = re
                .replace_all(&result.clone(), |caps: &regex::Captures| {
                    let m = caps.get(0).unwrap();
                    let end = m.end();
                    // If followed by an alphanumeric or underscore, don't replace
                    if end < bytes.len() {
                        let next = bytes[end];
                        if next.is_ascii_alphanumeric() || next == b'_' {
                            return caps[0].to_string();
                        }
                    }
                    value.clone()
                })
                .to_string();
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
///
/// Uses word-boundary-aware replacement to avoid mangling `melos-rs` into `melos-rs-rs`.
fn expand_command(command: &str) -> Result<Vec<String>> {
    let trimmed = command.trim();

    // Match standalone `melos` as a word. We then check in the replacement
    // whether it's followed by `-rs` (in which case we leave it alone).
    let re = regex::Regex::new(r"\bmelos\b")
        .map_err(|e| anyhow::anyhow!("Failed to compile regex: {}", e))?;

    // Split on `&&` to handle chained commands
    let parts: Vec<String> = trimmed
        .split("&&")
        .map(|part| {
            let part = part.trim();
            // Use replace_all with a closure that checks context after the match
            let bytes = part.as_bytes();
            re.replace_all(part, |caps: &regex::Captures| {
                let m = caps.get(0).unwrap();
                let end = m.end();
                // If followed by `-rs`, don't replace (it's already melos-rs)
                if end < bytes.len() && bytes[end] == b'-' {
                    // Check for "-rs" suffix
                    if part[end..].starts_with("-rs") {
                        return "melos".to_string();
                    }
                }
                "melos-rs".to_string()
            })
            .to_string()
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
    fn test_expand_preserves_melos_rs() {
        // Must NOT turn `melos-rs` into `melos-rs-rs`
        let result = expand_command("melos-rs run generate && melos run build").unwrap();
        assert_eq!(
            result,
            vec!["melos-rs run generate", "melos-rs run build"]
        );
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
    fn test_substitute_env_vars_word_boundary() {
        // When both $MELOS_ROOT and $MELOS_ROOT_PATH are defined,
        // $MELOS_ROOT must NOT match inside $MELOS_ROOT_PATH.
        let mut env = HashMap::new();
        env.insert("MELOS_ROOT".to_string(), "/root".to_string());
        env.insert("MELOS_ROOT_PATH".to_string(), "/workspace".to_string());

        // Bare $MELOS_ROOT_PATH should resolve to /workspace, not /root_PATH
        assert_eq!(
            substitute_env_vars("echo $MELOS_ROOT_PATH", &env),
            "echo /workspace"
        );

        // Bare $MELOS_ROOT alone should still resolve
        assert_eq!(
            substitute_env_vars("echo $MELOS_ROOT end", &env),
            "echo /root end"
        );

        // Both in the same string
        assert_eq!(
            substitute_env_vars("$MELOS_ROOT and $MELOS_ROOT_PATH", &env),
            "/root and /workspace"
        );

        // Braced forms should always be unambiguous
        assert_eq!(
            substitute_env_vars("${MELOS_ROOT} and ${MELOS_ROOT_PATH}", &env),
            "/root and /workspace"
        );

        // $MELOS_ROOT at end of string (no trailing char)
        assert_eq!(
            substitute_env_vars("path=$MELOS_ROOT", &env),
            "path=/root"
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
    fn test_extract_exec_command_strips_new_flags() {
        // Fallback path (no -- separator): should strip all known flags
        assert_eq!(
            extract_exec_command("melos exec --order-dependents --dry-run --timeout 30 flutter test"),
            "flutter test"
        );
    }

    #[test]
    fn test_parse_exec_flags_defaults() {
        let flags = parse_exec_flags("melos exec -- flutter test");
        assert_eq!(flags.concurrency, 5);
        assert!(!flags.fail_fast);
        assert!(!flags.order_dependents);
        assert!(flags.timeout.is_none());
        assert!(!flags.dry_run);
    }

    #[test]
    fn test_parse_exec_flags_concurrency() {
        let flags = parse_exec_flags("melos exec -c 3 -- flutter test");
        assert_eq!(flags.concurrency, 3);

        let flags2 = parse_exec_flags("melos exec --concurrency 10 -- dart test");
        assert_eq!(flags2.concurrency, 10);
    }

    #[test]
    fn test_parse_exec_flags_all() {
        let flags = parse_exec_flags(
            "melos exec -c 2 --fail-fast --order-dependents --timeout 60 --dry-run -- flutter test"
        );
        assert_eq!(flags.concurrency, 2);
        assert!(flags.fail_fast);
        assert!(flags.order_dependents);
        assert_eq!(flags.timeout, Some(Duration::from_secs(60)));
        assert!(flags.dry_run);
    }

    #[test]
    fn test_parse_exec_flags_timeout_zero() {
        let flags = parse_exec_flags("melos exec --timeout 0 -- flutter test");
        assert!(flags.timeout.is_none(), "timeout 0 means no timeout");
    }

    #[test]
    fn test_extract_melos_run_script_name() {
        // Should extract script name from `melos run <name>`
        assert_eq!(
            extract_melos_run_script_name("melos run build"),
            Some("build")
        );
        assert_eq!(
            extract_melos_run_script_name("melos-rs run build"),
            Some("build")
        );
        assert_eq!(
            extract_melos_run_script_name("melos-rs run generate:dart"),
            Some("generate:dart")
        );

        // Should return None for non-matching commands
        assert_eq!(extract_melos_run_script_name("flutter analyze ."), None);
        assert_eq!(extract_melos_run_script_name("dart format ."), None);

        // Should return None when there are extra args after the script name
        assert_eq!(
            extract_melos_run_script_name("melos-rs run build --verbose"),
            None
        );

        // Should return None when no script name is given
        assert_eq!(extract_melos_run_script_name("melos run "), None);
        assert_eq!(extract_melos_run_script_name("melos-rs run"), None);
    }
}
