use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use crate::runner::run_lifecycle_hook;
use melos_core::commands::build::{
    BuildStepResult, Platform, build_flutter_command, format_duration, resolve_android_build_type,
    resolve_flavors, resolve_platforms, resolve_simulator_command, validate_version_bump,
};
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::runner::ProcessRunner;
use melos_core::workspace::Workspace;

/// Arguments for the `build` command
#[derive(Args, Debug)]
pub struct BuildArgs {
    /// Build for Android
    #[arg(long)]
    pub android: bool,

    /// Build for iOS
    #[arg(long)]
    pub ios: bool,

    /// Build for all platforms (default when neither --android nor --ios is specified)
    #[arg(long)]
    pub all: bool,

    /// Build flavor/environment (can be specified multiple times; default: config defaultFlavor)
    #[arg(long)]
    pub flavor: Vec<String>,

    /// Android build type: apk, appbundle (default: config defaultType)
    #[arg(long = "type")]
    pub build_type: Option<String>,

    /// Build simulator-compatible artifacts (bundletool for Android, xcodebuild for iOS)
    #[arg(long)]
    pub simulator: bool,

    /// Override export options plist for iOS builds
    #[arg(long)]
    pub export_options_plist: Option<String>,

    /// Print commands without executing them
    #[arg(long)]
    pub dry_run: bool,

    /// Stop execution on first failure
    #[arg(long)]
    pub fail_fast: bool,

    /// Maximum number of concurrent build processes
    #[arg(short = 'c', long, default_value = "1")]
    pub concurrency: usize,

    /// Bump version before building: patch, minor, or major
    #[arg(long)]
    pub version_bump: Option<String>,

