use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

use super::filter::PackageFilters;

/// Full script configuration with optional metadata and filters
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptConfig {
    /// The command to run (required when `exec` is an object or absent; unused with `exec` string shorthand)
    #[serde(default)]
    pub run: String,

    /// Exec configuration: per-package command execution.
    ///
    /// - As a **string**: shorthand for `run: melos exec -- <string>`.
    ///   The string is the command to run in each package.
    /// - As an **object**: provides exec options (concurrency, failFast, orderDependents).
    ///   Paired with `run:` which contains the command to execute in each package.
    #[serde(default)]
    pub exec: Option<ExecEntry>,

    /// Multi-step workflow: each entry is either a shell command or a script name reference.
    ///
    /// When `steps` is present, `run` and `exec` are ignored.
    /// Steps are executed sequentially. Each step is resolved as:
    /// 1. If it matches a script name in the config → execute that script inline
    /// 2. Otherwise → execute as a shell command at workspace root
    ///
    /// `packageFilters` and exec options cannot be used on the steps wrapper.
    #[serde(default)]
    pub steps: Option<Vec<String>>,

    /// Whether this script is private (hidden from interactive selection and `run --list`).
    ///
    /// Private scripts can only be called as `steps:` references or explicitly by name.
    /// Use `--include-private` to override.
    #[serde(default)]
    pub private: Option<bool>,

    /// Human-readable description of what this script does
    #[serde(default)]
    pub description: Option<String>,

    /// Package-level filters for which packages this script applies to
    #[serde(default)]
    pub package_filters: Option<PackageFilters>,

    /// Environment variables to set when running this script
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Groups this script belongs to (for `run --group <name>` filtering).
    ///
    /// Scripts can belong to zero or more groups. When `--group` is specified,
    /// only scripts that belong to at least one matching group are shown/run.
    #[serde(default)]
    pub groups: Option<Vec<String>>,
}

/// Exec configuration that can be either a string shorthand or an options object.
///
/// - String: the command to run in each package (no `run:` needed)
/// - Object: exec options like concurrency/failFast, paired with `run:` for the command
#[derive(Debug, Clone)]
pub enum ExecEntry {
    /// String shorthand: the command to run in each package.
    /// Equivalent to `run: melos exec -- <command>`.
    Command(String),

    /// Object with exec options (paired with `run:` for the command).
    Options(ExecOptions),
}

/// Exec options for per-package command execution.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecOptions {
    /// Number of concurrent processes (default: 5)
    #[serde(default)]
    pub concurrency: Option<usize>,

    /// Stop on first failure
    #[serde(default)]
    pub fail_fast: bool,

    /// Execute packages in dependency order
    #[serde(default)]
    pub order_dependents: bool,
}

impl<'de> Deserialize<'de> for ExecEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        struct ExecEntryVisitor;

        impl<'de> de::Visitor<'de> for ExecEntryVisitor {
            type Value = ExecEntry;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str(
                    "a command string or an object with exec options (concurrency, failFast, orderDependents)",
                )
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ExecEntry::Command(v.to_string()))
            }

            fn visit_map<M: de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                let opts = ExecOptions::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(ExecEntry::Options(opts))
            }
        }

        deserializer.deserialize_any(ExecEntryVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_entry_string_shorthand() {
        let yaml = r#"
run: ""
exec: "echo hello"
"#;
        let config: ScriptConfig = yaml_serde::from_str(yaml).unwrap();
        assert!(matches!(config.exec, Some(ExecEntry::Command(ref cmd)) if cmd == "echo hello"));
    }

    #[test]
    fn test_exec_entry_object_options() {
        let yaml = r#"
run: flutter test
exec:
  concurrency: 3
  failFast: true
  orderDependents: true
"#;
        let config: ScriptConfig = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(config.run, "flutter test");
        match config.exec {
            Some(ExecEntry::Options(ref opts)) => {
                assert_eq!(opts.concurrency, Some(3));
                assert!(opts.fail_fast);
                assert!(opts.order_dependents);
            }
            _ => panic!("Expected ExecEntry::Options"),
        }
    }

    #[test]
    fn test_exec_entry_object_defaults() {
        let yaml = r#"
run: dart test
exec: {}
"#;
        let config: ScriptConfig = yaml_serde::from_str(yaml).unwrap();
        match config.exec {
            Some(ExecEntry::Options(ref opts)) => {
                assert_eq!(opts.concurrency, None);
                assert!(!opts.fail_fast);
                assert!(!opts.order_dependents);
            }
            _ => panic!("Expected ExecEntry::Options"),
        }
    }

    #[test]
    fn test_steps_parsing() {
        let yaml = r#"
steps:
  - analyze
  - dart format --set-exit-if-changed .
  - test:unit
"#;
        let config: ScriptConfig = yaml_serde::from_str(yaml).unwrap();
        let steps = config.steps.unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0], "analyze");
        assert_eq!(steps[1], "dart format --set-exit-if-changed .");
        assert_eq!(steps[2], "test:unit");
    }

    #[test]
    fn test_private_field() {
        let yaml = r#"
run: echo internal
private: true
"#;
        let config: ScriptConfig = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(config.private, Some(true));
    }

    #[test]
    fn test_private_field_default() {
        let yaml = r#"
run: echo hello
"#;
        let config: ScriptConfig = yaml_serde::from_str(yaml).unwrap();
        assert!(config.private.is_none());
    }

    #[test]
    fn test_run_defaults_to_empty_string() {
        let yaml = r#"
exec: "echo from exec"
"#;
        let config: ScriptConfig = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(config.run, "");
    }

    #[test]
    fn test_full_script_with_all_fields() {
        let yaml = r#"
run: flutter test
exec:
  concurrency: 2
  failFast: true
description: Run tests across packages
private: false
packageFilters:
  flutter: true
  dirExists: test
env:
  CI: "true"
"#;
        let config: ScriptConfig = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(config.run, "flutter test");
        assert!(config.exec.is_some());
        assert_eq!(
            config.description.as_deref(),
            Some("Run tests across packages")
        );
        assert_eq!(config.private, Some(false));
        assert!(config.package_filters.is_some());
        assert_eq!(config.env.get("CI"), Some(&"true".to_string()));
    }
}
