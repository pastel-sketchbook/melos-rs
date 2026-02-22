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

    /// Generate changelogs (default: true). Alias for the positive side of --[no-]changelog.
    #[arg(short = 'c', long, conflicts_with = "no_changelog")]
    pub changelog: bool,

    /// Skip git tag creation
    #[arg(long, alias = "no-git-tag")]
    pub no_git_tag_version: bool,

    /// Create git tags (default: true). Short flag for --[no-]git-tag-version.
    #[arg(short = 't', long, conflicts_with = "no_git_tag_version")]
    pub git_tag_version: bool,

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

    /// Print release URL links after versioning (requires `repository` in config).
    /// Generates prefilled release creation page links for each package.
    #[arg(long, short = 'r')]
    pub release_url: bool,
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

/// Options controlling changelog entry generation.
pub struct ChangelogOptions<'a> {
    pub include_body: bool,
    /// When true, only include commit bodies for breaking changes (default: true).
    /// Only has effect when `include_body` is true.
    pub only_breaking_bodies: bool,
    pub include_hash: bool,
    pub include_scopes: bool,
    pub repository: Option<&'a RepositoryConfig>,
    pub include_types: Option<&'a [String]>,
    pub exclude_types: Option<&'a [String]>,
    /// Whether to include the date in the version header (default: false per Melos docs).
    pub include_date: bool,
}

impl Default for ChangelogOptions<'_> {
    fn default() -> Self {
        Self {
            include_body: false,
            only_breaking_bodies: true,
            include_hash: false,
            include_scopes: true,
            repository: None,
            include_types: None,
            exclude_types: None,
            include_date: false,
        }
    }
}

