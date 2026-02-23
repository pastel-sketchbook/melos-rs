use crate::package::Package;

/// Paths removed during a deep clean.
pub const DEEP_CLEAN_DIRS: &[&str] = &[".dart_tool", "build"];

/// Files removed during a deep clean.
pub const DEEP_CLEAN_FILES: &[&str] = &["pubspec.lock"];

/// Result of attempting to remove a `pubspec_overrides.yaml` from a single package.
#[derive(Debug, Clone, PartialEq)]
pub enum OverrideRemoval {
    /// File was successfully removed.
    Removed,
    /// File existed but removal failed with the given error message.
    Failed(String),
    /// File did not exist (no action taken).
    NotPresent,
}

/// Remove `pubspec_overrides.yaml` files from packages (Melos 6.x mode).
///
/// Returns per-package results indicating what happened.
pub fn remove_pubspec_overrides(packages: &[Package]) -> Vec<(String, OverrideRemoval)> {
    packages
        .iter()
        .map(|pkg| {
            let override_path = pkg.path.join("pubspec_overrides.yaml");
            let result = if override_path.exists() {
                match std::fs::remove_file(&override_path) {
                    Ok(()) => OverrideRemoval::Removed,
                    Err(e) => OverrideRemoval::Failed(e.to_string()),
                }
            } else {
                OverrideRemoval::NotPresent
            };
            (pkg.name.clone(), result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_package(name: &str, path: PathBuf) -> Package {
        Package {
            name: name.to_string(),
            path,
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: vec![],
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        }
    }

    #[test]
    fn test_deep_clean_dirs_constant() {
        assert!(DEEP_CLEAN_DIRS.contains(&".dart_tool"));
        assert!(DEEP_CLEAN_DIRS.contains(&"build"));
        assert_eq!(DEEP_CLEAN_DIRS.len(), 2);
    }

    #[test]
    fn test_deep_clean_files_constant() {
        assert!(DEEP_CLEAN_FILES.contains(&"pubspec.lock"));
        assert_eq!(DEEP_CLEAN_FILES.len(), 1);
    }

    #[test]
    fn test_remove_pubspec_overrides_removes_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        // Create the override file
        let override_path = pkg_dir.join("pubspec_overrides.yaml");
        std::fs::write(
            &override_path,
            "dependency_overrides:\n  core:\n    path: ../core\n",
        )
        .unwrap();
        assert!(override_path.exists());

        let pkg = make_package("app", pkg_dir);
        let results = remove_pubspec_overrides(&[pkg]);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, OverrideRemoval::Removed);
        assert!(
            !override_path.exists(),
            "pubspec_overrides.yaml should be removed"
        );
    }

    #[test]
    fn test_remove_pubspec_overrides_no_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_dir = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let pkg = make_package("app", pkg_dir);
        let results = remove_pubspec_overrides(&[pkg]);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, OverrideRemoval::NotPresent);
    }

    #[test]
    fn test_remove_pubspec_overrides_multiple_packages() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_a_dir = dir.path().join("packages").join("a");
        let pkg_b_dir = dir.path().join("packages").join("b");
        std::fs::create_dir_all(&pkg_a_dir).unwrap();
        std::fs::create_dir_all(&pkg_b_dir).unwrap();

        // Only pkg_a has an override file
        let override_a = pkg_a_dir.join("pubspec_overrides.yaml");
        std::fs::write(&override_a, "# overrides").unwrap();

        let pkg_a = make_package("a", pkg_a_dir);
        let pkg_b = make_package("b", pkg_b_dir.clone());

        let results = remove_pubspec_overrides(&[pkg_a, pkg_b]);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, OverrideRemoval::Removed);
        assert_eq!(results[1].1, OverrideRemoval::NotPresent);
        assert!(!override_a.exists(), "a's override should be removed");
        assert!(
            !pkg_b_dir.join("pubspec_overrides.yaml").exists(),
            "b never had one"
        );
    }

    #[test]
    fn test_remove_pubspec_overrides_empty_packages() {
        let results = remove_pubspec_overrides(&[]);
        assert!(results.is_empty());
    }
}
