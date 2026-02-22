use serde::Deserialize;

use crate::cli::GlobalFilterArgs;

/// Package-level filters that can come from melos.yaml `packageFilters` or CLI flags.
///
/// Supports both script-level config (deserialized from YAML) and CLI global filter flags.
///
/// YAML example:
/// ```yaml
/// scripts:
///   test:flutter:
///     run: flutter test
///     packageFilters:
///       flutter: true
///       dirExists: test
/// ```
///
/// CLI example:
/// ```sh
/// melos-rs exec --scope="app*" --no-private -- flutter test
/// ```
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PackageFilters {
    /// Filter to only Flutter packages (true) or only Dart packages (false)
    #[serde(default)]
    pub flutter: Option<bool>,

    /// Only include packages where this directory exists
    #[serde(default)]
    pub dir_exists: Option<String>,

    /// Only include packages where this file exists
    #[serde(default)]
    pub file_exists: Option<String>,

    /// Only include packages that depend on these packages
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,

    /// Exclude packages that depend on these packages
    #[serde(default)]
    pub no_depends_on: Option<Vec<String>>,

    /// Exclude packages matching these glob patterns
    #[serde(default)]
    pub ignore: Option<Vec<String>>,

    /// Only include packages matching these glob/name patterns
    #[serde(default)]
    pub scope: Option<Vec<String>>,

    /// Exclude private packages (publish_to: none)
    #[serde(default)]
    pub no_private: bool,

    /// Only include packages changed since this git ref
    #[serde(default)]
    pub diff: Option<String>,

    /// Only include packages in these categories (from melos.yaml categories config)
    #[serde(default)]
    pub category: Option<Vec<String>>,

    /// Also include transitive dependencies of matched packages
    #[serde(default)]
    pub include_dependencies: bool,

    /// Also include transitive dependents of matched packages
    #[serde(default)]
    pub include_dependents: bool,
}

impl From<&GlobalFilterArgs> for PackageFilters {
    fn from(args: &GlobalFilterArgs) -> Self {
        Self {
            flutter: args.flutter_filter(),
            dir_exists: args.dir_exists.clone(),
            file_exists: args.file_exists.clone(),
            depends_on: if args.depends_on.is_empty() {
                None
            } else {
                Some(args.depends_on.clone())
            },
            no_depends_on: if args.no_depends_on.is_empty() {
                None
            } else {
                Some(args.no_depends_on.clone())
            },
            ignore: if args.ignore.is_empty() {
                None
            } else {
                Some(args.ignore.clone())
            },
            scope: if args.scope.is_empty() {
                None
            } else {
                Some(args.scope.clone())
            },
            no_private: args.no_private,
            diff: args.effective_diff().map(String::from),
            category: if args.category.is_empty() {
                None
            } else {
                Some(args.category.clone())
            },
            include_dependencies: args.include_dependencies,
            include_dependents: args.include_dependents,
        }
    }
}

impl PackageFilters {
    /// Merge another set of filters into this one. Values from `other` take precedence
    /// when both are set (non-None / non-empty).
    ///
    /// Used when combining global CLI filters with script-level packageFilters.
    pub fn merge(&self, other: &PackageFilters) -> PackageFilters {
        PackageFilters {
            flutter: other.flutter.or(self.flutter),
            dir_exists: other.dir_exists.clone().or_else(|| self.dir_exists.clone()),
            file_exists: other
                .file_exists
                .clone()
                .or_else(|| self.file_exists.clone()),
            depends_on: merge_opt_vec(&self.depends_on, &other.depends_on),
            no_depends_on: merge_opt_vec(&self.no_depends_on, &other.no_depends_on),
            ignore: merge_opt_vec(&self.ignore, &other.ignore),
            scope: merge_opt_vec(&self.scope, &other.scope),
            no_private: self.no_private || other.no_private,
            diff: other.diff.clone().or_else(|| self.diff.clone()),
            category: merge_opt_vec(&self.category, &other.category),
            include_dependencies: self.include_dependencies || other.include_dependencies,
            include_dependents: self.include_dependents || other.include_dependents,
        }
    }
}

