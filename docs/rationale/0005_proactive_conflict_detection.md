# 0005: Proactive Conflict Detection in Dry-Run

## Context

`melos-rs analyze --dry-run` runs `dart fix --dry-run` across workspace
packages, parses the output, and displays a consolidated view of proposed fixes.
When conflicting lint rules are both enabled (e.g. `omit_local_variable_types`
and `specify_nonobvious_local_variable_types`), `dart fix` proposes mirror-image
fixes that undo each other. Applying them creates a fix/analyze loop where the
codebase never reaches a clean state.

This was discovered in a real workspace using `package:all_lint_rules_community/all.yaml`,
which enables every Dart lint rule including conflicting pairs.

## Problem

Users running `melos-rs analyze --fix` see fixes applied, but `dart analyze`
still reports the same issues (now from the opposing rule). Running `--fix`
again reverts the changes. The cycle repeats indefinitely with no indication
of why.

Without tooling assistance, users must:
1. Notice the fix count is suspiciously symmetric across files
2. Research which lint rules conflict
3. Manually disable one in `analysis_options.yaml`

This is non-obvious even for experienced Dart developers.

## Decision

Detect conflicting lint rule pairs automatically from `dart fix --dry-run`
output using a heuristic, and emit a warning with actionable guidance.

### Heuristic

Two diagnostic codes are flagged as conflicting when they appear together in
the same file with **identical fix counts** across **2 or more files**. This
threshold was chosen because:

- **Equal counts per file** is the signature of mirror-image rules: if rule A
  wants to remove N type annotations, rule B wants to add N type annotations
  on the same variables
- **2+ files** filters out coincidental matches where two unrelated rules
  happen to have the same count in a single file
- The heuristic is **output-driven** with no hardcoded list of known
  conflicting pairs, so it works for any current or future Dart lint rules

### Implementation

```rust
fn detect_conflicting_diagnostics(
    entries: &[DryRunFileEntry],
    min_files: usize,
) -> Vec<ConflictingPair>
```

For each file entry, the function examines all diagnostic code pairs. When two
codes share an identical fix count, the pair is recorded. Pairs appearing in
`>= min_files` files are returned as conflicts.

The warning is emitted after the normal fix suggestion footer:

```
WARNING: Conflicting lint rules detected

  omit_local_variable_types and specify_nonobvious_local_variable_types conflict (13 files with equal fix counts)
  Disable one in analysis_options.yaml to avoid a fix/analyze loop:
    omit_local_variable_types: false
    specify_nonobvious_local_variable_types: false

Applying both conflicting fixes will undo each other, leaving the same warnings.
```

Both rules are shown as disable candidates. The tool does not prescribe which
one to keep because that depends on team preference.

## Alternatives Considered

### 1. Hardcoded conflict table

Maintain a list of known conflicting Dart lint pairs. Rejected because:
- Requires maintenance as new lint rules are added to the Dart SDK
- May miss conflicts from third-party lint packages
- The heuristic approach catches all cases without maintenance

### 2. Parse `analysis_options.yaml` directly

Read the lint configuration and check for known conflicts before running
`dart fix`. Rejected because:
- Requires resolving `include:` chains and package imports
- The `all_lint_rules_community` package resolves at `pub get` time, not
  statically readable from the YAML file
- Much more complex for the same outcome

### 3. No detection (status quo)

Let users figure it out. Rejected because the fix/analyze loop is confusing
and the heuristic is cheap to compute (O(entries * codes^2) where codes per
file is typically 1-3).

## Impact

| Scenario | Before | After |
|----------|--------|-------|
| Conflicting rules in workspace | Silent fix/analyze loop | Warning with disable suggestion |
| Non-conflicting rules | No change | No change (heuristic produces no false positives at threshold 2) |
| Runtime cost | N/A | Negligible — pair counting over parsed entries already in memory |
