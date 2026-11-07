//! Observed ASR task usage, independent from transcript and microphone gates.
//! Provider duration is cumulative per task; it is not a completed account bill.
use serde::{Deserialize,Serialize};
use std::{collections::HashMap,sync::{Arc,Mutex}};
use tauri::{State,WebviewWindow,Runtime};

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TaskUsage {pub task_id:String,pub source:String,pub state:String,pub reported_seconds:Option<f64>,pub inconsistent:bool}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {pub version:u32,pub tasks:Vec<TaskUsage>,#[serde(default,skip_serializing_if="Option::is_none")]pub history_missing:Option<bool>}
pub fn valid_id(id:&str)->bool {uuid::Uuid::parse_str(id).is_ok() && id.len()==36}
impl Snapshot {
    pub fn validate(&self)->Result<(),String> {
        let mut ids=std::collections::HashSet::new();
        if self.version!=1 || self.tasks.len()>20000 || self.tasks.iter().any(|r|!valid_id(&r.task_id)||!ids.insert(r.task_id.to_ascii_lowercase())
            || !["System","Microphone"].contains(&r.source.as_str()) || !["connecting","running","finished","failed","cancelled","interrupted"].contains(&r.state.as_str())
            || r.reported_seconds.is_some_and(|n|!n.is_finite()||n<0.0||n>9_007_199_254_740_991.0)) {return Err("云转写用量记录格式无效，原记录保留".into());}
        Ok(())
    }
}
#[derive(Default)]
pub struct UsageStore(Mutex<HashMap<String,Vec<TaskUsage>>>);
impl UsageStore {
    pub fn begin(self:&Arc<Self>,scope:&str,source:&str)->Result<TaskMeter,String> {
        if !valid_id(scope)||!["System","Microphone"].contains(&source){return Err("云转写用量会话无效".into());}
        let mut scopes=self.0.lock().unwrap_or_else(|e|e.into_inner());
        if scopes.values().map(Vec::len).sum::<usize>()>=20000 {return Err("云转写用量缓存已满，请保存会话并重开软件".into());}
        let tasks=scopes.entry(scope.into()).or_default();
        if tasks.len()>=2048 {return Err("本轮云转写连接已达2048次，请保存并重新开始会话".into());}
        let id=uuid::Uuid::new_v4().to_string();
        tasks.push(TaskUsage{task_id:id.clone(),source:source.into(),state:"connecting".into(),reported_seconds:None,inconsistent:false});
        Ok(TaskMeter{store:self.clone(),scope:scope.into(),id})
    }
    pub fn snapshot(&self,scope:&str)->Result<Snapshot,String> {
        if !valid_id(scope){return Err("云转写用量会话无效".into());}
        Ok(Snapshot{version:1,tasks:self.0.lock().unwrap_or_else(|e|e.into_inner()).get(scope).cloned().unwrap_or_default(),history_missing:None})
    }
}
pub struct TaskMeter {store:Arc<UsageStore>,scope:String,pub id:String}
impl TaskMeter {
    fn update(&self,f:impl FnOnce(&mut TaskUsage)) {
        let mut scopes=self.store.0.lock().unwrap_or_else(|e|e.into_inner());
        if let Some(r)=scopes.get_mut(&self.scope).and_then(|rows|rows.iter_mut().find(|r|r.task_id==self.id)){f(r);}
    }
    pub fn running(&self){self.update(|r|if r.state=="connecting"{r.state="running".into();});}
    pub fn observe(&self,seconds:Option<f64>){
        self.update(|r|{if let Some(n)=seconds {
            if !n.is_finite()||n<0.0||n>9_007_199_254_740_991.0||r.reported_seconds.is_some_and(|old|n<old){r.inconsistent=true;}
            else {r.reported_seconds=Some(n);}
        }});
    }
    pub fn finish(&self,state:&str){self.update(|r|if ["connecting","running"].contains(&r.state.as_str()){r.state=state.into();});}
}
impl Drop for TaskMeter {fn drop(&mut self){self.finish("cancelled");}}

#[tauri::command]
pub fn interview_cloud_asr_usage<R:Runtime>(window:WebviewWindow<R>,state:State<'_,crate::interview::InterviewState>,scope:String)->Result<Snapshot,String> {
    if window.label()!="main"{return Err("请在主窗口查看云转写用量".into());}
    state.cloud_asr.usage.snapshot(&scope)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cumulative_usage_is_task_scoped_and_survives_missing_or_cancelled_results(){
        let store=Arc::new(UsageStore::default());let scope=uuid::Uuid::new_v4().to_string();
        let a=store.begin(&scope,"System").unwrap();a.running();a.observe(Some(3.0));a.observe(Some(3.0));a.observe(Some(8.0));a.observe(None);a.finish("finished");
        let b=store.begin(&scope,"Microphone").unwrap();b.running();b.observe(Some(0.0));drop(b);
        let c=store.begin(&scope,"System").unwrap();drop(c);
        let rows=store.snapshot(&scope).unwrap();assert_eq!(rows.tasks.len(),3);assert_eq!(rows.tasks[0].reported_seconds,Some(8.0));assert_eq!(rows.tasks[0].state,"finished");assert_eq!(rows.tasks[1].reported_seconds,Some(0.0));assert_eq!(rows.tasks[1].state,"cancelled");assert_eq!(rows.tasks[2].reported_seconds,None);rows.validate().unwrap();
        let other=uuid::Uuid::new_v4().to_string();assert!(store.snapshot(&other).unwrap().tasks.is_empty());
    }
    #[test]
    fn invalid_or_decreasing_provider_values_remain_explicitly_inconsistent(){
        let store=Arc::new(UsageStore::default());let scope=uuid::Uuid::new_v4().to_string();let a=store.begin(&scope,"System").unwrap();
        a.observe(Some(5.0));a.observe(Some(4.0));a.observe(Some(f64::NAN));a.finish("failed");
        let snap=store.snapshot(&scope).unwrap();assert!(snap.tasks[0].inconsistent);assert_eq!(snap.tasks[0].reported_seconds,Some(5.0));snap.validate().unwrap();
        let mut bad=snap.clone();bad.tasks.push(bad.tasks[0].clone());assert!(bad.validate().is_err());assert!(store.snapshot("old-path").is_err());
    }
    #[test]
    fn display_window_cannot_read_the_cloud_usage_ledger(){
        use tauri::Manager;
        let app=tauri::test::mock_builder().manage(crate::interview::InterviewState::default()).invoke_handler(tauri::generate_handler![interview_cloud_asr_usage]).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let scope=uuid::Uuid::new_v4().to_string();let meter=app.state::<crate::interview::InterviewState>().cloud_asr.usage.begin(&scope,"System").unwrap();
        let window=tauri::WebviewWindowBuilder::new(&app,"display",Default::default()).build().unwrap();
        assert!(tauri::test::get_ipc_response(&window,tauri::webview::InvokeRequest{cmd:"interview_cloud_asr_usage".into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(serde_json::json!({"scope":scope})),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()}).is_err());drop(meter);
    }
}
