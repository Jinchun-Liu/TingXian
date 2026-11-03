//! Cancellation belongs to one recording, never to the shared model.
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

pub struct TranscriptionControl {
    flag: Arc<AtomicBool>,
    token: CancellationToken,
}
impl TranscriptionControl {
    pub fn new() -> Arc<Self> { Arc::new(Self {flag:Arc::new(AtomicBool::new(false)),token:CancellationToken::new()}) }
    pub fn cancel(&self) { self.flag.store(true,Ordering::SeqCst);self.token.cancel(); }
    pub fn is_cancelled(&self) -> bool { self.flag.load(Ordering::SeqCst) }
    pub fn flag(&self) -> Arc<AtomicBool> { self.flag.clone() }
    pub async fn cancelled(&self) { self.token.cancelled().await; }
}

// Cancelling/dropping the parent also asks synchronous Whisper to leave its
// native computation. Its child JoinSet aborts at the next cancellation point.
pub struct CancelOnDrop(Option<Arc<TranscriptionControl>>);
impl CancelOnDrop {
    pub fn new(control:Arc<TranscriptionControl>)->Self {Self(Some(control))}
    pub fn disarm(&mut self) {self.0=None;}
}
impl Drop for CancelOnDrop {fn drop(&mut self){if let Some(control)=&self.0{control.cancel();}}}

/// Borrows the handle: a timeout must not detach it and then unload its model.
pub async fn drain_or_cancel(
    task: &mut JoinHandle<()>, control: &TranscriptionControl,
    grace: Duration, cancellation_grace: Duration,
) -> Option<Result<(), JoinError>> {
    if !control.is_cancelled() {
        if let Ok(result)=tokio::time::timeout(grace,&mut *task).await {return Some(result);}
        control.cancel();
    }
    tokio::time::timeout(cancellation_grace,&mut *task).await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn graceful_work_finishes_without_cancellation() {
        let control=TranscriptionControl::new();let mut task=tokio::spawn(async {});
        assert!(drain_or_cancel(&mut task,&control,Duration::from_secs(1),Duration::from_secs(1)).await.unwrap().is_ok());
        assert!(!control.is_cancelled());
    }
    #[tokio::test]
    async fn overdue_work_is_cancelled_and_joined_without_affecting_next_recording() {
        let control=TranscriptionControl::new();let worker=control.clone();let next=TranscriptionControl::new();
        let mut task=tokio::spawn(async move {worker.cancelled().await;});
        assert!(drain_or_cancel(&mut task,&control,Duration::from_millis(5),Duration::from_secs(1)).await.unwrap().is_ok());
        assert!(control.is_cancelled());assert!(!next.is_cancelled());
    }
    #[tokio::test]
    async fn nonresponsive_task_remains_owned_and_can_be_retried() {
        let control=TranscriptionControl::new();let (tx,rx)=tokio::sync::oneshot::channel();
        let mut task=tokio::spawn(async move {let _=rx.await;});
        assert!(drain_or_cancel(&mut task,&control,Duration::from_millis(5),Duration::from_millis(5)).await.is_none());
        assert!(!task.is_finished());assert!(control.is_cancelled());
        tx.send(()).unwrap();
        assert!(drain_or_cancel(&mut task,&control,Duration::from_secs(1),Duration::from_secs(1)).await.unwrap().is_ok());
    }
    #[tokio::test]
    async fn aborting_parent_requests_native_cancellation() {
        let control=TranscriptionControl::new();let owned=control.clone();
        let (tx,rx)=tokio::sync::oneshot::channel();
        let task=tokio::spawn(async move {let _guard=CancelOnDrop::new(owned);tx.send(()).unwrap();std::future::pending::<()>().await;});
        rx.await.unwrap();task.abort();let _=task.await;
        assert!(control.is_cancelled());assert!(control.flag().load(Ordering::SeqCst));
    }
}
