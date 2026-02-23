use anyhow::Result;
use melos_core::events::Event as CoreEvent;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use crate::dispatch::DispatchResult;

/// Receive the next core event from an optional channel.
///
/// When a command is running (`rx` is `Some`), this awaits the next event.
/// Returns `Some(event)` for each event and `None` when the channel closes
/// (sender dropped, meaning the command task finished).
///
/// When no command is running (`rx` is `None`), this future pends forever
/// so `tokio::select!` skips this branch.
pub async fn recv_core_event(rx: &mut Option<UnboundedReceiver<CoreEvent>>) -> Option<CoreEvent> {
    match rx.as_mut() {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Poll an optional task handle for completion.
///
/// When a command task is running (`handle` is `Some`), this awaits the
/// `JoinHandle` and returns the join result. When no task is running
/// (`handle` is `None`), this future pends forever so `tokio::select!`
/// skips this branch.
///
/// This is intentionally a separate select branch from `recv_core_event`
/// to avoid blocking the event loop. The channel may close before the
/// JoinHandle resolves (e.g. if cloned senders in `ProcessRunner` outlive
/// the original sender), and awaiting the handle inline would freeze all
/// key event processing.
pub async fn poll_task_handle(
    handle: &mut Option<JoinHandle<Result<DispatchResult>>>,
) -> Result<Result<DispatchResult>, tokio::task::JoinError> {
    match handle.as_mut() {
        Some(h) => h.await,
        None => std::future::pending().await,
    }
}
