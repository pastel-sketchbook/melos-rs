use std::collections::HashMap;

use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::Package;
use crate::package::filter::apply_filters_with_categories;
use crate::workspace::Workspace;

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

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// Run health checks on the workspace
pub async fn run(workspace: &Workspace, args: HealthArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    // If no specific check is selected, run all
    let run_all =
        args.all || (!args.version_drift && !args.missing_fields && !args.sdk_consistency);

    let mut issues = 0u32;

    println!(
        "\n{} Running health checks on {} packages...\n",
        "$".cyan(),
        packages.len()
    );

    if run_all || args.version_drift {
        issues += check_version_drift(&packages);
    }

    if run_all || args.missing_fields {
        issues += check_missing_fields(&packages);
    }

    if run_all || args.sdk_consistency {
        issues += check_sdk_consistency(&packages);
    }

    println!();
    if issues > 0 {
        anyhow::bail!("{} health issue(s) found", issues);
    }

    println!("{}", "No health issues found.".green());
    Ok(())
}

// ---------------------------------------------------------------------------
// Version Drift
// ---------------------------------------------------------------------------

/// Check for the same external dependency being used at different version
/// constraints across workspace packages.
fn check_version_drift(packages: &[Package]) -> u32 {
    println!("{}", "Version drift check".bold().underline());

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

    let mut issues = 0u32;

    // Sort for deterministic output
    let mut dep_names: Vec<_> = dep_map.keys().cloned().collect();
    dep_names.sort();

    for dep_name in &dep_names {
        let versions = &dep_map[dep_name];
        if versions.len() <= 1 {
            continue; // consistent — only one version constraint used
        }

        issues += 1;
        println!(
            "  {} {} is used with {} different constraints:",
            "DRIFT".yellow().bold(),
            dep_name.bold(),
            versions.len()
        );

        let mut constraints: Vec<_> = versions.keys().cloned().collect();
        constraints.sort();

        for constraint in &constraints {
            let users = &versions[constraint];
            println!(
                "    {} {} in: {}",
                "->".dimmed(),
                constraint.cyan(),
                users.join(", ")
            );
        }
    }

    if issues == 0 {
        println!("  {} No version drift detected.", "OK".green());
    } else {
        println!(
            "\n  {} {} dependency(ies) have inconsistent version constraints.",
            "!".yellow(),
            issues
        );
    }

    println!();
    issues
}

// ---------------------------------------------------------------------------
// Missing Fields
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

/// Check public packages for missing recommended pubspec fields.
fn check_missing_fields(packages: &[Package]) -> u32 {
    println!("{}", "Missing fields check".bold().underline());

    let mut issues = 0u32;

    for pkg in packages {
        // Only check public (publishable) packages
        if pkg.is_private() {
            continue;
        }

        let fields = read_health_fields(pkg);
        let mut missing: Vec<&str> = Vec::new();

        if fields.description.as_deref().unwrap_or("").is_empty() {
            missing.push("description");
        }

        // homepage OR repository should be present
        let has_homepage = fields.homepage.as_deref().is_some_and(|s| !s.is_empty());
        let has_repository = fields.repository.as_deref().is_some_and(|s| !s.is_empty());
        if !has_homepage && !has_repository {
            missing.push("homepage/repository");
        }

        if fields.version.as_deref().unwrap_or("").is_empty() {
            missing.push("version");
        }

        if !missing.is_empty() {
            issues += 1;
            println!(
                "  {} {} missing: {}",
                "MISS".yellow().bold(),
                pkg.name.bold(),
                missing.join(", ")
            );
        }
    }

    if issues == 0 {
        println!(
            "  {} All public packages have required fields.",
            "OK".green()
        );
    } else {
        println!(
            "\n  {} {} public package(s) have missing recommended fields.",
            "!".yellow(),
            issues
        );
    }

    println!();
    issues
}

// ---------------------------------------------------------------------------
// SDK Consistency
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

/// Check that SDK constraints are consistent across packages.
fn check_sdk_consistency(packages: &[Package]) -> u32 {
    println!("{}", "SDK consistency check".bold().underline());

    // constraint -> [package_names]
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

    let mut issues = 0u32;

    // Report missing SDK constraints
    if !missing_sdk.is_empty() {
        issues += 1;
        println!(
            "  {} {} package(s) missing SDK constraint: {}",
            "MISS".yellow().bold(),
            missing_sdk.len(),
            missing_sdk.join(", ")
        );
    }

    // Report SDK drift
    if sdk_map.len() > 1 {
        issues += 1;
        println!(
            "  {} Dart SDK constraint used with {} different values:",
            "DRIFT".yellow().bold(),
            sdk_map.len()
        );
        let mut constraints: Vec<_> = sdk_map.keys().cloned().collect();
        constraints.sort();
        for constraint in &constraints {
            let users = &sdk_map[constraint];
            println!(
                "    {} {} in: {}",
                "->".dimmed(),
                constraint.cyan(),
                users.join(", ")
            );
        }
    }

    // Report Flutter SDK drift
    if flutter_map.len() > 1 {
        issues += 1;
        println!(
            "  {} Flutter SDK constraint used with {} different values:",
            "DRIFT".yellow().bold(),
            flutter_map.len()
        );
        let mut constraints: Vec<_> = flutter_map.keys().cloned().collect();
        constraints.sort();
        for constraint in &constraints {
            let users = &flutter_map[constraint];
            println!(
                "    {} {} in: {}",
                "->".dimmed(),
                constraint.cyan(),
                users.join(", ")
            );
        }
    }

    if issues == 0 {
        println!("  {} SDK constraints are consistent.", "OK".green());
    }

    println!();
    issues
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
        }
    }

    #[test]
    fn test_version_drift_no_issues() {
        let packages = vec![
            make_package("a", HashMap::from([("http".into(), "^1.0.0".into())])),
            make_package("b", HashMap::from([("http".into(), "^1.0.0".into())])),
        ];
        let issues = check_version_drift(&packages);
        assert_eq!(issues, 0);
    }

    #[test]
    fn test_version_drift_detected() {
        let packages = vec![
            make_package("a", HashMap::from([("http".into(), "^1.0.0".into())])),
            make_package("b", HashMap::from([("http".into(), "^2.0.0".into())])),
        ];
        let issues = check_version_drift(&packages);
        assert_eq!(issues, 1);
    }

    #[test]
    fn test_version_drift_skips_workspace_siblings() {
        // If "b" is a workspace package, a dep on "b" with different constraints
        // should NOT be flagged as version drift (those are sibling references).
        let packages = vec![
            make_package("a", HashMap::from([("b".into(), "^1.0.0".into())])),
            make_package("b", HashMap::new()),
        ];
        let issues = check_version_drift(&packages);
        assert_eq!(issues, 0);
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
        let issues = check_version_drift(&packages);
        // Only http has drift, path is consistent
        assert_eq!(issues, 1);
    }
}