    /// Increment build number before building
    #[arg(long)]
    pub build_number_bump: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Format a per-step completion line.
///
/// Example: `  OK android prod [release]: 3/3 passed (12.3s)`
fn format_step_result(result: &BuildStepResult) -> String {
    if result.skipped {
        return format!(
            "  {} {} {}: skipped (no matching packages)",
            "-".dimmed(),
            result.platform.to_string().dimmed(),
            result.flavor.dimmed(),
        );
    }

    let duration_str = format_duration(result.duration);
    if result.failed == 0 {
        format!(
            "  {} {} {} [{}]: {}/{} passed ({})",
            "OK".green(),
            result.platform,
            result.flavor,
            result.mode,
            result.passed,
            result.total_packages,
            duration_str,
        )
    } else {
        format!(
            "  {} {} {} [{}]: {}/{} failed ({})",
            "FAIL".red(),
            result.platform,
            result.flavor,
            result.mode,
            result.failed,
            result.total_packages,
            duration_str,
        )
    }
}

/// Format the final build summary table.
fn format_build_summary(results: &[BuildStepResult], total_duration: Duration) -> String {
    let mut lines = Vec::new();
    lines.push(format!("\n{}", "BUILD SUMMARY".bold()));

    let total_passed: usize = results.iter().map(|r| r.passed).sum();
    let total_failed: usize = results.iter().map(|r| r.failed).sum();
    let total_skipped = results.iter().filter(|r| r.skipped).count();

    for result in results {
        lines.push(format_step_result(result));
    }

    lines.push(String::new());

    let mut summary_parts = Vec::new();
    if total_passed > 0 {
        summary_parts.push(format!("{} passed", total_passed.to_string().green()));
    }
    if total_failed > 0 {
        summary_parts.push(format!("{} failed", total_failed.to_string().red()));
    }
    if total_skipped > 0 {
        summary_parts.push(format!("{} skipped", total_skipped.to_string().dimmed()));
    }

    lines.push(format!(
        "  Total: {} ({})",
        summary_parts.join(", "),
        format_duration(total_duration),
    ));

    lines.join("\n")
}

/// Run the `build` command
pub async fn run(workspace: &Workspace, args: BuildArgs) -> Result<()> {
    let build_config = workspace
        .config
        .command
        .as_ref()
        .and_then(|c| c.build.as_ref());

    let Some(build_config) = build_config else {
        bail!(
            "No build configuration found. Add a `command.build` section to melos.yaml.\n\
             See: docs/rationale/0004_build_command.md"
        );
    };

    if build_config.flavors.is_empty() {
        bail!("No flavors defined in command.build.flavors");
    }

    let platforms = resolve_platforms(args.android, args.ios, args.all)?;

    let available_flavors: Vec<String> = {
        let mut keys: Vec<String> = build_config.flavors.keys().cloned().collect();
        keys.sort();
        keys
    };

    let flavor_names = resolve_flavors(
        &args.flavor,
        build_config.default_flavor.as_deref(),
        &available_flavors,
    )?;

    // Merge config-level packageFilters with CLI filters
    let cli_filters = package_filters_from_args(&args.filters);
    let base_filters = if let Some(ref config_filters) = build_config.package_filters {
        config_filters.merge(&cli_filters)
    } else {
        cli_filters
    };

    // Pre-hook
    if let Some(hook) = workspace.hook("build", "pre") {
        run_lifecycle_hook(hook, "pre-build", &workspace.root_path, &[]).await?;
    }

    // Version bump: apply to all matching Flutter packages before building
    if let Some(ref bump) = args.version_bump {
        validate_version_bump(bump)?;
    }

    let needs_version_bump = args.version_bump.is_some() || args.build_number_bump;
    if needs_version_bump {
        // Collect all flutter packages (union of all platforms) for version bumping
        let mut version_filters = base_filters.clone();
        version_filters.flutter = Some(true);
        let version_packages = apply_filters_with_categories(
            &workspace.packages,
            &version_filters,
            Some(&workspace.root_path),
            &workspace.config.categories,
        )?;

        if version_packages.is_empty() {
            println!(
                "{} No Flutter packages matched filters — skipping version bump.",
                "i".blue(),
            );
        } else {
            // Determine bump types to apply (build number first, then version)
            let bump_label = match (&args.version_bump, args.build_number_bump) {
                (Some(v), true) => format!("{} + build number", v),
                (Some(v), false) => v.clone(),
                (None, true) => "build number".to_string(),
                (None, false) => unreachable!(),
            };

            println!(
                "\n{} Bumping {} ({} package(s))\n",
                "VERSION".yellow().bold(),
                bump_label.bold(),
                version_packages.len().to_string().cyan(),
            );

            if args.dry_run {
                for pkg in &version_packages {
                    let current = pkg.version.as_deref().unwrap_or("0.0.0");
                    println!("  {} {} ({})", "DRY".yellow(), pkg.name, current);
                }
            } else {
                for pkg in &version_packages {
                    // Build number bump first (if both requested, bump build number then version)
                    if args.build_number_bump {
                        let new_ver =
                            melos_core::commands::version::apply_version_bump(pkg, "build")?;
                        println!(
                            "  {} Updated {} to {}",
                            "OK".green(),
                            pkg.path.join("pubspec.yaml").display(),
                            new_ver
                        );
                    }
                    if let Some(ref bump) = args.version_bump {
                        let new_ver = melos_core::commands::version::apply_version_bump(pkg, bump)?;
                        println!(
                            "  {} Updated {} to {}",
                            "OK".green(),
                            pkg.path.join("pubspec.yaml").display(),
                            new_ver
                        );
                    }
                }
            }
        }
    }

    let mut total_failed = 0usize;
    let mut step_results: Vec<BuildStepResult> = Vec::new();
    let total_steps = platforms.len() * flavor_names.len();
    let mut current_step = 0usize;
    let build_start = Instant::now();

    // Build plan header
    println!(
        "\n{} {} platform(s) × {} flavor(s) = {} step(s)\n",
        "BUILD PLAN".bold(),
        platforms.len().to_string().cyan(),
        flavor_names.len().to_string().cyan(),
        total_steps.to_string().cyan(),
    );

    for platform in &platforms {
        // Add platform-specific dir_exists filter
        let mut filters = base_filters.clone();
        filters.flutter = Some(true);
        if filters.dir_exists.is_none() {
            filters.dir_exists = Some(platform.dir_name().to_string());
        }

        let packages = apply_filters_with_categories(
            &workspace.packages,
            &filters,
            Some(&workspace.root_path),
            &workspace.config.categories,
        )?;

        for flavor_name in &flavor_names {
            current_step += 1;
            let flavor = build_config
                .flavors
                .get(*flavor_name)
                .expect("flavor validated in resolve_flavors");

            if packages.is_empty() {
                step_results.push(BuildStepResult {
                    platform: *platform,
                    flavor: flavor_name.to_string(),
                    mode: flavor.mode.to_string(),
                    total_packages: 0,
                    passed: 0,
                    failed: 0,
                    duration: Duration::ZERO,
                    skipped: true,
                });
                println!(
                    "[{}/{}] {} No {} packages matched filters — skipping {}.",
                    current_step,
                    total_steps,
                    "-".dimmed(),
                    platform,
                    flavor_name,
                );
                continue;
            }

            // Resolve extra args per platform
            let extra_args: Vec<String> = match platform {
                Platform::Android => build_config
                    .android
                    .as_ref()
                    .map(|a| a.extra_args.clone())
                    .unwrap_or_default(),
                Platform::Ios => {
                    let mut ios_args = build_config
                        .ios
                        .as_ref()
                        .map(|i| i.extra_args.clone())
                        .unwrap_or_default();
                    // CLI --export-options-plist overrides config
                    if let Some(ref plist) = args.export_options_plist {
                        // Remove existing --export-options-plist if present
                        if let Some(pos) =
                            ios_args.iter().position(|a| a == "--export-options-plist")
                        {
                            ios_args.remove(pos);
                            if pos < ios_args.len() {
                                ios_args.remove(pos);
                            }
                        }
                        ios_args.push("--export-options-plist".to_string());
                        ios_args.push(plist.clone());
                    }
                    ios_args
                }
            };

            let build_type = match platform {
                Platform::Android => resolve_android_build_type(
                    args.build_type.as_deref(),
                    build_config.android.as_ref(),
                ),
                Platform::Ios => "ipa".to_string(),
            };

            let cmd =
                build_flutter_command(*platform, &build_type, flavor, flavor_name, &extra_args);

            println!(
                "\n[{}/{}] {} {} {} [{}] ({} package(s))\n",
                current_step,
                total_steps,
                "BUILD".cyan().bold(),
                platform.to_string().bold(),
                flavor_name.bold(),
                format!("--{}", flavor.mode).dimmed(),
                packages.len().to_string().cyan(),
            );

            if args.dry_run {
                for pkg in &packages {
                    println!("  {} {} → {}", "DRY".yellow(), pkg.name, cmd);
                }

                // Show simulator post-build in dry-run too
                if let Some(sim_cmd) = resolve_simulator_command(
                    args.simulator,
                    *platform,
                    build_config,
                    flavor_name,
                    &flavor.mode,
                )? {
                    for pkg in &packages {
                        println!("  {} {} → {}", "DRY".yellow().dimmed(), pkg.name, sim_cmd);
                    }
                }
                step_results.push(BuildStepResult {
                    platform: *platform,
                    flavor: flavor_name.to_string(),
                    mode: flavor.mode.to_string(),
                    total_packages: packages.len(),
                    passed: packages.len(),
                    failed: 0,
                    duration: Duration::ZERO,
                    skipped: false,
                });
                continue;
            }

            let step_start = Instant::now();

            let runner = ProcessRunner::new(args.concurrency, args.fail_fast);
            let env_vars = workspace.env_vars();
            let (tx, render_handle) = crate::render::spawn_plain_renderer();
            let results = runner
                .run_in_packages_with_events(
                    &packages,
                    &cmd,
                    &env_vars,
                    None,
                    Some(&tx),
                    &workspace.packages,
                )
                .await?;
            drop(tx);
            render_handle.await??;

            let failed = results.iter().filter(|(_, success)| !success).count();
            let passed = results.len() - failed;
            total_failed += failed;

            // Simulator post-build step
            let mut sim_failed = 0usize;
            if let Some(sim_cmd) = resolve_simulator_command(
                args.simulator,
                *platform,
                build_config,
                flavor_name,
                &flavor.mode,
            )? {
                println!(
                    "\n{} {} {} simulator post-build ({} package(s))\n",
                    "SIMULATOR".magenta().bold(),
                    platform.to_string().bold(),
                    flavor_name.bold(),
                    packages.len().to_string().cyan(),
                );

                // Run simulator command sequentially in each package dir
                // (concurrency=1: bundletool/xcodebuild are heavy processes)
                let sim_runner = ProcessRunner::new(1, args.fail_fast);
                let (sim_tx, sim_render) = crate::render::spawn_plain_renderer();
                let sim_results = sim_runner
                    .run_in_packages_with_events(
                        &packages,
                        &sim_cmd,
                        &env_vars,
                        None,
                        Some(&sim_tx),
                        &workspace.packages,
                    )
                    .await?;
                drop(sim_tx);
                sim_render.await??;

                sim_failed = sim_results.iter().filter(|(_, success)| !success).count();
                total_failed += sim_failed;
            }

            let step_duration = step_start.elapsed();
            let step_total_failed = failed + sim_failed;

            let step_result = BuildStepResult {
                platform: *platform,
                flavor: flavor_name.to_string(),
                mode: flavor.mode.to_string(),
                total_packages: packages.len(),
                passed: if step_total_failed == 0 {
                    packages.len()
                } else {
                    passed
                },
                failed: step_total_failed,
                duration: step_duration,
                skipped: false,
            };

            println!("{}", format_step_result(&step_result));
            step_results.push(step_result);

            if step_total_failed > 0 && args.fail_fast {
                bail!(
                    "{} package(s) failed building {} {}",
                    step_total_failed,
                    platform,
                    flavor_name
                );
            }
        }
    }

    // Post-hook
    if let Some(hook) = workspace.hook("build", "post") {
        run_lifecycle_hook(hook, "post-build", &workspace.root_path, &[]).await?;
    }

    let total_duration = build_start.elapsed();

    if args.dry_run {
        println!(
            "\n{}",
            "DRY RUN — no commands were executed.".yellow().bold()
        );
        return Ok(());
    }

    // Print final summary
    println!("{}", format_build_summary(&step_results, total_duration));

    if total_failed > 0 {
        bail!("{} package(s) failed", total_failed);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Strip ANSI escape codes so assertions work regardless of terminal env.
    fn strip_ansi(s: &str) -> String {
        // safety: this regex is valid
        let re = regex::Regex::new(r"\x1b\[[0-9;]*m").expect("valid regex");
        re.replace_all(s, "").to_string()
    }

    // -- format_step_result tests --

    #[test]
    fn test_format_step_result_all_passed() {
        let result = BuildStepResult {
            platform: Platform::Android,
            flavor: "prod".to_string(),
            mode: "release".to_string(),
            total_packages: 3,
            passed: 3,
            failed: 0,
            duration: Duration::from_secs_f64(12.3),
            skipped: false,
        };
        let output = strip_ansi(&format_step_result(&result));
        assert!(output.contains("android"));
        assert!(output.contains("prod"));
        assert!(output.contains("[release]"));
        assert!(output.contains("3/3 passed"));
        assert!(output.contains("12.3s"));
    }

    #[test]
    fn test_format_step_result_with_failures() {
        let result = BuildStepResult {
            platform: Platform::Ios,
            flavor: "qa".to_string(),
            mode: "debug".to_string(),
            total_packages: 4,
            passed: 2,
            failed: 2,
            duration: Duration::from_secs_f64(8.1),
            skipped: false,
        };
        let output = strip_ansi(&format_step_result(&result));
        assert!(output.contains("ios"));
        assert!(output.contains("qa"));
        assert!(output.contains("[debug]"));
        assert!(output.contains("2/4 failed"));
        assert!(output.contains("8.1s"));
    }

    #[test]
    fn test_format_step_result_skipped() {
        let result = BuildStepResult {
            platform: Platform::Ios,
            flavor: "prod".to_string(),
            mode: "release".to_string(),
            total_packages: 0,
            passed: 0,
            failed: 0,
            duration: Duration::ZERO,
            skipped: true,
        };
        let output = strip_ansi(&format_step_result(&result));
        assert!(output.contains("ios"));
        assert!(output.contains("prod"));
        assert!(output.contains("skipped"));
    }

    // -- format_build_summary tests --

    #[test]
    fn test_format_build_summary_all_passed() {
        let results = vec![
            BuildStepResult {
                platform: Platform::Android,
                flavor: "prod".to_string(),
                mode: "release".to_string(),
                total_packages: 3,
                passed: 3,
                failed: 0,
                duration: Duration::from_secs(10),
                skipped: false,
            },
            BuildStepResult {
                platform: Platform::Ios,
                flavor: "prod".to_string(),
                mode: "release".to_string(),
                total_packages: 2,
                passed: 2,
                failed: 0,
                duration: Duration::from_secs(15),
                skipped: false,
            },
        ];
        let output = format_build_summary(&results, Duration::from_secs(25));
        let plain = strip_ansi(&output);
        assert!(plain.contains("BUILD SUMMARY"));
        assert!(plain.contains("5 passed"));
        assert!(plain.contains("25.0s"));
        // No "failed" in output when all pass
        assert!(!plain.contains("failed"));
    }

    #[test]
    fn test_format_build_summary_with_failures() {
        let results = vec![
            BuildStepResult {
                platform: Platform::Android,
                flavor: "prod".to_string(),
                mode: "release".to_string(),
                total_packages: 3,
                passed: 3,
                failed: 0,
                duration: Duration::from_secs(10),
                skipped: false,
            },
            BuildStepResult {
                platform: Platform::Ios,
                flavor: "prod".to_string(),
                mode: "release".to_string(),
                total_packages: 2,
                passed: 1,
                failed: 1,
                duration: Duration::from_secs(8),
                skipped: false,
            },
        ];
        let output = format_build_summary(&results, Duration::from_secs(18));
        let plain = strip_ansi(&output);
        assert!(plain.contains("BUILD SUMMARY"));
        assert!(plain.contains("4 passed"));
        assert!(plain.contains("1 failed"));
        assert!(plain.contains("18.0s"));
    }

    #[test]
    fn test_format_build_summary_with_skipped() {
        let results = vec![
            BuildStepResult {
                platform: Platform::Android,
                flavor: "prod".to_string(),
                mode: "release".to_string(),
                total_packages: 3,
                passed: 3,
                failed: 0,
                duration: Duration::from_secs(10),
                skipped: false,
            },
            BuildStepResult {
                platform: Platform::Ios,
                flavor: "prod".to_string(),
                mode: "release".to_string(),
                total_packages: 0,
                passed: 0,
                failed: 0,
                duration: Duration::ZERO,
                skipped: true,
            },
        ];
        let output = format_build_summary(&results, Duration::from_secs(10));
        let plain = strip_ansi(&output);
        assert!(plain.contains("BUILD SUMMARY"));
        assert!(plain.contains("3 passed"));
        assert!(plain.contains("1 skipped"));
        assert!(plain.contains("skipped"));
    }

    #[test]
    fn test_format_build_summary_mixed() {
        let results = vec![
            BuildStepResult {
                platform: Platform::Android,
                flavor: "prod".to_string(),
                mode: "release".to_string(),
                total_packages: 3,
                passed: 3,
                failed: 0,
                duration: Duration::from_secs(12),
                skipped: false,
            },
            BuildStepResult {
                platform: Platform::Android,
                flavor: "qa".to_string(),
                mode: "debug".to_string(),
                total_packages: 3,
                passed: 2,
                failed: 1,
                duration: Duration::from_secs(8),
                skipped: false,
            },
            BuildStepResult {
                platform: Platform::Ios,
                flavor: "prod".to_string(),
                mode: "release".to_string(),
                total_packages: 2,
                passed: 2,
                failed: 0,
                duration: Duration::from_secs(15),
                skipped: false,
            },
            BuildStepResult {
                platform: Platform::Ios,
                flavor: "qa".to_string(),
                mode: "debug".to_string(),
                total_packages: 0,
                passed: 0,
                failed: 0,
                duration: Duration::ZERO,
                skipped: true,
            },
        ];
        let output = format_build_summary(&results, Duration::from_secs(35));
        let plain = strip_ansi(&output);
        assert!(plain.contains("BUILD SUMMARY"));
        assert!(plain.contains("7 passed"));
        assert!(plain.contains("1 failed"));
        assert!(plain.contains("1 skipped"));
        assert!(plain.contains("35.0s"));
    }

    // -- apply_version_bump integration tests (filesystem) --

    #[test]
    fn test_apply_version_bump_patch() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3\n").expect("write pubspec");

        let pkg = melos_core::package::Package {
            name: "test_app".to_string(),
            path: dir.path().to_path_buf(),
            version: Some("1.2.3".to_string()),
            is_flutter: true,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: std::collections::HashMap::new(),
            resolution: None,
        };

        let result = melos_core::commands::version::apply_version_bump(&pkg, "patch").unwrap();
        assert_eq!(result, "1.2.4");

        let content = std::fs::read_to_string(&pubspec).expect("read pubspec");
        assert!(content.contains("version: 1.2.4"));
    }

    #[test]
    fn test_apply_version_bump_minor() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3\n").expect("write pubspec");

        let pkg = melos_core::package::Package {
            name: "test_app".to_string(),
            path: dir.path().to_path_buf(),
            version: Some("1.2.3".to_string()),
            is_flutter: true,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: std::collections::HashMap::new(),
            resolution: None,
        };

        let result = melos_core::commands::version::apply_version_bump(&pkg, "minor").unwrap();
        assert_eq!(result, "1.3.0");
    }

