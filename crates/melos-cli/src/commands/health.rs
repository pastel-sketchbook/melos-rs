use std::collections::HashMap;

use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use melos_core::package::Package;
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

// ---------------------------------------------------------------------------
// JSON-serializable result types
// ---------------------------------------------------------------------------

/// A single constraint usage: which constraint string and which packages use it.
#[derive(serde::Serialize, Debug, Clone, PartialEq)]
pub struct ConstraintUsage {
    pub constraint: String,
    pub packages: Vec<String>,
}

/// A dependency with version drift: multiple constraints in use.
#[derive(serde::Serialize, Debug, Clone, PartialEq)]
pub struct VersionDriftIssue {
    pub dependency: String,
    pub constraints: Vec<ConstraintUsage>,
}

/// A package missing recommended pubspec fields.
#[derive(serde::Serialize, Debug, Clone, PartialEq)]
pub struct MissingFieldsIssue {
    pub package: String,
    pub missing: Vec<String>,
}

/// SDK consistency results.
#[derive(serde::Serialize, Debug, Clone, PartialEq, Default)]
pub struct SdkConsistencyResult {
    pub missing_sdk: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dart_sdk_drift: Vec<ConstraintUsage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flutter_sdk_drift: Vec<ConstraintUsage>,
}

/// Full health report for JSON output.
#[derive(serde::Serialize, Debug, Clone, PartialEq)]
pub struct HealthReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_drift: Option<Vec<VersionDriftIssue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_fields: Option<Vec<MissingFieldsIssue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdk_consistency: Option<SdkConsistencyResult>,
    pub total_issues: u32,
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
                serde_json::to_string_pretty(&report)
                    .expect("safety: HealthReport is always serializable")
            );
        } else {
            println!("{}", "No packages matched the given filters.".yellow());
        }
        return Ok(());
    }

    // If no specific check is selected, run all
    let run_all =
        args.all || (!args.version_drift && !args.missing_fields && !args.sdk_consistency);

    let mut total_issues = 0u32;

    // Collect structured data for each check
    let drift_data = if run_all || args.version_drift {
        let data = collect_version_drift(&packages);
        total_issues += data.len() as u32;
        Some(data)
    } else {
        None
    };

    let missing_data = if run_all || args.missing_fields {
        let data = collect_missing_fields(&packages);
        total_issues += data.len() as u32;
        Some(data)
    } else {
        None
    };

    let sdk_data = if run_all || args.sdk_consistency {
        let data = collect_sdk_consistency(&packages);
        let sdk_issues = if !data.missing_sdk.is_empty() {
            1u32
        } else {
            0
        } + if data.dart_sdk_drift.len() > 1 { 1 } else { 0 }
            + if data.flutter_sdk_drift.len() > 1 {
                1
            } else {
                0
            };
        total_issues += sdk_issues;
        Some(data)
    } else {
        None
    };

    if args.json {
        let report = HealthReport {
            version_drift: drift_data,
            missing_fields: missing_data,
            sdk_consistency: sdk_data,
            total_issues,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .expect("safety: HealthReport is always serializable")
        );

        if total_issues > 0 {
            // Exit non-zero for CI, but don't duplicate the message
            anyhow::bail!("{} health issue(s) found", total_issues);
        }
        return Ok(());
    }

    // Human-readable output
    println!(
        "\n{} Running health checks on {} packages...\n",
        "$".cyan(),
        packages.len()
    );

    if let Some(ref data) = drift_data {
        print_version_drift(data);
    }

    if let Some(ref data) = missing_data {
        print_missing_fields(data);
    }

    if let Some(ref data) = sdk_data {
        print_sdk_consistency(data);
    }

    println!();
    if total_issues > 0 {
        anyhow::bail!("{} health issue(s) found", total_issues);
    }

    println!("{}", "No health issues found.".green());
    Ok(())
}

// ---------------------------------------------------------------------------
// Version Drift — data collection
// ---------------------------------------------------------------------------

