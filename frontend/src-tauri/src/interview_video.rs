//! Main-window-only prerequisite inspection. This is not a capture/SDK start API.
use std::{sync::Arc, time::Duration};
use tauri::{Manager, State, WebviewWindow};
use video_worker_control::{BundledWorker, Controller, Snapshot};

#[derive(Default)]
pub struct VideoState(pub Arc<Controller>, #[cfg(windows)] pub Arc<video_worker_control::joint::host::Host>);
fn owner<R:tauri::Runtime>(window:&WebviewWindow<R>)->Result<(),String> {
    if window.label()=="main" {Ok(())} else {Err("仅主窗口可以管理视频".into())}
}
fn pinned_worker(root:std::path::PathBuf)->Option<BundledWorker> {
    if !cfg!(windows) {return None;}
    option_env!("TINGXIAN_VIDEO_WORKER_SHA256").filter(|v|!v.is_empty()).map(|hash|BundledWorker{
        path:root.join("video-worker").join("tingxian-video-worker.exe"),sha256:hash.to_string()
    })
}
// File inspection only. No caller-supplied path, digest, DLL load or device access.
#[tauri::command]
pub async fn video_components<R:tauri::Runtime>(window:WebviewWindow<R>)->Result<serde_json::Value,String> {
    owner(&window)?;
    #[cfg(windows)] {
        let root=window.app_handle().path().resource_dir().map_err(|_|"无法读取应用组件目录")?;
        tauri::async_runtime::spawn_blocking(move || {
            serde_json::to_value(video_worker_control::bundle::inspect(&root.join("video-worker/media"),option_env!("TINGXIAN_MEDIA_MANIFEST_SHA256")))
                .map_err(|_|"无法读取媒体组件状态".to_owned())
        }).await.map_err(|_|"媒体组件检查意外退出".to_owned())?
    }
    #[cfg(not(windows))] {
        Ok(serde_json::json!({"state":"unsupported_platform","verified_components":0,"media_started":false}))
    }
}
#[tauri::command]
pub fn video_status<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,VideoState>)->Result<Snapshot,String> {
    owner(&window)?;Ok(state.0.snapshot())
}
#[tauri::command]
pub async fn video_inspect<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,VideoState>,expected_generation:u64)->Result<Snapshot,String> {
    owner(&window)?;
    crate::interview_exit::ensure_not_exiting(window.app_handle())?;
    #[cfg(windows)]
    if !state.1.wait_idle(Duration::ZERO) {return Err("媒体输出清理尚未确认，请先停止并保留诊断".into());}
    let root=window.app_handle().path().resource_dir().map_err(|_|"无法读取应用组件目录")?;
    let Some(run)=state.0.begin_if_current(pinned_worker(root),expected_generation).map_err(str::to_owned)? else {return Ok(state.0.snapshot());};
    tauri::async_runtime::spawn_blocking(move ||run.execute()).await.map_err(|_|"视频检查意外退出，请重新检查".into())
}
async fn wait_idle(state:&VideoState)->Result<(),String> {
    let controller=state.0.clone();
    #[cfg(windows)]
    let media=state.1.clone();
    let done=tauri::async_runtime::spawn_blocking(move ||{
        #[cfg(windows)]
        let media_done=media.wait_idle(Duration::from_secs(12));
        #[cfg(not(windows))]
        let media_done=true;
        let worker_done=controller.wait_idle(Duration::from_secs(7));
        media_done && worker_done
    })
        .await.map_err(|_|"视频清理任务异常，尚未确认退出")?;
    if done {Ok(())} else {Err("媒体线程或工作进程清理尚未确认，请保留诊断并重试".into())}
}
#[tauri::command]
pub async fn video_stop<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,VideoState>)->Result<Snapshot,String> {
    owner(&window)?;
    #[cfg(windows)]
    state.1.request_stop();
    state.0.request_stop();wait_idle(&state).await?;Ok(state.0.snapshot())
}
pub(crate) fn suspend<R:tauri::Runtime>(app:&tauri::AppHandle<R>) {
    if let Some(state)=app.try_state::<VideoState>() {
        #[cfg(windows)]
        state.1.suspend();
        state.0.suspend();
    }
}
pub(crate) fn resume<R:tauri::Runtime>(app:&tauri::AppHandle<R>) {
    if let Some(state)=app.try_state::<VideoState>() {
        state.0.resume();
        #[cfg(windows)]
        state.1.resume();
    }
}
pub(crate) async fn quiesce<R:tauri::Runtime>(app:&tauri::AppHandle<R>)->Result<(),String> {
    if let Some(state)=app.try_state::<VideoState>() {
        #[cfg(windows)]
        state.1.suspend();
        state.0.suspend();wait_idle(&state).await?;
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

#[cfg(windows)]
pub fn media_installation_check()->Result<(),String> {
    use video_worker_control::bundle::{inspect,State};
    let exe=std::env::current_exe().map_err(|_|"无法定位安装目录")?;
    let root=exe.parent().ok_or("安装目录无效")?;
    let report=inspect(&root.join("video-worker/media"),option_env!("TINGXIAN_MEDIA_MANIFEST_SHA256"));
    println!("{}",serde_json::to_string(&report).map_err(|_|"组件报告序列化失败")?);
    if report.state!=State::IntegrityVerified || report.verified_components!=4 || report.media_started {
        return Err(format!("Native media integrity check failed: {:?}",report.state));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json,Value};
    #[test]
    fn video_ipc_is_main_only_and_exit_cancellation_does_not_restart_work() {
        let app=tauri::test::mock_builder().manage(VideoState::default()).manage(crate::interview_exit::ExitState::default())
            .invoke_handler(tauri::generate_handler![video_status,video_inspect,video_stop,video_components,
                crate::interview_exit::interview_exit_status,crate::interview_exit::interview_exit_cancel])
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let display=tauri::WebviewWindowBuilder::new(&app,"interview-conversation",Default::default()).build().unwrap();
        let invoke=|window:&tauri::WebviewWindow<tauri::test::MockRuntime>,cmd:&str,body:Value|
            tauri::test::get_ipc_response(window,tauri::webview::InvokeRequest{cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),
                error:tauri::ipc::CallbackFn(1),url:if cfg!(any(windows,target_os="android")){"http://tauri.localhost"}else{"tauri://localhost"}.parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),
                headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.into()});
        for cmd in ["video_status","video_inspect","video_stop","video_components"] {
            let error=invoke(&display,cmd,json!({"expectedGeneration":0})).err().expect("display must be rejected");
            assert_eq!(error,json!("仅主窗口可以管理视频"),"wrong denial for {cmd}");
        }
        assert_eq!(app.state::<VideoState>().0.snapshot().generation,0,"display changed video state");
        let components:Value=invoke(&main,"video_components",json!({})).expect("main component inspection failed").deserialize().unwrap();
        assert_eq!(components["media_started"],false);
        assert_eq!(components["verified_components"],if components["state"]=="integrity_verified" {4}else{0});
        assert_eq!(app.state::<VideoState>().0.snapshot().generation,0,"component read started an eye job");
        #[cfg(windows)]
        assert_eq!(app.state::<VideoState>().1.snapshot().generation,0,"display changed media authority");
        let initial:Value=invoke(&main,"video_status",json!({})).expect("main status must reach the handler").deserialize().unwrap();
        assert_eq!(initial["phase"],"off");
        crate::interview_exit::request_exit(app.handle(),false);
        #[cfg(windows)]
        assert!(app.state::<VideoState>().1.snapshot().suspended);
        assert!(invoke(&main,"video_inspect",json!({"expectedGeneration":0})).is_err(),"inspection admitted during quit");
        let request:Value=invoke(&main,"interview_exit_status",json!({})).unwrap().deserialize().unwrap();
        invoke(&main,"interview_exit_cancel",json!({"requestId":request["request_id"]})).unwrap();
        assert_eq!(app.state::<VideoState>().0.snapshot().phase,video_worker_control::Phase::Off);
        assert!(app.state::<VideoState>().0.wait_idle(Duration::ZERO));
        #[cfg(windows)]
        {
            let media=&app.state::<VideoState>().1;
            assert!(!media.snapshot().suspended && !media.snapshot().busy);
            assert!(media.snapshot().output.is_none() && media.wait_idle(Duration::ZERO));
            assert_eq!(media.snapshot().generation,1,"cancel must not restore old media intent");
        }
        invoke(&main,"video_stop",json!({})).unwrap();
        let stopped=app.state::<VideoState>().0.snapshot().generation;
        assert_eq!(invoke(&main,"video_inspect",json!({"expectedGeneration":stopped-1})).err(),Some(json!("视频状态已变化，请重新检查")),"stale webview launch must reach generation admission");
        assert_eq!(app.state::<VideoState>().0.snapshot().generation,stopped);
    }
}
