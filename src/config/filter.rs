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
    #[allow(dead_code)]
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
