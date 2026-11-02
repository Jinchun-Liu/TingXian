//! Interview assistance only: no tools, no audio upload, no permanent chat memory.
use interview_core::{Index, Selection, Turn, DEFAULT_ALIASES};
use serde_json::{json, Value};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tauri::{Emitter, Manager, State};
use tokio::sync::{Mutex, OnceCell};
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
    active: Mutex<Option<CancellationToken>>,
    pub(crate) lifecycle: Mutex<()>,
    network: Arc<Mutex<()>>,
    session: AtomicBool,
    key: Mutex<Option<String>>,
}
impl InterviewState {
    async fn index(&self, app:&tauri::AppHandle)->Result<&Index,String> {
        self.index.get_or_try_init(||async {
            let dir=app.path().app_data_dir().map_err(|e|e.to_string())?.join("interview");
            tokio::fs::create_dir_all(&dir).await.map_err(|e|e.to_string())?;
            Index::open(&dir.join("profile.sqlite")).await
        }).await
    }
    async fn api_key(&self)->String { self.key.lock().await.clone().unwrap_or_else(||std::env::var("DEEPSEEK_API_KEY").unwrap_or_default()) }
    async fn cancel(&self) { if let Some(token)=self.active.lock().await.take(){token.cancel();} }
}

#[tauri::command]
pub async fn interview_status(app:tauri::AppHandle,state:State<'_,InterviewState>)->Result<Value,String> {
    let index=state.index(&app).await?;
    let aliases=index.setting("aliases").await?;
    Ok(json!({"architecture":std::env::consts::ARCH,"os":std::env::consts::OS,
        "memory_bytes":sysinfo::System::new_all().total_memory(),
        "system_version":sysinfo::System::os_version(),"key_configured":!state.api_key().await.is_empty(),
        "model":"deepseek-flash","root":index.setting("root").await?,"suggested_root":dirs::home_dir().map(|p|p.join("Documents/InterviewProfile").to_string_lossy().into_owned()),"index_version":index.setting("version").await?,
        "session_active":state.session.load(Ordering::SeqCst),"aliases":if aliases.is_empty(){DEFAULT_ALIASES}else{&aliases},
        "facts":index.setting("facts").await?,"whisper_model":crate::whisper_engine::commands::whisper_get_current_model().await.unwrap_or(None),
        "audio_validation":"未验收；上游混合音频事件不提供可靠说话人身份"}))
}
#[tauri::command]
pub async fn interview_config(app:tauri::AppHandle,state:State<'_,InterviewState>,facts:String,aliases:String,key:Option<String>)->Result<(),String> {
    if facts.chars().count()>2200 || aliases.chars().count()>4000 {return Err("事实卡或别名过长".into());}
    let _guard=state.lifecycle.lock().await;
    if state.session.load(Ordering::SeqCst){return Err("请结束本次会话后修改资料配置".into());}
    let index=state.index(&app).await?;
    index.set("facts",&facts).await?;index.set("aliases",&aliases).await?;
    if let Some(key)=key {if !key.trim().is_empty(){*state.key.lock().await=Some(key.trim().into());}}
    Ok(())
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
pub async fn profile_import(app:tauri::AppHandle,state:State<'_,InterviewState>,root:String,selected:Vec<Selection>)->Result<interview_core::ImportReport,String> {
    let _guard=state.lifecycle.lock().await;
    if state.session.load(Ordering::SeqCst){return Err("会话期间索引已锁定，请先结束会话".into());}
    let python=std::env::var("INTERVIEW_PYTHON").unwrap_or_else(|_|"python3".into());
    state.index(&app).await?.import(&root,selected,&python).await
}
#[tauri::command]
pub async fn rag_search(app:tauri::AppHandle,state:State<'_,InterviewState>,question:String)->Result<Vec<interview_core::Evidence>,String> {
    if question.chars().count()>1500 {return Err("问题超过 1500 字符".into());}
    let index=state.index(&app).await?;let aliases=index.setting("aliases").await?;
    index.search(&question,if aliases.is_empty(){DEFAULT_ALIASES}else{&aliases}).await
}
#[tauri::command]
pub async fn answer_start(app:tauri::AppHandle,state:State<'_,InterviewState>,question_id:String,revision:u32,question:String,history:Vec<Turn>,dialogue:Option<Vec<interview_core::SpokenTurn>>)->Result<(),String> {
    if question_id.len()>100 || question.trim().is_empty() || question.chars().count()>1500 {return Err("问题或问题 ID 无效".into());}
    let _guard=state.lifecycle.lock().await;
    state.cancel().await;
    let index=state.index(&app).await?.clone();
    let key=state.api_key().await;
    if key.is_empty(){return Err("尚未配置 API Key；可使用本地检索".into());}
    state.session.store(true,Ordering::SeqCst);
    let cancel=CancellationToken::new();*state.active.lock().await=Some(cancel.clone());
    let network=state.network.clone();
    tauri::async_runtime::spawn(async move {
        let emit=|body:Value| {if !cancel.is_cancelled(){let _=app.emit("interview-answer",json!({"question_id":question_id,"revision":revision,"event":body}));}};
        let result=async {
            let _slot=tokio::select! {_=cancel.cancelled()=>return Err("已取消".into()),slot=network.lock()=>slot};
            let started=std::time::Instant::now();
            let aliases=index.setting("aliases").await?;
            let evidence=index.search(&question,if aliases.is_empty(){DEFAULT_ALIASES}else{&aliases}).await?;
            emit(json!({"type":"sources","evidence":evidence,"retrieval_ms":started.elapsed().as_millis(),"index_version":index.setting("version").await?}));
            let messages=interview_core::messages_with_dialogue(&question,&index.setting("facts").await?,&history,&evidence,&dialogue.unwrap_or_default())?;
            interview_core::stream_answer(&key,messages,cancel.clone(),|event|emit(serde_json::to_value(event).unwrap_or_default())).await
        }.await;
        if let Err(message)=result {emit(json!({"type":"error","message":message}));}
    });
    Ok(())
}
#[tauri::command]
pub async fn answer_cancel(state:State<'_,InterviewState>)->Result<(),String> {state.cancel().await;Ok(())}
#[tauri::command]
pub async fn session_clear(state:State<'_,InterviewState>)->Result<(),String> {
    let _guard=state.lifecycle.lock().await;state.cancel().await;state.session.store(false,Ordering::SeqCst);Ok(())
}

#[tauri::command]
pub async fn interview_history_clear(app:tauri::AppHandle,state:State<'_,InterviewState>,snapshot:Value)->Result<crate::interview_history::ClearReport,String> {
    let _guard=state.lifecycle.lock().await;
    state.cancel().await;
    let _network=state.network.lock().await;
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
pub async fn interview_audio_start(app:tauri::AppHandle,state:State<'_,InterviewState>,record_microphone:bool)->Result<crate::audio::recording_commands::MicrophoneStatus,String> {
    let _guard=state.lifecycle.lock().await;
    let existing=crate::api::api::api_get_transcript_config(app.clone(),app.state(),None).await?;
    // Upstream seeds Parakeet on first launch. The interview entry point is
    // explicitly Whisper-only; choose base unless a valid Whisper preference exists.
    if existing.as_ref().map_or(true,|config|config.provider!="localWhisper") {
        crate::api::api::api_save_transcript_config(app.clone(),app.state(),"localWhisper".into(),"base".into(),None,None).await?;
    }
    let config=crate::api::api::api_get_transcript_config(app.clone(),app.state(),None).await?
        .ok_or("请先在原生设置中下载并选择 Whisper 多语言模型")?;
    if config.provider!="localWhisper" || config.model.contains(".en") {
        return Err("面试辅助只使用本地多语言 Whisper；请在设置中选择 base 或 small，不选择 .en 模型".into());
    }
    let mut prefs=crate::audio::recording_preferences::load_recording_preferences(&app).await.map_err(|e|e.to_string())?;
    prefs.auto_save=false;
    crate::audio::recording_preferences::save_recording_preferences(&app,&prefs).await.map_err(|e|e.to_string())?;
    crate::set_language_preference("zh".into()).await?;
    let mic = if record_microphone { prefs.preferred_mic_device } else { Some("__interview_disabled__".into()) };
    crate::audio::recording_commands::start_recording_with_devices_and_meeting(app,mic,prefs.preferred_system_device,Some("Interview practice".into())).await?;
    Ok(crate::audio::recording_commands::microphone_status())
}

#[tauri::command]
pub async fn interview_microphone_set(app:tauri::AppHandle,state:State<'_,InterviewState>,enabled:bool)->Result<crate::audio::recording_commands::MicrophoneStatus,String> {
    let _guard = state.lifecycle.lock().await;
    MICROPHONE_LEVEL.store(0,Ordering::Relaxed);
    crate::audio::recording_commands::set_microphone_enabled(&app,enabled).await
}
