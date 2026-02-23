use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;
use semver::Version;

use crate::filter_ext::package_filters_from_args;
use melos_core::commands::version::{
    BumpType, ChangelogOptions, ConventionalCommit, apply_version_bump, compute_next_prerelease,
    compute_next_version, create_git_tag, create_release_branch, find_latest_git_tag,
    generate_changelog_entry, git_checkout, git_commit, git_current_branch, git_fetch_tags,
    git_push, graduate_version, highest_bump, is_prerelease, map_commits_to_packages,
    package_matches_filters, parse_commits_since, push_release_branch,
    update_dependency_constraint, update_git_tag_refs, validate_branch, write_changelog,
};
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::workspace::Workspace;

/// Arguments for the `version` command
#[derive(Args, Debug)]
pub struct VersionArgs {
    /// Version bump type (build, patch, minor, major) or an explicit version
    #[arg(default_value = "patch")]
    pub bump: String,

    /// Apply to all packages
    #[arg(long, short = 'a')]
    pub all: bool,

    /// Per-package version overrides (e.g., -Vanmobile:patch -Vadapter:build)
    #[arg(short = 'V', value_parser = melos_core::commands::version::parse_version_override)]
    pub overrides: Vec<(String, String)>,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Use conventional commits to determine version bumps
    #[arg(long)]
    pub conventional_commits: bool,

    /// Git ref to find conventional commits since (used with --conventional-commits).
    /// If not provided, defaults to the latest git tag or HEAD~10 if no tags exist.
    #[arg(long)]
    pub since_ref: Option<String>,

    /// Skip changelog generation
    #[arg(long)]
    pub no_changelog: bool,

    /// Generate changelogs (default: true). Alias for the positive side of --[no-]changelog.
    #[arg(short = 'c', long, conflicts_with = "no_changelog")]
    pub changelog: bool,

    /// Skip git tag creation
    #[arg(long, alias = "no-git-tag")]
    pub no_git_tag_version: bool,

    /// Create git tags (default: true). Short flag for --[no-]git-tag-version.
    #[arg(short = 't', long, conflicts_with = "no_git_tag_version")]
    pub git_tag_version: bool,

    /// Skip pushing commits and tags to remote
    #[arg(long)]
    pub no_git_push: bool,

    /// Coordinated versioning: bump all packages to the same version
    #[arg(long)]
    pub coordinated: bool,

    /// Version as prerelease (e.g., 1.0.0-dev.0). Cannot combine with --graduate.
    #[arg(long, short = 'p', conflicts_with = "graduate")]
    pub prerelease: bool,

    /// Graduate prerelease packages to stable (e.g., 1.0.0-dev.3 -> 1.0.0).
    /// Cannot combine with --prerelease.
    #[arg(long, short = 'g', conflicts_with = "prerelease")]
    pub graduate: bool,

    /// Prerelease identifier (e.g., beta -> 1.0.0-beta.0). Used with --prerelease.
    #[arg(long, default_value = "dev")]
    pub preid: String,

    /// Prerelease identifier for dependents only (falls back to --preid if not set)
    #[arg(long)]
    pub dependent_preid: Option<String>,

    /// Override the release commit message. Use {new_package_versions} as placeholder.
    #[arg(long, short = 'm')]
    pub message: Option<String>,

    /// Update dependency constraints in dependent packages (default: true)
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub dependent_constraints: bool,

    /// Patch-bump dependents when their constraints change (default: true).
    /// Only effective with --dependent-constraints and --conventional-commits.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub dependent_versions: bool,

    /// Print release URL links after versioning (requires `repository` in config).
    /// Generates prefilled release creation page links for each package.
    #[arg(long, short = 'r')]
    pub release_url: bool,

    /// Create a release branch after versioning. Value is a branch name pattern
    /// where `{version}` is replaced with the release version.
    /// Example: `release/{version}` creates `release/1.2.3`.
    /// Overrides the `releaseBranch` config setting.
    #[arg(long)]
    pub release_branch: Option<String>,

    /// Disable release branch creation even if configured.
    #[arg(long, conflicts_with = "release_branch")]
    pub no_release_branch: bool,

