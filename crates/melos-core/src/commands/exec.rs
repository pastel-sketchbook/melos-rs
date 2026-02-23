use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use crate::events::Event;
use crate::package::Package;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

use super::PackageResults;

/// Options for the exec command (clap-free).
#[derive(Debug, Clone)]
pub struct ExecOpts {
    pub command: String,
    pub concurrency: usize,
    pub fail_fast: bool,
    pub timeout: Option<Duration>,
}

/// Execute a shell command across packages, emitting events for progress tracking.
///
/// Returns [`PackageResults`] with per-package success/failure status.
pub async fn run(
    packages: &[Package],
    workspace: &Workspace,
    opts: &ExecOpts,
    events: Option<&UnboundedSender<Event>>,
) -> Result<PackageResults> {
    let runner = ProcessRunner::new(opts.concurrency, opts.fail_fast);
    let results = runner
        .run_in_packages_with_events(
            packages,
            &opts.command,
            &workspace.env_vars(),
            opts.timeout,
            events,
            &workspace.packages,
        )
        .await?;
    Ok(PackageResults::from(results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_opts_defaults() {
        let opts = ExecOpts {
            command: "echo hello".to_string(),
            concurrency: 5,
            fail_fast: false,
            timeout: None,
        };
        assert_eq!(opts.command, "echo hello");
        assert_eq!(opts.concurrency, 5);
        assert!(!opts.fail_fast);
        assert!(opts.timeout.is_none());
    }

    #[test]
    fn test_exec_opts_with_timeout() {
        let opts = ExecOpts {
            command: "dart test".to_string(),
            concurrency: 3,
            fail_fast: true,
            timeout: Some(Duration::from_secs(60)),
        };
        assert_eq!(opts.timeout, Some(Duration::from_secs(60)));
        assert!(opts.fail_fast);
    }
}
