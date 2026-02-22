use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde::Serialize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters_with_categories;
use crate::package::Package;
use crate::workspace::Workspace;

/// Output format for the list command
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum ListFormat {
    /// Default human-readable output
    #[default]
    Default,
    /// One package per line with full path (machine-parsable)
    Parsable,
    /// JSON output
    Json,
    /// Dependency graph as an adjacency list
    Graph,
    /// Graphviz DOT format
    Gviz,
    /// Mermaid diagram format
    Mermaid,
}

/// Arguments for the `list` command
#[derive(Args, Debug)]
pub struct ListArgs {
    /// Show full package details (path, version, dependencies)
    #[arg(short, long)]
    pub long: bool,

    /// Output as JSON (shorthand for --format=json)
    #[arg(long)]
    pub json: bool,

    /// Output format
    #[arg(long, value_enum, default_value_t = ListFormat::Default)]
    pub format: ListFormat,

    /// Show relative paths instead of absolute
    #[arg(long)]
    pub relative: bool,

    /// Detect and report dependency cycles
    #[arg(long)]
    pub cycles: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// List packages in the workspace
pub async fn run(workspace: &Workspace, args: ListArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters_with_categories(&workspace.packages, &filters, Some(&workspace.root_path), &workspace.config.categories)?;

    if packages.is_empty() {
        println!("{}", "No packages found.".yellow());
        return Ok(());
    }

    // If --cycles is requested, detect and report cycles regardless of format
    if args.cycles {
        return detect_and_report_cycles(&packages);
    }

    // Determine effective format (--json flag overrides --format)
    let format = if args.json {
        ListFormat::Json
    } else {
        args.format
    };

    match format {
        ListFormat::Default => {
            if args.long {
                print_long(&packages, workspace);
            } else {
                print_default(&packages);
            }
        }
        ListFormat::Parsable => print_parsable(&packages, workspace, args.relative),
        ListFormat::Json => print_json(&packages),
        ListFormat::Graph => print_graph(&packages),
        ListFormat::Gviz => print_gviz(&packages),
        ListFormat::Mermaid => print_mermaid(&packages),
    }

    Ok(())
}

fn print_default(packages: &[Package]) {
    println!();
    for pkg in packages {
        println!("  {}", pkg.name);
    }
    println!();
}

fn print_long(packages: &[Package], workspace: &Workspace) {
    println!(
        "\n{} ({} packages)\n",
        workspace.config.name.bold(),
        packages.len()
    );
    for pkg in packages {
        let version = pkg.version.as_deref().unwrap_or("unknown");
        let pkg_type = if pkg.is_flutter {
            "flutter".cyan()
        } else {
            "dart".blue()
        };
        let private_tag = if pkg.is_private() {
            " (private)".dimmed()
        } else {
            "".dimmed()
        };
        println!(
            "  {} {} [{}]{} {}",
            pkg.name.bold(),
            version.dimmed(),
            pkg_type,
            private_tag,
            pkg.path.display().to_string().dimmed()
        );
    }
    println!();
}

fn print_parsable(packages: &[Package], workspace: &Workspace, relative: bool) {
    for pkg in packages {
        let path = if relative {
            pkg.path
                .strip_prefix(&workspace.root_path)
                .unwrap_or(&pkg.path)
                .display()
                .to_string()
        } else {
            pkg.path.display().to_string()
        };
        let version = pkg.version.as_deref().unwrap_or("0.0.0");
        println!("{}:{}:{}", pkg.name, version, path);
    }
}

/// Serializable representation of a package for JSON output
#[derive(Serialize)]
struct PackageJson<'a> {
    name: &'a str,
    version: &'a str,
    path: String,
    flutter: bool,
    private: bool,
    dependencies: &'a Vec<String>,
}

fn print_json(packages: &[Package]) {
    let entries: Vec<PackageJson> = packages
        .iter()
        .map(|p| PackageJson {
            name: &p.name,
            version: p.version.as_deref().unwrap_or("unknown"),
            path: p.path.display().to_string(),
            flutter: p.is_flutter,
            private: p.is_private(),
            dependencies: &p.dependencies,
        })
        .collect();

    // serde_json handles all escaping correctly
    match serde_json::to_string_pretty(&entries) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("Failed to serialize packages to JSON: {}", e),
    }
}

