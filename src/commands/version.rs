use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::Args;
use colored::Colorize;
use semver::{Prerelease, Version};

use crate::config::RepositoryConfig;
use crate::package::Package;
use crate::workspace::Workspace;

/// Arguments for the `version` command
#[derive(Args, Debug)]
pub struct VersionArgs {
    /// Version bump type (build, patch, minor, major) or an explicit version
    #[arg(default_value = "patch")]
    pub bump: String,

    /// Apply to all packages
    #[arg(long, short = 'a')]
    pub all: bool,

    /// Per-package version overrides (e.g., -Vanmobile:patch -Vadapter:build)
    #[arg(short = 'V', value_parser = parse_version_override)]
    pub overrides: Vec<(String, String)>,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Use conventional commits to determine version bumps
    #[arg(long)]
    pub conventional_commits: bool,

    /// Git ref to find conventional commits since (used with --conventional-commits)
    #[arg(long, default_value = "HEAD~10")]
    pub since_ref: String,

    /// Skip changelog generation
    #[arg(long)]
    pub no_changelog: bool,

    /// Skip git tag creation
    #[arg(long)]
    pub no_git_tag: bool,

    /// Skip pushing commits and tags to remote
    #[arg(long)]
    pub no_git_push: bool,

    /// Coordinated versioning: bump all packages to the same version
    #[arg(long)]
    pub coordinated: bool,

    /// Version as prerelease (e.g., 1.0.0-dev.0). Cannot combine with --graduate.
    #[arg(long, short = 'p', conflicts_with = "graduate")]
    pub prerelease: bool,

    /// Graduate prerelease packages to stable (e.g., 1.0.0-dev.3 -> 1.0.0).
    /// Cannot combine with --prerelease.
    #[arg(long, short = 'g', conflicts_with = "prerelease")]
    pub graduate: bool,

    /// Prerelease identifier (e.g., beta -> 1.0.0-beta.0). Used with --prerelease.
    #[arg(long, default_value = "dev")]
    pub preid: String,

    /// Prerelease identifier for dependents only (falls back to --preid if not set)
    #[arg(long)]
    pub dependent_preid: Option<String>,

    /// Override the release commit message. Use {new_package_versions} as placeholder.
    #[arg(long, short = 'm')]
    pub message: Option<String>,

    /// Update dependency constraints in dependent packages (default: true)
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub dependent_constraints: bool,

