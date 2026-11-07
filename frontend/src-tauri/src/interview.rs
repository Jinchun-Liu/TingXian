//! Interview assistance: explicit external sharing and recoverable local snapshots.
use interview_core::{Index, Selection, Turn, DEFAULT_ALIASES};
use serde_json::{json, Value};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tauri::{Emitter, Manager, State};
use tokio::sync::{Mutex, OnceCell, RwLock};
use tokio_util::sync::CancellationToken;
use tauri_plugin_dialog::DialogExt;

static SYSTEM_LEVEL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static MICROPHONE_LEVEL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub(crate) fn observe_microphone_audio(data:&[f32]) {
    if !data.is_empty() {
        let rms=(data.iter().map(|v|v*v).sum::<f32>() / data.len() as f32).sqrt();
        MICROPHONE_LEVEL.store(rms.to_bits(),Ordering::Relaxed);
    }
}
pub(crate) fn observe_system_audio(data:&[f32]) {
    if !data.is_empty() {
        let rms=(data.iter().map(|v|v*v).sum::<f32>() / data.len() as f32).sqrt();
        SYSTEM_LEVEL.store(rms.to_bits(),Ordering::Relaxed);
    }
}
#[tauri::command]
pub fn interview_audio_level()->Value { json!({
    "system": f32::from_bits(SYSTEM_LEVEL.swap(0,Ordering::Relaxed)),
    "microphone": f32::from_bits(MICROPHONE_LEVEL.swap(0,Ordering::Relaxed)),
    "microphone_state": crate::audio::recording_commands::microphone_status()
}) }

#[derive(Default)]
pub struct InterviewState {
    index: OnceCell<Index>,
    active: Arc<crate::interview_jobs::AnswerJobs>,
    provider_keys: Mutex<std::collections::HashMap<String,(String,String)>>,
    pub(crate) lifecycle: Arc<Mutex<()>>,
    imports: Arc<std::sync::Mutex<Option<ProfileImportJob>>>,
    network: Arc<RwLock<()>>,
    pub(crate) session: AtomicBool,
    pub(crate) background:crate::interview_background::Work,
    pub(crate) cloud_asr:crate::interview_cloud_asr::Credentials,
    key: Mutex<Option<String>>,
    connection: crate::interview_connection::ConnectionState,
}
impl InterviewState {
    pub(crate) fn source_network(&self)->Arc<RwLock<()>> {self.network.clone()}
    pub(crate) async fn index<R:tauri::Runtime>(&self, app:&tauri::AppHandle<R>)->Result<&Index,String> {
        self.index.get_or_try_init(||async {
            let dir=app.path().app_data_dir().map_err(|e|e.to_string())?.join("interview");
            tokio::fs::create_dir_all(&dir).await.map_err(|e|e.to_string())?;
            Index::open(&dir.join("profile.sqlite")).await
        }).await
    }
    pub(crate) async fn api_key(&self)->String { self.key.lock().await.clone().unwrap_or_else(||std::env::var("DEEPSEEK_API_KEY").unwrap_or_default()) }
    pub(crate) async fn provider_key(&self,profile:&interview_core::ProviderProfile)->String {
        if let Some((endpoint,key))=self.provider_keys.lock().await.get(&profile.id){
            if endpoint==&profile.endpoint{return key.clone();}
        }
        if profile.id=="answer-1" && profile.endpoint==interview_core::DEEPSEEK_ENDPOINT {self.api_key().await}else{String::new()}
    }
    async fn ready_profiles(&self,config:&interview_core::ProviderConfig)->Vec<String>{
        let mut ready=vec![];
        for p in &config.answers {if p.enabled && (p.accepts_empty_key() || !self.provider_key(p).await.is_empty()){ready.push(p.id.clone());}}
        ready
    }
    async fn cancel(&self) { self.active.cancel_all(); self.connection.cancel(None); }
    async fn cancel_answers(&self)->Result<(),String> {
        // Interrupt a connection probe before waiting for its lifecycle lock.
        self.cancel().await;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(35);
        let message="取消仍在收尾，已暂停自动回答；请等待后重试取消";
        let _guard=tokio::time::timeout_at(deadline,self.lifecycle.lock()).await.map_err(|_|message)?;
        self.cancel().await;
        // A cancelled screenshot worker or stream still owns its slot until
        // it exits. Acknowledging earlier lets the next queued question fail
        // against an apparently free provider. Keep new starts behind us.
        let _network=tokio::time::timeout_at(deadline,self.network.write()).await.map_err(|_|message)?;
        Ok(())
    }
    pub(crate) async fn quiesce_for_exit<R:tauri::Runtime>(&self,app:&tauri::AppHandle<R>)->Result<crate::interview_exit::ExitLease,String> {
        self.background.cancel();
        self.cancel().await;
        {
            let mut jobs=self.imports.lock().unwrap_or_else(|e|e.into_inner());
            if let Some(job)=jobs.as_mut().filter(|j|!["completed","cancelled","failed"].contains(&j.latest.phase.as_str())) {
                if job.control.cancel()==interview_core::ImportCancelStatus::Requested {job.latest.phase="cancelling".into();}
            }
        }
        let wait=std::time::Duration::from_secs(35);
        let interview=tokio::time::timeout(wait,self.lifecycle.clone().lock_owned()).await.map_err(|_|"资料操作尚未结束，退出已暂停；请等待后重试")?;
        self.cancel().await;
        let network=tokio::time::timeout(wait,self.network.clone().write_owned()).await.map_err(|_|"AI请求尚未退出，未清理记录；请稍后重试")?;
        if crate::audio::recording_commands::is_recording().await {
            crate::audio::recording_commands::stop_recording(app.clone(),crate::audio::recording_commands::RecordingArgs{save_path:String::new()}).await?;
        }
        let capture=tokio::time::timeout(wait,crate::audio::common::acquire_engine_lifecycle_lock()).await.map_err(|_|"语音引擎尚未空闲，退出已暂停；请等待后重试")?;
        if crate::audio::recording_commands::is_recording().await {return Err("采集尚未结束，未清理记录".into());}
        self.session.store(false,Ordering::SeqCst);
        Ok(crate::interview_exit::ExitLease{_interview:interview,_network:network,_capture:capture})
    }
}

