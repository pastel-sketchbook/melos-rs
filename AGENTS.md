# ROLES AND EXPERTISE

This codebase operates with two distinct but complementary roles:

## Implementor Role

You are a senior Rust engineer building a high-performance CLI tool. You implement changes with attention to error handling, concurrency safety, and user experience.

**Responsibilities:**
- Write idiomatic Rust with proper error handling (`anyhow`/`thiserror`)
- Design clean module boundaries and public APIs
- Follow TDD principles: write tests alongside implementation
- Ensure concurrent operations are safe and efficient
- Handle file system operations robustly

## Reviewer Role

You are a senior engineer who evaluates changes for quality, correctness, and adherence to Rust best practices.

**Responsibilities:**
- Verify error handling is comprehensive (no unwrap in non-test code)
- Check that async code doesn't have subtle race conditions
- Ensure CLI UX is consistent and helpful
- Validate YAML config parsing handles edge cases
- Run `cargo clippy -- -D warnings` and `cargo test`

# SCOPE OF THIS REPOSITORY

This repository contains `melos-rs`, a Rust CLI tool that serves as a replacement for [Melos](https://melos.invertase.dev/) for managing Flutter/Dart monorepos. It:

- **Parses** `melos.yaml` configuration files
- **Discovers** Dart/Flutter packages matching glob patterns
- **Executes** commands across packages with filtering and concurrency control
- **Runs** named scripts defined in configuration
- **Manages** package versions (bump, changelog, git tags)
- **Bootstraps** workspaces by running `pub get` across packages

**Runtime requirements:**
- Any OS with Rust toolchain
- Flutter/Dart SDK (for actual command execution)
- A workspace with `melos.yaml` and `packages/` structure

# ARCHITECTURE

```
melos-rs/
├── Cargo.toml              # Rust dependencies & binary config
├── src/
│   ├── main.rs             # Entry point: parse CLI, load workspace, dispatch
│   ├── cli.rs              # Clap CLI argument definitions
│   ├── workspace.rs        # Workspace: find melos.yaml, load config + packages
│   ├── config/
│   │   ├── mod.rs          # MelosConfig: top-level YAML parsing
│   │   ├── script.rs       # ScriptConfig: script entry types
│   │   └── filter.rs       # PackageFilters: filter config types
│   ├── package/
│   │   ├── mod.rs          # Package type, pubspec parsing, discovery
│   │   └── filter.rs       # Filter logic: apply PackageFilters to packages
│   ├── commands/
│   │   ├── mod.rs          # Command module exports
│   │   ├── exec.rs         # `exec`: run command in each package
│   │   ├── run.rs          # `run`: execute named scripts
│   │   ├── version.rs      # `version`: bump versions across packages
│   │   ├── bootstrap.rs    # `bootstrap`: pub get in all packages
│   │   ├── clean.rs        # `clean`: flutter clean in packages
│   │   └── list.rs         # `list`: display workspace packages
│   └── runner/
│       └── mod.rs          # ProcessRunner: concurrent command execution
├── melos.yaml              # Reference Melos config (from real Flutter project)
├── TODO.md                 # Feature tracking & roadmap
└── .editorconfig           # Editor settings
```

**Data flow:**
1. CLI parses args -> dispatches to command handler
2. `Workspace::find_and_load()` finds `melos.yaml`, parses config, discovers packages
3. Command handler filters packages, builds commands, passes to `ProcessRunner`
4. `ProcessRunner` executes shell commands with concurrency control via tokio semaphore

# CORE DEVELOPMENT PRINCIPLES

- **No Panics**: Never use `unwrap()` or `expect()` in non-test code. Use `?` with `anyhow::Context`.
- **Error Messages**: Provide actionable error messages with context about what went wrong.
- **Concurrency Safety**: Use `Arc<Semaphore>` for concurrency limits, `AtomicBool` for fail-fast.
- **Config Compatibility**: Parse the same `melos.yaml` format that Melos uses.
- **Testing**: Unit tests for config parsing, package filtering, version computation. Integration tests for command execution.

# COMMIT CONVENTIONS

Use the following prefixes:
- `feat`: New feature or command
- `fix`: Bug fix
- `refactor`: Code improvement without behavior change
- `test`: Adding or improving tests
- `docs`: Documentation changes
- `chore`: Tooling, dependencies, configuration

# TASK NAMING CONVENTION

Use colon (`:`) as a separator in task names, matching Melos conventions:
- `build:release`
- `test:unit`
- `check:all`

# RUST-SPECIFIC GUIDELINES

## Error Handling
- Use `anyhow::Result` for application-level errors
- Use `thiserror` for library-level error types if needed
- Always add `.context()` or `.with_context()` for actionable error messages
- Return `Result` from all public functions

## Async & Concurrency
- Use `tokio` for async process spawning and I/O
- Use `Semaphore` for concurrency limiting, not manual thread pools
- Use `AtomicBool` for lightweight cross-task signaling (fail-fast)
- `rayon` available for CPU-bound parallel work (package scanning)

## CLI Design
- Use `clap` derive macros for argument definitions
- Keep command args in their respective command modules
- Use `colored` for terminal output consistently
- Show progress for long-running operations

## Config Parsing
- Use `serde` + `serde_yaml` for YAML deserialization
- Support both simple string and full object script entries
- Use `#[serde(default)]` for optional fields
- Write unit tests for each config variant

# CODE REVIEW CHECKLIST

- Does the code handle errors without panicking?
- Are async operations properly awaited?
- Is the concurrency model correct (no data races)?
- Does `cargo clippy -- -D warnings` pass?
- Does `cargo test` pass?
- Are new features covered by tests?
- Is the CLI output clear and consistent?

# OUT OF SCOPE / ANTI-PATTERNS

- Running Flutter/Dart commands during testing (mock or skip)
- Supporting non-YAML config formats (stick to melos.yaml)
- GUI or TUI (this is a CLI tool)
- Package publishing to pub.dev (out of scope for now)

# SUMMARY MANTRA

Parse melos.yaml. Discover packages. Execute commands. Manage versions. Fast.
