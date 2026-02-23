use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Args;
use colored::Colorize;

/// Arguments for the `init` command
#[derive(Args, Debug)]
pub struct InitArgs {
    /// Workspace name (defaults to current directory name)
    pub name: Option<String>,

    /// Directory to create the workspace in
    #[arg(short = 'd', long)]
    pub directory: Option<String>,

    /// Additional package glob patterns (comma-separated, can be repeated)
    #[arg(short = 'p', long)]
    pub packages: Vec<String>,

    /// Use legacy 6.x format (melos.yaml) instead of 7.x (pubspec.yaml with melos: key)
    #[arg(long)]
    pub legacy: bool,
}

/// Initialize a new Melos workspace
pub fn run(args: InitArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;

    // Resolve workspace name
    let workspace_name = match args.name {
        Some(name) => name,
        None => prompt_with_default(
            "Workspace name",
            cwd.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("my_workspace"),
        )?,
    };

    // Resolve target directory
    let cwd_name = cwd.file_name().and_then(|s| s.to_str()).unwrap_or("");

    let default_dir = if workspace_name == cwd_name {
        ".".to_string()
    } else {
        workspace_name.clone()
    };

    let target_dir_str = match args.directory {
        Some(dir) => dir,
        None => prompt_with_default("Directory", &default_dir)?,
    };

    let target_dir = if target_dir_str == "." {
        cwd.clone()
    } else {
        cwd.join(&target_dir_str)
    };

    // Ask about apps directory
    let include_apps = prompt_yes_no("Include an 'apps' directory?", true)?;

    let mut package_patterns: Vec<String> = vec!["packages/*".to_string()];
    if include_apps {
        package_patterns.push("apps/*".to_string());
    }
    // Add user-specified patterns (expand comma-separated values)
    for pattern in &args.packages {
        for p in pattern.split(',') {
            let trimmed = p.trim().to_string();
            if !trimmed.is_empty() && !package_patterns.contains(&trimmed) {
                package_patterns.push(trimmed);
            }
        }
    }

    println!(
        "\n{} Initializing workspace '{}' in '{}'...\n",
        "$".cyan(),
        workspace_name.bold(),
        target_dir.display()
    );

    create_dir_if_missing(&target_dir)?;
    create_dir_if_missing(&target_dir.join("packages"))?;
    if include_apps {
        create_dir_if_missing(&target_dir.join("apps"))?;
    }

    // Generate config files
    if args.legacy {
        write_legacy_config(&target_dir, &workspace_name, &package_patterns)?;
    } else {
        write_7x_config(&target_dir, &workspace_name, &package_patterns)?;
    }

    println!("{}", "Created:".green().bold());
    print_tree(&target_dir, &cwd, args.legacy, include_apps);

    println!(
        "\n{} Run '{}' to bootstrap your workspace.",
        "i".blue(),
        "melos-rs bootstrap".cyan()
    );

    Ok(())
}

/// Write Melos 7.x config: pubspec.yaml with `melos:` key
fn write_7x_config(dir: &Path, name: &str, package_patterns: &[String]) -> Result<()> {
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

/// Write Melos 6.x config: separate melos.yaml + basic pubspec.yaml
fn write_legacy_config(dir: &Path, name: &str, package_patterns: &[String]) -> Result<()> {
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

/// Create a directory if it doesn't already exist
fn create_dir_if_missing(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }
    Ok(())
}

/// Print a simple directory tree showing what was created
fn print_tree(dir: &Path, cwd: &Path, legacy: bool, include_apps: bool) {
    let display_dir = pathdiff::diff_paths(dir, cwd).unwrap_or_else(|| dir.to_path_buf());

    println!("  {}/", display_dir.display().to_string().bold());
    if legacy {
        println!("  {}", "├── melos.yaml".dimmed());
    }
    println!("  {}", "├── pubspec.yaml".dimmed());
    println!("  {}", "├── packages/".dimmed());
    if include_apps {
        println!("  {}", "└── apps/".dimmed());
    }
}

/// Prompt the user for input with a default value
fn prompt_with_default(label: &str, default: &str) -> Result<String> {
    print!("{} [{}]: ", label.bold(), default.dimmed());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    let trimmed = input.trim();

    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

/// Prompt for yes/no with a default
fn prompt_yes_no(question: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    print!("{} [{}]: ", question.bold(), hint.dimmed());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();

    if trimmed.is_empty() {
        Ok(default_yes)
    } else {
        Ok(trimmed == "y" || trimmed == "yes")
    }
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

        // Call again — should be a no-op
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
