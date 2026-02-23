# 0007: TUI Frontend with ratatui

## Context

melos-rs has a clean core/CLI split (Phase 1-3). `melos-core` is a library
crate with zero terminal dependencies. All business logic (config parsing,
package discovery, command execution, version management) emits structured
`Event` values through `tokio::sync::mpsc` channels. The CLI subscribes to
these events and renders them as colored text + progress bars via `colored`
and `indicatif`.

This architecture was designed from the start to support a second frontend.
The TUI is that frontend.

## Problem

The CLI is excellent for CI and scripted workflows. But for day-to-day
development in a Flutter monorepo, developers want:

1. **At-a-glance workspace overview** -- see all packages, their versions,
   SDK types, and health status without running multiple commands.
2. **Interactive command execution** -- pick a command or script, see live
   per-package progress, and scroll through output without losing context.
3. **Persistent dashboard** -- keep the workspace state visible while
   commands run, instead of the CLI's scroll-and-forget model.
4. **Keyboard-driven workflow** -- navigate packages, trigger commands, and
   inspect results without reaching for the mouse or remembering flag syntax.

A TUI provides all of this in a terminal-native interface that works over
SSH, in tmux, and alongside the editor.

## Decision

Build `melos-tui` as a third crate in the Cargo workspace, consuming
`melos-core` exactly as `melos-cli` does. Use `ratatui` (0.30) with the
`crossterm` backend for cross-platform terminal rendering.

### Why ratatui + crossterm

| Option | Pros | Cons |
|--------|------|------|
| **ratatui + crossterm** | Active ecosystem (1600+ GitHub stars), rich widget library, immediate-mode rendering, cross-platform, Rust edition 2021+ | Learning curve for layout system |
| cursive | Callback-based, simpler for forms | Smaller ecosystem, retained-mode doesn't fit event streaming |
| raw crossterm only | Full control | Enormous boilerplate for layouts, tables, scrolling |
| egui (terminal) | Familiar if coming from GUI | Not terminal-native, heavier deps |

ratatui is the clear winner for terminal UIs in Rust. Its immediate-mode
model (`render on every frame`) maps naturally to our event-driven
architecture: core events update `App` state, each frame reads that state
and renders widgets.

### Why a separate binary, not a mode flag

Two options were considered:

1. `melos-rs --tui` flag on the existing binary
2. `melos-tui` as a separate binary

Option 2 was chosen because:
- Keeps `melos-cli` dependency-free of ratatui/crossterm (faster compile,
  smaller binary for CI)
- Different binaries can have different CLI args (TUI has no subcommands)
- Users install only what they need (`cargo install melos-rs` for CLI,
  `cargo install melos-tui` for TUI)
- No risk of TUI deps breaking CI-focused CLI builds

## Architecture

### Crate layout

```
crates/
  melos-tui/
    Cargo.toml          # melos-core + ratatui + crossterm + tokio
    src/
      main.rs           # terminal init, event loop, teardown
      app.rs            # App state machine + event handlers
      ui.rs             # widget layout and rendering
      views/
        packages.rs     # package list table view
        commands.rs     # command/script picker view
        execution.rs    # live execution panel
        results.rs      # scrollable results view
        health.rs       # health dashboard view
```

### App state machine

```
                  Enter (run command)
    Idle ---------------------------------> Running
     ^                                        |
     |          command finishes              v
     +------- Done <--------------------------+
     |  Esc    ^                              |
     +---------+        Esc (cancel)          |
               +------------------------------+
```

| State | Description | Key bindings |
|-------|-------------|-------------|
| `Idle` | Workspace loaded. Package list + command picker visible. | arrows, tab, enter, q, /, ? |
| `Running` | Command executing. Live progress panel. | Esc (cancel), scroll |
| `Done` | Results displayed. Scrollable output. | Esc (back to Idle), arrows (scroll) |

### Dual event loop

The TUI must handle two independent event sources concurrently:

1. **Terminal events** (crossterm) -- keyboard input, resize, focus
2. **Core events** (melos-core) -- PackageStarted, PackageFinished, Progress, etc.

Both feed into a unified `AppEvent` enum:

```rust
enum AppEvent {
    Terminal(crossterm::event::Event),
    Core(melos_core::events::Event),
    Tick, // periodic redraw (4 fps when idle, 15 fps when running)
}
```

The main loop uses `tokio::select!` to poll both sources. On each event,
`App` updates its state, then `ui::draw()` renders the current state to the
terminal frame. This is ratatui's standard immediate-mode pattern.

### Layout

```
+----------------------------------------------------------+
| melos-rs TUI  |  workspace: my_app  |  15 packages       |  <- Header
+----------------------------------------------------------+
|                           |                              |
|  Packages                 |  Output / Results            |
|  -----------------------  |  --------------------------- |
|  > app_core     1.2.0  F  |  [app_core] running...       |
|    app_ui       1.0.0  F  |  [app_core] SUCCESS          |
|    auth_api     0.3.1  D  |  [app_ui] running...         |
|    payment      2.1.0  F  |  [app_ui] FAILED             |
|    ...                    |  [auth_api] running...       |
|                           |                              |
+----------------------------------------------------------+
|  Commands: bootstrap | clean | exec | test | analyze     |  <- Command bar
+----------------------------------------------------------+
|  q:quit  tab:switch  enter:run  /:filter  ?:help         |  <- Footer
+----------------------------------------------------------+
```

Two-column layout:
- **Left panel (40%)**: Package list table or command/script picker
  (tab to toggle)
- **Right panel (60%)**: Output stream during execution, results when done,
  or health dashboard