/// Generate a CHANGELOG.md entry for a package version
pub fn generate_changelog_entry(
    version: &str,
    commits: &[ConventionalCommit],
    opts: &ChangelogOptions<'_>,
) -> String {
    let mut sections: HashMap<&str, Vec<String>> = HashMap::new();

    // Group commits by type -> human-readable section
    for commit in commits {
        // Apply type filtering
        if let Some(included) = opts.include_types
            && !included.iter().any(|t| t == &commit.commit_type)
        {
            continue;
        }
        if let Some(excluded) = opts.exclude_types
            && excluded.iter().any(|t| t == &commit.commit_type)
        {
            continue;
        }
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

        let scope_prefix = if opts.include_scopes {
            commit
                .scope
                .as_ref()
                .map(|s| format!("**{}**: ", s))
                .unwrap_or_default()
        } else {
            String::new()
        };

        let hash_suffix = if opts.include_hash {
            if let Some(repo) = opts.repository {
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

        // Include body if configured, respecting only_breaking_bodies filter
        if opts.include_body
            && let Some(ref body) = commit.body
            && (!opts.only_breaking_bodies || commit.breaking)
        {
            entry.push_str(&format!("\n  {}", body.replace('\n', "\n  ")));
        }

        if commit.breaking {
            entry.push_str("\n  **BREAKING CHANGE**");
        }

        sections.entry(section).or_default().push(entry);
    }

    let mut output = if opts.include_date {
        let date = chrono_date_today();
        format!("## {} ({})\n", version, date)
    } else {
        format!("## {}\n", version)
    };

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

/// Fetch tags from the remote repository.
///
/// Used when `command.version.fetchTags: true` to ensure local tag data is
/// up-to-date before analyzing conventional commits relative to tags.
pub fn git_fetch_tags(root: &Path) -> Result<()> {
    let status = std::process::Command::new("git")
        .args(["fetch", "--tags"])
        .current_dir(root)
        .status()
        .context("Failed to fetch tags")?;

    if !status.success() {
        bail!("git fetch --tags failed");
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
// Aggregate changelog filter helper
// ---------------------------------------------------------------------------

/// Check if a package name matches the given aggregate changelog filters.
///
/// This is a simplified filter check for aggregate changelogs — it only
/// evaluates `scope` and `ignore` glob patterns against the package name.
fn package_matches_filters(
    pkg_name: &str,
    filters: &crate::config::filter::PackageFilters,
    _packages: &[Package],
) -> bool {
    // Scope filter: if set, package name must match at least one scope glob
    if let Some(ref scopes) = filters.scope {
        let matches_scope = scopes.iter().any(|pattern| {
            glob::Pattern::new(pattern)
                .map(|p| p.matches(pkg_name))
                .unwrap_or(false)
        });
        if !matches_scope {
            return false;
        }
    }

    // Ignore filter: if set, exclude packages matching any ignore glob
    if let Some(ref ignores) = filters.ignore {
        let matches_ignore = ignores.iter().any(|pattern| {
            glob::Pattern::new(pattern)
                .map(|p| p.matches(pkg_name))
                .unwrap_or(false)
        });
        if matches_ignore {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Git tag ref updating
// ---------------------------------------------------------------------------

/// Update git tag references in dependent packages' pubspec.yaml files.
///
/// When a package is versioned, other packages that depend on it via a git
/// dependency with `ref:` pointing to a tag may need their `ref:` updated
/// to point to the new tag.
///
/// Returns the number of files updated.
fn update_git_tag_refs(
    root: &Path,
    packages: &[Package],
    versioned: &[(String, String)],
) -> Result<usize> {
    let mut updated_count = 0;

    for pkg in packages {
        let pubspec_path = pkg.path.join("pubspec.yaml");
        if !pubspec_path.exists() {
            continue;
        }

        let content = std::fs::read_to_string(&pubspec_path)
            .with_context(|| format!("Failed to read {}", pubspec_path.display()))?;

        let mut new_content = content.clone();

        for (dep_name, new_version) in versioned {
            // Look for git dependency blocks like:
            //   dep_name:
            //     git:
            //       ...
            //       ref: dep_name-v1.2.3
            // We need to find the `ref:` line that contains a tag for this dependency
            let old_tag_pattern = format!(
                r"(?m)(^\s+{}:\s*\n(?:\s+\w[^\n]*\n)*?\s+ref:\s*)({}-v\S+)",
                regex::escape(dep_name),
                regex::escape(dep_name),
            );
            let new_tag = format!("{}-v{}", dep_name, new_version);

            if let Ok(re) = regex::Regex::new(&old_tag_pattern) {
                new_content = re
                    .replace(&new_content, format!("${{1}}{}", new_tag))
                    .to_string();
            }
        }

        if new_content != content {
            std::fs::write(&pubspec_path, &new_content)
                .with_context(|| format!("Failed to write {}", pubspec_path.display()))?;

            let rel_path = pubspec_path
                .strip_prefix(root)
                .unwrap_or(&pubspec_path);
            println!(
                "  {} Updated git tag refs in {}",
                "OK".green(),
                rel_path.display()
            );
            updated_count += 1;
        }
    }

    Ok(updated_count)
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

    // Fetch tags from remote if configured
    if version_config
        .map(|c| c.should_fetch_tags())
        .unwrap_or(false)
    {
        println!("  {} Fetching tags from remote...", "$".cyan());
        git_fetch_tags(&workspace.root_path)?;
        println!("  {} Tags fetched", "OK".green());
    }

    // Determine changelog/tag settings from config + CLI flags
    let should_changelog = if args.no_changelog {
        false
    } else {
        version_config.is_none_or(|c| c.should_changelog())
    };
    let should_tag = if args.no_git_tag_version {
        false
    } else {
        version_config.is_none_or(|c| c.should_tag())
    };

    // Resolve commit body inclusion: changelogCommitBodies takes precedence over
    // changelogConfig.includeCommitBody for backward compatibility.
    let (include_body, only_breaking_bodies) = if let Some(bodies_cfg) = version_config
        .and_then(|c| c.changelog_commit_bodies.as_ref())
    {
        (bodies_cfg.include, bodies_cfg.only_breaking)
    } else {
        let body = version_config
            .and_then(|c| c.changelog_config.as_ref())
            .and_then(|cc| cc.include_commit_body)
            .unwrap_or(false);
        // Legacy includeCommitBody includes ALL bodies (not just breaking)
        (body, false)
    };

    let include_hash = version_config
        .and_then(|c| c.changelog_config.as_ref())
        .and_then(|cc| cc.include_commit_id)
        // link_to_commits is an alias/override for including commit IDs
        .or_else(|| version_config.and_then(|c| c.link_to_commits))
        .unwrap_or(false);
    let include_scopes = version_config
        .and_then(|c| c.include_scopes)
        .unwrap_or(true); // Melos includes scopes by default

    // Resolve changelogFormat.includeDate (default: false per Melos docs)
    let include_date = version_config
        .map(|c| c.should_include_date())
        .unwrap_or(false);

    // Changelog commit type filtering
    let changelog_include_types: Option<Vec<String>> = version_config
        .and_then(|c| c.changelog_config.as_ref())
        .and_then(|cc| cc.include_types.clone());
    let changelog_exclude_types: Option<Vec<String>> = version_config
        .and_then(|c| c.changelog_config.as_ref())
        .and_then(|cc| cc.exclude_types.clone());

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
        let mapped = conventional_commits
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("conventional commits not loaded; this is a bug"))?;
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
                    let changelog_opts = ChangelogOptions {
                        include_body,
                        only_breaking_bodies,
                        include_hash,
                        include_scopes,
                        repository: repo,
                        include_types: changelog_include_types.as_deref(),
                        exclude_types: changelog_exclude_types.as_deref(),
                        include_date,
                    };
                    let entry = generate_changelog_entry(new_ver, commits, &changelog_opts);
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
                    let changelog_opts = ChangelogOptions {
                        include_body,
                        only_breaking_bodies,
                        include_hash,
                        include_scopes,
                        repository: repo,
                        include_types: changelog_include_types.as_deref(),
                        exclude_types: changelog_exclude_types.as_deref(),
                        include_date,
                    };
                    let entry = generate_changelog_entry(
                        summary_version,
                        &all_commits,
                        &changelog_opts,
                    );
                    write_changelog(&workspace.root_path, &entry)?;
                    println!(
                        "  {} Updated workspace CHANGELOG.md",
                        "OK".green()
                    );
                }
            }

            // Aggregate changelogs
            if let Some(agg_configs) = version_config.and_then(|c| c.changelogs.as_ref()) {
                for agg in agg_configs {
                    let agg_path = workspace.root_path.join(&agg.path);

                    // Filter commits to only those from packages matching the aggregate filters
                    let agg_commits: Vec<ConventionalCommit> = if let Some(ref filters) = agg.package_filters {
                        mapped
                            .iter()
                            .filter(|(pkg_name, _)| {
                                package_matches_filters(pkg_name, filters, &workspace.packages)
                            })
                            .flat_map(|(_, commits)| commits.iter().cloned())
                            .collect()
                    } else {
                        // No filters — include all commits
                        mapped.values().flatten().cloned().collect()
                    };

                    if !agg_commits.is_empty() {
                        let agg_version = versioned
                            .first()
                            .map(|(_, v)| v.as_str())
                            .unwrap_or("0.0.0");
                        let changelog_opts = ChangelogOptions {
                            include_body,
                            only_breaking_bodies,
                            include_hash,
                            include_scopes,
                            repository: repo,
                            include_types: changelog_include_types.as_deref(),
                            exclude_types: changelog_exclude_types.as_deref(),
                            include_date,
                        };
                        let entry = generate_changelog_entry(agg_version, &agg_commits, &changelog_opts);

                        // If the file has a description configured, ensure it's at the top
                        let full_entry = if let Some(ref desc) = agg.description {
                            if !agg_path.exists() {
                                format!("# Changelog\n\n{}\n\n{}", desc, entry)
                            } else {
                                entry
                            }
                        } else {
                            entry
                        };

                        write_changelog(&agg_path, &full_entry)?;
                        println!(
                            "  {} Updated aggregate changelog {}",
                            "OK".green(),
                            agg.path.bold()
                        );
                    }
                }
            }
        } else {
            println!(
                "\n{} Changelog generation requires --conventional-commits; skipping.",
                "NOTE:".yellow()
            );
        }
    }

    // Update git tag references in dependent packages if configured
    let should_update_refs = version_config
        .map(|c| c.should_update_git_tag_refs())
        .unwrap_or(false);
    if should_update_refs && !versioned.is_empty() {
        println!("\n{} Updating git tag references...", "$".cyan());
        let count = update_git_tag_refs(&workspace.root_path, &workspace.packages, &versioned)?;
        if count > 0 {
            println!(
                "  {} Updated git tag refs in {} file(s)",
                "OK".green(),
                count
            );
        } else {
            println!("  {} No git tag refs to update", "OK".green());
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

    // Print release URLs if requested (CLI flag or config)
    let should_release_url = args.release_url
        || version_config
            .map(|c| c.should_release_url())
            .unwrap_or(false);
    if should_release_url {
        if let Some(ref repo) = workspace.config.repository {
            println!("\n{} Release URLs:", "$".cyan());
            for (pkg_name, version) in &versioned {
                let tag = format!("{}-v{}", pkg_name, version);
                let title = format!("{} v{}", pkg_name, version);
                let url = repo.release_url(&tag, &title);
                println!("  {} {}", pkg_name.bold(), url);
            }
        } else {
            println!(
                "\n{} --release-url requires `repository` in config; skipping.",
                "WARN:".yellow()
            );
        }
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
        let entry = generate_changelog_entry("1.2.0", &commits, &ChangelogOptions::default());
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
        let entry = generate_changelog_entry("2.0.0", &commits, &ChangelogOptions {
            include_hash: true,
            ..ChangelogOptions::default()
        });
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
        let entry = generate_changelog_entry("1.0.0", &commits, &ChangelogOptions {
            include_scopes: false,
            ..ChangelogOptions::default()
        });
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
        let entry = generate_changelog_entry("1.0.0", &commits, &ChangelogOptions::default());
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
        let entry = generate_changelog_entry("2.0.0", &commits, &ChangelogOptions {
            include_hash: true,
            repository: Some(&repo),
            ..ChangelogOptions::default()
        });
        // Should contain a markdown link instead of bare hash
        assert!(entry.contains("[abc1234](https://github.com/org/repo/commit/abc1234)"));
        assert!(!entry.contains(" (abc1234)"), "Should not have bare hash");
    }

    #[test]
    fn test_generate_changelog_hash_no_repo() {
        let commits = vec![
            parse_conventional_commit("abc1234", "feat: something").unwrap(),
        ];
        let entry = generate_changelog_entry("1.0.0", &commits, &ChangelogOptions {
            include_hash: true,
            ..ChangelogOptions::default()
        });
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

    // -----------------------------------------------------------------------
    // Changelog commit type filtering tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_changelog_include_types() {
        let commits = vec![
            parse_conventional_commit("a1", "feat: new feature").unwrap(),
            parse_conventional_commit("b2", "fix: bug fix").unwrap(),
            parse_conventional_commit("c3", "chore: update deps").unwrap(),
            parse_conventional_commit("d4", "ci: update pipeline").unwrap(),
        ];
        let include = vec!["feat".to_string(), "fix".to_string()];
        let entry = generate_changelog_entry(
            "1.0.0", &commits, &ChangelogOptions {
                include_types: Some(&include),
                ..ChangelogOptions::default()
            },
        );
        assert!(entry.contains("new feature"));
        assert!(entry.contains("bug fix"));
        assert!(!entry.contains("update deps"), "chore should be excluded");
        assert!(!entry.contains("update pipeline"), "ci should be excluded");
    }

    #[test]
    fn test_changelog_exclude_types() {
        let commits = vec![
            parse_conventional_commit("a1", "feat: new feature").unwrap(),
            parse_conventional_commit("b2", "chore: update deps").unwrap(),
            parse_conventional_commit("c3", "ci: update pipeline").unwrap(),
        ];
        let exclude = vec!["chore".to_string(), "ci".to_string()];
        let entry = generate_changelog_entry(
            "1.0.0", &commits, &ChangelogOptions {
                exclude_types: Some(&exclude),
                ..ChangelogOptions::default()
            },
        );
        assert!(entry.contains("new feature"));
        assert!(!entry.contains("update deps"), "chore should be excluded");
        assert!(!entry.contains("update pipeline"), "ci should be excluded");
    }

    #[test]
    fn test_changelog_include_takes_precedence_over_exclude() {
        let commits = vec![
            parse_conventional_commit("a1", "feat: new feature").unwrap(),
            parse_conventional_commit("b2", "fix: bug fix").unwrap(),
            parse_conventional_commit("c3", "chore: update deps").unwrap(),
        ];
        // include only feat, but also exclude chore
        // include should be the primary filter
        let include = vec!["feat".to_string()];
        let exclude = vec!["chore".to_string()];
        let entry = generate_changelog_entry(
            "1.0.0", &commits, &ChangelogOptions {
                include_types: Some(&include),
                exclude_types: Some(&exclude),
                ..ChangelogOptions::default()
            },
        );
        assert!(entry.contains("new feature"));
        assert!(!entry.contains("bug fix"), "fix not in include list");
        assert!(!entry.contains("update deps"), "chore excluded");
    }

    #[test]
    fn test_changelog_no_filters() {
        let commits = vec![
            parse_conventional_commit("a1", "feat: new feature").unwrap(),
            parse_conventional_commit("b2", "chore: update deps").unwrap(),
        ];
        let entry = generate_changelog_entry(
            "1.0.0", &commits, &ChangelogOptions::default(),
        );
        // All types should be included
        assert!(entry.contains("new feature"));
        assert!(entry.contains("update deps"));
    }

    // -----------------------------------------------------------------------
    // Config parsing tests for new fields
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_changelog_config_with_type_filters() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    changelogConfig:
      includeCommitBody: true
      includeCommitId: true
      includeTypes:
        - feat
        - fix
      excludeTypes:
        - chore
        - ci
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        let changelog_config = version_config.changelog_config.unwrap();
        assert_eq!(
            changelog_config.include_types,
            Some(vec!["feat".to_string(), "fix".to_string()])
        );
        assert_eq!(
            changelog_config.exclude_types,
            Some(vec!["chore".to_string(), "ci".to_string()])
        );
    }

    #[test]
    fn test_parse_fetch_tags_config() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    fetchTags: true
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(version_config.should_fetch_tags());
    }

    #[test]
    fn test_parse_fetch_tags_default_false() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    branch: main
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(!version_config.should_fetch_tags());
    }

    // -----------------------------------------------------------------------
    // Bootstrap config parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_bootstrap_config_with_hooks() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  bootstrap:
    enforceLockfile: true
    hooks:
      pre: echo pre-bootstrap
      post: echo post-bootstrap
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let bootstrap_config = config.command.unwrap().bootstrap.unwrap();
        assert_eq!(bootstrap_config.enforce_lockfile, Some(true));
        let hooks = bootstrap_config.hooks.unwrap();
        assert_eq!(hooks.pre.as_deref(), Some("echo pre-bootstrap"));
        assert_eq!(hooks.post.as_deref(), Some("echo post-bootstrap"));
    }

    #[test]
    fn test_parse_clean_config_with_pre_hook() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  clean:
    hooks:
      pre: echo pre-clean
      post: echo post-clean
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let clean_config = config.command.unwrap().clean.unwrap();
        let hooks = clean_config.hooks.unwrap();
        assert_eq!(hooks.pre.as_deref(), Some("echo pre-clean"));
        assert_eq!(hooks.post.as_deref(), Some("echo post-clean"));
    }

    // -----------------------------------------------------------------------
    // Batch 15: releaseUrl config + CLI flag
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_release_url_config() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    releaseUrl: true
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(version_config.should_release_url());
    }

    #[test]
    fn test_parse_release_url_default_false() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    branch: main
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(!version_config.should_release_url());
    }

    #[test]
    fn test_repository_release_url() {
        let repo = RepositoryConfig {
            url: "https://github.com/invertase/melos".to_string(),
        };
        let url = repo.release_url("my_pkg-v1.2.0", "my_pkg v1.2.0");
        assert!(url.starts_with("https://github.com/invertase/melos/releases/new?"));
        assert!(url.contains("tag=my_pkg-v1.2.0"));
        assert!(url.contains("title=my_pkg%20v1.2.0"));
    }

    #[test]
    fn test_repository_release_url_special_chars() {
        let repo = RepositoryConfig {
            url: "https://github.com/org/repo".to_string(),
        };
        let url = repo.release_url("pkg-v2.0.0-beta.1", "pkg v2.0.0-beta.1");
        assert!(url.contains("tag=pkg-v2.0.0-beta.1"));
        // Spaces should be encoded as %20
        assert!(url.contains("title=pkg%20v2.0.0-beta.1"));
    }

    // -----------------------------------------------------------------------
    // Batch 15: aggregate changelogs config
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_aggregate_changelogs_config() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    changelogs:
      - path: CHANGELOG_APPS.md
        packageFilters:
          scope:
            - "app_*"
        description: "Changes in application packages"
      - path: CHANGELOG_LIBS.md
        packageFilters:
          scope:
            - "core_*"
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        let changelogs = version_config.changelogs.unwrap();
        assert_eq!(changelogs.len(), 2);
        assert_eq!(changelogs[0].path, "CHANGELOG_APPS.md");
        assert_eq!(
            changelogs[0].description.as_deref(),
            Some("Changes in application packages")
        );
        let filters = changelogs[0].package_filters.as_ref().unwrap();
        assert_eq!(filters.scope, Some(vec!["app_*".to_string()]));
        assert_eq!(changelogs[1].path, "CHANGELOG_LIBS.md");
        assert!(changelogs[1].description.is_none());
    }

    #[test]
    fn test_parse_aggregate_changelogs_default_none() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    branch: main
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(version_config.changelogs.is_none());
    }

    #[test]
    fn test_package_matches_filters_scope() {
        let filters = crate::config::filter::PackageFilters {
            scope: Some(vec!["app_*".to_string()]),
            ..Default::default()
        };
        assert!(package_matches_filters("app_mobile", &filters, &[]));
        assert!(package_matches_filters("app_web", &filters, &[]));
        assert!(!package_matches_filters("core_utils", &filters, &[]));
    }

    #[test]
    fn test_package_matches_filters_ignore() {
        let filters = crate::config::filter::PackageFilters {
            ignore: Some(vec!["*_example".to_string()]),
            ..Default::default()
        };
        assert!(package_matches_filters("app_mobile", &filters, &[]));
        assert!(!package_matches_filters("app_example", &filters, &[]));
    }

    #[test]
    fn test_package_matches_filters_scope_and_ignore() {
        let filters = crate::config::filter::PackageFilters {
            scope: Some(vec!["app_*".to_string()]),
            ignore: Some(vec!["app_example".to_string()]),
            ..Default::default()
        };
        assert!(package_matches_filters("app_mobile", &filters, &[]));
        assert!(!package_matches_filters("app_example", &filters, &[]));
        assert!(!package_matches_filters("core_utils", &filters, &[]));
    }

    #[test]
    fn test_package_matches_filters_no_filters() {
        let filters = crate::config::filter::PackageFilters::default();
        assert!(package_matches_filters("anything", &filters, &[]));
    }

    // -----------------------------------------------------------------------
    // Batch 15: changelogCommitBodies config
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_changelog_commit_bodies_config() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    changelogCommitBodies:
      include: true
      onlyBreaking: true
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        let bodies = version_config.changelog_commit_bodies.unwrap();
        assert!(bodies.include);
        assert!(bodies.only_breaking);
    }

    #[test]
    fn test_parse_changelog_commit_bodies_only_breaking_default() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    changelogCommitBodies:
      include: true
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        let bodies = version_config.changelog_commit_bodies.unwrap();
        assert!(bodies.include);
        assert!(bodies.only_breaking, "onlyBreaking should default to true");
    }

    #[test]
    fn test_parse_changelog_commit_bodies_all_bodies() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    changelogCommitBodies:
      include: true
      onlyBreaking: false
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        let bodies = version_config.changelog_commit_bodies.unwrap();
        assert!(bodies.include);
        assert!(!bodies.only_breaking);
    }

    #[test]
    fn test_changelog_only_breaking_bodies() {
        // Breaking commit with body should include body
        let breaking_commit = ConventionalCommit {
            commit_type: "feat".to_string(),
            scope: None,
            breaking: true,
            description: "new API".to_string(),
            body: Some("This changes the entire API surface.".to_string()),
            hash: "abc".to_string(),
        };
        // Non-breaking commit with body should NOT include body
        let normal_commit = ConventionalCommit {
            commit_type: "fix".to_string(),
            scope: None,
            breaking: false,
            description: "fix null check".to_string(),
            body: Some("Fixed a null pointer issue.".to_string()),
            hash: "def".to_string(),
        };
        let entry = generate_changelog_entry("1.0.0", &[breaking_commit, normal_commit], &ChangelogOptions {
            include_body: true,
            only_breaking_bodies: true,
            ..ChangelogOptions::default()
        });
        // Breaking commit's body should be included
        assert!(entry.contains("This changes the entire API surface."));
        // Non-breaking commit's body should NOT be included
        assert!(!entry.contains("Fixed a null pointer issue."));
    }

    #[test]
    fn test_changelog_all_bodies() {
        let breaking_commit = ConventionalCommit {
            commit_type: "feat".to_string(),
            scope: None,
            breaking: true,
            description: "new API".to_string(),
            body: Some("Breaking body.".to_string()),
            hash: "abc".to_string(),
        };
        let normal_commit = ConventionalCommit {
            commit_type: "fix".to_string(),
            scope: None,
            breaking: false,
            description: "fix null check".to_string(),
            body: Some("Normal body.".to_string()),
            hash: "def".to_string(),
        };
        let entry = generate_changelog_entry("1.0.0", &[breaking_commit, normal_commit], &ChangelogOptions {
            include_body: true,
            only_breaking_bodies: false,
            ..ChangelogOptions::default()
        });
        // Both bodies should be included
        assert!(entry.contains("Breaking body."));
        assert!(entry.contains("Normal body."));
    }

    // -----------------------------------------------------------------------
    // Batch 15: changelogFormat.includeDate config
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_changelog_format_include_date() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    changelogFormat:
      includeDate: true
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(version_config.should_include_date());
    }

    #[test]
    fn test_parse_changelog_format_default_no_date() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    branch: main
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(!version_config.should_include_date());
    }

    #[test]
    fn test_changelog_without_date() {
        let commits = vec![
            parse_conventional_commit("abc", "feat: something new").unwrap(),
        ];
        let entry = generate_changelog_entry("1.0.0", &commits, &ChangelogOptions {
            include_date: false,
            ..ChangelogOptions::default()
        });
        // Should be "## 1.0.0\n" without a date
        assert!(entry.starts_with("## 1.0.0\n"));
        // Should NOT contain a date pattern like (2026-02-22)
        assert!(!entry.contains("(20"));
    }

    #[test]
    fn test_changelog_with_date() {
        let commits = vec![
            parse_conventional_commit("abc", "feat: something new").unwrap(),
        ];
        let entry = generate_changelog_entry("1.0.0", &commits, &ChangelogOptions {
            include_date: true,
            ..ChangelogOptions::default()
        });
        // Should be "## 1.0.0 (YYYY-MM-DD)\n" with a date
        assert!(entry.starts_with("## 1.0.0 ("));
        // Check date pattern
        let re = regex::Regex::new(r"## 1\.0\.0 \(\d{4}-\d{2}-\d{2}\)").unwrap();
        assert!(re.is_match(&entry), "Expected date in header, got: {}", entry);
    }

    // -----------------------------------------------------------------------
    // Batch 15: updateGitTagRefs config
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_update_git_tag_refs_config() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    updateGitTagRefs: true
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(version_config.should_update_git_tag_refs());
    }

    #[test]
    fn test_parse_update_git_tag_refs_default_false() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
