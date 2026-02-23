use std::path::Path;

use anyhow::{Result, bail};
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::config::{BuildMode, FlavorConfig, SimulatorConfig};
use crate::package::filter::apply_filters_with_categories;
use crate::runner::{ProcessRunner, run_lifecycle_hook};
use crate::workspace::Workspace;

/// Valid values for the --version-bump flag
const VALID_VERSION_BUMPS: &[&str] = &["patch", "minor", "major"];

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

/// Platforms that can be targeted for a build
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Android,
    Ios,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::Android => write!(f, "android"),
            Platform::Ios => write!(f, "ios"),
        }
    }
}

impl Platform {
    /// The directory that must exist in a package for this platform
    fn dir_name(&self) -> &str {
        match self {
            Platform::Android => "android",
            Platform::Ios => "ios",
        }
    }

    /// The default flutter build type for this platform
    fn default_build_type(&self) -> &str {
        match self {
            Platform::Android => "appbundle",
            Platform::Ios => "ipa",
        }
    }
}

/// Assemble a `flutter build` command string from structured config.
///
/// Produces commands like:
/// `flutter build appbundle -t lib/main_prod.dart --release --flavor prod`
pub fn build_flutter_command(
    platform: Platform,
    build_type: &str,
    flavor: &FlavorConfig,
    flavor_name: &str,
    extra_args: &[String],
) -> String {
    let mut parts = vec![
        "flutter".to_string(),
        "build".to_string(),
        build_type.to_string(),
    ];

    parts.push("-t".to_string());
    parts.push(flavor.target.clone());

    parts.push(format!("--{}", flavor.mode));

    parts.push("--flavor".to_string());
    parts.push(flavor_name.to_string());

    for arg in extra_args {
        parts.push(arg.clone());
    }

    // Suppress: platform is used for future simulator logic and dir filtering,
    // but the command itself doesn't embed the platform name
    let _ = platform;

    parts.join(" ")
}

/// Resolve the artifact output path for a Flutter build.
///
/// Flutter uses platform-specific conventions for build output:
/// - **Android AAB**: `build/app/outputs/bundle/{flavor}{Mode}/app-{flavor}-{mode}.aab`
///   (e.g., `build/app/outputs/bundle/prodRelease/app-prod-release.aab`)
/// - **Android APK**: `build/app/outputs/flutter-apk/app-{flavor}-{mode}.apk`
///   (e.g., `build/app/outputs/flutter-apk/app-prod-release.apk`)
/// - **iOS IPA**: `build/ios/ipa/*.ipa` (exact name depends on the app)
///
/// Returns `None` for unsupported combinations (e.g., iOS IPA — path is app-name-dependent).
pub fn resolve_artifact_path(
    platform: Platform,
    build_type: &str,
    flavor_name: &str,
    mode: &BuildMode,
) -> Option<String> {
    let mode_str = mode.to_string();
    match platform {
        Platform::Android => {
            let capitalized_mode = capitalize_first(&mode_str);
            match build_type {
                "appbundle" => Some(format!(
                    "build/app/outputs/bundle/{flavor}{mode}/app-{flavor}-{mode_lower}.aab",
                    flavor = flavor_name,
                    mode = capitalized_mode,
                    mode_lower = mode_str,
                )),
                "apk" => Some(format!(
                    "build/app/outputs/flutter-apk/app-{flavor}-{mode}.apk",
                    flavor = flavor_name,
                    mode = mode_str,
                )),
                _ => None,
            }
        }
        Platform::Ios => None,
    }
}

