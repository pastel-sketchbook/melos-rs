# melos-rs TODO

A Rust CLI replacement for [Melos](https://melos.invertase.dev/) - Flutter/Dart monorepo management tool.

## Phase 1: Core Infrastructure (MVP)

- [x] Project scaffolding (Cargo.toml, module structure)
- [x] CLI argument parsing with `clap` (commands: exec, run, version, bootstrap, clean, list)
- [x] `melos.yaml` config parsing (name, packages, scripts, command hooks)
- [x] Package discovery from glob patterns
- [x] Pubspec.yaml parsing (name, version, dependencies, flutter detection, publish_to)
- [x] Package filtering (flutter/dart, dir-exists, file-exists, depends-on, scope, ignore)
- [x] Process runner with configurable concurrency and fail-fast
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
- [x] `--category=<cat>` flag (parsed, not yet applied - needs melos.yaml categories config)
- [x] `--include-dependencies` transitive dependency expansion
- [x] `--include-dependents` transitive dependent expansion
- [x] `PackageFilters::merge()` for combining CLI + script-level filters
- [x] `Package.publish_to` field, `Package.is_private()` method
- [x] 19 tests passing, clippy clean

## Phase 2: Command Implementations

### `exec` Command
- [x] Basic exec: run command in each package
- [x] `-c N` concurrency control
- [x] `--fail-fast` flag
- [x] Global filters (`--scope`, `--ignore`, `--depends-on`, etc.) via flattened GlobalFilterArgs
- [ ] Per-package env vars injected during exec (MELOS_PACKAGE_NAME, etc.)
- [ ] `--no-select` flag (skip package selection prompt)
- [ ] Colored per-package output prefixing

### `run` Command
- [x] Basic run: execute named scripts from config
- [x] `melos run` -> `melos-rs run` self-reference expansion
- [x] `&&` chain splitting for sequential execution
- [ ] Script-level `packageFilters` applied before exec
- [ ] `$MELOS_ROOT_PATH` and other env var substitution in commands
- [ ] Interactive script selection when no script name given
- [ ] `--no-select` flag support

### `version` Command
- [x] Version bump types: build, patch, minor, major
- [x] Per-package overrides with `-V package:bump` syntax
- [x] `--all` flag to bump all packages
- [x] `--yes` flag to skip confirmation
- [x] Pre-commit hook execution from config
- [x] Pubspec.yaml version rewriting
- [x] Flutter `+buildNumber` format handling
- [ ] CHANGELOG.md generation from git commits
- [ ] Git tag creation after version bump
- [ ] Branch validation (ensure on correct branch)
- [ ] Conventional commit parsing

### `bootstrap` Command
- [x] Run `flutter pub get` / `dart pub get` in each package
- [x] Global filters support
- [ ] Link local package dependencies (path overrides)
- [ ] Parallel bootstrapping with progress indicator

### `clean` Command
- [x] Run `flutter clean` in Flutter packages
- [x] Global filters support
- [ ] Clean build artifacts in pure Dart packages
- [ ] `--deep` flag to also delete `.dart_tool/`, `pubspec.lock`

### `list` Command
- [x] List all packages
- [x] `--long` flag for detailed output (now shows private tag)
- [x] `--json` flag for machine-readable output (now includes private field)
- [x] Global filters support (replaces per-command --flutter/--no-flutter)
- [ ] Dependency graph visualization
- [ ] `--graph` flag for tree output

## Phase 3: Advanced Features

### Script Execution Engine
- [ ] Full `melos exec` flag parsing when invoked from `run` command
- [ ] Recursive `melos run X` expansion (handle nested script references)
- [ ] Timeout support for long-running commands
- [ ] Dry-run mode (`--dry-run`)

### Package Management
- [ ] Topological sort for dependency-ordered execution
- [ ] Circular dependency detection
- [ ] `pub:get`, `pub:outdated`, `pub:upgrade` built-in commands
- [ ] Workspace-level pubspec overrides

### Build Commands (from melos.yaml scripts)
- [ ] `build:android` / `build:ios` wrapper commands
- [ ] Flavor/environment support (prod, qa, dev)
- [ ] Build artifact path resolution
- [ ] Simulator build + bundletool integration

### Versioning & Release
- [ ] `version:set` command (set explicit version across all packages)
- [ ] Coordinated versioning (keep all packages in sync)
- [ ] Git integration (commit, tag, push after version bump)
- [ ] CHANGELOG.md auto-generation from conventional commits
- [ ] Release branch management

### Developer Experience
- [ ] `melos-rs init` - generate a starter melos.yaml
- [ ] Tab completion for bash/zsh/fish
- [ ] Progress bars with `indicatif`
- [ ] Verbose/quiet/debug log levels
- [ ] Config validation and helpful error messages
- [ ] Watch mode for development (`--watch`)

## Phase 4: Parity & Beyond

- [ ] `format` command (dart format across packages)
- [ ] `publish` command (pub.dev publishing with dry-run)
- [ ] Full Melos CLI flag compatibility
- [ ] Migration guide from Melos to melos-rs
- [ ] Performance benchmarks vs Melos
- [ ] Plugin system for custom commands
- [ ] GitHub Actions integration helpers
- [ ] Monorepo health checks (unused deps, version drift)
