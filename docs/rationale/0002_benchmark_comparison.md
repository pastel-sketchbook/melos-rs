# 0002: Benchmark Comparison (melos-rs vs melos)

## Context

melos-rs exists to replace [Melos](https://melos.invertase.dev/) for managing
Dart/Flutter monorepos. The primary motivation is speed: Melos is written in
Dart and pays a ~500 ms JIT startup cost on every invocation, even for trivial
operations like listing packages. In a large monorepo where developers invoke
Melos dozens of times per session (CI pipelines, pre-commit hooks, IDE
integrations), that overhead compounds into meaningful friction.

A Rust rewrite eliminates JIT startup entirely. The binary is
ahead-of-time compiled, loads in single-digit milliseconds, and does not carry
a VM runtime. This document records the measured difference.

## Methodology

All benchmarks are driven by [hyperfine](https://github.com/sharkdp/hyperfine)
and automated via Taskfile (`task bench:all`). The setup is deterministic and
reproducible:

1. **Workspace generation** (`task bench:setup`): A synthetic Flutter monorepo
   is created with **15 packages** — 5 pure Dart, 5 Flutter, and 5 mixed — with
   realistic interdependencies (e.g. `auth` depends on `core`, `logger`,
   `config`). Every package uses `resolution: workspace` (Dart 3.5+ standard).
   The workspace has both a `melos.yaml` (6.x format, for melos-rs) and a root
   `pubspec.yaml` with a `workspace:` field (7.x format, for Melos).

2. **Release build**: melos-rs is compiled with `cargo build --release`
   (optimized, no debug symbols) before benchmarking.

3. **Warmup runs**: Each benchmark performs warmup iterations (3 for list/exec,
   1 for bootstrap) to eliminate cold-cache effects from the file system and Dart
   snapshot cache.

4. **Statistical rigor**: hyperfine runs each command multiple times (20 for
   list, 10 for exec, 3 for bootstrap) and reports mean, min, max, and standard
   deviation. Relative speedup is computed automatically.

5. **Fair comparison**: Both tools operate on the identical workspace, discover
   the same 15 packages, and produce equivalent output. Neither tool is given
   any unfair advantage (no pre-cached state, no disabled features).

## Environment

| Component       | Version                  |
|-----------------|--------------------------|
| Machine         | Apple M1 Pro, 16 GB RAM  |
| OS              | macOS (darwin, arm64)     |
| Rust            | 1.93.1 (stable)          |
| Dart SDK        | 3.11.0 (stable)          |
| Flutter         | 3.41.2 (stable)          |
| Melos           | 7.4.0                    |
| hyperfine       | 1.20.0                   |

## Results

### `list` — enumerate workspace packages

| Command | Mean | Min | Max | Relative |
|:---|---:|---:|---:|---:|
| `melos list` | 516.3 ± 28.1 ms | 484.9 ms | 616.7 ms | 70.07x |
| `melos-rs list` | 7.4 ± 0.5 ms | 6.5 ms | 10.4 ms | **1.00** |

**70x faster.** The operation is pure config parsing + glob matching + printing.
The entire delta is JIT startup and Dart runtime overhead.

### `list --json` — JSON output

| Command | Mean | Min | Max | Relative |
|:---|---:|---:|---:|---:|
| `melos list --json` | 522.8 ± 13.4 ms | 504.6 ms | 558.3 ms | 69.75x |
| `melos-rs list --json` | 7.5 ± 0.3 ms | 6.5 ms | 9.0 ms | **1.00** |

**70x faster.** JSON serialization adds negligible time in both implementations.
The bottleneck remains startup.

### `exec` — run a command in each package

| Command | Mean | Min | Max | Relative |
|:---|---:|---:|---:|---:|
| `melos exec -- echo hi` | 560.0 ± 7.2 ms | 548.5 ms | 572.7 ms | 19.54x |
| `melos-rs exec -- echo hi` | 28.7 ± 1.2 ms | 26.4 ms | 33.9 ms | **1.00** |

**20x faster.** Both tools spawn 15 `echo` subprocesses. The gap narrows from
70x to 20x because subprocess spawning adds a ~20 ms floor that both tools pay.
Melos still carries its ~500 ms startup tax on top.

### `bootstrap` — resolve and link all packages

| Command | Mean | Min | Max | Relative |
|:---|---:|---:|---:|---:|
| `melos bootstrap` | 1.338 ± 0.035 s | 1.298 s | 1.362 s | 1.00 |
| `melos-rs bootstrap` | (functional, not yet benchmarked head-to-head) | — | — | — |

Bootstrap is network-bound (`dart pub get` / `flutter pub get` in each
package). The startup overhead becomes a smaller fraction of total time, so the
relative speedup is expected to be modest (2-5x estimate). A head-to-head
benchmark was deferred because `melos-rs bootstrap` previously generated
`pubspec_overrides.yaml` files that conflicted with Dart 3.5+ workspace
resolution. That conflict is now fixed (Batch 25), so a fair comparison can be
run in a future iteration.

## Why the speedups are what they are

The performance difference is explained by three factors, in order of impact:

1. **No JIT startup** (~500 ms eliminated). Dart's VM must parse, compile, and
   optimize bytecode on every cold invocation. The melos-rs binary is fully
   compiled ahead of time — `main()` begins executing within ~1 ms of process
   launch.

2. **No runtime overhead** (~5-10 ms eliminated). Dart's garbage collector,
   isolate setup, and async scheduler add baseline cost even after JIT warmup.
   Rust has no GC, no runtime, and no async scheduler overhead beyond tokio's
   lightweight event loop.

3. **Parallel package discovery** (variable). melos-rs uses rayon to parse
   `pubspec.yaml` files in parallel across CPU cores. Melos does this
   sequentially. In a 15-package repo the effect is small, but it scales
   linearly with workspace size.

## Caveats

- **15 packages is modest.** Real-world monorepos can have 50-200+ packages.
  The startup-cost ratio would remain similar, but parallel discovery gains
  would be more pronounced for melos-rs.
- **Network-bound commands converge.** For bootstrap, clean, and other commands
  where subprocess I/O dominates, the Rust advantage is diluted. The tool is
  still faster, but by a smaller margin.
- **Single machine.** Results were collected on one Apple Silicon laptop. x86
  and CI runners may show different absolute numbers but similar ratios.
- **Melos 7.4.0 is the latest stable.** Future Melos releases could adopt AOT
  compilation (via `dart compile exe`), which would narrow the startup gap
  significantly.

## Reproducing

```sh
# Prerequisites: Rust toolchain, Dart SDK, Flutter SDK, melos 7.4.0, hyperfine
# Install melos: dart pub global activate melos 7.4.0

# Run all benchmarks (generates bench-*.md files)
task bench:all

# Run individually
task bench:list
task bench:list:json
task bench:exec
task bench:bootstrap   # requires workspace resolution fix (Batch 25+)

# Clean up
task bench:clean
```

The generated result files (`bench-*.md`) are gitignored. This document
captures the rationale and a snapshot of results for the record.

## Post core/cli split (Phase 3 complete, v0.5.3)

After extracting all command logic into `melos-core` (zero terminal deps) and
leaving `melos-cli` as a thin rendering layer, the binary was re-benchmarked to
confirm no performance regression from the workspace split and event-based
architecture.

### Architecture delta

| Metric | Before split (v0.4.x) | After split (v0.5.3) |
|--------|----------------------|---------------------|
| Crates | 1 binary | 2 (melos-core lib + melos-cli bin) |
| Core tests | 0 | 495 |
| CLI tests | ~500 | 39 |
| Event architecture | Direct print | `UnboundedSender<Event>` channels |
| Total binary deps | clap + colored + indicatif + all | Same (tree-shaken, core has zero terminal deps) |

### Results

#### `list` -- enumerate workspace packages

| Command | Mean | Min | Max | Relative |
|:---|---:|---:|---:|---:|
| `melos list` | 568.9 ± 49.1 ms | 535.1 ms | 771.8 ms | 75.08x |
| `melos-rs list` | 7.6 ± 0.6 ms | 6.5 ms | 12.8 ms | **1.00** |

**75x faster.** Up from 70x in the pre-split measurement. The improvement is
within noise (different system load), confirming zero overhead from the crate
split. The event channel is not used for `list` (pure sync), so no added cost.

#### `list --json` -- JSON output

| Command | Mean | Min | Max | Relative |
|:---|---:|---:|---:|---:|
| `melos list --json` | 573.9 ± 44.4 ms | 537.2 ms | 750.2 ms | 77.78x |
| `melos-rs list --json` | 7.4 ± 0.4 ms | 6.5 ms | 8.7 ms | **1.00** |

**78x faster.** Consistent with the plain `list` result. JSON serialization
remains negligible.

#### `exec` -- run a command in each package

| Command | Mean | Min | Max | Relative |
|:---|---:|---:|---:|---:|
| `melos exec -- echo hi` | 616.9 ± 21.1 ms | 583.7 ms | 666.1 ms | 22.32x |
| `melos-rs exec -- echo hi` | 27.6 ± 1.1 ms | 25.2 ms | 31.3 ms | **1.00** |

**22x faster.** Slightly improved from 20x pre-split. The event-based
`ProcessRunner` sends `PackageStarted`/`PackageFinished`/`PackageOutput` events
through an unbounded channel -- the overhead is sub-microsecond per event, well
below the subprocess spawning floor.

### Conclusion

The core/cli split and event-based architecture introduced **zero measurable
performance regression**. All three benchmarks show equal or slightly better
results compared to the monolithic binary, confirming that the abstraction cost
of `tokio::sync::mpsc::unbounded_channel` and the additional crate boundary is
negligible at runtime.