command:
  version:
    branch: main
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(!version_config.should_update_git_tag_refs());
    }

    #[test]
    fn test_update_git_tag_refs_in_pubspec() {
        // Create a temp dir with a package that has a git dependency ref
        let tmp = tempfile::tempdir().unwrap();
        let pkg_path = tmp.path().join("packages").join("my_app");
        std::fs::create_dir_all(&pkg_path).unwrap();

        let pubspec_content = r#"name: my_app
version: 1.0.0
dependencies:
  core_lib:
    git:
      url: https://github.com/org/repo.git
      path: packages/core_lib
      ref: core_lib-v1.0.0
"#;
        std::fs::write(pkg_path.join("pubspec.yaml"), pubspec_content).unwrap();

        let packages = vec![
            Package {
                name: "my_app".to_string(),
                path: pkg_path.clone(),
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                dependencies: vec!["core_lib".to_string()],
                dev_dependencies: vec![],
                dependency_versions: HashMap::new(),
                publish_to: None,
            },
        ];
        let versioned = vec![("core_lib".to_string(), "2.0.0".to_string())];

        let count = update_git_tag_refs(tmp.path(), &packages, &versioned).unwrap();
        assert_eq!(count, 1);

        let updated = std::fs::read_to_string(pkg_path.join("pubspec.yaml")).unwrap();
        assert!(updated.contains("ref: core_lib-v2.0.0"), "Expected updated ref, got:\n{}", updated);
        assert!(!updated.contains("ref: core_lib-v1.0.0"));
    }

    #[test]
    fn test_update_git_tag_refs_no_match() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_path = tmp.path().join("packages").join("my_app");
        std::fs::create_dir_all(&pkg_path).unwrap();

        // This pubspec has a path dependency, not a git ref
        let pubspec_content = r#"name: my_app
