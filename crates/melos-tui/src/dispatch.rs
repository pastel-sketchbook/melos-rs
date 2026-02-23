use std::sync::Arc;

use anyhow::{Result, bail};
use melos_core::commands::PackageResults;
use melos_core::commands::analyze::AnalyzeOpts;
use melos_core::commands::format::FormatOpts;
use melos_core::commands::publish::PublishOpts;
use melos_core::commands::test::TestOpts;
use melos_core::events::Event;
use melos_core::workspace::Workspace;
use tokio::sync::mpsc::UnboundedSender;

/// Dispatch a command to the appropriate core `run()` function.
///
/// This is called from a spawned tokio task. The `tx` sender streams
/// `Event` values back to the TUI event loop for live progress display.
/// When this function returns (or the task is aborted), the sender is
/// dropped which signals the event loop that the command has finished.
pub async fn dispatch_command(
    name: &str,
    workspace: &Arc<Workspace>,
    tx: UnboundedSender<Event>,
) -> Result<PackageResults> {
    let packages = &workspace.packages;

    match name {
        "analyze" => {
            let opts = AnalyzeOpts {
                concurrency: 1,
                fatal_warnings: false,
                fatal_infos: false,
                no_fatal: false,
            };
            melos_core::commands::analyze::run(packages, workspace, &opts, Some(&tx)).await
        }
        "format" => {
            let opts = FormatOpts {
                concurrency: 1,
                set_exit_if_changed: false,
                output: "show".to_string(),
                line_length: None,
            };
            melos_core::commands::format::run(packages, workspace, &opts, Some(&tx)).await
        }
        "test" => {
            let opts = TestOpts {
                concurrency: 1,
                fail_fast: false,
                coverage: false,
                test_randomize_ordering_seed: None,
                update_goldens: false,
                no_run: false,
                extra_args: vec![],
            };
            melos_core::commands::test::run(packages, workspace, &opts, Some(&tx)).await
        }
        "publish" => {
            let opts = PublishOpts {
                dry_run: true,
                concurrency: 1,
            };
            melos_core::commands::publish::run(packages, workspace, &opts, Some(&tx)).await
        }
        "exec" => {
            bail!("exec requires a command argument (not yet supported in TUI)")
        }
        "bootstrap" | "build" | "clean" | "health" | "list" | "run" | "version" => {
            bail!("{name} is not yet wired for TUI execution")
        }
        other => {
            bail!("unknown command: {other}")
        }
    }
}
