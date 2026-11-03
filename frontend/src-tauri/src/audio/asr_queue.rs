//! One admission budget spans pipeline, dispatcher and active ASR work.
//! Internal channel hops must keep the lease: dequeueing alone frees no budget.
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::mpsc;
use super::interview_health::WorkTicket;
use super::recording_state::{AudioChunk, AudioError, RecordingState};

pub const MAX_PENDING_JOBS: usize = 32;
pub const MAX_PENDING_PCM_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
struct Usage { jobs: usize, bytes: usize, failed: bool }
struct Budget { usage: Mutex<Usage>, jobs: usize, bytes: usize }

pub struct QueuedAudio {
    pub chunk: AudioChunk,
    pub lease: AudioLease,
}
pub struct AudioLease {
    budget: Arc<Budget>,
    bytes: usize,
    _work: WorkTicket,
}
impl Drop for AudioLease {
    fn drop(&mut self) {
        let mut usage = self.budget.usage.lock().unwrap();
        usage.jobs -= 1;
        usage.bytes -= self.bytes;
        // WorkTicket is dropped after this lock is released.
    }
}

#[derive(Clone)]
pub struct AsrSender {
    tx: mpsc::UnboundedSender<QueuedAudio>,
    budget: Arc<Budget>,
    state: Weak<RecordingState>,
}
pub type AsrReceiver = mpsc::UnboundedReceiver<QueuedAudio>;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    #[error("transcription audio budget exceeded; capture stopped")]
    Full,
    #[error("transcription admission already stopped after a failure")]
    Stopped,
    #[error("transcription consumer is unavailable; capture stopped")]
    Closed,
}

pub fn channel(state: &Arc<RecordingState>) -> (AsrSender, AsrReceiver) {
    channel_with_limits(state, MAX_PENDING_JOBS, MAX_PENDING_PCM_BYTES)
}
fn channel_with_limits(state: &Arc<RecordingState>, jobs: usize, bytes: usize) -> (AsrSender, AsrReceiver) {
    let (tx, rx) = mpsc::unbounded_channel();
    (AsrSender { tx, budget: Arc::new(Budget {usage: Mutex::new(Usage::default()), jobs, bytes}), state: Arc::downgrade(state) }, rx)
}