The header shows workspace name, config source, and package count. The
footer shows context-sensitive key bindings.

### Core integration pattern

The TUI calls core functions identically to how the CLI does:

```rust
// 1. Load workspace (same as CLI)
let workspace = Workspace::find_and_load(None)?;

// 2. Apply filters, create opts (same as CLI, minus clap)
let packages = apply_filters(&workspace, &filters)?;
let opts = FormatOpts { line_length: 80, .. };

// 3. Create event channel
let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

// 4. Spawn core command in background task
let handle = tokio::spawn(async move {
    melos_core::commands::format::run(&packages, &workspace, &opts, Some(&tx)).await
});

// 5. TUI event loop consumes rx alongside terminal events
loop {
    tokio::select! {
        Some(core_event) = rx.recv() => app.handle_core_event(core_event),
        Ok(term_event) = term_rx.recv() => app.handle_terminal_event(term_event),
        _ = tick.tick() => {},
    }
    terminal.draw(|f| ui::draw(f, &app))?;
    if app.should_quit() { break; }
}
```

### Widget reuse

ratatui provides all the widgets we need out of the box:

| View | Widget | Notes |
|------|--------|-------|
| Package list | `Table` | Sortable columns, highlight selected row |
| Command picker | `List` | With selection state |
| Output stream | `Paragraph` with `Line` spans | Auto-scroll, colored per-package |
| Progress | `Gauge` | Maps to core `Progress` event |
| Health dashboard | `Table` + `Paragraph` | Tabbed: drift / fields / SDK |
| Filter input | `Paragraph` as input | `/` activates, Esc cancels |
| Help overlay | `Paragraph` in `Block` | `?` toggles, centered popup |

### Scrollback buffer

Output from command execution is stored in a `Vec<Line>` scrollback buffer
with a configurable maximum (default: 10,000 lines). The right panel shows
a window into this buffer. Arrow keys and Page Up/Down scroll through it.
When a new line arrives and the user hasn't scrolled up, the view
auto-follows the tail.

## Keyboard navigation

| Key | Context | Action |
|-----|---------|--------|
| `q` / `Ctrl+C` | Any | Quit |
| `Tab` | Idle | Toggle left panel: packages / commands |
| Up/Down | Any | Navigate list / scroll output |
| `Enter` | Idle (command selected) | Execute command |
| `Esc` | Running | Cancel execution (graceful) |
| `Esc` | Done | Return to Idle |
| `/` | Idle | Open filter input |
| `?` | Any | Toggle help overlay |
| `h` | Idle | Run health check |
| `1`-`5` | Idle | Quick-switch views (packages, commands, health, output, scripts) |
| `Page Up/Down` | Output visible | Scroll output by page |
| `Home/End` | Output visible | Jump to start/end of output |

## Testing strategy

TUI code is inherently harder to unit test than CLI code. The strategy:

1. **App state logic is testable** -- `App` struct methods
   (`handle_core_event`, `handle_terminal_event`) are pure state
   transitions. Feed events in, assert state out. No terminal needed.
2. **Widget rendering can use ratatui's `TestBackend`** -- render to a
   buffer, assert cell contents for critical views.
3. **Integration: manual verification** -- TUI visual behavior is verified
   by running against the test workspace. Automated screenshot testing is
   out of scope.

## Implementation plan

Six batches, each producing a runnable binary:

| Batch | Scope | Deliverables |
|-------|-------|-------------|
| A | Crate scaffolding | Cargo.toml, terminal setup/teardown, empty App, quit on `q` |
| B | Workspace + packages | Load workspace on startup, package list table, keyboard nav |
| C | Command picker + execution | Command/script list, tab switching, wire execution to core |
| D | Live progress + output | Per-package output streaming, progress gauge, scrollback |
| E | Results + health | Results panel, health dashboard, error display |
| F | Polish | Filter bar, help overlay, themes, edge cases |

See TODO.md Phase 4 for the full task breakdown.

## Alternatives considered

### 1. Web-based dashboard

Serve a local web UI with a REST/WebSocket API. Rejected because:
- Adds HTTP server dependency, opens network port
- Doesn't work over SSH without port forwarding
- Heavier than needed for a dev tool
- Breaks the "terminal-native" philosophy

### 2. Enhance CLI with interactive mode (dialoguer/inquire)

Add interactive prompts and selection menus to the existing CLI. Rejected
because:
- Still linear (one command at a time, output scrolls away)
- Can't show persistent state alongside command output
- Prompt libraries don't support real-time progress rendering
- We already have interactive prompts where needed (version confirmation)

### 3. VS Code extension

Build a VS Code extension consuming melos-core via WASM or subprocess.
Rejected because:
- IDE extensions are explicitly out of scope (AGENTS.md)
- Would require JavaScript/TypeScript bridge code
- Only serves VS Code users (not Vim, Emacs, terminal-only)
- Can be built later as a separate project using the same core events

## Consequences

**Positive:**
- Developers get a persistent workspace dashboard in the terminal
- Validates the core/CLI separation -- TUI is the first non-CLI consumer
- Event architecture proves its value for real-time rendering
- Package list + health at a glance reduces context-switching
- Works everywhere a terminal works (SSH, tmux, CI debug)

**Negative:**
- Third crate increases workspace compile time
- ratatui adds ~15 transitive dependencies
- TUI testing is harder than CLI testing (visual behavior)
- Maintaining two frontends doubles the presentation-layer surface area

**Mitigations:**
- TUI crate compiles independently; `cargo install melos-rs` skips it
- ratatui deps are well-maintained and widely used
- App state tests cover logic without visual rendering
- Both frontends consume identical core events -- logic drift is impossible