    /// Patch-bump dependents when their constraints change (default: true).
    /// Only effective with --dependent-constraints and --conventional-commits.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub dependent_versions: bool,
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

// ---------------------------------------------------------------------------
// Conventional commit types
// ---------------------------------------------------------------------------

/// A parsed conventional commit
#[derive(Debug, Clone)]
pub struct ConventionalCommit {
    /// Commit type: feat, fix, chore, docs, refactor, test, etc.
    pub commit_type: String,
    /// Optional scope: feat(auth): ...
    pub scope: Option<String>,
    /// Whether this is a breaking change (trailing `!` or `BREAKING CHANGE:` footer)
    pub breaking: bool,
    /// The commit description (summary line after the colon)
    pub description: String,
    /// Full commit body (if any)
    pub body: Option<String>,
    /// Short commit hash
    pub hash: String,
}

impl ConventionalCommit {
    /// Determine the bump type this commit implies
    pub fn bump_type(&self) -> BumpType {
        if self.breaking {
            BumpType::Major
        } else if self.commit_type == "feat" {
            BumpType::Minor
        } else if self.commit_type == "fix" {
            BumpType::Patch
        } else {
            BumpType::None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BumpType {
    None,
    Patch,
    Minor,
    Major,
}

impl fmt::Display for BumpType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BumpType::None => write!(f, "none"),
            BumpType::Patch => write!(f, "patch"),
            BumpType::Minor => write!(f, "minor"),
            BumpType::Major => write!(f, "major"),
        }
    }
}

/// Parse a single commit message into a ConventionalCommit, if it matches the format.
///
/// Format: `type(scope)!: description`
/// - `type` is required (e.g., feat, fix, chore)
/// - `(scope)` is optional
/// - `!` indicates a breaking change
/// - `: description` is required
pub fn parse_conventional_commit(hash: &str, message: &str) -> Option<ConventionalCommit> {
    let re = regex::Regex::new(
        r"^(?P<type>[a-z]+)(?:\((?P<scope>[^)]+)\))?(?P<breaking>!)?:\s*(?P<desc>.+)"
    ).ok()?;

    let first_line = message.lines().next()?;
    let caps = re.captures(first_line)?;

    let commit_type = caps.name("type")?.as_str().to_string();
    let scope = caps.name("scope").map(|m| m.as_str().to_string());
    let breaking_mark = caps.name("breaking").is_some();
    let description = caps.name("desc")?.as_str().trim().to_string();

    // Check for BREAKING CHANGE footer in body
    let body_lines: Vec<&str> = message.lines().skip(1).collect();
    let body = if body_lines.is_empty() {
        None
    } else {
        Some(body_lines.join("\n").trim().to_string()).filter(|s| !s.is_empty())
    };
    let breaking_footer = body
        .as_ref()
        .is_some_and(|b| b.contains("BREAKING CHANGE:") || b.contains("BREAKING-CHANGE:"));

    Some(ConventionalCommit {
        commit_type,
        scope,
        breaking: breaking_mark || breaking_footer,
        description,
        body,
        hash: hash.to_string(),
    })
}

/// Retrieve git log commits since a ref and parse them as conventional commits.
/// Returns commits that successfully parse as conventional commits.
pub fn parse_commits_since(root: &Path, since_ref: &str) -> Result<Vec<ConventionalCommit>> {
    let output = std::process::Command::new("git")
        .args(["log", &format!("{}..HEAD", since_ref), "--format=%h%n%B%n---END---"])
        .current_dir(root)
        .output()
        .context("Failed to run git log")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git log failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();
    let mut current_hash = String::new();
    let mut current_message = Vec::new();

    for line in stdout.lines() {
        if line == "---END---" {
            if !current_hash.is_empty() {
                let message = current_message.join("\n");
                if let Some(commit) = parse_conventional_commit(&current_hash, message.trim()) {
                    commits.push(commit);
                }
            }
            current_hash.clear();
            current_message.clear();
        } else if current_hash.is_empty() {
            current_hash = line.to_string();
        } else {
            current_message.push(line.to_string());
        }
    }

    Ok(commits)
}

/// Map commits to packages based on changed files in each commit.
/// Returns a map of package name -> Vec<ConventionalCommit>.
pub fn map_commits_to_packages(
    root: &Path,
    commits: &[ConventionalCommit],
    packages: &[Package],
) -> Result<HashMap<String, Vec<ConventionalCommit>>> {
    let mut package_commits: HashMap<String, Vec<ConventionalCommit>> = HashMap::new();

    for commit in commits {
        let output = std::process::Command::new("git")
            .args(["diff-tree", "--no-commit-id", "-r", "--name-only", &commit.hash])
            .current_dir(root)
            .output()
            .context("Failed to run git diff-tree")?;

        let changed_files: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect();

        // Check which packages are affected by the changed files
        for pkg in packages {
            let pkg_relative = pkg
                .path
                .strip_prefix(root)
                .unwrap_or(&pkg.path);
            let pkg_prefix = pkg_relative.to_string_lossy();

            let affects_package = changed_files
                .iter()
                .any(|f| f.starts_with(pkg_prefix.as_ref()));

            if affects_package {
                package_commits
                    .entry(pkg.name.clone())
                    .or_default()
                    .push(commit.clone());
            }
        }
    }

    Ok(package_commits)
}

/// Determine the highest bump type from a list of commits
pub fn highest_bump(commits: &[ConventionalCommit]) -> BumpType {
    commits
        .iter()
        .map(|c| c.bump_type())
        .max()
        .unwrap_or(BumpType::None)
}

// ---------------------------------------------------------------------------
// CHANGELOG generation
// ---------------------------------------------------------------------------

/// Generate a CHANGELOG.md entry for a package version
pub fn generate_changelog_entry(
    version: &str,
    commits: &[ConventionalCommit],
    include_body: bool,
    include_hash: bool,
    include_scopes: bool,
    repository: Option<&RepositoryConfig>,
) -> String {
    let mut sections: HashMap<&str, Vec<String>> = HashMap::new();

    // Group commits by type -> human-readable section
    for commit in commits {
        let section = match commit.commit_type.as_str() {
            "feat" => "Features",
            "fix" => "Bug Fixes",
            "docs" => "Documentation",
            "refactor" => "Code Refactoring",
            "test" => "Tests",
            "chore" => "Chores",
            "perf" => "Performance Improvements",
            "ci" => "CI",
            "build" => "Build",
            "style" => "Style",
            _ => "Other Changes",
        };

        let scope_prefix = if include_scopes {
            commit
                .scope
                .as_ref()
                .map(|s| format!("**{}**: ", s))
                .unwrap_or_default()
        } else {
            String::new()
        };

        let hash_suffix = if include_hash {
            if let Some(repo) = repository {
                // Link the commit hash to the repository commit URL
                let url = repo.commit_url(&commit.hash);
                format!(" ([{}]({}))", commit.hash, url)
            } else {
                format!(" ({})", commit.hash)
            }
        } else {
            String::new()
        };

        let mut entry = format!("- {}{}{}", scope_prefix, commit.description, hash_suffix);

        if include_body
            && let Some(ref body) = commit.body
        {
            entry.push_str(&format!("\n  {}", body.replace('\n', "\n  ")));
        }

        if commit.breaking {
            entry.push_str("\n  **BREAKING CHANGE**");
        }

        sections.entry(section).or_default().push(entry);
    }

    let date = chrono_date_today();
    let mut output = format!("## {} ({})\n", version, date);

    // Emit sections in a stable order
    let section_order = [
        "Features",
        "Bug Fixes",
        "Performance Improvements",
        "Code Refactoring",
        "Documentation",
        "Tests",
        "CI",
        "Build",
        "Style",
        "Chores",
        "Other Changes",
    ];

    for &section_name in &section_order {
        if let Some(entries) = sections.get(section_name) {
            output.push_str(&format!("\n### {}\n\n", section_name));
            for entry in entries {
                output.push_str(&format!("{}\n", entry));
            }
        }
    }

    output
}

/// Get today's date as YYYY-MM-DD using Rust's SystemTime (no external process)
fn chrono_date_today() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = duration.as_secs();

