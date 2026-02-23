# 0003: Gang of Four Design Patterns — Analysis and Opportunities

## Context

This document reviews the melos-rs codebase against the 23 Gang of Four (GoF)
design patterns. The goal is to identify patterns already in use (often
implicitly, via Rust idioms), patterns that could be applied to reduce
duplication or improve extensibility, and patterns that were considered but
rejected as over-engineering at the current scale.

Rust's type system (enums, traits, closures) implements many GoF patterns at
the language level without the class hierarchies that the original patterns
assumed. Where Rust idioms achieve the same intent, we note the equivalence
rather than propose a refactor.

## Patterns Already in Use

### Command — enum dispatch in `main.rs`

The `Commands` enum (`cli.rs:150-194`) paired with the `match` in
`main.rs:120-135` is Rust's idiomatic Command pattern. Each variant carries its
own arguments struct, and each `commands::*::run()` function is the `execute()`
method:

```rust
match cli.command {
    Commands::Analyze(args) => commands::analyze::run(&workspace, args).await,
    Commands::Bootstrap(args) => commands::bootstrap::run(&workspace, args).await,
    // ... 11 more arms
}
```

This is superior to a trait-based Command hierarchy because:
- The compiler enforces exhaustive matching (adding a variant without a handler
  is a compile error).
- Each args type is distinct (no type erasure required).
- Zero-cost — no vtable indirection.

**Verdict: Keep as-is.** Enum dispatch is the right abstraction.

### Factory Method — config parsing and package construction

Two factory methods are central to the architecture:

1. `parse_config()` (`config/mod.rs:904-958`) dispatches on `ConfigSource` to
   produce a `MelosConfig` from either `melos.yaml` or `pubspec.yaml`:

   ```rust
   pub fn parse_config(source: &ConfigSource) -> Result<MelosConfig> {
       match source {
           ConfigSource::MelosYaml(_) => { /* direct deserialize */ }
           ConfigSource::PubspecYaml(_) => { /* wrapper deserialize + assemble */ }
       }
   }
   ```

2. `Package::from_path()` (`package/mod.rs:75`) reads a `pubspec.yaml` and
   constructs a `Package`, encapsulating YAML parsing, dependency extraction,
   and Flutter detection.

Additionally, the custom serde `Visitor` implementations for
`RepositoryConfig` (`config/mod.rs:347-413`) and `ExecEntry`
(`config/script.rs:94-124`) are Factory Methods that dispatch between string
and map input forms.

**Verdict: Keep as-is.** These are clean, idiomatic factory methods.

### Strategy — enum variant dispatch on `ScriptEntry`

`ScriptEntry` (`config/mod.rs:148-287`) has two variants (`Simple` and `Full`)
with 11 methods that dispatch via `match`. Each variant provides different
behavior — `Simple` returns defaults, `Full` delegates to `ScriptConfig`:

```rust
pub fn env(&self) -> &HashMap<String, String> {
    static EMPTY: LazyLock<HashMap<String, String>> = LazyLock::new(HashMap::new);
    match self {
        ScriptEntry::Simple(_) => &EMPTY,
        ScriptEntry::Full(config) => &config.env,
    }
}
```

A trait-based Strategy (`trait ScriptBehavior`) could reduce the repetitive
matching, but would sacrifice serde `#[serde(untagged)]` compatibility,
`Debug`/`Clone` derives, and zero-cost dispatch. The match arms are trivial
(one-line defaults for `Simple`).

**Verdict: Keep as-is.** Enum dispatch IS Rust's zero-cost Strategy pattern.

### Adapter — `From<&GlobalFilterArgs> for PackageFilters`

The `From` implementation (`config/filter.rs:83-121`) adapts CLI-layer types
(`GlobalFilterArgs` from clap) to domain-layer types (`PackageFilters`). This
is a clean boundary between the CLI and config domains:

```rust
impl From<&GlobalFilterArgs> for PackageFilters {
    fn from(args: &GlobalFilterArgs) -> Self { ... }
}
```

**Verdict: Keep as-is.** Textbook Adapter via Rust's `From` trait.

### Facade — `Workspace::find_and_load()`

`Workspace` (`workspace.rs:35-50`) hides eight subsystems behind a single
constructor call (`workspace.rs:60-149`): config discovery, YAML parsing,
validation, package discovery, nested workspace scanning, root-as-package
handling, ignore filtering, and SDK path resolution. Callers write:

