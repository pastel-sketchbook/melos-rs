use melos_core::events::Event as CoreEvent;
use tokio::sync::mpsc::UnboundedReceiver;

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
