use anyhow::Result;
use clap::{Args, Subcommand};
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::Package;
use crate::package::filter::apply_filters_with_categories;
use crate::runner::{ProcessRunner, create_progress_bar};
use crate::workspace::Workspace;

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

/// Dispatch to the appropriate pub sub-subcommand
pub async fn run(workspace: &Workspace, args: PubArgs) -> Result<()> {
    match args.command {
        PubCommand::Get(a) => run_pub_get(workspace, a).await,
        PubCommand::Outdated(a) => run_pub_outdated(workspace, a).await,
        PubCommand::Upgrade(a) => run_pub_upgrade(workspace, a).await,
        PubCommand::Downgrade(a) => run_pub_downgrade(workspace, a).await,
    }
}

/// Build the appropriate `pub` command prefix for a package (flutter vs dart)
fn pub_cmd(pkg: &Package) -> &'static str {
    if pkg.is_flutter { "flutter" } else { "dart" }
}

/// Run `dart pub get` / `flutter pub get` in each matching package
async fn run_pub_get(workspace: &Workspace, args: PubGetArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    println!(
        "\n{} Running pub get in {} package(s)...\n",
        "$".cyan(),
        packages.len()
    );

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    for pkg in &packages {
        println!("  {} {} ({})", "->".cyan(), pkg.name, pub_cmd(pkg));
    }
    println!();

    // Run each package with its appropriate command (flutter vs dart)
    run_pub_in_packages(&packages, "pub get", args.concurrency, &workspace.env_vars(), &workspace.packages).await
}

/// Run `dart pub outdated` in each matching package
async fn run_pub_outdated(workspace: &Workspace, args: PubOutdatedArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    println!(
        "\n{} Running pub outdated in {} package(s)...\n",
        "$".cyan(),
        packages.len()
    );

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // pub outdated is informational — don't fail on non-zero exit (outdated deps are expected)
    run_pub_in_packages(&packages, "pub outdated", args.concurrency, &workspace.env_vars(), &workspace.packages).await
}

/// Run `dart pub upgrade` in each matching package
async fn run_pub_upgrade(workspace: &Workspace, args: PubUpgradeArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    let subcmd = if args.major_versions {
        "pub upgrade --major-versions"
    } else {
        "pub upgrade"
    };

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
        println!("  {} {} ({})", "->".cyan(), pkg.name, pub_cmd(pkg));
    }
    println!();

    run_pub_in_packages(&packages, subcmd, args.concurrency, &workspace.env_vars(), &workspace.packages).await
}

/// Run `dart pub downgrade` in each matching package
async fn run_pub_downgrade(workspace: &Workspace, args: PubDowngradeArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    println!(
        "\n{} Running pub downgrade in {} package(s)...\n",
        "$".cyan(),
        packages.len()
    );

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    for pkg in &packages {
        println!("  {} {} ({})", "->".cyan(), pkg.name, pub_cmd(pkg));
    }
    println!();

    run_pub_in_packages(&packages, "pub downgrade", args.concurrency, &workspace.env_vars(), &workspace.packages).await
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
        let results = runner.run_in_packages_with_progress(&pkgs, &cmd, env_vars, None, Some(&pb), all_packages).await?;
        all_results.extend(results);
    }

    if !dart_pkgs.is_empty() {
        let cmd = format!("dart {}", pub_subcmd);
        let pkgs: Vec<Package> = dart_pkgs.into_iter().cloned().collect();
        pb.set_message(format!("dart {}...", pub_subcmd));
        let results = runner.run_in_packages_with_progress(&pkgs, &cmd, env_vars, None, Some(&pb), all_packages).await?;
        all_results.extend(results);
    }

    pb.finish_and_clear();

    let failed = all_results.iter().filter(|(_, success)| !success).count();
    if failed > 0 {
        anyhow::bail!("{} package(s) failed", failed);
    }

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
        };
        assert_eq!(pub_cmd(&pkg), "dart");
    }
}