    // Simple date calculation from Unix timestamp
    // Days since epoch
    let days = (total_secs / 86400) as i64;

    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Write or prepend a CHANGELOG entry to a package's CHANGELOG.md
pub fn write_changelog(pkg_path: &Path, entry: &str) -> Result<()> {
    let changelog_path = pkg_path.join("CHANGELOG.md");

    let existing = if changelog_path.exists() {
        std::fs::read_to_string(&changelog_path)
            .with_context(|| format!("Failed to read {}", changelog_path.display()))?
    } else {
        String::new()
    };

    // If there's an existing file with a top-level heading, insert after it
    let new_content = if existing.starts_with("# ") {
        // Find end of first line
        let first_newline = existing.find('\n').unwrap_or(existing.len());
        let header = &existing[..first_newline];
        let rest = &existing[first_newline..];
        format!("{}\n\n{}{}", header, entry, rest)
    } else if existing.is_empty() {
        format!("# Changelog\n\n{}", entry)
    } else {
        format!("{}\n{}", entry, existing)
    };

    std::fs::write(&changelog_path, new_content)
        .with_context(|| format!("Failed to write {}", changelog_path.display()))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Git operations
// ---------------------------------------------------------------------------

/// Validate that we are on the expected branch (from config)
pub fn validate_branch(root: &Path, expected_branch: &str) -> Result<()> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(root)
        .output()
        .context("Failed to get current git branch")?;

    let current = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if current != expected_branch {
        bail!(
            "Expected to be on branch '{}', but currently on '{}'. \
             Version bumps are restricted to the configured branch.",
            expected_branch,
            current
        );
    }

    Ok(())
}

/// Create an annotated git tag for a package version
pub fn create_git_tag(root: &Path, pkg_name: &str, version: &str) -> Result<()> {
    let tag_name = format!("{}-v{}", pkg_name, version);
    let message = format!("{} v{}", pkg_name, version);

    let status = std::process::Command::new("git")
        .args(["tag", "-a", &tag_name, "-m", &message])
        .current_dir(root)
        .status()
        .context("Failed to create git tag")?;

    if !status.success() {
        bail!("Failed to create git tag '{}'", tag_name);
    }

    println!("  {} Created tag {}", "TAG".blue(), tag_name.bold());
    Ok(())
}

/// Stage all changes and commit with the given message
pub fn git_commit(root: &Path, message: &str) -> Result<()> {
    let add_status = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .status()
        .context("Failed to stage changes")?;

    if !add_status.success() {
        bail!("git add failed");
    }

    let commit_status = std::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(root)
        .status()
        .context("Failed to create commit")?;

    if !commit_status.success() {
        bail!("git commit failed");
    }

    Ok(())
}

/// Push commits and tags to the remote repository
pub fn git_push(root: &Path, include_tags: bool) -> Result<()> {
    let push_status = std::process::Command::new("git")
        .args(["push"])
        .current_dir(root)
        .status()
        .context("Failed to push commits")?;

    if !push_status.success() {
        bail!("git push failed");
    }

    if include_tags {
        let tag_status = std::process::Command::new("git")
            .args(["push", "--tags"])
            .current_dir(root)
            .status()
            .context("Failed to push tags")?;

        if !tag_status.success() {
            bail!("git push --tags failed");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Version computation
// ---------------------------------------------------------------------------

/// Compute the next version given a current version string and a bump type
pub fn compute_next_version(current: &str, bump: &str) -> Result<Version> {
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
        "none" => {
            // No bump needed
        }
        explicit => {
            // Try to parse as explicit version
            version = Version::parse(explicit)
                .map_err(|_| anyhow::anyhow!("Invalid version or bump type: {}", explicit))?;
        }
    }

    Ok(version)
}

/// Compute the next prerelease version.
///
/// If the current version is already a prerelease with the same base bump and
/// preid, increment the prerelease counter. Otherwise, bump to the next base
/// version and start at `<preid>.0`.
///
/// Examples (bump = "minor", preid = "dev"):
///   - "1.0.0"         -> "1.1.0-dev.0"
///   - "1.1.0-dev.0"   -> "1.1.0-dev.1"
///   - "1.1.0-dev.5"   -> "1.1.0-dev.6"
///   - "1.1.0-beta.0"  -> "1.1.0-dev.0"  (different preid, reset)
///   - "2.0.0-dev.0" with bump "major" -> "2.0.0-dev.1" (already at major prerelease)
pub fn compute_next_prerelease(current: &str, bump: &str, preid: &str) -> Result<Version> {
    let current_ver = Version::parse(current)
        .or_else(|_| {
            let cleaned = current.split('+').next().unwrap_or(current);
            Version::parse(cleaned)
        })
        .unwrap_or_else(|_| Version::new(0, 0, 0));

    let current_base = Version::new(current_ver.major, current_ver.minor, current_ver.patch);
    let pre_str = current_ver.pre.as_str();

    // If current is already a prerelease with the same preid, just increment the counter
    if !pre_str.is_empty() {
        let prefix = format!("{}.", preid);
        if let Some(counter_str) = pre_str.strip_prefix(&prefix)
            && let Ok(counter) = counter_str.parse::<u64>()
        {
            // Same preid — increment counter, keep the same base
            let new_pre = format!("{}.{}", preid, counter + 1);
            let mut result = current_base;
            result.pre = Prerelease::new(&new_pre)
                .map_err(|e| anyhow::anyhow!("Invalid prerelease: {}", e))?;
            return Ok(result);
        }

        // Different preid — reset counter to 0 but keep the same base
        let new_pre = format!("{}.0", preid);
        let mut result = current_base;
        result.pre = Prerelease::new(&new_pre)
            .map_err(|e| anyhow::anyhow!("Invalid prerelease: {}", e))?;
        return Ok(result);
    }

    // Current is stable — bump the base, then add prerelease suffix
    let base = compute_next_version(
        &format!("{}.{}.{}", current_ver.major, current_ver.minor, current_ver.patch),
        bump,
    )?;
    let new_pre = format!("{}.0", preid);
    let mut result = base;
    result.pre = Prerelease::new(&new_pre)
        .map_err(|e| anyhow::anyhow!("Invalid prerelease: {}", e))?;
    Ok(result)
}

/// Graduate a prerelease version to stable by stripping the prerelease suffix.
///
/// Examples:
///   - "1.1.0-dev.3"  -> "1.1.0"
///   - "2.0.0-beta.1" -> "2.0.0"
///   - "1.0.0"        -> "1.0.0" (already stable, no change)
pub fn graduate_version(current: &str) -> Result<Version> {
    let ver = Version::parse(current)
        .or_else(|_| {
            let cleaned = current.split('+').next().unwrap_or(current);
            Version::parse(cleaned)
        })
        .unwrap_or_else(|_| Version::new(0, 0, 0));

    Ok(Version::new(ver.major, ver.minor, ver.patch))
}

/// Check whether a version string is a prerelease
pub fn is_prerelease(version_str: &str) -> bool {
    Version::parse(version_str)
        .or_else(|_| {
            let cleaned = version_str.split('+').next().unwrap_or(version_str);
            Version::parse(cleaned)
        })
        .map(|v| !v.pre.is_empty())
        .unwrap_or(false)
}

/// Extract build number from a Flutter version string like "1.2.3+42"
fn extract_build_number(version_str: &str) -> Option<u64> {
    version_str
        .split('+')
        .nth(1)
        .and_then(|b| b.parse().ok())
}

/// Apply a version bump to a package's pubspec.yaml
fn apply_version_bump(pkg: &Package, bump: &str) -> Result<String> {
    let pubspec_path = pkg.path.join("pubspec.yaml");
    let content = std::fs::read_to_string(&pubspec_path)
        .with_context(|| format!("Failed to read {}", pubspec_path.display()))?;

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

    std::fs::write(&pubspec_path, new_content)
        .with_context(|| format!("Failed to write {}", pubspec_path.display()))?;

    println!(
        "  {} Updated {} to {}",
        "OK".green(),
        pubspec_path.display(),
        next_version_str
    );

    Ok(next_version_str)
}

/// Update a dependent package's pubspec.yaml to use the new version constraint
/// for a bumped dependency. Returns true if any changes were made.
fn update_dependency_constraint(
    dependent_pkg: &Package,
    dep_name: &str,
    new_version: &str,
) -> Result<bool> {
    let pubspec_path = dependent_pkg.path.join("pubspec.yaml");
    let content = std::fs::read_to_string(&pubspec_path)
        .with_context(|| format!("Failed to read {}", pubspec_path.display()))?;

    // Parse the new version to build a caret constraint like ^1.2.0
    let ver = Version::parse(new_version)
        .or_else(|_| {
            let cleaned = new_version.split('+').next().unwrap_or(new_version);
            Version::parse(cleaned)
        })
        .unwrap_or_else(|_| Version::new(0, 0, 0));

    let constraint = format!("^{}", ver);

    // Match patterns like:
    //   dep_name: ^1.0.0
    //   dep_name: "^1.0.0"
    //   dep_name: '>=1.0.0 <2.0.0'
    //   dep_name: any
    // But NOT dep_name with a map value (path/git/sdk dependency)
    let pattern = format!(
        r#"(?m)^(\s+{dep}:\s*)(?:["']?)[<>=^~\d][^"\n]*(?:["']?)"#,
        dep = regex::escape(dep_name)
    );
    let re = regex::Regex::new(&pattern)?;

    if !re.is_match(&content) {
        return Ok(false);
    }

    let new_content = re
        .replace(&content, format!("${{1}}{}", constraint))
        .to_string();

    if new_content == content {
        return Ok(false);
    }

    std::fs::write(&pubspec_path, new_content)
        .with_context(|| format!("Failed to write {}", pubspec_path.display()))?;

    println!(
        "  {} Updated {} dependency on {} to {}",
        "OK".green(),
        dependent_pkg.name.bold(),
        dep_name,
        constraint
    );

    Ok(true)
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

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

    // Get version command config (if any)
    let version_config = workspace
        .config
        .command
        .as_ref()
        .and_then(|c| c.version.as_ref());

    // Branch validation
    if let Some(cfg) = version_config
        && let Some(ref branch) = cfg.branch
    {
        validate_branch(&workspace.root_path, branch)?;
        println!("  {} Branch validation passed ({})", "OK".green(), branch);
    }

    // Determine changelog/tag settings from config + CLI flags
    let should_changelog = if args.no_changelog {
        false
    } else {
        version_config.is_none_or(|c| c.should_changelog())
    };
    let should_tag = if args.no_git_tag {
        false
    } else {
        version_config.is_none_or(|c| c.should_tag())
    };
    let include_body = version_config
        .and_then(|c| c.changelog_config.as_ref())
        .and_then(|cc| cc.include_commit_body)
        .unwrap_or(false);
    let include_hash = version_config
        .and_then(|c| c.changelog_config.as_ref())
        .and_then(|cc| cc.include_commit_id)
        // link_to_commits is an alias/override for including commit IDs
        .or_else(|| version_config.and_then(|c| c.link_to_commits))
        .unwrap_or(false);
    let include_scopes = version_config
        .and_then(|c| c.include_scopes)
        .unwrap_or(true); // Melos includes scopes by default

    // Collect conventional commits if requested
    let conventional_commits = if args.conventional_commits {
        let commits = parse_commits_since(&workspace.root_path, &args.since_ref)?;
        println!(
            "  Found {} conventional commit(s) since {}",
            commits.len().to_string().bold(),
            args.since_ref
        );
        let mapped = map_commits_to_packages(&workspace.root_path, &commits, &workspace.packages)?;
        Some(mapped)
    } else {
        None
    };

    // Determine whether coordinated versioning is enabled (CLI flag or config)
    let is_coordinated = args.coordinated
        || version_config
            .map(|c| c.is_coordinated())
            .unwrap_or(false);

    // Determine which packages to version and how.
    //
    // The result is a Vec of (package, target_version_string) where the target
    // is either a bump type ("patch", "minor") or an explicit version ("1.2.0-dev.0").
    let packages_to_version: Vec<(&Package, String)> = if args.graduate {
        // Graduate mode: strip prerelease suffix from all prerelease packages
        let graduated: Vec<_> = workspace
            .packages
            .iter()
            .filter(|p| {
                let v = p.version.as_deref().unwrap_or("0.0.0");
                is_prerelease(v)
            })
            .map(|p| {
                let current = p.version.as_deref().unwrap_or("0.0.0");
                let stable = graduate_version(current)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| current.to_string());
                (p, stable)
            })
            .collect();

        if graduated.is_empty() {
            println!("{}", "No prerelease packages to graduate.".yellow());
            return Ok(());
        }

        println!(
            "  {} Graduating {} prerelease package(s) to stable",
            "INFO".blue(),
            graduated.len()
        );
        graduated
    } else if is_coordinated {
        // Coordinated versioning: bump ALL packages to the same version.
        let highest_current = workspace
            .packages
            .iter()
            .filter_map(|p| {
                let v_str = p.version.as_deref().unwrap_or("0.0.0");
                Version::parse(v_str)
                    .or_else(|_| {
                        let cleaned = v_str.split('+').next().unwrap_or(v_str);
                        Version::parse(cleaned)
                    })
                    .ok()
            })
            .max()
            .unwrap_or_else(|| Version::new(0, 0, 0));

        let base_str = format!("{}.{}.{}", highest_current.major, highest_current.minor, highest_current.patch);
        let coordinated_version = if args.prerelease {
            compute_next_prerelease(&base_str, &args.bump, &args.preid)?
        } else {
            compute_next_version(&base_str, &args.bump)?
        };
        let explicit = coordinated_version.to_string();

        println!(
            "  {} Coordinated versioning: all packages -> {}",
            "INFO".blue(),
            explicit.green()
        );

        workspace
            .packages
            .iter()
            .map(|p| (p, explicit.clone()))
            .collect()
    } else if !args.overrides.is_empty() {
        // Per-package overrides (prerelease modifier applied if --prerelease)
        args.overrides
            .iter()
            .filter_map(|(name, bump)| {
                workspace
                    .packages
                    .iter()
                    .find(|p| p.name.contains(name))
                    .map(|p| {
                        if args.prerelease {
                            let current = p.version.as_deref().unwrap_or("0.0.0");
                            let v = compute_next_prerelease(current, bump, &args.preid)
                                .map(|v| v.to_string())
                                .unwrap_or_else(|_| bump.clone());
                            (p, v)
                        } else {
                            (p, bump.clone())
                        }
                    })
            })
            .collect()
    } else if args.conventional_commits {
        // Use conventional commits to determine bumps
        let mapped = conventional_commits.as_ref().expect("commits should be loaded");
        workspace
            .packages
            .iter()
            .filter_map(|p| {
                let commits = mapped.get(&p.name)?;
                let bump = highest_bump(commits);
                if bump == BumpType::None {
                    None
                } else if args.prerelease {
                    let current = p.version.as_deref().unwrap_or("0.0.0");
                    let v = compute_next_prerelease(current, &bump.to_string(), &args.preid)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|_| bump.to_string());
                    Some((p, v))
                } else {
                    Some((p, bump.to_string()))
                }
            })
            .collect()
    } else if args.all {
        // Apply to all packages with the default bump type
        if args.prerelease {
            workspace
                .packages
                .iter()
                .map(|p| {
                    let current = p.version.as_deref().unwrap_or("0.0.0");
                    let v = compute_next_prerelease(current, &args.bump, &args.preid)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|_| args.bump.clone());
                    (p, v)
                })
                .collect()
        } else {
            workspace
                .packages
                .iter()
                .map(|p| (p, args.bump.clone()))
                .collect()
        }
    } else {
        println!(
            "{}",
            "Specify --all, --conventional-commits, --graduate, or use -V overrides to select packages."
                .yellow()
        );
        return Ok(());
    };

