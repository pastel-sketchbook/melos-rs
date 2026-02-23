use std::collections::HashSet;

use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use melos_core::commands::exec::ExecOpts;
use melos_core::package::Package;
use melos_core::package::filter::{apply_filters_with_categories, topological_sort};
use melos_core::watcher;
use melos_core::workspace::Workspace;

/// Arguments for the `exec` command
#[derive(Args, Debug)]
pub struct ExecArgs {
    /// Command to execute in each package
    #[arg(trailing_var_arg = true, required = true)]
    pub command: Vec<String>,

    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "5")]
    pub concurrency: usize,

    /// Stop execution on first failure
    #[arg(long)]
    pub fail_fast: bool,

    /// Execute packages in dependency order (topological sort)
    #[arg(long)]
    pub order_dependents: bool,

    /// Timeout per package in seconds (0 = no timeout)
    #[arg(long, default_value = "0")]
    pub timeout: u64,

    /// Print commands without executing them
    #[arg(long)]
    pub dry_run: bool,

    /// Watch for file changes and re-run on change
    #[arg(long)]
    pub watch: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Execute a command across all matching packages
pub async fn run(workspace: &Workspace, args: ExecArgs) -> Result<()> {
    let cmd_str = args.command.join(" ");
    let watch_mode = args.watch;

    let filters = package_filters_from_args(&args.filters);
    let mut packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    if args.order_dependents {
        packages = topological_sort(&packages);
    }

    // Initial run
    run_exec_once(&cmd_str, &packages, &args, workspace).await?;

    // If watch mode, start watching and re-run on changes
    if watch_mode {
        run_watch_loop(&cmd_str, &packages, &args, workspace).await?;
    }

    Ok(())
}

/// Execute the command once across the given packages.
///
/// Returns Ok(()) even if some packages fail (the error count is printed).
/// Only returns Err if watch mode is NOT active and packages failed.
async fn run_exec_once(
    cmd_str: &str,
    packages: &[Package],
    args: &ExecArgs,
    workspace: &Workspace,
) -> Result<()> {
    println!(
        "\n{} Running '{}' in packages...\n",
        "$".cyan(),
        cmd_str.bold()
    );

    if args.order_dependents {
        println!(
            "{} Packages ordered by dependencies (topological sort)\n",
            "i".blue()
        );
    }

    let timeout_display = if args.timeout > 0 {
        format!(", timeout {}s", args.timeout)
    } else {
        String::new()
    };

    println!(
        "Running in {} package(s) with concurrency {}{}:\n",
        packages.len().to_string().cyan(),
        args.concurrency.to_string().cyan(),
        timeout_display,
    );

    for pkg in packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // Dry-run mode: show what would be executed without running
    if args.dry_run {
        println!("{}", "DRY RUN — no commands were executed.".yellow().bold());
        return Ok(());
    }

    let timeout = if args.timeout > 0 {
        Some(std::time::Duration::from_secs(args.timeout))
    } else {
        None
    };

    let opts = ExecOpts {
        command: cmd_str.to_string(),
        concurrency: args.concurrency,
        fail_fast: args.fail_fast,
        timeout,
    };

    let (tx, render_handle) = crate::render::spawn_renderer(packages.len(), "exec");
    let results = melos_core::commands::exec::run(packages, workspace, &opts, Some(&tx)).await?;
    drop(tx);
    render_handle.await??;

    if results.failed() > 0 {
        if args.watch {
            eprintln!(
                "\n{} {} package(s) failed. Watching for changes...",
                "!".yellow().bold(),
                results.failed()
            );
        } else {
            anyhow::bail!(
                "{} package(s) failed exec ({} passed)",
                results.failed(),
                results.passed()
            );
        }
    } else if !args.watch {
        println!(
            "\n{}",
            format!("All {} package(s) passed exec.", results.passed()).green()
        );
    }

    Ok(())
}

/// Run the watch loop: wait for file changes, then re-execute in affected packages.
async fn run_watch_loop(
    cmd_str: &str,
    packages: &[Package],
    args: &ExecArgs,
    workspace: &Workspace,
) -> Result<()> {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    println!(
        "\n{} Watching {} package(s) for changes...",
        "i".blue(),
        packages.len()
    );

    let watch_packages: Vec<Package> = packages.to_vec();

    let watcher_handle = tokio::task::spawn_blocking(move || {
        watcher::start_watching(&watch_packages, 0, event_tx, shutdown_rx, None)
    });

    let shutdown_tx_ctrlc = shutdown_tx.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            println!("\n{} Stopping watcher...", "!".yellow());
            let _ = shutdown_tx_ctrlc.send(()).await;
        }
    });

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

        let affected: Vec<Package> = packages
            .iter()
            .filter(|p| changed_packages.contains(&p.name))
            .cloned()
            .collect();

        if affected.is_empty() {
            continue;
        }

        println!(
            "\n{} Changes detected in: {}\n",
            "↻".cyan().bold(),
            watcher::format_changed_packages(&changed_packages).bold(),
        );

        let timeout = if args.timeout > 0 {
            Some(std::time::Duration::from_secs(args.timeout))
        } else {
            None
        };

        let opts = ExecOpts {
            command: cmd_str.to_string(),
            concurrency: args.concurrency,
            fail_fast: args.fail_fast,
            timeout,
        };

        let (tx, render_handle) = crate::render::spawn_renderer(affected.len(), "exec");
        let result = melos_core::commands::exec::run(&affected, workspace, &opts, Some(&tx)).await;
        drop(tx);
        let _ = render_handle.await;

        match result {
            Ok(ref r) => {
                if r.failed() > 0 {
                    eprintln!(
                        "\n{} {} package(s) failed. Watching for changes...",
                        "!".yellow().bold(),
                        r.failed()
                    );
                } else {
                    println!(
                        "\n{} All packages succeeded. Watching for changes...",
                        "✓".green().bold(),
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "\n{} Execution error: {}. Watching for changes...",
                    "!".red().bold(),
                    e,
                );
            }
        }
    }

    let _ = shutdown_tx.send(()).await;
    let _ = watcher_handle.await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_args_defaults() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: ExecArgs,
        }

        let cli = TestCli::parse_from(["test", "flutter", "test"]);
        assert_eq!(cli.args.command, vec!["flutter", "test"]);
        assert_eq!(cli.args.concurrency, 5);
        assert!(!cli.args.fail_fast);
        assert!(!cli.args.order_dependents);
        assert_eq!(cli.args.timeout, 0);
        assert!(!cli.args.dry_run);
        assert!(!cli.args.watch);
    }

    #[test]
    fn test_exec_args_watch_flag() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: ExecArgs,
        }

        let cli = TestCli::parse_from(["test", "--watch", "flutter", "test"]);
        assert!(cli.args.watch);
        assert_eq!(cli.args.command, vec!["flutter", "test"]);
    }

    #[test]
    fn test_exec_args_all_flags() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: ExecArgs,
        }

        let cli = TestCli::parse_from([
            "test",
            "--watch",
            "--fail-fast",
            "--order-dependents",
            "--dry-run",
            "-c",
            "3",
            "--timeout",
            "60",
            "dart",
            "analyze",
            ".",
        ]);
        assert!(cli.args.watch);
        assert!(cli.args.fail_fast);
        assert!(cli.args.order_dependents);
        assert!(cli.args.dry_run);
        assert_eq!(cli.args.concurrency, 3);
        assert_eq!(cli.args.timeout, 60);
        assert_eq!(cli.args.command, vec!["dart", "analyze", "."]);
    }
}
