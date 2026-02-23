# melos-rs

A fast Rust CLI replacement for [Melos](https://melos.invertase.dev/) — the Flutter/Dart monorepo management tool.

Parses `melos.yaml`. Discovers packages. Executes commands. Manages versions. **Fast.**

## Benchmarks

### 15-package monorepo (synthetic)

Measured using [hyperfine](https://github.com/sharkdp/hyperfine):

| Command | melos | melos-rs | Speedup |
|:--------|------:|---------:|--------:|
| `list` | 518 ms | 7.6 ms | **68x** |
| `list --json` | 525 ms | 7.6 ms | **69x** |
| `exec -- echo hi` | 555 ms | 29 ms | **19x** |

### 4-package Flutter workspace (real-world)

Measured on [fl_template](https://github.com/nicosResworworked) (4 packages: adapter, model, ui, theme) with Melos 7.4.0 vs melos-rs 0.4.1:

| Command | melos | melos-rs | Speedup |
|:--------|------:|---------:|--------:|
| `list` | 546 ms | 32 ms | **17x** |
| `list --json` | 542 ms | 30 ms | **18x** |
| `exec -- echo hi` | 570 ms | 35 ms | **16x** |
| `format --set-exit-if-changed` | 1.36 s | 839 ms | **1.6x** |
| `analyze` | 9.93 s | 9.81 s | **1.01x** |

For I/O-bound commands (`analyze`, `format`), the bottleneck is the Dart toolchain itself. The speedup is most visible in orchestration-heavy commands (`list`, `exec`) where Dart VM startup overhead dominates.

## Features

Full parity with Melos 7.4.0 for CLI workflows:

**Commands**

| Command | Description |
|---------|-------------|
| `bootstrap` | Link packages and run `pub get` across the workspace |
| `clean` | Run `flutter clean` in packages (with optional deep clean) |
| `exec` | Execute arbitrary commands in each package |
| `run` | Run named scripts defined in `melos.yaml` |
| `list` | List packages (long, json, parsable, graph, gviz, mermaid) |
| `version` | Bump versions via conventional commits, generate changelogs, create git tags |
| `publish` | Publish packages to pub.dev with dry-run support |
| `test` | Run `dart test` / `flutter test` with coverage and golden updates |
| `analyze` | Run `dart analyze` with `--fix`, fatal warnings/infos control |
| `format` | Run `dart format` across packages |
| `pub` | Run `pub get`, `upgrade`, `downgrade`, `add`, `remove` |
| `init` | Scaffold a new Melos workspace (6.x or 7.x format) |
| `health` | Workspace health checks: version drift, missing fields, SDK consistency |
| `completion` | Generate shell completions for bash, zsh, fish |

**Package Filters** (shared across all commands)

`--scope`, `--ignore`, `--diff`/`--since`, `--dir-exists`, `--file-exists`, `--flutter`/`--no-flutter`, `--depends-on`, `--no-depends-on`, `--no-private`, `--published`/`--no-published`, `--category`, `--include-dependencies`, `--include-dependents`

**Configuration**

- `melos.yaml` (6.x format) and `pubspec.yaml` with `melos:` section (7.x format)
- Named scripts with steps, exec config, environment variables, groups, and privacy
- Command hooks (pre/post) for bootstrap, clean, test, publish, and version
- Workspace `categories` for package grouping
- `resolution: workspace` support (Dart 3.5+) — skips `pubspec_overrides.yaml` generation
- Shared dependency synchronization and version enforcement
- Repository config for commit/release URL generation

**Execution**

- Configurable concurrency with `--concurrency` / `-c` (default 5)
- `--fail-fast` to abort on first failure
- `--order-dependents` for topological execution order
- File watching with `--watch` for exec and run commands
- Cross-platform shell support (Unix `sh -c` / Windows `cmd /C`)
- Buffered output to prevent interleaving in concurrent mode
- Per-package environment variables (`MELOS_PACKAGE_NAME`, `MELOS_PACKAGE_VERSION`, etc.)

**Analyze Options**

| Flag | Description |
|------|-------------|
| `--fix` | Run `dart fix --apply` in each package before analyzing. Pre-scans for conflicting lint rules and skips fix if conflicts detected. |
| `--dry-run` | Preview fixes with `dart fix --dry-run` (no changes applied, skips analysis). Detects conflicting lint rules automatically. |
| `--code` | Comma-separated diagnostic codes to restrict fixes (requires `--fix` or `--dry-run`) |
| `--fatal-warnings` | Report warnings as fatal errors |
| `--fatal-infos` | Report info-level issues as fatal errors |
| `--no-fatal` | Override `--fatal-warnings` and `--fatal-infos` |
| `-c, --concurrency` | Max concurrent processes (default: 5) |

**Build Options** (beyond Melos parity)

Declarative build command configured via `melos.yaml`:

| Flag | Description |
|------|-------------|
| `--android` | Build for Android only |
| `--ios` | Build for iOS only |
| `--all` | Build for all platforms (default when neither `--android` nor `--ios` specified) |
| `--flavor <name>` | Build flavor/environment (repeatable; defaults to config `defaultFlavor`) |
| `--type <type>` | Android build type: `apk` or `appbundle` (defaults to config `defaultType`) |
| `--simulator` | Build simulator-compatible artifacts (bundletool/xcodebuild) |
| `--export-options-plist <path>` | Override export options plist for iOS builds |
| `--version-bump <level>` | Bump version before building: `patch`, `minor`, or `major` |
| `--build-number-bump` | Increment build number before building |
| `--dry-run` | Print commands without executing |
| `--fail-fast` | Stop on first failure |
| `-c, --concurrency` | Max concurrent build processes (default: 1) |

Build progress is reported per-step with timing and a summary table at completion.

## Installation

### From source

Requires [Rust toolchain](https://rustup.rs/) (nightly — uses `let_chains` feature).

```sh
cargo install --path .
```

### Build

```sh
# Debug build
cargo build

# Release build
cargo build --release
```

## Usage

```sh
# List all packages
melos-rs list

# Bootstrap the workspace
melos-rs bootstrap

# Execute a command across packages
melos-rs exec -- dart analyze

# Run a script defined in melos.yaml
melos-rs run build

# Bump versions using conventional commits
melos-rs version

# Filter by scope
melos-rs list --scope="my_package*"

# Shell completions
melos-rs completion bash >> ~/.bashrc
```

## Development

Uses [Task](https://taskfile.dev/) for development workflow:

```sh
# Format, lint, test
task check:all

# Run benchmarks (requires melos + hyperfine)
task bench:all

# Install locally
task install
```

### Project structure

```
src/
  main.rs             Entry point
  cli.rs              Clap CLI definitions
  workspace.rs        Workspace loading (config + packages)
  config/
    mod.rs            melos.yaml parsing
    script.rs         Script config types
    filter.rs         Filter config types
  package/
    mod.rs            Package model, pubspec parsing, discovery
    filter.rs         Package filter logic
  commands/           Command implementations
  runner/mod.rs       Concurrent process execution
  watcher/mod.rs      File watching
```

### Test suite

530 tests (504 unit + 26 integration). Run with:

```sh
cargo test
```

## License

MIT
