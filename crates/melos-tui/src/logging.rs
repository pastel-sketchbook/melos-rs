use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Initialize file-based logging with daily log rotation.
///
/// Logs are written to `melos-tui-YYYY-MM-DD.log` in the current working directory.
/// The log level defaults to `debug` and can be overridden via the `MELOS_LOG` or
/// `RUST_LOG` environment variables.
///
/// Returns a [`WorkerGuard`] that **must** be held for the lifetime of the program
/// to ensure buffered log records are flushed on shutdown.
pub fn init() -> WorkerGuard {
    let file_appender = tracing_appender::rolling::daily(".", "melos-tui");

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_env("MELOS_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("debug"));

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(true)
                .with_thread_ids(true),
        )
        .with(env_filter)
        .init();

    guard
}
