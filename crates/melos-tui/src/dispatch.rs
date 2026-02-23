use std::sync::Arc;

use anyhow::{Result, bail};
use melos_core::commands::PackageResults;
use melos_core::commands::analyze::AnalyzeOpts;
use melos_core::commands::bootstrap::BootstrapOpts;
use melos_core::commands::clean::CleanOpts;
use melos_core::commands::format::FormatOpts;
use melos_core::commands::health::{HealthOpts, HealthReport};
use melos_core::commands::publish::PublishOpts;
use melos_core::commands::test::TestOpts;
use melos_core::events::Event;
use melos_core::workspace::Workspace;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::CommandOpts;

/// Result of a dispatched command, carrying optional structured data
/// alongside the standard package results.
pub struct DispatchResult {
    pub package_results: PackageResults,
    pub health_report: Option<HealthReport>,
}

/// Dispatch a command to the appropriate core `run()` function.
///
/// This is called from a spawned tokio task. The `tx` sender streams
/// `Event` values back to the TUI event loop for live progress display.
/// When this function returns (or the task is aborted), the sender is
/// dropped which signals the event loop that the command has finished.
///
/// If `opts` is provided, the user-configured options from the overlay
/// are used. Otherwise, sensible defaults are applied.
pub async fn dispatch_command(
    name: &str,
    workspace: &Arc<Workspace>,
    tx: UnboundedSender<Event>,
    opts: Option<CommandOpts>,
) -> Result<DispatchResult> {
    let packages = &workspace.packages;

    match name {
        "analyze" => {
            let core_opts = match opts {
                Some(CommandOpts::Analyze {
                    concurrency,
                    fatal_warnings,
                    fatal_infos,
                    no_fatal,
                }) => AnalyzeOpts {
                    concurrency,
                    fatal_warnings,
                    fatal_infos,
                    no_fatal,
                },
                _ => AnalyzeOpts {
                    concurrency: 1,
                    fatal_warnings: false,
                    fatal_infos: false,
                    no_fatal: false,
                },
            };
            let r = melos_core::commands::analyze::run(packages, workspace, &core_opts, Some(&tx)).await?;
            Ok(DispatchResult { package_results: r, health_report: None })
        }
        "bootstrap" => {
            let core_opts = match opts {
                Some(CommandOpts::Bootstrap {
                    concurrency,
                    enforce_lockfile,
                    offline,
                    no_example,
                }) => BootstrapOpts {
                    concurrency,
                    enforce_lockfile,
                    no_example,
                    offline,
                },
                _ => BootstrapOpts {
                    concurrency: 1,
                    enforce_lockfile: false,
                    no_example: false,
                    offline: false,
                },
            };
            let r = melos_core::commands::bootstrap::run(packages, workspace, &core_opts, Some(&tx)).await?;
            Ok(DispatchResult { package_results: r, health_report: None })
        }
        "clean" => {
            let core_opts = match opts {
                Some(CommandOpts::Clean { concurrency }) => CleanOpts { concurrency },
                _ => CleanOpts { concurrency: 1 },
            };
            let r = melos_core::commands::clean::run(packages, workspace, &core_opts, Some(&tx)).await?;
            Ok(DispatchResult { package_results: r, health_report: None })
        }
        "format" => {
            let core_opts = match opts {
                Some(CommandOpts::Format {
                    concurrency,
                    set_exit_if_changed,
                    line_length,
                }) => FormatOpts {
                    concurrency,
                    set_exit_if_changed,
                    output: "show".to_string(),
                    line_length,
                },
                _ => FormatOpts {
                    concurrency: 1,
                    set_exit_if_changed: false,
                    output: "show".to_string(),
                    line_length: None,
                },
            };
            let r = melos_core::commands::format::run(packages, workspace, &core_opts, Some(&tx)).await?;
            Ok(DispatchResult { package_results: r, health_report: None })
        }
        "test" => {
            let core_opts = match opts {
                Some(CommandOpts::Test {
                    concurrency,
                    fail_fast,
                    coverage,
                    update_goldens,
                    no_run,
                }) => TestOpts {
                    concurrency,
                    fail_fast,
                    coverage,
                    test_randomize_ordering_seed: None,
                    update_goldens,
                    no_run,
                    extra_args: vec![],
                },
                _ => TestOpts {
                    concurrency: 1,
                    fail_fast: false,
                    coverage: false,
                    test_randomize_ordering_seed: None,
                    update_goldens: false,
                    no_run: false,
                    extra_args: vec![],
                },
            };
            let r = melos_core::commands::test::run(packages, workspace, &core_opts, Some(&tx)).await?;
            Ok(DispatchResult { package_results: r, health_report: None })
        }
        "publish" => {
            let core_opts = match opts {
                Some(CommandOpts::Publish {
                    concurrency,
                    dry_run,
                }) => PublishOpts {
                    concurrency,
                    dry_run,
                },
                _ => PublishOpts {
                    dry_run: true,
                    concurrency: 1,
                },
            };
            let r = melos_core::commands::publish::run(packages, workspace, &core_opts, Some(&tx)).await?;
            Ok(DispatchResult { package_results: r, health_report: None })
        }
        "health" => {
            let health_opts = match opts {
                Some(CommandOpts::Health {
                    version_drift,
                    missing_fields,
                    sdk_consistency,
                }) => HealthOpts {
                    version_drift,
                    missing_fields,
                    sdk_consistency,
                    all: false,
                    json: false,
                },
                _ => HealthOpts {
                    version_drift: true,
                    missing_fields: true,
                    sdk_consistency: true,
                    all: false,
                    json: false,
                },
            };
            dispatch_health(packages, &health_opts, &tx)
        }
        "exec" => {
            bail!("exec requires a command argument (not yet supported in TUI)")
        }
        "build" | "list" | "run" | "version" => {
            bail!("{name} is not yet wired for TUI execution")
        }
        other => {
            bail!("unknown command: {other}")
        }
    }
}

