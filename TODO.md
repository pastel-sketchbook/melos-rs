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
- [x] `PackageFilters::merge()` for combining CLI + script-level filters
- [x] `Package.publish_to` field, `Package.is_private()` method

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
- [ ] `melos-rs init` — scaffold new 7.x workspace

## Phase 3: Advanced Features

### Script Execution Engine
- [ ] Full `melos exec` flag parsing when invoked from `run` command
- [x] Script-level `env` field support (merged into run command environment)
- [ ] Recursive `melos run X` expansion (handle nested script references)
- [ ] Timeout support for long-running commands
- [ ] Dry-run mode (`--dry-run`)

### Package Management
- [x] Topological sort for dependency-ordered execution (Kahn's algorithm, wired to bootstrap + exec)
- [x] Circular dependency detection (`list --cycles`)
- [x] Category-based package filtering (`categories` config field)
- [x] Workspace-level pubspec overrides (pubspec_overrides.yaml for 6.x local linking)
- [ ] `pub:get`, `pub:outdated`, `pub:upgrade` built-in commands

### Build Commands (from melos.yaml scripts)
- [ ] `build:android` / `build:ios` wrapper commands
- [ ] Flavor/environment support (prod, qa, dev)
- [ ] Build artifact path resolution
- [ ] Simulator build + bundletool integration

### Versioning & Release
- [ ] `version:set` command (set explicit version across all packages)
- [ ] Coordinated versioning (keep all packages in sync)
- [ ] Git integration (push after version bump)
- [ ] Release branch management

### Developer Experience
- [ ] `melos-rs init` - scaffold a new 7.x workspace (pubspec.yaml with melos: key)
- [ ] Tab completion for bash/zsh/fish
- [ ] Progress bars with `indicatif` for more commands
- [ ] Verbose/quiet/debug log levels
- [ ] Config validation and helpful error messages
- [ ] Watch mode for development (`--watch`)

## Phase 4: Parity & Beyond

- [ ] Full Melos CLI flag compatibility
- [ ] Migration guide from Melos to melos-rs
- [ ] Performance benchmarks vs Melos
- [ ] Plugin system for custom commands
- [ ] GitHub Actions integration helpers
- [ ] Monorepo health checks (unused deps, version drift)