```rust
let workspace = Workspace::find_and_load(cli.sdk_path.as_deref())?;
```

**Verdict: Keep as-is.** Well-implemented Facade with minimal public surface.

### Null Object — static empty collections

`ScriptEntry::env()` (`config/mod.rs:263-270`) returns a reference to a static
empty `HashMap` for `Simple` variants, avoiding `Option<&HashMap>` in the
return type. This simplifies all callers — they can iterate without checking
for `None`.

**Verdict: Keep as-is.** Clean application of the Null Object concept.

### Iterator — parallel package discovery

`discover_packages()` (`package/mod.rs:206-244`) uses a two-phase pipeline:
sequential glob iteration to collect candidates, then `rayon::par_iter()` for
parallel pubspec parsing. The stdlib and rayon iterators ARE the Iterator
pattern — composition of lazy, chainable transformations.

**Verdict: Keep as-is.** Fully idiomatic.

### Builder — clap derive macros

Clap's `#[derive(Parser)]` generates a builder under the hood. The
`#[command(flatten)]` attribute composes `GlobalFilterArgs` into each command's
args struct, which is the Composite + Builder pattern. No manual builder is
needed on top of this.

**Verdict: Keep as-is.** Clap IS the builder.

### Observer — watcher module

The file watcher (`watcher/mod.rs`) uses `tokio::sync::mpsc` channels to
implement Observer. `PackageChangeEvent` objects are sent from the watcher
(producer) and consumed in `exec.rs` and `run.rs` (observers) to trigger
command re-execution on file changes.

**Verdict: Keep as-is.** Channel-based Observer is idiomatic async Rust.

## Opportunities for Application

### 1. Template Method — command execution skeleton (highest duplication)

**Problem.** Seven commands (`analyze`, `format`, `test`, `clean`, `bootstrap`,
`publish`, `exec`) follow a near-identical 6-step skeleton:

```
1. Convert CLI args to PackageFilters, apply filters
2. Empty-check: print warning and return if no packages match
3. Print header with package count + per-package listing
4. Run pre-lifecycle hook (if configured)
5. Build command, create ProcessRunner, execute across packages
6. Count failures, print summary, run post-lifecycle hook
```

Evidence of duplication (filter + empty check alone):

| File | Filter lines | Empty-check lines |
|------|-------------|-------------------|
| `analyze.rs` | 36-42 | 44-47 |
| `format.rs` | 36-42 | 50-53 |
| `test.rs` | 48-54 | 56-59 |
| `clean.rs` | 16-22 | 30-33 |
| `exec.rs` | 55-61 | 63-66 |

The lifecycle hook extraction is also duplicated verbatim across 5 files — a
5-line `if let Some(hook) = workspace.config.command.as_ref().and_then(...)` chain:

```rust
if let Some(pre_hook) = workspace
    .config
    .command
    .as_ref()
    .and_then(|c| c.test.as_ref())    // only this line varies
    .and_then(|cfg| cfg.hooks.as_ref())
    .and_then(|h| h.pre.as_deref())
{
    runner::run_lifecycle_hook(pre_hook, "pre-test", &workspace.root_path, &[]).await?;
}
```

**Proposed approach.** In Rust, Template Method is best expressed as a
higher-order function rather than an inheritance hierarchy:

```rust
// Shared utility in commands/mod.rs
async fn run_filtered_command(
    workspace: &Workspace,
    filters: &PackageFilters,
    label: &str,
    pre_hook: Option<&str>,
    post_hook: Option<&str>,
    execute: impl AsyncFn(&[Package]) -> Result<usize>,  // returns failure count
) -> Result<()> {
    let packages = apply_filters_with_categories(...)?;
    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }
    print_header(label, &packages);
    if let Some(hook) = pre_hook {
        runner::run_lifecycle_hook(hook, &format!("pre-{label}"), ...)?;
    }
    let failed = execute(&packages).await?;
    if let Some(hook) = post_hook {
        runner::run_lifecycle_hook(hook, &format!("post-{label}"), ...)?;
    }
    if failed > 0 { bail!(...) }
    Ok(())
}
```

A companion `Workspace::hook()` method would simplify hook extraction:

```rust
impl Workspace {
    pub fn hook(&self, command: &str, phase: &str) -> Option<&str> { ... }
}
// Usage: workspace.hook("test", "pre")
```

**Impact:** Eliminates ~15-20 lines of boilerplate per command (7 commands),
totaling ~100-140 lines of duplication removed. Each command's `run()` function
would shrink to its unique logic (building the command string, SDK splitting,
command-specific flags).