/// Collect version drift data without printing.
fn collect_version_drift(packages: &[Package]) -> Vec<VersionDriftIssue> {
    // Collect: dep_name -> { constraint -> [package_names] }
    let mut dep_map: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();

    // Workspace package names (to skip; we only care about external deps)
    let workspace_names: std::collections::HashSet<String> =
        packages.iter().map(|p| p.name.clone()).collect();

    for pkg in packages {
        for (dep_name, constraint) in &pkg.dependency_versions {
            if workspace_names.contains(dep_name) {
                continue; // skip workspace sibling references
            }
            dep_map
                .entry(dep_name.clone())
                .or_default()
                .entry(constraint.clone())
                .or_default()
                .push(pkg.name.clone());
        }
    }

    let mut dep_names: Vec<_> = dep_map.keys().cloned().collect();
    dep_names.sort();

    let mut issues = Vec::new();

    for dep_name in dep_names {
        let versions = dep_map.remove(&dep_name).unwrap_or_default();
        if versions.len() <= 1 {
            continue;
        }

        let mut constraints: Vec<_> = versions.keys().cloned().collect();
        constraints.sort();

        let constraint_usages: Vec<ConstraintUsage> = constraints
            .into_iter()
            .map(|c| {
                let pkgs = versions[&c].clone();
                ConstraintUsage {
                    constraint: c,
                    packages: pkgs,
                }
            })
            .collect();

        issues.push(VersionDriftIssue {
            dependency: dep_name,
            constraints: constraint_usages,
        });
    }

    issues
}

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

// ---------------------------------------------------------------------------
// Missing Fields — data collection
// ---------------------------------------------------------------------------

/// Pubspec fields read for the missing-fields health check.
#[derive(Debug, Default)]
struct PubspecHealthFields {
    description: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
    version: Option<String>,
}

/// Read health-relevant fields from a package's pubspec.yaml.
fn read_health_fields(pkg: &Package) -> PubspecHealthFields {
    let pubspec_path = pkg.path.join("pubspec.yaml");
    let content = match std::fs::read_to_string(&pubspec_path) {
        Ok(c) => c,
        Err(_) => return PubspecHealthFields::default(),
    };

    // Lightweight YAML parse — grab only the top-level keys we care about.
    #[derive(serde::Deserialize, Default)]
    struct Partial {
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        homepage: Option<String>,
        #[serde(default)]
        repository: Option<String>,
        #[serde(default)]
        version: Option<String>,
    }

    let parsed: Partial = yaml_serde::from_str(&content).unwrap_or_default();

    PubspecHealthFields {
        description: parsed.description,
        homepage: parsed.homepage,
        repository: parsed.repository,
        version: parsed.version,
    }
}

