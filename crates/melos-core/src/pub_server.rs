//! Pub server client for fetching published package versions and authenticating
//! against private pub repositories.
//!
//! Supports both `pub.dev` and custom hosted pub servers. Authentication tokens
//! are read from `~/.config/dart/pub-tokens.json` (Dart SDK standard location).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Default pub.dev hosted URL.
pub const DEFAULT_PUB_HOSTED_URL: &str = "https://pub.dev";

// ---------------------------------------------------------------------------
// Token file parsing
// ---------------------------------------------------------------------------

/// Top-level structure of `~/.config/dart/pub-tokens.json`.
#[derive(Debug, Deserialize)]
struct PubTokensFile {
    /// Version of the token file format (currently always 1).
    #[serde(default)]
    #[allow(dead_code)]
    version: u32,
    /// Hosted URL -> token entries. The Dart SDK uses `hostedUrl` as the key.
    #[serde(rename = "hostedUrl", default)]
    hosted_url: Vec<PubTokenEntry>,
}

/// A single hosted URL + token entry.
#[derive(Debug, Deserialize)]
struct PubTokenEntry {
    /// The hosted URL this token is for (e.g. `https://pub.example.com`).
    url: String,
    /// The bearer token for authentication.
    token: String,
}

/// Loaded pub tokens indexed by normalized URL.
#[derive(Debug, Clone)]
pub struct PubTokens {
    /// Map from normalized URL (lowercase, no trailing slash) to bearer token.
    tokens: HashMap<String, String>,
}

impl PubTokens {
    /// Load tokens from the default Dart SDK location.
    ///
    /// On macOS/Linux: `~/.config/dart/pub-tokens.json`
    /// On Windows: `%APPDATA%\dart\pub-tokens.json`
    ///
    /// Returns an empty token set if the file doesn't exist or can't be parsed.
    pub fn load() -> Self {
        Self::load_from_path(&Self::default_path())
    }

    /// Load tokens from a specific path. Returns empty set on any error.
    pub fn load_from_path(path: &Path) -> Self {
        match Self::try_load(path) {
            Ok(tokens) => tokens,
            Err(_) => Self {
                tokens: HashMap::new(),
            },
        }
    }

    /// Try to load and parse the token file.
    fn try_load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read pub tokens from {}", path.display()))?;
        let file: PubTokensFile = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse pub tokens from {}", path.display()))?;

        let mut tokens = HashMap::new();
        for entry in file.hosted_url {
            let normalized = normalize_url(&entry.url);
            tokens.insert(normalized, entry.token);
        }

        Ok(Self { tokens })
    }

    /// Get the bearer token for a given hosted URL, if one exists.
    pub fn token_for(&self, hosted_url: &str) -> Option<&str> {
        let normalized = normalize_url(hosted_url);
        self.tokens.get(&normalized).map(|s| s.as_str())
    }

    /// The default path for pub-tokens.json.
    fn default_path() -> PathBuf {
        if cfg!(target_os = "windows") {
            // %APPDATA%\dart\pub-tokens.json
            dirs_path("APPDATA", "dart")
        } else {
            // ~/.config/dart/pub-tokens.json
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".config")
                .join("dart")
                .join("pub-tokens.json")
        }
    }
}

/// Build a path from an environment variable + subdir.
fn dirs_path(env_var: &str, subdir: &str) -> PathBuf {
    let base = std::env::var(env_var).unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(subdir).join("pub-tokens.json")
}

/// Normalize a URL for consistent matching: lowercase, strip trailing slash.
fn normalize_url(url: &str) -> String {
    let lower = url.to_lowercase();
    lower.trim_end_matches('/').to_string()
}

// ---------------------------------------------------------------------------
// Pub API client
// ---------------------------------------------------------------------------

/// Response from the pub.dev API `GET /api/packages/<name>`.
#[derive(Debug, Deserialize)]
struct PubPackageResponse {
    versions: Vec<PubVersionEntry>,
}