**Risk:** Some commands have unique steps interleaved with the skeleton (e.g.,
`test.rs` filters packages without a `test/` directory, `clean.rs` handles
deep clean separately). The utility function must be flexible enough to
accommodate these — likely via the closure parameter.

**Priority: Medium.** The duplication is real but stable (commands rarely
change structure). Worth doing when adding the next command or during a
refactoring batch.

### 2. Observer — ProcessRunner event handler (strongest extensibility gain)

**Problem.** `ProcessRunner::run_in_packages_with_progress()`
(`runner/mod.rs:127-272`) mixes execution logic with presentation. Output
formatting (`runner/mod.rs:234-249`) is hardcoded:

```rust
// Hardcoded colored output — cannot be swapped for JSON, silent, or log-file output
if success {
    println!("{} {}", prefix, "SUCCESS".green());
} else {
    eprintln!("{} {}", prefix, "FAILED".red());
}
```

The `output_lock` mutex (`runner/mod.rs:142`) exists solely to serialize
presentation — an artifact of mixed concerns. If a caller wanted JSON output
(for CI), structured logging, or silent-for-tests mode, they would need to
modify `ProcessRunner` internals.

**Proposed approach.** Extract a trait for process lifecycle events:

```rust
trait ProcessEventHandler: Send + Sync {
    fn on_start(&self, package: &str);
    fn on_output(&self, package: &str, line: &str, is_stderr: bool);
    fn on_complete(&self, package: &str, success: bool, duration: Duration);
}

struct ColoredConsoleHandler { /* current behavior */ }
struct JsonHandler { /* structured output for CI */ }
struct SilentHandler;  /* for tests */
```

`ProcessRunner` would accept a `&dyn ProcessEventHandler` parameter, and the
current colored output would become `ColoredConsoleHandler`.

**Impact:** Decouples execution from presentation. Enables JSON output mode
(useful for CI integration), silent mode (useful for testing), and custom
reporters without modifying the runner. Also improves testability — tests can
assert on events rather than capturing stdout.

**Risk:** Adds a trait + 1-2 implementations. The `on_output` callback must
handle buffering (currently done inline). Moderate refactor, roughly 50 lines
of new code + 30 lines removed from the runner.

**Priority: Medium.** Valuable when JSON output or CI integration is on the
roadmap. Not urgent for current feature set.

### 3. Strategy — version resolution strategies

**Problem.** `version.rs` contains a multi-branch conditional (approximately
lines 1196-1331) selecting how versions are resolved:

- Manual version entry (interactive)
- `--graduate` (promote prerelease to stable)
- `--manual-version` (user-specified per-package)
- Conventional commits (automated bump)
- Default/fallback behavior

Each branch produces `Vec<(&Package, String)>` (package-to-new-version
mapping) but uses different logic to compute it.

**Proposed approach.** Extract a trait:

```rust
trait VersionResolver {
    fn resolve(&self, packages: &[Package], ...) -> Result<Vec<(&Package, String)>>;
}
struct GraduateResolver;
struct ManualResolver { versions: HashMap<String, String> }
struct ConventionalCommitResolver;
```

**Impact:** Each strategy becomes independently testable. Adding a new
resolution mode (e.g., calendar versioning) would not require modifying the
existing if-else chain.

**Risk:** The current branches share state (git history, changelog context)
that would need to be threaded through the trait. The refactor is non-trivial
and the current code works correctly.

**Priority: Low.** The version command is the most complex module but changes
infrequently. The if-else chain is long but each branch is locally
comprehensible. Defer unless a new resolution mode is needed.

### 4. Chain of Responsibility — package filter chain

**Problem.** `matches_filters()` (`package/filter.rs:109-191`) checks 9
filter predicates sequentially. Each `if let` block tests one filter
dimension and short-circuits on `false`:

```rust
fn matches_filters(pkg: &Package, filters: &PackageFilters) -> bool {
    if let Some(ref scopes) = filters.scope { ... if !matches_any { return false; } }
    if let Some(ref ignores) = filters.ignore { ... if matches_any { return false; } }
    if let Some(flutter) = filters.flutter && pkg.is_flutter != flutter { return false; }
    // ... 6 more checks
    true
}
```

**Proposed approach.** Model each filter as a handler:

