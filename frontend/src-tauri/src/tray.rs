use tauri::{
    Emitter,
    menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};

#[derive(Debug, Clone)]
pub enum RecordingState {
    Stopped,
    Starting,
    Recording,
    Pausing,
    Paused,
    Resuming,
    Stopping,
}

pub fn create_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // Start with default menu, will update with actual state after initialization
    // Pass can_record=true initially, will be updated by update_tray_menu immediately
    let menu = build_menu(app, RecordingState::Stopped)?;

    TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .tooltip("听弦")
        .icon(app.default_window_icon().unwrap().clone())
        .on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref()))
        .build(app)?;

    // Update tray menu with actual recording state after creation
    update_tray_menu(app);

    Ok(())
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    if id == "quit" { app.exit(0); return; }
    focus_main_window(app);
    let command = match id {
        "toggle_recording" => "start", "stop_recording" => "stop",
        "pause_recording" => "pause", "resume_recording" => "resume",
        "settings" => "audio", "history" => "history", "cancel_answers" => "cancel",
        _ => return,
    };
    if let Err(error) = app.emit_to("main", "tingxian-session-command", command) {
        log::warn!("未能将托盘操作交给主窗口：{error}");
    }
}

pub fn update_tray_menu<R: Runtime>(app: &AppHandle<R>) {
    // For sync update, spawn async task to get current state
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        // Small delay to ensure recording state has been updated
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        update_tray_menu_async(&app_clone).await;
    });
}

pub fn set_tray_state<R: Runtime>(app: &AppHandle<R>, state: RecordingState) {
    log::info!("Tray: Setting intermediate state: {:?}", state);
    // During recording state transitions, we assume recording is allowed (we're already recording)
    if let Ok(menu) = build_menu(app, state) {
        if let Some(tray) = app.tray_by_id("main-tray") {
            let result = tray.set_menu(Some(menu));
            log::info!("Tray: Intermediate state menu update result: {:?}", result);
        } else {
            log::warn!("Tray: Could not find tray with id 'main-tray'");
        }
    } else {
        log::error!("Tray: Failed to build menu for intermediate state");
    }
}

async fn get_current_recording_state() -> RecordingState {
    // Check if currently recording
    let is_recording = crate::audio::recording_commands::is_recording().await;
    log::info!(
        "Tray: get_current_recording_state - is_recording: {}",
        is_recording
    );

    if !is_recording {
        log::info!("Tray: Recording state is Stopped");
        return RecordingState::Stopped;
    }

    // Check if paused
    let is_paused = crate::audio::recording_commands::is_recording_paused().await;
    log::info!("Tray: is_paused: {}", is_paused);

    if is_paused {
        log::info!("Tray: Recording state is Paused");
        RecordingState::Paused
    } else {
        log::info!("Tray: Recording state is Recording");
        RecordingState::Recording
    }
}

pub async fn update_tray_menu_async<R: Runtime>(app: &AppHandle<R>) {
    log::info!("Tray: update_tray_menu_async called");
    // Get the current recording state
    let recording_state = get_current_recording_state().await;
    log::info!("Tray: Current recording state: {:?}", recording_state);

    if let Ok(menu) = build_menu(app, recording_state) {
        if let Some(tray) = app.tray_by_id("main-tray") {
            let result = tray.set_menu(Some(menu));
            log::info!("Tray: Menu update result: {:?}", result);
        } else {
            log::warn!("Tray: Could not find tray with id 'main-tray'");
        }
    } else {
        log::error!("Tray: Failed to build menu");
    }
}

fn build_menu<R: Runtime>(
    app: &AppHandle<R>,
    state: RecordingState,
) -> tauri::Result<tauri::menu::Menu<R>> {
    let mut builder = MenuBuilder::new(app);
        match state {
            RecordingState::Stopped => {
                builder = builder
                    .item(&MenuItemBuilder::with_id("toggle_recording", "开始持续聆听").build(app)?);
            }
            RecordingState::Starting => {
                builder = builder.item(
                    &MenuItemBuilder::new("正在开始采集…")
                        .enabled(false)
                        .build(app)?,
                );
            }
            RecordingState::Recording => {
                builder = builder
                    .item(&MenuItemBuilder::with_id("pause_recording", "暂停采集").build(app)?)
                    .item(&MenuItemBuilder::with_id("stop_recording", "停止采集").build(app)?);
            }
            RecordingState::Pausing => {
                builder = builder
                    .item(
                        &MenuItemBuilder::new("正在暂停…")
                            .enabled(false)
                            .build(app)?,
                    )
                    .item(&MenuItemBuilder::with_id("stop_recording", "停止采集").build(app)?);
            }
            RecordingState::Paused => {
                builder = builder
                    .item(
                        &MenuItemBuilder::with_id("resume_recording", "恢复采集")
                            .build(app)?,
                    )
                    .item(&MenuItemBuilder::with_id("stop_recording", "停止采集").build(app)?);
            }
            RecordingState::Resuming => {
                builder = builder
                    .item(
                        &MenuItemBuilder::new("正在恢复…")
                            .enabled(false)
                            .build(app)?,
                    )
                    .item(&MenuItemBuilder::with_id("stop_recording", "停止采集").build(app)?);
            }
            RecordingState::Stopping => {
                builder = builder.item(
                    &MenuItemBuilder::new("正在停止…")
                        .enabled(false)
                        .build(app)?,
                );
            }
        }
    builder.item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItemBuilder::with_id("open_window","打开听弦").build(app)?)
        .item(&MenuItemBuilder::with_id("settings","音频设置").build(app)?)
        .item(&MenuItemBuilder::with_id("history","会议记录").build(app)?)
        .item(&MenuItemBuilder::with_id("cancel_answers","取消回答与队列").build(app)?)
        .item(&MenuItemBuilder::with_id("quit","退出听弦").build(app)?)
        .build()
}

pub(crate) fn focus_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(e) = window.unminimize() {
            log::error!("Failed to unminimize main window: {}", e);
        }

        if let Err(e) = window.show() {
            log::error!("Failed to show main window: {}", e);
        }

        if let Err(e) = window.set_focus() {
            log::error!("Failed to focus main window: {}", e);
        }

        if let Err(e) = window.eval("window.focus()") {
            log::error!("Failed to focus main webview: {}", e);
        }
    } else {
        log::warn!("Could not find main window");
    }
}
