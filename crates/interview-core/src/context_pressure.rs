//! Advisory pressure from the real pre-pruning wire workset, never from the
//! accumulated journal length. Hard admission remains request_budget's job.
use crate::request_budget::RequestMeasurement;
use serde::Serialize;
use std::collections::VecDeque;
#[derive(Clone,Debug,Default,Serialize)]
pub struct Pressure {
    pub active:bool,pub hard_reached:bool,pub estimated_bytes:u64,pub hard_input_budget:u64,
    pub projected_bytes:u64,pub growth_bytes_per_second:u64,pub compaction_p95_ms:u64,
    pub image_cost_unknown:bool,pub known_foreground_requests:usize,
}
#[derive(Default)]
pub struct Tracker {previous:Option<(u64,u64,u64)>,durations:VecDeque<u64>,view:Pressure}
impl Tracker {
    pub fn view(&self)->Pressure{self.view.clone()}
    pub fn completed(&mut self,elapsed_ms:u64){self.durations.push_back(elapsed_ms.min(120_000));while self.durations.len()>32{self.durations.pop_front();}}
    pub fn observe(&mut self,m:&RequestMeasurement,now_ms:u64,foreground:usize)->Pressure{
        let bytes=m.serialized_text_bytes.saturating_add(m.image_reserve.unwrap_or(0));
        let growth=self.previous.filter(|(t,_,b)|*t<now_ms&&*b==m.hard_input_budget).map_or(0,|(t,p,_)|bytes.saturating_sub(p).saturating_mul(1000)/(now_ms-t).max(1));
        let mut times:Vec<_>=self.durations.iter().copied().collect();times.sort_unstable();
        let p95=if times.is_empty(){60_000}else{times[(times.len()*95).div_ceil(100).saturating_sub(1)]};
        // Counts already admitted foreground work, not speculative whole-history
        // growth. Unknown image cost remains an explicit uncertainty.
        let horizon=p95.saturating_mul(1+foreground.min(3) as u64);
        let projected=bytes.saturating_add(growth.saturating_mul(horizon)/1000);
        let soft=m.hard_input_budget.saturating_mul(75)/100;
        let low=m.hard_input_budget.saturating_mul(60)/100;
        let active=projected>=soft||(self.view.active&&bytes>low);
        self.view=Pressure{active,hard_reached:bytes>m.hard_input_budget,estimated_bytes:bytes,hard_input_budget:m.hard_input_budget,projected_bytes:projected,growth_bytes_per_second:growth,compaction_p95_ms:p95,image_cost_unknown:!m.complete_local_estimate,known_foreground_requests:foreground.min(3)};
        self.previous=Some((now_ms,bytes,m.hard_input_budget));self.view()
    }
}
#[cfg(test)]mod tests{
    use super::*;use crate::{context_budget::ContextBudget,request_budget::{measure,WindowBasis}};use serde_json::json;
    fn m(n:usize)->RequestMeasurement{measure(&json!({"max_tokens":512,"messages":[{"role":"user","content":"x".repeat(n)}]}),&ContextBudget{context_tokens:4096,max_output_tokens:512,safety_tokens:256},WindowBasis::ConfiguredUnknown,None).unwrap()}
    #[test]fn small_window_prediction_hysteresis_model_switch_and_growth_are_explicit(){
        let mut t=Tracker::default();assert!(!t.observe(&m(1000),1000,0).active);t.completed(2000);
        let p=t.observe(&m(1900),3000,2);assert!(p.active);assert!(p.estimated_bytes<p.hard_input_budget*75/100);assert_eq!(p.growth_bytes_per_second,450);assert_eq!(p.compaction_p95_ms,2000);
        assert!(t.observe(&m(2200),6000,0).active);assert!(!t.observe(&m(1500),9000,0).active);assert!(t.observe(&m(4000),10000,0).hard_reached);
        let mut changed=m(1000);changed.hard_input_budget=2000;assert_eq!(t.observe(&changed,11000,0).growth_bytes_per_second,0);
    }
}
