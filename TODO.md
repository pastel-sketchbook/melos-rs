# melos-rs TODO

A Rust CLI replacement for [Melos](https://melos.invertase.dev/) - Flutter/Dart monorepo management tool.

## Parity Target

Tracking feature parity against **Melos 7.4.0** (latest stable as of 2026-02-22).

| Area | Melos 7.4.0 | melos-rs | Notes |
|------|-------------|----------|-------|
| Config: `melos.yaml` (6.x) | Yes | Yes | Full support |
| Config: `pubspec.yaml` (7.x) | Yes | Yes | `melos:` section parsing |
| `bootstrap` | Yes | Yes | Full support incl. `resolution: workspace` skip, dry-run |
| `clean` | Yes | Yes | Deep clean + hooks, dry-run |
| `exec` | Yes | Yes | Concurrency, fail-fast, watch, timeout, dry-run |
| `run` | Yes | Yes | Steps, exec config, watch, groups, private |
| `list` | Yes | Yes | All formats: long, json, parsable, graph, gviz, mermaid |
| `version` | Yes | Yes | Conventional commits, changelogs, git tags, release branches, dry-run |
| `publish` | Yes | Yes | Dry-run, git tags, release URLs |
| `format` | Yes | Yes | All flags |
| `analyze` | Yes | Yes | Fatal warnings/infos |
| `test` | Yes | Yes | Coverage, goldens, hooks |
| `pub get/upgrade/downgrade/add/remove` | Yes | Yes | All subcommands |
| `init` | Yes | Yes | 6.x and 7.x scaffolding |
| `completion` | Yes | Yes | bash/zsh/fish |
| `health` | N/A | Yes | melos-rs exclusive: version drift, missing fields, SDK consistency, JSON output |
| `resolution: workspace` | Yes | Yes | Skip `pubspec_overrides.yaml` for workspace-resolved packages |
| IDE integration | Yes (IntelliJ, VS Code) | No | Out of scope for CLI tool |
| Migration guide | Yes | Yes | `docs/migration-from-melos.md` (Batch 27) |

## Phase 1: Core Infrastructure (MVP)

- [x] Project scaffolding (Cargo.toml, module structure)
- [x] CLI argument parsing with `clap` (commands: exec, run, version, bootstrap, clean, list, format, publish)
- [x] `melos.yaml` config parsing (name, packages, scripts, command hooks)
- [x] Package discovery from glob patterns
- [x] Pubspec.yaml parsing (name, version, dependencies, flutter detection, publish_to)
- [x] Package filtering (flutter/dart, dir-exists, file-exists, depends-on, scope, ignore)
- [x] Process runner with configurable concurrency, fail-fast, and colored per-package output
- [x] Workspace environment variables (MELOS_ROOT_PATH, MELOS_PACKAGE_*)

## Phase 1.5: Global Filters

- [x] `GlobalFilterArgs` shared across all commands (flattened into each subcommand)
- [x] `--scope=<glob>` filter (glob matching on package names, multiple allowed)
- [x] `--ignore=<glob>` filter (glob exclusion on package names, multiple allowed)
- [x] `--diff=<ref>` / `--since=<ref>` filter (git-based change detection)
- [x] `--dir-exists=<path>` filter
- [x] `--file-exists=<path>` filter
- [x] `--flutter` / `--no-flutter` filter
- [x] `--depends-on=<pkg>` filter (multiple allowed)
- [x] `--no-depends-on=<pkg>` filter (multiple allowed)
- [x] `--no-private` filter (exclude publish_to: none packages)
- [x] `--category=<cat>` flag (resolved via `apply_filters_with_categories()` using `MelosConfig.categories`)
- [x] `--include-dependencies` transitive dependency expansion
- [x] `--include-dependents` transitive dependent expansion
- [x] `--published` / `--no-published` filter (publishable vs private packages)
- [x] `PackageFilters::merge()` for combining CLI + script-level filters
- [x] `Package.publish_to` field, `Package.is_private()` method
- [x] Top-level `ignore` config field (workspace-wide glob exclusions applied at package discovery)

## Phase 2: Command Implementations

### `exec` Command
- [x] Basic exec: run command in each package
- [x] `-c N` concurrency control (default 5, matching Melos)
- [x] `--fail-fast` flag
- [x] Global filters (`--scope`, `--ignore`, `--depends-on`, etc.) via flattened GlobalFilterArgs
- [x] Per-package env vars injected during exec (MELOS_PACKAGE_NAME, VERSION, PATH)
- [x] Colored per-package output prefixing (10 rotating colors)
- [x] `--order-dependents` flag (topological sort for dependency-ordered execution)

### `version` Command
- [x] Version bump types: build, patch, minor, major
- [x] Per-package overrides with `-V package:bump` syntax
- [x] `--all` flag to bump all packages
- [x] `--yes` flag to skip confirmation
- [x] Pre-commit hook execution from config
- [x] Post-commit hook execution from config
- [x] Pubspec.yaml version rewriting
- [x] Flutter `+buildNumber` format handling
- [x] CHANGELOG.md generation from conventional commits
- [x] Git tag creation after version bump (`--no-git-tag` to skip)
- [x] Branch validation (ensure on correct branch from config)
- [x] Conventional commit parsing (`--conventional-commits` flag)
- [x] Commit type to bump mapping (feat=minor, fix=patch, breaking=major)
- [x] Per-package commit mapping via git diff-tree
- [x] Workspace-level CHANGELOG.md aggregation
- [x] Configurable changelog options (include body, include commit ID)
- [x] Configurable commit message template
- [x] `--no-changelog` flag to skip changelog generation
- [x] `include_scopes` config wired to changelog generation
- [x] `link_to_commits` config wired to changelog commit ID inclusion
- [x] Interactive y/n confirmation prompt (replaces `--yes` requirement)
- [x] Pure-Rust date formatting (no shell-out to `date`)

### `run` Command
- [x] Basic run: execute named scripts from config
- [x] `melos run` -> `melos-rs run` self-reference expansion
- [x] `&&` chain splitting for sequential execution
- [x] Script-level `packageFilters` applied before exec
- [x] CLI global filters merged with script-level `packageFilters` via `merge()`
- [x] `$MELOS_ROOT_PATH` and env var substitution in commands (`$VAR` and `${VAR}`)
- [x] Interactive script selection when no script name given
- [x] `--no-select` flag support
- [x] Exec-style script detection and execution (parses `-c`, `--fail-fast`, `--` separator)

### `bootstrap` Command
- [x] Run `flutter pub get` / `dart pub get` in each package
- [x] Global filters support
- [x] Parallel bootstrapping with configurable concurrency (`-c N`)
- [x] Progress bar with `indicatif`
- [x] Link local package dependencies (pubspec_overrides.yaml for 6.x mode)
- [x] Config-driven `run_pub_get_in_parallel: false` forces sequential execution

### `clean` Command
- [x] Run `flutter clean` in Flutter packages
- [x] Clean build artifacts in pure Dart packages (build/, .dart_tool/)
- [x] Global filters support
- [x] `--deep` flag to also delete `.dart_tool/`, `build/`, `pubspec.lock`
- [x] Remove `pubspec_overrides.yaml` files (6.x mode cleanup)
- [x] Post-clean hook execution (`command.clean.hooks.post`)

### `list` Command
- [x] List all packages
- [x] `--long` flag for detailed output (shows private tag)
- [x] `--json` flag for machine-readable JSON output
- [x] Global filters support
- [x] `--parsable` format (name:version:path per line)
- [x] `--relative` flag (show relative paths)
- [x] `--format=graph` dependency graph as adjacency list
- [x] `--format=gviz` Graphviz DOT output
- [x] `--format=mermaid` Mermaid diagram output
- [x] `--cycles` flag for circular dependency detection (Kahn's algorithm)

### `format` Command
- [x] `dart format .` across all matching packages
- [x] `-c N` concurrency control
- [x] `--set-exit-if-changed` flag (CI mode)
- [x] `--output` flag (write, json, none)
- [x] `--line-length` flag
- [x] Global filters support

### `publish` Command
- [x] `dart pub publish` across non-private packages
- [x] `--dry-run` flag (default: true, safe by default)
- [x] `--git-tag-version` flag (creates annotated git tags after publish)
- [x] `--yes` flag to skip confirmation
- [x] `-c N` concurrency control
- [x] Automatic private package exclusion
- [x] Interactive y/n confirmation prompt

## Phase 2.5: Config Extensions

- [x] `VersionCommandConfig` with branch, message template, changelog options, hooks
- [x] `ChangelogConfig` with include_commit_body, include_commit_id
- [x] `BootstrapCommandConfig` with run_pub_get_in_parallel
- [x] `CleanCommandConfig` with hooks

## Phase 2.6: Dual Config Support (Melos 6.x + 7.x)

- [x] `ConfigSource` enum (`MelosYaml` vs `PubspecYaml`) in workspace.rs
- [x] Auto-detection: walk up searching for `melos.yaml` then `pubspec.yaml` with `melos:` key
- [x] `melos.yaml` preferred over `pubspec.yaml` when both exist (user hasn't migrated)
- [x] 7.x config parsing: extract `melos:` section from pubspec.yaml
- [x] 7.x name resolution: `melos.name` override or fall back to pubspec `name`
- [x] 7.x package paths: `melos.packages` or fall back to `workspace:` field
- [x] `Workspace.config_source` field for downstream mode decisions
- [x] Bootstrap: generate `pubspec_overrides.yaml` in 6.x mode only
- [x] Clean: remove `pubspec_overrides.yaml` in 6.x mode only
- [x] Startup banner shows config mode (`[melos.yaml]` or `[pubspec.yaml]`)
- [x] Actionable error message when no config found
- [x] `melos-rs init` — scaffold new 7.x workspace (with `--legacy` flag for 6.x)

## Phase 3: Advanced Features

### Script Execution Engine
- [x] Full `melos exec` flag parsing when invoked from `run` command (unified `ExecFlags` struct)
- [x] Script-level `env` field support (merged into run command environment)
- [x] Recursive `melos run X` expansion (handle nested script references, cycle detection, max depth 16)
- [x] Timeout support for long-running commands (`--timeout <seconds>` on exec)
- [x] Dry-run mode (`--dry-run` on exec)
- [x] Script `exec:` object syntax (`ExecEntry` enum: string shorthand vs options with concurrency/fail-fast/order-dependents)
- [x] Script `steps:` multi-step sequential workflows
- [x] Script `private` field (hidden from interactive selection and `--list`)
- [x] `run --list` / `run --list --json` (show available scripts)
- [x] `--include-private` flag for `run --list`

### Package Management
- [x] Topological sort for dependency-ordered execution (Kahn's algorithm, wired to bootstrap + exec)
- [x] Circular dependency detection (`list --cycles`)
- [x] Category-based package filtering (`categories` config field)
- [x] Workspace-level pubspec overrides (pubspec_overrides.yaml for 6.x local linking)
- [x] `pub get`, `pub outdated`, `pub upgrade` subcommands (groups by SDK, `--major-versions` flag)

### Versioning & Release
- [x] `version:set` command (works via `melos-rs version 2.0.0 --all`)
- [x] Coordinated versioning (`--coordinated` flag / `command.version.coordinated` config)
- [x] Git push after version bump (`--no-git-push` flag / `command.version.gitPush` config)
- [x] Prerelease versioning (`--prerelease`/`-p`, `--preid`, `--dependent-preid`)
- [x] Graduate prerelease to stable (`--graduate`/`-g`)
- [x] Dependent constraint auto-updates (`--dependent-constraints`, `--dependent-versions`)
- [x] Custom commit message (`--message`/`-m` with `{new_package_versions}` placeholder)
- [x] Repository config for changelog commit links (`repository:` URL string or object form)
- [x] `fetchTags` config option (run `git fetch --tags` before conventional commit analysis)
- [x] Aggregate `changelogs` config (multiple changelogs with path, packageFilters, description; type filtering)
- [x] `--release-url` / `-r` flag for version command (prefilled GitHub release creation page links)
- [x] `changelogCommitBodies` config (include/onlyBreaking options for commit body inclusion)
- [x] `changelogFormat.includeDate` config (optional date in version headers, default false)
- [x] `updateGitTagRefs` config (update git dependency `ref:` tags in pubspec.yaml)
- [x] Release branch management (auto-create release branches during `version`)

### Developer Experience
- [x] `melos-rs init` - scaffold new 7.x workspace (with `--legacy` for 6.x)
- [x] Tab completion for bash/zsh/fish (`completion` subcommand via `clap_complete`)
- [x] Progress bars with `indicatif` for more commands _(see "Known Gaps" section below)_
- [x] Verbose/quiet log levels (`--verbose` / `--quiet` global flags)
- [x] Config validation and helpful warning messages
- [x] Watch mode for development (`--watch`) _(see "Known Gaps" section below)_

## Batch 14: Bootstrap Maturity & Version Polish

- [x] `--enforce-lockfile` CLI flag for bootstrap (pass through to `pub get`)
- [x] Bootstrap lifecycle hooks (pre/post hooks, matching clean/version pattern)
- [x] Clean pre-hook support (`command.clean.hooks.pre`)
- [x] `--no-example` / `--offline` flags for bootstrap (pass through to `pub get`)
- [x] `fetchTags` config option for version command (`git fetch --tags` before analysis)
- [x] Changelog commit type filtering (include/exclude conventional commit types)
- [x] `bs` alias for bootstrap command

## Batch 15: Version Command Polish

- [x] `--release-url` / `-r` flag (generate prefilled GitHub release creation page links)
- [x] Aggregate `changelogs` config (multiple changelogs with `path`, `packageFilters`, `description`)
- [x] `changelogCommitBodies` config (`include` + `onlyBreaking` options for commit body in changelogs)
- [x] `changelogFormat.includeDate` config (optional date in changelog version headers, default false)
- [x] `updateGitTagRefs` config (scan pubspec.yaml git deps and update `ref:` tags to new versions)

## Batch 16: Melos Parity Gaps

- [x] `analyze` command (`dart analyze` across packages with `--fatal-warnings`, `--fatal-infos`, `--no-fatal`, `-c` concurrency)
- [x] `run --group` + script `groups` field (filter scripts by group in selection and `--list`)
- [x] Script overriding built-in commands (scripts with same name as commands take precedence, except `run`/`init`/`completion`)
- [x] `sdkPath` config + `--sdk-path` global flag + `MELOS_SDK_PATH` env var (CLI > env > config priority)
- [x] Publish hooks (pre/post) via `command.publish.hooks` config, `MELOS_PUBLISH_DRY_RUN` env var
- [x] `MELOS_PACKAGES` env var (comma-delimited scope override applied in `apply_filters_with_categories`)

## Known Gaps & Dead Code

### Dead code (parsed but not wired)
- [x] `BootstrapCommandConfig::enforce_versions_for_dependency_resolution` — parsed from config, has `#[allow(dead_code)]` at `src/config/mod.rs:669`. Needs to be consulted during bootstrap dependency resolution.

### Missing commands / features
- [x] Release branch management (auto-create release branches during `version`) — done in Batch 24
- [x] Progress bars with `indicatif` for more commands (only bootstrap has one currently)
- [x] Watch mode for development (`--watch` flag on exec/run)

## Phase 4: Parity & Beyond

- [x] Full Melos CLI flag compatibility audit
- [x] Migration guide from Melos to melos-rs (`docs/migration-from-melos.md`)
- [x] Performance benchmarks vs Melos (68x list, 69x list --json, 19x exec)
- [x] Monorepo health checks (unused deps, version drift)

## Batch 17: Health, Progress & CLI Parity

- [x] Wire `enforce_versions_for_dependency_resolution` in bootstrap (dead code → functional)
  - Added `dependency_versions: HashMap<String, String>` field to `Package` struct
  - Added `extract_version_constraint()` helper for YAML value parsing
  - `enforce_versions()` validates workspace sibling version constraints using `semver`
  - Removed `#[allow(dead_code)]` from config field
- [x] Add progress bars (`indicatif`) to exec, clean, format, analyze, publish
  - Added `create_progress_bar()` helper in `runner/mod.rs` for consistent style
  - Used `run_in_packages_with_progress()` for real-time progress tracking
  - Refactored bootstrap to use shared helper
- [x] Melos CLI flag compatibility audit
  - Fixed format `--concurrency` default: 5 → 1 (matching Melos)
  - Added `--no-enforce-lockfile` flag to bootstrap
  - Added list shorthand flags: `-r` (relative), `-p` (parsable), `--graph`, `--gviz`, `--mermaid`
  - Renamed version `--no-git-tag` to `--no-git-tag-version` (with alias for backward compat)
  - Added version `--changelog / -c` positive toggle and `--git-tag-version / -t`
- [x] Monorepo health checks command (`melos-rs health`)
  - `--version-drift`: detects same external dep at different constraint versions
  - `--missing-fields`: checks public packages for description, homepage/repository, version
  - `--sdk-consistency`: checks Dart/Flutter SDK constraints are consistent across packages
  - `--all / -a`: runs all checks (default if none selected)
  - Includes filtering support via `GlobalFilterArgs`

## Batch 18: Watch Mode

- [x] File watcher module (`src/watcher/mod.rs`) using `notify` + `notify-debouncer-mini`
  - Watches package directories recursively for `.dart`, `.yaml`, `.json`, `.arb`, `.g.dart` files
  - Filters out build artifacts (`.dart_tool/`, `build/`, `.symlinks/`, `.fvm/`, IDE dirs)
  - 500ms debounce window to coalesce rapid changes
  - Identifies which package owns each changed file
  - Graceful shutdown via channel signal
- [x] `--watch` flag on `exec` command
  - Runs command initially across all matched packages
  - Watches for file changes and re-runs only in affected packages
  - In watch mode, failures are reported but don't stop watching
  - Ctrl+C cleanly stops the watcher
- [x] `--watch` flag on `run` command
  - Runs named script initially, then watches for changes
  - Watches packages matching script's `packageFilters` (or all packages)
  - Re-runs the entire script on any change
  - Works with all script modes: steps, exec config, and shell commands
- [x] `PackageFilters::is_empty()` helper for detecting unfiltered state
- [x] Tests: 29 new tests (watcher unit tests, integration tests, CLI flag parsing, filter tests)

## Batch 19: Melos Config Parity

- [x] Parent package environment variables for example packages
  - Added `find_parent_package()` in runner that finds the deepest parent package containing an example
  - `build_package_env()` now accepts `all_packages` and sets `MELOS_PARENT_PACKAGE_NAME`, `MELOS_PARENT_PACKAGE_VERSION`, `MELOS_PARENT_PACKAGE_PATH`
  - Updated `run_in_packages()` and `run_in_packages_with_progress()` signatures across all 12 call sites
- [x] `dependencyOverridePaths` bootstrap config
  - Added `dependency_override_paths: Option<Vec<String>>` to `BootstrapCommandConfig`
  - `generate_pubspec_overrides()` scans override paths for packages (single dir or immediate subdirs)
  - Adds discovered packages as `dependency_overrides` entries alongside workspace siblings
- [x] Progress bars for pub commands (`pub get`/`pub upgrade`/`pub downgrade`)
  - Updated `run_pub_in_packages()` to use `create_progress_bar` and `run_in_packages_with_progress`
- [x] `runPubGetOffline` bootstrap config
  - Added `run_pub_get_offline: Option<bool>` to `BootstrapCommandConfig`
  - Wired into bootstrap: `let offline = args.offline || config_run_pub_get_offline(workspace);`
- [x] `useRootAsPackage` config
  - Added `use_root_as_package: Option<bool>` to `MelosConfig` and `MelosSection`
  - Wired into `Workspace::find_and_load()` — includes root dir as a package via `Package::from_path()`
  - Propagated from `MelosSection` in the `from_pubspec_yaml` constructor
- [x] Tests: 17 new tests (parent env vars, dependency override paths, offline config, override path helpers)

## Batch 20: Quality & Polish

- [x] Error handling hardening
  - Replaced `sem.acquire().await.unwrap()` with documented `.expect()` in runner (semaphore never closed)
  - Replaced `.expect("commits should be loaded")` with `.ok_or_else(|| anyhow!(...))` in version command
  - Added safety comments on regex `caps.get(0).expect()` calls in run command
- [x] Dead code cleanup
  - Removed dead `"bs"` and `"run"` entries from `OVERRIDABLE_COMMANDS` constant (clap resolves aliases before our code; `run` is never overridden)
  - Added documentation comment explaining exclusion rationale
- [x] Integration test suite (`tests/cli.rs`) using `assert_cmd` + `predicates`
  - Added `assert_cmd` and `predicates` dev-dependencies
  - Fixture workspace helper `create_fixture_workspace()` builds temp dirs with `melos.yaml` + packages
  - `test_help_output` — `--help` exits 0 with expected subcommands
  - `test_version_flag` — `--version` shows binary name
  - `test_no_workspace_error` — running in empty dir exits 1 with actionable error
  - `test_init_creates_7x_workspace` — `init` scaffolds 7.x workspace with pubspec.yaml
  - `test_init_legacy_creates_melos_yaml` — `init --legacy` scaffolds 6.x workspace with melos.yaml
  - `test_list_packages` — `list` shows discovered packages
  - `test_list_json_output` — `list --json` outputs valid JSON array with correct fields
  - `test_list_parsable_output` — `list --parsable` outputs `name:version:path` format
  - `test_list_graph_output` — `list --graph` shows dependency adjacency
  - `test_exec_echo` — `exec -- echo hello` runs in each package
  - `test_exec_dry_run` — `exec --dry-run` prints commands without executing
  - `test_completion_bash` — `completion bash` generates shell completions
  - `test_health_check_no_issues` — `health --version-drift` reports no issues on clean workspace
  - `test_list_with_scope_filter` — `list --scope` correctly filters packages
- [x] Tests: 14 new integration tests (286 total: 272 unit + 14 integration)

## Batch 21: Melos Parity Features

- [x] `pub downgrade` subcommand
  - Added `Downgrade(PubDowngradeArgs)` variant to `PubCommand` enum
  - Added `PubDowngradeArgs` struct with concurrency + filters
  - Added `run_pub_downgrade()` function following `run_pub_upgrade()` pattern
  - Wired into `pub` command dispatch
- [x] `test` command
  - Created `src/commands/test.rs` with `TestArgs` struct
  - Supports: `--coverage`, `--fail-fast`, `--test-randomize-ordering-seed`, `--no-run`, `-c` concurrency, extra args via `--`
  - Filters packages to only those with a `test/` directory
  - Uses flutter/dart SDK detection per package
  - Progress bar with `indicatif`
  - Wired into `cli.rs`, `commands/mod.rs`, `main.rs` (including `OVERRIDABLE_COMMANDS` and `get_overridable_command_name`)
- [x] Version command filter support
  - Added `#[command(flatten)] pub filters: GlobalFilterArgs` to `VersionArgs`
  - Added filtering step at start of `run()` that creates `eligible_packages`
  - Applied filters to all 5 package-selection branches (graduate, coordinated, overrides, conventional_commits, --all)
  - Kept `workspace.packages` for dependent constraint updates
- [x] Bootstrap shared dependencies
  - Added `environment`, `dependencies`, `dev_dependencies` fields to `BootstrapCommandConfig`
  - Implemented `sync_shared_dependencies()` — reads each package's pubspec.yaml, updates matching deps to shared version constraints
  - Uses line-level YAML manipulation (`sync_yaml_section`) to preserve comments and formatting
  - `yaml_value_to_constraint()` converts shared dep values (string, null) to constraint strings
  - Integrated into bootstrap flow (runs after enforce_versions, before pub get)
- [x] `discoverNestedWorkspaces` config option
  - Added `discover_nested_workspaces: Option<bool>` to `MelosConfig` and `MelosSection`
  - Implemented `discover_nested_workspace_packages()` — scans packages with `workspace:` field in pubspec.yaml
  - Recursively discovers packages from nested workspace paths
  - Deduplicates by name, re-sorts after adding nested packages
  - Wired into `Workspace::find_and_load()` after initial discovery
- [x] Tests: 18 new unit tests (304 total: 290 unit + 14 integration)

### Batch 22: Test Coverage

- [x] `analyze` command unit tests (6 tests)
  - Extracted `build_analyze_command()` from inline logic in `run()`
  - Tests: default command, `--fatal-warnings`, `--fatal-infos`, both fatal, `--no-fatal` override, `--no-fatal` alone
- [x] `format` command unit tests (7 tests)
  - Extracted `build_format_command()` from inline logic in `run()`
  - Tests: default command, `--set-exit-if-changed`, `--output=json`, `--output=none`, `--line-length`, all flags combined, write output not added
- [x] `clean` command unit tests (6 tests)
  - Tests: `DEEP_CLEAN_DIRS` constant, `DEEP_CLEAN_FILES` constant, `remove_pubspec_overrides` removes existing files, no-op when no file, multiple packages (partial), empty packages list
- [x] `publish` command unit tests (5 tests)
  - Extracted `build_publish_command()` and `build_git_tag()` from inline logic in `run()`
  - Tests: dry-run command, real publish command, git tag format, prerelease tag, zero version tag
- [x] Integration tests (6 tests)
  - `test_clean_dart_packages` — verifies build/ dirs removed for pure Dart packages
  - `test_clean_deep` — verifies `--deep` removes `.dart_tool/`, `build/`, `pubspec.lock`
  - `test_exec_with_scope_filter` — exec with `--scope` runs only in matched package
  - `test_list_with_ignore_filter` — list with `--ignore` excludes matched package
  - `test_list_with_no_private_filter` — list with `--no-private` excludes `publish_to: none`
  - `test_init_7x_with_apps` — init with apps directory accepted creates both packages/ and apps/
- [x] Tests: 30 new tests (334 total: 314 unit + 20 integration)

### Batch 23: Runner Output Buffering, Version Auto-Detect, Missing Commands & Cross-Platform

- [x] Theme A: Runner output buffering
  - Changed `Stdio::inherit()` → `Stdio::piped()` in `runner/mod.rs`
  - Added `output_lock: Arc<std::sync::Mutex<()>>` for atomic output printing
  - Captures `(success, stdout_buf, stderr_buf)` tuple per package
  - Prints all buffered output lines with package prefix under lock after completion
  - Prevents interleaved output in concurrent mode
- [x] Theme B: Version command auto-detect since-ref
  - Added `find_latest_git_tag(root: &Path) -> Option<String>` using `git describe --tags --abbrev=0`
  - Changed `since_ref` from `String` with default `"HEAD~10"` to `Option<String>`
  - Resolution chain: CLI flag → latest git tag → `"HEAD~10"` fallback
- [x] Theme C1: `pub add` / `pub remove` subcommands
  - Added `Add(PubAddArgs)` and `Remove(PubRemoveArgs)` variants to `PubCommand` enum
  - `PubAddArgs`: package name, `--dev` flag, concurrency, filters
  - `PubRemoveArgs`: package name, concurrency, filters
  - `build_pub_add_command()` and `build_pub_remove_command()` helpers
  - Wired into `pub` command dispatch
- [x] Theme C2: `--update-goldens` flag for test command
  - Added `#[arg(long)] pub update_goldens: bool` to `TestArgs`
  - Wired into `build_extra_flags()` to append `--update-goldens`
- [x] Theme C3: Test command hooks (pre/post) + `TestCommandConfig`
  - Added `TestCommandConfig` and `TestHooks` structs in `config/mod.rs`
  - Added `test: Option<TestCommandConfig>` to `CommandConfig`
  - Added pre-test and post-test hook execution in `test.rs` (following `clean.rs` pattern)
  - Hooks run once per `melos test` invocation (before/after all packages)
- [x] Theme D: Cross-platform shell support
  - Added `pub fn shell_command() -> (&'static str, &'static str)` in `runner/mod.rs`
  - Returns `("cmd", "/C")` on Windows, `("sh", "-c")` on Unix
  - Updated all 9 call sites: `runner/mod.rs`, `version.rs` (2), `clean.rs` (2), `publish.rs` (2), `bootstrap.rs` (1), `run.rs` (2)
- [x] Tests: 13 new tests (347 total: 327 unit + 20 integration)
  - `test_shell_command_returns_platform_appropriate_values`
  - `test_find_latest_git_tag_no_repo`, `test_find_latest_git_tag_no_tags`, `test_find_latest_git_tag_with_tag`
  - `test_build_pub_add_command_regular`, `test_build_pub_add_command_dev`, `test_build_pub_add_command_with_version`, `test_build_pub_remove_command`
  - `test_build_extra_flags_update_goldens_only`, `test_build_test_command_with_update_goldens`
  - `test_parse_test_config_with_hooks`, `test_parse_test_config_pre_only`, `test_parse_test_config_absent`

### Batch 24: Performance & Release Management
- [x] Theme A: Parallel package discovery with rayon
  - Added `rayon` dependency to `Cargo.toml`
  - Refactored `discover_packages()` in `package/mod.rs` to two phases:
    1. Sequential glob iteration to collect candidate directories (fast)
    2. Parallel `Package::from_path()` via `rayon::par_iter()` for pubspec parsing
  - Deterministic output preserved via post-discovery sort by name
- [x] Theme B: Release branch management in version command
  - Added `releaseBranch` config field to `VersionCommandConfig` (pattern string with `{version}` placeholder)
  - Added `release_branch_pattern()` accessor method
  - Added `--release-branch <pattern>` CLI flag to `VersionArgs` (overrides config)
  - Added `--no-release-branch` CLI flag (disables even if configured)
  - Added helper functions: `create_release_branch()`, `push_release_branch()`, `git_checkout()`, `git_current_branch()`
  - Wired into `run()`: after push, creates release branch from HEAD, pushes if push is enabled, switches back to original branch
- [x] Theme C: Release URL support in publish command
  - Added `--release-url` / `-r` flag to `PublishArgs`
  - After successful publish (non-dry-run), prints prefilled GitHub release creation page links
  - Reuses `RepositoryConfig::release_url()` from config module
  - Warns when `--release-url` is used without `repository` in config
- [x] Tests: 9 new tests (356 total: 336 unit + 20 integration)
  - `test_parse_release_branch_config`, `test_parse_release_branch_default_none`, `test_parse_release_branch_custom_pattern`
  - `test_create_release_branch_in_git_repo`, `test_create_release_branch_custom_pattern`
  - `test_git_checkout_back_to_original`, `test_release_branch_pattern_no_placeholder`
  - `test_release_url_format_matches_tag`, `test_release_url_prerelease_tag`

### Batch 25: Workspace Resolution Support (Dart 3.5+)
- [x] Theme A: `resolution: workspace` support in Package model
  - Added `resolution: Option<String>` field to `PubspecYaml` (serde deserialization)
  - Added `resolution: Option<String>` field to `Package` struct
  - Wired `resolution` from `pubspec.resolution` into `Package::from_path()`
  - Added `Package::uses_workspace_resolution()` method (case-insensitive check for "workspace")
- [x] Theme B: Bootstrap compatibility with workspace-resolved packages
  - Updated `generate_pubspec_overrides()` to skip packages with `resolution: workspace`
  - Updated bootstrap `run()` to skip override generation entirely when ALL packages use workspace resolution, with info message
  - Fixes `pubspec_overrides.yaml` conflict: Dart 3.5+ rejects overrides for workspace-resolved packages
- [x] Theme C: Melos 7.4.0 parity tracking
  - Added feature-by-feature parity comparison table to TODO.md
  - Tracks coverage status for all Melos 7.4.0 features (commands, config, IDE integration, etc.)
- [x] Housekeeping: Updated 20 Package constructor sites across 8 test files to include `resolution: None`
  - `bootstrap.rs` (6 sites), `filter.rs` (10 sites), `clean.rs`, `health.rs`, `list.rs`, `pub_cmds.rs` (2), `version.rs` (2), `runner/mod.rs`, `watcher/mod.rs` (2)
- [x] Tests: 7 new tests (363 total: 343 unit + 20 integration)
  - `test_pubspec_resolution_field_parsed`, `test_pubspec_resolution_field_absent`, `test_pubspec_resolution_case_insensitive`
  - `test_uses_workspace_resolution_true`, `test_uses_workspace_resolution_false`
  - `test_generate_overrides_skips_workspace_resolution`, `test_bootstrap_skips_overrides_when_all_workspace_resolution`

### Batch 26: GoF Design Patterns (LOC-reducing only)
- [x] Analysis: Evaluated 5 GoF pattern candidates, applied 2 that reduce LOC
  - Full rationale documented in `docs/rationale/0003_gof_patterns.md`
  - 9 GoF patterns already in use idiomatically (Iterator, Builder, Strategy, etc.)
  - 3 candidates deferred (Observer +29 LOC, Strategy +50 LOC, Chain of Responsibility +25 LOC)
- [x] Theme A: Template Method — `Workspace::hook()` (-44 LOC)
  - Added `hook(&self, command: &str, phase: &str) -> Option<&str>` to `Workspace`
  - Centralizes 4-level Option chain: config.command → command_config → hooks → phase
  - Supports `"bootstrap"`, `"clean"`, `"test"`, `"publish"` commands with `"pre"`/`"post"` phases
  - Replaced 8 hook extraction call sites across 4 files (bootstrap.rs ×2, clean.rs ×2, test.rs ×2, publish.rs ×2)
  - Each site reduced from 5-7 lines to 1 line
  - Version hooks excluded (use `pre_commit`/`post_commit` fields, different pattern)
- [x] Theme B: Builder — `make_changelog_opts` closure factory (-15 LOC)
  - Replaced 3 identical 10-line `ChangelogOptions` struct constructions in `version.rs`
  - Closure captures 8 shared locals and returns fresh `ChangelogOptions` at each call site
- [x] Tests: 8 new tests (371 total: 351 unit + 20 integration)
  - `test_hook_no_command_config`, `test_hook_bootstrap_pre`, `test_hook_clean_post`
  - `test_hook_test_both`, `test_hook_publish_pre`, `test_hook_unknown_command`
  - `test_hook_unknown_phase`, `test_hook_no_hooks_configured`
- [x] Benchmark: melos-rs list 67.92x faster than melos list (7.6ms vs 518.2ms)

### Batch 27: Housekeeping & Migration Guide
- [x] Fix stale TODO.md entries
  - Parity table: `bootstrap` updated from "Partial" to "Yes" (completed in Batch 25)
  - Parity table: removed `build:android/ios` row (low priority, out of scope)
  - Phase 3: checked release branch management (completed in Batch 24)
  - Phase 3: clarified `build:android/ios` as out of scope
  - Phase 4: checked performance benchmarks (68x list, 69x list --json, 19x exec)
  - Known Gaps: marked release branch as done
- [x] Migration guide from Melos to melos-rs (`docs/migration-from-melos.md`)
  - Covers: binary name swap, config compatibility, script auto-adaptation
  - Documents behavioral differences (publish dry-run default, format concurrency, init format)
  - Lists features only in melos-rs (health, watch, deep clean, cycle detection, mermaid)
  - Lists features only in Melos (IDE integration)
  - CI/CD migration examples
- [x] README.md created (Batch 26)
- [x] `task check:all` passes — 371 tests, zero clippy warnings

### Batch 28: CLI UX — dry-run, JSON output, standardized summaries
- [x] Theme A: `--dry-run` for bootstrap, clean, version commands
  - Added `dry_run: bool` to `BootstrapArgs` and `CleanArgs` in `cli.rs`
  - Added `dry_run: bool` to `VersionArgs` in `version.rs`
  - Bootstrap dry-run: shows package list + "DRY RUN" message, skips hooks/pub get/overrides
  - Clean dry-run: shows package list + deep clean info + "DRY RUN" message, skips hooks/deletion
  - Version dry-run: shows version change plan + "DRY RUN" message, skips confirmation/writes/git
  - Pattern follows exec.rs reference (early return after package listing)
- [x] Theme B: `--json` on health command
  - Added `json: bool` flag to `HealthArgs`
  - Defined serializable result types: `HealthReport`, `VersionDriftIssue`, `MissingFieldsIssue`, `SdkConsistencyResult`, `ConstraintUsage`
  - Refactored check functions to separate data collection from presentation
  - `collect_version_drift()`, `collect_missing_fields()`, `collect_sdk_consistency()` return structured data
  - `print_version_drift()`, `print_missing_fields()`, `print_sdk_consistency()` render human-readable output
  - JSON mode: serializes `HealthReport` with `serde_json::to_string_pretty()`, skips all println
  - Null fields omitted via `#[serde(skip_serializing_if = "Option::is_none")]`
  - Empty arrays omitted via `#[serde(skip_serializing_if = "Vec::is_empty")]`
- [x] Theme C: Standardized result summaries across all commands
  - Gold standard: `test.rs` pattern with `passed`/`failed` counts
  - `analyze.rs`: `"{failed} package(s) failed analysis ({passed} passed)"` / `"All {passed} package(s) passed analysis."`
  - `format.rs`: `"{failed} package(s) failed/have formatting changes ({passed} passed)"` / `"All {passed} package(s) passed formatting."`
  - `exec.rs`: `"{failed} package(s) failed exec ({passed} passed)"` / `"All {passed} package(s) passed exec."`
  - `publish.rs`: `"{failed} package(s) failed to publish ({passed} passed)"` / `"All {n} package(s) {validated/published}."`
  - `pub_cmds.rs`: `"{failed} package(s) failed ({passed} passed)"` / `"All {passed} package(s) succeeded."`
  - `bootstrap.rs`: `"All {n} package(s) bootstrapped."` (fail-fast, no aggregate count)
  - `clean.rs`: `"{failed} package(s) failed cleaning ({passed} passed)"` / `"All {n} package(s) passed cleaning."`
- [x] Tests: 7 new tests (378 total: 358 unit + 20 integration)
  - `test_version_drift_json_serializable`, `test_missing_fields_json_serializable`
  - `test_sdk_consistency_json_serializable`, `test_health_report_json_serializable`
  - `test_collect_missing_fields_skips_private`, `test_collect_sdk_consistency_missing`
  - `test_build_sorted_usages_deterministic`
- [x] `task check:all` passes — 378 tests, zero clippy warnings

## Batch 29 — Integration tests for Batch 28 features (6 tests)

End-to-end CLI tests for dry-run modes, health --json, and exec summary.

- [x] `test_bootstrap_dry_run` — bootstrap --dry-run shows packages + "DRY RUN", does not generate pubspec_overrides.yaml
- [x] `test_clean_dry_run` — clean --dry-run shows "DRY RUN", build artifacts remain on disk
- [x] `test_version_dry_run` — version --dry-run --all shows "Version changes:" plan + "DRY RUN", pubspec versions unchanged
- [x] `test_health_json_no_issues` — health --json --version-drift outputs valid JSON with total_issues: 0
- [x] `test_health_json_with_drift` — health --json --version-drift detects drift, exits non-zero, JSON has VersionDriftIssue entries
- [x] `test_exec_success_summary` — exec outputs standardized "All 2 package(s) passed exec." message
- [x] `task check:all` passes — 384 tests (358 unit + 26 integration), zero clippy warnings

## Batch 30 — Fix package discovery excluding artifact directories (14 tests)

Real-world bug: `discover_packages()` with `packages/**` glob descended into `.dart_tool`,
`.symlinks`, `.pub-cache`, and `build` directories, discovering cached dependency pubspec.yaml
files (e.g. `path_provider_windows`, `url_launcher_example`) as workspace packages.

- [x] Added `EXCLUDED_PACKAGE_DIRS` constant with 9 artifact directory names matching real Melos
      `_commonIgnorePatterns` (`.dart_tool`, `.symlinks`, `.plugin_symlinks`, `.pub-cache`, `.pub`,
      `.fvm`, `build`, `.idea`, `.vscode`)
- [x] Added `is_in_excluded_dir()` helper that checks if any path component is in the exclusion set
- [x] Updated `discover_packages()` to skip excluded directories before checking for pubspec.yaml
- [x] Tests: 14 new tests (405 total: 379 unit + 26 integration)
  - 10 unit tests for `is_in_excluded_dir()` covering all excluded dirs, normal paths, nested paths
  - 4 integration tests for `discover_packages()` exclusion: `.dart_tool`, `.symlinks`, `build`,
    and combined multiple artifact directories
- [x] `task check:all` passes — 405 tests, zero clippy warnings

## Batch 31 — Strip outer quotes from exec commands in run scripts (v0.2.3)

Real-world bug: When a script has `run: melos exec -- "flutter pub upgrade && exit"`,
the double quotes are literal YAML characters. `extract_exec_command()` uses `split_whitespace()`
which doesn't understand quoting, so after joining parts after `--`, the result is
`"flutter pub upgrade && exit"` with literal quote chars. When passed to `sh -c`, the shell
treats the double-quoted content as a single word (command name), causing
`sh: flutter pub upgrade && exit: command not found`.

- [x] Added `strip_outer_quotes()` helper in `run.rs` that removes matching outer `"` or `'`
      from extracted commands, with `len >= 2` guard to prevent panics on single-char strings
- [x] Updated `extract_exec_command()` to call `strip_outer_quotes()` on both the `--` path
      and the fallback path
- [x] Tests: 8 new tests (413 total: 387 unit + 26 integration)
  - 5 unit tests for `strip_outer_quotes()`: double quotes, single quotes, mismatched quotes,
    no quotes, empty string
  - 3 unit tests for `extract_exec_command()` with quoted commands: double-quoted after `--`,
    single-quoted after `--`, quoted fallback without `--`
- [x] `task check:all` passes — 413 tests, zero clippy warnings

## Batch 32 — Parse --file-exists flag from inline exec commands (v0.2.4)

Real-world bug: When a YAML script has `run: melos exec --file-exists="pubspec.yaml" -c 1 --fail-fast -- "cmd"`,
the `--file-exists` flag was silently ignored by `parse_exec_flags()` (fell through to `_ => {}`).
This meant the command ran in ALL packages instead of only those containing `pubspec.yaml`.

- [x] Added `file_exists: Option<String>` field to `ExecFlags` struct
- [x] Updated `parse_exec_flags()` to recognize `--file-exists=<value>` (equals form) and
      `--file-exists <value>` (space-separated form), with quote stripping for quoted values
- [x] Updated `run_exec_script()` to apply the parsed `file_exists` filter to the package set
      before execution (lowest priority: packageFilters and CLI filters take precedence)
- [x] Updated `extract_exec_command()` fallback path to strip `--file-exists` flags from the
      command string
- [x] Tests: 7 new tests (420 total: 394 unit + 26 integration)
  - 5 unit tests for `parse_exec_flags()`: equals-quoted, equals-unquoted, space-separated,
    single-quoted, absent (none)
  - 2 unit tests for `extract_exec_command()` fallback: equals form stripped, space form stripped
- [x] `task check:all` passes — 420 tests, zero clippy warnings

## Batch 33 — Build command config parsing & CLI wiring (v0.3.0, Batch A)

Beyond-Melos-parity feature: `melos-rs build` replaces 20-30 duplicated build scripts with a
single declarative config block + CLI interface. This batch implements config parsing, CLI args,
and full command wiring.

- [x] Bumped `Cargo.toml` version to `0.3.0`
- [x] Created `docs/rationale/0004_build_command.md` (detailed rationale document)
- [x] Added config structs to `src/config/mod.rs`:
  - `BuildCommandConfig` (flavors, defaultFlavor, android, ios, packageFilters, hooks)
  - `FlavorConfig` (target, mode)
  - `BuildMode` enum (Release, Debug, Profile) with Display impl
  - `AndroidBuildConfig` (types, defaultType, extraArgs, simulator)
  - `IosBuildConfig` (extraArgs, simulator)
  - `SimulatorConfig` (enabled, command)
  - `BuildHooks` (pre, post)
  - Wired `build: Option<BuildCommandConfig>` into `CommandConfig`
- [x] Created `src/commands/build.rs` (706 lines) with:
  - `BuildArgs` struct (clap derive) with all CLI flags
  - `Platform` enum (Android, Ios) with Display, dir_name(), default_build_type()
  - `build_flutter_command()` — assembles flutter build command string
  - `resolve_platforms()`, `resolve_flavors()`, `resolve_android_build_type()` helpers
  - `run()` async function — full build execution with filter merging, platform iteration,
    flavor iteration, hooks, dry-run, fail-fast
- [x] Wired build command into CLI:
  - `pub mod build` in `src/commands/mod.rs`
  - `Build(BuildArgs)` variant in `Commands` enum in `src/cli.rs`
  - `"build"` in `OVERRIDABLE_COMMANDS` and dispatch in `src/main.rs`
  - `"build"` arm in `get_overridable_command_name()` in `src/main.rs`
  - `"build"` arm in `Workspace::hook()` in `src/workspace.rs`
- [x] Tests: 25 new tests (445 total: 419 unit + 26 integration)
  - 5 command assembly: Android prod release, Android QA debug, iOS prod release,
    profile mode, extra args
  - 5 platform resolution: default is all, android only, ios only, all flag, both explicit
  - 6 flavor resolution: explicit, multiple, unknown errors, default from config,
    single available, multiple no default errors
  - 3 platform methods: display, dir_name, default_build_type
  - 1 build mode: display
  - 5 config parsing: full config, minimal, android defaults, simulator, flavor default mode,
    with package filters
- [x] `task check:all` passes — 446 tests (420 unit + 26 integration), zero clippy warnings

## Remaining / Future

Stretch goals and out-of-scope items. None of these are required for Melos 7.4.0 CLI parity.

- [ ] Plugin system for custom commands
- [ ] GitHub Actions integration helpers
- [ ] IDE integration (IntelliJ, VS Code) -- out of scope for CLI tool

---

## Core Library Extraction -- `melos-core` + `melos-tui` (Beyond Melos Parity)

Extract business logic into a reusable library crate (`melos-core`) with an event-based
architecture. The current CLI becomes a thin rendering layer. A new TUI frontend (`melos-tui`)
consumes the same core. See `docs/rationale/0006_core_lib_extraction.md` for full rationale.

### Phase 1 -- Cargo workspace + mechanical extraction

Convert the single binary crate into a Cargo workspace. Move pure logic modules into
`melos-core` with zero behavior change.

- [x] Create `Cargo.toml` workspace root with `[workspace]` and `[workspace.dependencies]`
- [x] Create `crates/melos-core/Cargo.toml` with shared deps (tokio, serde, serde_yaml/yaml_serde, anyhow, regex, glob, notify)
- [x] Create `crates/melos-cli/Cargo.toml` with CLI deps (clap, colored, indicatif) + `melos-core` dep
- [x] Move `src/config/` to `crates/melos-core/src/config/` (as-is, pure logic)
- [x] Move `src/package/` to `crates/melos-core/src/package/` (as-is, pure logic)
- [x] Move `src/workspace.rs` to `crates/melos-core/src/workspace.rs` (warnings vec instead of eprintln)
- [x] Move `src/watcher/` to `crates/melos-core/src/watcher/` (removed colored/emoji, plain text)
- [x] Extract pure logic functions from each command file into `crates/melos-core/src/commands/` -- done, Phase 3 Batch A (Batch 45)
- [x] Move `src/runner/mod.rs` core logic to `crates/melos-core/src/runner.rs` -- done, Phase 2 (Batch 44b)
- [x] Keep CLI `run()` wrappers in `crates/melos-cli/src/commands/` -- commands + runner stay in CLI for now
- [x] Move `src/cli.rs` and `src/main.rs` to `crates/melos-cli/src/`
- [x] Create `crates/melos-core/src/lib.rs` with public module exports
- [x] Create `crates/melos-cli/src/filter_ext.rs` -- `package_filters_from_args()` free function (orphan rule workaround)
- [x] Verify `task check:all` passes -- 533 tests (353 CLI unit + 26 integration + 154 core unit), zero clippy warnings
- [x] Verify binary name is still `melos-rs`
- [x] Remove empty `src/` and `tests/` directories from workspace root
- [x] Update `Taskfile.yml` for workspace structure (--workspace flags, install path)

### Phase 2 -- Event enum + ProcessRunner refactor

Define the core Event type and refactor ProcessRunner to emit events instead of
driving progress bars directly.

- [x] Create `crates/melos-core/src/events.rs` with `Event` enum:
  - `CommandStarted { command, package_count }`
  - `CommandFinished { command, duration }`
  - `PackageStarted { name }`
  - `PackageFinished { name, success, duration }`
  - `PackageOutput { name, line, is_stderr }`
  - `Progress { completed, total, message }`
  - `Warning(String)`, `Info(String)`
- [x] Move `ProcessRunner`, `shell_command`, `build_package_env`, `find_parent_package` to `melos-core/src/runner.rs`
  - New method: `run_in_packages_with_events()` accepts `Option<&UnboundedSender<Event>>`
  - Old `run_in_packages()` is convenience wrapper passing `None`
  - 11 unit tests moved from CLI to core (CLI: 353->342, core: 154->165)
- [x] Create CLI render module `crates/melos-cli/src/render.rs`:
  - `spawn_renderer(total, message)` -- progress bar + event loop
  - `spawn_plain_renderer()` -- colored output, no progress bar
  - `create_progress_bar(total, message)` -- standalone progress bar for non-runner use
  - `render_loop()` -- internal event consumer with per-package color assignment
- [x] Gut CLI `runner/mod.rs` to only `run_lifecycle_hook()` (imports `shell_command` from core)
- [x] Update all 10 CLI command files to use event-based patterns:
  - `format.rs` -- standard `spawn_renderer` + `run_in_packages_with_events`
  - `exec.rs` -- two `spawn_renderer` instances (normal + watch)
  - `publish.rs` -- standard + lifecycle hooks
  - `pub_cmds.rs` -- flutter/dart split with `Progress` events
  - `test.rs` -- flutter/dart split with `Progress` events + lifecycle hooks
  - `build.rs` -- `spawn_plain_renderer` + lifecycle hooks
  - `run.rs` -- `spawn_plain_renderer` + `shell_command` import update
  - `clean.rs` -- mixed: renderer for flutter, manual pb for dart filesystem ops
  - `bootstrap.rs` -- renderer with `bail_msg` pattern for error collection
  - `analyze.rs` -- flutter/dart split with `Progress` events + `scan_dry_run` uses standalone pb
- [x] Verify `task check:all` passes -- 533 tests (342 CLI + 26 integration + 165 core), zero clippy warnings
- [x] `indicatif` and `colored` already absent from `melos-core` (never added)

### Phase 3 -- Migrate command run() functions to core

Move command orchestration logic from CLI wrappers into core, one command at a time.
Each core command accepts `UnboundedSender<Event>` and returns a typed result summary.

- [x] **Simple commands (batch A):** done, Batch 45
  - `list` -- core: `detect_cycles()`, `generate_gviz()`, `generate_mermaid()`, `build_packages_json()`
  - `clean` -- core: `DEEP_CLEAN_DIRS/FILES`, `OverrideRemoval` enum, `remove_pubspec_overrides()`
  - `format` -- core: `FormatOpts`, `build_format_command()`, `run()` returns `PackageResults`
  - `test` -- core: `TestOpts`, `build_extra_flags()`, `build_test_command()`, `run()` returns `PackageResults`
  - `init` -- core: `write_7x_config()`, `write_legacy_config()`, `create_dir_if_missing()`
  - `health` -- core: `HealthOpts`, all data types, all `collect_*` functions, `run()` returns `HealthReport`
- [ ] **Medium commands (batch B):**
  - `exec` -- core handles package iteration + watch loop, emits events
  - `bootstrap` -- core handles pub get + overrides + enforce, emits events
  - `publish` -- core handles dry-run/publish + git tag, emits events
  - `analyze` -- core handles fix/dry-run/analyze phases, emits `AnalyzeDryRun`/`ConflictDetected` events
- [ ] **Complex commands (batch C):**
  - `run` -- core handles script resolution, step execution, watch loop, emits events
  - `build` -- core handles platform/flavor iteration, simulator post-build, emits `BuildStepResult` events
  - `version` -- core handles conventional commits, changelog, git ops, emits `VersionBumped` events
- [ ] Define typed opts structs in core (decoupled from clap): `AnalyzeOpts`, `BootstrapOpts`, `ExecOpts`, etc.
- [ ] CLI maps `clap` args to core opts structs in each wrapper
- [ ] Verify `task check:all` passes after each batch

### Phase 4 -- TUI frontend with ratatui

Build `melos-tui` binary consuming `melos-core`.

- [ ] Create `crates/melos-tui/Cargo.toml` with deps: `melos-core`, `ratatui`, `crossterm`
- [ ] Implement terminal setup/teardown (alternate screen, raw mode)
- [ ] Implement `App` state machine:
  - `Idle` -- workspace loaded, package list displayed
  - `Running` -- command executing, live progress
  - `Done` -- results displayed, scrollable
- [ ] Implement core event loop: poll crossterm events + core events, render on each frame
- [ ] **Views:**
  - Package list (table with name, version, path, flutter/dart)
  - Command picker (list of available commands + scripts)
  - Execution panel (live per-package progress, output streaming)
  - Results panel (pass/fail summary, scrollable output)
  - Health dashboard (version drift, missing fields, SDK consistency)
- [ ] Keyboard navigation: arrow keys, enter to run, q to quit, tab to switch panels
- [ ] Wire workspace loading on startup (show loading spinner)
- [ ] Wire command execution: user picks command, TUI spawns core task, renders events
- [ ] Verify `melos-tui` builds and runs against test workspace

---

## Build Command — `melos-rs build` (Beyond Melos Parity)

Melos has no built-in build command. Teams currently define 20–30 nearly identical scripts
in `melos.yaml` that differ only in platform/flavor/mode. `melos-rs build` eliminates this
boilerplate with a single declarative config block + CLI interface.

### Problem

A typical Flutter monorepo `melos.yaml` has explosive script duplication for builds:
- `build:android`, `build:android-qa`, `build:android-dev`
- `build:android-aab`, `build:android-simulator`, `build:android-simulator-qa`, `build:android-simulator-dev`
- `build:ios`, `build:ios-qa`, `build:ios-dev`
- `build:ios-simulator`, `build:ios-simulator-qa`, `build:ios-simulator-dev`
- `build:all`, `build:all-simulator`, `build:all-simulator-qa`
- `build:all:patch-version`, `build:all:minor-version`, `build:all:major-version`

Each script is essentially `flutter build <type> -t lib/main_<flavor>.dart --<mode> --flavor <flavor>`
with minor variations. This is ~170 lines of YAML that should be ~20 lines of config.

### Config Design (`melos.yaml`)

```yaml
command:
  build:
    # Flavor definitions — entry point, build mode, and extra args per flavor
    flavors:
      prod:
        target: lib/main_prod.dart       # -t flag
        mode: release                     # --release / --debug / --profile
      qa:
        target: lib/main_qa.dart
        mode: debug
      dev:
        target: lib/main_dev.dart
        mode: debug

    # Default flavor when --flavor is not specified on CLI
    defaultFlavor: prod

    # Platform-specific config
    android:
      # Build types to produce (maps to `flutter build <type>`)
      types: [appbundle, apk]
      # Default type when --type is not specified
      defaultType: appbundle
      # Simulator post-build: bundletool extraction to universal.apk
      simulator:
        enabled: true
        # Command template. Placeholders: {aab_path}, {output_dir}, {flavor}, {mode}
        command: >-
          bundletool build-apks --overwrite --mode=universal
          --bundle={aab_path} --output={output_dir}/{flavor}-unv.apks
          && unzip -o {output_dir}/{flavor}-unv.apks universal.apk -d {output_dir}

    ios:
      # Extra args appended to all iOS builds
      extraArgs: ["--export-options-plist", "ios/runner/exportOptions.plist"]
      # Simulator post-build: xcodebuild for .app file
      simulator:
        enabled: true
        # Command template. Placeholders: {flavor}, {mode}, {configuration}
        command: >-
          xcodebuild -configuration {configuration}
          -workspace ios/Runner.xcworkspace -scheme {flavor}
          -sdk iphonesimulator -derivedDataPath build/ios/archive/simulator

    # Package filters applied to all build targets (same as script packageFilters)
    packageFilters:
      flutter: true

    # Hooks
    hooks:
      pre: echo "Starting build..."
      post: echo "Build complete."
```

### CLI Interface

```
melos-rs build [OPTIONS]

PLATFORMS:
    --android              Build for Android
    --ios                  Build for iOS
    --all                  Build for all platforms (default when none specified)

FLAVORS:
    --flavor <NAME>        Build flavor/environment (default: config defaultFlavor)
                           Can be specified multiple times: --flavor prod --flavor qa

ANDROID OPTIONS:
    --type <TYPE>          Android build type: apk, appbundle (default: config defaultType)
    --simulator            Build simulator-compatible artifact (bundletool extraction)

IOS OPTIONS:
    --simulator            Build simulator .app via xcodebuild instead of .ipa
    --export-options-plist <PATH>  Override export options plist

GENERAL:
    --dry-run              Print commands without executing
    --fail-fast            Stop on first failure
    -c, --concurrency <N>  Max concurrent builds (default: 1)
    --version-bump <TYPE>  Bump version before build: patch, minor, major
    --build-number-bump    Increment build number before build
    --scope <GLOB>         Filter packages by name
    --ignore <GLOB>        Exclude packages by name
```

### Examples

```bash
# Build prod APK for Android (replaces `melos run build:android`)
melos-rs build --android --type apk

# Build QA appbundle (replaces `melos run build:android-qa`)
melos-rs build --android --flavor qa

# Build prod IPA for iOS (replaces `melos run build:ios`)
melos-rs build --ios

# Build simulator artifacts for both platforms (replaces `melos run build:all-simulator`)
melos-rs build --all --simulator

# Build QA simulator for iOS (replaces `melos run build:ios-simulator-qa`)
melos-rs build --ios --simulator --flavor qa

# Bump patch version and build all prod (replaces `melos run build:all:patch-version`)
melos-rs build --all --version-bump patch

# Build all flavors at once
melos-rs build --android --flavor prod --flavor qa --flavor dev
```

### Implementation Batches

#### Batch A — Config parsing & CLI args (done, Batch 33)
- [x] Add `BuildCommandConfig` struct with `flavors`, `defaultFlavor`, `android`, `ios`, `packageFilters`, `hooks`
- [x] Add `FlavorConfig` struct with `target`, `mode` fields
- [x] Add `PlatformConfig` struct with `types`, `defaultType`, `extraArgs`, `simulator` fields
- [x] Add `SimulatorConfig` struct with `enabled`, `command` template fields
- [x] Wire into `CommandConfig.build: Option<BuildCommandConfig>` in `config/mod.rs`
- [x] Add `BuildArgs` struct in `commands/build.rs` with clap derive
- [x] Add `Build(BuildArgs)` variant to `Commands` enum in `cli.rs`
- [x] Add `"build"` to `OVERRIDABLE_COMMANDS` in `main.rs`
- [x] Implement `build_flutter_command()`, `resolve_platforms()`, `resolve_flavors()`, `run()`
- [x] Hook support via `workspace.hook("build", "pre")` / `workspace.hook("build", "post")`
- [x] Tests: config parsing for all build config variants + command assembly + resolution logic

#### Batch B — Core build command execution (done, included in Batch 33)
- [x] Implement `build_flutter_command()` — assembles `flutter build <type> -t <target> --<mode> --flavor <flavor> [extraArgs]`
- [x] Implement `commands::build::run()` — filter packages (flutter + dirExists), build command, execute via ProcessRunner
- [x] Platform detection: `--android` filters to `dirExists: android`, `--ios` filters to `dirExists: ios`
- [x] Multi-flavor support: iterate flavors, build each sequentially
- [x] Hook support via `workspace.hook("build", "pre")` / `workspace.hook("build", "post")`
- [x] Dry-run mode
- [x] Tests: command string assembly for all platform/flavor/mode combos

#### Batch C — Simulator builds & post-processing (done, Batch 34)
- [x] `resolve_artifact_path()` — Flutter build output path conventions (AAB, APK)
- [x] `expand_simulator_template()` — placeholder expansion: `{aab_path}`, `{apk_path}`, `{output_dir}`, `{flavor}`, `{mode}`, `{configuration}`
- [x] `resolve_simulator_command()` — validates config, expands template, returns command string
- [x] Wire `--simulator` flag into `build::run()` flow (after regular build, per-package via ProcessRunner)
- [x] Dry-run mode shows simulator post-build commands
- [x] Error handling: missing config, disabled simulator, missing template all produce actionable errors
- [x] Remove `#[allow(dead_code)]` from `IosBuildConfig.simulator`, `AndroidBuildConfig.simulator`, `SimulatorConfig` struct
- [x] Tests: 24 new tests covering artifact paths, template expansion, simulator command resolution
- Total: 444 unit tests + 26 integration tests = 470 tests

#### Batch D — Version bump integration (done, Batch 35)
- [x] Made `apply_version_bump()` and `extract_build_number()` public in `version.rs`
- [x] Added `VALID_VERSION_BUMPS` constant — restricts build command to `patch`/`minor`/`major` (excludes `build` since `--build-number-bump` handles that)
- [x] Added `validate_version_bump()` function with actionable error messages
- [x] Wired `--version-bump` into `build::run()` — validates bump type, applies to all filtered packages before build loop
- [x] Wired `--build-number-bump` into `build::run()` — calls `apply_version_bump(pkg, "build")` for each package
- [x] Fixed bug in `apply_version_bump`: `compute_next_version("1.2.3+42", "patch")` returned semver with build metadata, causing `format!("{}+{}", next_version, n)` to produce "1.2.4+42+42". Fix: use `major.minor.patch` format to strip build metadata before appending preserved build number
- [x] Tests: 6 validation tests (patch/minor/major accepted, build/empty/arbitrary rejected) + 7 filesystem-based version bump tests (patch/minor/major/build, build-from-zero, patch-preserves-build-number, noop-when-absent)
- Total: 457 unit tests + 26 integration tests = 483 tests

#### Batch 37 — Composite builds & progress reporting (done, v0.3.1)
- [x] `--all` platform flag — already functional in `resolve_platforms()` (when `--all` or neither `--android`/`--ios` specified, builds both platforms sequentially)
- [x] Added `BuildStepResult` struct to track per-step outcomes (platform, flavor, mode, passed/failed counts, duration)
- [x] Added `format_duration()` helper — human-readable durations (e.g., "1m 23.4s", "45.6s", "0.0s")
- [x] Added `format_step_result()` helper — per-step summary lines (e.g., "OK android prod [release]: 3/3 passed (12.3s)")
- [x] Added `format_build_summary()` helper — full summary table with OK/FAIL/skipped status and totals
- [x] Wired progress tracking into `run()` — build plan header with step count, `[n/total]` step counters, per-step timing via `Instant`, per-step completion line, final summary table
- [x] Moved `packages.is_empty()` skip check inside flavor loop so each platform+flavor combo gets tracked as "skipped" in summary
- [x] Dry-run steps tracked as `BuildStepResult` with `passed = packages.len()` and `Duration::ZERO`
- [x] Simulator failures counted in step results (`sim_failed` added to `failed` count)
- [x] Tests: 4 format_duration + 3 format_step_result + 4 format_build_summary + 2 BuildStepResult struct = 13 new tests
- Total: 470 unit tests + 26 integration tests = 496 tests

#### Batch 38 — Analyze --fix & README updates (done, v0.3.2)
- [x] Added `--fix` flag to `analyze` command — runs `dart fix --apply` in each package before `dart analyze`
- [x] Added `FIX_COMMAND` constant for the fix command string
- [x] Fix failures are non-fatal (warns and continues with analysis)
- [x] Updated README: analyze command description updated, added Analyze Options table, added Build Options table
- [x] Updated README test count to 473 unit + 26 integration = 499
- [x] Tests: 1 FIX_COMMAND constant + 1 AnalyzeArgs --fix parsing + 1 --fix combined with --fatal-warnings = 3 new tests
- Total: 473 unit tests + 26 integration tests = 499 tests

#### Batch 40 — Proactive conflict detection in dry-run (done, v0.4.0)
- [x] Added `detect_conflicting_diagnostics()` — heuristic: two codes appearing in same files with equal fix counts across 2+ files signals conflicting lint rules
- [x] Added `format_conflict_warnings()` — formats warning block with rule names, file count, and `analysis_options.yaml` disable suggestions
- [x] Integrated into `--dry-run` display path — warning appears after fix suggestions when conflicts detected
- [x] No hardcoded lint pairs — detection is purely output-driven
- [x] Tests: 5 detect_conflicting_diagnostics + 1 format_conflict_warnings + 1 integration = 7 new tests
- [x] Created `docs/rationale/0005_proactive_conflict_detection.md`
- Total: 500 unit tests + 26 integration tests = 526 tests

#### Batch 41 — Real-world benchmarks (done, v0.4.1)
- [x] Built release binary, verified melos 7.4.0 + hyperfine 1.20.0 available
- [x] Benchmarked 5 commands on fl_template (4-package Flutter workspace): list, list --json, exec, format, analyze
- [x] Results: 16-18x faster for orchestration commands (list, exec); 1.6x for format; 1.01x for analyze (Dart toolchain bottleneck)
- [x] Updated README: added second benchmark table for real-world workspace with analysis of speedup patterns
- [x] Bumped version to 0.4.1
- Total: 500 unit tests + 26 integration tests = 526 tests (no test changes, benchmarks only)

#### Batch 42 — Fix conflict pre-scan in --fix mode (done)
- [x] Extracted `scan_dry_run()` async function from inline dry-run scanning logic — reusable by both `--dry-run` and `--fix` paths
- [x] Added `DryRunScan` struct to hold entries, codes, and conflicts from a scan
- [x] `--fix` mode now pre-scans with `dart fix --dry-run` when no `--code` filter is set
- [x] If conflicts detected: skips `dart fix --apply`, prints conflict warning, suggests `--code` or fixing `analysis_options.yaml`, falls through to analyze-only
- [x] If `--code` is set: bypasses pre-scan (user explicitly chose diagnostics)
- [x] Progress label is context-aware: "previewing fixes" for `--dry-run`, "scanning for conflicts" for `--fix` pre-scan
- [x] Tests: 3 skip-logic decision tests + 1 DryRunScan struct assembly test = 4 new tests
- [x] Updated README: `--fix` description now mentions conflict pre-scan
- Total: 504 unit tests + 26 integration tests = 530 tests — Analyze --dry-run, --code flags (done, v0.3.3)
- [x] Added `--dry-run` flag to `analyze` command — runs `dart fix --dry-run` only, skips analysis (conflicts with `--fix`)
- [x] Added `--code` flag — comma-separated diagnostic codes appended as `--code=<code>` to dart fix command
- [x] `--code` validated to require `--fix` or `--dry-run`
- [x] Replaced `FIX_APPLY_COMMAND`/`FIX_DRY_RUN_COMMAND` constants with `build_fix_command(apply, codes)` function
- [x] Rewrote `run()` to handle three modes: dry-run (preview only), fix+analyze, analyze-only
- [x] Updated README: added `--dry-run` and `--code` rows to Analyze Options table
- [x] Tests: 4 build_fix_command + 2 --code flag parsing + replaced 2 stale constant tests = net +8 new tests
- [x] Dry-run output parser: `parse_dry_run_output()` with `FIX_LINE_RE` regex handling both bullet and dash separators across Dart SDK versions
- [x] Dry-run display: consolidated sorted results with footer showing both `dart fix` and `melos-rs analyze --fix` commands
- [x] `command_has_builtin_flags()` in main.rs: bypasses script override when command-specific flags are present (analyze, bootstrap, clean, format, test, publish)
- [x] Mode-specific headers: "Analyzing" / "Fixing and analyzing" / "Previewing fixes for"
- [x] `strip_ansi()` test helper in build.rs: fixes ANSI-dependent test assertions for colored output
- [x] Tests: 5 parse_dry_run_output + 2 parse_fix_line + 2 build_fix_command with codes + 3 dry-run integration = net +12 new tests
- Total: 493 unit tests + 26 integration tests = 519 tests

#### Batch 36 — Refactoring pass (done)
- [x] Audited all 10 if-else chains with 3+ branches across src/ (version.rs, build.rs, config/mod.rs, run.rs, exec.rs)
- [x] Converted `config/mod.rs:1106` package path resolution from if-let chain to tuple `match (wrapper.melos.packages, wrapper.workspace)` (-3 LOC)
- [x] Converted `run.rs:276` script mode display tag from if-else chain to tuple `match (steps, exec)` (-2 LOC)
- [x] Deferred VersionStrategy enum (version.rs:1209-1344, 6-branch chain) — would add +50 LOC per AGENTS.md GoF policy
- [x] Deferred ScriptMode enum (run.rs:354-362) — would add infrastructure LOC without net reduction
- [x] Confirmed zero `unwrap()` in production code (438 calls all in test modules)
- [x] Confirmed all 7 `.expect()` calls in production code have safety justification comments
- [x] Updated AGENTS.md to document `.expect()` with safety comment convention (was previously "never use expect")
- Total: 457 unit tests + 26 integration tests = 483 tests (no test changes, refactoring only)

#### Batch 43 — Analyze consistency: Flutter/Dart SDK split and dry-run analysis (done)
- [x] `build_analyze_command()` now takes `is_flutter: bool` — uses `flutter analyze` for Flutter packages and `dart analyze` for Dart packages, matching SDK default behavior (`flutter analyze` defaults to `--fatal-infos`, `dart analyze` does not)
- [x] Analysis phase splits packages into Flutter/Dart groups using the same pattern as `test.rs` and `bootstrap.rs`
- [x] Package listing now shows SDK type: `-> pkg_name (flutter)` / `-> pkg_name (dart)`
- [x] `--dry-run` now falls through to analysis phase after previewing fixes — previously returned early, causing inconsistent SUCCESS/FAILED results between `analyze`, `analyze --fix`, and `analyze --dry-run`
- [x] All three analyze modes now produce consistent results on real workspace (fl_template)
- [x] Tests: 3 new `build_analyze_command` tests for Flutter SDK (default, no_fatal, fatal_warnings)
- [x] Updated README test count to 533
- Total: 507 unit tests + 26 integration tests = 533 tests

#### Batch 44 — Core library extraction: workspace split (done, v0.5.0)
- [x] Converted root `Cargo.toml` to workspace manifest with `[workspace]`, `[workspace.package]`, `[workspace.dependencies]`
- [x] Created `crates/melos-core/` library crate — config/, package/, workspace.rs, watcher/ modules; zero terminal deps (no clap, colored, indicatif)
- [x] Created `crates/melos-cli/` binary crate — main.rs, cli.rs, commands/, runner/, filter_ext.rs
- [x] Moved `ConfigSource` enum from workspace.rs into config/mod.rs to break circular dependency
- [x] Created `filter_ext.rs` with `package_filters_from_args()` free function — orphan rule workaround for `From<&GlobalFilterArgs> for PackageFilters`; updated all 13 command call sites
- [x] Replaced `eprintln!` in workspace `find_and_load()` with `warnings: Vec<String>` field — CLI main.rs prints warnings with colored formatting
- [x] Removed `colored::Colorize` import and emoji from watcher/mod.rs — CLI callers print watch message
- [x] Removed `#![feature(let_chains)]` from both crate roots (stable since Rust 1.88, running 1.93.1)
- [x] Fixed 17 clippy lints in test code: `unnecessary_get_then_check` (5), `needless_borrows_for_generic_args` (1), `manual_range_contains` (3), `useless_vec` (8)
- [x] Updated Taskfile.yml for workspace structure (--workspace flags, install path, version extraction)
- [x] Removed empty src/ and tests/ directories from workspace root
- Total: 507 unit tests + 26 integration tests = 533 tests (353 CLI unit + 154 core unit + 26 integration); 5 new filter_ext tests

#### Batch 45 — Phase 3 Batch A: migrate simple command logic to core (done, v0.5.2)
- [x] Created `crates/melos-core/src/commands/mod.rs` — `PackageResults` shared type with `passed()`/`failed()` helpers + 3 tests
- [x] Created `crates/melos-core/src/commands/format.rs` — `FormatOpts`, `build_format_command()`, `run()` returns `PackageResults` + 7 tests
- [x] Created `crates/melos-core/src/commands/test.rs` — `TestOpts`, `build_extra_flags()`, `build_test_command()`, `run()` returns `PackageResults` + 7 tests
- [x] Created `crates/melos-core/src/commands/clean.rs` — `DEEP_CLEAN_DIRS/FILES` constants, `OverrideRemoval` enum, `remove_pubspec_overrides()` + 6 tests
- [x] Created `crates/melos-core/src/commands/list.rs` — `PackageJson`, `CycleResult`, `detect_cycles()`, `generate_gviz()`, `generate_mermaid()`, `build_packages_json()` + 5 tests
- [x] Created `crates/melos-core/src/commands/health.rs` — `HealthOpts`, all 5 data types, all `collect_*` functions, `run()` returns `HealthReport` + 11 tests
- [x] Created `crates/melos-core/src/commands/init.rs` — `write_7x_config()`, `write_legacy_config()`, `create_dir_if_missing()` + 6 tests
- [x] Updated 6 CLI command files to import from core, removed duplicated logic and tests
- [x] Architecture: core gets opts structs (no clap), pure logic, orchestration; CLI retains clap args, colored output, renderer, lifecycle hooks
- [x] Updated Phase 3 checklist and deferred items
- Total: 513 unit tests + 26 integration tests = 539 tests (303 CLI unit + 210 core unit + 26 integration); 45 net new core tests
