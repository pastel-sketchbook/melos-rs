use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::time::Duration;

use anyhow::{Result, bail};
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::ScriptEntry;
use crate::config::filter::PackageFilters;
use crate::package::Package;
use crate::package::filter::{apply_filters_with_categories, topological_sort};
use crate::runner::ProcessRunner;
use crate::watcher;
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

    /// List available scripts instead of running one
    #[arg(long)]
    pub list: bool,

    /// Output script list as JSON (use with --list)
    #[arg(long, requires = "list")]
    pub json: bool,

    /// Include private scripts in interactive selection and --list output
    #[arg(long)]
    pub include_private: bool,

    /// Filter scripts by group (can be repeated)
    #[arg(long)]
    pub group: Vec<String>,

    /// Watch for file changes and re-run the script on change
    #[arg(long)]
    pub watch: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Execute a named script from the melos.yaml scripts section
pub async fn run(workspace: &Workspace, args: RunArgs) -> Result<()> {
    // Handle --list mode
    if args.list {
        return list_scripts(workspace, args.json, args.include_private, &args.group);
    }

    let script_name = match args.script {
        Some(name) => name,
        None if args.no_select => {
            bail!("No script name provided and --no-select is set");
        }
        None => select_script_interactive(workspace, args.include_private, &args.group)?,
    };

    let watch_mode = args.watch;
    let cli_filters: PackageFilters = (&args.filters).into();

    // Initial run
    let mut visited = HashSet::new();
    let result = run_script_recursive(workspace, &script_name, &cli_filters, &mut visited, 0).await;

    if let Err(e) = &result {
        if watch_mode {
            eprintln!(
                "\n{} Script failed: {}. Watching for changes...",
                "!".yellow().bold(),
                e,
            );
        } else {
            return result;
        }
    }

    // If watch mode, start watching and re-run on changes
    if watch_mode {
        run_watch_loop(workspace, &script_name, &cli_filters).await?;
    }

    Ok(())
}

/// Run the watch loop for a named script: wait for file changes, then re-execute.
///
/// Watches all workspace packages (or just filtered ones if the script has packageFilters)
/// and re-runs the entire script on any change.
async fn run_watch_loop(
    workspace: &Workspace,
    script_name: &str,
    cli_filters: &PackageFilters,
) -> Result<()> {
    // Determine which packages to watch:
    // If the script has packageFilters, watch only those packages.
    // Otherwise watch all workspace packages.
    let script = workspace
        .config
        .scripts
        .get(script_name)
        .ok_or_else(|| anyhow::anyhow!("Script '{}' not found in config", script_name))?;

    let watch_packages = if let Some(script_filters) = script.package_filters() {
        let merged = script_filters.merge(cli_filters);
        apply_filters_with_categories(
            &workspace.packages,
            &merged,
            Some(&workspace.root_path),
            &workspace.config.categories,
        )?
    } else if !cli_filters.is_empty() {
        apply_filters_with_categories(
            &workspace.packages,
            cli_filters,
            Some(&workspace.root_path),
            &workspace.config.categories,
        )?
    } else {
        workspace.packages.clone()
    };

    if watch_packages.is_empty() {
        println!("{}", "No packages to watch.".yellow());
        return Ok(());
    }

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    let watch_pkgs_clone: Vec<Package> = watch_packages.to_vec();

    let watcher_handle = tokio::task::spawn_blocking(move || {
        watcher::start_watching(&watch_pkgs_clone, 0, event_tx, shutdown_rx)
    });

    let shutdown_tx_ctrlc = shutdown_tx.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            println!("\n{} Stopping watcher...", "!".yellow());
            let _ = shutdown_tx_ctrlc.send(()).await;
        }
    });

    // Watch loop
    loop {
        let first_event = match event_rx.recv().await {
            Some(e) => e,
            None => break,
        };

        let mut changed_packages = HashSet::new();
        changed_packages.insert(first_event.package_name);
        while let Ok(event) = event_rx.try_recv() {
            changed_packages.insert(event.package_name);
        }

        println!(
            "\n{} Changes detected in: {}\n",
            "\u{21bb}".cyan().bold(),
            watcher::format_changed_packages(&changed_packages).bold(),
        );

        // Re-run the entire script
        let mut visited = HashSet::new();
        match run_script_recursive(workspace, script_name, cli_filters, &mut visited, 0).await {
            Ok(()) => {
                println!(
                    "\n{} Script '{}' succeeded. Watching for changes...",
                    "\u{2713}".green().bold(),
                    script_name,
                );
            }
            Err(e) => {
                eprintln!(
                    "\n{} Script '{}' failed: {}. Watching for changes...",
                    "!".yellow().bold(),
                    script_name,
                    e,
                );
            }
        }
    }

    let _ = shutdown_tx.send(()).await;
    let _ = watcher_handle.await;

    Ok(())
}

