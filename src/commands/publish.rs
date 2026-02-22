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

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Publish packages to pub.dev
pub async fn run(workspace: &Workspace, args: PublishArgs) -> Result<()> {
    // Start with global filters, then also exclude private packages by default
    let mut filters: PackageFilters = (&args.filters).into();
    // Publishing only makes sense for non-private packages
    filters.no_private = true;

    let packages = apply_filters_with_categories(&workspace.packages, &filters, Some(&workspace.root_path), &workspace.config.categories)?;

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
        println!(
            "{}",
            "Use --dry-run=false to publish for real.\n".dimmed()
        );
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

    // Run pre-publish hook if configured
    if let Some(publish_config) = workspace
        .config
        .command
        .as_ref()
        .and_then(|c| c.publish.as_ref())
        && let Some(hooks) = &publish_config.hooks
        && let Some(ref pre_hook) = hooks.pre
    {
        println!(
            "\n{} Running pre-publish hook: {}",
            "$".cyan(),
            pre_hook
        );
        let status = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(pre_hook)
            .current_dir(&workspace.root_path)
            .env("MELOS_PUBLISH_DRY_RUN", dry_run_str)
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("Pre-publish hook failed with exit code: {}", status.code().unwrap_or(-1));
        }
    }

    // Build the publish command
    let mut cmd = String::from("dart pub publish");
    if args.dry_run {
        cmd.push_str(" --dry-run");
    } else {
        // --force skips the pub.dev confirmation prompt
        cmd.push_str(" --force");
    }

    let pb = create_progress_bar(packages.len() as u64, "publishing");
    let runner = ProcessRunner::new(args.concurrency, false);
    let results = runner
        .run_in_packages_with_progress(&packages, &cmd, &workspace.env_vars(), None, Some(&pb))
        .await?;
    pb.finish_and_clear();

    let failed = results.iter().filter(|(_, success)| !success).count();
    let succeeded: Vec<_> = results
        .iter()
        .filter(|(_, success)| *success)
        .map(|(name, _)| name.clone())
        .collect();

    // Git tag creation for successfully published packages
    if args.git_tag_version && !args.dry_run && !succeeded.is_empty() {
        println!("\n{} Creating git tags...\n", "$".cyan());
        for pkg_name in &succeeded {
            if let Some(pkg) = packages.iter().find(|p| &p.name == pkg_name) {
                let version = pkg.version.as_deref().unwrap_or("0.0.0");
                let tag = format!("{}-v{}", pkg_name, version);
                let tag_result = std::process::Command::new("git")
                    .args(["tag", "-a", &tag, "-m", &format!("Release {} v{}", pkg_name, version)])
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

    // Run post-publish hook if configured (before bail on failure, matching Melos behavior)
    if let Some(publish_config) = workspace
        .config
        .command
        .as_ref()
        .and_then(|c| c.publish.as_ref())
        && let Some(hooks) = &publish_config.hooks
        && let Some(ref post_hook) = hooks.post
    {
        println!(
            "\n{} Running post-publish hook: {}",
            "$".cyan(),
            post_hook
        );
        let status = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(post_hook)
            .current_dir(&workspace.root_path)
            .env("MELOS_PUBLISH_DRY_RUN", dry_run_str)
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("Post-publish hook failed with exit code: {}", status.code().unwrap_or(-1));
        }
    }

    if failed > 0 {
        anyhow::bail!("{} package(s) failed to publish", failed);
    }

    let action = if args.dry_run {
        "validated"
    } else {
        "published"
    };
    println!("\n{}", format!("All packages {}.", action).green());
    Ok(())
}
