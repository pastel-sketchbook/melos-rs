# 0004: Build Command — Beyond Melos Parity

## Context

Melos (Dart) has no built-in build command. Teams define platform builds as
scripts in `melos.yaml`, leading to massive duplication. A real-world
`melos.yaml` from a production Flutter monorepo contains 24 build-related
scripts (~170 lines of YAML) that are nearly identical — differing only in
platform (android/ios), flavor (prod/qa/dev), build type (apk/appbundle/ipa),
build mode (release/debug), and simulator post-processing.

`melos-rs build` replaces this with a declarative config block and a single
CLI command with combinable flags.

## Problem

The script duplication follows a combinatorial pattern:

```
platforms (2) x flavors (3) x variants (device/simulator) = 12 base scripts
+ composite scripts (build:all, build:all-simulator, etc.)
+ version-bump composites (build:all:patch-version, etc.)
= 24 scripts total
```

Each script is essentially:

```
flutter build <type> -t lib/main_<flavor>.dart --<mode> --flavor <flavor> [extra-args]
```

The scripts share the same `packageFilters` (`flutter: true`, `dirExists:
android|ios`) and the same exec flags (`--file-exists="pubspec.yaml" -c 1
--fail-fast`). Only the 4-5 parameters above vary.

Simulator scripts add a post-build step:
- **Android**: `bundletool build-apks --mode=universal` + `unzip` to extract a
  universal APK from the AAB
- **iOS**: `xcodebuild -sdk iphonesimulator` to produce a `.app` file from the
  archive

## Decision

Introduce a `command.build` config section and a `melos-rs build` CLI command
that eliminates the combinatorial script explosion.

### Why a config block instead of just CLI flags

Pure CLI flags (e.g., `melos-rs build --android --flavor prod --target
lib/main_prod.dart --mode release`) would require users to remember entry
points and modes per flavor. The config block declares these once, and the CLI
selects from them by name:

```yaml
command:
  build:
    flavors:
      prod:
        target: lib/main_prod.dart
        mode: release
      qa:
        target: lib/main_qa.dart
        mode: debug
```

```bash
# User only needs to know the flavor name
melos-rs build --android --flavor qa
```

### Why not extend Melos script syntax

Melos scripts are shell commands. Making them "smarter" (e.g., templated
scripts with variable substitution for flavor/platform) would create a
mini-language inside YAML strings. A structured config block is more
discoverable, validatable at parse time, and composable via CLI flags.

### Why simulator config is a command template

Simulator post-build steps are inherently platform-specific and vary across
teams (some use `bundletool`, some use Gradle tasks, some skip it entirely).
A command template with placeholders (`{aab_path}`, `{flavor}`, etc.) gives
teams control without melos-rs needing to understand every build tool:

```yaml
android:
  simulator:
    enabled: true
    command: >-
      bundletool build-apks --overwrite --mode=universal
      --bundle={aab_path} --output={output_dir}/{flavor}-unv.apks
      && unzip -o {output_dir}/{flavor}-unv.apks universal.apk -d {output_dir}
```

This avoids encoding Flutter/Gradle/Xcode output path conventions into
melos-rs binary, which would break when those tools change.

## Config Design

```yaml
command:
  build:
    flavors:
      <name>:
        target: <path>          # -t flag (entry point)
        mode: release|debug|profile

    defaultFlavor: <name>       # used when --flavor is omitted

    android:
      types: [appbundle, apk]   # flutter build <type>
      defaultType: appbundle
      extraArgs: []             # appended to all android builds
      simulator:
        enabled: true|false
        command: <template>     # placeholders: {aab_path}, {output_dir}, {flavor}, {mode}

    ios:
      extraArgs: []             # e.g., ["--export-options-plist", "ios/runner/exportOptions.plist"]
      simulator:
        enabled: true|false
        command: <template>     # placeholders: {flavor}, {mode}, {configuration}

    packageFilters:             # same as script packageFilters
      flutter: true

    hooks:
      pre: <command>
      post: <command>
```

### Config precedence

1. CLI flags override everything (`--flavor qa` overrides `defaultFlavor`)
2. Config defaults apply when CLI flags are absent
3. Platform-specific `extraArgs` are always appended
4. `packageFilters` from config are merged with CLI `--scope`/`--ignore` (CLI wins)

## CLI Design

