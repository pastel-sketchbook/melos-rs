use anyhow::Result;
use clap::{Args, Subcommand};
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use crate::runner::{ProcessRunner, create_progress_bar};
use melos_core::package::Package;
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::workspace::Workspace;

/// Arguments for the `pub` command
#[derive(Args, Debug)]
pub struct PubArgs {
    #[command(subcommand)]
    pub command: PubCommand,
}

/// Pub sub-subcommands
#[derive(Subcommand, Debug)]
pub enum PubCommand {
    /// Run `dart pub get` / `flutter pub get` in each package
    Get(PubGetArgs),

    /// Run `dart pub outdated` in each package
    Outdated(PubOutdatedArgs),

    /// Run `dart pub upgrade` in each package
    Upgrade(PubUpgradeArgs),

    /// Run `dart pub downgrade` in each package
    Downgrade(PubDowngradeArgs),

    /// Run `dart pub add` / `flutter pub add` in each package
    Add(PubAddArgs),

    /// Run `dart pub remove` / `flutter pub remove` in each package
    Remove(PubRemoveArgs),
}

/// Arguments for `pub get`
#[derive(Args, Debug)]
pub struct PubGetArgs {
    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "5")]
    pub concurrency: usize,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Arguments for `pub outdated`
#[derive(Args, Debug)]
pub struct PubOutdatedArgs {
    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "1")]
    pub concurrency: usize,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Arguments for `pub upgrade`
#[derive(Args, Debug)]
pub struct PubUpgradeArgs {
    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "5")]
    pub concurrency: usize,

    /// Upgrade to latest major versions (passes --major-versions)
    #[arg(long)]
    pub major_versions: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Arguments for `pub downgrade`
#[derive(Args, Debug)]
pub struct PubDowngradeArgs {
    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "5")]
    pub concurrency: usize,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Arguments for `pub add`
#[derive(Args, Debug)]
pub struct PubAddArgs {
    /// Package to add (e.g., "http" or "http:^1.0.0")
    pub package: String,

    /// Add as a dev dependency
    #[arg(long)]
    pub dev: bool,

    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "5")]
    pub concurrency: usize,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Arguments for `pub remove`
#[derive(Args, Debug)]
pub struct PubRemoveArgs {
    /// Package to remove
    pub package: String,

    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "5")]
    pub concurrency: usize,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Dispatch to the appropriate pub sub-subcommand
pub async fn run(workspace: &Workspace, args: PubArgs) -> Result<()> {
    match args.command {
        PubCommand::Get(a) => run_pub_get(workspace, a).await,
        PubCommand::Outdated(a) => run_pub_outdated(workspace, a).await,
        PubCommand::Upgrade(a) => run_pub_upgrade(workspace, a).await,
        PubCommand::Downgrade(a) => run_pub_downgrade(workspace, a).await,
        PubCommand::Add(a) => run_pub_add(workspace, a).await,
        PubCommand::Remove(a) => run_pub_remove(workspace, a).await,
    }
}

/// Build the appropriate `pub` command prefix for a package (flutter vs dart)
fn pub_cmd(pkg: &Package) -> &'static str {
    if pkg.is_flutter { "flutter" } else { "dart" }
}

