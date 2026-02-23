use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use tokio::sync::mpsc::UnboundedSender;

use crate::events::Event;
use crate::package::Package;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

use super::PackageResults;

/// Options for the analyze command (clap-free).
#[derive(Debug, Clone)]
pub struct AnalyzeOpts {
    pub concurrency: usize,
    pub fatal_warnings: bool,
    pub fatal_infos: bool,
    pub no_fatal: bool,
}

/// Result of a `dart fix --dry-run` scan across packages.
pub struct DryRunScan {
    pub entries: Vec<DryRunFileEntry>,
    pub codes: BTreeSet<String>,
    pub conflicts: Vec<ConflictingPair>,
}

/// A file entry parsed from `dart fix --dry-run` output.
#[derive(Debug)]
pub struct DryRunFileEntry {
    /// Path relative to workspace root (e.g., "packages/ui/lib/foo.dart")
    pub path: String,
    /// Diagnostic fixes: (code, count)
    pub fixes: Vec<(String, usize)>,
}

/// A pair of diagnostic codes detected as conflicting.
#[derive(Debug, PartialEq)]
pub struct ConflictingPair {
    pub code_a: String,
    pub code_b: String,
    pub file_count: usize,
}

/// Regex for diagnostic fix lines from `dart fix --dry-run` output.
///
/// Matches patterns like:
///   `omit_local_variable_types - 2 fixes`        (Dart SDK 3.x)
///   `omit_local_variable_types \u{2022} 2 fixes`  (older SDKs)
///
/// Captures: (1) diagnostic code, (2) fix count.
/// The separator is matched as any non-word, non-digit character sequence.
pub static FIX_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // safety: this regex is valid and tested
    Regex::new(r"^(\w+)\s+\S+\s+(\d+)\s+fix(?:es)?$").expect("valid regex")
});

/// Build a `dart fix` command string.
///
/// - `apply`: true for `--apply`, false for `--dry-run`
/// - `codes`: optional diagnostic codes to restrict fixes to
pub fn build_fix_command(apply: bool, codes: &[String]) -> String {
    let mut parts = vec!["dart".to_string(), "fix".to_string()];
    parts.push(if apply {
        "--apply".to_string()
    } else {
        "--dry-run".to_string()
    });
    for code in codes {
        parts.push(format!("--code={code}"));
    }
    parts.join(" ")
}

/// Build the analyze command string from flags.
///
/// Uses `flutter analyze` for Flutter packages and `dart analyze` for
/// Dart-only packages. This matters because `flutter analyze` defaults to
/// `--fatal-infos` while `dart analyze` does not.
pub fn build_analyze_command(
    is_flutter: bool,
    fatal_warnings: bool,
    fatal_infos: bool,
    no_fatal: bool,
) -> String {
    let sdk = if is_flutter { "flutter" } else { "dart" };
    let mut cmd_parts = vec![sdk.to_string(), "analyze".to_string()];

    if !no_fatal {
        if fatal_warnings {
            cmd_parts.push("--fatal-warnings".to_string());
        }
        if fatal_infos {
            cmd_parts.push("--fatal-infos".to_string());
        }
    } else {
        cmd_parts.push("--no-fatal-warnings".to_string());
        cmd_parts.push("--no-fatal-infos".to_string());
    }

    // Analyze the current directory (package root)
    cmd_parts.push(".".to_string());

    cmd_parts.join(" ")
}

/// Parse a single diagnostic fix line from `dart fix --dry-run` output.
///
/// Returns `(code, count)` on success.
pub fn parse_fix_line(line: &str) -> Option<(String, usize)> {
    let caps = FIX_LINE_RE.captures(line)?;
    let code = caps[1].to_string();
    let count: usize = caps[2].parse().ok()?;
    Some((code, count))
}