```
melos-rs build [OPTIONS]

PLATFORMS:
    --android              Build for Android
    --ios                  Build for iOS
    --all                  Build for all platforms (default when none specified)

FLAVORS:
    --flavor <NAME>        Flavor to build (repeatable; default: config defaultFlavor)

ANDROID:
    --type <TYPE>          apk | appbundle (default: config defaultType)
    --simulator            Run simulator post-build (bundletool extraction)

IOS:
    --simulator            Run simulator post-build (xcodebuild)
    --export-options-plist <PATH>  Override export options plist

GENERAL:
    --dry-run              Print commands without executing
    --fail-fast            Stop on first failure
    -c, --concurrency <N>  Max concurrent builds (default: 1)
    --version-bump <TYPE>  Bump version before build: patch | minor | major
    --build-number-bump    Increment build number before build
    --scope <GLOB>         Filter packages by name
    --ignore <GLOB>        Exclude packages by name
```

### Execution order

1. Pre-hook (if configured)
2. Version bump (if `--version-bump` or `--build-number-bump`)
3. For each platform (android first, then ios when `--all`):
   a. Filter packages (`flutter: true` + `dirExists: <platform>` + CLI filters)
   b. For each flavor:
      - Build: `flutter build <type> -t <target> --<mode> --flavor <flavor> [extraArgs]`
      - Simulator post-build (if `--simulator` and configured)
4. Post-hook (if configured)

### Command assembly

The core function assembles a `flutter build` command from structured config:

```rust
fn build_flutter_command(
    platform: Platform,
    build_type: &str,       // "apk", "appbundle", "ipa"
    flavor: &FlavorConfig,
    flavor_name: &str,
    extra_args: &[String],
) -> String {
    // -> "flutter build appbundle -t lib/main_prod.dart --release --flavor prod"
}
```

This is testable without running Flutter — unit tests verify command string
assembly for all platform/flavor/mode combinations.

## Alternatives Considered

### 1. Script templates with variable interpolation

Add `${FLAVOR}`, `${MODE}`, `${TARGET}` variables to script `run:` strings,
expanded at runtime. Rejected because:
- Requires a custom template language inside YAML strings
- No compile-time validation of variable names
- Doesn't reduce the number of scripts (each combo still needs a script entry)
- Simulator post-build steps can't be expressed as simple variable substitution

### 2. Matrix builds in scripts

Add a `matrix:` key to scripts (like GitHub Actions):

```yaml
build:android:
  matrix:
    flavor: [prod, qa, dev]
    type: [apk, appbundle]
  run: flutter build ${type} -t lib/main_${flavor}.dart ...
```

Rejected because:
- Still requires template syntax in run strings
- Matrix expansion is harder to reason about than explicit CLI flags
- Doesn't naturally handle the asymmetry between platforms (iOS has no
  `--type`, Android has no `--export-options-plist`)

### 3. No build command — keep scripts

Keep the current script-based approach. Rejected because:
- 170 lines of duplicated YAML is a maintenance burden
- Adding a new flavor requires touching 12+ scripts
- No input validation — typos in `--flavor` silently produce wrong builds
- This is the exact kind of boilerplate a monorepo tool should eliminate

## Implementation Plan

Four batches, each independently testable:

| Batch | Scope | Key deliverables |
|-------|-------|-----------------|
| A | Config parsing + CLI args | `BuildCommandConfig`, `FlavorConfig`, `PlatformConfig`, `BuildArgs`, wiring into `Commands` enum |
| B | Core execution | `build_flutter_command()`, package filtering by platform, multi-flavor iteration, hooks, dry-run |
| C | Simulator post-build | Template placeholder expansion, bundletool/xcodebuild integration, artifact path resolution |
| D | Version bump integration | `--version-bump`, `--build-number-bump`, `--all` composite builds, progress reporting |

Each batch follows the project's TDD approach: unit tests for command assembly
and config parsing, integration tests for end-to-end behavior (with mocked
Flutter SDK).

## Impact

| Metric | Before (scripts) | After (build command) |
|--------|------------------:|----------------------:|
| YAML lines for builds | ~170 | ~30 |
| Scripts to maintain | 24 | 0 |
| Adding a new flavor | Touch 12+ scripts | Add 3-line config block |
| Input validation | None (shell errors) | Parse-time config validation |
| Discoverability | Read all scripts | `melos-rs build --help` |
