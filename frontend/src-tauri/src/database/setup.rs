use tauri::{AppHandle, Manager};
use super::manager::DatabaseManager;
use crate::state::AppState;
static OPEN:tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Fresh installs and upgrades take the same path. No webview listener, login or network service needed.
#[tauri::command]
pub async fn tingxian_storage_ready(app:AppHandle)->Result<(),String>{
    let _guard=OPEN.lock().await;
    if app.try_state::<AppState>().is_none(){
        let db_manager=DatabaseManager::new_from_app_handle(&app).await.map_err(|e|e.to_string())?;
        app.manage(AppState{db_manager});
    }
    Ok(())
}
