//! In-memory connection evidence. A configured key is not a successful check.
use serde::{Deserialize,Serialize};
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum CheckStatus {Untested,Checking,Cancelling,Passed,Failed,Cancelled}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct ConnectionReport {
    pub check_id:Option<String>,pub status:CheckStatus,pub model:Option<String>,pub started_at:Option<String>,pub finished_at:Option<String>,
    pub elapsed_ms:Option<u64>,pub attempts:u8,pub total_tokens:Option<u64>,pub message:String,
}
impl Default for ConnectionReport {
    fn default()->Self {Self{check_id:None,status:CheckStatus::Untested,model:None,started_at:None,finished_at:None,elapsed_ms:None,attempts:0,total_tokens:None,message:"尚未测试连接".into()}}
}
#[derive(Default)]
struct Check {report:ConnectionReport,cancel:Option<CancellationToken>,cancelled:std::collections::VecDeque<String>}
#[derive(Default)]
pub struct ConnectionState {inner:Mutex<Check>}
impl ConnectionState {
    pub fn snapshot(&self)->ConnectionReport {self.inner.lock().unwrap_or_else(|e|e.into_inner()).report.clone()}
    pub fn begin(&self,id:&str,model:&str)->Result<CancellationToken,String> {
        uuid::Uuid::parse_str(id).map_err(|_|"连接测试ID无效")?;
        let mut check=self.inner.lock().unwrap_or_else(|e|e.into_inner());
        if matches!(check.report.status,CheckStatus::Checking|CheckStatus::Cancelling) {return Err("连接测试仍在进行，请取消或等待完成".into());}
        if check.cancelled.iter().any(|old|old==id) {
            check.report=ConnectionReport{check_id:Some(id.into()),status:CheckStatus::Cancelled,model:Some(model.into()),finished_at:Some(chrono::Utc::now().to_rfc3339()),elapsed_ms:Some(0),message:"连接测试已在发送前取消".into(),..Default::default()};
            return Err("连接测试已在发送前取消".into());
        }
        if check.report.check_id.as_deref()==Some(id) {return Err("此测试请求已处理，请重新点击测试".into());}
        let cancel=CancellationToken::new();
        check.report=ConnectionReport{check_id:Some(id.into()),status:CheckStatus::Checking,model:Some(model.into()),started_at:Some(chrono::Utc::now().to_rfc3339()),message:"正在测试连接…".into(),..Default::default()};
        check.cancel=Some(cancel.clone());Ok(cancel)
    }
    pub fn cancel(&self,id:Option<&str>)->ConnectionReport {
        let mut check=self.inner.lock().unwrap_or_else(|e|e.into_inner());
        if let Some(id)=id.filter(|id|uuid::Uuid::parse_str(id).is_ok()) {
            if !check.cancelled.iter().any(|old|old==id){check.cancelled.push_back(id.into());}
            while check.cancelled.len()>32 {check.cancelled.pop_front();}
        }
        if id.is_none() || id==check.report.check_id.as_deref() {
            if let Some(cancel)=&check.cancel {cancel.cancel();check.report.status=CheckStatus::Cancelling;check.report.message="正在取消连接测试…".into();}
        }
        check.report.clone()
    }
    pub fn invalidate(&self) {
        let mut check=self.inner.lock().unwrap_or_else(|e|e.into_inner());
        if let Some(cancel)=check.cancel.take(){cancel.cancel();}
        check.report=ConnectionReport{message:"配置已变化，请重新测试连接".into(),..Default::default()};
    }
    pub fn finish(&self,id:&str,outcome:interview_core::ProbeOutcome)->Result<ConnectionReport,String> {
        let mut check=self.inner.lock().unwrap_or_else(|e|e.into_inner());
        if check.report.check_id.as_deref()!=Some(id) {return Err("这次连接测试已失效，未覆盖当前状态".into());}
        let cancelled=check.cancel.take().is_some_and(|c|c.is_cancelled());
        check.report.status=if cancelled {CheckStatus::Cancelled}else if outcome.success {CheckStatus::Passed}else{CheckStatus::Failed};
        check.report.message=if cancelled {"连接测试已取消；取消前的请求仍可能产生用量".into()}else{outcome.message};
        check.report.finished_at=Some(chrono::Utc::now().to_rfc3339());check.report.elapsed_ms=Some(outcome.elapsed_ms);
        check.report.attempts=outcome.attempts;check.report.total_tokens=outcome.total_tokens;
        Ok(check.report.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn outcome(success:bool)->interview_core::ProbeOutcome {interview_core::ProbeOutcome{success,message:if success{"ok"}else{"API Key 无效（401）"}.into(),elapsed_ms:100,attempts:1,total_tokens:Some(9)}}
    #[test]
    fn evidence_is_scoped_and_stale_cancel_or_finish_cannot_change_a_new_check() {
        let state=ConnectionState::default();assert_eq!(state.snapshot().status,CheckStatus::Untested);
        let a=uuid::Uuid::new_v4().to_string();let b=uuid::Uuid::new_v4().to_string();
        let token=state.begin(&a,"synthetic-model").unwrap();assert!(state.begin(&b,"other-model").is_err());
        state.cancel(Some(&uuid::Uuid::new_v4().to_string()));assert!(!token.is_cancelled());state.cancel(Some(&a));assert!(token.is_cancelled());
        let report=state.finish(&a,outcome(true)).unwrap();assert_eq!(report.status,CheckStatus::Cancelled);assert!(report.finished_at.is_some());
        state.begin(&b,"other-model").unwrap();assert!(state.finish(&a,outcome(true)).is_err());
        let report=state.finish(&b,outcome(false)).unwrap();assert_eq!(report.status,CheckStatus::Failed);assert!(report.message.contains("401"));assert_eq!(report.attempts,1);
        state.invalidate();assert_eq!(state.snapshot().status,CheckStatus::Untested);assert!(state.snapshot().check_id.is_none());
        let c=uuid::Uuid::new_v4().to_string();state.begin(&c,"synthetic-model").unwrap();let report=state.finish(&c,outcome(true)).unwrap();assert_eq!(report.status,CheckStatus::Passed);assert_eq!(report.total_tokens,Some(9));assert!(state.begin(&c,"synthetic-model").is_err());
    }
    #[test]
    fn early_cancel_survives_begin_and_configuration_invalidation() {
        let state=ConnectionState::default();let id=uuid::Uuid::new_v4().to_string();
        state.cancel(Some(&id));state.invalidate();assert!(state.begin(&id,"synthetic-model").is_err());
        let report=state.snapshot();assert_eq!(report.status,CheckStatus::Cancelled);assert_eq!(report.attempts,0);
    }
}
