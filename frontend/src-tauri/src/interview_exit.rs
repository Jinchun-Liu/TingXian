//! Normal quit is a handshake: stop work -> archive native records -> clear
//! the archived browser cache -> approve exit. Force-quit remains recoverable
//! through the next launch's residue notice, never reported as clean exit.
use serde::Serialize;
use serde_json::{json, Value};
use std::{path::Path, sync::Mutex};
use tauri::{Emitter, Manager, State};
use tokio::sync::OwnedMutexGuard;

pub(crate) struct ExitLease {
    pub(crate) _interview: OwnedMutexGuard<()>,
    pub(crate) _network: tokio::sync::OwnedRwLockWriteGuard<()>,
    pub(crate) _capture: OwnedMutexGuard<()>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all="snake_case")]
enum Phase { Requested, Preparing, Prepared, Archiving, Archived, Failed, Approved }
#[derive(Clone, Debug, Serialize)]
pub struct ExitRequest { pub request_id:u64, phase:Phase, restart:bool }
#[derive(Default)]
struct Gate { generation:u64, request:Option<ExitRequest>, lease:Option<ExitLease> }
#[derive(Default)]
pub struct ExitState { gate:Mutex<Gate>, operation:tokio::sync::Mutex<()> }
impl ExitState {
    fn request(&self,restart:bool)->ExitRequest {
        let mut gate=self.gate.lock().unwrap_or_else(|e|e.into_inner());
        if let Some(request)=&gate.request {return request.clone();}
        gate.generation+=1;
        let request=ExitRequest{request_id:gate.generation,phase:Phase::Requested,restart};
        gate.request=Some(request.clone());request
    }
    fn snapshot(&self)->Option<ExitRequest> {self.gate.lock().unwrap_or_else(|e|e.into_inner()).request.clone()}
    fn generation(&self)->u64 {self.gate.lock().unwrap_or_else(|e|e.into_inner()).generation}
    pub fn approved(&self)->bool {self.snapshot().is_some_and(|r|r.phase==Phase::Approved)}
    fn set_phase(&self,id:u64,allowed:&[Phase],next:Phase)->Result<(),String> {
        let mut gate=self.gate.lock().unwrap_or_else(|e|e.into_inner());
        let request=gate.request.as_mut().filter(|r|r.request_id==id).ok_or("退出请求已失效，请重新退出")?;
        if !allowed.contains(&request.phase){return Err("退出步骤尚未完成，请等待当前操作或重试".into());}
        request.phase=next;Ok(())
    }
    async fn prepare<R:tauri::Runtime>(&self,id:u64,app:&tauri::AppHandle<R>)->Result<(),String> {
        let _operation=self.operation.try_lock().map_err(|_|"退出清理正在进行，请等待")?;
        self.set_phase(id,&[Phase::Requested,Phase::Prepared,Phase::Archived,Phase::Failed],Phase::Preparing)?;
        if self.gate.lock().unwrap_or_else(|e|e.into_inner()).lease.is_some() {
            return self.set_phase(id,&[Phase::Preparing],Phase::Prepared);
        }
        match app.state::<crate::interview::InterviewState>().quiesce_for_exit(app).await {
            Ok(lease)=>{
                self.gate.lock().unwrap_or_else(|e|e.into_inner()).lease=Some(lease);
                self.set_phase(id,&[Phase::Preparing],Phase::Prepared)
            }
            Err(error)=>{self.set_phase(id,&[Phase::Preparing],Phase::Failed)?;Err(error)}
        }
    }
    pub async fn archive_with<F>(&self,id:u64,pool:&sqlx::SqlitePool,root:&Path,recovery:Value,trash:F)->Result<Option<String>,String>
    where F:FnOnce(&Path)->Result<std::path::PathBuf,String> {
        let _operation=self.operation.try_lock().map_err(|_|"退出清理正在进行，请等待")?;
        self.set_phase(id,&[Phase::Prepared],Phase::Archiving)?;
        let result=async {
            let meetings=recovery.get("meetings").and_then(Value::as_array).ok_or("恢复缓存缺少会议快照，未清理")?;
            let transcripts=recovery.get("transcripts").and_then(Value::as_array).ok_or("恢复缓存缺少转写快照，未清理")?;
            let residue=crate::interview_history::residue_status(pool,root).await?;
            if residue.recording_folders+residue.staged_archives+residue.database_meetings==0 && meetings.is_empty() && transcripts.is_empty(){return Ok(None);}
            let report=crate::interview_history::clear_temporary_history_with(pool,root,json!({"reason":"normal_exit","recovery":recovery}),trash).await?;
            Ok::<_,String>(Some(report.archive_path))
        }.await;
        self.set_phase(id,&[Phase::Archiving],if result.is_ok(){Phase::Archived}else{Phase::Failed})?;
        result
    }
    fn cancel(&self,id:u64)->Result<(),String> {
        let _operation=self.operation.try_lock().map_err(|_|"正在停止或归档，请等待操作结束")?;
        let mut gate=self.gate.lock().unwrap_or_else(|e|e.into_inner());
        let request=gate.request.as_ref().filter(|r|r.request_id==id).ok_or("退出请求已失效")?;
        if request.phase==Phase::Approved{return Err("应用已经准备退出".into());}
        gate.request=None;
        gate.lease=None; // Release all guards together; never restart devices or AI.
        Ok(())
    }
    fn approve(&self,id:u64)->Result<bool,String> {
        let _operation=self.operation.try_lock().map_err(|_|"退出清理正在进行，请等待")?;
        self.set_phase(id,&[Phase::Archived],Phase::Approved)?;
        Ok(self.snapshot().unwrap().restart)
    }
}

