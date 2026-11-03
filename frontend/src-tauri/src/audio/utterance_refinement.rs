//! Keep a bounded in-memory copy of one speech turn. Prefixes remain useful for
//! live captions; its final chunk can replace them after a full-context decode.
use super::vad::SpeechSegment;

pub struct UtteranceRefinement {
    limit: usize,
    identity: Option<u64>,
    samples: Vec<f32>,
    start_ms: f64,
    exhausted: bool,
    had_prefix: bool,
}
impl UtteranceRefinement {
    pub fn new(limit: usize) -> Self {
        Self { limit, identity: None, samples: vec![], start_ms: 0.0, exhausted: false, had_prefix: false }
    }
    pub fn reset(&mut self) { self.identity=None; self.samples.clear(); self.exhausted=false; self.had_prefix=false; }
    pub fn receive(&mut self, mut segment: SpeechSegment) -> (SpeechSegment, bool) {
        if self.identity != Some(segment.boundary.utterance_id) {
            self.reset(); self.identity=Some(segment.boundary.utterance_id); self.start_ms=segment.start_timestamp_ms;
        }
        if !self.exhausted {
            if self.samples.len()+segment.samples.len() > self.limit { self.exhausted=true; self.samples.clear(); }
            else { self.samples.extend_from_slice(&segment.samples); }
        }
        let replace=segment.boundary.is_final && self.had_prefix && !self.exhausted;
        if replace { segment.samples=std::mem::take(&mut self.samples); segment.start_timestamp_ms=self.start_ms; }
        if segment.boundary.is_final { self.reset(); } else { self.had_prefix=true; }
        (segment, replace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::vad::SpeechBoundary;
    fn part(id:u64,data:&[f32],start:f64,final_:bool)->SpeechSegment {
        SpeechSegment {samples:data.to_vec(),start_timestamp_ms:start,end_timestamp_ms:start+data.len() as f64,
            confidence:1.0,boundary:SpeechBoundary {utterance_id:id,is_final:final_,last_voice_ms:None,timing:None}}
    }
    #[test]
    fn final_replaces_prefixes_once_and_resets_between_turns() {
        let mut r=UtteranceRefinement::new(8);
        assert!(!r.receive(part(1,&[1.0,2.0],10.0,false)).1);
        assert!(!r.receive(part(1,&[3.0,4.0],12.0,false)).1);
        let (full,replace)=r.receive(part(1,&[],14.0,true));
        assert!(replace);assert_eq!(full.samples,vec![1.0,2.0,3.0,4.0]);assert_eq!(full.start_timestamp_ms,10.0);
        let (next,replace)=r.receive(part(2,&[5.0],30.0,true));assert!(!replace);assert_eq!(next.samples,vec![5.0]);
    }
    #[test]
    fn mute_and_limit_never_reuse_old_audio_or_grow_unbounded() {
        let mut r=UtteranceRefinement::new(3);
        r.receive(part(1,&[1.0,2.0],0.0,false));r.reset();
        let (fresh,replace)=r.receive(part(1,&[3.0],9.0,true));assert!(!replace);assert_eq!(fresh.samples,vec![3.0]);
        r.receive(part(2,&[1.0,2.0],0.0,false));r.receive(part(2,&[3.0,4.0],2.0,false));
        assert!(r.samples.is_empty());
        let (tail,replace)=r.receive(part(2,&[5.0],4.0,true));assert!(!replace);assert_eq!(tail.samples,vec![5.0]);
    }
}