#[tauri::command]
pub async fn interview_status(app:tauri::AppHandle,state:State<'_,InterviewState>)->Result<Value,String> {
    let index=state.index(&app).await?;
    let aliases=index.setting("aliases").await?;
    let documents:i64=sqlx::query_scalar("SELECT count(*) FROM documents").fetch_one(&index.pool).await.map_err(|e|e.to_string())?;
    let chunks:i64=sqlx::query_scalar("SELECT count(*) FROM chunks").fetch_one(&index.pool).await.map_err(|e|e.to_string())?;
    let providers=index.provider_config().await?;
    let ready=state.ready_profiles(&providers).await;
    let request_config=&providers.answers[0].request;
    Ok(json!({"architecture":std::env::consts::ARCH,"os":std::env::consts::OS,
        "memory_bytes":sysinfo::System::new_all().total_memory(),
        "system_version":sysinfo::System::os_version(),"key_configured":!state.api_key().await.is_empty(),"workspace_ready":!ready.is_empty(),"answer_capacity":ready.len(),"connection":state.connection.snapshot(),
        "vision_ready":providers.vision.enabled && (providers.vision.accepts_empty_key() || !state.provider_key(&providers.vision).await.is_empty()),"vision_model":providers.vision.request.model,
        "preferences":index.preferences().await?,"model":request_config.model,"root":index.setting("root").await?,"suggested_root":dirs::home_dir().map(|p|p.join("Documents").to_string_lossy().into_owned()),"index_version":index.setting("version").await?,
        "session_active":state.session.load(Ordering::SeqCst),"index_documents":documents,"index_chunks":chunks,"documents":index.documents().await?,"aliases":if aliases.is_empty(){DEFAULT_ALIASES}else{&aliases},
        "facts":index.setting("facts").await?,"fact_card":index.fact_card().await?,"whisper_model":crate::whisper_engine::commands::whisper_get_current_model().await.unwrap_or(None),
        "cloud_asr_mode":index.cloud_asr_config().await?.mode,
        "audio_validation":"未验收；上游混合音频事件不提供可靠说话人身份"}))
}
#[tauri::command]
pub async fn interview_preferences_get<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>)->Result<interview_core::InterviewPreferences,String> {
    state.index(&app).await?.preferences().await
}
#[tauri::command]
pub async fn interview_aliases_state<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>)->Result<interview_core::alias_sources::AliasState,String> {
    if window.label()!="main" {return Err("请在主窗口查看别名".into());}
    state.index(&app).await?.alias_state().await
}
#[tauri::command]
pub async fn interview_aliases_save<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>,aliases:String,expected:String)->Result<(),String> {
    if window.label()!="main" {return Err("请在主窗口修改别名".into());}
    crate::interview_exit::ensure_not_exiting(&app)?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前操作")?;
    if state.session.load(Ordering::SeqCst)||crate::audio::recording_commands::is_recording().await {return Err("请结束当前会话后修改别名".into());}
    crate::interview_exit::ensure_not_exiting(&app)?;
    state.index(&app).await?.set_aliases(&aliases,Some(&expected)).await
}
#[tauri::command]
pub async fn interview_aliases_manual<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>,lines:Vec<String>,expected:String)->Result<(),String> {
    if window.label()!="main" {return Err("请在主窗口核对别名".into());}
    crate::interview_exit::ensure_not_exiting(&app)?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前操作")?;
    if state.session.load(Ordering::SeqCst)||crate::audio::recording_commands::is_recording().await {return Err("请结束当前会话后核对别名".into());}
    crate::interview_exit::ensure_not_exiting(&app)?;
    state.index(&app).await?.confirm_manual_aliases(&lines,&expected).await
}
#[tauri::command]
pub async fn interview_preferences_state<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>)->Result<interview_core::preference_sources::PreferenceState,String> {
    if window.label()!="main" {return Err("请在主窗口查看设置来源".into());}
    state.index(&app).await?.preference_state().await
}
#[tauri::command]
pub async fn interview_preferences_manual<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>,fields:Vec<interview_core::preference_sources::ManualSetting>)->Result<interview_core::preference_sources::PreferenceState,String> {
    if window.label()!="main" {return Err("请在主窗口核对设置".into());}
    crate::interview_exit::ensure_not_exiting(&app)?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前操作")?;
    if state.session.load(Ordering::SeqCst)||crate::audio::recording_commands::is_recording().await {return Err("请结束当前会话后核对设置".into());}
    crate::interview_exit::ensure_not_exiting(&app)?;
    state.index(&app).await?.confirm_manual_settings(&fields).await
}
#[tauri::command]
pub async fn interview_preferences_save<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,preferences:interview_core::InterviewPreferences,expected:Option<interview_core::InterviewPreferences>)->Result<interview_core::InterviewPreferences,String> {
    crate::interview_exit::ensure_not_exiting(&app)?;
    preferences.validate()?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前操作")?;
    crate::interview_exit::ensure_not_exiting(&app)?;
    if state.session.load(Ordering::SeqCst) || crate::audio::recording_commands::is_recording().await {return Err("请结束当前会话后修改对话场景".into());}
    state.index(&app).await?.set_preferences_expected(&preferences,expected.as_ref()).await?;Ok(preferences)
}
#[tauri::command]
pub async fn interview_request_config_get<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>)->Result<interview_core::RequestConfig,String> {
    state.index(&app).await?.request_config().await
}
#[tauri::command]
pub async fn interview_providers_get<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>)->Result<Value,String> {
    let config=state.index(&app).await?.provider_config().await?;
    let mut configured=serde_json::Map::new();
    for profile in config.answers.iter().chain([&config.vision,&config.background]) {
        configured.insert(profile.id.clone(),json!(!state.provider_key(profile).await.is_empty()));
    }
    Ok(json!({"config":config,"key_configured":configured,"capacity":state.ready_profiles(&config).await.len()}))
}
#[tauri::command]
pub async fn interview_providers_save<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,config:interview_core::ProviderConfig,keys:std::collections::HashMap<String,String>)->Result<(),String> {
    crate::interview_exit::ensure_not_exiting(&app)?;config.validate()?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成当前操作")?;
    if state.session.load(Ordering::SeqCst) || crate::audio::recording_commands::is_recording().await {return Err("请结束当前会话后修改接口".into());}
    let _network=state.network.try_write().map_err(|_|"请求正在结束，请稍后保存")?;
    for (id,key) in &keys {
        if !config.answers.iter().chain([&config.vision,&config.background]).any(|p|&p.id==id) || key.len()>4096 || key.chars().any(char::is_control) {return Err("接口凭证格式无效".into());}
    }
    state.index(&app).await?.set_provider_config(&config).await?;
    let mut stored=state.provider_keys.lock().await;
    for profile in config.answers.iter().chain([&config.vision,&config.background]) {
        if let Some(key)=keys.get(&profile.id){stored.insert(profile.id.clone(),(profile.endpoint.clone(),key.trim().into()));}
        else if stored.get(&profile.id).is_some_and(|(endpoint,_)|endpoint!=&profile.endpoint){stored.remove(&profile.id);}
    }
    state.connection.invalidate();Ok(())
}
#[tauri::command]
pub async fn interview_provider_test<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,profile_id:String)->Result<interview_core::ProbeOutcome,String> {
    crate::interview_exit::ensure_not_exiting(&app)?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请等待当前操作结束")?;
    if state.session.load(Ordering::SeqCst) || crate::audio::recording_commands::is_recording().await {return Err("请结束会话后测试接口".into());}
    let _network=state.network.try_write().map_err(|_|"请求正在结束")?;
    let config=state.index(&app).await?.provider_config().await?;
    let profile=config.answers.iter().chain([&config.vision,&config.background]).find(|p|p.id==profile_id).ok_or("接口不存在")?;
    let key=state.provider_key(profile).await;
    let check_id=format!("profile-{}-{}",profile.id,chrono::Utc::now().timestamp_millis());
    let cancel=state.connection.begin(&check_id,&profile.request.model)?;
    let start=std::time::Instant::now();let mut attempts=0;let mut total_tokens=None;
    let result=interview_core::stream_answer_with_provider(&key,interview_core::connection_test_messages(),cancel,profile,|event|match event {
        interview_core::StreamEvent::RequestSent{attempt}=>attempts=attempts.max(attempt),
        interview_core::StreamEvent::Usage{usage,..}=>total_tokens=usage["total_tokens"].as_u64(),
        _=>{},
    }).await;
    let outcome=interview_core::ProbeOutcome{success:result.is_ok(),message:interview_core::redact_credentials(&result.err().unwrap_or_else(||"连接成功（固定测试文字；视觉能力需图片联测）".into()),Some(&key)),elapsed_ms:start.elapsed().as_millis() as u64,attempts,total_tokens};
    state.connection.finish(&check_id,outcome.clone())?;Ok(outcome)
}
#[tauri::command]
pub async fn interview_request_config_save<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,config:interview_core::RequestConfig)->Result<interview_core::RequestConfig,String> {
    crate::interview_exit::ensure_not_exiting(&app)?;
    config.validate()?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前操作")?;
    crate::interview_exit::ensure_not_exiting(&app)?;
    if state.session.load(Ordering::SeqCst) || crate::audio::recording_commands::is_recording().await {return Err("请结束当前会话后修改回答请求设置".into());}
    state.index(&app).await?.set_request_config(&config).await?;state.connection.invalidate();Ok(config)
}
#[tauri::command]
pub async fn interview_config<R:tauri::Runtime>(app:tauri::AppHandle<R>,window:tauri::WebviewWindow<R>,state:State<'_,InterviewState>,facts:Option<String>,aliases:Option<String>,key:Option<String>)->Result<(),String> {
    if window.label()!="main" {return Err("请在主窗口修改配置".into());}
    crate::interview_exit::ensure_not_exiting(&app)?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前资料操作")?;
    if state.session.load(Ordering::SeqCst){return Err("请结束本次会话后修改资料配置".into());}
    let index=state.index(&app).await?;
    if let Some(text)=facts {if text!=index.fact_card().await?.text {return Err("请使用事实更正入口核对来源并确认，普通配置不会覆盖事实卡".into());}}
    if let Some(aliases)=aliases {index.set_aliases(&aliases,None).await?;}
    if let Some(key)=key {if !key.trim().is_empty(){*state.key.lock().await=Some(key.trim().into());state.connection.invalidate();}}
    Ok(())
}
#[tauri::command]
pub fn interview_connection_status(state:State<'_,InterviewState>)->crate::interview_connection::ConnectionReport {state.connection.snapshot()}
#[tauri::command]
pub fn interview_connection_cancel(state:State<'_,InterviewState>,check_id:String)->crate::interview_connection::ConnectionReport {state.connection.cancel(Some(&check_id))}
#[tauri::command]
pub async fn interview_connection_test<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,check_id:String,confirmed:bool)->Result<crate::interview_connection::ConnectionReport,String> {
    connection_test_with(&app,&state,check_id,confirmed,|key,config,cancel|async move {interview_core::probe_connection(&key,&config,cancel).await}).await
}
async fn connection_test_with<R,F,Fut>(app:&tauri::AppHandle<R>,state:&InterviewState,check_id:String,confirmed:bool,runner:F)->Result<crate::interview_connection::ConnectionReport,String>
where R:tauri::Runtime,F:FnOnce(String,interview_core::RequestConfig,CancellationToken)->Fut,Fut:std::future::Future<Output=interview_core::ProbeOutcome> {
    if !confirmed {return Err("请点击连接测试后再发送固定测试文字".into());}
    crate::interview_exit::ensure_not_exiting(app)?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前操作")?;
    crate::interview_exit::ensure_not_exiting(app)?;
    if state.session.load(Ordering::SeqCst) || crate::audio::recording_commands::is_recording().await {return Err("请结束当前会话后测试连接".into());}
    let _network=state.network.try_write().map_err(|_|"模型请求仍在结束，请稍后测试连接")?;
    let key=state.api_key().await;if key.trim().is_empty(){return Err("尚未配置 API Key；请先在资料与设置中填写".into());}
    let config=state.index(app).await?.request_config().await?;
    let cancel=state.connection.begin(&check_id,&config.model)?;
    if let Err(error)=crate::interview_exit::ensure_not_exiting(app) {
        state.connection.cancel(Some(&check_id));
        state.connection.finish(&check_id,interview_core::ProbeOutcome{success:false,message:error.clone(),elapsed_ms:0,attempts:0,total_tokens:None})?;
        return Err(error);
    }
    let mut result=runner(key.clone(),config,cancel).await;
    result.message=interview_core::redact_credentials(&result.message,Some(&key));
    state.connection.finish(&check_id,result)
}
#[tauri::command]
pub async fn profile_facts_correct<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,update:interview_core::FactCorrection)->Result<interview_core::FactCard,String> {
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前资料操作")?;
    if state.session.load(Ordering::SeqCst) {return Err("会话期间事实卡已锁定，请先结束会话再更正".into());}
    state.index(&app).await?.correct_facts(update).await
}
#[tauri::command]
pub async fn profile_facts_current<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>)->Result<interview_core::FactCard,String> {
    state.index(&app).await?.fact_card().await
}
#[tauri::command]
pub async fn profile_facts_history<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,before:Option<i64>)->Result<interview_core::FactHistory,String> {
    state.index(&app).await?.fact_history(before).await
}
#[tauri::command]
pub async fn profile_choose(app:tauri::AppHandle)->Result<Option<String>,String> {
    tauri::async_runtime::spawn_blocking(move||app.dialog().file().blocking_pick_folder().map(|p|p.to_string())).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn profile_scan(root:String)->Result<Vec<interview_core::Candidate>,String> {
    tauri::async_runtime::spawn_blocking(move||interview_core::scan(&root)).await.map_err(|e|e.to_string())?
}
#[tauri::command]
pub async fn profile_structured_preview(state:State<'_,InterviewState>,root:String,path:String)->Result<interview_core::StructuredPreview,String> {
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前资料操作")?;
    if state.session.load(Ordering::SeqCst){return Err("请结束会话后预览新的资料文件".into());}
    let python=crate::tingxian_runtime::document_python();
    interview_core::preview_structured(&root,&path,&python).await
}
#[derive(Clone, serde::Serialize)]
pub struct ProfileImportEvent {
    import_id: String,
    phase: String,
    completed: usize,
    total: usize,
    path: Option<String>,
    report: Option<interview_core::ImportReport>,
    message: Option<String>,
}
struct ProfileImportJob { control: interview_core::ImportControl, latest: ProfileImportEvent }
fn publish_import(app:&tauri::AppHandle, jobs:&std::sync::Mutex<Option<ProfileImportJob>>, event:ProfileImportEvent) {
    let mut slot=jobs.lock().unwrap_or_else(|e|e.into_inner());
    if let Some(job)=slot.as_mut().filter(|job|job.latest.import_id==event.import_id) {
        job.latest=event.clone();
        let _=app.emit("profile-import",event);
    }
}
#[tauri::command]
pub async fn profile_import(app:tauri::AppHandle,state:State<'_,InterviewState>,import_id:String,root:String,selected:Vec<Selection>)->Result<(),String> {
    crate::interview_exit::ensure_not_exiting(&app)?;
    if import_id.is_empty() || import_id.len()>100 || !import_id.chars().all(|c|c.is_ascii_alphanumeric() || c=='-') {return Err("导入任务 ID 无效".into());}
    let guard=state.lifecycle.clone().try_lock_owned().map_err(|_|"另一项操作正在进行，请稍后再导入")?;
    if state.session.load(Ordering::SeqCst){return Err("会话期间索引已锁定，请先结束会话".into());}
    let index=state.index(&app).await?.clone();
    let python=crate::tingxian_runtime::document_python();
    let control=interview_core::ImportControl::default();
    let initial=ProfileImportEvent {import_id:import_id.clone(),phase:"starting".into(),completed:0,total:selected.len(),path:None,report:None,message:None};
    let jobs=state.imports.clone();
    *jobs.lock().unwrap_or_else(|e|e.into_inner())=Some(ProfileImportJob {control:control.clone(),latest:initial});
    tauri::async_runtime::spawn(async move {
        // Supervise the task so an unexpected parser/tokenizer panic cannot
        // leave the UI permanently busy or retain the lifecycle lock.
        let progress_app=app.clone();let progress_jobs=jobs.clone();let progress_id=import_id.clone();
        let result=tauri::async_runtime::spawn(async move {
            index.import_controlled(&root,selected,&python,&control,|progress| {
                publish_import(&progress_app,&progress_jobs,ProfileImportEvent {import_id:progress_id.clone(),phase:progress.phase,completed:progress.completed,total:progress.total,path:progress.path,report:None,message:None});
            }).await
        }).await.unwrap_or_else(|_|Err("导入任务异常结束，请检查资料后重试".into()));
        let (phase,report,message)=match result {
            Ok(report)=>("completed",Some(report),None),
            Err(message) if message==interview_core::IMPORT_CANCELLED =>("cancelled",None,Some(message)),
            Err(message)=>("failed",None,Some(message)),
        };
        let total=jobs.lock().unwrap_or_else(|e|e.into_inner()).as_ref().map(|job|job.latest.total).unwrap_or(0);
        drop(guard);
        publish_import(&app,&jobs,ProfileImportEvent {import_id,phase:phase.into(),completed:report.as_ref().map(|r|r.included).unwrap_or(0),total,path:None,report,message});
    });
    Ok(())
}
#[tauri::command]
pub fn profile_import_cancel(state:State<'_,InterviewState>,import_id:String)->Result<interview_core::ImportCancelStatus,String> {
    let mut jobs=state.imports.lock().unwrap_or_else(|e|e.into_inner());
    let job=jobs.as_mut().filter(|job|job.latest.import_id==import_id).ok_or("导入任务已变更，请刷新状态")?;
    if ["completed","cancelled","failed"].contains(&job.latest.phase.as_str()) {return Ok(interview_core::ImportCancelStatus::Finished);}
    let status=job.control.cancel();
    if status==interview_core::ImportCancelStatus::Requested { job.latest.phase="cancelling".into(); }
    Ok(status)
}
#[tauri::command]
pub fn profile_import_status(state:State<'_,InterviewState>)->Option<ProfileImportEvent> {
    state.imports.lock().unwrap_or_else(|e|e.into_inner()).as_ref().map(|job|job.latest.clone())
}

#[tauri::command]
pub async fn profile_document_update<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,update:interview_core::DocumentUpdate)->Result<(),String> {
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前资料操作")?;
    if state.session.load(Ordering::SeqCst) {return Err("会话期间索引已锁定，请先结束会话再核对资料".into());}
    state.index(&app).await?.update_document(update).await
}
#[tauri::command]
pub async fn profile_document_segments<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,path:String,expected_root:String,expected_hash:String)->Result<Vec<interview_core::Segment>,String> {
    state.index(&app).await?.document_segments(&path,&expected_root,&expected_hash).await
}
#[tauri::command]
pub async fn profile_document_records<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>,path:String,expected_root:String,expected_hash:String)->Result<Vec<interview_core::assertions::RecordSegment>,String> {
    if window.label()!="main" {return Err("请在主窗口核对资料".into());}
    state.index(&app).await?.document_records(&path,&expected_root,&expected_hash).await
}
#[tauri::command]
pub async fn profile_record_revise<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>,update:interview_core::assertions::AssertionUpdate)->Result<interview_core::assertions::AssertionRecord,String> {
    if window.label()!="main" {return Err("请在主窗口修订资料".into());}
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前资料操作")?;
    crate::interview_exit::ensure_not_exiting(&app)?;
    if state.session.load(Ordering::SeqCst) {return Err("请先结束会话再修订资料".into());}
    let _network=state.network.try_write().map_err(|_|"请先取消在途回答再修订资料")?;
    state.index(&app).await?.revise_record(update).await
}
#[tauri::command]
pub async fn profile_record_history<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>,source:interview_core::FactSourceRef,before:Option<i64>)->Result<interview_core::assertions::AssertionHistory,String> {
    if window.label()!="main" {return Err("请在主窗口查看修订历史".into());}
    state.index(&app).await?.assertion_history(source,before).await
}
#[tauri::command]
pub async fn profile_document_revision_history<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>,path:String,root:String,offset:i64)->Result<interview_core::assertions::DocumentAssertionHistory,String> {
    if window.label()!="main" {return Err("请在主窗口查看修订历史".into());}
    state.index(&app).await?.document_assertion_history(&path,&root,offset).await
}
#[tauri::command]
pub async fn profile_index_clear<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>)->Result<crate::interview_index::IndexClearReport,String> {
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前操作，再清除资料索引")?;
    if state.session.load(Ordering::SeqCst) {return Err("会话期间索引已锁定，请先结束会话再清除索引".into());}
    let index=state.index(&app).await?;
    let root=app.path().app_data_dir().map_err(|e|e.to_string())?.join("interview");
    crate::interview_index::clear_index_with(index,&root,crate::interview_history::move_to_trash).await
}

