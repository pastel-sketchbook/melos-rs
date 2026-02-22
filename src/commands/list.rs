use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::cli::GlobalFilterArgs;
use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters;
use crate::workspace::Workspace;

/// Arguments for the `list` command
#[derive(Args, Debug)]
pub struct ListArgs {
    /// Show full package details (path, version, dependencies)
    #[arg(short, long)]
    pub long: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    #[command(flatten)]
    pub filters: GlobalFilterArgs,
}

/// List packages in the workspace
pub async fn run(workspace: &Workspace, args: ListArgs) -> Result<()> {
    let filters: PackageFilters = (&args.filters).into();
    let packages = apply_filters(&workspace.packages, &filters, Some(&workspace.root_path))?;

    if packages.is_empty() {
        println!("{}", "No packages found.".yellow());
        return Ok(());
    }

    if args.json {
        // Simple JSON output
        let json_packages: Vec<yaml_serde::Value> = packages
            .iter()
            .map(|p| {
                let mut map = yaml_serde::Mapping::new();
                map.insert(
                    yaml_serde::Value::String("name".to_string()),
                    yaml_serde::Value::String(p.name.clone()),
                );
                map.insert(
                    yaml_serde::Value::String("path".to_string()),
                    yaml_serde::Value::String(p.path.display().to_string()),
                );
                map.insert(
                    yaml_serde::Value::String("version".to_string()),
                    yaml_serde::Value::String(
                        p.version.clone().unwrap_or_else(|| "unknown".to_string()),
                    ),
                );
                map.insert(
                    yaml_serde::Value::String("flutter".to_string()),
                    yaml_serde::Value::Bool(p.is_flutter),
                );
                map.insert(
                    yaml_serde::Value::String("private".to_string()),
                    yaml_serde::Value::Bool(p.is_private()),
                );
                yaml_serde::Value::Mapping(map)
            })
            .collect();
        // Using serde_json would be better, but keeping deps minimal
        for pkg in &json_packages {
            println!("{}", yaml_serde::to_string(pkg)?);
        }
    } else if args.long {
        println!(
            "\n{} ({} packages)\n",
            workspace.config.name.bold(),
            packages.len()
        );
        for pkg in &packages {
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
    } else {
        println!();
        for pkg in &packages {
            println!("  {}", pkg.name);
        }
    }

    println!();
    Ok(())
}
