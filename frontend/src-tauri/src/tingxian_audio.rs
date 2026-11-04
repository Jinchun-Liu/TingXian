//! Local audio settings share the recorder's existing JSON store and SQLite row.
use tauri::{AppHandle, Manager, State};
use crate::{audio::recording_preferences as prefs, database::repositories::setting::SettingsRepository, interview::InterviewState, state::AppState, whisper_engine::{commands as whisper, ModelStatus}};
use serde_json::{json, Value};

pub(crate) async fn selected_model(app: &AppHandle) -> Result<String, String> {
    let bundled = crate::tingxian_runtime::prepare_model(app).await?;
    let database = app.state::<AppState>();
    let row: Option<(String, String)> = sqlx::query_as("SELECT provider, model FROM transcript_settings WHERE id = '1'")
        .fetch_optional(database.db_manager.pool()).await.map_err(|e|e.to_string())?;
    if let Some((provider, model)) = row {
        let missing_seed = model == "base" && bundled.is_some() && !app.path().app_data_dir().map_err(|e|e.to_string())?.join("models/ggml-base.bin").is_file();
        if provider == "localWhisper" && !model.is_empty() && !missing_seed { return Ok(model); }
    }
    Ok(bundled.unwrap_or("base").into())
}

#[tauri::command]
pub async fn tingxian_asr_models(app: AppHandle) -> Result<Value, String> {
    let selected = selected_model(&app).await?;
    whisper::whisper_init().await?;
    let models = whisper::whisper_get_available_models().await?;
    Ok(json!({"selected":selected,"models":models}))
}

#[tauri::command]
pub async fn tingxian_select_model(app: AppHandle, state: State<'_, InterviewState>, model: String) -> Result<(), String> {
    let _guard = state.lifecycle.lock().await;
    if crate::audio::recording_commands::is_recording().await { return Err("请先停止采集再切换模型".into()); }
    whisper::whisper_init().await?;
    if !whisper::whisper_get_available_models().await?.iter().any(|m|m.name == model && matches!(m.status, ModelStatus::Available)) {
        return Err("模型尚未就绪，请先下载或重新下载".into());
    }
    save_model(&app, &model).await
}

pub(crate) async fn save_model(app: &AppHandle, model: &str) -> Result<(), String> {
    SettingsRepository::save_transcript_config(app.state::<AppState>().db_manager.pool(), "localWhisper", model)
        .await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn tingxian_select_devices(app: AppHandle, state: State<'_, InterviewState>, microphone: Option<String>, system: Option<String>) -> Result<(), String> {
    let _guard = state.lifecycle.lock().await;
    if crate::audio::recording_commands::is_recording().await {return Err("请先停止采集再切换设备".into());}
    let mut config = prefs::load_recording_preferences(&app).await.map_err(|e|e.to_string())?;
    config.preferred_mic_device = microphone;
    config.preferred_system_device = system;
    prefs::save_recording_preferences(&app, &config).await.map_err(|e|e.to_string())
}
