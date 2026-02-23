# Migrating from Melos to melos-rs

This guide covers switching from [Melos](https://melos.invertase.dev/) (Dart) to **melos-rs** (Rust). The tools are near-identical in behavior — most migrations require only changing the binary name.

## Quick Start

```sh
# Install melos-rs
cargo install --path .

# Replace `melos` with `melos-rs` in your commands
melos-rs bootstrap
melos-rs list
melos-rs run build
```

**Your `melos.yaml` works as-is.** No config changes required. Both 6.x (`melos.yaml`) and 7.x (`pubspec.yaml` with `melos:` section) formats are supported.

## What Works Identically

These commands and features are drop-in replacements — just swap `melos` for `melos-rs`:

- `bootstrap` (alias `bs`) — pub get, local linking, hooks, shared deps
- `clean` — flutter clean across packages
- `exec` — run commands in each package with concurrency control
- `run` — execute named scripts from config
- `list` — all formats (long, json, parsable, graph, gviz)
- `version` — conventional commits, changelogs, git tags, release branches
- `publish` — publish to pub.dev with dry-run
- `format` — dart format across packages
- `analyze` — dart analyze with fatal controls
- `test` — dart/flutter test with coverage
- `pub` — get, outdated, upgrade, downgrade, add, remove
- `init` — scaffold new workspace
- `completion` — shell completions for bash, zsh, fish
- All global filters: `--scope`, `--ignore`, `--diff`/`--since`, `--dir-exists`, `--file-exists`, `--flutter`/`--no-flutter`, `--depends-on`/`--no-depends-on`, `--no-private`, `--published`/`--no-published`, `--category`, `--include-dependencies`, `--include-dependents`
- All config fields: `packages`, `scripts`, `command.*` hooks, `categories`, `repository`, `sdkPath`, `useRootAsPackage`, `discoverNestedWorkspaces`

## Scripts Auto-Adapt

References to `melos` in your scripts are automatically rewritten to `melos-rs` at runtime. You do **not** need to update your `melos.yaml` scripts:

```yaml
# This works as-is — melos-rs expands "melos run" to "melos-rs run"
scripts:
  build:
    run: melos run generate && melos run compile
```

## Behavioral Differences

### publish --dry-run defaults to true

melos-rs defaults `--dry-run` to `true` (safe by default). To actually publish:

```sh
melos-rs publish --no-dry-run
```

### format --concurrency defaults to 1

Matches Melos behavior. Use `-c N` to increase parallelism.

### init creates 7.x format by default

```sh
melos-rs init                  # Creates pubspec.yaml with melos: section (7.x)
melos-rs init --legacy         # Creates melos.yaml (6.x)
```

### Bootstrap and workspace resolution

Packages with `resolution: workspace` (Dart 3.5+) are automatically skipped during `pubspec_overrides.yaml` generation to avoid conflicts with the Dart workspace resolver.

## Features Only in melos-rs

These are additions not available in Melos:

| Feature | Usage |
|---------|-------|
| Workspace health checks | `melos-rs health --version-drift --missing-fields --sdk-consistency` |
| File watching | `melos-rs exec --watch -- dart test` or `melos-rs run build --watch` |
| Exec dry-run | `melos-rs exec --dry-run -- echo hi` |
| Exec timeout | `melos-rs exec --timeout 30 -- long_task` |
| Deep clean | `melos-rs clean --deep` (removes `.dart_tool/`, `build/`, `pubspec.lock`) |
| Cycle detection | `melos-rs list --cycles` |
| Mermaid diagrams | `melos-rs list --format=mermaid` |
| Buffered output | Concurrent output is never interleaved |

## Features Only in Melos

These Melos features are not available in melos-rs:

| Feature | Notes |
|---------|-------|
| IDE integration | IntelliJ and VS Code plugins — out of scope for a CLI tool |
| `build:android`/`build:ios` | Use scripts in `melos.yaml` instead |

## CI/CD Migration

Replace `melos` with `melos-rs` in your CI scripts:

```yaml
# Before (GitHub Actions)
- run: dart pub global activate melos
- run: melos bootstrap

# After
- run: cargo install --path . # or download prebuilt binary
- run: melos-rs bootstrap
```

The `MELOS_PACKAGES` environment variable works identically for CI scope overrides:

```sh
MELOS_PACKAGES="my_app,my_core" melos-rs test
```

## Verifying the Migration

After switching, run these checks:

```sh
# Verify package discovery matches
melos-rs list --json

# Verify bootstrap works
melos-rs bootstrap

# Run your existing scripts
melos-rs run <your-script>

# Check workspace health (bonus)
melos-rs health
```
