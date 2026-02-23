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

## Opportunities Evaluated — Cost/Benefit Analysis

Five potential GoF applications were identified. Each was projected for
lines-of-code impact before deciding whether to apply.

| # | Pattern | Added | Removed | Net | Verdict |
|---|---------|------:|--------:|----:|---------|
| 1 | Template Method (`Workspace::hook()`) | +10 | -54 | **-44** | **Applied** — pure deduplication |
| 2 | Observer (`ProcessEventHandler` trait) | +64 | -35 | **+29** | Deferred — adds LOC for extensibility not yet needed |
| 3 | Strategy (`VersionResolver` trait) | +58 | -8 | **+50** | Deferred — branches share state, trait adds ceremony |
| 4 | Chain of Responsibility (filter chain) | +105 | -80 | **+25** | Deferred — 9 structs to replace an 80-line function |
| 5 | Builder (`ChangelogOptions` factory) | +15 | -30 | **-15** | **Applied** — pure deduplication |

**Decision rule:** Only apply patterns that reduce or hold LOC parity.
Patterns that add LOC are documented for future consideration when a concrete
need arises (e.g., JSON output mode would justify #2, a new version resolution
mode would justify #3, doubling the filter count would justify #4).

### Applied: Template Method — `Workspace::hook()` (net -44 lines)

**Problem.** Lifecycle hook extraction is duplicated across 5 command files
(bootstrap, clean, test, publish, version) — 10 call sites total, each a
5-7 line `if let Some(hook) = workspace.config.command.as_ref().and_then(...)`
chain where only the command-name accessor varies:

```rust
// Before: 7 lines per hook site (× 10 sites = 70 lines)
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

// After: 1 line per hook site
if let Some(hook) = workspace.hook("test", "pre") {
    runner::run_lifecycle_hook(hook, "pre-test", &workspace.root_path, &[]).await?;
}
```

The full `run_filtered_command()` skeleton was not extracted because commands
have too much unique logic interleaved (test filters for `test/` dir, clean
handles deep clean, publish has confirmation prompt). Forcing them into a
shared closure would obscure rather than clarify.

### Applied: Builder — `ChangelogOptions` factory (net -15 lines)

**Problem.** `ChangelogOptions` is constructed identically 3 times in
`version.rs` (lines 1442-1451, 1474-1483, 1516-1525):

```rust
// Before: 10-line block × 3 = 30 lines
let changelog_opts = ChangelogOptions {
    include_body,
    only_breaking_bodies,
    include_hash,
    include_scopes,
    repository: repo,
    include_types: changelog_include_types.as_deref(),
    exclude_types: changelog_exclude_types.as_deref(),
    include_date,
};

// After: 1 closure call × 3 = 3 lines
let changelog_opts = make_opts();
```

Extracted as a closure capturing the shared local variables, replacing 30 lines
of copy-paste with 3 one-line calls plus a 12-line closure definition.

### Deferred: Observer — ProcessRunner event handler (+29 lines)

The runner mixes execution and presentation (`runner/mod.rs:234-249`). A
`ProcessEventHandler` trait would decouple them, enabling JSON output for CI
or silent mode for tests. However, no consumer currently needs this — the
colored console output is the only mode. The +29 line overhead is not justified
until a second output mode is needed.

**Trigger to revisit:** Adding `--output=json` to any command, or needing
silent execution in integration tests.

### Deferred: Strategy — version resolution (+50 lines)

The if/else chain (`version.rs:1196-1331`) selects among 5 version resolution
modes. A trait hierarchy would make each mode independently testable, but the
branches share significant state (git history, prerelease computation, eligible
packages). Threading this through a trait adds ~50 lines of struct definitions
and trait plumbing with no functional benefit.

**Trigger to revisit:** Adding a 6th resolution mode (e.g., calendar
versioning), at which point the if/else chain becomes unwieldy.

### Deferred: Chain of Responsibility — filter chain (+25 lines)

`matches_filters()` (`package/filter.rs:109-191`) is 80 lines checking 9
predicates. Decomposing into 9 filter structs + trait + builder would add 105
lines while removing 80 — a net increase that scatters logic currently visible
in one function across 10 files/types.

**Trigger to revisit:** Filter count exceeding ~15, or filters becoming
user-composable via config.

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

| Pattern | Status | Action |
|---------|--------|--------|
| Command | In use (enum dispatch) | Keep |
| Factory Method | In use (parse_config, from_path, serde Visitors) | Keep |
| Strategy | In use (ScriptEntry enum dispatch) | Keep |
| Adapter | In use (From trait) | Keep |
| Facade | In use (Workspace) | Keep |
| Null Object | In use (static empty collections) | Keep |
| Iterator | In use (stdlib + rayon) | Keep |
| Builder | In use (clap derives) | Keep |
| Observer | In use (watcher channels) | Keep |
| **Template Method** | **Applied** | `Workspace::hook()` — net -44 lines |
| **Builder (ChangelogOptions)** | **Applied** | Closure factory — net -15 lines |
| Observer (ProcessRunner) | Deferred | Revisit when JSON output needed |
| Strategy (version) | Deferred | Revisit when 6th resolution mode added |
| Chain of Responsibility | Deferred | Revisit when filter count exceeds ~15 |

The codebase uses 9 GoF patterns idiomatically through Rust's type system.
Two patterns were applied where they reduce LOC (net -59 lines combined).
Three were evaluated with line-count projections and deferred — each has a
concrete trigger condition documented above for future reconsideration.
Ten patterns were explicitly rejected as inapplicable at the current scale.
