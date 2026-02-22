use std::collections::HashMap;

use serde::Deserialize;

use super::filter::PackageFilters;

/// Full script configuration with optional metadata and filters
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptConfig {
    /// The command to run
    pub run: String,

    /// Human-readable description of what this script does
    #[serde(default)]
    pub description: Option<String>,

    /// Package-level filters for which packages this script applies to
    #[serde(default)]
    pub package_filters: Option<PackageFilters>,

    /// Environment variables to set when running this script
    #[serde(default)]
    pub env: HashMap<String, String>,
}