/// Dispatch the health command, which is synchronous.
///
/// Wraps the sync `health::run()` and emits events to match the async pattern
/// so the TUI shows progress and results consistently. Returns the structured
/// `HealthReport` alongside empty package results for dashboard rendering.
fn dispatch_health(
    packages: &[melos_core::package::Package],
    opts: &HealthOpts,
    tx: &UnboundedSender<Event>,
) -> Result<DispatchResult> {
    use std::time::Instant;

    let _ = tx.send(Event::CommandStarted {
        command: "health".to_string(),
        package_count: packages.len(),
    });

    let start = Instant::now();
    let report: HealthReport = melos_core::commands::health::run(packages, opts);

    // Emit summary as info events.
    if report.total_issues == 0 {
        let _ = tx.send(Event::Info("No health issues found.".to_string()));
    } else {
        if let Some(ref drifts) = report.version_drift {
            for drift in drifts {
                let _ = tx.send(Event::Warning(format!(
                    "version drift: {} has {} different constraints",
                    drift.dependency,
                    drift.constraints.len()
                )));
            }
        }
        if let Some(ref missing_list) = report.missing_fields {
            for missing in missing_list {
                let _ = tx.send(Event::Warning(format!(
                    "missing fields in {}: {}",
                    missing.package,
                    missing.missing.join(", ")
                )));
            }
        }
        if let Some(ref sdk) = report.sdk_consistency
            && !sdk.missing_sdk.is_empty()
        {
            let _ = tx.send(Event::Warning(format!(
                "packages missing sdk constraint: {}",
                sdk.missing_sdk.join(", ")
            )));
        }
    }

    let duration = start.elapsed();
    let _ = tx.send(Event::CommandFinished {
        command: "health".to_string(),
        duration,
    });

    Ok(DispatchResult {
        package_results: PackageResults {
            results: Vec::new(),
        },
        health_report: Some(report),
    })
}