/// A single version entry from the pub API response.
#[derive(Debug, Deserialize)]
struct PubVersionEntry {
    version: String,
}

/// Fetch all published version strings for a package from a pub server.
///
/// Calls `GET <hosted_url>/api/packages/<package_name>` and parses
/// `versions[].version` from the response.
///
/// If `tokens` is provided and contains a matching token for `hosted_url`,
/// the request includes an `Authorization: Bearer <token>` header.
///
/// Returns an empty vec on 404 (package not yet published).
/// Returns an error on network/parse failures.
pub async fn fetch_published_versions(
    hosted_url: &str,
    package_name: &str,
    tokens: Option<&PubTokens>,
) -> Result<Vec<String>> {
    let url = format!(
        "{}/api/packages/{}",
        hosted_url.trim_end_matches('/'),
        package_name
    );

    let client = reqwest::Client::new();
    let mut request = client.get(&url);

    // Inject auth token if available
    if let Some(tokens) = tokens
        && let Some(token) = tokens.token_for(hosted_url)
    {
        request = request.bearer_auth(token);
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("Failed to fetch versions from {}", url))?;

    // 404 = package not yet published, return empty
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }

    // Other error statuses
    if !response.status().is_success() {
        anyhow::bail!(
            "Pub server returned HTTP {} for {}",
            response.status(),
            url
        );
    }

    let body: PubPackageResponse = response
        .json()
        .await
        .with_context(|| format!("Failed to parse JSON response from {}", url))?;

    Ok(body.versions.into_iter().map(|v| v.version).collect())
}

/// Check if a specific version is already published on a pub server.
///
/// Convenience wrapper around [`fetch_published_versions`].
pub async fn is_version_published(
    hosted_url: &str,
    package_name: &str,
    version: &str,
    tokens: Option<&PubTokens>,
) -> Result<bool> {
    let versions = fetch_published_versions(hosted_url, package_name, tokens).await?;
    Ok(versions.iter().any(|v| v == version))
}

