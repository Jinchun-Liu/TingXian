//! One token per opened microphone stream. Revocation is permanent, including
//! for audio already queued for VAD or an in-flight ASR result.
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct MicrophoneGate {
    pub epoch: u64,
    enabled: AtomicBool,
}

impl MicrophoneGate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { epoch: NEXT_EPOCH.fetch_add(1, Ordering::SeqCst), enabled: AtomicBool::new(true) })
    }
    pub fn enabled(&self) -> bool { self.enabled.load(Ordering::SeqCst) }
    pub fn revoke(&self) { self.enabled.store(false, Ordering::SeqCst); }
}

pub fn allowed(gate: &Option<Arc<MicrophoneGate>>) -> bool {
    gate.as_ref().map_or(true, |gate| gate.enabled())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reopening_never_revives_queued_or_inflight_audio() {
        let first = MicrophoneGate::new();
        let queued = Some(first.clone());
        assert!(allowed(&queued));
        first.revoke();
        let reopened = MicrophoneGate::new();
        assert!(!allowed(&queued));
        assert!(reopened.enabled());
        assert_ne!(first.epoch, reopened.epoch);
        assert!(allowed(&None)); // System audio is independent.
    }
}
