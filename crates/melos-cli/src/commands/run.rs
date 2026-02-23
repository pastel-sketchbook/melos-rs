use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};

use anyhow::{Result, bail};
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use melos_core::commands::run::{
    MAX_SCRIPT_DEPTH, expand_command, extract_exec_command, extract_melos_run_script_name,
    is_exec_command, normalize_line_continuations, parse_exec_flags, substitute_env_vars,
};
use melos_core::config::ScriptEntry;
use melos_core::config::filter::PackageFilters;
use melos_core::package::Package;
use melos_core::package::filter::{apply_filters_with_categories, topological_sort};
use melos_core::runner::ProcessRunner;
use melos_core::watcher;
use melos_core::workspace::Workspace;

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
    let cli_filters = package_filters_from_args(&args.filters);

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

    println!(
        "\n{} Watching {} package(s) for changes...",
        "i".blue(),
        watch_packages.len()
    );

    let watch_pkgs_clone: Vec<Package> = watch_packages.to_vec();

    let watcher_handle = tokio::task::spawn_blocking(move || {
        watcher::start_watching(&watch_pkgs_clone, 0, event_tx, shutdown_rx, None)
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
            let mode = match (entry.steps().is_some(), entry.has_exec_config()) {
                (true, _) => format!(" {}", "(steps)".dimmed()),
                (_, true) => format!(" {}", "(exec)".dimmed()),
                _ => String::new(),
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

                let (shell, shell_flag) = melos_core::runner::shell_command();
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

                let (shell, shell_flag) = melos_core::runner::shell_command();
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

    let (tx, render_handle) = crate::render::spawn_plain_renderer();
    let runner = ProcessRunner::new(concurrency, fail_fast);
    let results = runner
        .run_in_packages_with_events(
            &packages,
            &substituted,
            env_vars,
            None,
            Some(&tx),
            &workspace.packages,
        )
        .await?;
    drop(tx);
    render_handle.await??;

    let failed = results.iter().filter(|(_, success)| !success).count();
    if failed > 0 {
        bail!("{} package(s) failed", failed);
    }

    Ok(())
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

    let (tx, render_handle) = crate::render::spawn_plain_renderer();
    let runner = ProcessRunner::new(flags.concurrency, flags.fail_fast);
    let results = runner
        .run_in_packages_with_events(
            &packages,
            &actual_cmd,
            env_vars,
            flags.timeout,
            Some(&tx),
            &workspace.packages,
        )
        .await?;
    drop(tx);
    render_handle.await??;

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