/// Common logic for all pub subcommands: filter packages, print header, and run.
///
/// `show_sdk` controls whether the SDK (flutter/dart) is shown next to each
/// package name in the listing.
async fn run_pub_subcommand(
    workspace: &Workspace,
    filters: &GlobalFilterArgs,
    subcmd: &str,
    concurrency: usize,
    show_sdk: bool,
) -> Result<()> {
    let pf = package_filters_from_args(filters);
    let packages = apply_filters_with_categories(
        &workspace.packages,
        &pf,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    println!(
        "\n{} Running {} in {} package(s)...\n",
        "$".cyan(),
        subcmd,
        packages.len()
    );

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    for pkg in &packages {
        if show_sdk {
            println!("  {} {} ({})", "->".cyan(), pkg.name, pub_cmd(pkg));
        } else {
            println!("  {} {}", "->".cyan(), pkg.name);
        }
    }
    println!();

    run_pub_in_packages(
        &packages,
        subcmd,
        concurrency,
        &workspace.env_vars(),
        &workspace.packages,
    )
    .await
}

/// Run `dart pub get` / `flutter pub get` in each matching package
async fn run_pub_get(workspace: &Workspace, args: PubGetArgs) -> Result<()> {
    run_pub_subcommand(workspace, &args.filters, "pub get", args.concurrency, true).await
}

/// Run `dart pub outdated` in each matching package.
///
/// pub outdated is informational — non-zero exit from outdated deps is expected,
/// but the runner still reports failures per-package.
async fn run_pub_outdated(workspace: &Workspace, args: PubOutdatedArgs) -> Result<()> {
    run_pub_subcommand(
        workspace,
        &args.filters,
        "pub outdated",
        args.concurrency,
        false,
    )
    .await
}

/// Run `dart pub upgrade` in each matching package
async fn run_pub_upgrade(workspace: &Workspace, args: PubUpgradeArgs) -> Result<()> {
    let subcmd = if args.major_versions {
        "pub upgrade --major-versions"
    } else {
        "pub upgrade"
    };
    run_pub_subcommand(workspace, &args.filters, subcmd, args.concurrency, true).await
}

/// Run `dart pub downgrade` in each matching package
async fn run_pub_downgrade(workspace: &Workspace, args: PubDowngradeArgs) -> Result<()> {
    run_pub_subcommand(
        workspace,
        &args.filters,
        "pub downgrade",
        args.concurrency,
        true,
    )
    .await
}

/// Run `dart pub add` / `flutter pub add` in each matching package
async fn run_pub_add(workspace: &Workspace, args: PubAddArgs) -> Result<()> {
    let subcmd = build_pub_add_command(&args.package, args.dev);
    run_pub_subcommand(workspace, &args.filters, &subcmd, args.concurrency, true).await
}

/// Run `dart pub remove` / `flutter pub remove` in each matching package
async fn run_pub_remove(workspace: &Workspace, args: PubRemoveArgs) -> Result<()> {
    let subcmd = build_pub_remove_command(&args.package);
    run_pub_subcommand(workspace, &args.filters, &subcmd, args.concurrency, true).await
}

/// Build the `pub add` subcommand string.
fn build_pub_add_command(package: &str, dev: bool) -> String {
    if dev {
        format!("pub add --dev {}", package)
    } else {
        format!("pub add {}", package)
    }
}

/// Build the `pub remove` subcommand string.
fn build_pub_remove_command(package: &str) -> String {
    format!("pub remove {}", package)
}

/// Run a `pub` subcommand in each package, using the appropriate SDK (flutter vs dart).
///
/// Because each package may use a different command prefix (flutter vs dart), we run
/// them individually through the ProcessRunner with per-package command construction.
async fn run_pub_in_packages(
    packages: &[Package],
    pub_subcmd: &str,
    concurrency: usize,
    env_vars: &std::collections::HashMap<String, String>,
    all_packages: &[Package],
) -> Result<()> {
    // Group packages by SDK to batch them efficiently
    let flutter_pkgs: Vec<&Package> = packages.iter().filter(|p| p.is_flutter).collect();
    let dart_pkgs: Vec<&Package> = packages.iter().filter(|p| !p.is_flutter).collect();

    let pb = create_progress_bar(packages.len() as u64, pub_subcmd);
    let runner = ProcessRunner::new(concurrency, false);
    let mut all_results = Vec::new();

    if !flutter_pkgs.is_empty() {
        let cmd = format!("flutter {}", pub_subcmd);
        let pkgs: Vec<Package> = flutter_pkgs.into_iter().cloned().collect();
        pb.set_message(format!("flutter {}...", pub_subcmd));
        let results = runner
            .run_in_packages_with_progress(&pkgs, &cmd, env_vars, None, Some(&pb), all_packages)
            .await?;
        all_results.extend(results);
    }

    if !dart_pkgs.is_empty() {
        let cmd = format!("dart {}", pub_subcmd);
        let pkgs: Vec<Package> = dart_pkgs.into_iter().cloned().collect();
        pb.set_message(format!("dart {}...", pub_subcmd));
        let results = runner
            .run_in_packages_with_progress(&pkgs, &cmd, env_vars, None, Some(&pb), all_packages)
            .await?;
        all_results.extend(results);
    }

    pb.finish_and_clear();

    let failed = all_results.iter().filter(|(_, success)| !success).count();
    let passed = all_results.len() - failed;

    if failed > 0 {
        anyhow::bail!("{} package(s) failed ({} passed)", failed, passed);
    }

    println!(
        "\n{}",
        format!("All {} package(s) succeeded.", passed).green()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_pub_cmd_flutter() {
        let pkg = Package {
            name: "app".to_string(),
            version: Some("1.0.0".to_string()),
            path: std::path::PathBuf::from("/pkg/app"),
            is_flutter: true,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
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
            dependency_versions: HashMap::new(),
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
}
