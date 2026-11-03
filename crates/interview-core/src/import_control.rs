use crate::Result;
use serde::Serialize;
use std::sync::{Arc, atomic::{AtomicU8, Ordering}};
use tokio_util::sync::CancellationToken;

pub const IMPORT_CANCELLED: &str = "导入已取消，原资料库保持不变";
const RUNNING: u8 = 0;
const CANCELLED: u8 = 1;
const COMMITTING: u8 = 2;
const FINISHED: u8 = 3;

#[derive(Default)]
struct Inner { phase: AtomicU8, token: CancellationToken }
#[derive(Clone, Default)]
pub struct ImportControl(Arc<Inner>);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportCancelStatus { Requested, Committing, Finished }
#[derive(Debug, Clone, Serialize)]
pub struct ImportProgress {
    pub phase: String,
    pub completed: usize,
    pub total: usize,
    pub path: Option<String>,
}

impl ImportControl {
    pub fn cancel(&self) -> ImportCancelStatus {
        match self.0.phase.compare_exchange(RUNNING, CANCELLED, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) | Err(CANCELLED) => { self.0.token.cancel(); ImportCancelStatus::Requested }
            Err(COMMITTING) => ImportCancelStatus::Committing,
            _ => ImportCancelStatus::Finished,
        }
    }
    pub fn check(&self) -> Result<()> {
        if self.0.phase.load(Ordering::SeqCst) == CANCELLED || self.0.token.is_cancelled() { Err(IMPORT_CANCELLED.into()) } else { Ok(()) }
    }
    pub(crate) async fn run<T>(&self, work: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        self.check()?;
        tokio::select! { biased;
            _ = self.0.token.cancelled() => Err(IMPORT_CANCELLED.into()),
            result = work => result,
        }
    }
    /// Linearization point: accepted cancellation always precedes commit;
    /// cancellation after this point must never claim that publication stopped.
    pub(crate) fn begin_commit(&self) -> Result<()> {
        self.0.phase.compare_exchange(RUNNING, COMMITTING, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ()).map_err(|_| IMPORT_CANCELLED.into())
    }
    pub(crate) fn finish(&self) { self.0.phase.store(FINISHED, Ordering::SeqCst); }
}
