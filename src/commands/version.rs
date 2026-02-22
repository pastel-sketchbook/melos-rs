use anyhow::{bail, Result};
use clap::Args;
use colored::Colorize;
use semver::Version;

use crate::package::Package;
use crate::workspace::Workspace;

/// Arguments for the `version` command
#[derive(Args, Debug)]
pub struct VersionArgs {
    /// Version bump type (build, patch, minor, major) or an explicit version
    #[arg(default_value = "patch")]
    pub bump: String,

    /// Apply to all packages
    #[arg(long)]
    pub all: bool,

    /// Per-package version overrides (e.g., -Vanmobile:patch -Vadapter:build)
    #[arg(short = 'V', value_parser = parse_version_override)]
    pub overrides: Vec<(String, String)>,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,
}

/// Parse a version override flag like "anmobile:patch"
fn parse_version_override(s: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid version override '{}'. Expected format: package:bump",
            s
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Execute the version command
pub async fn run(workspace: &Workspace, args: VersionArgs) -> Result<()> {
    println!(
        "\n{} Managing versions across packages...\n",
        "$".cyan()
    );

    if workspace.packages.is_empty() {
        println!("{}", "No packages found in workspace.".yellow());
        return Ok(());
    }

    // Determine which packages to version and how
    let packages_to_version: Vec<(&Package, &str)> = if !args.overrides.is_empty() {
        // Use per-package overrides
        args.overrides
            .iter()
            .filter_map(|(name, bump)| {
                workspace
                    .packages
                    .iter()
                    .find(|p| p.name.contains(name))
                    .map(|p| (p, bump.as_str()))
            })
            .collect()
    } else if args.all {
        // Apply to all packages with the default bump type
        workspace
            .packages
            .iter()
            .map(|p| (p, args.bump.as_str()))
            .collect()
    } else {
        println!("{}", "Specify --all or use -V overrides to select packages.".yellow());
        return Ok(());
    };

    // Show plan
    println!("Version changes:");
    for (pkg, bump) in &packages_to_version {
        let current = pkg.version.as_deref().unwrap_or("0.0.0");
        let next = compute_next_version(current, bump)?;
        println!(
            "  {} {} -> {} ({})",
            pkg.name.bold(),
            current.dimmed(),
            next.to_string().green(),
            bump
        );
    }

    if !args.yes {
        // TODO: Add interactive confirmation prompt
        println!(
            "\n{} Use --yes to skip confirmation (interactive prompt not yet implemented)",
            "NOTE:".yellow()
        );
        return Ok(());
    }

    // Apply version changes
    for (pkg, bump) in &packages_to_version {
        apply_version_bump(pkg, bump)?;
    }

    // Run pre-commit hook if configured
    if let Some(ref cmd_config) = workspace.config.command
        && let Some(ref version_config) = cmd_config.version
        && let Some(ref hooks) = version_config.hooks
        && let Some(ref pre_commit) = hooks.pre_commit
    {
        println!(
            "\n{} Running pre-commit hook: {}",
            "$".cyan(),
            pre_commit
        );
        let status = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(pre_commit)
            .current_dir(&workspace.root_path)
            .status()
            .await?;

        if !status.success() {
            bail!("Pre-commit hook failed");
        }
    }

    Ok(())
}

/// Compute the next version given a current version string and a bump type
fn compute_next_version(current: &str, bump: &str) -> Result<Version> {
    let mut version = Version::parse(current)
        .or_else(|_| {
            // Try to handle Flutter-style versions like "1.2.3+4"
            let cleaned = current.split('+').next().unwrap_or(current);
            Version::parse(cleaned)
        })
        .unwrap_or_else(|_| Version::new(0, 0, 0));

    match bump {
        "major" => {
            version.major += 1;
            version.minor = 0;
            version.patch = 0;
        }
        "minor" => {
            version.minor += 1;
            version.patch = 0;
        }
        "patch" => {
            version.patch += 1;
        }
        "build" => {
            // For build bumps, we increment the build metadata
            // Flutter uses +N format, so we handle that
            let build_num = extract_build_number(current).unwrap_or(0) + 1;
            version.build = semver::BuildMetadata::new(&build_num.to_string())?;
        }
        explicit => {
            // Try to parse as explicit version
            version = Version::parse(explicit)
                .map_err(|_| anyhow::anyhow!("Invalid version or bump type: {}", explicit))?;
        }
    }

    Ok(version)
}

/// Extract build number from a Flutter version string like "1.2.3+42"
fn extract_build_number(version_str: &str) -> Option<u64> {
    version_str
        .split('+')
        .nth(1)
        .and_then(|b| b.parse().ok())
}

/// Apply a version bump to a package's pubspec.yaml
fn apply_version_bump(pkg: &Package, bump: &str) -> Result<()> {
    let pubspec_path = pkg.path.join("pubspec.yaml");
    let content = std::fs::read_to_string(&pubspec_path)?;

    let current_version = pkg.version.as_deref().unwrap_or("0.0.0");
    let next_version = compute_next_version(current_version, bump)?;

    // Build the full version string (preserving +buildNumber format for Flutter)
    let next_version_str = if bump == "build" {
        let build_num = extract_build_number(current_version).unwrap_or(0) + 1;
        let base = current_version.split('+').next().unwrap_or(current_version);
        format!("{}+{}", base, build_num)
    } else {
        let build_num = extract_build_number(current_version);
        match build_num {
            Some(n) => format!("{}+{}", next_version, n),
            None => next_version.to_string(),
        }
    };

    // Replace version in pubspec.yaml
    let new_content = regex::Regex::new(r"(?m)^version:\s*\S+")?
        .replace(&content, &format!("version: {}", next_version_str))
        .to_string();

    std::fs::write(&pubspec_path, new_content)?;

    println!(
        "  {} Updated {} to {}",
        "OK".green(),
        pubspec_path.display(),
        next_version_str
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_next_version_patch() {
        let v = compute_next_version("1.2.3", "patch").unwrap();
        assert_eq!(v.to_string(), "1.2.4");
    }

    #[test]
    fn test_compute_next_version_minor() {
        let v = compute_next_version("1.2.3", "minor").unwrap();
        assert_eq!(v.to_string(), "1.3.0");
    }

    #[test]
    fn test_compute_next_version_major() {
        let v = compute_next_version("1.2.3", "major").unwrap();
        assert_eq!(v.to_string(), "2.0.0");
    }

    #[test]
    fn test_extract_build_number() {
        assert_eq!(extract_build_number("1.2.3+42"), Some(42));
        assert_eq!(extract_build_number("1.2.3"), None);
    }
}