/// Collect missing-fields data without printing.
fn collect_missing_fields(packages: &[Package]) -> Vec<MissingFieldsIssue> {
    let mut issues = Vec::new();

    for pkg in packages {
        // Only check public (publishable) packages
        if pkg.is_private() {
            continue;
        }

        let fields = read_health_fields(pkg);
        let mut missing: Vec<String> = Vec::new();

        if fields.description.as_deref().unwrap_or("").is_empty() {
            missing.push("description".to_string());
        }

        // homepage OR repository should be present
        let has_homepage = fields.homepage.as_deref().is_some_and(|s| !s.is_empty());
        let has_repository = fields.repository.as_deref().is_some_and(|s| !s.is_empty());
        if !has_homepage && !has_repository {
            missing.push("homepage/repository".to_string());
        }

        if fields.version.as_deref().unwrap_or("").is_empty() {
            missing.push("version".to_string());
        }

        if !missing.is_empty() {
            issues.push(MissingFieldsIssue {
                package: pkg.name.clone(),
                missing,
            });
        }
    }

    issues
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

// ---------------------------------------------------------------------------
// SDK Consistency — data collection
// ---------------------------------------------------------------------------

/// Pubspec environment / SDK constraints.
#[derive(Debug, Default)]
struct SdkConstraints {
    sdk: Option<String>,
    flutter: Option<String>,
}

/// Read SDK constraints from a package's pubspec.yaml `environment` key.
fn read_sdk_constraints(pkg: &Package) -> SdkConstraints {
    let pubspec_path = pkg.path.join("pubspec.yaml");
    let content = match std::fs::read_to_string(&pubspec_path) {
        Ok(c) => c,
        Err(_) => return SdkConstraints::default(),
    };

    #[derive(serde::Deserialize, Default)]
    struct Env {
        #[serde(default)]
        sdk: Option<String>,
        #[serde(default)]
        flutter: Option<String>,
    }

    #[derive(serde::Deserialize, Default)]
    struct Partial {
        #[serde(default)]
        environment: Option<Env>,
    }

    let parsed: Partial = yaml_serde::from_str(&content).unwrap_or_default();

    match parsed.environment {
        Some(env) => SdkConstraints {
            sdk: env.sdk,
            flutter: env.flutter,
        },
        None => SdkConstraints::default(),
    }
}

/// Collect SDK consistency data without printing.
fn collect_sdk_consistency(packages: &[Package]) -> SdkConsistencyResult {
    let mut sdk_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut flutter_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut missing_sdk: Vec<String> = Vec::new();

    for pkg in packages {
        let constraints = read_sdk_constraints(pkg);
        match constraints.sdk {
            Some(ref s) if !s.is_empty() => {
                sdk_map.entry(s.clone()).or_default().push(pkg.name.clone());
            }
            _ => {
                missing_sdk.push(pkg.name.clone());
            }
        }

        if let Some(ref f) = constraints.flutter
            && !f.is_empty()
        {
            flutter_map
                .entry(f.clone())
                .or_default()
                .push(pkg.name.clone());
        }
    }

    // Build sorted constraint usage lists
    let dart_sdk_drift = build_sorted_usages(&sdk_map);
    let flutter_sdk_drift = build_sorted_usages(&flutter_map);

    SdkConsistencyResult {
        missing_sdk,
        dart_sdk_drift,
        flutter_sdk_drift,
    }
}

/// Convert a constraint map into sorted `ConstraintUsage` entries.
fn build_sorted_usages(map: &HashMap<String, Vec<String>>) -> Vec<ConstraintUsage> {
    let mut constraints: Vec<_> = map.keys().cloned().collect();
    constraints.sort();
    constraints
        .into_iter()
        .map(|c| ConstraintUsage {
            packages: map[&c].clone(),
            constraint: c,
        })
        .collect()
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_package(name: &str, dep_versions: HashMap<String, String>) -> Package {
        Package {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/test/{}", name)),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: dep_versions.keys().cloned().collect(),
            dev_dependencies: vec![],
            dependency_versions: dep_versions,
            resolution: None,
        }
    }

    #[test]
    fn test_version_drift_no_issues() {
        let packages = vec![
            make_package("a", HashMap::from([("http".into(), "^1.0.0".into())])),
            make_package("b", HashMap::from([("http".into(), "^1.0.0".into())])),
        ];
        let issues = collect_version_drift(&packages);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_version_drift_detected() {
        let packages = vec![
            make_package("a", HashMap::from([("http".into(), "^1.0.0".into())])),
            make_package("b", HashMap::from([("http".into(), "^2.0.0".into())])),
        ];
        let issues = collect_version_drift(&packages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].dependency, "http");
        assert_eq!(issues[0].constraints.len(), 2);
    }

    #[test]
    fn test_version_drift_skips_workspace_siblings() {
        // If "b" is a workspace package, a dep on "b" with different constraints
        // should NOT be flagged as version drift (those are sibling references).
        let packages = vec![
            make_package("a", HashMap::from([("b".into(), "^1.0.0".into())])),
            make_package("b", HashMap::new()),
        ];
        let issues = collect_version_drift(&packages);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_version_drift_multiple_deps() {
        let packages = vec![
            make_package(
                "a",
                HashMap::from([
                    ("http".into(), "^1.0.0".into()),
                    ("path".into(), "^1.8.0".into()),
                ]),
            ),
            make_package(
                "b",
                HashMap::from([
                    ("http".into(), "^2.0.0".into()),
                    ("path".into(), "^1.8.0".into()),
                ]),
            ),
        ];
        let issues = collect_version_drift(&packages);
        // Only http has drift, path is consistent
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn test_version_drift_json_serializable() {
        let issues = vec![VersionDriftIssue {
            dependency: "http".to_string(),
            constraints: vec![
                ConstraintUsage {
                    constraint: "^1.0.0".to_string(),
                    packages: vec!["a".to_string()],
                },
                ConstraintUsage {
                    constraint: "^2.0.0".to_string(),
                    packages: vec!["b".to_string()],
                },
            ],
        }];
        let json =
            serde_json::to_string(&issues).expect("safety: VersionDriftIssue should serialize");
        assert!(json.contains("http"));
        assert!(json.contains("^1.0.0"));
    }

    #[test]
    fn test_missing_fields_json_serializable() {
        let issues = vec![MissingFieldsIssue {
            package: "my_pkg".to_string(),
            missing: vec!["description".to_string(), "version".to_string()],
        }];
        let json =
            serde_json::to_string(&issues).expect("safety: MissingFieldsIssue should serialize");
        assert!(json.contains("my_pkg"));
        assert!(json.contains("description"));
    }

    #[test]
    fn test_sdk_consistency_json_serializable() {
        let result = SdkConsistencyResult {
            missing_sdk: vec!["orphan".to_string()],
            dart_sdk_drift: vec![
                ConstraintUsage {
                    constraint: ">=3.0.0 <4.0.0".to_string(),
                    packages: vec!["a".to_string()],
                },
                ConstraintUsage {
                    constraint: ">=3.2.0 <4.0.0".to_string(),
                    packages: vec!["b".to_string()],
                },
            ],
            flutter_sdk_drift: vec![],
        };
        let json =
            serde_json::to_string(&result).expect("safety: SdkConsistencyResult should serialize");
        assert!(json.contains("orphan"));
        assert!(json.contains(">=3.0.0 <4.0.0"));
        // flutter_sdk_drift should be skipped (empty)
        assert!(!json.contains("flutter_sdk_drift"));
    }

    #[test]
    fn test_health_report_json_serializable() {
        let report = HealthReport {
            version_drift: Some(vec![]),
            missing_fields: None,
            sdk_consistency: None,
            total_issues: 0,
        };
        let json =
            serde_json::to_string_pretty(&report).expect("safety: HealthReport should serialize");
        assert!(json.contains("total_issues"));
        assert!(json.contains("version_drift"));
        // missing_fields and sdk_consistency are None, should be skipped
        assert!(!json.contains("missing_fields"));
        assert!(!json.contains("sdk_consistency"));
    }

    #[test]
    fn test_collect_missing_fields_skips_private() {
        // Private packages (publish_to: "none") should be skipped
        let private_pkg = Package {
            name: "private_pkg".to_string(),
            path: PathBuf::from("/tmp/test/private_pkg"),
            version: None,
            is_flutter: false,
            publish_to: Some("none".to_string()),
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        };
        let issues = collect_missing_fields(&[private_pkg]);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_collect_sdk_consistency_missing() {
        // Packages without pubspec.yaml on disk will have missing SDK
        let pkg = make_package("missing_pubspec", HashMap::new());
        let result = collect_sdk_consistency(&[pkg]);
        assert_eq!(result.missing_sdk, vec!["missing_pubspec"]);
    }

    #[test]
    fn test_build_sorted_usages_deterministic() {
        let mut map = HashMap::new();
        map.insert("^2.0.0".to_string(), vec!["b".to_string()]);
        map.insert("^1.0.0".to_string(), vec!["a".to_string()]);

        let usages = build_sorted_usages(&map);
        assert_eq!(usages[0].constraint, "^1.0.0");
        assert_eq!(usages[1].constraint, "^2.0.0");
    }
}
