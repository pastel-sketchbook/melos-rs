use serde::Deserialize;

/// Package-level filters defined in script configuration
///
/// These correspond to the `packageFilters` block in melos.yaml scripts, e.g.:
/// ```yaml
/// scripts:
///   test:flutter:
///     run: flutter test
///     packageFilters:
///       flutter: true
///       dirExists: test
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

    /// Exclude packages matching these names
    #[serde(default)]
    pub ignore: Option<Vec<String>>,

    /// Only include packages matching these scope names
    #[serde(default)]
    pub scope: Option<Vec<String>>,
}