#[cfg(test)]
mod import_command_tests {
    use super::*;
    #[tokio::test]
    async fn cancellation_acknowledgement_waits_for_workers_and_blocks_new_starts(){
        let state=Arc::new(InterviewState::default());
        let profiles=vec!["answer-1".into()];
        let lease=state.active.reserve("synthetic-question",1,&profiles).unwrap();
        let network=state.network.clone().read_owned().await;
        let worker=state.clone();let cancel=tokio::spawn(async move{worker.cancel_answers().await});
        lease.cancel.cancelled().await;
        assert!(!cancel.is_finished(),"cancel acknowledged while old worker still holds its slot");
        assert!(state.lifecycle.try_lock().is_err(),"new starts can overtake cancellation drain");
        assert!(state.active.reserve("next",2,&profiles).is_err());
        drop(lease);drop(network);
        cancel.await.unwrap().unwrap();
        assert!(state.lifecycle.try_lock().is_ok());
        assert!(state.network.try_write().is_ok());
        assert!(state.active.reserve("next",2,&profiles).is_ok());
    }
    #[tokio::test]
    async fn connection_checks_require_explicit_action_share_locks_and_cancel_without_touching_interview_history() {
        use crate::interview_connection::CheckStatus;
        let state=InterviewState::default();let root=std::env::temp_dir().join(format!("interview-connection-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let index=Index::open(&root.join("profile.sqlite")).await.unwrap();
        index.correct_facts(interview_core::FactCorrection{expected_version:0,text:"SYNTHETIC_PRIVATE_FACT_NEVER_SENT".into(),source_note:"Synthetic fixture".into(),sources:vec![],reason:"Synthetic connection isolation test".into(),confirmed:true}).await.unwrap();
        assert!(state.index.set(index).is_ok());*state.key.lock().await=Some("synthetic-configured-key".into());
        let app=tauri::test::mock_builder().manage(state).manage(crate::interview_exit::ExitState::default()).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let state=app.state::<InterviewState>();let id=uuid::Uuid::new_v4().to_string();
        let forbidden=|_:String,_:interview_core::RequestConfig,_:CancellationToken|async {panic!("preflight must not invoke a network runner")};
        assert!(connection_test_with(app.handle(),&state,id.clone(),false,forbidden).await.is_err());
        state.session.store(true,Ordering::SeqCst);assert!(connection_test_with(app.handle(),&state,id.clone(),true,forbidden).await.is_err());state.session.store(false,Ordering::SeqCst);
        let started=Arc::new(tokio::sync::Notify::new());let signal=started.clone();
        let running=connection_test_with(app.handle(),&state,id.clone(),true,move |key,config,cancel|async move {
            assert_eq!(key,"synthetic-configured-key");assert_eq!(config,interview_core::RequestConfig::default());signal.notify_one();cancel.cancelled().await;
            interview_core::ProbeOutcome{success:false,message:"已取消".into(),elapsed_ms:10,attempts:1,total_tokens:None}
        });
        let control=async {
            started.notified().await;assert_eq!(state.connection.snapshot().status,CheckStatus::Checking);
            assert!(state.lifecycle.try_lock().is_err());assert!(state.network.try_write().is_err());
            assert!(connection_test_with(app.handle(),&state,uuid::Uuid::new_v4().to_string(),true,forbidden).await.is_err());
            state.cancel().await;
        };
        let (result,())=tokio::join!(running,control);let report=result.unwrap();assert_eq!(report.status,CheckStatus::Cancelled);
        assert!(state.lifecycle.try_lock().is_ok());assert!(state.network.try_write().is_ok());assert!(!state.session.load(Ordering::SeqCst));assert!(state.active.is_empty());
        let index=state.index.get().unwrap();assert_eq!(index.setting("facts").await.unwrap(),"SYNTHETIC_PRIVATE_FACT_NEVER_SENT");assert!(index.documents().await.unwrap().is_empty());
        let report=connection_test_with(app.handle(),&state,uuid::Uuid::new_v4().to_string(),true,|key,_,_|async move {interview_core::ProbeOutcome{success:false,message:format!("synthetic failure {key}"),elapsed_ms:2,attempts:1,total_tokens:None}}).await.unwrap();
        assert!(!report.message.contains("synthetic-configured-key"));assert_eq!(report.status,CheckStatus::Failed);
    }
    #[test]
    fn preferences_ipc_rejects_active_session_and_preserves_valid_configuration() {
        let state=InterviewState::default();let lifecycle=state.lifecycle.clone();
        let root=std::env::temp_dir().join(format!("interview-preferences-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        assert!(state.index.set(tauri::async_runtime::block_on(Index::open(&root.join("profile.sqlite"))).unwrap()).is_ok());
        let app=tauri::test::mock_builder().manage(state).manage(crate::interview_exit::ExitState::default())
            .invoke_handler(tauri::generate_handler![interview_preferences_get,interview_preferences_save])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let window=tauri::WebviewWindowBuilder::new(&app,"test",Default::default()).build().unwrap();
        let invoke=|cmd:&str,body:Value|tauri::test::get_ipc_response(&window,tauri::webview::InvokeRequest {cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let mut custom=serde_json::to_value(interview_core::InterviewPreferences::default()).unwrap();custom["role"]="策展助理".into();custom["answer_language"]="en".into();
        let save=||invoke("interview_preferences_save",json!({"preferences":custom}));
        let guard=tauri::async_runtime::block_on(lifecycle.lock());assert!(save().is_err());drop(guard);
        let state=app.state::<InterviewState>();state.session.store(true,Ordering::SeqCst);assert!(save().is_err());state.session.store(false,Ordering::SeqCst);
        assert_eq!(save().unwrap().deserialize::<Value>().unwrap(),custom);
        let mut bad=custom.clone();bad["transcription_language"]="invalid".into();assert!(invoke("interview_preferences_save",json!({"preferences":bad})).is_err());
        assert_eq!(invoke("interview_preferences_get",json!({})).unwrap().deserialize::<Value>().unwrap(),custom);
    }
    #[test]
    fn request_config_ipc_preserves_active_session_and_busy_boundaries() {
        let state=InterviewState::default();let lifecycle=state.lifecycle.clone();
        let root=std::env::temp_dir().join(format!("interview-request-config-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        assert!(state.index.set(tauri::async_runtime::block_on(Index::open(&root.join("profile.sqlite"))).unwrap()).is_ok());
        let app=tauri::test::mock_builder().manage(state).manage(crate::interview_exit::ExitState::default())
            .invoke_handler(tauri::generate_handler![interview_request_config_get,interview_request_config_save,interview_connection_status,interview_connection_cancel,interview_connection_test,crate::interview_exit::interview_exit_request])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let window=tauri::WebviewWindowBuilder::new(&app,"test",Default::default()).build().unwrap();
        let invoke=|cmd:&str,body:Value|tauri::test::get_ipc_response(&window,tauri::webview::InvokeRequest {cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let defaults=serde_json::to_value(interview_core::RequestConfig::default()).unwrap();
        assert_eq!(invoke("interview_request_config_get",json!({})).unwrap().deserialize::<Value>().unwrap(),defaults);
        assert_eq!(invoke("interview_connection_status",json!({})).unwrap().deserialize::<Value>().unwrap()["status"],"untested");
        let cancelled_id=uuid::Uuid::new_v4().to_string();
        assert!(invoke("interview_connection_test",json!({"checkId":cancelled_id,"confirmed":false})).unwrap_err().to_string().contains("请点击连接测试"));
        assert!(invoke("interview_connection_cancel",json!({"checkId":cancelled_id})).is_ok());
        assert!(app.state::<InterviewState>().connection.begin(&cancelled_id,"synthetic-model").is_err());
        let passed_id=uuid::Uuid::new_v4().to_string();app.state::<InterviewState>().connection.begin(&passed_id,"deepseek-flash").unwrap();
        app.state::<InterviewState>().connection.finish(&passed_id,interview_core::ProbeOutcome{success:true,message:"ok".into(),elapsed_ms:1,attempts:1,total_tokens:Some(9)}).unwrap();
        let custom=json!({"model":"synthetic-model","connect_timeout_secs":2,"first_answer_warning_secs":3,"total_timeout_secs":10});
        // Legacy callers omit the budget; the IPC response must normalize it
        // to the app's 256K baseline while preserving the caller's values.
        let mut expected=custom.clone();
        expected["budget"]=json!({"context_tokens":262144,"max_output_tokens":8192,"safety_tokens":2048});
        let save=||invoke("interview_request_config_save",json!({"config":custom}));
        let guard=tauri::async_runtime::block_on(lifecycle.lock_owned());assert!(save().is_err());drop(guard);
        app.state::<InterviewState>().session.store(true,Ordering::SeqCst);assert!(save().is_err());
        app.state::<InterviewState>().session.store(false,Ordering::SeqCst);assert_eq!(save().unwrap().deserialize::<Value>().unwrap(),expected);
        assert_eq!(invoke("interview_connection_status",json!({})).unwrap().deserialize::<Value>().unwrap()["status"],"untested");
        let mut bad=custom.clone();bad["total_timeout_secs"]=2.into();assert!(invoke("interview_request_config_save",json!({"config":bad})).is_err());
        let mut bad=custom.clone();bad["extra"]=true.into();assert!(invoke("interview_request_config_save",json!({"config":bad})).is_err());
        assert_eq!(invoke("interview_request_config_get",json!({})).unwrap().deserialize::<Value>().unwrap(),expected);
        assert!(invoke("interview_exit_request",json!({"restart":false})).is_ok());assert!(save().is_err());
        assert!(!app.state::<InterviewState>().session.load(Ordering::SeqCst));
        assert!(app.state::<InterviewState>().active.is_empty());
    }
    #[test]
    fn structured_preview_ipc_is_read_only_and_rejects_busy_or_active_session() {
        let state=InterviewState::default();let lifecycle=state.lifecycle.clone();
        let root=std::env::temp_dir().join(format!("interview-structured-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("fields.json"),r#"{"topic":"synthetic preview","zero":0,"empty":"","nil":null}"#).unwrap();
        let app=tauri::test::mock_builder().manage(state).invoke_handler(tauri::generate_handler![profile_structured_preview])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let webview=tauri::WebviewWindowBuilder::new(&app,"test",Default::default()).build().unwrap();
        let invoke=|path:&str|tauri::test::get_ipc_response(&webview,tauri::webview::InvokeRequest {
            cmd:"profile_structured_preview".into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),
            body:tauri::ipc::InvokeBody::Json(json!({"root":root,"path":path})),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let guard=tauri::async_runtime::block_on(lifecycle.lock_owned());assert!(invoke("fields.json").is_err());drop(guard);
        let state=app.state::<InterviewState>();state.session.store(true,Ordering::SeqCst);assert!(invoke("fields.json").is_err());state.session.store(false,Ordering::SeqCst);
        assert!(invoke("../fields.json").is_err());
        let preview=invoke("fields.json").unwrap().deserialize::<Value>().unwrap();
        assert_eq!(preview["path"],"fields.json");assert_eq!(preview["file_hash"].as_str().unwrap().len(),64);assert_eq!(preview["fields"].as_array().unwrap().len(),5);
        assert!(state.index.get().is_none(),"preview must not initialize or mutate the real profile database");assert!(state.active.is_empty());assert!(!state.session.load(Ordering::SeqCst));
    }
    #[test]
    fn local_search_returns_both_conflicting_sources_without_model_or_fact_mutation() {
        let state=InterviewState::default();let root=std::env::temp_dir().join(format!("interview-conflicts-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(root.join("source")).unwrap();
        std::fs::write(root.join("source/a.md"),"本科毕业时间：2027年7月。").unwrap();std::fs::write(root.join("source/b.md"),"本科毕业时间：2026年7月。").unwrap();
        let index=tauri::async_runtime::block_on(Index::open(&root.join("synthetic.sqlite"))).unwrap();
        let selected=["a.md","b.md"].into_iter().map(|path|Selection {path:path.into(),kind:"personal".into(),allow_external:false,structured:None}).collect();
        tauri::async_runtime::block_on(index.import(root.join("source").to_str().unwrap(),selected,"unused")).unwrap();assert!(state.index.set(index).is_ok());let lifecycle=state.lifecycle.clone();
        let app=tauri::test::mock_builder().manage(state).invoke_handler(tauri::generate_handler![rag_search]).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let webview=tauri::WebviewWindowBuilder::new(&app,"test",Default::default()).build().unwrap();
        let invoke=||tauri::test::get_ipc_response(&webview,tauri::webview::InvokeRequest {cmd:"rag_search".into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(json!({"question":"本科毕业时间"})),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let guard=tauri::async_runtime::block_on(lifecycle.lock_owned());assert!(invoke().is_err());drop(guard);
        let review=invoke().unwrap().deserialize::<Value>().unwrap();assert_eq!(review["evidence"].as_array().unwrap().len(),2);assert_eq!(review["conflicts"].as_array().unwrap().len(),1);
        let state=app.state::<InterviewState>();assert!(!state.session.load(Ordering::SeqCst));assert!(state.active.is_empty());assert_eq!(tauri::async_runtime::block_on(state.index.get().unwrap().fact_card()).unwrap().version,0);
    }
    #[test]
    fn fact_commands_require_confirmation_and_cannot_be_bypassed_by_config() {
        let state=InterviewState::default();let root=std::env::temp_dir().join(format!("interview-facts-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let index=tauri::async_runtime::block_on(Index::open(&root.join("synthetic.sqlite"))).unwrap();
        tauri::async_runtime::block_on(index.set("aliases","original alias")).unwrap();assert!(state.index.set(index).is_ok());let lifecycle=state.lifecycle.clone();
        let app=tauri::test::mock_builder().manage(state).invoke_handler(tauri::generate_handler![profile_facts_correct,profile_facts_current,profile_facts_history,interview_config])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let webview=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let invoke=|cmd:&str,body:Value|tauri::test::get_ipc_response(&webview,tauri::webview::InvokeRequest {cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let update=json!({"update":{"expected_version":0,"text":"synthetic checked fact","source_note":"Synthetic self attestation","sources":[],"reason":"Synthetic correction","confirmed":true}});
        let guard=tauri::async_runtime::block_on(lifecycle.lock_owned());assert!(invoke("profile_facts_correct",update.clone()).is_err());drop(guard);
        let state=app.state::<InterviewState>();state.session.store(true,Ordering::SeqCst);
        assert!(invoke("profile_facts_correct",update.clone()).is_err());assert!(invoke("profile_facts_current",json!({})).is_ok());
        state.session.store(false,Ordering::SeqCst);
        let mut unconfirmed=update.clone();unconfirmed["update"]["confirmed"]=json!(false);assert!(invoke("profile_facts_correct",unconfirmed).is_err());
        let corrected=invoke("profile_facts_correct",update.clone()).unwrap().deserialize::<Value>().unwrap();assert_eq!(corrected["version"],1);
        assert!(invoke("profile_facts_correct",update).is_err());
        assert!(invoke("interview_config",json!({"facts":"bypass attempt","aliases":"should not change","key":"synthetic-key-placeholder"})).is_err());
        assert_eq!(tauri::async_runtime::block_on(state.index.get().unwrap().setting("aliases")).unwrap(),"original alias");assert!(tauri::async_runtime::block_on(state.key.lock()).is_none());
        assert!(invoke("interview_config",json!({"aliases":"updated alias","key":null})).is_ok());
        let synthetic=format!("sk-{}","synthetic01234567".repeat(2));
        assert!(invoke("interview_config",json!({"aliases":format!("值为{synthetic}"),"key":null})).is_err());
        assert_eq!(tauri::async_runtime::block_on(state.index.get().unwrap().setting("aliases")).unwrap(),"updated alias");
        let checked=uuid::Uuid::new_v4().to_string();state.connection.begin(&checked,"deepseek-flash").unwrap();
        state.connection.finish(&checked,interview_core::ProbeOutcome{success:true,message:"ok".into(),elapsed_ms:1,attempts:1,total_tokens:None}).unwrap();
        assert!(invoke("interview_config",json!({"aliases":"updated alias","key":"synthetic-replacement-key"})).is_ok());
        assert_eq!(state.connection.snapshot().status,crate::interview_connection::CheckStatus::Untested);
        let history=invoke("profile_facts_history",json!({"before":null})).unwrap().deserialize::<Value>().unwrap();assert_eq!(history["versions"].as_array().unwrap().len(),2);assert_eq!(history["versions"][0]["text"],"synthetic checked fact");
    }
    #[test]
    fn document_review_ipc_checks_identity_and_session_without_touching_real_profile() {
        let state=InterviewState::default();
        let root=std::env::temp_dir().join(format!("interview-document-ipc-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();std::fs::create_dir(root.join("source")).unwrap();
        std::fs::write(root.join("source/doc.md"),"synthetic review snapshot").unwrap();
        let index=tauri::async_runtime::block_on(Index::open(&root.join("synthetic.sqlite"))).unwrap();
        tauri::async_runtime::block_on(index.import(root.join("source").to_str().unwrap(),vec![interview_core::Selection {path:"doc.md".into(),kind:"technical".into(),allow_external:false,structured:None}],"unused")).unwrap();
        let doc=tauri::async_runtime::block_on(index.documents()).unwrap().remove(0);
        assert!(state.index.set(index).is_ok());let lifecycle=state.lifecycle.clone();
        let app=tauri::test::mock_builder().manage(state).invoke_handler(tauri::generate_handler![profile_document_update,profile_document_segments])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let webview=tauri::WebviewWindowBuilder::new(&app,"test",Default::default()).build().unwrap();
        let invoke=|cmd:&str,body:Value|tauri::test::get_ipc_response(&webview,tauri::webview::InvokeRequest {
            cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),
            body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string(),
        });
        let update=json!({"update":{"path":doc.path,"expected_root":doc.root,"expected_hash":doc.hash,"confirmed":true,"valid_from":null,"valid_until":"2026-12-31"}});
        let preview=json!({"path":doc.path,"expectedRoot":doc.root,"expectedHash":doc.hash});
        let guard=tauri::async_runtime::block_on(lifecycle.lock_owned());
        assert!(invoke("profile_document_update",update.clone()).is_err());drop(guard);
        let state=app.state::<InterviewState>();state.session.store(true,Ordering::SeqCst);
        assert!(invoke("profile_document_update",update.clone()).is_err());
        assert!(invoke("profile_document_segments",preview.clone()).is_ok());
        state.session.store(false,Ordering::SeqCst);
        let mut stale=update.clone();stale["update"]["expected_hash"]=json!("stale");
        assert!(invoke("profile_document_update",stale).is_err());
        assert!(invoke("profile_document_update",update).is_ok());
        let docs=tauri::async_runtime::block_on(state.index.get().unwrap().documents()).unwrap();
        assert_eq!(docs[0].metadata.confirmation,interview_core::Confirmation::Confirmed);
        let segments=invoke("profile_document_segments",preview.clone()).unwrap().deserialize::<Value>().unwrap();
        assert!(segments[0]["text"].as_str().unwrap().contains("synthetic review snapshot"));
        let mut stale=preview;stale["expectedRoot"]=json!("/stale/root");
        assert!(invoke("profile_document_segments",stale).is_err());
    }
    #[test]
    fn cancel_uses_real_ipc_and_remains_available_while_lifecycle_is_locked() {
        let state=InterviewState::default();
        let control=interview_core::ImportControl::default();
        *state.imports.lock().unwrap()=Some(ProfileImportJob {control:control.clone(),latest:ProfileImportEvent {
            import_id:"synthetic-import-1".into(),phase:"parsing".into(),completed:0,total:1,path:None,report:None,message:None,
        }});
        let _lifecycle=tauri::async_runtime::block_on(state.lifecycle.clone().lock_owned());
        let app=tauri::test::mock_builder().manage(state)
            .invoke_handler(tauri::generate_handler![profile_import_cancel,profile_import_status])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let webview=tauri::WebviewWindowBuilder::new(&app,"test",Default::default()).build().unwrap();
        let invoke=|cmd:&str,body:Value| tauri::test::get_ipc_response(&webview,tauri::webview::InvokeRequest {
            cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),
            body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string(),
        });
        assert!(invoke("profile_import_cancel",json!({"importId":"stale-job"})).is_err());
        assert!(control.check().is_ok());
        let result=invoke("profile_import_cancel",json!({"importId":"synthetic-import-1"})).unwrap().deserialize::<Value>().unwrap();
        assert_eq!(result,"requested");assert_eq!(control.check().unwrap_err(),interview_core::IMPORT_CANCELLED);
        let status=invoke("profile_import_status",json!({})).unwrap().deserialize::<Value>().unwrap();
        assert_eq!(status["phase"],"cancelling");assert_eq!(status["import_id"],"synthetic-import-1");
    }
    #[test]
    fn index_clear_ipc_refuses_live_session_and_busy_lifecycle_then_handles_empty_index() {
        let state=InterviewState::default();
        let root=std::env::temp_dir().join(format!("interview-index-ipc-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let index=tauri::async_runtime::block_on(Index::open(&root.join("synthetic.sqlite"))).unwrap();
        assert!(state.index.set(index).is_ok());
        let lifecycle=state.lifecycle.clone();
        let app=tauri::test::mock_builder().manage(state).invoke_handler(tauri::generate_handler![profile_index_clear])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let webview=tauri::WebviewWindowBuilder::new(&app,"test",Default::default()).build().unwrap();
        let invoke=||tauri::test::get_ipc_response(&webview,tauri::webview::InvokeRequest {
            cmd:"profile_index_clear".into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),
            body:tauri::ipc::InvokeBody::Json(json!({})),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string(),
        });
        let guard=tauri::async_runtime::block_on(lifecycle.lock_owned());
        assert!(invoke().is_err());drop(guard);
        let state=app.state::<InterviewState>();state.session.store(true,Ordering::SeqCst);
        assert!(invoke().is_err());state.session.store(false,Ordering::SeqCst);
        let report=invoke().unwrap().deserialize::<Value>().unwrap();
        assert!(report["archive_path"].is_null());assert_eq!(report["version"],0);
    }
}
#[tauri::command]
pub async fn rag_search<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,question:String)->Result<interview_core::RetrievalReview,String> {
    if question.chars().count()>1500 {return Err("问题超过 1500 字符".into());}
    let _guard=state.lifecycle.try_lock().map_err(|_|"请先完成或取消当前资料操作")?;
    let index=state.index(&app).await?;let aliases=index.request_aliases().await?;
    let facts=index.fact_card().await?;
    index.search_review(&question,if aliases.is_empty(){DEFAULT_ALIASES}else{&aliases},Some(&facts)).await
}
#[tauri::command]
pub async fn answer_start(app:tauri::AppHandle,state:State<'_,InterviewState>,question_id:String,revision:u32,question:String,history:Vec<Turn>,dialogue:Option<Vec<interview_core::SpokenTurn>>,answer_first:Option<bool>,meeting_mode:Option<String>)->Result<(),String> {
    start_answer(app,&state,question_id,revision,question,history,dialogue,None,answer_first.unwrap_or(false),meeting_mode).await
}
#[tauri::command]
pub async fn interview_screenshot_answer_start(app:tauri::AppHandle,window:tauri::WebviewWindow,state:State<'_,InterviewState>,question_id:String,revision:u32,question:String,history:Vec<Turn>,dialogue:Option<Vec<interview_core::SpokenTurn>>,display_id:Option<u32>,meeting_mode:Option<String>)->Result<(),String> {
    if window.label()!="main" {return Err("请在主窗口点击截屏并回答".into());}
    start_answer(app,&state,question_id,revision,question,history,dialogue,Some(display_id),true,meeting_mode).await
}
async fn start_answer(app:tauri::AppHandle,state:&InterviewState,question_id:String,revision:u32,question:String,history:Vec<Turn>,dialogue:Option<Vec<interview_core::SpokenTurn>>,screenshot:Option<Option<u32>>,answer_first:bool,meeting_mode:Option<String>)->Result<(),String> {
    let meeting_instruction=match meeting_mode.as_deref().unwrap_or("pair") {
        "pair"=>"",
        "panel"=>"多对一面试。多位提问者可能连续补充条件，围绕当前完整问题回应，不推断评委身份。",
        "group"|"groups"=>"群面。当前问题已交给使用者或由其手动选中。区分他人的例子与使用者实际经历。对端通道混有多人，不能据此认定具体说话人。",
        "mixed"=>"复合会议。围绕当前交给使用者的事项给可直接说出口的回应。区分讨论、建议、已定决议与待确认事项，不代替他人作承诺，不猜负责人或期限。",
        _=>return Err("未知参会模式".into()),
    };
    crate::interview_exit::ensure_not_exiting(&app)?;
    if question_id.len()>100 || question.trim().is_empty() || question.chars().count()>1500 {return Err("问题或问题 ID 无效".into());}
    let _guard=state.lifecycle.lock().await;
    crate::interview_exit::ensure_not_exiting(&app)?;
    let index=state.index(&app).await?.clone();
    let providers=if answer_first {index.provider_config().await?} else {interview_core::ProviderConfig::initial(index.request_config().await?)};
    let ready=if screenshot.is_some(){
        if !providers.vision.enabled || (!providers.vision.accepts_empty_key() && state.provider_key(&providers.vision).await.is_empty()) {
            return Err("请先配置并启用支持图片的视觉模型及 API Key，尚未截屏或发送".into());
        }
        vec!["vision".to_string()]
    }else if !answer_first {if state.api_key().await.is_empty(){vec![]}else{vec!["answer-1".into()]}}else{state.ready_profiles(&providers).await};
    if ready.is_empty(){return Err("尚未配置启用接口的 API Key；可使用本地检索".into());}
    let lease=state.active.reserve(&question_id,revision,&ready)?;
    let profile=providers.answers.iter().chain(std::iter::once(&providers.vision)).find(|p|p.id==lease.profile).unwrap().clone();
    let key=if answer_first{state.provider_key(&profile).await}else{state.api_key().await};
    // Acquire before releasing lifecycle, so clear/exit cannot overtake a
    // spawned request that has not acquired its network guard yet.
    let network=state.network.clone().read_owned().await;
    state.session.store(true,Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        let cancel=lease.cancel.clone();
        let emit=|body:Value| {if !cancel.is_cancelled(){let _=app.emit("interview-answer",json!({"question_id":question_id,"revision":revision,"event":body}));}};
        let mut terminal=None;
        let result=async {
            // Capture immediately after the explicit click, before retrieval.
            // Only this local future owns image bytes; no event/history contains them.
            let screenshot=if let Some(display_id)=screenshot {Some(crate::interview_screenshot::capture(display_id,cancel.clone()).await?)}else{None};
            if cancel.is_cancelled(){return Err("已取消".to_string());}
            let started=std::time::Instant::now();
            let aliases=index.request_aliases().await?;
            let facts=index.request_fact_card().await?;
            let review=index.search_review(&question,if aliases.is_empty(){DEFAULT_ALIASES}else{&aliases},Some(&facts)).await?;
            let evidence=review.evidence;
            emit(json!({"type":"sources","evidence":evidence,"conflicts":review.conflicts,"retrieval_ms":started.elapsed().as_millis(),"index_version":index.setting("version").await?,"fact_version":facts.version,"profile_id":profile.id}));
            let prefs=index.request_preferences().await?;
            let history=index.request_history(&history).await?;
            let mut messages=interview_core::messages_with_preferences(&question,&facts,&history,&evidence,&dialogue.unwrap_or_default(),&prefs)?;
            if let Some(role)=index.background_role().await? {
                let mut input:Value=serde_json::from_str(messages[1]["content"].as_str().ok_or("缺少问题上下文")?).map_err(|e|e.to_string())?;
                input["interview_role"]=json!(role);messages[1]["content"]=json!(input.to_string());
            }
            if answer_first {
                let system=messages[0]["content"].as_str().unwrap_or_default();
                messages[0]["content"]=json!(format!("{system}\n{}",interview_core::ANSWER_FIRST_RULES));
            }
            if !meeting_instruction.is_empty() {
                let system=messages[0]["content"].as_str().unwrap_or_default();
                messages[0]["content"]=json!(format!("{system}\n{meeting_instruction}"));
            }
            let messages=if let Some(png)=screenshot {interview_core::attach_screenshot(messages,&png)?}else{messages};
            interview_core::stream_answer_with_provider(&key,messages,cancel.clone(),&profile,|event|{
                let body=serde_json::to_value(event).unwrap_or_default();
                if body["type"]=="done" {terminal=Some(body);}else{emit(body);}
            }).await
        }.await;
        // The next queued question can use this profile as soon as it sees done.
        drop(lease);drop(network);
        if let Err(message)=result {emit(json!({"type":"error","message":interview_core::redact_credentials(&message,Some(&key))}));}
        else if let Some(body)=terminal{emit(body);}
    });
    Ok(())
}
#[tauri::command]
pub async fn answer_cancel(state:State<'_,InterviewState>)->Result<(),String> {state.cancel_answers().await}
#[tauri::command]
pub async fn profile_source_exclude<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,app:tauri::AppHandle<R>,state:State<'_,InterviewState>,update:interview_core::source_policy::SourceExclusion)->Result<(),String> {
    if window.label()!="main" {return Err("请在主窗口调整资料范围".into());}
    state.background.cancel();
    state.cancel().await;
    let _guard=state.lifecycle.lock().await;
    crate::interview_exit::ensure_not_exiting(&app)?;
    // No new request can start across the withdrawal. Cancel old work and wait
    // for its network lease before acknowledging the new source policy.
    state.cancel().await;
    let _network=tokio::time::timeout(std::time::Duration::from_secs(35),state.network.write()).await.map_err(|_|"旧请求仍在结束，尚未修改来源范围，请重试")?;
    state.index(&app).await?.exclude_source(update).await
}

#[cfg(test)]
mod source_withdrawal_tests {
    use super::*;
    #[test]
    fn applied_setting_review_ipc_is_main_only_and_preserves_session_and_expected_values(){
        let root=std::env::temp_dir().join(format!("tingxian-preference-review-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let index=tauri::async_runtime::block_on(Index::open(&root.join("synthetic.sqlite"))).unwrap();
        tauri::async_runtime::block_on(async {
            index.set_preferences(&interview_core::InterviewPreferences{context:"需核对的虚构背景".into(),..Default::default()}).await.unwrap();
            let binding=json!({"format":"preference-sources-v1","fields":{"context":{"value_hash":interview_core::digest("需核对的虚构背景".as_bytes()),"snapshot":"missing-source"}},"snapshots":{}});
            index.set("interview_preference_sources",&binding.to_string()).await.unwrap();
        });
        let state=InterviewState::default();assert!(state.index.set(index).is_ok());
        let app=tauri::test::mock_builder().manage(state).manage(crate::interview_exit::ExitState::default())
            .invoke_handler(tauri::generate_handler![interview_preferences_state,interview_preferences_manual])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let display=tauri::WebviewWindowBuilder::new(&app,"display",Default::default()).build().unwrap();
        let invoke=|window:&tauri::WebviewWindow<tauri::test::MockRuntime>,cmd:&str,body:Value|tauri::test::get_ipc_response(window,tauri::webview::InvokeRequest {cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let body=json!({"fields":[{"field":"context","expected":"需核对的虚构背景"}]});
        assert!(invoke(&display,"interview_preferences_state",json!({})).is_err());assert!(invoke(&display,"interview_preferences_manual",body.clone()).is_err());
        let view=invoke(&main,"interview_preferences_state",json!({})).unwrap().deserialize::<Value>().unwrap();assert_eq!(view["blocked_fields"],json!(["context"]));assert_eq!(view["preferences"]["context"],"需核对的虚构背景");
        let state=app.state::<InterviewState>();state.session.store(true,Ordering::SeqCst);assert!(invoke(&main,"interview_preferences_manual",body.clone()).is_err());state.session.store(false,Ordering::SeqCst);
        let guard=tauri::async_runtime::block_on(state.lifecycle.clone().lock_owned());assert!(invoke(&main,"interview_preferences_manual",body.clone()).is_err());drop(guard);
        assert!(invoke(&main,"interview_preferences_manual",json!({"fields":[{"field":"context","expected":"旧值"}]})).is_err());
        let result=invoke(&main,"interview_preferences_manual",body).unwrap().deserialize::<Value>().unwrap();assert_eq!(result["blocked_fields"],json!([]));assert_eq!(result["preferences"]["context"],"需核对的虚构背景");
    }
    #[test]
    fn alias_ipc_checks_window_session_lifecycle_and_expected_editor_value() {
        let root=std::env::temp_dir().join(format!("tingxian-alias-review-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let index=tauri::async_runtime::block_on(Index::open(&root.join("synthetic.sqlite"))).unwrap();
        tauri::async_runtime::block_on(async {
            index.set("aliases","企业,契约").await.unwrap();
            index.set("alias_sources",&json!({"format":"alias-sources-v1","lines":{(interview_core::digest("企业,契约".as_bytes())):"missing"},"snapshots":{}}).to_string()).await.unwrap();
        });
        let state=InterviewState::default();assert!(state.index.set(index).is_ok());
        let app=tauri::test::mock_builder().manage(state).manage(crate::interview_exit::ExitState::default())
            .invoke_handler(tauri::generate_handler![interview_aliases_state,interview_aliases_save,interview_aliases_manual,interview_config])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let display=tauri::WebviewWindowBuilder::new(&app,"display",Default::default()).build().unwrap();
        let invoke=|window:&tauri::WebviewWindow<tauri::test::MockRuntime>,cmd:&str,body:Value|tauri::test::get_ipc_response(window,tauri::webview::InvokeRequest {cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let save=json!({"aliases":"企业,契约\n本人,manual","expected":"企业,契约"});let manual=json!({"lines":["企业,契约"],"expected":"企业,契约"});
        for (cmd,body) in [("interview_aliases_state",json!({})),("interview_aliases_save",save.clone()),("interview_aliases_manual",manual.clone()),("interview_config",json!({"aliases":"bypass"}))] {assert!(invoke(&display,cmd,body).is_err());}
        let view=invoke(&main,"interview_aliases_state",json!({})).unwrap().deserialize::<Value>().unwrap();assert_eq!(view["blocked_lines"],json!(["企业,契约"]));
        let state=app.state::<InterviewState>();
        state.session.store(true,Ordering::SeqCst);
        assert!(invoke(&main,"interview_aliases_save",save.clone()).is_err());assert!(invoke(&main,"interview_aliases_manual",manual.clone()).is_err());
        state.session.store(false,Ordering::SeqCst);
        let guard=tauri::async_runtime::block_on(state.lifecycle.clone().lock_owned());assert!(invoke(&main,"interview_aliases_save",save.clone()).is_err());assert!(invoke(&main,"interview_aliases_manual",manual.clone()).is_err());drop(guard);
        assert!(invoke(&main,"interview_aliases_save",json!({"aliases":"stale","expected":"old"})).is_err());
        assert!(invoke(&main,"interview_config",json!({"key":null})).is_ok());
        assert_eq!(invoke(&main,"interview_aliases_state",json!({})).unwrap().deserialize::<Value>().unwrap()["aliases"],"企业,契约");
        assert!(invoke(&main,"interview_aliases_save",save).is_ok());assert!(invoke(&main,"interview_aliases_manual",manual).is_err());
        assert!(invoke(&main,"interview_aliases_manual",json!({"lines":["企业,契约"],"expected":"企业,契约\n本人,manual"})).is_ok());
        assert_eq!(invoke(&main,"interview_aliases_state",json!({})).unwrap().deserialize::<Value>().unwrap()["blocked_lines"],json!([]));
    }
    #[tokio::test]
    async fn source_withdrawal_interrupts_background_before_waiting_for_its_lifecycle_lock(){
        let root=std::env::temp_dir().join(format!("tingxian-background-withdrawal-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let index=interview_core::Index::open(&root.join("synthetic.sqlite")).await.unwrap();
        sqlx::query("INSERT INTO documents VALUES('fictional.txt','synthetic-hash','personal',1)").execute(&index.pool).await.unwrap();
        let state=InterviewState::default();assert!(state.index.set(index).is_ok());
        let token=state.background.fixture_running();
        let app=tauri::test::mock_builder().manage(state).manage(crate::interview_exit::ExitState::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let window=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let state=app.state::<InterviewState>();
        let lifecycle=state.lifecycle.clone().lock_owned().await;
        let network=state.network.clone().read_owned().await;
        let withdrawal=crate::interview::profile_source_exclude(window,app.handle().clone(),app.state(),interview_core::source_policy::SourceExclusion{path:"fictional.txt".into(),expected_root:String::new(),expected_hash:"synthetic-hash".into(),excluded:true});
        let finish_worker=async {
            tokio::time::timeout(std::time::Duration::from_secs(3),token.cancelled()).await.expect("withdrawal waited for background without cancelling it");
            assert!(!state.index.get().unwrap().documents().await.unwrap()[0].excluded,"source policy changed before old worker exited");
            drop(network);drop(lifecycle);
        };
        let (result,())=tokio::join!(withdrawal,finish_worker);result.unwrap();
        assert!(state.index.get().unwrap().documents().await.unwrap()[0].excluded);
    }
    #[tokio::test]
    async fn interview_record_revision_is_main_only_session_locked_and_provenance_bound() {
        let root=std::env::temp_dir().join(format!("tingxian-assertion-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let index=Index::open(&root.join("synthetic.sqlite")).await.unwrap();
        sqlx::query("INSERT INTO documents VALUES('fictional.md','hash-A','personal',0)").execute(&index.pool).await.unwrap();
        sqlx::query("INSERT INTO chunks VALUES('C1','fictional.md','行1','虚构原文')").execute(&index.pool).await.unwrap();
        let state=InterviewState::default();assert!(state.index.set(index).is_ok());
        let app=tauri::test::mock_builder().manage(state).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let display=tauri::WebviewWindowBuilder::new(&app,"display",Default::default()).build().unwrap();
        let update=||interview_core::assertions::AssertionUpdate {source:interview_core::FactSourceRef{source_id:"C1".into(),path:"fictional.md".into(),root:"".into(),hash:"hash-A".into()},expected_revision:0,claim:"虚构计划尚未完成".into(),activity:interview_core::assertions::Activity::Planned,reason:"本人确认计划".into(),confirmed:true};
        assert!(profile_record_revise(display,app.handle().clone(),app.state(),update()).await.is_err());
        let state=app.state::<InterviewState>();state.session.store(true,Ordering::SeqCst);
        assert!(profile_record_revise(main.clone(),app.handle().clone(),app.state(),update()).await.is_err());state.session.store(false,Ordering::SeqCst);
        let lease=state.network.read().await;
        assert!(profile_record_revise(main.clone(),app.handle().clone(),app.state(),update()).await.is_err());drop(lease);
        let revised=profile_record_revise(main.clone(),app.handle().clone(),app.state(),update()).await.unwrap();assert_eq!(revised.source.text,"虚构原文");
        assert!(profile_record_revise(main.clone(),app.handle().clone(),app.state(),update()).await.is_err());
        let records=profile_document_records(main,app.handle().clone(),app.state(),"fictional.md".into(),"".into(),"hash-A".into()).await.unwrap();assert_eq!(records[0].user_assertion.as_ref().unwrap().revision,1);
    }
    #[tokio::test]
    async fn interview_source_withdrawal_cancels_inflight_and_waits_before_acknowledging() {
        let root=std::env::temp_dir().join(format!("tingxian-source-policy-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let index=Index::open(&root.join("synthetic.sqlite")).await.unwrap();
        sqlx::query("INSERT INTO documents VALUES('synthetic.txt','synthetic-hash','personal',1)").execute(&index.pool).await.unwrap();
        let state=InterviewState::default();assert!(state.index.set(index).is_ok());
        let app=tauri::test::mock_builder().manage(state).manage(crate::interview_exit::ExitState::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let display=tauri::WebviewWindowBuilder::new(&app,"display",Default::default()).build().unwrap();
        let update=||interview_core::source_policy::SourceExclusion{path:"synthetic.txt".into(),expected_root:String::new(),expected_hash:"synthetic-hash".into(),excluded:true};
        assert!(profile_source_exclude(display,app.handle().clone(),app.state(),update()).await.is_err());
        let state=app.state::<InterviewState>();state.session.store(true,Ordering::SeqCst);
        let lease=state.active.reserve("synthetic-q",1,&["answer-1".into()]).unwrap();
        let network=state.network.clone().read_owned().await;
        let withdrawal=profile_source_exclude(main,app.handle().clone(),app.state(),update());
        let releasing=async {
            tokio::time::timeout(std::time::Duration::from_secs(3),lease.cancel.cancelled()).await.unwrap();
            assert!(state.lifecycle.try_lock().is_err(),"new starts stay behind withdrawal");
            let count:i64=sqlx::query_scalar("SELECT count(*) FROM background_library WHERE key GLOB 'excluded/*'").fetch_one(&state.index.get().unwrap().pool).await.unwrap();
            assert_eq!(count,0,"do not acknowledge while old network worker still runs");
            drop(lease);drop(network);
        };
        let (result,())=tokio::join!(withdrawal,releasing);result.unwrap();
        assert!(state.index.get().unwrap().documents().await.unwrap()[0].excluded);
        assert!(state.session.load(Ordering::SeqCst),"withdrawing a source must not stop recording/session");
        assert!(state.active.is_empty());
    }
}
#[tauri::command]
pub async fn session_clear(state:State<'_,InterviewState>)->Result<(),String> {
    let _guard=state.lifecycle.lock().await;state.cancel().await;let _network=state.network.write().await;state.session.store(false,Ordering::SeqCst);Ok(())
}

#[tauri::command]
pub async fn interview_history_clear(app:tauri::AppHandle,state:State<'_,InterviewState>,snapshot:Value)->Result<crate::interview_history::ClearReport,String> {
    let _guard=state.lifecycle.lock().await;
    state.cancel().await;
    let _network=state.network.write().await;
    let _capture=crate::audio::common::acquire_engine_lifecycle_lock().await;
    let recording=crate::audio::recording_commands::get_recording_state().await;
    if recording.get("is_recording").and_then(Value::as_bool)==Some(true) {
        return Err("请先停止当前采集，等待转写处理完成后再清理".into());
    }
    let prefs=crate::audio::recording_preferences::load_recording_preferences(&app).await.map_err(|e|e.to_string())?;
    let database=app.state::<crate::state::AppState>();
    let result=crate::interview_history::clear_history_with(database.db_manager.pool(),&prefs.save_folder,snapshot,crate::interview_history::move_to_trash).await?;
    state.session.store(false,Ordering::SeqCst);
    SYSTEM_LEVEL.store(0,Ordering::Relaxed); MICROPHONE_LEVEL.store(0,Ordering::Relaxed);
    Ok(result)
}

#[tauri::command]
pub async fn interview_audio_start(app:tauri::AppHandle,state:State<'_,InterviewState>,record_microphone:bool,cloud_asr_allowed:Option<bool>)->Result<crate::audio::recording_commands::MicrophoneStatus,String> {
    crate::interview_exit::ensure_not_exiting(&app)?;
    let _guard=state.lifecycle.lock().await;
    crate::interview_exit::ensure_not_exiting(&app)?;
    if crate::audio::recording_commands::is_recording().await{return Err("采集已经开始".into());}
    let cloud=crate::interview_cloud_asr::CloudStart::prepare(&app,&state,cloud_asr_allowed.unwrap_or(false),record_microphone).await?;
    let preferences=state.index(&app).await?.preferences().await?;
    if cloud.is_none(){
        let model=crate::tingxian_audio::selected_model(&app).await?;
        crate::tingxian_audio::save_model(&app,&model).await?;
        if model.contains(".en") && preferences.transcription_language!="en" {return Err("英文专用模型仅支持英文转写".into());}
    }
    let mut prefs=crate::audio::recording_preferences::load_recording_preferences(&app).await.map_err(|e|e.to_string())?;
    prefs.auto_save=false;
    crate::audio::recording_preferences::save_recording_preferences(&app,&prefs).await.map_err(|e|e.to_string())?;
    crate::set_language_preference(preferences.transcription_language).await?;
    let mic = if record_microphone { prefs.preferred_mic_device } else { Some("__interview_disabled__".into()) };
    crate::audio::recording_commands::start_recording_with_asr(app.clone(),mic,prefs.preferred_system_device,Some("Interview practice".into()),cloud).await?;
    let status=crate::audio::recording_commands::microphone_status();
    // The generic recorder permits microphone-only capture after a system
    // stream error. An interview cannot claim to hear the remote participant.
    if !status.health.as_ref().map_or(false,|h|h.active && h.system_started) {
        crate::audio::recording_commands::stop_recording(app,crate::audio::recording_commands::RecordingArgs {save_path:String::new()}).await
            .map_err(|e|format!("系统音频未成功启动，停止采集也失败：{e}；请重启应用"))?;
        return Err("系统音频未成功启动，采集已停止。请检查系统音频权限和输出设备后重新开始".into());
    }
    Ok(status)
}

#[tauri::command]
pub async fn interview_microphone_set(app:tauri::AppHandle,state:State<'_,InterviewState>,enabled:bool)->Result<crate::audio::recording_commands::MicrophoneStatus,String> {
    let _guard = state.lifecycle.lock().await;
    MICROPHONE_LEVEL.store(0,Ordering::Relaxed);
    crate::audio::recording_commands::set_microphone_enabled(&app,enabled).await
}

#[tauri::command]
pub async fn interview_session_save<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,snapshot:crate::interview_sessions::Snapshot,confirmed:bool)->Result<crate::interview_sessions::SavedSession,String> {
    let _guard=state.lifecycle.lock().await;
    crate::interview_sessions::save_with_key(app.state::<crate::state::AppState>().db_manager.pool(),snapshot,confirmed,&state.api_key().await).await
}
#[tauri::command]
pub async fn interview_session_export(state:State<'_,InterviewState>,snapshot:Value)->Result<crate::interview_sessions::ExportReport,String> {
    crate::interview_sessions::export(snapshot,&state.api_key().await)
}
#[tauri::command]
pub async fn interview_session_list<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,before:Option<i64>)->Result<crate::interview_sessions::SessionPage,String> {
    let _guard=state.lifecycle.lock().await;
    crate::interview_sessions::list(app.state::<crate::state::AppState>().db_manager.pool(),before).await
}
#[tauri::command]
pub async fn tingxian_session_preview<R:tauri::Runtime>(app:tauri::AppHandle<R>,id:i64)->Result<crate::interview_sessions::Snapshot,String> {
    crate::interview_sessions::load(app.state::<crate::state::AppState>().db_manager.pool(),id).await
}
#[tauri::command]
pub async fn interview_session_load<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,id:i64)->Result<crate::interview_sessions::Snapshot,String> {
    let _guard=state.lifecycle.lock().await;
    if crate::audio::recording_commands::is_recording().await || state.session.load(Ordering::SeqCst) {return Err("请先结束当前会话再恢复保存记录".into());}
    crate::interview_sessions::load(app.state::<crate::state::AppState>().db_manager.pool(),id).await
}

#[cfg(test)]
mod interview_session_ipc_tests {
    use super::*;
    #[test]
    fn explicit_snapshot_ipc_roundtrip_and_active_session_guard() {
        let root=std::env::temp_dir().join(format!("meetily-session-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let db=tauri::async_runtime::block_on(crate::database::manager::DatabaseManager::new(root.join("test.sqlite").to_str().unwrap(),root.join("no-legacy.sqlite").to_str().unwrap())).unwrap();
        let app=tauri::test::mock_builder().manage(InterviewState::default()).manage(crate::state::AppState{db_manager:db})
          .invoke_handler(tauri::generate_handler![interview_session_save,interview_session_export,interview_session_list,interview_session_load,interview_residue_status,interview_residue_clear])
          .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let window=tauri::WebviewWindowBuilder::new(&app,"test",Default::default()).build().unwrap();
        let invoke=|cmd:&str,body:Value|tauri::test::get_ipc_response(&window,tauri::webview::InvokeRequest {cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let mut snapshot=json!({"version":1,"history":[{"id":"test-r1","role":"remote","question":"synthetic question","suggestion":"synthetic answer","confirmed_answer":null,"status":"done","sources":[],"time":"00:01","timing":""}],"transcripts":[],"spoken_dialogue":[],"current":{"question":"","answer":""},"calls":1,"usage":"unknown"});
        snapshot["usage_ledger"]=json!({"version":1,"legacy_calls":0,"requests":[{"question_id":"test-r1","revision":0,"accepted":true,"attempts":2,"total_tokens":19,"state":"done"}]});
        assert!(invoke("interview_session_save",json!({"snapshot":snapshot,"confirmed":false})).is_err());
        let row=invoke("interview_session_save",json!({"snapshot":snapshot,"confirmed":true})).unwrap().deserialize::<Value>().unwrap();
        let page=invoke("interview_session_list",json!({"before":null})).unwrap().deserialize::<Value>().unwrap();assert_eq!(page["sessions"][0]["id"],row["id"]);
        let loaded=invoke("interview_session_load",json!({"id":row["id"]})).unwrap().deserialize::<Value>().unwrap();assert_eq!(loaded,snapshot);
        let known="synthetic-configured-token";*tauri::async_runtime::block_on(app.state::<InterviewState>().key.lock())=Some(known.into());
        let mut with_key=snapshot.clone();with_key["current"]["question"]=format!("凭证为{known}；null不等于0").into();
        let exported=invoke("interview_session_export",json!({"snapshot":with_key})).unwrap().deserialize::<Value>().unwrap();
        assert_eq!(exported["credentials_redacted"],true);assert!(!exported["content"].as_str().unwrap().contains(known));
        let saved=invoke("interview_session_save",json!({"snapshot":with_key,"confirmed":true})).unwrap().deserialize::<Value>().unwrap();
        let restored=invoke("interview_session_load",json!({"id":saved["id"]})).unwrap().deserialize::<Value>().unwrap();
        assert_eq!(serde_json::from_str::<Value>(exported["content"].as_str().unwrap()).unwrap(),restored);
        app.state::<InterviewState>().session.store(true,Ordering::SeqCst);
        assert!(invoke("interview_session_load",json!({"id":row["id"]})).is_err());
        assert_eq!(invoke("interview_residue_status",json!({})).unwrap().deserialize::<Value>().unwrap()["active"],true);
        assert!(invoke("interview_residue_clear",json!({"recovery":{}})).is_err(),"cleanup must not cancel an active interview");
        assert!(app.state::<InterviewState>().index.get().is_none(),"session recovery must not touch the profile or call AI");
        app.state::<InterviewState>().session.store(false,Ordering::SeqCst);
        tauri::async_runtime::block_on(app.state::<crate::state::AppState>().db_manager.pool().close());
    }
}

#[tauri::command]
pub async fn interview_residue_status<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>)->Result<Value,String> {
    let _guard=state.lifecycle.lock().await;
    if crate::audio::recording_commands::is_recording().await || state.session.load(Ordering::SeqCst) {return Ok(json!({"active":true}));}
    let prefs=crate::audio::recording_preferences::load_recording_preferences(&app).await.map_err(|e|e.to_string())?;
    let report=crate::interview_history::residue_status(app.state::<crate::state::AppState>().db_manager.pool(),&prefs.save_folder).await?;
    Ok(json!({"active":false,"residue":report}))
}
#[tauri::command]
pub async fn interview_residue_clear<R:tauri::Runtime>(app:tauri::AppHandle<R>,state:State<'_,InterviewState>,recovery:Value)->Result<crate::interview_history::ClearReport,String> {
    let _guard=state.lifecycle.lock().await;
    let _capture=crate::audio::common::acquire_engine_lifecycle_lock().await;
    if crate::audio::recording_commands::is_recording().await || state.session.load(Ordering::SeqCst) {return Err("当前会话仍在进行，未清理任何残留；请先结束会话".into());}
    let _network=state.network.write().await;
    let prefs=crate::audio::recording_preferences::load_recording_preferences(&app).await.map_err(|e|e.to_string())?;
    crate::interview_history::clear_temporary_history_with(app.state::<crate::state::AppState>().db_manager.pool(),&prefs.save_folder,json!({"recovery":recovery}),crate::interview_history::move_to_trash).await
}

#[cfg(test)]
mod preparation_ipc_tests {
    use super::*;
    use crate::interview_background;
    #[test]
    fn preparation_ipc_is_main_window_only_and_obeys_session_and_lifecycle_guards(){
        let dir=std::env::temp_dir().join(format!("tingxian-preparation-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&dir).unwrap();
        let index=tauri::async_runtime::block_on(interview_core::Index::open(&dir.join("synthetic.sqlite"))).unwrap();
        let state=InterviewState::default();assert!(state.index.set(index).is_ok());
        let app=tauri::test::mock_builder().manage(state).manage(crate::interview_exit::ExitState::default())
            .invoke_handler(tauri::generate_handler![interview_background::background_preparation_export,interview_background::background_preparation_import,interview_background::background_preparation_save,interview_background::background_preparation_view])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let display=tauri::WebviewWindowBuilder::new(&app,"display",Default::default()).build().unwrap();
        let invoke=|window:&tauri::WebviewWindow<tauri::test::MockRuntime>,cmd:&str,body:Value|tauri::test::get_ipc_response(window,tauri::webview::InvokeRequest {cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
        let commands=[("background_preparation_export",json!({})),("background_preparation_import",json!({})),("background_preparation_view",json!({})),("background_preparation_save",json!({"candidate":{},"version":"0","previousHash":null}))];
        for (cmd,body) in &commands{assert!(invoke(&display,cmd,body.clone()).is_err());}
        let state=app.state::<InterviewState>();state.session.store(true,Ordering::SeqCst);
        for (cmd,body) in &commands{assert!(invoke(&main,cmd,body.clone()).is_err());}state.session.store(false,Ordering::SeqCst);
        let lock=tauri::async_runtime::block_on(state.lifecycle.clone().lock_owned());for (cmd,body) in &commands{assert!(invoke(&main,cmd,body.clone()).is_err());}drop(lock);
        assert_eq!(invoke(&main,"background_preparation_view",json!({})).unwrap().deserialize::<Value>().unwrap(),Value::Null);
        assert!(invoke(&main,"background_preparation_save",commands[3].1.clone()).is_err());
    }
}