pub(crate) fn ensure_not_exiting<R:tauri::Runtime>(app:&tauri::AppHandle<R>)->Result<(),String> {
    if app.try_state::<ExitState>().is_some_and(|s|s.snapshot().is_some()){Err("正在退出，已暂停新采集和新请求；可在退出提示中取消退出".into())}else{Ok(())}
}
pub(crate) fn ensure_history_generation<R:tauri::Runtime>(app:&tauri::AppHandle<R>,generation:u64)->Result<(),String> {
    ensure_not_exiting(app)?;
    if app.try_state::<ExitState>().is_some_and(|s|s.generation()!=generation){return Err("这份保存请求早于退出清理，已取消，避免重新写回旧记录".into());}Ok(())
}
pub(crate) fn request_exit<R:tauri::Runtime>(app:&tauri::AppHandle<R>,restart:bool) {
    let request=app.state::<ExitState>().request(restart);
    crate::tray::focus_main_window(app);
    let _=app.emit("interview-exit-requested",request);
}
#[tauri::command]
pub fn interview_exit_request<R:tauri::Runtime>(app:tauri::AppHandle<R>,restart:Option<bool>) {request_exit(&app,restart.unwrap_or(false));}
#[tauri::command]
pub fn interview_exit_status(state:State<'_,ExitState>)->Option<ExitRequest> {state.snapshot()}
#[tauri::command]
pub fn interview_exit_generation(state:State<'_,ExitState>)->u64 {state.generation()}
#[tauri::command]
pub async fn interview_exit_prepare<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,ExitState>,request_id:u64)->Result<(),String> {state.prepare(request_id,&app).await}
#[tauri::command]
pub async fn interview_exit_archive<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,ExitState>,request_id:u64,recovery:Value)->Result<Option<String>,String> {
    let prefs=crate::audio::recording_preferences::load_recording_preferences(&app).await.map_err(|e|e.to_string())?;
    state.archive_with(request_id,app.state::<crate::state::AppState>().db_manager.pool(),&prefs.save_folder,recovery,crate::interview_history::move_to_trash).await
}
#[tauri::command]
pub fn interview_exit_cancel(state:State<'_,ExitState>,request_id:u64)->Result<(),String> {state.cancel(request_id)}
#[tauri::command]
pub fn interview_exit_finish<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,ExitState>,request_id:u64)->Result<(),String> {
    if state.approve(request_id)? {app.request_restart();}else{app.exit(0);}Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quit_requires_archive_and_current_generation_and_keeps_restart_intent() {
        let gate=ExitState::default();let first=gate.request(true);
        assert!(gate.approve(first.request_id).is_err());
        assert_eq!(gate.request(false).request_id,first.request_id);
        gate.cancel(first.request_id).unwrap();let next=gate.request(false);
        assert!(next.request_id>first.request_id);assert!(gate.cancel(first.request_id).is_err());
        gate.set_phase(next.request_id,&[Phase::Requested],Phase::Prepared).unwrap();
        assert!(gate.approve(next.request_id).is_err());
        gate.set_phase(next.request_id,&[Phase::Prepared],Phase::Archived).unwrap();
        assert!(!gate.approve(next.request_id).unwrap());assert!(gate.approved());
        assert!(gate.cancel(next.request_id).is_err());
        assert!(gate.approve(next.request_id).is_err());
    }
    #[tokio::test]
    async fn cleanup_failure_cannot_approve_and_empty_exit_does_not_create_a_trash_archive() {
        let state=ExitState::default();let request=state.request(false);
        state.set_phase(request.request_id,&[Phase::Requested],Phase::Prepared).unwrap();
        let pool=sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE meetings(id TEXT,title TEXT,folder_path TEXT)").execute(&pool).await.unwrap();
        let root=std::env::temp_dir().join(format!("meetily-exit-empty-{}",uuid::Uuid::new_v4()));
        assert!(state.archive_with(request.request_id,&pool,&root,json!({}),|_|panic!("invalid snapshot must not trash")).await.is_err());
        assert!(state.approve(request.request_id).is_err());
        state.set_phase(request.request_id,&[Phase::Failed],Phase::Prepared).unwrap();
        assert!(state.archive_with(request.request_id,&pool,&root,json!({"meetings":[],"transcripts":[]}),|_|panic!("empty exit must not trash")).await.unwrap().is_none());
        assert!(!root.exists());assert!(!state.approve(request.request_id).unwrap());
    }
    #[test]
    fn preparation_ipc_holds_lifecycle_until_cancel_and_blocks_new_work() {
        use tauri::Manager;
        let root=std::env::temp_dir().join(format!("meetily-exit-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let db=tauri::async_runtime::block_on(crate::database::manager::DatabaseManager::new(root.join("test.sqlite").to_str().unwrap(),root.join("no-legacy.sqlite").to_str().unwrap())).unwrap();
        let app=tauri::test::mock_builder().manage(ExitState::default()).manage(crate::interview::InterviewState::default())
            .manage(crate::state::AppState{db_manager:db})
            .invoke_handler(tauri::generate_handler![interview_exit_status,interview_exit_prepare,interview_exit_cancel,crate::api::api::api_save_transcript])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let window=tauri::WebviewWindowBuilder::new(&app,"test",Default::default()).build().unwrap();
        let invoke=|cmd:&str,body:Value|tauri::test::get_ipc_response(&window,tauri::webview::InvokeRequest {cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let request=app.state::<ExitState>().request(false);
        assert!(ensure_not_exiting(app.handle()).is_err());
        invoke("interview_exit_prepare",json!({"requestId":request.request_id})).unwrap();
        assert!(invoke("api_save_transcript",json!({"meetingTitle":"Interview practice","transcripts":[],"folderPath":null,"authToken":null})).is_err(),"late frontend save must not recreate archived history");
        assert!(app.state::<crate::interview::InterviewState>().lifecycle.try_lock().is_err());
        let held=tauri::async_runtime::block_on(async {tokio::time::timeout(std::time::Duration::from_millis(20),crate::audio::common::acquire_engine_lifecycle_lock()).await.is_err()});assert!(held);
        assert!(app.state::<ExitState>().approve(request.request_id).is_err());
        invoke("interview_exit_cancel",json!({"requestId":request.request_id})).unwrap();
        assert!(invoke("api_save_transcript",json!({"meetingTitle":"Interview practice","transcripts":[],"folderPath":null,"authToken":null,"exitGeneration":0})).is_err(),"cancelled quit must not re-enable an old save request");
        assert!(invoke("api_save_transcript",json!({"meetingTitle":"synthetic fresh meeting","transcripts":[],"folderPath":null,"authToken":null,"exitGeneration":request.request_id})).is_ok(),"fresh save must work after cancelled quit");
        assert!(app.state::<crate::interview::InterviewState>().lifecycle.try_lock().is_ok());
        assert!(ensure_not_exiting(app.handle()).is_ok());
        tauri::async_runtime::block_on(app.state::<crate::state::AppState>().db_manager.pool().close());
    }
    #[cfg(target_os="macos")]
    #[tokio::test]
    #[ignore="creates isolated fixture and uses real macOS Trash; run explicitly"]
    async fn real_trash_exit_keeps_saved_answers_and_does_not_approve_on_failure() {
        use std::fs;
        let root=std::env::temp_dir().join(format!("meetily-exit-trash-{}",uuid::Uuid::new_v4()));fs::create_dir(&root).unwrap();
        let root=root.canonicalize().unwrap();let recordings=root.join("recordings");fs::create_dir(&recordings).unwrap();
        let owned=recordings.join("Interview practice_exit");fs::create_dir(&owned).unwrap();
        fs::write(owned.join("metadata.json"),json!({"meeting_name":"Interview practice","status":"completed"}).to_string()).unwrap();
        fs::write(owned.join("transcripts.json"),"synthetic exit speech").unwrap();
        let db=crate::database::manager::DatabaseManager::new(root.join("test.sqlite").to_str().unwrap(),root.join("no-legacy.sqlite").to_str().unwrap()).await.unwrap();
        let app=tauri::test::mock_builder().manage(ExitState::default()).manage(crate::interview::InterviewState::default())
            .manage(crate::state::AppState{db_manager:db}).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let pool=app.state::<crate::state::AppState>().db_manager.pool().clone();
        sqlx::query("INSERT INTO meetings(id,title,created_at,updated_at,folder_path) VALUES('temporary','Interview practice',datetime('now'),datetime('now'),?)").bind(owned.to_str().unwrap()).execute(&pool).await.unwrap();
        let snapshot=serde_json::from_value(json!({"version":1,"history":[{"id":"saved-question","role":"remote","question":"synthetic saved question","suggestion":"synthetic saved AI answer","confirmed_answer":null,"status":"done","sources":[],"time":"00:01","timing":""}],"transcripts":[],"spoken_dialogue":[],"current":{"question":"synthetic saved question","answer":"synthetic saved AI answer"},"calls":1,"usage":"unknown"})).unwrap();
        let saved=crate::interview_sessions::save(&pool,snapshot,true).await.unwrap();
        let state=app.state::<ExitState>();let request=state.request(false);
        state.prepare(request.request_id,app.handle()).await.unwrap();
        let recovery=json!({"meetings":[{"meetingId":"synthetic"}],"transcripts":[{"text":"synthetic cache speech"}]});
        assert!(state.archive_with(request.request_id,&pool,&recordings,recovery.clone(),|_|Err("synthetic Trash unavailable".into())).await.is_err());
        assert!(state.approve(request.request_id).is_err());assert!(owned.exists());
        assert!(crate::interview_sessions::load(&pool,saved.id).await.is_ok());
        assert!(app.state::<crate::interview::InterviewState>().lifecycle.try_lock().is_err());
        state.prepare(request.request_id,app.handle()).await.unwrap();
        let backup=state.archive_with(request.request_id,&pool,&recordings,recovery.clone(),crate::interview_history::move_to_trash).await.unwrap().unwrap();
        let archived:Value=serde_json::from_slice(&fs::read(Path::new(&backup).join("history.json")).unwrap()).unwrap();
        assert_eq!(archived["session_and_recovery"]["recovery"],recovery);
        assert_eq!(archived["includes_saved_sessions"],false);assert!(!owned.exists());
        assert_eq!(crate::interview_sessions::load(&pool,saved.id).await.unwrap().current.answer,"synthetic saved AI answer");
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM meetings").fetch_one(&pool).await.unwrap(),0);
        assert!(!state.approve(request.request_id).unwrap()); // Deliberately do not exit the test host.
        assert!(state.approved());pool.close().await;
        println!("PASS: real Trash exit archive, failed-then-retried cleanup, retained saved answers; fixture {}",root.display());
    }
}
