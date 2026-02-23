//! Pure logic helpers for the `run` command.
//!
//! This module contains string-processing utilities used when executing named
//! scripts from `melos.yaml`. All functions are free of terminal/colored
//! dependencies so they can be tested and reused independently.

use std::collections::HashMap;
use std::time::Duration;

/// Maximum recursion depth for nested script references.
pub const MAX_SCRIPT_DEPTH: usize = 16;

/// Parsed exec flags extracted from a `melos exec [flags] -- <command>` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecFlags {
    pub concurrency: usize,
    pub fail_fast: bool,
    pub order_dependents: bool,
    pub timeout: Option<Duration>,
    pub dry_run: bool,
    pub file_exists: Option<String>,
}

impl Default for ExecFlags {
    fn default() -> Self {
        Self {
            concurrency: 5, // Melos default
            fail_fast: false,
            order_dependents: false,
            timeout: None,
            dry_run: false,
            file_exists: None,
        }
    }
}

/// Parse all exec flags from a `melos exec [flags] -- <command>` string.
///
/// Recognizes: `-c N` / `--concurrency N`, `--fail-fast`, `--order-dependents`,
/// `--timeout N`, `--dry-run`, `--file-exists[=]<path>`.
pub fn parse_exec_flags(command: &str) -> ExecFlags {
    let mut flags = ExecFlags::default();
    let parts: Vec<&str> = command.split_whitespace().collect();

    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "-c" | "--concurrency" => {
                if i + 1 < parts.len() {
                    if let Ok(n) = parts[i + 1].parse::<usize>() {
                        flags.concurrency = n;
                    }
                    i += 1;
                }
            }
            "--fail-fast" => flags.fail_fast = true,
            "--order-dependents" => flags.order_dependents = true,
            "--dry-run" => flags.dry_run = true,
            "--timeout" => {
                if i + 1 < parts.len() {
                    if let Ok(secs) = parts[i + 1].parse::<u64>()
                        && secs > 0
                    {
                        flags.timeout = Some(Duration::from_secs(secs));
                    }
                    i += 1;
                }
            }
            "--file-exists" => {
                // Space-separated form: --file-exists pubspec.yaml
                if i + 1 < parts.len() {
                    flags.file_exists = Some(strip_outer_quotes(parts[i + 1]).to_string());
                    i += 1;
                }
            }
            s if s.starts_with("--file-exists=") => {
                // Equals form: --file-exists="pubspec.yaml" or --file-exists=pubspec.yaml
                let value = &s["--file-exists=".len()..];
                flags.file_exists = Some(strip_outer_quotes(value).to_string());
            }
            "--" => break, // Stop parsing flags at separator
            _ => {}
        }
        i += 1;
    }

    flags
}

/// Check if a command is an exec-style command (runs in each package).
pub fn is_exec_command(command: &str) -> bool {
    let trimmed = command.trim();
    trimmed.starts_with("melos exec")
        || trimmed.starts_with("melos-rs exec")
        || trimmed.contains("melos exec")
        || trimmed.contains("melos-rs exec")
}

/// Extract the actual command from a `melos exec -- <command>` string.
///
/// The command after `--` may be wrapped in quotes in the YAML source
/// (e.g. `-- "flutter pub upgrade && exit"`). Because `split_whitespace`
/// does not understand quoting, the leading and trailing quote characters
/// end up as part of the first/last tokens. [`strip_outer_quotes`] removes
/// them so the shell receives a plain command string.
pub fn extract_exec_command(command: &str) -> String {
    // Look for standalone `--` separator (space-delimited token, not just any `--` prefix)
    let parts: Vec<&str> = command.split_whitespace().collect();
    if let Some(pos) = parts.iter().position(|&p| p == "--") {
        let joined = parts[pos + 1..].join(" ");
        return strip_outer_quotes(&joined);
    }

    // Fallback: strip `melos exec` / `melos-rs exec` prefix and all known flags
    let stripped = command
        .replace("melos-rs exec", "")
        .replace("melos exec", "");

    // Remove known flags like -c N, --fail-fast, --order-dependents, --timeout N, --dry-run,
    // --file-exists[=]<value>
    let parts: Vec<&str> = stripped.split_whitespace().collect();
    let mut result = Vec::new();
    let mut skip_next = false;

    for part in &parts {
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(
            *part,
            "-c" | "--concurrency" | "--timeout" | "--file-exists"
        ) {
            skip_next = true;
            continue;
        }
        if matches!(*part, "--fail-fast" | "--order-dependents" | "--dry-run") {
            continue;
        }
        if part.starts_with("--file-exists=") {
            continue;
        }
        result.push(*part);
    }

    let joined = result.join(" ");
    strip_outer_quotes(joined.trim())
}