/// List available scripts.
///
/// With `--json`, outputs a JSON array of script objects.
/// Otherwise, prints a formatted table.
fn list_scripts(
    workspace: &Workspace,
    json: bool,
    include_private: bool,
    groups: &[String],
) -> Result<()> {
    let mut scripts: Vec<(&String, &ScriptEntry)> = workspace
        .config
        .scripts
        .iter()
        .filter(|(_, entry)| include_private || !entry.is_private())
        .filter(|(_, entry)| groups.is_empty() || groups.iter().any(|g| entry.in_group(g)))
        .collect();
    scripts.sort_by_key(|(name, _)| *name);

    if json {
        let entries: Vec<serde_json::Value> = scripts
            .iter()
            .map(|(name, entry)| {
                let mut obj = serde_json::Map::new();
                obj.insert(
                    "name".to_string(),
                    serde_json::Value::String((*name).clone()),
                );
                if let Some(desc) = entry.description() {
                    obj.insert(
                        "description".to_string(),
                        serde_json::Value::String(desc.to_string()),
                    );
                }
                if entry.is_private() {
                    obj.insert("private".to_string(), serde_json::Value::Bool(true));
                }
                if entry.has_exec_config() {
                    obj.insert("exec".to_string(), serde_json::Value::Bool(true));
                }
                if entry.steps().is_some() {
                    obj.insert("steps".to_string(), serde_json::Value::Bool(true));
                }
                if let Some(groups) = entry.groups() {
                    let groups_json: Vec<serde_json::Value> = groups
                        .iter()
                        .map(|g| serde_json::Value::String(g.clone()))
                        .collect();
                    obj.insert("groups".to_string(), serde_json::Value::Array(groups_json));
                }
                serde_json::Value::Object(obj)
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
        );
    } else {
        if scripts.is_empty() {
            println!("No scripts available.");
            return Ok(());
        }

        println!("\n{}\n", "Available scripts:".bold());
        for (name, entry) in &scripts {
            let desc = entry
                .description()
                .map(|d| format!(" - {}", d.trim().dimmed()))
                .unwrap_or_default();
            let private_tag = if entry.is_private() {
                format!(" {}", "[private]".dimmed())
            } else {
                String::new()
            };
            let mode = if entry.steps().is_some() {
                format!(" {}", "(steps)".dimmed())
            } else if entry.has_exec_config() {
                format!(" {}", "(exec)".dimmed())
            } else {
                String::new()
            };
            println!(
                "  {} {}{}{}{}",
                "->".cyan(),
                name.bold(),
                desc,
                mode,
                private_tag
            );
        }
        println!();
    }

    Ok(())
}

/// Recursively execute a named script, resolving nested `melos run <X>` references.
///
/// When a script's expanded command is `melos-rs run <other_script>` and that
/// script exists in the config, it is executed inline instead of shelling out.
/// A visited set tracks the call chain to detect and prevent cycles.
///
/// Supports three execution modes:
/// 1. **Steps**: execute each step sequentially (shell commands or script references)
/// 2. **Exec config**: per-package execution using `exec:` config
/// 3. **Run command**: shell command at workspace root (with `melos exec` string detection)
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

    let mut env_vars = workspace.env_vars();
    // Merge script-level env vars (they take precedence over workspace vars)
    env_vars.extend(script.env().iter().map(|(k, v)| (k.clone(), v.clone())));

    if let Some(steps) = script.steps() {
        run_steps(workspace, steps, &env_vars, cli_filters, visited, depth).await?;
    } else if let Some(exec_cmd) = script.exec_command() {
        // Mode 2: Exec config (per-package execution via config, not string parsing)
        run_exec_config_script(workspace, script, exec_cmd, &env_vars, cli_filters).await?;
    } else if let Some(run_command) = script.run_command() {
        // Mode 3: Traditional run command
        let substituted =
            normalize_line_continuations(&substitute_env_vars(run_command, &env_vars));

        if is_exec_command(&substituted) {
            // Legacy exec-style: `melos exec -- <command>` in run string
            run_exec_script(workspace, script, &substituted, &env_vars, cli_filters).await?;
        } else {
            // Regular shell command at workspace root
            let expanded = expand_command(&substituted)?;
            for cmd in &expanded {
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

                let (shell, shell_flag) = crate::runner::shell_command();
                let status = tokio::process::Command::new(shell)
                    .arg(shell_flag)
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
    } else {
        bail!(
            "Script '{}' has no runnable configuration (no `run`, `exec`, or `steps` defined)",
            script_name
        );
    }

    // Remove from visited so the same script can appear in separate chains
    // (e.g. A -> B, A -> C -> B is fine; A -> B -> A is a cycle)
    visited.remove(script_name);

    Ok(())
}

/// Execute a multi-step script workflow.
///
/// Each step is either:
/// 1. A script name reference (if it matches a script in the config) → execute inline
/// 2. A shell command → execute at workspace root
async fn run_steps(
    workspace: &Workspace,
    steps: &[String],
    env_vars: &HashMap<String, String>,
    cli_filters: &PackageFilters,
    visited: &mut HashSet<String>,
    depth: usize,
) -> Result<()> {
    for (i, step) in steps.iter().enumerate() {
        let step = step.trim();
        if step.is_empty() {
            continue;
        }

        println!(
            "{}Step {}/{}: {}",
            "  ".repeat(depth),
            i + 1,
            steps.len(),
            step.bold()
        );

        if workspace.config.scripts.contains_key(step) {
            Box::pin(run_script_recursive(
                workspace,
                step,
                cli_filters,
                visited,
                depth + 1,
            ))
            .await?;
        } else {
            let substituted = substitute_env_vars(step, env_vars);
            let expanded = expand_command(&substituted)?;

            for cmd in &expanded {
                println!(
                    "{}{} {}",
                    "  ".repeat(depth + 1),
                    ">".dimmed(),
                    cmd.dimmed()
                );

                let (shell, shell_flag) = crate::runner::shell_command();
                let status = tokio::process::Command::new(shell)
                    .arg(shell_flag)
                    .arg(cmd)
                    .current_dir(&workspace.root_path)
                    .envs(env_vars)
                    .status()
                    .await?;

                if !status.success() {
                    bail!(
                        "Step '{}' failed with exit code: {}",
                        step,
                        status.code().unwrap_or(-1)
                    );
                }
            }
        }
    }

    Ok(())
}

/// Run a script that uses exec config (not string-parsed `melos exec` style).
///
/// The exec command comes from the config's `exec:` field, and options
/// come from `ExecOptions` (concurrency, failFast, orderDependents).
async fn run_exec_config_script(
    workspace: &Workspace,
    script: &ScriptEntry,
    exec_command: &str,
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

    let concurrency = script
        .exec_options()
        .and_then(|o| o.concurrency)
        .unwrap_or(5);
    let fail_fast = script.exec_options().is_some_and(|o| o.fail_fast);
    let order_dependents = script.exec_options().is_some_and(|o| o.order_dependents);

    if order_dependents {
        packages = topological_sort(&packages);
        println!(
            "{} Packages ordered by dependencies (topological sort)\n",
            "i".blue()
        );
    }

    println!(
        "Running in {} package(s) with concurrency {}:\n",
        packages.len().to_string().cyan(),
        concurrency.to_string().cyan(),
    );
    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // Substitute env vars in the exec command
    let substituted = substitute_env_vars(exec_command, env_vars);

    let runner = ProcessRunner::new(concurrency, fail_fast);
    let results = runner
        .run_in_packages(&packages, &substituted, env_vars, None, &workspace.packages)
        .await?;

    let failed = results.iter().filter(|(_, success)| !success).count();
    if failed > 0 {
        bail!("{} package(s) failed", failed);
    }

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
    file_exists: Option<String>,
}

impl Default for ExecFlags {
    fn default() -> Self {
        Self {
            concurrency: 5, // Melos default
            fail_fast: false,
            order_dependents: false,
            timeout: None,
            dry_run: false,
            file_exists: None,
        }
    }
}

/// Parse all exec flags from a `melos exec [flags] -- <command>` string.
///
/// Recognizes: `-c N` / `--concurrency N`, `--fail-fast`, `--order-dependents`,
/// `--timeout N`, `--dry-run`, `--file-exists[=]<path>`.
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
            "--file-exists" => {
                // Space-separated form: --file-exists pubspec.yaml
                if i + 1 < parts.len() {
                    flags.file_exists = Some(strip_outer_quotes(parts[i + 1]).to_string());
                    i += 1;
                }
            }
            s if s.starts_with("--file-exists=") => {
                // Equals form: --file-exists="pubspec.yaml" or --file-exists=pubspec.yaml
                let value = &s["--file-exists=".len()..];
                flags.file_exists = Some(strip_outer_quotes(value).to_string());
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
    let mut filters = if let Some(script_filters) = script.package_filters() {
        script_filters.merge(cli_filters)
    } else {
        cli_filters.clone()
    };

    let flags = parse_exec_flags(command);

    // Apply inline --file-exists from the exec command string when not already
    // set by packageFilters or CLI filters (inline flag is lowest priority)
    if filters.file_exists.is_none()
        && let Some(ref fe) = flags.file_exists
    {
        filters.file_exists = Some(fe.clone());
    }

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
        .run_in_packages(
            &packages,
            &actual_cmd,
            env_vars,
            flags.timeout,
            &workspace.packages,
        )
        .await?;

    let failed = results.iter().filter(|(_, success)| !success).count();
    if failed > 0 {
        bail!("{} package(s) failed", failed);
    }

    Ok(())
}

/// Prompt the user to select a script interactively from available scripts
fn select_script_interactive(
    workspace: &Workspace,
    include_private: bool,
    groups: &[String],
) -> Result<String> {
    let scripts: Vec<(&String, &ScriptEntry)> = workspace
        .config
        .scripts
        .iter()
        .filter(|(_, entry)| include_private || !entry.is_private())
        .filter(|(_, entry)| groups.is_empty() || groups.iter().any(|g| entry.in_group(g)))
        .collect();

    if scripts.is_empty() {
        if !include_private && workspace.config.scripts.values().any(|e| e.is_private()) {
            bail!(
                "No scripts available (all scripts are private). Use --include-private to see them."
            );
        }
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
    io::stdin().lock().read_line(&mut input)?;
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

/// Extract the actual command from a `melos exec -- <command>` string.
///
/// The command after `--` may be wrapped in quotes in the YAML source
/// (e.g. `-- "flutter pub upgrade && exit"`). Because `split_whitespace`
/// does not understand quoting, the leading and trailing quote characters
/// end up as part of the first/last tokens. [`strip_outer_quotes`] removes
/// them so the shell receives a plain command string.
fn extract_exec_command(command: &str) -> String {
    // Look for standalone `--` separator (space-delimited token, not just any `--` prefix)
    let parts: Vec<&str> = command.split_whitespace().collect();
    if let Some(pos) = parts.iter().position(|&p| p == "--") {
        let joined = parts[pos + 1..].join(" ");
        return strip_outer_quotes(&joined);
    }

    // Fallback: strip `melos exec` / `melos-rs exec` prefix and all known flags
    let stripped = command
        .replace("melos-rs exec", "")
        .replace("melos exec", "");

    // Remove known flags like -c N, --fail-fast, --order-dependents, --timeout N, --dry-run,
    // --file-exists[=]<value>
    let parts: Vec<&str> = stripped.split_whitespace().collect();
    let mut result = Vec::new();
    let mut skip_next = false;

    for part in &parts {
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(
            *part,
            "-c" | "--concurrency" | "--timeout" | "--file-exists"
        ) {
            skip_next = true;
            continue;
        }
        if matches!(*part, "--fail-fast" | "--order-dependents" | "--dry-run") {
            continue;
        }
        if part.starts_with("--file-exists=") {
            continue;
        }
        result.push(*part);
    }

    let joined = result.join(" ");
    strip_outer_quotes(joined.trim())
}

/// Strip matching outer quote characters (double or single) from a string.
///
/// When YAML contains `-- "flutter pub upgrade && exit"`, the quotes are
/// literal characters in the plain scalar. After `split_whitespace` + `join`,
/// they appear as `"flutter pub upgrade && exit"`. The shell would treat the
/// quoted content as a single word (command name), causing "command not found".
/// Stripping the outer quotes produces a plain command string the shell
/// interprets correctly.
fn strip_outer_quotes(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        return trimmed[1..trimmed.len() - 1].to_string();
    }
    trimmed.to_string()
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

/// Normalize shell line continuations in a command string.
///
/// YAML literal block scalars (`|`) preserve newlines and backslash characters
/// literally. A script like:
/// ```yaml
/// run: |
///   melos exec -c 1 -- \
///     flutter analyze .
/// ```
/// produces the string `"melos exec -c 1 -- \\\n    flutter analyze .\n"`.
/// The `\<newline>` is a shell line continuation that should collapse into a
/// single space so that downstream parsing (split_whitespace, etc.) does not
/// treat the backslash as part of the command.
fn normalize_line_continuations(command: &str) -> String {
    let mut result = String::with_capacity(command.len());
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'\n') {
            // Consume the newline and any leading whitespace on the next line
            chars.next(); // skip '\n'
            while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
                chars.next();
            }
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
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
                    // Group 0 (the full match) is always present in a Captures
                    let m = caps.get(0).expect("regex group 0 always exists");
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
                // Group 0 (the full match) is always present in a Captures
                let m = caps.get(0).expect("regex group 0 always exists");
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
        assert_eq!(result, vec!["melos-rs run generate", "melos-rs run build"]);
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
        assert_eq!(substitute_env_vars("path=$MELOS_ROOT", &env), "path=/root");
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
            extract_exec_command(
                "melos exec --order-dependents --dry-run --timeout 30 flutter test"
            ),
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
        assert!(flags.file_exists.is_none());
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
            "melos exec -c 2 --fail-fast --order-dependents --timeout 60 --dry-run -- flutter test",
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

    // ── --file-exists flag parsing tests ────────────────────────────────

    #[test]
    fn test_parse_exec_flags_file_exists_equals_quoted() {
        // Real-world: --file-exists="pubspec.yaml" (quotes are literal YAML chars)
        let flags = parse_exec_flags(
            r#"melos exec --file-exists="pubspec.yaml" -c 1 --fail-fast -- "flutter pub upgrade && exit""#,
        );
        assert_eq!(flags.file_exists, Some("pubspec.yaml".to_string()));
        assert_eq!(flags.concurrency, 1);
        assert!(flags.fail_fast);
    }

    #[test]
    fn test_parse_exec_flags_file_exists_equals_unquoted() {
        let flags = parse_exec_flags("melos exec --file-exists=pubspec.yaml -- flutter test");
        assert_eq!(flags.file_exists, Some("pubspec.yaml".to_string()));
    }

    #[test]
    fn test_parse_exec_flags_file_exists_space_separated() {
        let flags = parse_exec_flags("melos exec --file-exists pubspec.yaml -- flutter test");
        assert_eq!(flags.file_exists, Some("pubspec.yaml".to_string()));
    }

    #[test]
    fn test_parse_exec_flags_file_exists_single_quoted() {
        let flags =
            parse_exec_flags("melos exec --file-exists='test/widget_test.dart' -- flutter test");
        assert_eq!(flags.file_exists, Some("test/widget_test.dart".to_string()));
    }

    #[test]
    fn test_parse_exec_flags_no_file_exists() {
        let flags = parse_exec_flags("melos exec -c 3 --fail-fast -- flutter test");
        assert!(flags.file_exists.is_none());
    }

    #[test]
    fn test_extract_exec_command_strips_file_exists_equals() {
        // Fallback path (no -- separator): --file-exists=<val> should be stripped
        assert_eq!(
            extract_exec_command("melos exec --file-exists=pubspec.yaml --fail-fast flutter test"),
            "flutter test"
        );
    }

    #[test]
    fn test_extract_exec_command_strips_file_exists_space() {
        // Fallback path: --file-exists <val> (space-separated) should be stripped
        assert_eq!(
            extract_exec_command("melos exec --file-exists pubspec.yaml --fail-fast flutter test"),
            "flutter test"
        );
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

    // ── normalize_line_continuations tests ──────────────────────────────

    #[test]
    fn test_normalize_line_continuations_basic() {
        // Backslash-newline is collapsed to a single space
        assert_eq!(
            normalize_line_continuations("melos exec -c 1 -- \\\n    flutter analyze ."),
            "melos exec -c 1 --  flutter analyze ."
        );
    }

    #[test]
    fn test_normalize_line_continuations_multiline_yaml() {
        // Simulates what YAML `|` produces for the analyze script
        let yaml_string = "melos exec -c 1 -- \\\n  flutter analyze .\n";
        let normalized = normalize_line_continuations(yaml_string);
        assert_eq!(normalized, "melos exec -c 1 --  flutter analyze .\n");
        // After normalization, extract_exec_command should work correctly
        assert_eq!(extract_exec_command(&normalized), "flutter analyze .");
    }

    #[test]
    fn test_normalize_line_continuations_no_continuations() {
        let cmd = "melos exec -c 1 -- flutter analyze .";
        assert_eq!(normalize_line_continuations(cmd), cmd);
    }

    #[test]
    fn test_normalize_line_continuations_backslash_not_before_newline() {
        // Backslash not followed by newline should be preserved
        let cmd = "echo hello\\ world";
        assert_eq!(normalize_line_continuations(cmd), cmd);
    }

    #[test]
    fn test_normalize_line_continuations_multiple() {
        let cmd = "first \\\n  second \\\n  third";
        assert_eq!(normalize_line_continuations(cmd), "first  second  third");
    }

    // --- strip_outer_quotes tests ---

    #[test]
    fn test_strip_outer_quotes_double() {
        assert_eq!(
            strip_outer_quotes("\"flutter pub upgrade && exit\""),
            "flutter pub upgrade && exit"
        );
    }

    #[test]
    fn test_strip_outer_quotes_single() {
        assert_eq!(
            strip_outer_quotes("'flutter pub upgrade && exit'"),
            "flutter pub upgrade && exit"
        );
    }

    #[test]
    fn test_strip_outer_quotes_no_quotes() {
        assert_eq!(
            strip_outer_quotes("flutter pub upgrade"),
            "flutter pub upgrade"
        );
    }

    #[test]
    fn test_strip_outer_quotes_mismatched() {
        // Mismatched quotes should not be stripped
        assert_eq!(
            strip_outer_quotes("\"flutter pub upgrade'"),
            "\"flutter pub upgrade'"
        );
    }

    #[test]
    fn test_strip_outer_quotes_single_char() {
        // Edge case: single quote char alone
        assert_eq!(strip_outer_quotes("\""), "\"");
    }

    // --- extract_exec_command with quotes tests ---

    #[test]
    fn test_extract_exec_command_strips_double_quotes() {
        // Real-world case: melos.yaml has -- "flutter pub upgrade && exit"
        assert_eq!(
            extract_exec_command(
                "melos exec --file-exists=\"pubspec.yaml\" -c 1 --fail-fast -- \"flutter pub upgrade && exit\""
            ),
            "flutter pub upgrade && exit"
        );
    }

    #[test]
    fn test_extract_exec_command_strips_single_quotes() {
        assert_eq!(
            extract_exec_command("melos exec -c 1 -- 'flutter pub upgrade && exit'"),
            "flutter pub upgrade && exit"
        );
    }

    #[test]
    fn test_extract_exec_command_no_quotes_preserved() {
        // No quotes: command should be returned as-is
        assert_eq!(
            extract_exec_command("melos exec -- flutter test"),
            "flutter test"
        );
    }
}