/// Parse `dart fix --dry-run` stdout into file entries.
///
/// Each file path is prefixed with `pkg_prefix` (the package's relative path
/// from the workspace root) to produce workspace-relative paths.
pub fn parse_dry_run_output(stdout: &str, pkg_prefix: &str) -> Vec<DryRunFileEntry> {
    let mut entries = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_fixes: Vec<(String, usize)> = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();

        // Skip blank lines and known non-file lines; flush any pending entry
        if trimmed.is_empty()
            || trimmed.starts_with("Computing fixes")
            || trimmed.starts_with("Nothing to fix")
            || trimmed.starts_with("To fix")
            || trimmed.contains("fixes in")
            || trimmed.contains("fix in")
        {
            if let Some(path) = current_path.take()
                && !current_fixes.is_empty()
            {
                entries.push(DryRunFileEntry {
                    path,
                    fixes: std::mem::take(&mut current_fixes),
                });
            }
            continue;
        }

        // Indented line = diagnostic fix (or dart fix suggestion, which the regex skips)
        if line.starts_with("  ") {
            if let Some((code, count)) = parse_fix_line(trimmed) {
                current_fixes.push((code, count));
            }
        } else if trimmed.ends_with(".dart") {
            // Non-indented Dart file path = start of new file entry
            if let Some(path) = current_path.take()
                && !current_fixes.is_empty()
            {
                entries.push(DryRunFileEntry {
                    path,
                    fixes: std::mem::take(&mut current_fixes),
                });
            }
            current_path = Some(format!("{}/{}", pkg_prefix, trimmed));
        }
        // Any other non-indented line is ignored (future-proofing)
    }

    // Flush last entry
    if let Some(path) = current_path
        && !current_fixes.is_empty()
    {
        entries.push(DryRunFileEntry {
            path,
            fixes: current_fixes,
        });
    }

    entries
}

/// Detect diagnostic code pairs that likely conflict.
///
/// Two codes are considered conflicting when they appear together in the same
/// file with identical fix counts across at least `min_files` files.
pub fn detect_conflicting_diagnostics(
    entries: &[DryRunFileEntry],
    min_files: usize,
) -> Vec<ConflictingPair> {
    // For each file, build a map of code -> count, then record every equal-count pair
    let mut pair_counts: BTreeMap<(String, String), usize> = BTreeMap::new();

    for entry in entries {
        let fixes = &entry.fixes;
        for i in 0..fixes.len() {
            for j in (i + 1)..fixes.len() {
                if fixes[i].1 == fixes[j].1 {
                    // Normalize pair order for consistent counting
                    let (a, b) = if fixes[i].0 < fixes[j].0 {
                        (fixes[i].0.clone(), fixes[j].0.clone())
                    } else {
                        (fixes[j].0.clone(), fixes[i].0.clone())
                    };
                    *pair_counts.entry((a, b)).or_insert(0) += 1;
                }
            }
        }
    }

    pair_counts
        .into_iter()
        .filter(|(_, count)| *count >= min_files)
        .map(|((code_a, code_b), file_count)| ConflictingPair {
            code_a,
            code_b,
            file_count,
        })
        .collect()
}

/// Format a warning block for conflicting diagnostic pairs.
pub fn format_conflict_warnings(conflicts: &[ConflictingPair]) -> String {
    let mut lines = Vec::new();
    lines.push("WARNING: Conflicting lint rules detected".to_string());
    lines.push(String::new());
    for conflict in conflicts {
        lines.push(format!(
            "  {} and {} conflict ({} files with equal fix counts)",
            conflict.code_a, conflict.code_b, conflict.file_count,
        ));
        lines.push(
            "  Disable one in analysis_options.yaml to avoid a fix/analyze loop:".to_string(),
        );
        lines.push(format!("    {}: false", conflict.code_a));
        lines.push(format!("    {}: false", conflict.code_b));
        lines.push(String::new());
    }
    lines.push(
        "Applying both conflicting fixes will undo each other, leaving the same warnings."
            .to_string(),
    );
    lines.join("\n")
}

/// Assemble a [`DryRunScan`] from parsed entries.
///
/// Sorts entries by path for deterministic output, extracts unique codes,
/// and detects conflicting diagnostic pairs.
pub fn assemble_dry_run_scan(
    mut entries: Vec<DryRunFileEntry>,
    min_conflict_files: usize,
) -> DryRunScan {
    let mut codes = BTreeSet::new();
    for entry in &entries {
        for (code, _) in &entry.fixes {
            codes.insert(code.clone());
        }
    }

    // Sort by path for deterministic output across concurrent runs
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let conflicts = detect_conflicting_diagnostics(&entries, min_conflict_files);

    DryRunScan {
        entries,
        codes,
        conflicts,
    }
}