impl AsrSender {
    pub fn send(&self, chunk: AudioChunk) -> Result<(), AdmissionError> {
        let state = self.state.upgrade().ok_or(AdmissionError::Stopped)?;
        if state.transcription_control.is_cancelled() {return Err(AdmissionError::Stopped);}
        // Count allocated PCM capacity, including spare Vec capacity, rather
        // than just duration. No copies of admitted PCM escape the lease.
        let bytes = chunk.data.capacity().checked_mul(std::mem::size_of::<f32>()).unwrap_or(usize::MAX);
        let mut usage = self.budget.usage.lock().unwrap();
        if usage.failed { return Err(AdmissionError::Stopped); }
        if usage.jobs >= self.budget.jobs || bytes > self.budget.bytes.saturating_sub(usage.bytes) {
            usage.failed = true;
            drop(usage);
            // Never silently omit a prefix and later answer an apparently
            // complete turn. Stop capture and latch the failure until restart.
            state.report_error(AudioError::TranscriptionBacklog);
            return Err(AdmissionError::Full);
        }
        usage.jobs += 1; usage.bytes += bytes;
        let lease = AudioLease { budget: self.budget.clone(), bytes, _work: state.health.begin_audio_work(bytes as u64) };
        // Keep admissions ordered through send and latch a closed consumer
        // before another producer can accept work. Drop errors outside lock.
        match self.tx.send(QueuedAudio {chunk, lease}) {
            Ok(()) => Ok(()),
            Err(error) => {
                usage.failed = true;
                drop(usage);
                drop(error);
                state.report_error(AudioError::ChannelClosed);
                Err(AdmissionError::Closed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{recording_state::DeviceType, vad::SpeechBoundary};
    fn chunk(id: u64, capacity: usize) -> AudioChunk {
        let mut data = Vec::with_capacity(capacity); data.push(0.1);
        AudioChunk { data, sample_rate:16000, timestamp:0.0, chunk_id:id, device_type:DeviceType::System,
            speech_boundary:Some(SpeechBoundary {utterance_id:id,is_final:true,last_voice_ms:None,timing:None}),
            microphone_gate:None,replaces_utterance:true }
    }
    fn started_state() -> Arc<RecordingState> {
        let state=RecordingState::new();state.start_recording().unwrap();state
    }
    fn health(state: &RecordingState) -> super::super::interview_health::AudioHealthSnapshot {
        state.health.snapshot(true,state.is_recording(),false,true,false)
    }
    #[tokio::test]
    async fn budget_covers_both_queue_hops_and_the_in_flight_decode() {
        let state=started_state();let (tx,mut rx)=channel_with_limits(&state,2,64);
        tx.send(chunk(1,8)).unwrap();
        let accepted=rx.recv().await.unwrap();
        let (worker_tx,mut worker_rx)=mpsc::unbounded_channel();
        worker_tx.send(accepted).ok().unwrap();
        let QueuedAudio {chunk: first,lease}=worker_rx.recv().await.unwrap();
        assert_eq!(first.speech_boundary.unwrap().utterance_id,1);assert!(first.replaces_utterance);
        assert_eq!(health(&state).asr_pending_bytes,32);
        tx.send(chunk(2,8)).unwrap();assert_eq!(health(&state).asr_pending_bytes,64);
        assert_eq!(tx.send(chunk(3,1)),Err(AdmissionError::Full));assert!(!state.is_recording());
        assert_eq!(health(&state).asr_peak_bytes,64);
        drop(first);drop(lease);drop(rx);
        assert_eq!(health(&state).asr_pending,0);assert_eq!(health(&state).asr_pending_bytes,0);
        assert_eq!(tx.send(chunk(4,1)),Err(AdmissionError::Stopped),"drain must not silently re-arm admissions");
    }
    #[test]
    fn oversize_allocation_stops_capture_and_reports_once_without_retaining_pcm() {
        use std::sync::atomic::{AtomicUsize,Ordering};
        let state=started_state();let errors=Arc::new(AtomicUsize::new(0));let callback=errors.clone();
        state.set_error_callback(move |error| {assert!(matches!(error,AudioError::TranscriptionBacklog));callback.fetch_add(1,Ordering::Relaxed);});
        let (tx,_rx)=channel_with_limits(&state,8,64);
        assert_eq!(tx.send(chunk(1,17)),Err(AdmissionError::Full));
        for id in 2..1002 {assert_eq!(tx.send(chunk(id,1)),Err(AdmissionError::Stopped));}
        assert_eq!(errors.load(Ordering::Relaxed),1);assert!(!state.is_recording());
        assert_eq!(health(&state).asr_pending,0);assert_eq!(health(&state).asr_peak_bytes,0);
    }
    #[tokio::test]
    async fn closed_receiver_and_aborted_worker_release_the_entire_budget() {
        let state=started_state();let (tx,rx)=channel_with_limits(&state,2,64);drop(rx);
        assert_eq!(tx.send(chunk(1,8)),Err(AdmissionError::Closed));
        assert_eq!(health(&state).asr_pending,0);assert_eq!(health(&state).asr_pending_bytes,0);
        let next=started_state();let (next_tx,mut next_rx)=channel_with_limits(&next,2,64);
        next_tx.send(chunk(1,8)).unwrap();let accepted=next_rx.recv().await.unwrap();
        let task=tokio::spawn(async move {let _accepted=accepted;std::future::pending::<()>().await;});
        task.abort();let _=task.await;assert_eq!(health(&next).asr_pending_bytes,0);
        next_tx.send(chunk(2,8)).unwrap();assert!(next.is_recording(),"an older recording cannot poison a new budget");
    }
    #[test]
    fn production_budget_accepts_three_minute_whole_turn_but_bounds_count_and_bytes() {
        let state=started_state();let (tx,_rx)=channel(&state);
        tx.send(chunk(1,180*16000)).unwrap();
        let mut accepted=1;
        for id in 2..100 {if tx.send(chunk(id,8*16000)).is_err(){break;}accepted+=1;}
        assert!((2..=MAX_PENDING_JOBS).contains(&accepted));
        assert!(health(&state).asr_peak_bytes<=MAX_PENDING_PCM_BYTES as u64);
        assert!(!state.is_recording());
        let next=started_state();let (tx,_rx)=channel(&next);
        let mut long_jobs=0;
        for id in 1..20 {if tx.send(chunk(id,180*16000)).is_err(){break;}long_jobs+=1;}
        assert_eq!(long_jobs,MAX_PENDING_PCM_BYTES/(180*16000*4));
        assert!(long_jobs<MAX_PENDING_JOBS);assert!(health(&next).asr_peak_bytes<=MAX_PENDING_PCM_BYTES as u64);
        assert!(!next.is_recording());
    }
    #[test]
    fn cancellation_rejects_new_work_without_a_false_capture_failure() {
        let state=started_state();let (tx,rx)=channel(&state);
        tx.send(chunk(1,128)).unwrap();state.transcription_control.cancel();
        assert_eq!(tx.send(chunk(2,128)),Err(AdmissionError::Stopped));
        assert_eq!(state.get_error_count(),0);drop(rx);
        assert_eq!(health(&state).asr_pending,0);assert_eq!(health(&state).asr_pending_bytes,0);
    }
}
