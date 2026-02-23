use std::path::Path;

use anyhow::{Context, Result, bail};

/// Write Melos 7.x config: pubspec.yaml with `melos:` key.
pub fn write_7x_config(dir: &Path, name: &str, package_patterns: &[String]) -> Result<()> {
    let pubspec_path = dir.join("pubspec.yaml");
    if pubspec_path.exists() {
        bail!(
            "pubspec.yaml already exists at '{}'. Remove it first or use a different directory.",
            pubspec_path.display()
        );
    }

    let workspace_entries: String = package_patterns
        .iter()
        .map(|p| format!("  - {}", p))
        .collect::<Vec<_>>()
        .join("\n");

    let content = format!(
        r#"name: {name}

environment:
  sdk: ^3.0.0

workspace:
{workspace_entries}

melos:
  scripts: {{}}
"#
    );

    std::fs::write(&pubspec_path, content)
        .with_context(|| format!("Failed to write {}", pubspec_path.display()))?;

    Ok(())
}

/// Write Melos 6.x config: separate melos.yaml + basic pubspec.yaml.
pub fn write_legacy_config(dir: &Path, name: &str, package_patterns: &[String]) -> Result<()> {
    let melos_path = dir.join("melos.yaml");
    if melos_path.exists() {
        bail!(
            "melos.yaml already exists at '{}'. Remove it first or use a different directory.",
            melos_path.display()
        );
    }

    let pubspec_path = dir.join("pubspec.yaml");
    if pubspec_path.exists() {
        bail!(
            "pubspec.yaml already exists at '{}'. Remove it first or use a different directory.",
            pubspec_path.display()
        );
    }

    // melos.yaml
    let packages_yaml: String = package_patterns
        .iter()
        .map(|p| format!("  - {}", p))
        .collect::<Vec<_>>()
        .join("\n");

    let melos_content = format!(
        r#"name: {name}

packages:
{packages_yaml}

scripts: {{}}
"#
    );

    std::fs::write(&melos_path, melos_content)
        .with_context(|| format!("Failed to write {}", melos_path.display()))?;

    // pubspec.yaml (basic root package)
    let pubspec_content = format!(
        r#"name: {name}

environment:
  sdk: ^3.0.0

dev_dependencies:
  melos: ^7.0.0
"#
    );

    std::fs::write(&pubspec_path, pubspec_content)
        .with_context(|| format!("Failed to write {}", pubspec_path.display()))?;

    Ok(())
}

/// Create a directory if it doesn't already exist.
pub fn create_dir_if_missing(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_write_7x_config() {
        let dir = TempDir::new().unwrap();
        let patterns = vec!["packages/*".to_string(), "apps/*".to_string()];
        write_7x_config(dir.path(), "my_workspace", &patterns).unwrap();

        let pubspec = std::fs::read_to_string(dir.path().join("pubspec.yaml")).unwrap();
        assert!(pubspec.contains("name: my_workspace"));
        assert!(pubspec.contains("workspace:"));
        assert!(pubspec.contains("  - packages/*"));
        assert!(pubspec.contains("  - apps/*"));
        assert!(pubspec.contains("melos:"));
        assert!(pubspec.contains("scripts: {}"));
        assert!(pubspec.contains("sdk: ^3.0.0"));
    }

    #[test]
    fn test_write_legacy_config() {
        let dir = TempDir::new().unwrap();
        let patterns = vec!["packages/*".to_string()];
        write_legacy_config(dir.path(), "my_workspace", &patterns).unwrap();

        let melos = std::fs::read_to_string(dir.path().join("melos.yaml")).unwrap();
        assert!(melos.contains("name: my_workspace"));
        assert!(melos.contains("  - packages/*"));

        let pubspec = std::fs::read_to_string(dir.path().join("pubspec.yaml")).unwrap();
        assert!(pubspec.contains("name: my_workspace"));
        assert!(pubspec.contains("melos: ^7.0.0"));
    }

    #[test]
    fn test_write_7x_config_already_exists() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("pubspec.yaml"), "existing").unwrap();

        let result = write_7x_config(dir.path(), "test", &["packages/*".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_write_legacy_config_melos_already_exists() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("melos.yaml"), "existing").unwrap();

        let result = write_legacy_config(dir.path(), "test", &["packages/*".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_create_dir_if_missing() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("a").join("b").join("c");
        assert!(!sub.exists());
        create_dir_if_missing(&sub).unwrap();
        assert!(sub.exists());

        // Call again -- should be a no-op
        create_dir_if_missing(&sub).unwrap();
        assert!(sub.exists());
    }

    #[test]
    fn test_write_7x_config_multiple_patterns() {
        let dir = TempDir::new().unwrap();
        let patterns = vec![
            "packages/*".to_string(),
            "apps/*".to_string(),
            "modules/**".to_string(),
        ];
        write_7x_config(dir.path(), "multi", &patterns).unwrap();

        let pubspec = std::fs::read_to_string(dir.path().join("pubspec.yaml")).unwrap();
        assert!(pubspec.contains("  - packages/*"));
        assert!(pubspec.contains("  - apps/*"));
        assert!(pubspec.contains("  - modules/**"));
    }
}