/// Run `dart analyze` across packages, splitting by SDK type.
///
/// Returns combined [`PackageResults`] from both Flutter and Dart runs.
pub async fn run(
    packages: &[Package],
    workspace: &Workspace,
    opts: &AnalyzeOpts,
    events: Option<&UnboundedSender<Event>>,
) -> Result<PackageResults> {
    let flutter_pkgs: Vec<_> = packages.iter().filter(|p| p.is_flutter).cloned().collect();
    let dart_pkgs: Vec<_> = packages.iter().filter(|p| !p.is_flutter).cloned().collect();

    let runner = ProcessRunner::new(opts.concurrency, false);
    let mut all_results = Vec::new();

    if !flutter_pkgs.is_empty() {
        let cmd = build_analyze_command(true, opts.fatal_warnings, opts.fatal_infos, opts.no_fatal);
        if let Some(tx) = events {
            let _ = tx.send(Event::Progress {
                completed: 0,
                total: 0,
                message: "flutter analyze...".into(),
            });
        }
        let results = runner
            .run_in_packages_with_events(
                &flutter_pkgs,
                &cmd,
                &workspace.env_vars(),
                None,
                events,
                &workspace.packages,
            )
            .await?;
        all_results.extend(results);
    }

    if !dart_pkgs.is_empty() {
        let cmd =
            build_analyze_command(false, opts.fatal_warnings, opts.fatal_infos, opts.no_fatal);
        if let Some(tx) = events {
            let _ = tx.send(Event::Progress {
                completed: 0,
                total: 0,
                message: "dart analyze...".into(),
            });
        }
        let results = runner
            .run_in_packages_with_events(
                &dart_pkgs,
                &cmd,
                &workspace.env_vars(),
                None,
                events,
                &workspace.packages,
            )
            .await?;
        all_results.extend(results);
    }

    Ok(PackageResults::from(all_results))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- build_analyze_command tests --

    #[test]
    fn test_build_analyze_command_default() {
        let cmd = build_analyze_command(false, false, false, false);
        assert_eq!(cmd, "dart analyze .");
    }

    #[test]
    fn test_build_analyze_command_fatal_warnings() {
        let cmd = build_analyze_command(false, true, false, false);
        assert_eq!(cmd, "dart analyze --fatal-warnings .");
    }

    #[test]
    fn test_build_analyze_command_fatal_infos() {
        let cmd = build_analyze_command(false, false, true, false);
        assert_eq!(cmd, "dart analyze --fatal-infos .");
    }

    #[test]
    fn test_build_analyze_command_both_fatal() {
        let cmd = build_analyze_command(false, true, true, false);
        assert_eq!(cmd, "dart analyze --fatal-warnings --fatal-infos .");
    }

    #[test]
    fn test_build_analyze_command_no_fatal_overrides() {
        let cmd = build_analyze_command(false, true, true, true);
        assert_eq!(cmd, "dart analyze --no-fatal-warnings --no-fatal-infos .");
    }

    #[test]
    fn test_build_analyze_command_no_fatal_alone() {
        let cmd = build_analyze_command(false, false, false, true);
        assert_eq!(cmd, "dart analyze --no-fatal-warnings --no-fatal-infos .");
    }

    #[test]
    fn test_build_analyze_command_flutter_default() {
        let cmd = build_analyze_command(true, false, false, false);
        assert_eq!(cmd, "flutter analyze .");
    }

    #[test]
    fn test_build_analyze_command_flutter_no_fatal() {
        let cmd = build_analyze_command(true, false, false, true);
        assert_eq!(
            cmd,
            "flutter analyze --no-fatal-warnings --no-fatal-infos ."
        );
    }

    #[test]
    fn test_build_analyze_command_flutter_fatal_warnings() {
        let cmd = build_analyze_command(true, true, false, false);
        assert_eq!(cmd, "flutter analyze --fatal-warnings .");
    }

    // -- build_fix_command tests --

    #[test]
    fn test_build_fix_command_apply() {
        let cmd = build_fix_command(true, &[]);
        assert_eq!(cmd, "dart fix --apply");
    }

    #[test]
    fn test_build_fix_command_dry_run() {
        let cmd = build_fix_command(false, &[]);
        assert_eq!(cmd, "dart fix --dry-run");
    }

    #[test]
    fn test_build_fix_command_apply_with_codes() {
        let codes = vec![
            "deprecated_member_use".to_string(),
            "unused_import".to_string(),
        ];
        let cmd = build_fix_command(true, &codes);
        assert_eq!(
            cmd,
            "dart fix --apply --code=deprecated_member_use --code=unused_import"
        );
    }

    #[test]
    fn test_build_fix_command_dry_run_with_single_code() {
        let codes = vec!["unnecessary_cast".to_string()];
        let cmd = build_fix_command(false, &codes);
        assert_eq!(cmd, "dart fix --dry-run --code=unnecessary_cast");
    }

    // -- parse_fix_line tests --

    #[test]
    fn test_parse_fix_line_dash_separator() {
        let result = parse_fix_line("omit_local_variable_types - 4 fixes");
        assert_eq!(result, Some(("omit_local_variable_types".to_string(), 4)));
    }

    #[test]
    fn test_parse_fix_line_bullet_separator() {
        let result = parse_fix_line("omit_local_variable_types \u{2022} 4 fixes");
        assert_eq!(result, Some(("omit_local_variable_types".to_string(), 4)));
    }

    #[test]
    fn test_parse_fix_line_single_fix() {
        let result = parse_fix_line("unused_import - 1 fix");
        assert_eq!(result, Some(("unused_import".to_string(), 1)));
    }

    #[test]
    fn test_parse_fix_line_dart_fix_command() {
        assert!(parse_fix_line("dart fix --apply --code=foo").is_none());
    }

    #[test]
    fn test_parse_fix_line_empty() {
        assert!(parse_fix_line("").is_none());
    }

    // -- parse_dry_run_output tests --

    #[test]
    fn test_parse_dry_run_output_dart3_format() {
        let stdout = "\
Computing fixes in ui (dry run)...

112 proposed fixes in 13 files.

lib/app.router.dart
  omit_local_variable_types - 2 fixes
  specify_nonobvious_local_variable_types - 2 fixes

lib/main.dart
  omit_local_variable_types - 4 fixes
  specify_nonobvious_local_variable_types - 4 fixes

To fix an individual diagnostic, run one of:
  dart fix --apply --code=omit_local_variable_types
  dart fix --apply --code=specify_nonobvious_local_variable_types

To fix all diagnostics, run:
  dart fix --apply";

        let entries = parse_dry_run_output(stdout, "packages/ui");
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].path, "packages/ui/lib/app.router.dart");
        assert_eq!(entries[0].fixes.len(), 2);
        assert_eq!(
            entries[0].fixes[0],
            ("omit_local_variable_types".to_string(), 2)
        );
        assert_eq!(
            entries[0].fixes[1],
            ("specify_nonobvious_local_variable_types".to_string(), 2)
        );

        assert_eq!(entries[1].path, "packages/ui/lib/main.dart");
        assert_eq!(entries[1].fixes.len(), 2);
        assert_eq!(
            entries[1].fixes[0],
            ("omit_local_variable_types".to_string(), 4)
        );
    }

    #[test]
    fn test_parse_dry_run_output_bullet_separator() {
        let stdout = "\
Computing fixes in /workspace/packages/ui...

lib/shared/router.helper.dart
  omit_local_variable_types \u{2022} 4 fixes
  specify_nonobvious_local_variable_types \u{2022} 4 fixes

lib/utils/register_fonts.dart
  omit_local_variable_types \u{2022} 2 fixes
  specify_nonobvious_local_variable_types \u{2022} 2 fixes

12 fixes in 2 files.

To fix an individual diagnostic, run one of:
  dart fix --apply --code=omit_local_variable_types
  dart fix --apply --code=specify_nonobvious_local_variable_types

To fix all diagnostics, run:
  dart fix --apply";

        let entries = parse_dry_run_output(stdout, "packages/ui");
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].path, "packages/ui/lib/shared/router.helper.dart");
        assert_eq!(entries[0].fixes.len(), 2);
        assert_eq!(
            entries[0].fixes[0],
            ("omit_local_variable_types".to_string(), 4)
        );

        assert_eq!(entries[1].path, "packages/ui/lib/utils/register_fonts.dart");
        assert_eq!(entries[1].fixes.len(), 2);
    }

    #[test]
    fn test_parse_dry_run_output_single_file() {
        let stdout = "\
Computing fixes in core (dry run)...

1 proposed fix in 1 file.

lib/src/utils.dart
  unnecessary_cast - 1 fix";

        let entries = parse_dry_run_output(stdout, "packages/core");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "packages/core/lib/src/utils.dart");
        assert_eq!(entries[0].fixes, vec![("unnecessary_cast".to_string(), 1)]);
    }

    #[test]
    fn test_parse_dry_run_output_nothing_to_fix() {
        let stdout = "Computing fixes in core (dry run)...\nNothing to fix!";
        let entries = parse_dry_run_output(stdout, "packages/core");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_dry_run_output_empty() {
        let entries = parse_dry_run_output("", "packages/core");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_dry_run_output_skips_footer_suggestions() {
        let stdout = "\
lib/foo.dart
  unused_import - 3 fixes

3 fixes in 1 file.

To fix an individual diagnostic, run one of:
  dart fix --apply --code=unused_import

To fix all diagnostics, run:
  dart fix --apply";

        let entries = parse_dry_run_output(stdout, "pkg");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "pkg/lib/foo.dart");
        assert_eq!(entries[0].fixes, vec![("unused_import".to_string(), 3)]);
    }

    #[test]
    fn test_parse_dry_run_output_ignores_non_dart_lines() {
        let stdout = "\
Computing fixes in ui (dry run)...

Some unexpected line here

lib/foo.dart
  unused_import - 1 fix";

        let entries = parse_dry_run_output(stdout, "pkg");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "pkg/lib/foo.dart");
    }

    // -- detect_conflicting_diagnostics tests --

    #[test]
    fn test_detect_conflicts_equal_counts_across_files() {
        let entries = vec![
            DryRunFileEntry {
                path: "pkg/lib/a.dart".to_string(),
                fixes: vec![
                    ("omit_local_variable_types".to_string(), 4),
                    ("specify_nonobvious_local_variable_types".to_string(), 4),
                ],
            },
            DryRunFileEntry {
                path: "pkg/lib/b.dart".to_string(),
                fixes: vec![
                    ("omit_local_variable_types".to_string(), 2),
                    ("specify_nonobvious_local_variable_types".to_string(), 2),
                ],
            },
        ];

        let conflicts = detect_conflicting_diagnostics(&entries, 2);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].code_a, "omit_local_variable_types");
        assert_eq!(
            conflicts[0].code_b,
            "specify_nonobvious_local_variable_types"
        );
        assert_eq!(conflicts[0].file_count, 2);
    }

    #[test]
    fn test_detect_conflicts_below_threshold_ignored() {
        let entries = vec![DryRunFileEntry {
            path: "pkg/lib/a.dart".to_string(),
            fixes: vec![
                ("omit_local_variable_types".to_string(), 3),
                ("specify_nonobvious_local_variable_types".to_string(), 3),
            ],
        }];

        let conflicts = detect_conflicting_diagnostics(&entries, 2);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_detect_conflicts_unequal_counts_not_flagged() {
        let entries = vec![
            DryRunFileEntry {
                path: "pkg/lib/a.dart".to_string(),
                fixes: vec![
                    ("unused_import".to_string(), 3),
                    ("deprecated_member_use".to_string(), 1),
                ],
            },
            DryRunFileEntry {
                path: "pkg/lib/b.dart".to_string(),
                fixes: vec![
                    ("unused_import".to_string(), 2),
                    ("deprecated_member_use".to_string(), 5),
                ],
            },
        ];

        let conflicts = detect_conflicting_diagnostics(&entries, 2);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_detect_conflicts_no_entries() {
        let conflicts = detect_conflicting_diagnostics(&[], 2);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_detect_conflicts_single_diagnostic_per_file() {
        let entries = vec![
            DryRunFileEntry {
                path: "pkg/lib/a.dart".to_string(),
                fixes: vec![("unused_import".to_string(), 2)],
            },
            DryRunFileEntry {
                path: "pkg/lib/b.dart".to_string(),
                fixes: vec![("unused_import".to_string(), 1)],
            },
        ];

        let conflicts = detect_conflicting_diagnostics(&entries, 2);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_detect_conflicts_multiple_pairs() {
        let entries = vec![
            DryRunFileEntry {
                path: "pkg/lib/a.dart".to_string(),
                fixes: vec![
                    ("rule_a".to_string(), 2),
                    ("rule_b".to_string(), 2),
                    ("rule_c".to_string(), 5),
                    ("rule_d".to_string(), 5),
                ],
            },
            DryRunFileEntry {
                path: "pkg/lib/b.dart".to_string(),
                fixes: vec![
                    ("rule_a".to_string(), 3),
                    ("rule_b".to_string(), 3),
                    ("rule_c".to_string(), 1),
                    ("rule_d".to_string(), 1),
                ],
            },
        ];

        let conflicts = detect_conflicting_diagnostics(&entries, 2);
        assert_eq!(conflicts.len(), 2);
        assert_eq!(conflicts[0].code_a, "rule_a");
        assert_eq!(conflicts[0].code_b, "rule_b");
        assert_eq!(conflicts[1].code_a, "rule_c");
        assert_eq!(conflicts[1].code_b, "rule_d");
    }

    // -- format_conflict_warnings tests --

    #[test]
    fn test_format_conflict_warnings_single_pair() {
        let conflicts = vec![ConflictingPair {
            code_a: "omit_local_variable_types".to_string(),
            code_b: "specify_nonobvious_local_variable_types".to_string(),
            file_count: 13,
        }];

        let output = format_conflict_warnings(&conflicts);
        assert!(output.contains("WARNING: Conflicting lint rules detected"));
        assert!(output.contains("omit_local_variable_types and specify_nonobvious_local_variable_types conflict (13 files"));
        assert!(output.contains("Disable one in analysis_options.yaml"));
        assert!(output.contains("omit_local_variable_types: false"));
        assert!(output.contains("specify_nonobvious_local_variable_types: false"));
        assert!(output.contains("Applying both conflicting fixes will undo each other"));
    }

    // -- pre-scan skip logic tests --

    #[test]
    fn test_fix_skipped_when_conflicts_detected_no_code_filter() {
        let entries = vec![
            DryRunFileEntry {
                path: "pkg/lib/a.dart".to_string(),
                fixes: vec![
                    ("omit_local_variable_types".to_string(), 4),
                    ("specify_nonobvious_local_variable_types".to_string(), 4),
                ],
            },
            DryRunFileEntry {
                path: "pkg/lib/b.dart".to_string(),
                fixes: vec![
                    ("omit_local_variable_types".to_string(), 2),
                    ("specify_nonobvious_local_variable_types".to_string(), 2),
                ],
            },
        ];
        let conflicts = detect_conflicting_diagnostics(&entries, 2);
        let code_filter: Vec<String> = vec![];

        let skip_fix = !conflicts.is_empty() && code_filter.is_empty();
        assert!(skip_fix);
    }

    #[test]
    fn test_fix_not_skipped_when_code_filter_set() {
        let entries = vec![
            DryRunFileEntry {
                path: "pkg/lib/a.dart".to_string(),
                fixes: vec![
                    ("omit_local_variable_types".to_string(), 4),
                    ("specify_nonobvious_local_variable_types".to_string(), 4),
                ],
            },
            DryRunFileEntry {
                path: "pkg/lib/b.dart".to_string(),
                fixes: vec![
                    ("omit_local_variable_types".to_string(), 2),
                    ("specify_nonobvious_local_variable_types".to_string(), 2),
                ],
            },
        ];
        let conflicts = detect_conflicting_diagnostics(&entries, 2);
        let code_filter = ["omit_local_variable_types".to_string()];

        let skip_fix = !conflicts.is_empty() && code_filter.is_empty();
        assert!(!skip_fix);
    }

    #[test]
    fn test_fix_not_skipped_when_no_conflicts() {
        let entries = vec![
            DryRunFileEntry {
                path: "pkg/lib/a.dart".to_string(),
                fixes: vec![("unused_import".to_string(), 3)],
            },
            DryRunFileEntry {
                path: "pkg/lib/b.dart".to_string(),
                fixes: vec![("deprecated_member_use".to_string(), 1)],
            },
        ];
        let conflicts = detect_conflicting_diagnostics(&entries, 2);
        let code_filter: Vec<String> = vec![];

        let skip_fix = !conflicts.is_empty() && code_filter.is_empty();
        assert!(!skip_fix);
    }

    #[test]
    fn test_dry_run_scan_struct_assembly() {
        let entries = vec![
            DryRunFileEntry {
                path: "pkg/lib/a.dart".to_string(),
                fixes: vec![("rule_a".to_string(), 2), ("rule_b".to_string(), 2)],
            },
            DryRunFileEntry {
                path: "pkg/lib/b.dart".to_string(),
                fixes: vec![("rule_a".to_string(), 3), ("rule_b".to_string(), 3)],
            },
        ];

        let scan = assemble_dry_run_scan(entries, 2);

        assert_eq!(scan.entries.len(), 2);
        assert_eq!(scan.codes.len(), 2);
        assert!(scan.codes.contains("rule_a"));
        assert!(scan.codes.contains("rule_b"));
        assert_eq!(scan.conflicts.len(), 1);
        assert_eq!(scan.conflicts[0].code_a, "rule_a");
        assert_eq!(scan.conflicts[0].code_b, "rule_b");
    }
}