    #[test]
    fn test_apply_version_bump_major() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3\n").expect("write pubspec");

        let pkg = melos_core::package::Package {
            name: "test_app".to_string(),
            path: dir.path().to_path_buf(),
            version: Some("1.2.3".to_string()),
            is_flutter: true,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: std::collections::HashMap::new(),
            resolution: None,
        };

        let result = melos_core::commands::version::apply_version_bump(&pkg, "major").unwrap();
        assert_eq!(result, "2.0.0");
    }

    #[test]
    fn test_apply_version_bump_build_number() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3+5\n").expect("write pubspec");

        let pkg = melos_core::package::Package {
            name: "test_app".to_string(),
            path: dir.path().to_path_buf(),
            version: Some("1.2.3+5".to_string()),
            is_flutter: true,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: std::collections::HashMap::new(),
            resolution: None,
        };

        let result = melos_core::commands::version::apply_version_bump(&pkg, "build").unwrap();
        assert_eq!(result, "1.2.3+6");

        let content = std::fs::read_to_string(&pubspec).expect("read pubspec");
        assert!(content.contains("version: 1.2.3+6"));
    }

    #[test]
    fn test_apply_version_bump_build_number_from_zero() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.0.0\n").expect("write pubspec");

        let pkg = melos_core::package::Package {
            name: "test_app".to_string(),
            path: dir.path().to_path_buf(),
            version: Some("1.0.0".to_string()),
            is_flutter: true,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: std::collections::HashMap::new(),
            resolution: None,
        };

        let result = melos_core::commands::version::apply_version_bump(&pkg, "build").unwrap();
        assert_eq!(result, "1.0.0+1");
    }

    #[test]
    fn test_apply_version_bump_patch_preserves_build_number() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3+42\n").expect("write pubspec");

        let pkg = melos_core::package::Package {
            name: "test_app".to_string(),
            path: dir.path().to_path_buf(),
            version: Some("1.2.3+42".to_string()),
            is_flutter: true,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: std::collections::HashMap::new(),
            resolution: None,
        };

        let result = melos_core::commands::version::apply_version_bump(&pkg, "patch").unwrap();
        assert_eq!(result, "1.2.4+42");

        let content = std::fs::read_to_string(&pubspec).expect("read pubspec");
        assert!(content.contains("version: 1.2.4+42"));
    }
}