/// Strip matching outer quote characters (double or single) from a string.
///
/// When YAML contains `-- "flutter pub upgrade && exit"`, the quotes are
/// literal characters in the plain scalar. After `split_whitespace` + `join`,
/// they appear as `"flutter pub upgrade && exit"`. The shell would treat the
/// quoted content as a single word (command name), causing "command not found".
/// Stripping the outer quotes produces a plain command string the shell
/// interprets correctly.
pub fn strip_outer_quotes(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        return trimmed[1..trimmed.len() - 1].to_string();
    }
    trimmed.to_string()
}

/// Extract the script name from a `melos-rs run <name>` or `melos run <name>` command.
///
/// Returns `Some(script_name)` if the command is a simple `melos[-rs] run <name>` invocation
/// with no extra flags or arguments after the script name.
/// Returns `None` if the command doesn't match or has trailing args.
pub fn extract_melos_run_script_name(command: &str) -> Option<&str> {
    let trimmed = command.trim();

    // Try `melos-rs run <name>` first, then `melos run <name>`
    let rest = trimmed
        .strip_prefix("melos-rs run ")
        .or_else(|| trimmed.strip_prefix("melos run "))?;

    let rest = rest.trim();

    // The script name must be a single token (no spaces, no extra args)
    if rest.is_empty() || rest.contains(' ') {
        return None;
    }

    Some(rest)
}

/// Normalize shell line continuations in a command string.
///
/// YAML literal block scalars (`|`) preserve newlines and backslash characters
/// literally. A script like:
/// ```yaml
/// run: |
///   melos exec -c 1 -- \
///     flutter analyze .
/// ```
/// produces the string `"melos exec -c 1 -- \\\n    flutter analyze .\n"`.
/// The `\<newline>` is a shell line continuation that should collapse into a
/// single space so that downstream parsing (split_whitespace, etc.) does not
/// treat the backslash as part of the command.
pub fn normalize_line_continuations(command: &str) -> String {
    let mut result = String::with_capacity(command.len());
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'\n') {
            // Consume the newline and any leading whitespace on the next line
            chars.next(); // skip '\n'
            while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
                chars.next();
            }
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}

/// Substitute environment variables in a command string.
///
/// Replaces `${VAR_NAME}` (braced form) and `$VAR_NAME` (bare form) with their
/// values from the env map. The bare `$VAR` form uses word-boundary matching to
/// avoid replacing `$MELOS_ROOT_PATH` when `$MELOS_ROOT` is also defined.
pub fn substitute_env_vars(command: &str, env: &HashMap<String, String>) -> String {
    let mut result = command.to_string();

    // Sort keys by length descending so longer variable names are replaced first.
    // This prevents `$MELOS_ROOT` from matching before `$MELOS_ROOT_PATH`.
    let mut sorted_keys: Vec<&String> = env.keys().collect();
    sorted_keys.sort_by_key(|k| std::cmp::Reverse(k.len()));

    for key in sorted_keys {
        let value = &env[key];
        // Replace ${VAR} form (always safe - braces delimit the name)
        result = result.replace(&format!("${{{}}}", key), value);

        // Replace $VAR form with word-boundary awareness:
        // Match $KEY only when NOT followed by another alphanumeric or underscore.
        // Since the regex crate doesn't support lookahead, we use a replacement
        // closure that checks the character after the match.
        let pattern = format!(r"\${}", regex::escape(key));
        if let Ok(re) = regex::Regex::new(&pattern) {
            let bytes = result.clone();
            let bytes = bytes.as_bytes();
            result = re
                .replace_all(&result.clone(), |caps: &regex::Captures| {
                    // safety: regex group 0 always exists in a Captures
                    let m = caps.get(0).expect("regex group 0 always exists");
                    let end = m.end();
                    // If followed by an alphanumeric or underscore, don't replace
                    if end < bytes.len() {
                        let next = bytes[end];
                        if next.is_ascii_alphanumeric() || next == b'_' {
                            return caps[0].to_string();
                        }
                    }
                    value.clone()
                })
                .to_string();
        }
    }

    result
}

