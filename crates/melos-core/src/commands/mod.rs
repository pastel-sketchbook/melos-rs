pub mod analyze;
pub mod bootstrap;
pub mod clean;
pub mod exec;
pub mod format;
pub mod health;
pub mod init;
pub mod list;
pub mod publish;
pub mod test;

/// Results from running a command across multiple packages.
///
/// Each entry is a `(package_name, success)` tuple.
#[derive(Debug, Clone)]
pub struct PackageResults {
    pub results: Vec<(String, bool)>,
}

impl PackageResults {
    /// Number of packages that succeeded.
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|(_, s)| *s).count()
    }

    /// Number of packages that failed.
    pub fn failed(&self) -> usize {
        self.results.iter().filter(|(_, s)| !*s).count()
    }
}

impl From<Vec<(String, bool)>> for PackageResults {
    fn from(results: Vec<(String, bool)>) -> Self {
        Self { results }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_results_counts() {
        let results = PackageResults::from(vec![
            ("a".to_string(), true),
            ("b".to_string(), false),
            ("c".to_string(), true),
        ]);
        assert_eq!(results.passed(), 2);
        assert_eq!(results.failed(), 1);
    }

    #[test]
    fn test_package_results_all_passed() {
        let results = PackageResults::from(vec![("a".to_string(), true), ("b".to_string(), true)]);
        assert_eq!(results.passed(), 2);
        assert_eq!(results.failed(), 0);
    }

    #[test]
    fn test_package_results_empty() {
        let results = PackageResults::from(vec![]);
        assert_eq!(results.passed(), 0);
        assert_eq!(results.failed(), 0);
    }
}
