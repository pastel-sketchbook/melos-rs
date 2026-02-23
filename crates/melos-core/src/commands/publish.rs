use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use crate::events::Event;
use crate::package::Package;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

use super::PackageResults;

/// Options for the publish command (clap-free).
#[derive(Debug, Clone)]
pub struct PublishOpts {
    pub dry_run: bool,
    pub concurrency: usize,
}

/// Build the `dart pub publish` command string.
pub fn build_publish_command(dry_run: bool) -> String {
    let mut cmd = String::from("dart pub publish");
    if dry_run {
        cmd.push_str(" --dry-run");
    } else {
        // --force skips the pub.dev confirmation prompt
        cmd.push_str(" --force");
    }
    cmd
}

/// Build the git tag name for a published package.
pub fn build_git_tag(package_name: &str, version: &str) -> String {
    format!("{}-v{}", package_name, version)
}

/// Run `dart pub publish` across packages, emitting events for progress tracking.
///
/// Returns [`PackageResults`] with per-package success/failure status.
pub async fn run(
    packages: &[Package],
    workspace: &Workspace,
    opts: &PublishOpts,
    events: Option<&UnboundedSender<Event>>,
) -> Result<PackageResults> {
    let cmd = build_publish_command(opts.dry_run);
    let runner = ProcessRunner::new(opts.concurrency, false);
    let results = runner
        .run_in_packages_with_events(
            packages,
            &cmd,
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
    fn test_build_publish_command_dry_run() {
        let cmd = build_publish_command(true);
        assert_eq!(cmd, "dart pub publish --dry-run");
    }

    #[test]
    fn test_build_publish_command_real() {
        let cmd = build_publish_command(false);
        assert_eq!(cmd, "dart pub publish --force");
    }

    #[test]
    fn test_build_git_tag() {
        assert_eq!(build_git_tag("my_package", "1.2.3"), "my_package-v1.2.3");
    }

    #[test]
    fn test_build_git_tag_prerelease() {
        assert_eq!(build_git_tag("core", "2.0.0-beta.1"), "core-v2.0.0-beta.1");
    }

    #[test]
    fn test_build_git_tag_zero_version() {
        assert_eq!(build_git_tag("utils", "0.0.0"), "utils-v0.0.0");
    }
}