```rust
trait PackageFilter {
    fn matches(&self, pkg: &Package) -> bool;
}
struct ScopeFilter(Vec<String>);
struct FlutterFilter(bool);
// compose into Vec<Box<dyn PackageFilter>>
```

**Impact:** Each filter becomes independently unit-testable. New filters
require no modification to existing code (Open/Closed principle).

**Risk:** Adds ~9 structs, a trait, and a composition builder. The current
function is ~80 lines and all 9 filters are visible at a glance. The filter
set is determined by `PackageFilters` and evolves slowly. The abstraction cost
outweighs the benefit at 9 checks.

**Priority: Low.** Revisit if filter count exceeds ~15 or if filters need to
be user-composable (e.g., via config file). At current scale, the imperative
approach is clearer.

### 5. Builder — `ChangelogOptions` construction

**Problem.** `ChangelogOptions` is constructed identically 3 times in
`version.rs` (approximately lines 1442-1451, 1474-1483, 1516-1525) with the
same field mapping from `VersionCommandConfig`.

**Proposed approach.** Add a `ChangelogOptions::from_config()` factory method
or implement `From<&VersionCommandConfig>`:

```rust
impl From<&VersionCommandConfig> for ChangelogOptions {
    fn from(cfg: &VersionCommandConfig) -> Self { ... }
}
```

**Impact:** Eliminates 3x ~10-line copy-paste blocks. One-line change at each
call site.

**Risk:** Negligible. Pure deduplication.

**Priority: Low.** Small quality-of-life improvement. Fold into the next
version-command batch.

## Patterns Considered and Rejected

| Pattern | Where considered | Why rejected |
|---------|-----------------|--------------|
| **Abstract Factory** | Config parsing (`MelosYaml` vs `PubspecYaml`) | Only 2 formats. `match` is simpler than a factory hierarchy. |
| **Singleton** | Global state | Correctly absent. `Workspace` is created once in `main()` and threaded via `&`. No global mutable state exists. |
| **Decorator** | Filter pipeline in `apply_filters_with_categories()` | Pipeline is linear and fixed (not user-configurable). Decorator would add allocations and obfuscate the flow. |
| **Visitor** | Package discovery, nested workspace traversal | Only one traversal consumer exists. Visitor separates traversal from action, but there is only one action. |
| **Composite** | `PackageFilters::merge()` | Merge produces a flat value, not a tree. Composite requires recursive structure. |
| **State** | `Verbosity` enum | Simple 3-variant value type used as a config parameter. No state transitions occur. |
| **Prototype** | Package/config cloning | Rust's `Clone` derive covers this natively. No special prototype registry needed. |
| **Flyweight** | Shared package references | Packages are already passed by `&` reference. `Arc` is used only where async tasks require owned data. |
| **Mediator** | Cross-command coordination | Commands are independent — no cross-command communication exists or is needed. |
| **Bridge** | Separating abstraction from implementation | The codebase has a single implementation target (CLI). No platform-specific backends exist. |

## Summary

| Pattern | Status | Priority | Action |
|---------|--------|----------|--------|
| Command | In use (enum dispatch) | — | Keep |
| Factory Method | In use (parse_config, from_path, serde Visitors) | — | Keep |
| Strategy | In use (ScriptEntry enum dispatch) | — | Keep |
| Adapter | In use (From trait) | — | Keep |
| Facade | In use (Workspace) | — | Keep |
| Null Object | In use (static empty collections) | — | Keep |
| Iterator | In use (stdlib + rayon) | — | Keep |
| Builder | In use (clap derives) | — | Keep |
| Observer | In use (watcher channels) | — | Keep |
| **Template Method** | **Opportunity** | **Medium** | Extract `run_filtered_command()` utility + `Workspace::hook()` |
| **Observer (ProcessRunner)** | **Opportunity** | **Medium** | Extract `ProcessEventHandler` trait from runner |
| **Strategy (version)** | **Opportunity** | **Low** | Extract `VersionResolver` trait |
| **Chain of Responsibility** | **Opportunity** | **Low** | Defer — 9 filters, function is 80 lines |
| **Builder (ChangelogOptions)** | **Opportunity** | **Low** | Add `From<&VersionCommandConfig>` |

The codebase already uses 9 GoF patterns idiomatically through Rust's type
system. Two medium-priority opportunities exist (Template Method for command
boilerplate, Observer for runner output). Three low-priority opportunities are
noted for future consideration. Ten patterns were explicitly evaluated and
rejected as unnecessary at the current scale.
