# 0006: Core Library Extraction with Event-Based Architecture

## Context

melos-rs is a single binary crate where business logic (config parsing, package
discovery, command execution, version computation) and presentation (terminal
output, progress bars, colored text) live side by side. Every `run()` function
in `src/commands/` mixes both concerns: it computes what to do and prints results
as it goes.

This works well for the current CLI but prevents reuse of the core logic in
alternative frontends. A TUI (using ratatui) is the immediate motivation, but
the same separation would support LSP integration, CI tooling, or library
consumers in the future.

## Problem

Adding a TUI frontend today would require either:

1. **Duplicating logic** -- rewriting command orchestration in TUI code, keeping
   two implementations in sync.
2. **Screen-scraping** -- running the CLI as a subprocess and parsing its colored
   terminal output, which is fragile and lossy.
3. **Refactoring first** -- extracting shared logic into a library crate that
   both CLI and TUI depend on.

Option 3 is the only maintainable path.

## Decision

Extract core logic into a `melos-core` library crate. Use an **event-based
architecture** where core functions emit structured events through channels
rather than printing directly. Frontends subscribe to these events and render
them however they choose.

### Why events over return values

Two architectural options were evaluated:

**Option A: Return data structs.** Each command returns a result struct
(e.g. `AnalyzeResult { passed, failed, conflicts }`). The frontend renders it
after the command completes.

**Option B: Emit events through channels.** Each command sends structured events
as work progresses (`PackageStarted`, `PackageFinished`, `Progress`, `Warning`).
The frontend renders them in real time.

Option B was chosen because:

- **Streaming output is essential.** Commands like `bootstrap`, `exec`, and
  `analyze` run for seconds to minutes across many packages. Users need live
  progress, not a blank screen followed by a dump. The TUI needs incremental
  widget updates; the CLI needs incremental terminal lines.
- **The pattern already exists.** `watcher/mod.rs` already emits
  `PackageChangeEvent` through `mpsc::UnboundedSender`. Extending this to all
  commands is a natural evolution, not a new paradigm.
- **Decouples pacing from rendering.** Core emits events at its own pace.
  CLI prints each event immediately. TUI batches them into frame renders at
  60fps. Neither constrains the other.
- **Progress bars become frontend concerns.** Core emits
  `Progress { completed, total }`. CLI renders it as an indicatif bar. TUI
  renders it as a gauge widget. Core never imports indicatif.

Option A would work for simple commands (`list`, `health`) but forces
long-running commands to either block until done (bad UX) or return iterators
(essentially reinventing channels with more ceremony). Since both short and long
commands need the same architecture, events are the unified answer.

## Architecture

### Crate layout

```
melos-rs/
  Cargo.toml                # Cargo workspace root
  crates/
    melos-core/             # library: zero terminal dependencies
      Cargo.toml            # depends on: tokio, serde, anyhow, glob, regex, notify
      src/
        lib.rs              # public API surface
        workspace.rs        # Workspace loading (no println)
        config/             # MelosConfig, YAML parsing (moved as-is)
        package/            # Package model, discovery, filtering (moved as-is)
        commands/           # pure logic + event emission
        runner.rs           # ProcessRunner emitting events
        watcher.rs          # file watcher (already event-based)
        events.rs           # Event enum, shared types
    melos-cli/              # binary: current CLI, depends on melos-core
      Cargo.toml            # depends on: melos-core, clap, colored, indicatif
      src/
        main.rs             # clap dispatch
        cli.rs              # arg definitions
        render.rs           # event -> terminal output
    melos-tui/              # binary: ratatui frontend, depends on melos-core
      Cargo.toml            # depends on: melos-core, ratatui, crossterm
      src/
        main.rs             # terminal setup, event loop
        app.rs              # App state machine
        ui.rs               # widget layout and rendering
```

### Event enum

```rust
// melos-core/src/events.rs

pub enum Event {
    // -- Lifecycle --
    CommandStarted { command: String, package_count: usize },
    CommandFinished { command: String, duration: Duration },

    // -- Per-package progress --
    PackageStarted { name: String },
    PackageFinished { name: String, success: bool, duration: Duration },
    PackageOutput { name: String, line: String },

    // -- Aggregate progress --
    Progress { completed: u64, total: u64, message: String },

    // -- Diagnostics --
    Warning(String),
    Info(String),

    // -- Command-specific data --
    AnalyzeDryRun { entries: Vec<DryRunEntry> },
    ConflictDetected { pairs: Vec<ConflictPair> },
    VersionBumped { package: String, from: String, to: String },
    ListPackage { name: String, version: String, path: PathBuf, is_flutter: bool },
}
```

### Core function signature pattern

```rust
// melos-core/src/commands/analyze.rs

pub async fn analyze(
    workspace: &Workspace,
    opts: AnalyzeOpts,
    tx: mpsc::UnboundedSender<Event>,
) -> Result<AnalyzeSummary> {
    tx.send(Event::CommandStarted { ... });

    for pkg in &packages {
        tx.send(Event::PackageStarted { name: pkg.name.clone() });
        // ... run analysis ...
        tx.send(Event::PackageFinished { name, success, duration });
    }

    tx.send(Event::CommandFinished { ... });
    Ok(AnalyzeSummary { passed, failed })
}
```

### CLI render loop

```rust
// melos-cli/src/render.rs

pub fn render_events(mut rx: mpsc::UnboundedReceiver<Event>) {
    while let Some(event) = rx.blocking_recv() {
        match event {
            Event::PackageStarted { name } => println!("  {} {}", "->".cyan(), name),
            Event::Progress { completed, total, .. } => pb.set_position(completed),
            Event::Warning(msg) => eprintln!("{} {}", "WARN".yellow(), msg),
            // ...
        }
    }
}
```

### TUI render loop

```rust
// melos-tui/src/app.rs

pub fn handle_core_event(&mut self, event: Event) {
    match event {
        Event::PackageStarted { name } => self.running_packages.push(name),
        Event::PackageFinished { name, success, .. } => {
            self.running_packages.retain(|n| n != &name);
            self.results.push((name, success));
        }
        Event::Progress { completed, total, .. } => {
            self.progress = (completed, total);
        }
        // ... update state, ratatui redraws on next frame
    }
}
```

## Migration strategy

The extraction is designed to be incremental. Each phase produces a working
binary with no behavior change. See TODO.md for the full task breakdown.

- **Phase 1**: Cargo workspace + mechanical move of pure logic (config, package,
  helpers). CLI still works identically.
- **Phase 2**: Define Event enum. Refactor ProcessRunner to emit events. CLI
  subscribes and renders.
- **Phase 3**: Migrate command run() functions one at a time, starting with
  simple ones (list, clean, format) then complex ones (analyze, version).
- **Phase 4**: Build melos-tui consuming the same core.

## Consequences

**Positive:**
- Core is independently testable with no terminal dependencies
- TUI and CLI share identical business logic -- zero drift
- Event architecture enables future frontends (LSP, web, CI integrations)
- Progress reporting becomes consistent across all commands
- Core crate could be published for third-party tooling

**Negative:**
- Cargo workspace adds compile-time and dependency management overhead
- Every new command touches two crates (core logic + CLI rendering)
- Event enum grows with each command -- needs discipline to keep it focused
- Channel overhead is negligible but non-zero

**Mitigations:**
- Shared workspace dependencies via `[workspace.dependencies]` minimize duplication
- A `Command` trait or macro could reduce boilerplate for new commands
- Event enum uses a flat structure (no nested generics) for simplicity