    /// Show what version bumps would be applied without making changes
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub filters: crate::cli::GlobalFilterArgs,
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Execute the version command
pub async fn run(workspace: &Workspace, args: VersionArgs) -> Result<()> {
    println!("\n{} Managing versions across packages...\n", "$".cyan());

    if workspace.packages.is_empty() {
        println!("{}", "No packages found in workspace.".yellow());
        return Ok(());
    }

    // Apply global filters to narrow down which packages are eligible for versioning.
    // This allows `melos-rs version --scope core_* --all patch` to bump only matching packages.
    let filters = package_filters_from_args(&args.filters);
    let eligible_packages = if filters.is_empty() {
        workspace.packages.clone()
    } else {
        apply_filters_with_categories(
            &workspace.packages,
            &filters,
            Some(&workspace.root_path),
            &workspace.config.categories,
        )?
    };

    let version_config = workspace
        .config
        .command
        .as_ref()
        .and_then(|c| c.version.as_ref());

    // Branch validation
    if let Some(cfg) = version_config
        && let Some(ref branch) = cfg.branch
    {
        validate_branch(&workspace.root_path, branch)?;
        println!("  {} Branch validation passed ({})", "OK".green(), branch);
    }

    // Fetch tags from remote if configured
    if version_config
        .map(|c| c.should_fetch_tags())
        .unwrap_or(false)
    {
        println!("  {} Fetching tags from remote...", "$".cyan());
        git_fetch_tags(&workspace.root_path)?;
        println!("  {} Tags fetched", "OK".green());
    }

    // Determine changelog/tag settings from config + CLI flags
    let should_changelog = if args.no_changelog {
        false
    } else {
        version_config.is_none_or(|c| c.should_changelog())
    };
    let should_tag = if args.no_git_tag_version {
        false
    } else {
        version_config.is_none_or(|c| c.should_tag())
    };

    // Resolve commit body inclusion: changelogCommitBodies takes precedence over
    // changelogConfig.includeCommitBody for backward compatibility.
    let (include_body, only_breaking_bodies) =
        if let Some(bodies_cfg) = version_config.and_then(|c| c.changelog_commit_bodies.as_ref()) {
            (bodies_cfg.include, bodies_cfg.only_breaking)
        } else {
            let body = version_config
                .and_then(|c| c.changelog_config.as_ref())
                .and_then(|cc| cc.include_commit_body)
                .unwrap_or(false);
            // Legacy includeCommitBody includes ALL bodies (not just breaking)
            (body, false)
        };

    let include_hash = version_config
        .and_then(|c| c.changelog_config.as_ref())
        .and_then(|cc| cc.include_commit_id)
        // link_to_commits is an alias/override for including commit IDs
        .or_else(|| version_config.and_then(|c| c.link_to_commits))
        .unwrap_or(false);
    let include_scopes = version_config
        .and_then(|c| c.include_scopes)
        .unwrap_or(true); // Melos includes scopes by default

    // Resolve changelogFormat.includeDate (default: false per Melos docs)
    let include_date = version_config
        .map(|c| c.should_include_date())
        .unwrap_or(false);

    // Changelog commit type filtering
    let changelog_include_types: Option<Vec<String>> = version_config
        .and_then(|c| c.changelog_config.as_ref())
        .and_then(|cc| cc.include_types.clone());
    let changelog_exclude_types: Option<Vec<String>> = version_config
        .and_then(|c| c.changelog_config.as_ref())
        .and_then(|cc| cc.exclude_types.clone());

    // Collect conventional commits if requested
    let conventional_commits = if args.conventional_commits {
        // Resolve since_ref: CLI flag -> latest git tag -> fallback "HEAD~10"
        let since_ref = args.since_ref.clone().unwrap_or_else(|| {
            find_latest_git_tag(&workspace.root_path).unwrap_or_else(|| "HEAD~10".to_string())
        });
        let commits = parse_commits_since(&workspace.root_path, &since_ref)?;
        println!(
            "  Found {} conventional commit(s) since {}",
            commits.len().to_string().bold(),
            since_ref
        );
        let mapped = map_commits_to_packages(&workspace.root_path, &commits, &workspace.packages)?;
        Some(mapped)
    } else {
        None
    };

    // Determine whether coordinated versioning is enabled (CLI flag or config)
    let is_coordinated =
        args.coordinated || version_config.map(|c| c.is_coordinated()).unwrap_or(false);

    // Determine which packages to version and how.
    //
    // The result is a Vec of (package, target_version_string) where the target
    // is either a bump type ("patch", "minor") or an explicit version ("1.2.0-dev.0").
    let packages_to_version: Vec<(&melos_core::package::Package, String)> = if args.graduate {
        // Graduate mode: strip prerelease suffix from all prerelease packages
        let graduated: Vec<_> = eligible_packages
            .iter()
            .filter(|p| {
                let v = p.version.as_deref().unwrap_or("0.0.0");
                is_prerelease(v)
            })
            .map(|p| {
                let current = p.version.as_deref().unwrap_or("0.0.0");
                let stable = graduate_version(current)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| current.to_string());
                (p, stable)
            })
            .collect();

        if graduated.is_empty() {
            println!("{}", "No prerelease packages to graduate.".yellow());
            return Ok(());
        }

        println!(
            "  {} Graduating {} prerelease package(s) to stable",
            "INFO".blue(),
            graduated.len()
        );
        graduated
    } else if is_coordinated {
        // Coordinated versioning: bump ALL packages to the same version.
        let highest_current = eligible_packages
            .iter()
            .filter_map(|p| {
                let v_str = p.version.as_deref().unwrap_or("0.0.0");
                Version::parse(v_str)
                    .or_else(|_| {
                        let cleaned = v_str.split('+').next().unwrap_or(v_str);
                        Version::parse(cleaned)
                    })
                    .ok()
            })
            .max()
            .unwrap_or_else(|| Version::new(0, 0, 0));

        let base_str = format!(
            "{}.{}.{}",
            highest_current.major, highest_current.minor, highest_current.patch
        );
        let coordinated_version = if args.prerelease {
            compute_next_prerelease(&base_str, &args.bump, &args.preid)?
        } else {
            compute_next_version(&base_str, &args.bump)?
        };
        let explicit = coordinated_version.to_string();

        println!(
            "  {} Coordinated versioning: all packages -> {}",
            "INFO".blue(),
            explicit.green()
        );

        eligible_packages
            .iter()
            .map(|p| (p, explicit.clone()))
            .collect()
    } else if !args.overrides.is_empty() {
        // Per-package overrides (prerelease modifier applied if --prerelease)
        args.overrides
            .iter()
            .filter_map(|(name, bump)| {
                eligible_packages
                    .iter()
                    .find(|p| p.name.contains(name))
                    .map(|p| {
                        if args.prerelease {
                            let current = p.version.as_deref().unwrap_or("0.0.0");
                            let v = compute_next_prerelease(current, bump, &args.preid)
                                .map(|v| v.to_string())
                                .unwrap_or_else(|_| bump.clone());
                            (p, v)
                        } else {
                            (p, bump.clone())
                        }
                    })
            })
            .collect()
    } else if args.conventional_commits {
        // Use conventional commits to determine bumps
        let mapped = conventional_commits
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("conventional commits not loaded; this is a bug"))?;
        eligible_packages
            .iter()
            .filter_map(|p| {
                let commits = mapped.get(&p.name)?;
                let bump = highest_bump(commits);
                if bump == BumpType::None {
                    None
                } else if args.prerelease {
                    let current = p.version.as_deref().unwrap_or("0.0.0");
                    let v = compute_next_prerelease(current, &bump.to_string(), &args.preid)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|_| bump.to_string());
                    Some((p, v))
                } else {
                    Some((p, bump.to_string()))
                }
            })
            .collect()
    } else if args.all {
        // Apply to all packages with the default bump type
        if args.prerelease {
            eligible_packages
                .iter()
                .map(|p| {
                    let current = p.version.as_deref().unwrap_or("0.0.0");
                    let v = compute_next_prerelease(current, &args.bump, &args.preid)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|_| args.bump.clone());
                    (p, v)
                })
                .collect()
        } else {
            eligible_packages
                .iter()
                .map(|p| (p, args.bump.clone()))
                .collect()
        }
    } else {
        println!(
            "{}",
            "Specify --all, --conventional-commits, --graduate, or use -V overrides to select packages."
                .yellow()
        );
        return Ok(());
    };

    if packages_to_version.is_empty() {
        println!("{}", "No packages need version bumps.".yellow());
        return Ok(());
    }

    // Show plan
    println!("\nVersion changes:");
    for (pkg, bump) in &packages_to_version {
        let current = pkg.version.as_deref().unwrap_or("0.0.0");
        let next = compute_next_version(current, bump)?;
        println!(
            "  {} {} -> {} ({})",
            pkg.name.bold(),
            current.dimmed(),
            next.to_string().green(),
            bump
        );
    }

    // Dry-run mode: show the plan without applying changes
    if args.dry_run {
        println!(
            "\n{}",
            "DRY RUN \u{2014} no version changes were applied."
                .yellow()
                .bold()
        );
        return Ok(());
    }

    if !args.yes {
        print!(
            "\n{} Apply these version changes? [y/N] ",
            "CONFIRM:".yellow()
        );
        std::io::Write::flush(&mut std::io::stdout()).context("Failed to flush stdout")?;

        let mut input = String::new();
        std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut input)
            .context("Failed to read user input")?;
        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // Apply version changes and collect new versions for tagging
    let mut versioned: Vec<(String, String)> = Vec::new(); // (pkg_name, new_version)
    for (pkg, bump) in &packages_to_version {
        let new_version = apply_version_bump(pkg, bump)?;
        println!(
            "  {} Updated {} to {}",
            "OK".green(),
            pkg.path.join("pubspec.yaml").display(),
            new_version
        );
        versioned.push((pkg.name.clone(), new_version));
    }

    // Update dependent package constraints (--dependent-constraints, default: on)
    if args.dependent_constraints && !versioned.is_empty() {
        let versioned_names: std::collections::HashMap<&str, &str> = versioned
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();

        // Find packages that depend on any bumped package but were not themselves bumped
        let mut dependents_to_bump: Vec<(&melos_core::package::Package, String)> = Vec::new();

        for pkg in &workspace.packages {
            if versioned_names.contains_key(pkg.name.as_str()) {
                continue; // Already bumped
            }

            let mut was_updated = false;
            for dep_name in pkg.dependencies.iter().chain(pkg.dev_dependencies.iter()) {
                if let Some(&new_ver) = versioned_names.get(dep_name.as_str()) {
                    let updated = update_dependency_constraint(pkg, dep_name, new_ver)?;
                    if updated {
                        let constraint = format!(
                            "^{}",
                            semver::Version::parse(new_ver)
                                .or_else(|_| {
                                    let cleaned = new_ver.split('+').next().unwrap_or(new_ver);
                                    semver::Version::parse(cleaned)
                                })
                                .unwrap_or_else(|_| semver::Version::new(0, 0, 0))
                        );
                        println!(
                            "  {} Updated {} dependency on {} to {}",
                            "OK".green(),
                            pkg.name.bold(),
                            dep_name,
                            constraint
                        );
                        was_updated = true;
                    }
                }
            }

            if was_updated && args.dependent_versions {
                // Determine the version for the dependent
                let dependent_ver = if args.prerelease {
                    let preid = args.dependent_preid.as_deref().unwrap_or(&args.preid);
                    let current = pkg.version.as_deref().unwrap_or("0.0.0");
                    compute_next_prerelease(current, "patch", preid)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|_| "patch".to_string())
                } else {
                    "patch".to_string()
                };
                dependents_to_bump.push((pkg, dependent_ver));
            }
        }

        if !dependents_to_bump.is_empty() {
            println!(
                "\n{} Bumping {} dependent package(s)...",
                "$".cyan(),
                dependents_to_bump.len()
            );
            for (pkg, bump) in &dependents_to_bump {
                let new_version = apply_version_bump(pkg, bump)?;
                println!(
                    "  {} Updated {} to {}",
                    "OK".green(),
                    pkg.path.join("pubspec.yaml").display(),
                    new_version
                );
                versioned.push((pkg.name.clone(), new_version));
            }
        }
    }

    // Generate changelogs
    if should_changelog {
        if let Some(ref mapped) = conventional_commits {
            let repo = workspace.config.repository.as_ref();
            let make_changelog_opts = || ChangelogOptions {
                include_body,
                only_breaking_bodies,
                include_hash,
                include_scopes,
                repository: repo,
                include_types: changelog_include_types.as_deref(),
                exclude_types: changelog_exclude_types.as_deref(),
                include_date,
            };
            println!("\n{} Generating changelogs...", "$".cyan());
            for (pkg, _bump) in &packages_to_version {
                if let Some(commits) = mapped.get(&pkg.name)
                    && !commits.is_empty()
                {
                    let new_ver = versioned
                        .iter()
                        .find(|(n, _)| n == &pkg.name)
                        .map(|(_, v)| v.as_str())
                        .unwrap_or("unknown");
                    let entry = generate_changelog_entry(new_ver, commits, &make_changelog_opts());
                    write_changelog(&pkg.path, &entry)?;
                    println!(
                        "  {} Updated CHANGELOG.md for {}",
                        "OK".green(),
                        pkg.name.bold()
                    );
                }
            }

            // Workspace-level changelog
            let should_workspace = version_config
                .map(|c| c.should_workspace_changelog())
                .unwrap_or(true);
            if should_workspace {
                let all_commits: Vec<ConventionalCommit> =
                    mapped.values().flatten().cloned().collect();
                if !all_commits.is_empty() {
                    let summary_version = versioned
                        .first()
                        .map(|(_, v)| v.as_str())
                        .unwrap_or("0.0.0");
                    let entry = generate_changelog_entry(
                        summary_version,
                        &all_commits,
                        &make_changelog_opts(),
                    );
                    write_changelog(&workspace.root_path, &entry)?;
                    println!("  {} Updated workspace CHANGELOG.md", "OK".green());
                }
            }

            // Aggregate changelogs
            if let Some(agg_configs) = version_config.and_then(|c| c.changelogs.as_ref()) {
                for agg in agg_configs {
                    let agg_path = workspace.root_path.join(&agg.path);

                    // Filter commits to only those from packages matching the aggregate filters
                    let agg_commits: Vec<ConventionalCommit> =
                        if let Some(ref filters) = agg.package_filters {
                            mapped
                                .iter()
                                .filter(|(pkg_name, _)| {
                                    package_matches_filters(pkg_name, filters, &workspace.packages)
                                })
                                .flat_map(|(_, commits)| commits.iter().cloned())
                                .collect()
                        } else {
                            // No filters -- include all commits
                            mapped.values().flatten().cloned().collect()
                        };

                    if !agg_commits.is_empty() {
                        let agg_version = versioned
                            .first()
                            .map(|(_, v)| v.as_str())
                            .unwrap_or("0.0.0");
                        let entry = generate_changelog_entry(
                            agg_version,
                            &agg_commits,
                            &make_changelog_opts(),
                        );

                        // If the file has a description configured, ensure it's at the top
                        let full_entry = if let Some(ref desc) = agg.description {
                            if !agg_path.exists() {
                                format!("# Changelog\n\n{}\n\n{}", desc, entry)
                            } else {
                                entry
                            }
                        } else {
                            entry
                        };

                        write_changelog(&agg_path, &full_entry)?;
                        println!(
                            "  {} Updated aggregate changelog {}",
                            "OK".green(),
                            agg.path.bold()
                        );
                    }
                }
            }
        } else {
            println!(
                "\n{} Changelog generation requires --conventional-commits; skipping.",
                "NOTE:".yellow()
            );
        }
    }

    // Update git tag references in dependent packages if configured
    let should_update_refs = version_config
        .map(|c| c.should_update_git_tag_refs())
        .unwrap_or(false);
    if should_update_refs && !versioned.is_empty() {
        println!("\n{} Updating git tag references...", "$".cyan());
        let count = update_git_tag_refs(&workspace.root_path, &workspace.packages, &versioned)?;
        if count > 0 {
            println!(
                "  {} Updated git tag refs in {} file(s)",
                "OK".green(),
                count
            );
        } else {
            println!("  {} No git tag refs to update", "OK".green());
        }
    }

    if let Some(pre_commit) = version_config
        .and_then(|cfg| cfg.hooks.as_ref())
        .and_then(|h| h.pre_commit.as_deref())
    {
        crate::runner::run_lifecycle_hook(pre_commit, "pre-commit", &workspace.root_path, &[])
            .await?;
    }

    let new_package_versions = versioned
        .iter()
        .map(|(name, ver)| format!(" - {} @ {}", name, ver))
        .collect::<Vec<_>>()
        .join("\n");

    let commit_message = if let Some(ref msg) = args.message {
        // CLI --message overrides everything
        msg.replace("{new_package_versions}", &new_package_versions)
    } else {
        let template = version_config
            .map(|c| c.message_template().to_string())
            .unwrap_or_else(|| {
                "chore(release): publish packages\n\n{new_package_versions}".to_string()
            });
        template.replace("{new_package_versions}", &new_package_versions)
    };
    println!(
        "\n{} Committing: {}",
        "$".cyan(),
        commit_message
            .lines()
            .next()
            .unwrap_or(&commit_message)
            .dimmed()
    );
    git_commit(&workspace.root_path, &commit_message)?;
    println!("  {} Committed version changes", "OK".green());

    if let Some(post_commit) = version_config
        .and_then(|cfg| cfg.hooks.as_ref())
        .and_then(|h| h.post_commit.as_deref())
    {
        crate::runner::run_lifecycle_hook(post_commit, "post-commit", &workspace.root_path, &[])
            .await?;
    }

    if should_tag {
        println!("\n{} Creating git tags...", "$".cyan());
        for (pkg_name, version) in &versioned {
            let tag_name = create_git_tag(&workspace.root_path, pkg_name, version)?;
            println!("  {} Created tag {}", "TAG".blue(), tag_name.bold());
        }
    }

    let should_push = if args.no_git_push {
        false
    } else {
        version_config.is_none_or(|c| c.should_git_push())
    };
    if should_push {
        println!("\n{} Pushing to remote...", "$".cyan());
        git_push(&workspace.root_path, should_tag)?;
        println!(
            "  {} Pushed commits{}",
            "OK".green(),
            if should_tag { " and tags" } else { "" }
        );
    }

    // Print release URLs if requested (CLI flag or config)
    let should_release_url = args.release_url
        || version_config
            .map(|c| c.should_release_url())
            .unwrap_or(false);
    if should_release_url {
        if let Some(ref repo) = workspace.config.repository {
            println!("\n{} Release URLs:", "$".cyan());
            for (pkg_name, version) in &versioned {
                let tag = format!("{}-v{}", pkg_name, version);
                let title = format!("{} v{}", pkg_name, version);
                let url = repo.release_url(&tag, &title);
                println!("  {} {}", pkg_name.bold(), url);
            }
        } else {
            println!(
                "\n{} --release-url requires `repository` in config; skipping.",
                "WARN:".yellow()
            );
        }
    }

    // Create release branch if requested (CLI flag or config pattern)
    let release_branch_pattern = if args.no_release_branch {
        None
    } else {
        args.release_branch
            .as_deref()
            .or_else(|| version_config.and_then(|c| c.release_branch_pattern()))
    };
    if let Some(pattern) = release_branch_pattern {
        // Determine the version string: use the first versioned package's version
        if let Some((_, version)) = versioned.first() {
            let original_branch = git_current_branch(&workspace.root_path)?;
            let branch_name =
                create_release_branch(&workspace.root_path, pattern, &version.to_string())?;
            println!(
                "\n{} Created release branch: {}",
                "$".cyan(),
                branch_name.bold()
            );

            if should_push {
                push_release_branch(&workspace.root_path, &branch_name)?;
                println!("  {} Pushed release branch", "OK".green());
            }

            // Switch back to the original branch
            git_checkout(&workspace.root_path, &original_branch)?;
            println!(
                "  {} Switched back to {}",
                "OK".green(),
                original_branch.bold()
            );
        }
    }

    Ok(())
}