/// Expand a run command, resolving `melos run <X>` references to the actual
/// melos-rs binary, and splitting `&&` chains into separate commands.
///
/// For example:
///   "melos run generate:dart && melos run generate:flutter"
/// becomes:
///   ["melos-rs run generate:dart", "melos-rs run generate:flutter"]
///
/// Uses word-boundary-aware replacement to avoid mangling `melos-rs` into `melos-rs-rs`.
pub fn expand_command(command: &str) -> anyhow::Result<Vec<String>> {
    let trimmed = command.trim();

    // Match standalone `melos` as a word. We then check in the replacement
    // whether it's followed by `-rs` (in which case we leave it alone).
    let re = regex::Regex::new(r"\bmelos\b")
        .map_err(|e| anyhow::anyhow!("Failed to compile regex: {}", e))?;

    // Split on `&&` to handle chained commands
    let parts: Vec<String> = trimmed
        .split("&&")
        .map(|part| {
            let part = part.trim();
            // Use replace_all with a closure that checks context after the match
            let bytes = part.as_bytes();
            re.replace_all(part, |caps: &regex::Captures| {
                // safety: regex group 0 always exists in a Captures
                let m = caps.get(0).expect("regex group 0 always exists");
                let end = m.end();
                // If followed by `-rs`, don't replace (it's already melos-rs)
                if end < bytes.len() && bytes[end] == b'-' {
                    // Check for "-rs" suffix
                    if part[end..].starts_with("-rs") {
                        return "melos".to_string();
                    }
                }
                "melos-rs".to_string()
            })
            .to_string()
        })
        .collect();

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_expand_simple_command() {
        let result = expand_command("flutter analyze .").unwrap();
        assert_eq!(result, vec!["flutter analyze ."]);
    }

    #[test]
    fn test_expand_chained_command() {
        let result =
            expand_command("melos run generate:dart && melos run generate:flutter").unwrap();
        assert_eq!(
            result,
            vec![
                "melos-rs run generate:dart",
                "melos-rs run generate:flutter"
            ]
        );
    }

    #[test]
    fn test_expand_exec_command() {
        let result = expand_command("melos exec -c 1 -- flutter analyze .").unwrap();
        assert_eq!(result, vec!["melos-rs exec -c 1 -- flutter analyze ."]);
    }

    #[test]
    fn test_expand_preserves_melos_rs() {
        // Must NOT turn `melos-rs` into `melos-rs-rs`
        let result = expand_command("melos-rs run generate && melos run build").unwrap();
        assert_eq!(result, vec!["melos-rs run generate", "melos-rs run build"]);
    }

    #[test]
    fn test_substitute_env_vars() {
        let mut env = HashMap::new();
        env.insert("MELOS_ROOT_PATH".to_string(), "/workspace".to_string());

        assert_eq!(
            substitute_env_vars("echo $MELOS_ROOT_PATH", &env),
            "echo /workspace"
        );
        assert_eq!(
            substitute_env_vars("echo ${MELOS_ROOT_PATH}/bin", &env),
            "echo /workspace/bin"
        );
    }

    #[test]
    fn test_substitute_env_vars_word_boundary() {
        // When both $MELOS_ROOT and $MELOS_ROOT_PATH are defined,
        // $MELOS_ROOT must NOT match inside $MELOS_ROOT_PATH.
        let mut env = HashMap::new();
        env.insert("MELOS_ROOT".to_string(), "/root".to_string());
        env.insert("MELOS_ROOT_PATH".to_string(), "/workspace".to_string());

        // Bare $MELOS_ROOT_PATH should resolve to /workspace, not /root_PATH
        assert_eq!(
            substitute_env_vars("echo $MELOS_ROOT_PATH", &env),
            "echo /workspace"
        );

        // Bare $MELOS_ROOT alone should still resolve
        assert_eq!(
            substitute_env_vars("echo $MELOS_ROOT end", &env),
            "echo /root end"
        );

        // Both in the same string
        assert_eq!(
            substitute_env_vars("$MELOS_ROOT and $MELOS_ROOT_PATH", &env),
            "/root and /workspace"
        );

        // Braced forms should always be unambiguous
        assert_eq!(
            substitute_env_vars("${MELOS_ROOT} and ${MELOS_ROOT_PATH}", &env),
            "/root and /workspace"
        );

        // $MELOS_ROOT at end of string (no trailing char)
        assert_eq!(substitute_env_vars("path=$MELOS_ROOT", &env), "path=/root");
    }

    #[test]
    fn test_is_exec_command() {
        assert!(is_exec_command("melos exec -- flutter test"));
        assert!(is_exec_command("melos-rs exec -c 5 -- dart test"));
        assert!(!is_exec_command("flutter analyze ."));
        assert!(!is_exec_command("dart format ."));
    }

    #[test]
    fn test_extract_exec_command() {
        assert_eq!(
            extract_exec_command("melos exec -- flutter test"),
            "flutter test"
        );
        assert_eq!(
            extract_exec_command("melos exec -c 5 -- dart analyze ."),
            "dart analyze ."
        );
    }

    #[test]
    fn test_extract_exec_command_strips_new_flags() {
        // Fallback path (no -- separator): should strip all known flags
        assert_eq!(
            extract_exec_command(
                "melos exec --order-dependents --dry-run --timeout 30 flutter test"
            ),
            "flutter test"
        );
    }

    #[test]
    fn test_parse_exec_flags_defaults() {
        let flags = parse_exec_flags("melos exec -- flutter test");
        assert_eq!(flags.concurrency, 5);
        assert!(!flags.fail_fast);
        assert!(!flags.order_dependents);
        assert!(flags.timeout.is_none());
        assert!(!flags.dry_run);
        assert!(flags.file_exists.is_none());
    }

    #[test]
    fn test_parse_exec_flags_concurrency() {
        let flags = parse_exec_flags("melos exec -c 3 -- flutter test");
        assert_eq!(flags.concurrency, 3);

        let flags2 = parse_exec_flags("melos exec --concurrency 10 -- dart test");
        assert_eq!(flags2.concurrency, 10);
    }

    #[test]
    fn test_parse_exec_flags_all() {
        let flags = parse_exec_flags(
            "melos exec -c 2 --fail-fast --order-dependents --timeout 60 --dry-run -- flutter test",
        );
        assert_eq!(flags.concurrency, 2);
        assert!(flags.fail_fast);
        assert!(flags.order_dependents);
        assert_eq!(flags.timeout, Some(Duration::from_secs(60)));
        assert!(flags.dry_run);
    }

    #[test]
    fn test_parse_exec_flags_timeout_zero() {
        let flags = parse_exec_flags("melos exec --timeout 0 -- flutter test");
        assert!(flags.timeout.is_none(), "timeout 0 means no timeout");
    }

    // -- --file-exists flag parsing tests --

    #[test]
    fn test_parse_exec_flags_file_exists_equals_quoted() {
        // Real-world: --file-exists="pubspec.yaml" (quotes are literal YAML chars)
        let flags = parse_exec_flags(
            r#"melos exec --file-exists="pubspec.yaml" -c 1 --fail-fast -- "flutter pub upgrade && exit""#,
        );
        assert_eq!(flags.file_exists, Some("pubspec.yaml".to_string()));
        assert_eq!(flags.concurrency, 1);
        assert!(flags.fail_fast);
    }

    #[test]
    fn test_parse_exec_flags_file_exists_equals_unquoted() {
        let flags = parse_exec_flags("melos exec --file-exists=pubspec.yaml -- flutter test");
        assert_eq!(flags.file_exists, Some("pubspec.yaml".to_string()));
    }

    #[test]
    fn test_parse_exec_flags_file_exists_space_separated() {
        let flags = parse_exec_flags("melos exec --file-exists pubspec.yaml -- flutter test");
        assert_eq!(flags.file_exists, Some("pubspec.yaml".to_string()));
    }

    #[test]
    fn test_parse_exec_flags_file_exists_single_quoted() {
        let flags =
            parse_exec_flags("melos exec --file-exists='test/widget_test.dart' -- flutter test");
        assert_eq!(flags.file_exists, Some("test/widget_test.dart".to_string()));
    }

    #[test]
    fn test_parse_exec_flags_no_file_exists() {
        let flags = parse_exec_flags("melos exec -c 3 --fail-fast -- flutter test");
        assert!(flags.file_exists.is_none());
    }

    #[test]
    fn test_extract_exec_command_strips_file_exists_equals() {
        // Fallback path (no -- separator): --file-exists=<val> should be stripped
        assert_eq!(
            extract_exec_command("melos exec --file-exists=pubspec.yaml --fail-fast flutter test"),
            "flutter test"
        );
    }

    #[test]
    fn test_extract_exec_command_strips_file_exists_space() {
        // Fallback path: --file-exists <val> (space-separated) should be stripped
        assert_eq!(
            extract_exec_command("melos exec --file-exists pubspec.yaml --fail-fast flutter test"),
            "flutter test"
        );
    }

    #[test]
    fn test_extract_melos_run_script_name() {
        // Should extract script name from `melos run <name>`
        assert_eq!(
            extract_melos_run_script_name("melos run build"),
            Some("build")
        );
        assert_eq!(
            extract_melos_run_script_name("melos-rs run build"),
            Some("build")
        );
        assert_eq!(
            extract_melos_run_script_name("melos-rs run generate:dart"),
            Some("generate:dart")
        );

        // Should return None for non-matching commands
        assert_eq!(extract_melos_run_script_name("flutter analyze ."), None);
        assert_eq!(extract_melos_run_script_name("dart format ."), None);

        // Should return None when there are extra args after the script name
        assert_eq!(
            extract_melos_run_script_name("melos-rs run build --verbose"),
            None
        );

        // Should return None when no script name is given
        assert_eq!(extract_melos_run_script_name("melos run "), None);
        assert_eq!(extract_melos_run_script_name("melos-rs run"), None);
    }

    // -- normalize_line_continuations tests --

    #[test]
    fn test_normalize_line_continuations_basic() {
        // Backslash-newline is collapsed to a single space
        assert_eq!(
            normalize_line_continuations("melos exec -c 1 -- \\\n    flutter analyze ."),
            "melos exec -c 1 --  flutter analyze ."
        );
    }

    #[test]
    fn test_normalize_line_continuations_multiline_yaml() {
        // Simulates what YAML `|` produces for the analyze script
        let yaml_string = "melos exec -c 1 -- \\\n  flutter analyze .\n";
        let normalized = normalize_line_continuations(yaml_string);
        assert_eq!(normalized, "melos exec -c 1 --  flutter analyze .\n");
        // After normalization, extract_exec_command should work correctly
        assert_eq!(extract_exec_command(&normalized), "flutter analyze .");
    }

    #[test]
    fn test_normalize_line_continuations_no_continuations() {
        let cmd = "melos exec -c 1 -- flutter analyze .";
        assert_eq!(normalize_line_continuations(cmd), cmd);
    }

    #[test]
    fn test_normalize_line_continuations_backslash_not_before_newline() {
        // Backslash not followed by newline should be preserved
        let cmd = "echo hello\\ world";
        assert_eq!(normalize_line_continuations(cmd), cmd);
    }

    #[test]
    fn test_normalize_line_continuations_multiple() {
        let cmd = "first \\\n  second \\\n  third";
        assert_eq!(normalize_line_continuations(cmd), "first  second  third");
    }

    // -- strip_outer_quotes tests --

    #[test]
    fn test_strip_outer_quotes_double() {
        assert_eq!(
            strip_outer_quotes("\"flutter pub upgrade && exit\""),
            "flutter pub upgrade && exit"
        );
    }

    #[test]
    fn test_strip_outer_quotes_single() {
        assert_eq!(
            strip_outer_quotes("'flutter pub upgrade && exit'"),
            "flutter pub upgrade && exit"
        );
    }

    #[test]
    fn test_strip_outer_quotes_no_quotes() {
        assert_eq!(
            strip_outer_quotes("flutter pub upgrade"),
            "flutter pub upgrade"
        );
    }

    #[test]
    fn test_strip_outer_quotes_mismatched() {
        // Mismatched quotes should not be stripped
        assert_eq!(
            strip_outer_quotes("\"flutter pub upgrade'"),
            "\"flutter pub upgrade'"
        );
    }

    #[test]
    fn test_strip_outer_quotes_single_char() {
        // Edge case: single quote char alone
        assert_eq!(strip_outer_quotes("\""), "\"");
    }

    // -- extract_exec_command with quotes tests --

    #[test]
    fn test_extract_exec_command_strips_double_quotes() {
        // Real-world case: melos.yaml has -- "flutter pub upgrade && exit"
        assert_eq!(
            extract_exec_command(
                "melos exec --file-exists=\"pubspec.yaml\" -c 1 --fail-fast -- \"flutter pub upgrade && exit\""
            ),
            "flutter pub upgrade && exit"
        );
    }

    #[test]
    fn test_extract_exec_command_strips_single_quotes() {
        assert_eq!(
            extract_exec_command("melos exec -c 1 -- 'flutter pub upgrade && exit'"),
            "flutter pub upgrade && exit"
        );
    }

    #[test]
    fn test_extract_exec_command_no_quotes_preserved() {
        // No quotes: command should be returned as-is
        assert_eq!(
            extract_exec_command("melos exec -- flutter test"),
            "flutter test"
        );
    }
}