/// Capitalize the first letter of a string (e.g., "release" -> "Release").
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Expand placeholders in a simulator command template.
///
/// Supported placeholders:
/// - `{aab_path}` — resolved AAB artifact path (Android only)
/// - `{apk_path}` — resolved APK artifact path (Android only)
/// - `{output_dir}` — the directory containing the artifact
/// - `{flavor}` — the flavor name (e.g., "prod", "qa")
/// - `{mode}` — the build mode (e.g., "release", "debug")
/// - `{configuration}` — Xcode-style configuration: "Debug-{flavor}" (iOS)
///
/// Returns `Err` if the template references `{aab_path}` but no AAB path can be resolved.
pub fn expand_simulator_template(
    template: &str,
    platform: Platform,
    flavor_name: &str,
    mode: &BuildMode,
) -> Result<String> {
    let mode_str = mode.to_string();
    let mut result = template.to_string();

    result = result.replace("{flavor}", flavor_name);
    result = result.replace("{mode}", &mode_str);

    // iOS configuration: "Debug-{flavor}"
    let configuration = format!("Debug-{}", flavor_name);
    result = result.replace("{configuration}", &configuration);

    // Android artifact paths
    if result.contains("{aab_path}") {
        let aab_path =
            resolve_artifact_path(platform, "appbundle", flavor_name, mode).ok_or_else(|| {
                anyhow::anyhow!(
                    "Cannot resolve AAB path for {} {} (only Android appbundle is supported)",
                    platform,
                    flavor_name
                )
            })?;
        let output_dir = Path::new(&aab_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        result = result.replace("{aab_path}", &aab_path);
        // Also fill {output_dir} if referenced, using the aab_path's directory
        result = result.replace("{output_dir}", &output_dir);
    } else if result.contains("{apk_path}") {
        let apk_path =
            resolve_artifact_path(platform, "apk", flavor_name, mode).ok_or_else(|| {
                anyhow::anyhow!(
                    "Cannot resolve APK path for {} {} (only Android APK is supported)",
                    platform,
                    flavor_name
                )
            })?;
        let output_dir = Path::new(&apk_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        result = result.replace("{apk_path}", &apk_path);
        result = result.replace("{output_dir}", &output_dir);
    } else if result.contains("{output_dir}") {
        // Fallback: resolve output_dir from the default build type for this platform
        let default_type = platform.default_build_type();
        let artifact = resolve_artifact_path(platform, default_type, flavor_name, mode);
        let output_dir = artifact
            .as_ref()
            .and_then(|p| Path::new(p).parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        result = result.replace("{output_dir}", &output_dir);
    }

    Ok(result)
}

/// Resolve which platforms to build based on CLI flags.
///
/// Returns an error if conflicting flags are given.
fn resolve_platforms(args: &BuildArgs) -> Result<Vec<Platform>> {
    match (args.android, args.ios, args.all) {
        (false, false, false) | (false, false, true) => Ok(vec![Platform::Android, Platform::Ios]),
        (true, false, false) => Ok(vec![Platform::Android]),
        (false, true, false) => Ok(vec![Platform::Ios]),
        (true, true, false) | (true, true, true) => Ok(vec![Platform::Android, Platform::Ios]),
        (true, false, true) | (false, true, true) => Ok(vec![Platform::Android, Platform::Ios]),
    }
}

/// Resolve which flavors to build, falling back to config default.
fn resolve_flavors<'a>(
    args: &'a BuildArgs,
    config_default: Option<&'a str>,
    available: &'a [String],
) -> Result<Vec<&'a str>> {
    if !args.flavor.is_empty() {
        for f in &args.flavor {
            if !available.contains(f) {
                bail!(
                    "Unknown flavor '{}'. Available: {}",
                    f,
                    available.join(", ")
                );
            }
        }
        return Ok(args.flavor.iter().map(|s| s.as_str()).collect());
    }

    if let Some(default) = config_default {
        if !available.contains(&default.to_string()) {
            bail!(
                "Default flavor '{}' is not defined in build.flavors",
                default
            );
        }
        return Ok(vec![default]);
    }

    if available.len() == 1 {
        return Ok(vec![available[0].as_str()]);
    }

    bail!(
        "No --flavor specified and no defaultFlavor configured. Available: {}",
        available.join(", ")
    );
}

/// Resolve the Android build type from CLI flags and config.
fn resolve_android_build_type(args: &BuildArgs, workspace: &Workspace) -> String {
    if let Some(ref t) = args.build_type {
        return t.clone();
    }

    if let Some(ref cmd) = workspace.config.command
        && let Some(ref build) = cmd.build
        && let Some(ref android) = build.android
    {
        return android.default_type.clone();
    }

    Platform::Android.default_build_type().to_string()
}

/// Resolve the simulator post-build command for a platform/flavor, if applicable.
///
/// Returns `Ok(Some(expanded_command))` when `--simulator` is requested and the
/// platform's simulator config is enabled with a command template.
/// Returns `Ok(None)` when no simulator step should run.
/// Returns `Err` when `--simulator` is requested but the config is missing or disabled.
fn resolve_simulator_command(
    simulator_requested: bool,
    platform: Platform,
    build_config: &crate::config::BuildCommandConfig,
    flavor_name: &str,
    mode: &BuildMode,
) -> Result<Option<String>> {
    if !simulator_requested {
        return Ok(None);
    }

    let sim_config: Option<&SimulatorConfig> = match platform {
        Platform::Android => build_config
            .android
            .as_ref()
            .and_then(|a| a.simulator.as_ref()),
        Platform::Ios => build_config.ios.as_ref().and_then(|i| i.simulator.as_ref()),
    };

    let Some(sim) = sim_config else {
        bail!(
            "--simulator requested but no simulator config found for {}.\n\
             Add a `command.build.{}.simulator` section to melos.yaml.",
            platform,
            platform,
        );
    };

    if !sim.enabled {
        bail!(
            "--simulator requested but simulator is disabled for {}.\n\
             Set `command.build.{}.simulator.enabled: true` in melos.yaml.",
            platform,
            platform,
        );
    }

    let Some(ref template) = sim.command else {
        bail!(
            "--simulator requested but no command template found for {}.\n\
             Set `command.build.{}.simulator.command` in melos.yaml.",
            platform,
            platform,
        );
    };

    let expanded = expand_simulator_template(template, platform, flavor_name, mode)?;
    Ok(Some(expanded))
}

/// Validate the --version-bump argument value.
fn validate_version_bump(bump: &str) -> Result<()> {
    if !VALID_VERSION_BUMPS.contains(&bump) {
        bail!(
            "Invalid --version-bump value '{}'. Must be one of: {}",
            bump,
            VALID_VERSION_BUMPS.join(", ")
        );
    }
    Ok(())
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

    let platforms = resolve_platforms(&args)?;

    let available_flavors: Vec<String> = {
        let mut keys: Vec<String> = build_config.flavors.keys().cloned().collect();
        keys.sort();
        keys
    };

    let flavor_names = resolve_flavors(
        &args,
        build_config.default_flavor.as_deref(),
        &available_flavors,
    )?;

    // Merge config-level packageFilters with CLI filters
    let cli_filters: PackageFilters = (&args.filters).into();
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
                        crate::commands::version::apply_version_bump(pkg, "build")?;
                    }
                    if let Some(ref bump) = args.version_bump {
                        crate::commands::version::apply_version_bump(pkg, bump)?;
                    }
                }
            }
        }
    }

    let mut total_failed = 0usize;

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

        if packages.is_empty() {
            println!(
                "{} No {} packages matched filters — skipping.",
                "i".blue(),
                platform
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
                    if let Some(pos) = ios_args.iter().position(|a| a == "--export-options-plist") {
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
            Platform::Android => resolve_android_build_type(&args, workspace),
            Platform::Ios => "ipa".to_string(),
        };

        for flavor_name in &flavor_names {
            let flavor = build_config
                .flavors
                .get(*flavor_name)
                .expect("flavor validated in resolve_flavors");

            let cmd =
                build_flutter_command(*platform, &build_type, flavor, flavor_name, &extra_args);

            println!(
                "\n{} {} {} [{}] ({} package(s))\n",
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
                continue;
            }

            let runner = ProcessRunner::new(args.concurrency, args.fail_fast);
            let env_vars = workspace.env_vars();
            let results = runner
                .run_in_packages(&packages, &cmd, &env_vars, None, &workspace.packages)
                .await?;

            let failed = results.iter().filter(|(_, success)| !success).count();
            total_failed += failed;

            if failed > 0 && args.fail_fast {
                bail!(
                    "{} package(s) failed building {} {}",
                    failed,
                    platform,
                    flavor_name
                );
            }

            // Simulator post-build step
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
                let sim_results = sim_runner
                    .run_in_packages(&packages, &sim_cmd, &env_vars, None, &workspace.packages)
                    .await?;

                let sim_failed = sim_results.iter().filter(|(_, success)| !success).count();
                total_failed += sim_failed;

                if sim_failed > 0 && args.fail_fast {
                    bail!(
                        "{} package(s) failed simulator post-build {} {}",
                        sim_failed,
                        platform,
                        flavor_name
                    );
                }
            }
        }
    }

    // Post-hook
    if let Some(hook) = workspace.hook("build", "post") {
        run_lifecycle_hook(hook, "post-build", &workspace.root_path, &[]).await?;
    }

    if args.dry_run {
        println!(
            "\n{}",
            "DRY RUN — no commands were executed.".yellow().bold()
        );
        return Ok(());
    }

    if total_failed > 0 {
        bail!("{} package(s) failed", total_failed);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BuildMode;

    // ── build_flutter_command tests ─────────────────────────────────────

    #[test]
    fn test_build_command_android_prod_release() {
        let flavor = FlavorConfig {
            target: "lib/main_prod.dart".to_string(),
            mode: BuildMode::Release,
        };
        let cmd = build_flutter_command(Platform::Android, "appbundle", &flavor, "prod", &[]);
        assert_eq!(
            cmd,
            "flutter build appbundle -t lib/main_prod.dart --release --flavor prod"
        );
    }

    #[test]
    fn test_build_command_android_qa_debug() {
        let flavor = FlavorConfig {
            target: "lib/main_qa.dart".to_string(),
            mode: BuildMode::Debug,
        };
        let cmd = build_flutter_command(Platform::Android, "apk", &flavor, "qa", &[]);
        assert_eq!(
            cmd,
            "flutter build apk -t lib/main_qa.dart --debug --flavor qa"
        );
    }

    #[test]
    fn test_build_command_ios_prod_release() {
        let flavor = FlavorConfig {
            target: "lib/main_prod.dart".to_string(),
            mode: BuildMode::Release,
        };
        let extra = vec![
            "--export-options-plist".to_string(),
            "ios/runner/exportOptions.plist".to_string(),
        ];
        let cmd = build_flutter_command(Platform::Ios, "ipa", &flavor, "prod", &extra);
        assert_eq!(
            cmd,
            "flutter build ipa -t lib/main_prod.dart --release --flavor prod --export-options-plist ios/runner/exportOptions.plist"
        );
    }

    #[test]
    fn test_build_command_profile_mode() {
        let flavor = FlavorConfig {
            target: "lib/main.dart".to_string(),
            mode: BuildMode::Profile,
        };
        let cmd = build_flutter_command(Platform::Android, "apk", &flavor, "staging", &[]);
        assert_eq!(
            cmd,
            "flutter build apk -t lib/main.dart --profile --flavor staging"
        );
    }

    #[test]
    fn test_build_command_with_extra_args() {
        let flavor = FlavorConfig {
            target: "lib/main_dev.dart".to_string(),
            mode: BuildMode::Debug,
        };
        let extra = vec!["--split-per-abi".to_string(), "--no-shrink".to_string()];
        let cmd = build_flutter_command(Platform::Android, "apk", &flavor, "dev", &extra);
        assert_eq!(
            cmd,
            "flutter build apk -t lib/main_dev.dart --debug --flavor dev --split-per-abi --no-shrink"
        );
    }

    // ── resolve_platforms tests ─────────────────────────────────────────

    fn make_args(android: bool, ios: bool, all: bool) -> BuildArgs {
        BuildArgs {
            android,
            ios,
            all,
            flavor: vec![],
            build_type: None,
            simulator: false,
            export_options_plist: None,
            dry_run: false,
            fail_fast: false,
            concurrency: 1,
            version_bump: None,
            build_number_bump: false,
            filters: GlobalFilterArgs::default(),
        }
    }

    #[test]
    fn test_resolve_platforms_default_is_all() {
        let args = make_args(false, false, false);
        let platforms = resolve_platforms(&args).unwrap();
        assert_eq!(platforms, vec![Platform::Android, Platform::Ios]);
    }

    #[test]
    fn test_resolve_platforms_android_only() {
        let args = make_args(true, false, false);
        let platforms = resolve_platforms(&args).unwrap();
        assert_eq!(platforms, vec![Platform::Android]);
    }

    #[test]
    fn test_resolve_platforms_ios_only() {
        let args = make_args(false, true, false);
        let platforms = resolve_platforms(&args).unwrap();
        assert_eq!(platforms, vec![Platform::Ios]);
    }

    #[test]
    fn test_resolve_platforms_all_flag() {
        let args = make_args(false, false, true);
        let platforms = resolve_platforms(&args).unwrap();
        assert_eq!(platforms, vec![Platform::Android, Platform::Ios]);
    }

    #[test]
    fn test_resolve_platforms_both_explicit() {
        let args = make_args(true, true, false);
        let platforms = resolve_platforms(&args).unwrap();
        assert_eq!(platforms, vec![Platform::Android, Platform::Ios]);
    }

    // ── resolve_flavors tests ───────────────────────────────────────────

    #[test]
    fn test_resolve_flavors_explicit() {
        let mut args = make_args(false, false, false);
        args.flavor = vec!["qa".to_string()];
        let available = vec!["prod".to_string(), "qa".to_string(), "dev".to_string()];
        let flavors = resolve_flavors(&args, None, &available).unwrap();
        assert_eq!(flavors, vec!["qa"]);
    }

    #[test]
    fn test_resolve_flavors_multiple_explicit() {
        let mut args = make_args(false, false, false);
        args.flavor = vec!["prod".to_string(), "dev".to_string()];
        let available = vec!["prod".to_string(), "qa".to_string(), "dev".to_string()];
        let flavors = resolve_flavors(&args, None, &available).unwrap();
        assert_eq!(flavors, vec!["prod", "dev"]);
    }

    #[test]
    fn test_resolve_flavors_unknown_errors() {
        let mut args = make_args(false, false, false);
        args.flavor = vec!["staging".to_string()];
        let available = vec!["prod".to_string(), "qa".to_string()];
        let err = resolve_flavors(&args, None, &available).unwrap_err();
        assert!(err.to_string().contains("Unknown flavor 'staging'"));
    }

    #[test]
    fn test_resolve_flavors_default_from_config() {
        let args = make_args(false, false, false);
        let available = vec!["prod".to_string(), "qa".to_string()];
        let flavors = resolve_flavors(&args, Some("prod"), &available).unwrap();
        assert_eq!(flavors, vec!["prod"]);
    }

    #[test]
    fn test_resolve_flavors_single_available_no_default() {
        let args = make_args(false, false, false);
        let available = vec!["prod".to_string()];
        let flavors = resolve_flavors(&args, None, &available).unwrap();
        assert_eq!(flavors, vec!["prod"]);
    }

    #[test]
    fn test_resolve_flavors_multiple_available_no_default_errors() {
        let args = make_args(false, false, false);
        let available = vec!["prod".to_string(), "qa".to_string()];
        let err = resolve_flavors(&args, None, &available).unwrap_err();
        assert!(err.to_string().contains("No --flavor specified"));
    }

    // ── Platform tests ──────────────────────────────────────────────────

    #[test]
    fn test_platform_display() {
        assert_eq!(Platform::Android.to_string(), "android");
        assert_eq!(Platform::Ios.to_string(), "ios");
    }

    #[test]
    fn test_platform_dir_name() {
        assert_eq!(Platform::Android.dir_name(), "android");
        assert_eq!(Platform::Ios.dir_name(), "ios");
    }

    #[test]
    fn test_platform_default_build_type() {
        assert_eq!(Platform::Android.default_build_type(), "appbundle");
        assert_eq!(Platform::Ios.default_build_type(), "ipa");
    }

    // ── BuildMode tests ─────────────────────────────────────────────────

    #[test]
    fn test_build_mode_display() {
        assert_eq!(BuildMode::Release.to_string(), "release");
        assert_eq!(BuildMode::Debug.to_string(), "debug");
        assert_eq!(BuildMode::Profile.to_string(), "profile");
    }

    // ── Config parsing tests ────────────────────────────────────────────

    #[test]
    fn test_parse_build_config_full() {
        let yaml = r#"
            flavors:
              prod:
                target: lib/main_prod.dart
                mode: release
              qa:
                target: lib/main_qa.dart
                mode: debug
            defaultFlavor: prod
            android:
              types: [appbundle, apk]
              defaultType: appbundle
              extraArgs: ["--split-per-abi"]
            ios:
              extraArgs: ["--export-options-plist", "ios/runner/exportOptions.plist"]
            hooks:
              pre: echo starting
              post: echo done
        "#;
        let config: crate::config::BuildCommandConfig =
            yaml_serde::from_str(yaml).expect("should parse full build config");
        assert_eq!(config.flavors.len(), 2);
        assert_eq!(config.default_flavor, Some("prod".to_string()));
        let prod = config.flavors.get("prod").expect("prod flavor");
        assert_eq!(prod.target, "lib/main_prod.dart");
        assert_eq!(prod.mode, BuildMode::Release);
        let qa = config.flavors.get("qa").expect("qa flavor");
        assert_eq!(qa.target, "lib/main_qa.dart");
        assert_eq!(qa.mode, BuildMode::Debug);
        let android = config.android.expect("android config");
        assert_eq!(android.types, vec!["appbundle", "apk"]);
        assert_eq!(android.default_type, "appbundle");
        assert_eq!(android.extra_args, vec!["--split-per-abi"]);
        let ios = config.ios.expect("ios config");
        assert_eq!(
            ios.extra_args,
            vec!["--export-options-plist", "ios/runner/exportOptions.plist"]
        );
        let hooks = config.hooks.expect("hooks");
        assert_eq!(hooks.pre, Some("echo starting".to_string()));
        assert_eq!(hooks.post, Some("echo done".to_string()));
    }

    #[test]
    fn test_parse_build_config_minimal() {
        let yaml = r#"
            flavors:
              prod:
                target: lib/main.dart
        "#;
        let config: crate::config::BuildCommandConfig =
            yaml_serde::from_str(yaml).expect("should parse minimal build config");
        assert_eq!(config.flavors.len(), 1);
        assert!(config.default_flavor.is_none());
        assert!(config.android.is_none());
        assert!(config.ios.is_none());
        assert!(config.hooks.is_none());
        let prod = config.flavors.get("prod").expect("prod flavor");
        assert_eq!(prod.target, "lib/main.dart");
        assert_eq!(prod.mode, BuildMode::Release); // default
    }

    #[test]
    fn test_parse_android_config_defaults() {
        let yaml = "{}";
        let config: crate::config::AndroidBuildConfig =
            yaml_serde::from_str(yaml).expect("should parse empty android config");
        assert_eq!(config.types, vec!["appbundle"]);
        assert_eq!(config.default_type, "appbundle");
        assert!(config.extra_args.is_empty());
        assert!(config.simulator.is_none());
    }

    #[test]
    fn test_parse_simulator_config() {
        let yaml = r#"
            enabled: true
            command: "bundletool build-apks --mode=universal --bundle={aab_path}"
        "#;
        let config: crate::config::SimulatorConfig =
            yaml_serde::from_str(yaml).expect("should parse simulator config");
        assert!(config.enabled);
        assert_eq!(
            config.command,
            Some("bundletool build-apks --mode=universal --bundle={aab_path}".to_string())
        );
    }

    #[test]
    fn test_parse_flavor_mode_default_is_release() {
        let yaml = r#"
            target: lib/main.dart
        "#;
        let config: crate::config::FlavorConfig =
            yaml_serde::from_str(yaml).expect("should parse flavor with default mode");
        assert_eq!(config.mode, BuildMode::Release);
    }

    #[test]
    fn test_parse_build_config_with_package_filters() {
        let yaml = r#"
            flavors:
              prod:
                target: lib/main.dart
            packageFilters:
              flutter: true
        "#;
        let config: crate::config::BuildCommandConfig =
            yaml_serde::from_str(yaml).expect("should parse build config with filters");
        let filters = config.package_filters.expect("package_filters");
        assert_eq!(filters.flutter, Some(true));
    }

    // ── resolve_artifact_path tests ─────────────────────────────────────

    #[test]
    fn test_resolve_artifact_path_android_aab_prod_release() {
        let path =
            resolve_artifact_path(Platform::Android, "appbundle", "prod", &BuildMode::Release);
        assert_eq!(
            path,
            Some("build/app/outputs/bundle/prodRelease/app-prod-release.aab".to_string())
        );
    }

    #[test]
    fn test_resolve_artifact_path_android_aab_qa_debug() {
        let path = resolve_artifact_path(Platform::Android, "appbundle", "qa", &BuildMode::Debug);
        assert_eq!(
            path,
            Some("build/app/outputs/bundle/qaDebug/app-qa-debug.aab".to_string())
        );
    }

    #[test]
    fn test_resolve_artifact_path_android_apk_prod_release() {
        let path = resolve_artifact_path(Platform::Android, "apk", "prod", &BuildMode::Release);
        assert_eq!(
            path,
            Some("build/app/outputs/flutter-apk/app-prod-release.apk".to_string())
        );
    }

    #[test]
    fn test_resolve_artifact_path_android_apk_dev_debug() {
        let path = resolve_artifact_path(Platform::Android, "apk", "dev", &BuildMode::Debug);
        assert_eq!(
            path,
            Some("build/app/outputs/flutter-apk/app-dev-debug.apk".to_string())
        );
    }

    #[test]
    fn test_resolve_artifact_path_android_unknown_type() {
        let path = resolve_artifact_path(Platform::Android, "unknown", "prod", &BuildMode::Release);
        assert!(path.is_none());
    }

    #[test]
    fn test_resolve_artifact_path_ios_returns_none() {
        let path = resolve_artifact_path(Platform::Ios, "ipa", "prod", &BuildMode::Release);
        assert!(path.is_none());
    }

    // ── capitalize_first tests ──────────────────────────────────────────

    #[test]
    fn test_capitalize_first_basic() {
        assert_eq!(capitalize_first("release"), "Release");
        assert_eq!(capitalize_first("debug"), "Debug");
        assert_eq!(capitalize_first("profile"), "Profile");
    }

    #[test]
    fn test_capitalize_first_empty() {
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn test_capitalize_first_single_char() {
        assert_eq!(capitalize_first("a"), "A");
    }

    // ── expand_simulator_template tests ─────────────────────────────────

    #[test]
    fn test_expand_template_android_bundletool() {
        let template = "bundletool build-apks --overwrite --mode=universal --bundle={aab_path} --output={output_dir}/{flavor}-unv.apks && unzip -o {output_dir}/{flavor}-unv.apks universal.apk -d {output_dir}";
        let result =
            expand_simulator_template(template, Platform::Android, "qa", &BuildMode::Debug)
                .unwrap();
        assert_eq!(
            result,
            "bundletool build-apks --overwrite --mode=universal \
             --bundle=build/app/outputs/bundle/qaDebug/app-qa-debug.aab \
             --output=build/app/outputs/bundle/qaDebug/qa-unv.apks \
             && unzip -o build/app/outputs/bundle/qaDebug/qa-unv.apks universal.apk \
             -d build/app/outputs/bundle/qaDebug"
        );
    }

    #[test]
    fn test_expand_template_android_prod_release() {
        let template = "bundletool build-apks --bundle={aab_path} --output={output_dir}/out.apks";
        let result =
            expand_simulator_template(template, Platform::Android, "prod", &BuildMode::Release)
                .unwrap();
        assert_eq!(
            result,
            "bundletool build-apks \
             --bundle=build/app/outputs/bundle/prodRelease/app-prod-release.aab \
             --output=build/app/outputs/bundle/prodRelease/out.apks"
        );
    }

    #[test]
    fn test_expand_template_ios_xcodebuild() {
        let template = "xcodebuild -configuration {configuration} -workspace ios/Runner.xcworkspace -scheme {flavor} -sdk iphonesimulator -derivedDataPath build/ios/archive/simulator";
        let result =
            expand_simulator_template(template, Platform::Ios, "prod", &BuildMode::Release)
                .unwrap();
        assert_eq!(
            result,
            "xcodebuild -configuration Debug-prod -workspace ios/Runner.xcworkspace -scheme prod -sdk iphonesimulator -derivedDataPath build/ios/archive/simulator"
        );
    }

    #[test]
    fn test_expand_template_ios_qa() {
        let template =
            "xcodebuild -configuration {configuration} -scheme {flavor} -sdk iphonesimulator";
        let result =
            expand_simulator_template(template, Platform::Ios, "qa", &BuildMode::Debug).unwrap();
        assert_eq!(
            result,
            "xcodebuild -configuration Debug-qa -scheme qa -sdk iphonesimulator"
        );
    }

    #[test]
    fn test_expand_template_flavor_and_mode_only() {
        let template = "echo Building {flavor} in {mode} mode";
        let result =
            expand_simulator_template(template, Platform::Android, "dev", &BuildMode::Debug)
                .unwrap();
        assert_eq!(result, "echo Building dev in debug mode");
    }

    #[test]
    fn test_expand_template_apk_path() {
        let template = "install {apk_path}";
        let result =
            expand_simulator_template(template, Platform::Android, "prod", &BuildMode::Release)
                .unwrap();
        assert_eq!(
            result,
            "install build/app/outputs/flutter-apk/app-prod-release.apk"
        );
    }

    #[test]
    fn test_expand_template_output_dir_fallback() {
        let template = "ls {output_dir}";
        let result =
            expand_simulator_template(template, Platform::Android, "prod", &BuildMode::Release)
                .unwrap();
        assert_eq!(result, "ls build/app/outputs/bundle/prodRelease");
    }

    #[test]
    fn test_expand_template_output_dir_ios_fallback_empty() {
        let template = "ls {output_dir}";
        let result =
            expand_simulator_template(template, Platform::Ios, "prod", &BuildMode::Release)
                .unwrap();
        // iOS has no artifact path resolution, so output_dir is empty
        assert_eq!(result, "ls ");
    }

    #[test]
    fn test_expand_template_aab_path_fails_for_ios() {
        let template = "bundletool --bundle={aab_path}";
        let result =
            expand_simulator_template(template, Platform::Ios, "prod", &BuildMode::Release);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot resolve AAB path")
        );
    }

    // ── resolve_simulator_command tests ──────────────────────────────────

    fn make_build_config_with_simulator(
        android_sim: Option<SimulatorConfig>,
        ios_sim: Option<SimulatorConfig>,
    ) -> crate::config::BuildCommandConfig {
        use std::collections::HashMap;
        let mut flavors = HashMap::new();
        flavors.insert(
            "prod".to_string(),
            FlavorConfig {
                target: "lib/main.dart".to_string(),
                mode: BuildMode::Release,
            },
        );
        crate::config::BuildCommandConfig {
            flavors,
            default_flavor: Some("prod".to_string()),
            android: Some(crate::config::AndroidBuildConfig {
                types: vec!["appbundle".to_string()],
                default_type: "appbundle".to_string(),
                extra_args: vec![],
                simulator: android_sim,
            }),
            ios: Some(crate::config::IosBuildConfig {
                extra_args: vec![],
                simulator: ios_sim,
            }),
            package_filters: None,
            hooks: None,
        }
    }

    #[test]
    fn test_resolve_simulator_not_requested() {
        let config = make_build_config_with_simulator(None, None);
        let result = resolve_simulator_command(
            false,
            Platform::Android,
            &config,
            "prod",
            &BuildMode::Release,
        )
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_simulator_android_enabled() {
        let sim = SimulatorConfig {
            enabled: true,
            command: Some("bundletool --bundle={aab_path}".to_string()),
        };
        let config = make_build_config_with_simulator(Some(sim), None);
        let result = resolve_simulator_command(
            true,
            Platform::Android,
            &config,
            "prod",
            &BuildMode::Release,
        )
        .unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().contains("bundletool"));
    }

    #[test]
    fn test_resolve_simulator_ios_enabled() {
        let sim = SimulatorConfig {
            enabled: true,
            command: Some("xcodebuild -configuration {configuration} -scheme {flavor}".to_string()),
        };
        let config = make_build_config_with_simulator(None, Some(sim));
        let result =
            resolve_simulator_command(true, Platform::Ios, &config, "prod", &BuildMode::Release)
                .unwrap();
        assert_eq!(
            result,
            Some("xcodebuild -configuration Debug-prod -scheme prod".to_string())
        );
    }

    #[test]
    fn test_resolve_simulator_no_config_errors() {
        let config = make_build_config_with_simulator(None, None);
        let result = resolve_simulator_command(
            true,
            Platform::Android,
            &config,
            "prod",
            &BuildMode::Release,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no simulator config")
        );
    }

    #[test]
    fn test_resolve_simulator_disabled_errors() {
        let sim = SimulatorConfig {
            enabled: false,
            command: Some("bundletool".to_string()),
        };
        let config = make_build_config_with_simulator(Some(sim), None);
        let result = resolve_simulator_command(
            true,
            Platform::Android,
            &config,
            "prod",
            &BuildMode::Release,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("simulator is disabled")
        );
    }

    #[test]
    fn test_resolve_simulator_no_command_errors() {
        let sim = SimulatorConfig {
            enabled: true,
            command: None,
        };
        let config = make_build_config_with_simulator(Some(sim), None);
        let result = resolve_simulator_command(
            true,
            Platform::Android,
            &config,
            "prod",
            &BuildMode::Release,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no command template")
        );
    }

    // ── validate_version_bump tests ─────────────────────────────────────

    #[test]
    fn test_validate_version_bump_patch() {
        assert!(validate_version_bump("patch").is_ok());
    }

    #[test]
    fn test_validate_version_bump_minor() {
        assert!(validate_version_bump("minor").is_ok());
    }

    #[test]
    fn test_validate_version_bump_major() {
        assert!(validate_version_bump("major").is_ok());
    }

    #[test]
    fn test_validate_version_bump_build_rejected() {
        let err = validate_version_bump("build").unwrap_err();
        assert!(err.to_string().contains("Invalid --version-bump"));
        assert!(err.to_string().contains("patch, minor, major"));
    }

    #[test]
    fn test_validate_version_bump_empty_rejected() {
        let err = validate_version_bump("").unwrap_err();
        assert!(err.to_string().contains("Invalid --version-bump"));
    }

    #[test]
    fn test_validate_version_bump_arbitrary_rejected() {
        let err = validate_version_bump("prerelease").unwrap_err();
        assert!(err.to_string().contains("Invalid --version-bump"));
    }

    // ── apply_version_bump integration tests (filesystem) ───────────────

    #[test]
    fn test_apply_version_bump_patch() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3\n").expect("write pubspec");

        let pkg = crate::package::Package {
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

        let result = crate::commands::version::apply_version_bump(&pkg, "patch").unwrap();
        assert_eq!(result, "1.2.4");

        let content = std::fs::read_to_string(&pubspec).expect("read pubspec");
        assert!(content.contains("version: 1.2.4"));
    }

    #[test]
    fn test_apply_version_bump_minor() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3\n").expect("write pubspec");

        let pkg = crate::package::Package {
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

        let result = crate::commands::version::apply_version_bump(&pkg, "minor").unwrap();
        assert_eq!(result, "1.3.0");
    }

    #[test]
    fn test_apply_version_bump_major() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3\n").expect("write pubspec");

        let pkg = crate::package::Package {
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

        let result = crate::commands::version::apply_version_bump(&pkg, "major").unwrap();
        assert_eq!(result, "2.0.0");
    }

    #[test]
    fn test_apply_version_bump_build_number() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3+5\n").expect("write pubspec");

        let pkg = crate::package::Package {
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

        let result = crate::commands::version::apply_version_bump(&pkg, "build").unwrap();
        assert_eq!(result, "1.2.3+6");

        let content = std::fs::read_to_string(&pubspec).expect("read pubspec");
        assert!(content.contains("version: 1.2.3+6"));
    }

    #[test]
    fn test_apply_version_bump_build_number_from_zero() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.0.0\n").expect("write pubspec");

        let pkg = crate::package::Package {
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

        let result = crate::commands::version::apply_version_bump(&pkg, "build").unwrap();
        assert_eq!(result, "1.0.0+1");
    }

    #[test]
    fn test_apply_version_bump_patch_preserves_build_number() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pubspec = dir.path().join("pubspec.yaml");
        std::fs::write(&pubspec, "name: test_app\nversion: 1.2.3+42\n").expect("write pubspec");

        let pkg = crate::package::Package {
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

        let result = crate::commands::version::apply_version_bump(&pkg, "patch").unwrap();
        assert_eq!(result, "1.2.4+42");

        let content = std::fs::read_to_string(&pubspec).expect("read pubspec");
        assert!(content.contains("version: 1.2.4+42"));
    }

    // ── VALID_VERSION_BUMPS constant test ───────────────────────────────

    #[test]
    fn test_valid_version_bumps_constant() {
        assert_eq!(VALID_VERSION_BUMPS, &["patch", "minor", "major"]);
    }
}
