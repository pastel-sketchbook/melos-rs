use serde::Deserialize;

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

    /// Filter by published status.
    ///
    /// - `Some(true)`: only include publishable packages (publish_to is NOT "none")
    /// - `Some(false)`: only include non-published/private packages (publish_to IS "none")
    /// - `None`: no filter
    #[serde(default)]
    pub published: Option<bool>,
}

impl PackageFilters {
    /// Returns true if no filter criteria are set (everything is default/empty).
    pub fn is_empty(&self) -> bool {
        self.flutter.is_none()
            && self.dir_exists.is_none()
            && self.file_exists.is_none()
            && self.depends_on.is_none()
            && self.no_depends_on.is_none()
            && self.ignore.is_none()
            && self.scope.is_none()
            && !self.no_private
            && self.diff.is_none()
            && self.category.is_none()
            && !self.include_dependencies
            && !self.include_dependents
            && self.published.is_none()
    }

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
            published: other.published.or(self.published),
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
    fn test_merge_published_other_wins() {
        let a = PackageFilters {
            published: Some(true),
            ..Default::default()
        };
        let b = PackageFilters {
            published: Some(false),
            ..Default::default()
        };
        let merged = a.merge(&b);
        assert_eq!(merged.published, Some(false));
    }

    #[test]
    fn test_merge_published_fallback() {
        let a = PackageFilters {
            published: Some(true),
            ..Default::default()
        };
        let b = PackageFilters::default();
        let merged = a.merge(&b);
        assert_eq!(merged.published, Some(true));
    }

    #[test]
    fn test_is_empty_default() {
        assert!(PackageFilters::default().is_empty());
    }

    #[test]
    fn test_is_empty_with_scope() {
        let f = PackageFilters {
            scope: Some(vec!["app*".to_string()]),
            ..Default::default()
        };
        assert!(!f.is_empty());
    }

    #[test]
    fn test_is_empty_with_flutter() {
        let f = PackageFilters {
            flutter: Some(true),
            ..Default::default()
        };
        assert!(!f.is_empty());
    }

    #[test]
    fn test_is_empty_with_no_private() {
        let f = PackageFilters {
            no_private: true,
            ..Default::default()
        };
        assert!(!f.is_empty());
    }
}
