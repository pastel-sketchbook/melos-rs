use anyhow::{Result, bail};
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::FlavorConfig;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters_with_categories;
use crate::runner::{ProcessRunner, run_lifecycle_hook};
use crate::workspace::Workspace;

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
}
