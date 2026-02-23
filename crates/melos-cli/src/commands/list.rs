use std::collections::HashSet;

use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::filter_ext::package_filters_from_args;
use melos_core::commands::list::{
    build_packages_json, detect_cycles, generate_gviz, generate_mermaid,
};
use melos_core::package::Package;
use melos_core::package::filter::apply_filters_with_categories;
use melos_core::workspace::Workspace;

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
    #[arg(short, long)]
    pub relative: bool,

    /// Output as parsable list (shorthand for --format=parsable)
    #[arg(short, long)]
    pub parsable: bool,

    /// Show dependency graph (shorthand for --format=graph)
    #[arg(long)]
    pub graph: bool,

    /// Show dependency graph in Graphviz DOT language (shorthand for --format=gviz)
    #[arg(long)]
    pub gviz: bool,

    /// Show dependency graph in Mermaid diagram format (shorthand for --format=mermaid)
    #[arg(long)]
    pub mermaid: bool,

    /// Detect and report dependency cycles
    #[arg(long)]
    pub cycles: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// List packages in the workspace
pub async fn run(workspace: &Workspace, args: ListArgs) -> Result<()> {
    let filters = package_filters_from_args(&args.filters);
    let packages = apply_filters_with_categories(
        &workspace.packages,
        &filters,
        Some(&workspace.root_path),
        &workspace.config.categories,
    )?;

    if packages.is_empty() {
        println!("{}", "No packages found.".yellow());
        return Ok(());
    }

    // If --cycles is requested, detect and report cycles regardless of format
    if args.cycles {
        return detect_and_report_cycles(&packages);
    }

    // Determine effective format (shorthand flags override --format)
    let format = match (
        args.json,
        args.parsable,
        args.graph,
        args.gviz,
        args.mermaid,
    ) {
        (true, _, _, _, _) => ListFormat::Json,
        (_, true, _, _, _) => ListFormat::Parsable,
        (_, _, true, _, _) => ListFormat::Graph,
        (_, _, _, true, _) => ListFormat::Gviz,
        (_, _, _, _, true) => ListFormat::Mermaid,
        _ => args.format,
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
        ListFormat::Gviz => println!("{}", generate_gviz(&packages)),
        ListFormat::Mermaid => println!("{}", generate_mermaid(&packages)),
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

fn print_json(packages: &[Package]) {
    let entries = build_packages_json(packages);

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

/// Detect cycles using core logic and report with colored output.
fn detect_and_report_cycles(packages: &[Package]) -> Result<()> {
    let result = detect_cycles(packages);

    if !result.has_cycles() {
        println!(
            "\n  {} No dependency cycles detected ({} packages).\n",
            "OK".green(),
            result.total
        );
    } else {
        println!(
            "\n  {} Dependency cycle(s) detected involving {} package(s):\n",
            "WARNING".yellow().bold(),
            result.cycle_packages.len()
        );
        for (name, deps) in &result.cycle_packages {
            println!("    {} -> {}", name.bold(), deps.join(", "));
        }
        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
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
    fn test_parsable_format() {
        let packages = vec![make_pkg("core", vec![]), make_pkg("app", vec!["core"])];
        // Just verify it doesn't panic; output goes to stdout
        print_parsable(
            &packages,
            &melos_core::workspace::Workspace {
                root_path: PathBuf::from("/workspace"),
                config_source: melos_core::config::ConfigSource::MelosYaml(PathBuf::from(
                    "/workspace/melos.yaml",
                )),
                config: melos_core::config::MelosConfig {
                    name: "test".to_string(),
                    packages: vec![],
                    repository: None,
                    sdk_path: None,
                    command: None,
                    scripts: Default::default(),
                    ignore: None,
                    categories: Default::default(),
                    use_root_as_package: None,
                    discover_nested_workspaces: None,
                },
                packages: packages.clone(),
                sdk_path: None,
                warnings: vec![],
            },
            false,
        );
    }
}
