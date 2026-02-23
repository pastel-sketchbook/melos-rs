use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use crate::events::Event;
use crate::package::Package;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

use super::PackageResults;

/// Options for the format command (clap-free).
#[derive(Debug, Clone)]
pub struct FormatOpts {
    pub concurrency: usize,
    pub set_exit_if_changed: bool,
    pub output: String,
    pub line_length: Option<u32>,
}

/// Build the `dart format` command string from flags.
pub fn build_format_command(
    set_exit_if_changed: bool,
    output: &str,
    line_length: Option<u32>,
) -> String {
    let mut cmd_parts = vec!["dart".to_string(), "format".to_string()];

    if set_exit_if_changed {
        cmd_parts.push("--set-exit-if-changed".to_string());
    }

    if output != "write" {
        cmd_parts.push(format!("--output={}", output));
    }

    if let Some(line_length) = line_length {
        cmd_parts.push(format!("--line-length={}", line_length));
    }

    // Format the current directory (package root)
    cmd_parts.push(".".to_string());

    cmd_parts.join(" ")
}

/// Run `dart format` across packages, emitting events for progress tracking.
///
/// Returns [`PackageResults`] with per-package success/failure status.
pub async fn run(
    packages: &[Package],
    workspace: &Workspace,
    opts: &FormatOpts,
    events: Option<&UnboundedSender<Event>>,
) -> Result<PackageResults> {
    let cmd_str = build_format_command(opts.set_exit_if_changed, &opts.output, opts.line_length);
    let runner = ProcessRunner::new(opts.concurrency, false);
    let results = runner
        .run_in_packages_with_events(
            packages,
            &cmd_str,
            &workspace.env_vars(),
            None,
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
    fn test_build_format_command_default() {
        let cmd = build_format_command(false, "write", None);
        assert_eq!(cmd, "dart format .");
    }

    #[test]
    fn test_build_format_command_set_exit_if_changed() {
        let cmd = build_format_command(true, "write", None);
        assert_eq!(cmd, "dart format --set-exit-if-changed .");
    }

    #[test]
    fn test_build_format_command_json_output() {
        let cmd = build_format_command(false, "json", None);
        assert_eq!(cmd, "dart format --output=json .");
    }

    #[test]
    fn test_build_format_command_none_output() {
        let cmd = build_format_command(false, "none", None);
        assert_eq!(cmd, "dart format --output=none .");
    }

    #[test]
    fn test_build_format_command_line_length() {
        let cmd = build_format_command(false, "write", Some(120));
        assert_eq!(cmd, "dart format --line-length=120 .");
    }

    #[test]
    fn test_build_format_command_all_flags() {
        let cmd = build_format_command(true, "json", Some(80));
        assert_eq!(
            cmd,
            "dart format --set-exit-if-changed --output=json --line-length=80 ."
        );
    }

    #[test]
    fn test_build_format_command_write_output_not_added() {
        // "write" is the default and should not be added to the command
        let cmd = build_format_command(false, "write", None);
        assert!(!cmd.contains("--output"));
    }
}