fn print_graph(packages: &[Package]) {
    // Build a set of known package names for filtering
    let known: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();

    println!();
    for pkg in packages {
        let local_deps: Vec<&str> = pkg
            .dependencies
            .iter()
            .filter(|d| known.contains(d.as_str()))
            .map(|d| d.as_str())
            .collect();

        if local_deps.is_empty() {
            println!("  {} -> (no local dependencies)", pkg.name.bold());
        } else {
            println!("  {} -> {}", pkg.name.bold(), local_deps.join(", "));
        }
    }
    println!();
}

fn print_gviz(packages: &[Package]) {
    let known: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();

    println!("digraph packages {{");
    println!("  rankdir=LR;");
    println!("  node [shape=box];");

    for pkg in packages {
        // Sanitize name for DOT (replace hyphens with underscores for node IDs)
        let node_id = pkg.name.replace('-', "_");
        println!("  {} [label=\"{}\"];", node_id, pkg.name);

        for dep in &pkg.dependencies {
            if known.contains(dep.as_str()) {
                let dep_id = dep.replace('-', "_");
                println!("  {} -> {};", node_id, dep_id);
            }
        }
    }

    println!("}}");
}

fn print_mermaid(packages: &[Package]) {
    let known: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();

    println!("graph LR");

    for pkg in packages {
        let node_id = pkg.name.replace('-', "_");
        println!("  {}[{}]", node_id, pkg.name);

        for dep in &pkg.dependencies {
            if known.contains(dep.as_str()) {
                let dep_id = dep.replace('-', "_");
                println!("  {} --> {}", node_id, dep_id);
            }
        }
    }
}

/// Detect circular dependencies among workspace packages using Kahn's algorithm.
fn detect_and_report_cycles(packages: &[Package]) -> Result<()> {
    let known: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();

    // Build adjacency list and in-degree map
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();

    for pkg in packages {
        adj.entry(pkg.name.as_str()).or_default();
        in_degree.entry(pkg.name.as_str()).or_insert(0);

        for dep in &pkg.dependencies {
            if known.contains(dep.as_str()) {
                adj.entry(pkg.name.as_str())
                    .or_default()
                    .push(dep.as_str());
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
        println!(
            "\n  {} No dependency cycles detected ({} packages).\n",
            "OK".green(),
            total
        );
    } else {
        // Packages remaining in the graph (in_degree > 0) are part of cycles
        let cycle_packages: Vec<&str> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg > 0)
            .map(|(&name, _)| name)
            .collect();

        println!(
            "\n  {} Dependency cycle(s) detected involving {} package(s):\n",
            "WARNING".yellow().bold(),
            cycle_packages.len()
        );
        for name in &cycle_packages {
            let deps: Vec<&&str> = adj
                .get(name)
                .map(|d| d.iter().filter(|dd| cycle_packages.contains(dd)).collect())
                .unwrap_or_default();
            let dep_names: Vec<&str> = deps.into_iter().copied().collect();
            println!("    {} -> {}", name.bold(), dep_names.join(", "));
        }
        println!();
    }

    Ok(())
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
        }
    }

    #[test]
    fn test_parsable_format() {
        let packages = vec![make_pkg("core", vec![]), make_pkg("app", vec!["core"])];
        // Just verify it doesn't panic; output goes to stdout
        print_parsable(
            &packages,
            &crate::workspace::Workspace {
                root_path: PathBuf::from("/workspace"),
                config: crate::config::MelosConfig {
                    name: "test".to_string(),
                    packages: vec![],
                    command: None,
                    scripts: Default::default(),
                    categories: Default::default(),
                },
                packages: packages.clone(),
            },
            false,
        );
    }

    #[test]
    fn test_cycle_detection_no_cycles() {
        let packages = vec![
            make_pkg("core", vec![]),
            make_pkg("utils", vec!["core"]),
            make_pkg("app", vec!["core", "utils"]),
        ];
        // Should not error
        let result = detect_and_report_cycles(&packages);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cycle_detection_with_cycles() {
        let packages = vec![
            make_pkg("a", vec!["b"]),
            make_pkg("b", vec!["a"]),
            make_pkg("c", vec![]),
        ];
        let result = detect_and_report_cycles(&packages);
        assert!(result.is_ok()); // Reports cycles but doesn't error
    }
}
