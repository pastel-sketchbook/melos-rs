use anyhow::Result;
use clap::Args;
use colored::Colorize;

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

    /// Only list Flutter packages
    #[arg(long)]
    pub flutter: bool,

    /// Only list non-Flutter (Dart) packages
    #[arg(long)]
    pub no_flutter: bool,
}

/// List packages in the workspace
pub async fn run(workspace: &Workspace, args: ListArgs) -> Result<()> {
    let packages: Vec<_> = workspace
        .packages
        .iter()
        .filter(|p| {
            if args.flutter {
                p.is_flutter
            } else if args.no_flutter {
                !p.is_flutter
            } else {
                true
            }
        })
        .collect();

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
            println!(
                "  {} {} [{}] {}",
                pkg.name.bold(),
                version.dimmed(),
                pkg_type,
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
