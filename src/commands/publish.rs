use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters_with_categories;
use crate::runner::{ProcessRunner, create_progress_bar};
use crate::workspace::Workspace;

/// Arguments for the `publish` command
#[derive(Args, Debug)]
pub struct PublishArgs {
    /// Perform a dry run (default: true). Use --no-dry-run to actually publish.
    #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
    pub dry_run: bool,

    /// Create a git tag for each published package version
    #[arg(short = 't', long)]
    pub git_tag_version: bool,

    /// Maximum number of concurrent publish operations
    #[arg(short = 'c', long, default_value = "1")]
    pub concurrency: usize,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Print release URL links after publishing (requires `repository` in config).
    /// Generates prefilled release creation page links for each published package.
    #[arg(long, short = 'r')]
    pub release_url: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Publish packages to pub.dev
pub async fn run(workspace: &Workspace, args: PublishArgs) -> Result<()> {
    // Start with global filters, then also exclude private packages by default
    let mut filters: PackageFilters = (&args.filters).into();
    // Publishing only makes sense for non-private packages
    filters.no_private = true;

    let packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    if packages.is_empty() {
        println!(
            "{}",
            "No publishable packages found (private packages are excluded).".yellow()
        );
        return Ok(());
    }

    let dry_run_label = if args.dry_run {
        " (dry run)".yellow()
    } else {
        "".normal()
    };

    println!(
        "\n{} Publishing {} packages{}...\n",
        "$".cyan(),
        packages.len(),
        dry_run_label
    );

    for pkg in &packages {
        let version = pkg.version.as_deref().unwrap_or("unknown");
        println!("  {} {} {}", "->".cyan(), pkg.name.bold(), version.dimmed());
    }
    println!();

    if args.dry_run {
        println!(
            "{}",
            "Dry run mode: no packages will actually be published.".dimmed()
        );
        println!("{}", "Use --dry-run=false to publish for real.\n".dimmed());
    }

    if !args.yes && !args.dry_run {
        print!(
            "\n{} Publish these packages to pub.dev? [y/N] ",
            "CONFIRM:".yellow()
        );
        std::io::Write::flush(&mut std::io::stdout())?;

        let mut input = String::new();
        std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut input)?;
        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    let dry_run_str = if args.dry_run { "true" } else { "false" };

    if let Some(pre_hook) = workspace.hook("publish", "pre") {
        crate::runner::run_lifecycle_hook(
            pre_hook,
            "pre-publish",
            &workspace.root_path,
            &[("MELOS_PUBLISH_DRY_RUN", dry_run_str)],
        )
        .await?;
    }

    let cmd = build_publish_command(args.dry_run);

    let pb = create_progress_bar(packages.len() as u64, "publishing");
    let runner = ProcessRunner::new(args.concurrency, false);
    let results = runner
        .run_in_packages_with_progress(
            &packages,
            &cmd,
            &workspace.env_vars(),
            None,
            Some(&pb),
            &workspace.packages,
        )
        .await?;
    pb.finish_and_clear();

    let failed = results.iter().filter(|(_, success)| !success).count();
    let succeeded: Vec<_> = results
        .iter()
        .filter(|(_, success)| *success)
        .map(|(name, _)| name.clone())
        .collect();

    if args.git_tag_version && !args.dry_run && !succeeded.is_empty() {
        println!("\n{} Creating git tags...\n", "$".cyan());
        for pkg_name in &succeeded {
            if let Some(pkg) = packages.iter().find(|p| &p.name == pkg_name) {
                let version = pkg.version.as_deref().unwrap_or("0.0.0");
                let tag = build_git_tag(pkg_name, version);
                let tag_result = std::process::Command::new("git")
                    .args([
                        "tag",
                        "-a",
                        &tag,
                        "-m",
                        &format!("Release {} v{}", pkg_name, version),
                    ])
                    .current_dir(&workspace.root_path)
                    .status();

                match tag_result {
                    Ok(status) if status.success() => {
                        println!("  {} {}", "TAG".green(), tag);
                    }
                    Ok(_) => {
                        eprintln!("  {} Failed to create tag {}", "WARN".yellow(), tag);
                    }
                    Err(e) => {
                        eprintln!("  {} Git tag error for {}: {}", "WARN".yellow(), tag, e);
                    }
                }
            }
        }
    }

    // Print release URLs if requested (only for real publishes, not dry runs)
    if args.release_url && !args.dry_run && !succeeded.is_empty() {
        if let Some(ref repo) = workspace.config.repository {
            println!("\n{} Release URLs:", "$".cyan());
            for pkg_name in &succeeded {
                if let Some(pkg) = packages.iter().find(|p| &p.name == pkg_name) {
                    let version = pkg.version.as_deref().unwrap_or("0.0.0");
                    let tag = format!("{}-v{}", pkg_name, version);
                    let title = format!("{} v{}", pkg_name, version);
                    let url = repo.release_url(&tag, &title);
                    println!("  {} {}", pkg_name.bold(), url);
                }
            }
        } else {
            println!(
                "\n{} --release-url requires `repository` in config; skipping.",
                "WARN:".yellow()
            );
        }
    }

    // Run post-publish hook before bail on failure, matching Melos behavior
    if let Some(post_hook) = workspace.hook("publish", "post") {
        crate::runner::run_lifecycle_hook(
            post_hook,
            "post-publish",
            &workspace.root_path,
            &[("MELOS_PUBLISH_DRY_RUN", dry_run_str)],
        )
        .await?;
    }

    if failed > 0 {
        let passed = results.len() - failed;
        anyhow::bail!(
            "{} package(s) failed to publish ({} passed)",
            failed,
            passed
        );
    }

    let action = if args.dry_run {
        "validated"
    } else {
        "published"
    };
    println!(
        "\n{}",
        format!("All {} package(s) {}.", results.len(), action).green()
    );
    Ok(())
}

/// Build the `dart pub publish` command string.
fn build_publish_command(dry_run: bool) -> String {
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
fn build_git_tag(package_name: &str, version: &str) -> String {
    format!("{}-v{}", package_name, version)
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

    #[test]
    fn test_release_url_format_matches_tag() {
        use crate::config::RepositoryConfig;

        let repo = RepositoryConfig {
            url: "https://github.com/org/repo".to_string(),
        };
        let pkg_name = "my_package";
        let version = "1.2.3";
        let tag = format!("{}-v{}", pkg_name, version);
        let title = format!("{} v{}", pkg_name, version);
        let url = repo.release_url(&tag, &title);
        assert!(url.contains("tag=my_package-v1.2.3"));
        assert!(url.contains("title=my_package%20v1.2.3"));
    }

    #[test]
    fn test_release_url_prerelease_tag() {
        use crate::config::RepositoryConfig;

        let repo = RepositoryConfig {
            url: "https://github.com/org/repo".to_string(),
        };
        let tag = format!("{}-v{}", "core", "2.0.0-beta.1");
        let title = format!("{} v{}", "core", "2.0.0-beta.1");
        let url = repo.release_url(&tag, &title);
        assert!(url.contains("tag=core-v2.0.0-beta.1"));
        assert!(url.starts_with("https://github.com/org/repo/releases/new?"));
    }
}
