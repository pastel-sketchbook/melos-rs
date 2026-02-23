use std::collections::{HashMap, HashSet, VecDeque};

use crate::package::Package;

/// Serializable representation of a package for JSON output.
#[derive(serde::Serialize, Debug, Clone)]
pub struct PackageJson<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub path: String,
    pub flutter: bool,
    pub private: bool,
    pub dependencies: &'a Vec<String>,
}

/// Build a list of [`PackageJson`] from packages for JSON serialization.
pub fn build_packages_json(packages: &[Package]) -> Vec<PackageJson<'_>> {
    packages
        .iter()
        .map(|p| PackageJson {
            name: &p.name,
            version: p.version.as_deref().unwrap_or("unknown"),
            path: p.path.display().to_string(),
            flutter: p.is_flutter,
            private: p.is_private(),
            dependencies: &p.dependencies,
        })
        .collect()
}

/// Result of dependency cycle detection.
#[derive(Debug, Clone)]
pub struct CycleResult {
    /// Packages involved in cycles, with their cycle-participating dependencies.
    pub cycle_packages: Vec<(String, Vec<String>)>,
    /// Total number of packages analyzed.
    pub total: usize,
}

impl CycleResult {
    /// Whether any cycles were detected.
    pub fn has_cycles(&self) -> bool {
        !self.cycle_packages.is_empty()
    }
}

/// Detect circular dependencies among workspace packages using Kahn's algorithm.
pub fn detect_cycles(packages: &[Package]) -> CycleResult {
    let known: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();

    // Build adjacency list and in-degree map
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();

    for pkg in packages {
        adj.entry(pkg.name.as_str()).or_default();
        in_degree.entry(pkg.name.as_str()).or_insert(0);

        for dep in &pkg.dependencies {
            if known.contains(dep.as_str()) {
                adj.entry(pkg.name.as_str()).or_default().push(dep.as_str());
                *in_degree.entry(dep.as_str()).or_insert(0) += 1;
            }
        }
    }

    // Kahn's algorithm: topological sort
    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|&(_, &deg)| deg == 0)
        .map(|(&name, _)| name)
        .collect();
    let mut visited = 0usize;

    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(neighbors) = adj.get(node) {
            for &neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(neighbor) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }

    let total = packages.len();
    if visited == total {
        CycleResult {
            cycle_packages: vec![],
            total,
        }
    } else {
        let cycle_names: HashSet<&str> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg > 0)
            .map(|(&name, _)| name)
            .collect();

        let mut cycle_packages: Vec<(String, Vec<String>)> = cycle_names
            .iter()
            .map(|&name| {
                let deps: Vec<String> = adj
                    .get(name)
                    .map(|d| {
                        d.iter()
                            .filter(|dd| cycle_names.contains(**dd))
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                (name.to_string(), deps)
            })
            .collect();

        // Sort for deterministic output
        cycle_packages.sort_by(|a, b| a.0.cmp(&b.0));

        CycleResult {
            cycle_packages,
            total,
        }
    }
}

/// Generate Graphviz DOT format string.
pub fn generate_gviz(packages: &[Package]) -> String {
    let known: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    let mut lines = Vec::new();

    lines.push("digraph packages {".to_string());
    lines.push("  rankdir=LR;".to_string());
    lines.push("  node [shape=box];".to_string());

    for pkg in packages {
        let node_id = pkg.name.replace('-', "_");
        lines.push(format!("  {} [label=\"{}\"];", node_id, pkg.name));

        for dep in &pkg.dependencies {
            if known.contains(dep.as_str()) {
                let dep_id = dep.replace('-', "_");
                lines.push(format!("  {} -> {};", node_id, dep_id));
            }
        }
    }

    lines.push("}".to_string());
    lines.join("\n")
}

/// Generate Mermaid diagram format string.
pub fn generate_mermaid(packages: &[Package]) -> String {
    let known: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    let mut lines = Vec::new();

    lines.push("graph LR".to_string());

    for pkg in packages {
        let node_id = pkg.name.replace('-', "_");
        lines.push(format!("  {}[{}]", node_id, pkg.name));

        for dep in &pkg.dependencies {
            if known.contains(dep.as_str()) {
                let dep_id = dep.replace('-', "_");
                lines.push(format!("  {} --> {}", node_id, dep_id));
            }
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_pkg(name: &str, deps: Vec<&str>) -> Package {
        Package {
            name: name.to_string(),
            path: PathBuf::from(format!("/workspace/packages/{}", name)),
            version: Some("1.0.0".to_string()),
            is_flutter: false,
            publish_to: None,
            dependencies: deps.into_iter().map(String::from).collect(),
            dev_dependencies: vec![],
            dependency_versions: HashMap::new(),
            resolution: None,
        }
    }

    #[test]
    fn test_cycle_detection_no_cycles() {
        let packages = vec![
            make_pkg("core", vec![]),
            make_pkg("utils", vec!["core"]),
            make_pkg("app", vec!["core", "utils"]),
        ];
        let result = detect_cycles(&packages);
        assert!(!result.has_cycles());
        assert_eq!(result.total, 3);
    }

    #[test]
    fn test_cycle_detection_with_cycles() {
        let packages = vec![
            make_pkg("a", vec!["b"]),
            make_pkg("b", vec!["a"]),
            make_pkg("c", vec![]),
        ];
        let result = detect_cycles(&packages);
        assert!(result.has_cycles());
        assert_eq!(result.cycle_packages.len(), 2);
        assert_eq!(result.total, 3);
    }

    #[test]
    fn test_generate_gviz_basic() {
        let packages = vec![make_pkg("core", vec![]), make_pkg("app", vec!["core"])];
        let output = generate_gviz(&packages);
        assert!(output.contains("digraph packages {"));
        assert!(output.contains("core [label=\"core\"]"));
        assert!(output.contains("app -> core"));
        assert!(output.ends_with('}'));
    }

    #[test]
    fn test_generate_mermaid_basic() {
        let packages = vec![make_pkg("core", vec![]), make_pkg("app", vec!["core"])];
        let output = generate_mermaid(&packages);
        assert!(output.starts_with("graph LR"));
        assert!(output.contains("core[core]"));
        assert!(output.contains("app --> core"));
    }

    #[test]
    fn test_build_packages_json() {
        let packages = vec![make_pkg("core", vec![])];
        let json_entries = build_packages_json(&packages);
        assert_eq!(json_entries.len(), 1);
        assert_eq!(json_entries[0].name, "core");
        assert_eq!(json_entries[0].version, "1.0.0");
        assert!(!json_entries[0].flutter);
    }
}