/// Determine the hosted URL for a package.
///
/// If `publish_to` is `Some` and not `"none"`, use it as the hosted URL.
/// Otherwise, fall back to `DEFAULT_PUB_HOSTED_URL`.
///
/// Returns `None` if the package is private (publish_to: "none").
pub fn hosted_url_for_package(publish_to: Option<&str>) -> Option<&str> {
    match publish_to {
        Some(url) if url.eq_ignore_ascii_case("none") => None,
        Some(url) if !url.is_empty() => Some(url),
        _ => Some(DEFAULT_PUB_HOSTED_URL),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // -- normalize_url tests --

    #[test]
    fn test_normalize_url_strips_trailing_slash() {
        assert_eq!(normalize_url("https://pub.dev/"), "https://pub.dev");
    }

    #[test]
    fn test_normalize_url_lowercases() {
        assert_eq!(
            normalize_url("https://PUB.Example.COM"),
            "https://pub.example.com"
        );
    }

    #[test]
    fn test_normalize_url_already_normalized() {
        assert_eq!(normalize_url("https://pub.dev"), "https://pub.dev");
    }

    // -- PubTokens tests --

    #[test]
    fn test_load_tokens_valid_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pub-tokens.json");
        fs::write(
            &path,
            r#"{
                "version": 1,
                "hostedUrl": [
                    {"url": "https://pub.dev", "token": "pub-dev-token"},
                    {"url": "https://custom.pub.example.com/", "token": "custom-token"}
                ]
            }"#,
        )
        .unwrap();

        let tokens = PubTokens::load_from_path(&path);
        assert_eq!(tokens.token_for("https://pub.dev"), Some("pub-dev-token"));
        assert_eq!(
            tokens.token_for("https://custom.pub.example.com/"),
            Some("custom-token")
        );
        assert_eq!(
            tokens.token_for("https://custom.pub.example.com"),
            Some("custom-token"),
            "trailing slash normalization"
        );
    }

    #[test]
    fn test_load_tokens_missing_file() {
        let tokens = PubTokens::load_from_path(Path::new("/nonexistent/pub-tokens.json"));
        assert!(tokens.tokens.is_empty());
    }

    #[test]
    fn test_load_tokens_malformed_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pub-tokens.json");
        fs::write(&path, "not json at all").unwrap();

        let tokens = PubTokens::load_from_path(&path);
        assert!(tokens.tokens.is_empty());
    }

    #[test]
    fn test_load_tokens_empty_hosted_url_array() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pub-tokens.json");
        fs::write(&path, r#"{"version": 1, "hostedUrl": []}"#).unwrap();

        let tokens = PubTokens::load_from_path(&path);
        assert!(tokens.tokens.is_empty());
    }

    #[test]
    fn test_token_for_case_insensitive_url_match() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pub-tokens.json");
        fs::write(
            &path,
            r#"{"version": 1, "hostedUrl": [{"url": "https://Pub.Dev", "token": "t1"}]}"#,
        )
        .unwrap();

        let tokens = PubTokens::load_from_path(&path);
        assert_eq!(tokens.token_for("https://pub.dev"), Some("t1"));
        assert_eq!(tokens.token_for("https://PUB.DEV"), Some("t1"));
    }

    #[test]
    fn test_token_for_no_match() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pub-tokens.json");
        fs::write(
            &path,
            r#"{"version": 1, "hostedUrl": [{"url": "https://pub.dev", "token": "t1"}]}"#,
        )
        .unwrap();

        let tokens = PubTokens::load_from_path(&path);
        assert!(tokens.token_for("https://other.example.com").is_none());
    }

    // -- hosted_url_for_package tests --

    #[test]
    fn test_hosted_url_for_package_custom() {
        assert_eq!(
            hosted_url_for_package(Some("https://custom.pub.dev")),
            Some("https://custom.pub.dev")
        );
    }

    #[test]
    fn test_hosted_url_for_package_none_private() {
        assert_eq!(hosted_url_for_package(Some("none")), None);
        assert_eq!(hosted_url_for_package(Some("NONE")), None);
    }

    #[test]
    fn test_hosted_url_for_package_default() {
        assert_eq!(
            hosted_url_for_package(None),
            Some(DEFAULT_PUB_HOSTED_URL)
        );
    }

    #[test]
    fn test_hosted_url_for_package_empty_string() {
        assert_eq!(
            hosted_url_for_package(Some("")),
            Some(DEFAULT_PUB_HOSTED_URL)
        );
    }

    // -- fetch_published_versions tests (require network, so mock-style) --
    // Real HTTP tests would use a mock server; these test the parsing/logic.

    #[test]
    fn test_pub_package_response_deserialization() {
        let json = r#"{
            "name": "my_package",
            "versions": [
                {"version": "1.0.0"},
                {"version": "1.1.0"},
                {"version": "2.0.0-beta.1"}
            ]
        }"#;
        let response: PubPackageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.versions.len(), 3);
        assert_eq!(response.versions[0].version, "1.0.0");
        assert_eq!(response.versions[1].version, "1.1.0");
        assert_eq!(response.versions[2].version, "2.0.0-beta.1");
    }

    #[test]
    fn test_pub_package_response_empty_versions() {
        let json = r#"{"name": "new_package", "versions": []}"#;
        let response: PubPackageResponse = serde_json::from_str(json).unwrap();
        assert!(response.versions.is_empty());
    }

    #[test]
    fn test_pub_package_response_extra_fields_ignored() {
        let json = r#"{
            "name": "pkg",
            "latest": {"version": "1.0.0"},
            "versions": [{"version": "1.0.0", "pubspec": {}}]
        }"#;
        let response: PubPackageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.versions.len(), 1);
    }
}
