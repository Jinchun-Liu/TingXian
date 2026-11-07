//! Main owner only. Independent storage and bounded tasks; no device commands.
use interview_core::education::{self as core,Request,ReviewResult};
use serde_json::{json,Value};
use std::sync::Arc;
use tauri::{Manager,State,WebviewWindow};
use tokio::sync::OnceCell;

#[derive(Default)]
pub struct EducationState {pool:OnceCell<sqlx::SqlitePool>,tasks:Arc<core::Tasks>}
impl EducationState {
    async fn pool<R:tauri::Runtime>(&self,app:&tauri::AppHandle<R>)->Result<&sqlx::SqlitePool,String>{
        self.pool.get_or_try_init(||async{let dir=app.path().app_data_dir().map_err(|_|"无法定位教育记录目录")?.join("education");tokio::fs::create_dir_all(&dir).await.map_err(|_|"无法创建教育记录目录")?;core::open(&dir.join("learning.sqlite")).await}).await
    }
}
fn owner<R:tauri::Runtime>(window:&WebviewWindow<R>)->Result<(),String>{if window.label()=="main"{Ok(())}else{Err("展示窗不能读取或操作教育记录".into())}}
#[tauri::command]
pub async fn education_list<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,EducationState>)->Result<Vec<Value>,String>{owner(&window)?;core::list(state.pool(window.app_handle()).await?).await}
#[tauri::command]
pub async fn education_load<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,EducationState>,id:String)->Result<Value,String>{owner(&window)?;core::load(state.pool(window.app_handle()).await?,&id).await}
#[tauri::command]
pub async fn education_save<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,EducationState>,document:Value,expected:u64)->Result<u64,String>{owner(&window)?;core::save(state.pool(window.app_handle()).await?,document,expected).await}
#[tauri::command]
pub fn education_cancel<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,EducationState>,session_id:String,generation:u64)->Result<(),String>{owner(&window)?;state.tasks.cancel(&session_id,generation);Ok(())}
#[tauri::command]
pub async fn education_preview<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,EducationState>,interview:State<'_,crate::interview::InterviewState>,request:Request)->Result<Value,String>{
    owner(&window)?;let (_,budget)=core::messages(&request)?;
    let config=interview.index(window.app_handle()).await?.provider_config().await?;
    let profile=config.answers.iter().chain([&config.background]).find(|p|p.id==request.profile_id&&p.enabled).ok_or("请选择已启用的文字模型接口")?;
    let key=interview.provider_key(profile).await;
    if key.is_empty()&&!profile.accepts_empty_key(){return Err("需要在模型接口中配置自己的生成模型；仍可手动练习和保存".into());}
    Ok(json!({"budget":budget,"scope_epoch":state.tasks.scope(),"model":profile.request.model,"endpoint":profile.endpoint,"profile_stamp":interview_core::digest(serde_json::to_string(profile).unwrap().as_bytes()),"output_reserve":interview_core::model::MAX_ANSWER_TOKENS,"context_limit":request.context_limit,"estimate":"UTF-8 字节保守上界，不是精确 tokenizer"}))
}
#[tauri::command]
pub async fn education_generate<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,EducationState>,interview:State<'_,crate::interview::InterviewState>,request:Request)->Result<ReviewResult,String>{
    owner(&window)?;crate::interview_exit::ensure_not_exiting(window.app_handle())?;
    let epoch=request.scope_epoch;
    if epoch!=state.tasks.scope(){return Err("全局取消后原预览已失效，请重新预览".into());}
    if !request.confirmed{return Err("需明确确认本次发送范围".into());}core::messages(&request)?;
    if request.lane=="background"&&interview.education_assist_active(){return Err("实时辅助活跃，后台评估已暂停；可稍后明确继续".into());}
    if let Some(result)=state.tasks.cached(&request)?{return Ok(result);}
    let _network=interview.network.read().await;
    let config=interview.index(window.app_handle()).await?.provider_config().await?;
    let profile=config.answers.iter().chain([&config.background]).find(|p|p.id==request.profile_id&&p.enabled).ok_or("所选生成模型已变化，请重新预览")?;
    if request.profile_stamp!=interview_core::digest(serde_json::to_string(profile).unwrap().as_bytes()){return Err("模型配置已变化，请重新预览本次发送范围".into());}
    let key=interview.provider_key(profile).await;
    crate::interview_exit::ensure_not_exiting(window.app_handle())?;
    let lease=state.tasks.begin_scoped(&request,epoch)?;
    let work=core::evaluate(&request,profile,&key,lease.cancel.clone());tokio::pin!(work);
    let mut ticker=tokio::time::interval(std::time::Duration::from_millis(100));
    let result=loop{tokio::select!{result=&mut work=>break result,_=ticker.tick()=>{if crate::interview_exit::ensure_not_exiting(window.app_handle()).is_err()||(request.lane=="background"&&interview.education_assist_active()){lease.cancel.cancel();}}}}?;
    state.tasks.complete(&request,result.clone());Ok(result)
}
pub(crate) fn cancel_all<R:tauri::Runtime>(app:&tauri::AppHandle<R>){if let Some(state)=app.try_state::<EducationState>(){state.tasks.cancel_all();}}
pub(crate) async fn quiesce<R:tauri::Runtime>(app:&tauri::AppHandle<R>)->Result<(),String>{
    if let Some(state)=app.try_state::<EducationState>(){state.tasks.cancel_all();let wait=async{while !state.tasks.idle(){tokio::time::sleep(std::time::Duration::from_millis(20)).await;}};tokio::time::timeout(std::time::Duration::from_secs(5),wait).await.map_err(|_|"教育请求尚未确认停止，退出保留")?;}Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    #[tokio::test]
    async fn global_assist_cancel_and_clear_drain_education_network_holders(){
        let app=tauri::test::mock_builder().manage(EducationState::default()).manage(crate::interview::InterviewState::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        for clear in [false,true]{
            let request:Request=serde_json::from_value(json!({"task_id":if clear{"clear"}else{"cancel"},"session_id":"synthetic","generation":1,"lane":"interactive","input_json":"{}","input_hash":"synthetic","profile_id":"answer-1","context_limit":32768,"confirmed":true})).unwrap();
            let state=app.state::<EducationState>();let epoch=state.tasks.scope();let lease=state.tasks.begin(&request).unwrap();
            let network=app.state::<crate::interview::InterviewState>().network.clone().read_owned().await;
            let drain=tokio::spawn(async move{lease.cancel.cancelled().await;drop(network);drop(lease);});
            let cancel=async{if clear{crate::interview::session_clear(app.handle().clone(),app.state()).await}else{crate::interview::answer_cancel(app.handle().clone(),app.state()).await}};
            tokio::time::timeout(std::time::Duration::from_secs(3),cancel).await.expect("education blocked assist cancellation").unwrap();
            drain.await.unwrap();assert!(state.tasks.idle());assert!(state.tasks.scope()>epoch);
        }
    }
    #[test]
    fn education_ipc_rejects_display_and_quit_cancels_without_restart(){
        let app=tauri::test::mock_builder().manage(EducationState::default()).manage(crate::interview::InterviewState::default()).manage(crate::interview_exit::ExitState::default())
            .invoke_handler(tauri::generate_handler![education_list,education_load,education_save,education_cancel,education_preview,education_generate])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let display=tauri::WebviewWindowBuilder::new(&app,"interview-conversation",Default::default()).build().unwrap();
        let request=json!({"task_id":"task-1","session_id":"study-1","generation":2,"lane":"interactive","input_json":"{}","input_hash":"invalid","profile_id":"answer-1","context_limit":32768,"confirmed":true});
        let invoke=|window:&tauri::WebviewWindow<tauri::test::MockRuntime>,cmd:&str|tauri::test::get_ipc_response(window,tauri::webview::InvokeRequest{cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:if cfg!(windows){"http://tauri.localhost"}else{"tauri://localhost"}.parse().unwrap(),body:tauri::ipc::InvokeBody::Json(json!({"id":"study-1","document":{},"expected":0,"sessionId":"study-1","generation":3,"request":request})),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.into()});
        for cmd in ["education_list","education_load","education_save","education_cancel","education_preview","education_generate"]{assert_eq!(invoke(&display,cmd).err(),Some(json!("展示窗不能读取或操作教育记录")),"{cmd}");}
        assert!(app.state::<EducationState>().pool.get().is_none(),"rejected display must never open storage");
        assert!(invoke(&main,"education_preview").is_err());assert!(app.state::<EducationState>().pool.get().is_none());
        let r:Request=serde_json::from_value(request.clone()).unwrap();let lease=app.state::<EducationState>().tasks.begin(&r).unwrap();
        crate::interview_exit::request_exit(app.handle(),false);assert!(lease.cancel.is_cancelled());assert!(invoke(&main,"education_generate").is_err());drop(lease);assert!(app.state::<EducationState>().tasks.idle());
    }
}