version: 1.0.0
dependencies:
  core_lib:
    path: ../core_lib
"#;
        std::fs::write(pkg_path.join("pubspec.yaml"), pubspec_content).unwrap();

        let packages = vec![
            Package {
                name: "my_app".to_string(),
                path: pkg_path,
                version: Some("1.0.0".to_string()),
                is_flutter: false,
                dependencies: vec!["core_lib".to_string()],
                dev_dependencies: vec![],
                dependency_versions: HashMap::new(),
                publish_to: None,
            },
        ];
        let versioned = vec![("core_lib".to_string(), "2.0.0".to_string())];

        let count = update_git_tag_refs(tmp.path(), &packages, &versioned).unwrap();
        assert_eq!(count, 0);
    }

    // -----------------------------------------------------------------------
    // Batch 15: url_encode helper
    // -----------------------------------------------------------------------

    #[test]
    fn test_url_encode_basic() {
        assert_eq!(crate::config::url_encode("hello"), "hello");
        assert_eq!(crate::config::url_encode("hello world"), "hello%20world");
        assert_eq!(crate::config::url_encode("a+b"), "a%2Bb");
        assert_eq!(crate::config::url_encode("v1.0.0-beta.1"), "v1.0.0-beta.1");
    }

    // -----------------------------------------------------------------------
    // Batch 15: combined config parsing test
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_all_batch15_config_fields() {
        let yaml = r#"
name: test_project
packages:
  - packages/**
repository: https://github.com/org/repo
command:
  version:
    releaseUrl: true
    updateGitTagRefs: true
    changelogFormat:
      includeDate: true
    changelogCommitBodies:
      include: true
      onlyBreaking: false
    changelogs:
      - path: CHANGELOG_MOBILE.md
        packageFilters:
          scope:
            - "mobile_*"
        description: "Mobile changes"
"#;
        let config: crate::config::MelosConfig = yaml_serde::from_str(yaml).unwrap();
        let version_config = config.command.unwrap().version.unwrap();
        assert!(version_config.should_release_url());
        assert!(version_config.should_update_git_tag_refs());
        assert!(version_config.should_include_date());

        let bodies = version_config.changelog_commit_bodies.unwrap();
        assert!(bodies.include);
        assert!(!bodies.only_breaking);

        let changelogs = version_config.changelogs.unwrap();
        assert_eq!(changelogs.len(), 1);
        assert_eq!(changelogs[0].path, "CHANGELOG_MOBILE.md");
    }
}
