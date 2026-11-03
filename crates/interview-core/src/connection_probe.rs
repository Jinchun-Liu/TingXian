use crate::{RequestConfig,StreamEvent};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use tokio_util::sync::CancellationToken;

#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct ProbeOutcome {pub success:bool,pub message:String,pub elapsed_ms:u64,pub attempts:u8,pub total_tokens:Option<u64>}

// Fixed synthetic input, independent of profile, transcripts and interview history.
pub fn connection_test_messages()->Value {
    json!([{"role":"system","content":"这是API连接自检。请只回复：建议回答：连接成功。"},{"role":"user","content":"连接测试"}])
}
pub async fn probe_connection(key:&str,config:&RequestConfig,cancel:CancellationToken)->ProbeOutcome {
    let start=std::time::Instant::now();let mut attempts=0;let mut total_tokens=None;
    let result=crate::stream_answer_with_config(key,connection_test_messages(),cancel,config,|event|match event {
        StreamEvent::RequestSent{attempt}=>attempts=attempts.max(attempt),
        StreamEvent::Usage{usage,..}=>{if let Some(tokens)=usage["total_tokens"].as_u64(){total_tokens=Some(tokens);}},
        StreamEvent::Done{usage,..}=>{if let Some(tokens)=usage.and_then(|u|u["total_tokens"].as_u64()){total_tokens=Some(tokens);}},
        _=>{}, // Do not retain test response text or stream it into interview history.
    }).await;
    ProbeOutcome{success:result.is_ok(),message:result.err().unwrap_or_else(||"连接成功，已收到完整文本响应".into()),elapsed_ms:start.elapsed().as_millis() as u64,attempts,total_tokens}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn fixed_probe_has_no_profile_inputs_and_missing_key_does_not_send() {
        assert_eq!(connection_test_messages().as_array().unwrap().len(),2);
        assert_eq!(connection_test_messages()[1]["content"],"连接测试");
        let result=probe_connection("",&RequestConfig::default(),CancellationToken::new()).await;
        assert!(!result.success);assert_eq!(result.attempts,0);assert!(result.total_tokens.is_none());assert!(result.message.contains("API Key"));
    }
}
