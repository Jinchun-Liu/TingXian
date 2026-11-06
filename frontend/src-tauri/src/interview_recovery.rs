use serde::{Deserialize,Serialize};
use tauri::{Manager,State,WebviewWindow};
use crate::{interview::InterviewState,interview_sessions::{Snapshot,validate_snapshot,scrub}};

#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composer {pub question:String,pub confirmed_answer:String}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySnapshot {pub version:u32,pub snapshot:Snapshot,pub composer:Composer}
impl RecoverySnapshot {
    fn validate(&self)->Result<(),String>{
        if self.version!=1{return Err("恢复草稿版本无效".into());}
        validate_snapshot(&self.snapshot,true)?;
        let mut data=serde_json::to_value(self).map_err(|e|e.to_string())?;
        scrub(&mut data,0)
    }
}
fn owner<R:tauri::Runtime>(window:&WebviewWindow<R>)->Result<(),String>{
    if window.label()=="main"{Ok(())}else{Err("仅主窗口可以管理恢复草稿".into())}
}
#[tauri::command]
pub async fn interview_recovery_list<R:tauri::Runtime>(window:WebviewWindow<R>)->Result<interview_core::recovery::DraftList,String>{
    owner(&window)?;
    interview_core::recovery::list(window.app_handle().state::<crate::state::AppState>().db_manager.pool()).await
}
#[tauri::command]
pub async fn interview_recovery_write<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,id:String,epoch:i64,revision:i64,draft:RecoverySnapshot)->Result<(),String>{
    owner(&window)?;draft.validate()?;
    // Exit retains the capture lock while flushing this independent checkpoint.
    // SQLite epochs fence history cleanup, including writes from old renderers.
    interview_core::recovery::write(window.app_handle().state::<crate::state::AppState>().db_manager.pool(),&id,epoch,revision,serde_json::to_value(draft).map_err(|e|e.to_string())?,&state.api_key().await).await
}
#[tauri::command]
pub async fn interview_recovery_load<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,id:String)->Result<RecoverySnapshot,String>{
    owner(&window)?;
    let _guard=state.lifecycle.lock().await;
    if crate::audio::recording_commands::is_recording().await || state.session.load(std::sync::atomic::Ordering::SeqCst) {return Err("请先结束当前会话再恢复草稿".into());}
    let value=interview_core::recovery::load(window.app_handle().state::<crate::state::AppState>().db_manager.pool(),&id).await?;
    let draft:RecoverySnapshot=serde_json::from_value(value).map_err(|_|"恢复草稿结构无效，当前窗口保留")?;
    draft.validate()?;Ok(draft)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json,Value};
    #[test]
    fn recovery_ipc_owns_drafts_and_restores_without_ai_or_capture() {
        let root=std::env::temp_dir().join(format!("tingxian-recovery-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let db=tauri::async_runtime::block_on(crate::database::manager::DatabaseManager::new(root.join("test.sqlite").to_str().unwrap(),root.join("no-legacy.sqlite").to_str().unwrap())).unwrap();
        let app=tauri::test::mock_builder().manage(InterviewState::default()).manage(crate::state::AppState{db_manager:db})
          .invoke_handler(tauri::generate_handler![interview_recovery_list,interview_recovery_write,interview_recovery_load])
          .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let display=tauri::WebviewWindowBuilder::new(&app,"display",Default::default()).build().unwrap();
        let invoke=|window:&tauri::WebviewWindow<tauri::test::MockRuntime>,cmd:&str,body:Value|tauri::test::get_ipc_response(window,tauri::webview::InvokeRequest{cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let draft=json!({"version":1,"snapshot":{"version":1,"history":[],"transcripts":[],"spoken_dialogue":[],"current":{"question":"","answer":""},"calls":0,"usage":""},"composer":{"question":"尚未发送的问题","confirmed_answer":"尚未确认的回答"}});
        let listing=invoke(&main,"interview_recovery_list",json!({})).unwrap().deserialize::<Value>().unwrap();
        let args=json!({"id":"editor-only","epoch":listing["epoch"],"revision":1,"draft":draft});
        for (cmd,body) in [("interview_recovery_list",json!({})),("interview_recovery_write",args.clone()),("interview_recovery_load",json!({"id":"editor-only"}))]{
            assert!(invoke(&display,cmd,body).is_err(),"display window accessed recovery state");
        }
        // Normal exit holds this lock: the final write must remain independent.
        let state=app.state::<InterviewState>();let lease=tauri::async_runtime::block_on(state.lifecycle.lock());
        invoke(&main,"interview_recovery_write",args.clone()).unwrap();drop(lease);
        assert_eq!(invoke(&main,"interview_recovery_load",json!({"id":"editor-only"})).unwrap().deserialize::<Value>().unwrap(),draft);
        assert!(invoke(&main,"interview_recovery_write",args.clone()).is_err(),"late revision was accepted");
        let mut bad=args;bad["revision"]=2.into();bad["draft"]["version"]=99.into();assert!(invoke(&main,"interview_recovery_write",bad.clone()).is_err());
        bad["draft"]["version"]=1.into();bad["draft"]["api_key"]="synthetic".into();assert!(invoke(&main,"interview_recovery_write",bad).is_err());
        state.session.store(true,std::sync::atomic::Ordering::SeqCst);assert!(invoke(&main,"interview_recovery_load",json!({"id":"editor-only"})).is_err());
        state.session.store(false,std::sync::atomic::Ordering::SeqCst);
        assert!(!tauri::async_runtime::block_on(crate::audio::recording_commands::is_recording()));
        tauri::async_runtime::block_on(app.state::<crate::state::AppState>().db_manager.pool().close());
    }
}