/// Merge two optional vecs: if both present, concatenate; otherwise take whichever is Some.
fn merge_opt_vec(a: &Option<Vec<String>>, b: &Option<Vec<String>>) -> Option<Vec<String>> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let mut merged = a.clone();
            merged.extend(b.iter().cloned());
            Some(merged)
        }
        (Some(a), None) => Some(a.clone()),
        (None, Some(b)) => Some(b.clone()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_both_none() {
        let a = PackageFilters::default();
        let b = PackageFilters::default();
        let merged = a.merge(&b);
        assert!(merged.flutter.is_none());
        assert!(merged.scope.is_none());
        assert!(merged.ignore.is_none());
        assert!(!merged.no_private);
    }

    #[test]
    fn test_merge_other_takes_precedence() {
        let a = PackageFilters {
            flutter: Some(true),
            dir_exists: Some("test".to_string()),
            ..Default::default()
        };
        let b = PackageFilters {
            flutter: Some(false),
            ..Default::default()
        };
        let merged = a.merge(&b);
        // `other` (b) takes precedence for flutter
        assert_eq!(merged.flutter, Some(false));
        // `self` (a) dir_exists survives since b has None
        assert_eq!(merged.dir_exists, Some("test".to_string()));
    }

    #[test]
    fn test_merge_scope_concatenation() {
        let a = PackageFilters {
            scope: Some(vec!["app_*".to_string()]),
            ..Default::default()
        };
        let b = PackageFilters {
            scope: Some(vec!["core_*".to_string()]),
            ..Default::default()
        };
        let merged = a.merge(&b);
        let scope = merged.scope.unwrap();
        assert_eq!(scope, vec!["app_*", "core_*"]);
    }

    #[test]
    fn test_merge_no_private_or() {
        let a = PackageFilters {
            no_private: true,
            ..Default::default()
        };
        let b = PackageFilters::default();
        let merged = a.merge(&b);
        assert!(
            merged.no_private,
            "no_private should be true if either is true"
        );
    }

    #[test]
    fn test_merge_include_dependencies_or() {
        let a = PackageFilters::default();
        let b = PackageFilters {
            include_dependencies: true,
            ..Default::default()
        };
        let merged = a.merge(&b);
        assert!(merged.include_dependencies);
    }

    #[test]
    fn test_merge_diff_other_wins() {
        let a = PackageFilters {
            diff: Some("HEAD~5".to_string()),
            ..Default::default()
        };
        let b = PackageFilters {
            diff: Some("main".to_string()),
            ..Default::default()
        };
        let merged = a.merge(&b);
        assert_eq!(merged.diff, Some("main".to_string()));
    }

    #[test]
    fn test_from_global_filter_args() {
        let args = GlobalFilterArgs {
            scope: vec!["app*".to_string()],
            ignore: vec!["test*".to_string()],
            diff: Some("main".to_string()),
            since: None,
            dir_exists: Some("lib".to_string()),
            file_exists: None,
            flutter: true,
            no_flutter: false,
            depends_on: vec!["core".to_string()],
            no_depends_on: vec![],
            no_private: true,
            category: vec!["apps".to_string()],
            include_dependencies: true,
            include_dependents: false,
        };
        let filters: PackageFilters = (&args).into();
        assert_eq!(filters.flutter, Some(true));
        assert_eq!(filters.scope, Some(vec!["app*".to_string()]));
        assert_eq!(filters.ignore, Some(vec!["test*".to_string()]));
        assert_eq!(filters.diff, Some("main".to_string()));
        assert_eq!(filters.dir_exists, Some("lib".to_string()));
        assert!(filters.file_exists.is_none());
        assert_eq!(filters.depends_on, Some(vec!["core".to_string()]));
        assert!(filters.no_depends_on.is_none());
        assert!(filters.no_private);
        assert_eq!(filters.category, Some(vec!["apps".to_string()]));
        assert!(filters.include_dependencies);
        assert!(!filters.include_dependents);
    }

    #[test]
    fn test_from_global_filter_args_no_flutter() {
        let args = GlobalFilterArgs {
            flutter: false,
            no_flutter: true,
            ..Default::default()
        };
        let filters: PackageFilters = (&args).into();
        assert_eq!(filters.flutter, Some(false));
    }

    #[test]
    fn test_from_global_filter_args_since_alias() {
        let args = GlobalFilterArgs {
            since: Some("v1.0.0".to_string()),
            ..Default::default()
        };
        let filters: PackageFilters = (&args).into();
        assert_eq!(filters.diff, Some("v1.0.0".to_string()));
    }
}
