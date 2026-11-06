//! Explicit native-only offline intent. No deserializable paths, camera, preview
//! or virtual output authority. A cancelled/failed partial file is diagnostic.
use crate::Reason;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EyeProvider { Ar087, Synthetic }
pub struct EyeJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub provider: EyeProvider,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_cannot_change_intent_or_claim_incomplete_output_as_complete() {
        let job=EyeJob{input:PathBuf::new(),output:PathBuf::new(),width:4,height:2,frames:3,provider:EyeProvider::Synthetic};
        let good=serde_json::json!({"protocol":2,"generation":5,"provider":"synthetic","width":4,"height":2,
            "expected_frames":3,"corrected":3,"neutral":0,"outcome":"completed","error":0,"cleanup_uncertain":false,
            "camera_started":false,"audio_started":false,"external_output_started":false});
        assert!(EyeReport::parse(&serde_json::to_vec(&good).unwrap(),&job,5).is_ok());
        for (field,value) in [("generation",serde_json::json!(4)),("provider",serde_json::json!("ar087")),
            ("width",serde_json::json!(1280)),("corrected",serde_json::json!(2)),("neutral",serde_json::json!(1)),
            ("error",serde_json::json!(4)),("camera_started",serde_json::json!(true)),("audio_started",serde_json::json!(true)),
            ("external_output_started",serde_json::json!(true)),("unknown",serde_json::json!(1))] {
            let mut value_copy=good.clone(); value_copy[field]=value;
            assert_eq!(EyeReport::parse(&serde_json::to_vec(&value_copy).unwrap(),&job,5),Err(Reason::Protocol),"{field}");
        }
    }
}
impl EyeJob {
    pub(crate) fn frame_bytes(&self) -> Result<u64, Reason> {
        if self.width == 0 || self.height == 0 || self.width > 1920 || self.height > 1080
            || self.frames == 0 || self.frames > 100_000 { return Err(Reason::EyeInput); }
        Ok(u64::from(self.width) * u64::from(self.height) * 3)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EyeOutcome { Completed, Cancelled, Fault }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EyeReport {
    pub protocol: u32,
    pub generation: u64,
    pub provider: EyeProvider,
    pub width: u32,
    pub height: u32,
    pub expected_frames: u32,
    pub corrected: u32,
    pub neutral: u32,
    pub outcome: EyeOutcome,
    pub error: u32,
    pub cleanup_uncertain: bool,
    pub camera_started: bool,
    pub audio_started: bool,
    pub external_output_started: bool,
}
impl EyeReport {
    pub(crate) fn parse(bytes: &[u8], job: &EyeJob, generation: u64) -> Result<Self, Reason> {
        if bytes.len() > 4096 { return Err(Reason::Protocol); }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| Reason::Protocol)?;
        let total = value.corrected.checked_add(value.neutral).ok_or(Reason::Protocol)?;
        if value.protocol != 2 || value.generation != generation || value.provider != job.provider
            || value.width != job.width || value.height != job.height || value.expected_frames != job.frames
            || total > job.frames || value.error > 10 || value.neutral > 1
            || value.camera_started || value.audio_started || value.external_output_started
            || (value.cleanup_uncertain && value.error != 8) { return Err(Reason::Protocol); }
        match value.outcome {
            EyeOutcome::Completed if total != job.frames || value.neutral != 0 || value.error != 0
                || value.cleanup_uncertain => return Err(Reason::Protocol),
            EyeOutcome::Fault if value.error == 0 => return Err(Reason::Protocol),
            EyeOutcome::Cancelled if value.cleanup_uncertain => return Err(Reason::Protocol),
            _ => {}
        }
        Ok(value)
    }
}
