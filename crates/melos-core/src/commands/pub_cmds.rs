use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use crate::events::Event;
use crate::package::Package;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

use super::PackageResults;

/// Which `pub` sub-subcommand to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubSubcommand {
    Get,
    Outdated,
    Upgrade,
    Downgrade,
    Add {
        package: String,
        dev: bool,
    },
    Remove {
        package: String,
    },
}

/// Options for the `pub` command (clap-free).
#[derive(Debug, Clone)]
pub struct PubOpts {
    pub subcommand: PubSubcommand,
    pub concurrency: usize,
    /// For `Upgrade` only: pass `--major-versions`.
    pub major_versions: bool,
}

/// Return the appropriate SDK command prefix for a package.
pub fn pub_cmd(pkg: &Package) -> &'static str {
    if pkg.is_flutter { "flutter" } else { "dart" }
}

/// Build the `pub add` subcommand string.
pub fn build_pub_add_command(package: &str, dev: bool) -> String {
    if dev {
        format!("pub add --dev {package}")
    } else {
        format!("pub add {package}")
    }
}

/// Build the `pub remove` subcommand string.
pub fn build_pub_remove_command(package: &str) -> String {
    format!("pub remove {package}")
}

/// Resolve the `pub` subcommand string from options.
fn resolve_subcmd(opts: &PubOpts) -> String {
    match &opts.subcommand {
        PubSubcommand::Get => "pub get".to_string(),
        PubSubcommand::Outdated => "pub outdated".to_string(),
        PubSubcommand::Upgrade => {
            if opts.major_versions {
                "pub upgrade --major-versions".to_string()
            } else {
                "pub upgrade".to_string()
            }
        }
        PubSubcommand::Downgrade => "pub downgrade".to_string(),
        PubSubcommand::Add { package, dev } => build_pub_add_command(package, *dev),
        PubSubcommand::Remove { package } => build_pub_remove_command(package),
    }
}

/// Run a `pub` subcommand across packages, splitting by SDK (flutter vs dart).
///
/// Returns [`PackageResults`] with per-package success/failure status.
pub async fn run(
    packages: &[Package],
    workspace: &Workspace,
    opts: &PubOpts,
    events: Option<&UnboundedSender<Event>>,
) -> Result<PackageResults> {
    let subcmd = resolve_subcmd(opts);

    let flutter_pkgs: Vec<Package> = packages.iter().filter(|p| p.is_flutter).cloned().collect();
    let dart_pkgs: Vec<Package> = packages.iter().filter(|p| !p.is_flutter).cloned().collect();

    let runner = ProcessRunner::new(opts.concurrency, false);
    let env_vars = workspace.env_vars();
    let mut all_results: Vec<(String, bool)> = Vec::new();

    if !flutter_pkgs.is_empty() {
        let cmd = format!("flutter {subcmd}");
        if let Some(tx) = events {
            let _ = tx.send(Event::Progress {
                completed: 0,
                total: 0,
                message: format!("flutter {subcmd}..."),
            });
        }
        let results = runner
            .run_in_packages_with_events(
                &flutter_pkgs,
                &cmd,
                &env_vars,
                None,
                events,
                &workspace.packages,
            )
            .await?;
        all_results.extend(results);
    }

    if !dart_pkgs.is_empty() {
        let cmd = format!("dart {subcmd}");
        if let Some(tx) = events {
            let _ = tx.send(Event::Progress {
                completed: 0,
                total: 0,
                message: format!("dart {subcmd}..."),
            });
        }
        let results = runner
            .run_in_packages_with_events(
                &dart_pkgs,
                &cmd,
                &env_vars,
                None,
                events,
                &workspace.packages,
            )
            .await?;
        all_results.extend(results);
    }

    Ok(PackageResults::from(all_results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pub_cmd_flutter() {
        let pkg = Package {
            name: "app".to_string(),
            version: Some("1.0.0".to_string()),
            path: std::path::PathBuf::from("/pkg/app"),
            is_flutter: true,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: std::collections::HashMap::new(),
            publish_to: None,
            resolution: None,
        };
        assert_eq!(pub_cmd(&pkg), "flutter");
    }

    #[test]
    fn test_pub_cmd_dart() {
        let pkg = Package {
            name: "core".to_string(),
            version: Some("1.0.0".to_string()),
            path: std::path::PathBuf::from("/pkg/core"),
            is_flutter: false,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: std::collections::HashMap::new(),
            publish_to: None,
            resolution: None,
        };
        assert_eq!(pub_cmd(&pkg), "dart");
    }

    #[test]
    fn test_build_pub_add_command_regular() {
        let cmd = build_pub_add_command("http", false);
        assert_eq!(cmd, "pub add http");
    }

    #[test]
    fn test_build_pub_add_command_dev() {
        let cmd = build_pub_add_command("mockito", true);
        assert_eq!(cmd, "pub add --dev mockito");
    }

    #[test]
    fn test_build_pub_add_command_with_version() {
        let cmd = build_pub_add_command("http:^1.0.0", false);
        assert_eq!(cmd, "pub add http:^1.0.0");
    }

    #[test]
    fn test_build_pub_remove_command() {
        let cmd = build_pub_remove_command("http");
        assert_eq!(cmd, "pub remove http");
    }

    #[test]
    fn test_resolve_subcmd_get() {
        let opts = PubOpts {
            subcommand: PubSubcommand::Get,
            concurrency: 5,
            major_versions: false,
        };
        assert_eq!(resolve_subcmd(&opts), "pub get");
    }

    #[test]
    fn test_resolve_subcmd_outdated() {
        let opts = PubOpts {
            subcommand: PubSubcommand::Outdated,
            concurrency: 1,
            major_versions: false,
        };
        assert_eq!(resolve_subcmd(&opts), "pub outdated");
    }

    #[test]
    fn test_resolve_subcmd_upgrade() {
        let opts = PubOpts {
            subcommand: PubSubcommand::Upgrade,
            concurrency: 5,
            major_versions: false,
        };
        assert_eq!(resolve_subcmd(&opts), "pub upgrade");
    }

    #[test]
    fn test_resolve_subcmd_upgrade_major() {
        let opts = PubOpts {
            subcommand: PubSubcommand::Upgrade,
            concurrency: 5,
            major_versions: true,
        };
        assert_eq!(resolve_subcmd(&opts), "pub upgrade --major-versions");
    }

    #[test]
    fn test_resolve_subcmd_downgrade() {
        let opts = PubOpts {
            subcommand: PubSubcommand::Downgrade,
            concurrency: 5,
            major_versions: false,
        };
        assert_eq!(resolve_subcmd(&opts), "pub downgrade");
    }

    #[test]
    fn test_resolve_subcmd_add() {
        let opts = PubOpts {
            subcommand: PubSubcommand::Add {
                package: "http".to_string(),
                dev: false,
            },
            concurrency: 5,
            major_versions: false,
        };
        assert_eq!(resolve_subcmd(&opts), "pub add http");
    }

    #[test]
    fn test_resolve_subcmd_add_dev() {
        let opts = PubOpts {
            subcommand: PubSubcommand::Add {
                package: "mockito".to_string(),
                dev: true,
            },
            concurrency: 5,
            major_versions: false,
        };
        assert_eq!(resolve_subcmd(&opts), "pub add --dev mockito");
    }

    #[test]
    fn test_resolve_subcmd_remove() {
        let opts = PubOpts {
            subcommand: PubSubcommand::Remove {
                package: "http".to_string(),
            },
            concurrency: 5,
            major_versions: false,
        };
        assert_eq!(resolve_subcmd(&opts), "pub remove http");
    }
}
