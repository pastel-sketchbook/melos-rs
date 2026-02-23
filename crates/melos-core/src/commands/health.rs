use std::collections::HashMap;

use crate::package::Package;

/// Options for the health command (clap-free).
#[derive(Debug, Clone)]
pub struct HealthOpts {
    pub version_drift: bool,
    pub missing_fields: bool,
    pub sdk_consistency: bool,
    pub all: bool,
    pub json: bool,
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

// ---------------------------------------------------------------------------
// Analysis functions
// ---------------------------------------------------------------------------

/// Run all enabled health checks and return a structured report.
pub fn run(packages: &[Package], opts: &HealthOpts) -> HealthReport {
    let run_all =
        opts.all || (!opts.version_drift && !opts.missing_fields && !opts.sdk_consistency);

    let mut total_issues = 0u32;

    let drift_data = if run_all || opts.version_drift {
        let data = collect_version_drift(packages);
        total_issues += data.len() as u32;
        Some(data)
    } else {
        None
    };

    let missing_data = if run_all || opts.missing_fields {
        let data = collect_missing_fields(packages);
        total_issues += data.len() as u32;
        Some(data)
    } else {
        None
    };

    let sdk_data = if run_all || opts.sdk_consistency {
        let data = collect_sdk_consistency(packages);
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

    HealthReport {
        version_drift: drift_data,
        missing_fields: missing_data,
        sdk_consistency: sdk_data,
        total_issues,
    }
}

// ---------------------------------------------------------------------------
// Version Drift
// ---------------------------------------------------------------------------

/// Collect version drift data: external dependencies used with multiple constraints.
pub fn collect_version_drift(packages: &[Package]) -> Vec<VersionDriftIssue> {
    // Collect: dep_name -> { constraint -> [package_names] }
    let mut dep_map: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();

    // Workspace package names (to skip; we only care about external deps)
    let workspace_names: std::collections::HashSet<String> =
        packages.iter().map(|p| p.name.clone()).collect();

    for pkg in packages {
        for (dep_name, constraint) in &pkg.dependency_versions {
            if workspace_names.contains(dep_name) {
                continue;
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

    // Lightweight YAML parse -- grab only the top-level keys we care about.
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

/// Collect missing-fields data for public (publishable) packages.
pub fn collect_missing_fields(packages: &[Package]) -> Vec<MissingFieldsIssue> {
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

/// Collect SDK consistency data across packages.
pub fn collect_sdk_consistency(packages: &[Package]) -> SdkConsistencyResult {
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

    let dart_sdk_drift = build_sorted_usages(&sdk_map);
    let flutter_sdk_drift = build_sorted_usages(&flutter_map);

    SdkConsistencyResult {
        missing_sdk,
        dart_sdk_drift,
        flutter_sdk_drift,
    }
}

/// Convert a constraint map into sorted [`ConstraintUsage`] entries.
pub fn build_sorted_usages(map: &HashMap<String, Vec<String>>) -> Vec<ConstraintUsage> {
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