    if packages_to_version.is_empty() {
        println!("{}", "No packages need version bumps.".yellow());
        return Ok(());
    }

    // Show plan
    println!("\nVersion changes:");
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
        print!(
            "\n{} Apply these version changes? [y/N] ",
            "CONFIRM:".yellow()
        );
        std::io::Write::flush(&mut std::io::stdout())?;

        let mut input = String::new();
        std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut input)?;
        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // Apply version changes and collect new versions for tagging
    let mut versioned: Vec<(String, String)> = Vec::new(); // (pkg_name, new_version)
    for (pkg, bump) in &packages_to_version {
        let new_version = apply_version_bump(pkg, bump)?;
        versioned.push((pkg.name.clone(), new_version));
    }

    // Update dependent package constraints (--dependent-constraints, default: on)
    if args.dependent_constraints && !versioned.is_empty() {
        let versioned_names: HashMap<&str, &str> = versioned
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();

        // Find packages that depend on any bumped package but were not themselves bumped
        let mut dependents_to_bump: Vec<(&Package, String)> = Vec::new();

        for pkg in &workspace.packages {
            if versioned_names.contains_key(pkg.name.as_str()) {
                continue; // Already bumped
            }

            let mut was_updated = false;
            for dep_name in pkg.dependencies.iter().chain(pkg.dev_dependencies.iter()) {
                if let Some(&new_ver) = versioned_names.get(dep_name.as_str()) {
                    let updated = update_dependency_constraint(pkg, dep_name, new_ver)?;
                    if updated {
                        was_updated = true;
                    }
                }
            }

            if was_updated && args.dependent_versions {
                // Determine the version for the dependent
                let dependent_ver = if args.prerelease {
                    let preid = args.dependent_preid.as_deref().unwrap_or(&args.preid);
                    let current = pkg.version.as_deref().unwrap_or("0.0.0");
                    compute_next_prerelease(current, "patch", preid)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|_| "patch".to_string())
                } else {
                    "patch".to_string()
                };
                dependents_to_bump.push((pkg, dependent_ver));
            }
        }

        // Apply patch bumps to dependents
        if !dependents_to_bump.is_empty() {
            println!(
                "\n{} Bumping {} dependent package(s)...",
                "$".cyan(),
                dependents_to_bump.len()
            );
            for (pkg, bump) in &dependents_to_bump {
                let new_version = apply_version_bump(pkg, bump)?;
                versioned.push((pkg.name.clone(), new_version));
            }
        }
    }

    // Generate changelogs
    if should_changelog {
        if let Some(ref mapped) = conventional_commits {
            let repo = workspace.config.repository.as_ref();
            println!("\n{} Generating changelogs...", "$".cyan());
            for (pkg, _bump) in &packages_to_version {
                if let Some(commits) = mapped.get(&pkg.name)
                    && !commits.is_empty()
                {
                    let new_ver = versioned
                        .iter()
                        .find(|(n, _)| n == &pkg.name)
                        .map(|(_, v)| v.as_str())
                        .unwrap_or("unknown");
                    let entry =
                        generate_changelog_entry(new_ver, commits, include_body, include_hash, include_scopes, repo);
                    write_changelog(&pkg.path, &entry)?;
                    println!(
                        "  {} Updated CHANGELOG.md for {}",
                        "OK".green(),
                        pkg.name.bold()
                    );
                }
            }

            // Workspace-level changelog
            let should_workspace = version_config
                .map(|c| c.should_workspace_changelog())
                .unwrap_or(true);
            if should_workspace {
                let all_commits: Vec<ConventionalCommit> =
                    mapped.values().flatten().cloned().collect();
                if !all_commits.is_empty() {
                    let summary_version = versioned
                        .first()
                        .map(|(_, v)| v.as_str())
                        .unwrap_or("0.0.0");
                    let entry = generate_changelog_entry(
                        summary_version,
                        &all_commits,
                        include_body,
                        include_hash,
                        include_scopes,
                        repo,
                    );
                    write_changelog(&workspace.root_path, &entry)?;
                    println!(
                        "  {} Updated workspace CHANGELOG.md",
                        "OK".green()
                    );
                }
            }
        } else {
            println!(
                "\n{} Changelog generation requires --conventional-commits; skipping.",
                "NOTE:".yellow()
            );
        }
    }

    // Run pre-commit hook if configured
    if let Some(cfg) = version_config
        && let Some(ref hooks) = cfg.hooks
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

    // Git commit
    let new_package_versions = versioned
        .iter()
        .map(|(name, ver)| format!(" - {} @ {}", name, ver))
        .collect::<Vec<_>>()
        .join("\n");

    let commit_message = if let Some(ref msg) = args.message {
        // CLI --message overrides everything
        msg.replace("{new_package_versions}", &new_package_versions)
    } else {
        let template = version_config
            .map(|c| c.message_template().to_string())
            .unwrap_or_else(|| "chore(release): publish packages\n\n{new_package_versions}".to_string());
        template.replace("{new_package_versions}", &new_package_versions)
    };
    println!("\n{} Committing: {}", "$".cyan(), commit_message.lines().next().unwrap_or(&commit_message).dimmed());
    git_commit(&workspace.root_path, &commit_message)?;
    println!("  {} Committed version changes", "OK".green());

    // Run post-commit hook if configured
    if let Some(cfg) = version_config
        && let Some(ref hooks) = cfg.hooks
        && let Some(ref post_commit) = hooks.post_commit
    {
        println!(
            "\n{} Running post-commit hook: {}",
            "$".cyan(),
            post_commit
        );
        let status = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(post_commit)
            .current_dir(&workspace.root_path)
            .status()
            .await?;

        if !status.success() {
            bail!("Post-commit hook failed");
        }
    }

    // Create git tags
    if should_tag {
        println!("\n{} Creating git tags...", "$".cyan());
        for (pkg_name, version) in &versioned {
            create_git_tag(&workspace.root_path, pkg_name, version)?;
        }
    }

    // Push commits and tags to remote
    let should_push = if args.no_git_push {
        false
    } else {
        version_config.is_none_or(|c| c.should_git_push())
    };
    if should_push {
        println!("\n{} Pushing to remote...", "$".cyan());
        git_push(&workspace.root_path, should_tag)?;
        println!("  {} Pushed commits{}", "OK".green(),
            if should_tag { " and tags" } else { "" });
    }

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
    fn test_compute_next_version_none() {
        let v = compute_next_version("1.2.3", "none").unwrap();
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn test_extract_build_number() {
        assert_eq!(extract_build_number("1.2.3+42"), Some(42));
        assert_eq!(extract_build_number("1.2.3"), None);
    }

    #[test]
    fn test_parse_conventional_commit_feat() {
        let commit = parse_conventional_commit("abc1234", "feat: add new login flow").unwrap();
        assert_eq!(commit.commit_type, "feat");
        assert!(commit.scope.is_none());
        assert!(!commit.breaking);
        assert_eq!(commit.description, "add new login flow");
        assert_eq!(commit.bump_type(), BumpType::Minor);
    }

    #[test]
    fn test_parse_conventional_commit_fix_with_scope() {
        let commit =
            parse_conventional_commit("def5678", "fix(auth): handle token expiry").unwrap();
        assert_eq!(commit.commit_type, "fix");
        assert_eq!(commit.scope.as_deref(), Some("auth"));
        assert!(!commit.breaking);
        assert_eq!(commit.description, "handle token expiry");
        assert_eq!(commit.bump_type(), BumpType::Patch);
    }

    #[test]
    fn test_parse_conventional_commit_breaking_bang() {
        let commit =
            parse_conventional_commit("ghi9012", "feat(api)!: remove deprecated endpoint")
                .unwrap();
        assert_eq!(commit.commit_type, "feat");
        assert!(commit.breaking);
        assert_eq!(commit.bump_type(), BumpType::Major);
    }

    #[test]
    fn test_parse_conventional_commit_breaking_footer() {
        let msg = "feat: new API\n\nBREAKING CHANGE: old API removed";
        let commit = parse_conventional_commit("jkl3456", msg).unwrap();
        assert!(commit.breaking);
        assert_eq!(commit.bump_type(), BumpType::Major);
    }

    #[test]
    fn test_parse_conventional_commit_chore() {
        let commit = parse_conventional_commit("mno7890", "chore: update deps").unwrap();
        assert_eq!(commit.commit_type, "chore");
        assert_eq!(commit.bump_type(), BumpType::None);
    }

    #[test]
    fn test_parse_non_conventional_returns_none() {
        assert!(parse_conventional_commit("xyz", "just a normal commit").is_none());
        assert!(parse_conventional_commit("xyz", "Update README").is_none());
    }

    #[test]
    fn test_highest_bump() {
        let commits = vec![
            parse_conventional_commit("a", "fix: bug").unwrap(),
            parse_conventional_commit("b", "feat: feature").unwrap(),
            parse_conventional_commit("c", "chore: cleanup").unwrap(),
        ];
        assert_eq!(highest_bump(&commits), BumpType::Minor);
    }

    #[test]
    fn test_highest_bump_with_breaking() {
        let commits = vec![
            parse_conventional_commit("a", "fix: bug").unwrap(),
            parse_conventional_commit("b", "feat!: breaking feature").unwrap(),
        ];
        assert_eq!(highest_bump(&commits), BumpType::Major);
    }

    #[test]
    fn test_highest_bump_empty() {
        assert_eq!(highest_bump(&[]), BumpType::None);
    }

    #[test]
    fn test_generate_changelog_entry() {
        let commits = vec![
            parse_conventional_commit("abc1234", "feat: add login").unwrap(),
            parse_conventional_commit("def5678", "fix: handle null").unwrap(),
            parse_conventional_commit("ghi9012", "chore: update deps").unwrap(),
        ];
        let entry = generate_changelog_entry("1.2.0", &commits, false, false, true, None);
        assert!(entry.contains("## 1.2.0"));
        assert!(entry.contains("### Features"));
        assert!(entry.contains("- add login"));
        assert!(entry.contains("### Bug Fixes"));
        assert!(entry.contains("- handle null"));
        assert!(entry.contains("### Chores"));
        assert!(entry.contains("- update deps"));
    }

    #[test]
    fn test_generate_changelog_with_hash() {
        let commits = vec![
            parse_conventional_commit("abc1234", "feat(ui): new button").unwrap(),
        ];
        let entry = generate_changelog_entry("2.0.0", &commits, false, true, true, None);
        assert!(entry.contains("(abc1234)"));
        assert!(entry.contains("**ui**: new button"));
    }

    #[test]
    fn test_bump_type_display() {
        assert_eq!(BumpType::None.to_string(), "none");
        assert_eq!(BumpType::Patch.to_string(), "patch");
        assert_eq!(BumpType::Minor.to_string(), "minor");
        assert_eq!(BumpType::Major.to_string(), "major");
    }

    #[test]
    fn test_chrono_date_today_format() {
        let date = chrono_date_today();
        // Should match YYYY-MM-DD format
        let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
        assert!(
            re.is_match(&date),
            "Date '{}' doesn't match YYYY-MM-DD format",
            date
        );

        // Year should be reasonable (2020-2099)
        let year: u32 = date[..4].parse().unwrap();
        assert!(year >= 2020 && year <= 2099, "Year {} out of range", year);

        // Month should be 01-12
        let month: u32 = date[5..7].parse().unwrap();
        assert!(month >= 1 && month <= 12, "Month {} out of range", month);

        // Day should be 01-31
        let day: u32 = date[8..10].parse().unwrap();
        assert!(day >= 1 && day <= 31, "Day {} out of range", day);
    }

    #[test]
    fn test_generate_changelog_without_scopes() {
        let commits = vec![
            parse_conventional_commit("abc1234", "feat(ui): new button").unwrap(),
            parse_conventional_commit("def5678", "fix(auth): handle token").unwrap(),
        ];
        let entry = generate_changelog_entry("1.0.0", &commits, false, false, false, None);
        // With include_scopes=false, scope prefix should NOT appear
        assert!(!entry.contains("**ui**"), "Scope should not be included");
        assert!(
            !entry.contains("**auth**"),
            "Scope should not be included"
        );
        // But the descriptions should still be present
        assert!(entry.contains("new button"));
        assert!(entry.contains("handle token"));
    }

    #[test]
    fn test_generate_changelog_with_scopes() {
        let commits = vec![
            parse_conventional_commit("abc1234", "feat(ui): new button").unwrap(),
        ];
        let entry = generate_changelog_entry("1.0.0", &commits, false, false, true, None);
        assert!(entry.contains("**ui**: new button"));
    }

    // -----------------------------------------------------------------------
    // Coordinated versioning tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_coordinated_picks_highest_version() {
        // Simulate the coordinated logic: find max version, bump once
        let versions = vec!["1.0.0", "2.3.1", "1.5.0", "0.9.0"];
        let highest = versions
            .iter()
            .filter_map(|v| Version::parse(v).ok())
            .max()
            .unwrap();
        assert_eq!(highest, Version::new(2, 3, 1));

        // Apply a patch bump to the highest
        let next = compute_next_version(&highest.to_string(), "patch").unwrap();
        assert_eq!(next.to_string(), "2.3.2");
    }

    #[test]
    fn test_coordinated_minor_bump() {
        let versions = vec!["1.0.0", "3.1.0", "2.0.0"];
        let highest = versions
            .iter()
            .filter_map(|v| Version::parse(v).ok())
            .max()
            .unwrap();
        assert_eq!(highest, Version::new(3, 1, 0));

        let next = compute_next_version(&highest.to_string(), "minor").unwrap();
        assert_eq!(next.to_string(), "3.2.0");
    }

    #[test]
    fn test_coordinated_major_bump() {
        let versions = vec!["1.0.0", "1.2.0", "1.2.3"];
        let highest = versions
            .iter()
            .filter_map(|v| Version::parse(v).ok())
            .max()
            .unwrap();
        assert_eq!(highest, Version::new(1, 2, 3));

        let next = compute_next_version(&highest.to_string(), "major").unwrap();
        assert_eq!(next.to_string(), "2.0.0");
    }

    #[test]
    fn test_coordinated_explicit_version() {
        // Coordinated with an explicit version string as bump
        let next = compute_next_version("1.0.0", "5.0.0").unwrap();
        assert_eq!(next.to_string(), "5.0.0");
    }

    #[test]
    fn test_coordinated_all_same_version() {
        // All packages already at the same version
        let versions = vec!["2.0.0", "2.0.0", "2.0.0"];
        let highest = versions
            .iter()
            .filter_map(|v| Version::parse(v).ok())
            .max()
            .unwrap();
        assert_eq!(highest, Version::new(2, 0, 0));

        let next = compute_next_version(&highest.to_string(), "patch").unwrap();
        assert_eq!(next.to_string(), "2.0.1");
    }

    #[test]
    fn test_coordinated_with_flutter_build_numbers() {
        // Flutter versions like "1.2.3+4" — semver parses +N as build metadata.
        // Build metadata is ignored in ordering so 2.0.0+5 == 2.0.0 for max().
        // The coordinated logic strips build metadata via to_string() on the
        // base version before bumping, so the result is a clean semver.
        let versions = vec!["1.0.0", "2.0.0+5"];
        let highest = versions
            .iter()
            .filter_map(|v| {
                Version::parse(v)
                    .or_else(|_| {
                        let cleaned = v.split('+').next().unwrap_or(v);
                        Version::parse(cleaned)
                    })
                    .ok()
            })
            .max()
            .unwrap();
        // major.minor.patch is 2.0.0 regardless of build metadata
        assert_eq!(highest.major, 2);
        assert_eq!(highest.minor, 0);
        assert_eq!(highest.patch, 0);

        // Bump using only the base version (stripping build metadata)
        let base = format!("{}.{}.{}", highest.major, highest.minor, highest.patch);
        let next = compute_next_version(&base, "minor").unwrap();
        assert_eq!(next.to_string(), "2.1.0");
    }

    // -----------------------------------------------------------------------
    // Prerelease versioning tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_compute_next_prerelease_fresh() {
        // Stable version -> first prerelease
        let v = compute_next_prerelease("1.0.0", "minor", "dev").unwrap();
        assert_eq!(v.to_string(), "1.1.0-dev.0");
    }

    #[test]
    fn test_compute_next_prerelease_increment() {
        // Already a prerelease with same preid + base -> increment counter
        let v = compute_next_prerelease("1.1.0-dev.0", "minor", "dev").unwrap();
        assert_eq!(v.to_string(), "1.1.0-dev.1");
    }

    #[test]
    fn test_compute_next_prerelease_increment_high() {
        let v = compute_next_prerelease("1.1.0-dev.5", "minor", "dev").unwrap();
        assert_eq!(v.to_string(), "1.1.0-dev.6");
    }

    #[test]
    fn test_compute_next_prerelease_different_preid() {
        // Different preid -> reset to 0
        let v = compute_next_prerelease("1.1.0-beta.3", "minor", "dev").unwrap();
        assert_eq!(v.to_string(), "1.1.0-dev.0");
    }

    #[test]
    fn test_compute_next_prerelease_different_base() {
        // Current is dev.3 at 1.1.0, bump type is "major" but since already a
        // prerelease with the same preid, it just increments the counter
        let v = compute_next_prerelease("1.1.0-dev.3", "major", "dev").unwrap();
        assert_eq!(v.to_string(), "1.1.0-dev.4");
    }

    #[test]
    fn test_compute_next_prerelease_patch() {
        let v = compute_next_prerelease("2.0.0", "patch", "alpha").unwrap();
        assert_eq!(v.to_string(), "2.0.1-alpha.0");
    }

    #[test]
    fn test_compute_next_prerelease_major_already_at_major() {
        // Already at major prerelease -> increment
        let v = compute_next_prerelease("2.0.0-dev.0", "major", "dev").unwrap();
        assert_eq!(v.to_string(), "2.0.0-dev.1");
    }

    // -----------------------------------------------------------------------
    // Graduate versioning tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_graduate_version_dev() {
        let v = graduate_version("1.1.0-dev.3").unwrap();
        assert_eq!(v.to_string(), "1.1.0");
    }

    #[test]
    fn test_graduate_version_beta() {
        let v = graduate_version("2.0.0-beta.1").unwrap();
        assert_eq!(v.to_string(), "2.0.0");
    }

    #[test]
    fn test_graduate_version_already_stable() {
        let v = graduate_version("1.0.0").unwrap();
        assert_eq!(v.to_string(), "1.0.0");
    }

    // -----------------------------------------------------------------------
    // is_prerelease tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_prerelease_true() {
        assert!(is_prerelease("1.0.0-dev.0"));
        assert!(is_prerelease("2.3.1-beta.5"));
        assert!(is_prerelease("0.1.0-alpha.0"));
    }

    #[test]
    fn test_is_prerelease_false() {
        assert!(!is_prerelease("1.0.0"));
        assert!(!is_prerelease("2.3.1"));
        assert!(!is_prerelease("0.0.0"));
    }

    #[test]
    fn test_is_prerelease_invalid() {
        assert!(!is_prerelease("not-a-version"));
    }

    // -----------------------------------------------------------------------
    // Changelog with repository commit links
    // -----------------------------------------------------------------------

    #[test]
    fn test_generate_changelog_with_commit_links() {
        let repo = RepositoryConfig {
            url: "https://github.com/org/repo".to_string(),
        };
        let commits = vec![
            parse_conventional_commit("abc1234", "feat(ui): new button").unwrap(),
        ];
        let entry = generate_changelog_entry("2.0.0", &commits, false, true, true, Some(&repo));
        // Should contain a markdown link instead of bare hash
        assert!(entry.contains("[abc1234](https://github.com/org/repo/commit/abc1234)"));
        assert!(!entry.contains(" (abc1234)"), "Should not have bare hash");
    }

    #[test]
    fn test_generate_changelog_hash_no_repo() {
        let commits = vec![
            parse_conventional_commit("abc1234", "feat: something").unwrap(),
        ];
        let entry = generate_changelog_entry("1.0.0", &commits, false, true, true, None);
        // Without repository, should be bare hash in parens
        assert!(entry.contains("(abc1234)"));
        assert!(!entry.contains("[abc1234]"));
    }

    // -----------------------------------------------------------------------
    // Commit message template with {new_package_versions} placeholder
    // -----------------------------------------------------------------------

    #[test]
    fn test_message_placeholder_replacement() {
        let template = "chore(release): publish\n\n{new_package_versions}";
        let versions = vec![
            ("pkg_a".to_string(), "1.2.0".to_string()),
            ("pkg_b".to_string(), "3.0.0".to_string()),
        ];
        let new_package_versions = versions
            .iter()
            .map(|(name, ver)| format!(" - {} @ {}", name, ver))
            .collect::<Vec<_>>()
            .join("\n");
        let result = template.replace("{new_package_versions}", &new_package_versions);
        assert!(result.contains(" - pkg_a @ 1.2.0"));
        assert!(result.contains(" - pkg_b @ 3.0.0"));
        assert!(result.starts_with("chore(release): publish"));
    }
}
