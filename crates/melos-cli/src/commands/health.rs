use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use melos_core::commands::health::{
    HealthOpts, HealthReport, MissingFieldsIssue, SdkConsistencyResult, VersionDriftIssue,
};
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::workspace::Workspace;

/// Arguments for the `health` command
#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Check for version drift: the same external dependency used at different
    /// versions across workspace packages
    #[arg(long)]
    pub version_drift: bool,

    /// Check for missing pubspec fields (description, homepage) in public packages
    #[arg(long)]
    pub missing_fields: bool,

    /// Check that SDK constraints are consistent across packages
    #[arg(long)]
    pub sdk_consistency: bool,

    /// Run all checks (default if no specific check is selected)
    #[arg(long, short = 'a')]
    pub all: bool,

    /// Output results as JSON instead of human-readable text
    #[arg(long)]
    pub json: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Run health checks on the workspace
pub async fn run(workspace: &Workspace, args: HealthArgs) -> Result<()> {
    let filters = package_filters_from_args(&args.filters);
    let packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    if packages.is_empty() {
        if args.json {
            let report = HealthReport {
                version_drift: None,
                missing_fields: None,
                sdk_consistency: None,
                total_issues: 0,
            };
            println!(
                "{}",
                // safety: HealthReport derives Serialize with only primitive/String/Option fields
                serde_json::to_string_pretty(&report).expect("HealthReport is always serializable")
            );
        } else {
            println!("{}", "No packages matched the given filters.".yellow());
        }
        return Ok(());
    }

    let opts = HealthOpts {
        version_drift: args.version_drift,
        missing_fields: args.missing_fields,
        sdk_consistency: args.sdk_consistency,
        all: args.all,
        json: args.json,
    };

    let report = melos_core::commands::health::run(&packages, &opts);

    if args.json {
        println!(
            "{}",
            // safety: HealthReport derives Serialize with only primitive/String/Option fields
            serde_json::to_string_pretty(&report).expect("HealthReport is always serializable")
        );

        if report.total_issues > 0 {
            anyhow::bail!("{} health issue(s) found", report.total_issues);
        }
        return Ok(());
    }

    // Human-readable output
    println!(
        "\n{} Running health checks on {} packages...\n",
        "$".cyan(),
        packages.len()
    );

    if let Some(ref data) = report.version_drift {
        print_version_drift(data);
    }

    if let Some(ref data) = report.missing_fields {
        print_missing_fields(data);
    }

    if let Some(ref data) = report.sdk_consistency {
        print_sdk_consistency(data);
    }

    println!();
    if report.total_issues > 0 {
        anyhow::bail!("{} health issue(s) found", report.total_issues);
    }

    println!("{}", "No health issues found.".green());
    Ok(())
}

// ---------------------------------------------------------------------------
// Presentation (colored terminal output)
// ---------------------------------------------------------------------------

/// Print version drift results in human-readable format.
fn print_version_drift(issues: &[VersionDriftIssue]) {
    println!("{}", "Version drift check".bold().underline());

    for issue in issues {
        println!(
            "  {} {} is used with {} different constraints:",
            "DRIFT".yellow().bold(),
            issue.dependency.bold(),
            issue.constraints.len()
        );
        for usage in &issue.constraints {
            println!(
                "    {} {} in: {}",
                "->".dimmed(),
                usage.constraint.cyan(),
                usage.packages.join(", ")
            );
        }
    }

    if issues.is_empty() {
        println!("  {} No version drift detected.", "OK".green());
    } else {
        println!(
            "\n  {} {} dependency(ies) have inconsistent version constraints.",
            "!".yellow(),
            issues.len()
        );
    }

    println!();
}

/// Print missing-fields results in human-readable format.
fn print_missing_fields(issues: &[MissingFieldsIssue]) {
    println!("{}", "Missing fields check".bold().underline());

    for issue in issues {
        println!(
            "  {} {} missing: {}",
            "MISS".yellow().bold(),
            issue.package.bold(),
            issue.missing.join(", ")
        );
    }

    if issues.is_empty() {
        println!(
            "  {} All public packages have required fields.",
            "OK".green()
        );
    } else {
        println!(
            "\n  {} {} public package(s) have missing recommended fields.",
            "!".yellow(),
            issues.len()
        );
    }

    println!();
}

/// Print SDK consistency results in human-readable format.
fn print_sdk_consistency(data: &SdkConsistencyResult) {
    println!("{}", "SDK consistency check".bold().underline());

    if !data.missing_sdk.is_empty() {
        println!(
            "  {} {} package(s) missing SDK constraint: {}",
            "MISS".yellow().bold(),
            data.missing_sdk.len(),
            data.missing_sdk.join(", ")
        );
    }

    if data.dart_sdk_drift.len() > 1 {
        println!(
            "  {} Dart SDK constraint used with {} different values:",
            "DRIFT".yellow().bold(),
            data.dart_sdk_drift.len()
        );
        for usage in &data.dart_sdk_drift {
            println!(
                "    {} {} in: {}",
                "->".dimmed(),
                usage.constraint.cyan(),
                usage.packages.join(", ")
            );
        }
    }

    if data.flutter_sdk_drift.len() > 1 {
        println!(
            "  {} Flutter SDK constraint used with {} different values:",
            "DRIFT".yellow().bold(),
            data.flutter_sdk_drift.len()
        );
        for usage in &data.flutter_sdk_drift {
            println!(
                "    {} {} in: {}",
                "->".dimmed(),
                usage.constraint.cyan(),
                usage.packages.join(", ")
            );
        }
    }

    let has_issues = !data.missing_sdk.is_empty()
        || data.dart_sdk_drift.len() > 1
        || data.flutter_sdk_drift.len() > 1;

    if !has_issues {
        println!("  {} SDK constraints are consistent.", "OK".green());
    }

    println!();
}
