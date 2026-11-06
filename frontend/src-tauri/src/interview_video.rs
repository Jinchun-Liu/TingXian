//! Main-window-only prerequisite inspection. This is not a capture/SDK start API.
use std::{sync::Arc, time::Duration};
use tauri::{Manager, State, WebviewWindow};
use video_worker_control::{BundledWorker, Controller, Snapshot};

#[derive(Default)]
pub struct VideoState(pub Arc<Controller>);
fn owner<R:tauri::Runtime>(window:&WebviewWindow<R>)->Result<(),String> {
    if window.label()=="main" {Ok(())} else {Err("仅主窗口可以管理视频".into())}
}
fn pinned_worker(root:std::path::PathBuf)->Option<BundledWorker> {
    if !cfg!(windows) {return None;}
    option_env!("TINGXIAN_VIDEO_WORKER_SHA256").filter(|v|!v.is_empty()).map(|hash|BundledWorker{
        path:root.join("video-worker").join("tingxian-video-worker.exe"),sha256:hash.to_string()
    })
}
#[tauri::command]
pub fn video_status<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,VideoState>)->Result<Snapshot,String> {
    owner(&window)?;Ok(state.0.snapshot())
}
#[tauri::command]
pub async fn video_inspect<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,VideoState>)->Result<Snapshot,String> {
    owner(&window)?;
    crate::interview_exit::ensure_not_exiting(window.app_handle())?;
    let root=window.app_handle().path().resource_dir().map_err(|_|"无法读取应用组件目录")?;
    let Some(run)=state.0.begin(pinned_worker(root)).map_err(str::to_owned)? else {return Ok(state.0.snapshot());};
    tauri::async_runtime::spawn_blocking(move ||run.execute()).await.map_err(|_|"视频检查意外退出，请重新检查".into())
}
async fn wait_idle(controller:Arc<Controller>)->Result<(),String> {
    let done=tauri::async_runtime::spawn_blocking(move ||controller.wait_idle(Duration::from_secs(7)))
        .await.map_err(|_|"视频清理任务异常，尚未确认退出")?;
    if done {Ok(())} else {Err("视频工作进程尚未确认退出，请保留诊断并重试".into())}
}
#[tauri::command]
pub async fn video_stop<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,VideoState>)->Result<Snapshot,String> {
    owner(&window)?;state.0.request_stop();wait_idle(state.0.clone()).await?;Ok(state.0.snapshot())
}
pub(crate) fn suspend<R:tauri::Runtime>(app:&tauri::AppHandle<R>) {
    if let Some(state)=app.try_state::<VideoState>() {state.0.suspend();}
}
pub(crate) fn resume<R:tauri::Runtime>(app:&tauri::AppHandle<R>) {
    if let Some(state)=app.try_state::<VideoState>() {state.0.resume();}
}
pub(crate) async fn quiesce<R:tauri::Runtime>(app:&tauri::AppHandle<R>)->Result<(),String> {
    if let Some(state)=app.try_state::<VideoState>() {
        state.0.suspend();wait_idle(state.0.clone()).await?;
    }
    Ok(())
}
// Explicit installer check: no app-data access or Tauri/audio/GUI initialization.
// Uses the same embedded pin, native worker and controller as the main command.
#[cfg(windows)]
pub fn installation_check()->Result<(),String> {
    let exe=std::env::current_exe().map_err(|_|"无法定位安装目录")?;
    let root=exe.parent().ok_or("安装目录无效")?.to_path_buf();
    let controller=Arc::new(Controller::default());
    let run=controller.begin(pinned_worker(root)).map_err(str::to_owned)?.ok_or("未随包部署视频检查组件")?;
    let status=run.execute();
    if status.reason!=Some(video_worker_control::Reason::RuntimeNotPinned) || status.inspection.is_none() {
        return Err(format!("视频检查组件失败：{:?}",status.reason));
    }
    println!("{}",serde_json::to_string(&status).map_err(|_|"检查结果序列化失败")?);
    Ok(())
}
