# melos-rs TODO

A Rust CLI replacement for [Melos](https://melos.invertase.dev/) - Flutter/Dart monorepo management tool.

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

### Build Commands (from melos.yaml scripts)
- [ ] _(see "Known Gaps" section below)_

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
- [ ] Release branch management _(see "Known Gaps" section below)_

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
- [ ] `build:android` / `build:ios` wrapper commands (flavor/environment support, artifact resolution, simulator/bundletool)
- [ ] Release branch management (auto-create/merge release branches during `version`)
- [x] Progress bars with `indicatif` for more commands (only bootstrap has one currently)
- [x] Watch mode for development (`--watch` flag on exec/run)

## Phase 4: Parity & Beyond

- [x] Full Melos CLI flag compatibility audit
- [ ] Migration guide from Melos to melos-rs
- [ ] Performance benchmarks vs Melos
- [ ] Plugin system for custom commands
- [ ] GitHub Actions integration helpers
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
