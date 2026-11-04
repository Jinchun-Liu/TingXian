//! The main webview owns capture and AI work. The secondary webview is a
//! presentation client; its small typed action channel cannot start a request.
use serde::{Deserialize,Serialize};
use std::sync::Mutex;
use tauri::{Emitter,Manager,State,WebviewWindow,WebviewWindowBuilder,WebviewUrl};

const DISPLAY:&str="interview-conversation";
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayTurn {id:String,role:String,question:String,suggestion:String,status:String,time:String,#[serde(default)] passive:bool}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields,rename_all="camelCase")]
pub struct Preferences {font_size:String,show_self:bool,focus:bool}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presentation {epoch:String,version:u64,history:Vec<DisplayTurn>,preferences:Preferences,selected_id:Option<String>,#[serde(default)] call_notice:String}
impl Presentation {
    fn validate(&self)->Result<(),String>{
        if self.call_notice.chars().count()>100 || self.epoch.len()>100 || self.history.len()>10000 || !["small","medium","large"].contains(&self.preferences.font_size.as_str()) {return Err("对话展示数据无效".into());}
        if self.history.iter().any(|t|t.id.len()>1000 || t.question.len()>64000 || t.suggestion.len()>128000 || !["remote","self","manual"].contains(&t.role.as_str())) {return Err("对话展示记录超过限制".into());}
        Ok(())
    }
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(tag="type",rename_all="snake_case",deny_unknown_fields)]
pub enum PresentationAction {Select{id:Option<String>},Edit{id:String}}
struct Bounds {position:tauri::PhysicalPosition<i32>,size:tauri::PhysicalSize<u32>}
#[derive(Default)]
pub struct PresentationState {snapshot:Mutex<Option<Presentation>>,original:Mutex<Option<Bounds>>}
fn owner<R:tauri::Runtime>(window:&WebviewWindow<R>)->Result<(),String>{if window.label()=="main"{Ok(())}else{Err("仅主窗口可以修改展示状态".into())}}

#[tauri::command]
pub fn interview_presentation_publish<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,PresentationState>,snapshot:Presentation)->Result<(),String>{
    owner(&window)?;snapshot.validate()?;
    *state.snapshot.lock().unwrap_or_else(|e|e.into_inner())=Some(snapshot.clone());
    window.app_handle().emit_to(DISPLAY,"interview-presentation",&snapshot).map_err(|e|e.to_string())
}
#[tauri::command]
pub fn interview_presentation_get(state:State<'_,PresentationState>)->Option<Presentation>{state.snapshot.lock().unwrap_or_else(|e|e.into_inner()).clone()}
#[tauri::command]
pub fn interview_presentation_action<R:tauri::Runtime>(window:WebviewWindow<R>,action:PresentationAction)->Result<(),String>{
    if window.label()!=DISPLAY{return Err("操作来源不是对话窗口".into());}
    window.app_handle().emit_to("main","interview-presentation-action",action).map_err(|e|e.to_string())
}
pub fn restore_main<R:tauri::Runtime>(app:&tauri::AppHandle<R>){
    let state=app.state::<PresentationState>();
    *state.snapshot.lock().unwrap_or_else(|e|e.into_inner())=None;
    if let Some(bounds)=state.original.lock().unwrap_or_else(|e|e.into_inner()).take(){
        if let Some(main)=app.get_webview_window("main") {
            let _=main.set_size(bounds.size);let _=main.set_position(bounds.position);let _=main.show();
        }
    }
    let _=app.emit_to("main","interview-window-mode",false);
}
#[tauri::command]
pub fn interview_window_mode_get<R:tauri::Runtime>(app:tauri::AppHandle<R>)->bool{app.get_webview_window(DISPLAY).is_some()}
#[tauri::command]
pub async fn interview_window_mode<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,PresentationState>,enabled:bool)->Result<bool,String>{
    owner(&window)?;let app=window.app_handle();
    crate::interview_exit::ensure_not_exiting(app)?;
    if !enabled {
        if let Some(display)=app.get_webview_window(DISPLAY){display.close().map_err(|e|e.to_string())?;}
        restore_main(app);return Ok(false);
    }
    if let Some(display)=app.get_webview_window(DISPLAY){display.show().map_err(|e|e.to_string())?;return Ok(true);}
    let position=window.outer_position().map_err(|e|e.to_string())?;
    let size=window.inner_size().map_err(|e|e.to_string())?;
    let scale=window.scale_factor().map_err(|e|e.to_string())?;
    let height=(size.height as f64/scale).max(400.);
    let display=WebviewWindowBuilder::new(app,DISPLAY,WebviewUrl::App("interview-display".into()))
        .title("听弦 · 对话").resizable(true).decorations(true)
        .inner_size((size.width as f64/scale-336.).max(600.),height).min_inner_size(380.,280.)
        .position(position.x as f64/scale+336.,position.y as f64/scale).build().map_err(|e|e.to_string())?;
    *state.original.lock().unwrap_or_else(|e|e.into_inner())=Some(Bounds{position,size});
    if let Err(error)=window.set_size(tauri::LogicalSize::new(320.,height)){let _=display.close();restore_main(app);return Err(error.to_string());}
    app.emit_to("main","interview-window-mode",true).map_err(|e|e.to_string())?;Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json,Value};
    #[test]
    fn display_channel_rejects_settings_secrets_and_generation_actions(){
        let snapshot=json!({"epoch":"fixture","version":1,"history":[{"id":"q","role":"remote","question":"合成问题","suggestion":"合成答案","status":"done","time":"00:00"}],"preferences":{"fontSize":"medium","showSelf":false,"focus":true},"selected_id":null});
        let parsed:Presentation=serde_json::from_value(snapshot.clone()).unwrap();parsed.validate().unwrap();
        let mut invalid=snapshot.clone();invalid["api_key"]=json!("synthetic-secret");assert!(serde_json::from_value::<Presentation>(invalid).is_err());
        assert!(serde_json::from_value::<PresentationAction>(json!({"type":"answer_start","id":"q"})).is_err());
        let app=tauri::test::mock_builder().manage(PresentationState::default()).invoke_handler(tauri::generate_handler![interview_presentation_publish,interview_presentation_get,interview_presentation_action]).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();
        let child=WebviewWindowBuilder::new(&app,DISPLAY,Default::default()).build().unwrap();
        let call=|window:&WebviewWindow<tauri::test::MockRuntime>,cmd:&str,body:Value|tauri::test::get_ipc_response(window,tauri::webview::InvokeRequest {
            cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.into(),
        });
        assert!(call(&child,"interview_presentation_publish",json!({"snapshot":snapshot})).is_err());
        call(&main,"interview_presentation_publish",json!({"snapshot":snapshot})).unwrap();
        assert_eq!(call(&child,"interview_presentation_get",json!({})).unwrap().deserialize::<Value>().unwrap()["history"][0]["question"],"合成问题");
        assert!(call(&main,"interview_presentation_action",json!({"action":{"type":"edit","id":"q"}})).is_err());
        call(&child,"interview_presentation_action",json!({"action":{"type":"select","id":"q"}})).unwrap();
        restore_main(app.handle());assert!(app.state::<PresentationState>().snapshot.lock().unwrap().is_none());
    }
}
