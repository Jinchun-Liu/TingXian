//! Local, single-use screenshot previews. No image is written to disk or emitted
//! to the read-only display window. Preparing a preview never reserves an AI lane.
use std::sync::{Arc,Mutex};
use std::time::{Duration,Instant};
use serde::{Deserialize,Serialize};
use tauri::{Manager,State};
use tokio_util::sync::CancellationToken;
use crate::interview::{InterviewState,AnswerImage};

const TTL:Duration=Duration::from_secs(300);

#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct PreviewContext {
    question_id:String, question:String,
    history:Vec<interview_core::Turn>, dialogue:Vec<interview_core::SpokenTurn>, meeting_mode:String,
}
impl PreviewContext {
    fn hash(&self)->Result<String,String> {
        if self.question_id.is_empty() || self.question_id.len()>100 || self.question.trim().is_empty()
            || self.question.chars().count()>1500 || self.history.len()>3 || self.dialogue.len()>12
            || !["pair","panel","group","groups","mixed"].contains(&self.meeting_mode.as_str()) {return Err("截图问题上下文无效，未发送".into());}
        let bytes=serde_json::to_vec(self).map_err(|_|"截图上下文格式无效")?;
        if bytes.len()>64*1024 {return Err("截图上下文过长，请减少输入后重试".into());}
        Ok(interview_core::digest(&bytes))
    }
}

struct Entry {
    id:String, question_id:String, context_hash:String, topology:String, provider:String,
    created:Instant, cancel:CancellationToken, png:Option<Vec<u8>>,
}
#[derive(Default)]
pub(crate) struct PreviewStore(Mutex<Option<Entry>>,Mutex<std::collections::VecDeque<String>>);
struct CaptureLease {store:Arc<PreviewStore>, id:String, cancel:CancellationToken}
impl Drop for CaptureLease {
    fn drop(&mut self) {
        let mut slot=self.store.0.lock().unwrap_or_else(|e|e.into_inner());
        if slot.as_ref().is_some_and(|e|e.id==self.id && e.png.is_none()) {
            if let Some(e)=slot.take(){e.cancel.cancel();}
        }
    }
}
impl PreviewStore {
    fn begin(self:&Arc<Self>,context:&PreviewContext,topology:String,provider:String,now:Instant)->Result<CaptureLease,String> {
        let context_hash=context.hash()?;
        let cancelled=self.1.lock().unwrap_or_else(|e|e.into_inner());
        if cancelled.contains(&context.question_id){return Err("截图已取消，请重新发起".into());}
        let mut slot=self.0.lock().unwrap_or_else(|e|e.into_inner());
        if slot.as_ref().is_some_and(|e|now.duration_since(e.created)>=TTL) {if let Some(e)=slot.take(){e.cancel.cancel();}}
        if slot.is_some(){return Err("已有截图正在准备或预览，请先取消".into());}
        let id=uuid::Uuid::new_v4().to_string();let cancel=CancellationToken::new();
        *slot=Some(Entry{id:id.clone(),question_id:context.question_id.clone(),context_hash,topology,provider,created:now,cancel:cancel.clone(),png:None});
        Ok(CaptureLease{store:self.clone(),id,cancel})
    }
    fn finish(&self,id:&str,png:Vec<u8>,now:Instant)->Result<(),String> {
        // Validation is shared with the final image payload, including its bound.
        if png.len()>interview_core::MAX_SCREENSHOT_BYTES || !png.starts_with(b"\x89PNG\r\n\x1a\n") {return Err("截图内容无效或超过 10 MiB，未发送".into());}
        let mut slot=self.0.lock().unwrap_or_else(|e|e.into_inner());
        let entry=slot.as_mut().filter(|e|e.id==id && !e.cancel.is_cancelled() && now.duration_since(e.created)<TTL).ok_or("截图已取消或过期，请重新截取")?;
        entry.png=Some(png);Ok(())
    }
    pub(crate) fn clear(&self,question_id:Option<&str>) {
        // A discard can overtake an async IPC start. Remember recent explicit
        // cancellations so a late create cannot open a new capture afterward.
        let mut cancelled=self.1.lock().unwrap_or_else(|e|e.into_inner());
        if let Some(id)=question_id {if !cancelled.iter().any(|q|q==id){cancelled.push_back(id.into());}while cancelled.len()>128{cancelled.pop_front();}}
        let mut slot=self.0.lock().unwrap_or_else(|e|e.into_inner());
        if slot.as_ref().is_some_and(|e|question_id.is_none() || question_id==Some(e.question_id.as_str())) {
            if let Some(e)=slot.take(){e.cancel.cancel();}
        }
    }
    fn bind_provider(&self,id:&str,provider:String)->Result<(),String> {
        let mut slot=self.0.lock().unwrap_or_else(|e|e.into_inner());
        slot.as_mut().filter(|e|e.id==id && !e.cancel.is_cancelled()).ok_or("截图已取消")?.provider=provider;Ok(())
    }
    fn expire(&self,id:&str) {
        let mut slot=self.0.lock().unwrap_or_else(|e|e.into_inner());
        if slot.as_ref().is_some_and(|e|e.id==id && e.created.elapsed()>=TTL) {if let Some(e)=slot.take(){e.cancel.cancel();}}
    }
    pub(crate) fn take(&self,id:&str,context_hash:&str,topology:&str,provider:&str,now:Instant)->Result<Vec<u8>,String> {
        let mut slot=self.0.lock().unwrap_or_else(|e|e.into_inner());
        if !slot.as_ref().is_some_and(|e|e.id==id) {return Err("截图已取消、已使用或过期，请重新截取".into());}
        let entry=slot.take().unwrap();
        if entry.cancel.is_cancelled() || now.duration_since(entry.created)>=TTL {return Err("截图已过期，请重新截取".into());}
        if entry.context_hash!=context_hash {return Err("截图对应的问题或上下文已变化，请重新截取".into());}
        if entry.topology!=topology {return Err("屏幕连接或分辨率已变化，请重新选择并预览".into());}
        if entry.provider!=provider {return Err("视觉模型配置已变化，请重新预览".into());}
        entry.png.ok_or_else(||"截图尚未完成，未发送".into())
    }
}
pub(crate) fn topology()->Result<String,String> {
    let mut screens=crate::interview_screenshot::interview_screenshot_displays()?;
    screens.sort_by_key(|d|d.id);
    Ok(interview_core::digest(&serde_json::to_vec(&screens).map_err(|_|"屏幕列表无效")?))
}
pub(crate) fn provider_hash(profile:&interview_core::ProviderProfile)->Result<String,String> {
    Ok(interview_core::digest(&serde_json::to_vec(profile).map_err(|_|"视觉模型配置无效")?))
}
fn preview_url(png:&[u8])->Result<String,String> {
    // Reuse the production PNG validation/encoding without altering frozen
    // model request code. This only assembles JSON locally; it cannot send it.
    let mut payload=interview_core::attach_screenshot(serde_json::json!([
        {"role":"system","content":""},{"role":"user","content":""}
    ]),png)?;
    match payload[1]["content"][1]["image_url"]["url"].take() {
        serde_json::Value::String(url)=>Ok(url),_=>Err("截图预览编码失败".into()),
    }
}
#[derive(Serialize)]
pub struct Preview {preview_id:String,data_url:String,display_id:u32,topology:String,expires_in_seconds:u64,requires_confirmation:bool}

#[tauri::command]
pub async fn interview_screenshot_preview_create<R:tauri::Runtime>(app:tauri::AppHandle<R>,window:tauri::WebviewWindow<R>,state:State<'_,InterviewState>,context:PreviewContext,display_id:u32)->Result<Preview,String> {
    if window.label()!="main" {return Err("请在主窗口截屏".into());}
    crate::interview_exit::ensure_not_exiting(&app)?;
    let requires_confirmation=!crate::interview_screenshot::capture_permission_granted();
    // Register before the first await, so global cancellation sees this work.
    let lease=state.screenshots.begin(&context,topology()?,String::new(),Instant::now())?;
    {
        let _guard=state.lifecycle.lock().await;
        crate::interview_exit::ensure_not_exiting(&app)?;
        let profiles=state.index(&app).await?.provider_config().await?;
        state.screenshots.bind_provider(&lease.id,provider_hash(&profiles.vision)?)?;
    }
    let png=crate::interview_screenshot::capture(Some(display_id),lease.cancel.clone()).await?;
    let data_url=preview_url(&png)?;
    let current=topology()?;
    {
        let slot=state.screenshots.0.lock().unwrap_or_else(|e|e.into_inner());
        if !slot.as_ref().is_some_and(|e|e.id==lease.id && e.topology==current) {return Err("截屏期间目标已变化或操作已取消，请重试".into());}
    }
    state.screenshots.finish(&lease.id,png,Instant::now())?;
    let expiry_store=state.screenshots.clone();let expiry_id=lease.id.clone();
    tauri::async_runtime::spawn(async move {tokio::time::sleep(TTL).await;expiry_store.expire(&expiry_id);});
    Ok(Preview{preview_id:lease.id.clone(),data_url,display_id,topology:current,expires_in_seconds:TTL.as_secs(),requires_confirmation})
}
#[tauri::command]
pub fn interview_screenshot_preview_discard<R:tauri::Runtime>(window:tauri::WebviewWindow<R>,state:State<'_,InterviewState>,question_id:String)->Result<(),String> {
    if window.label()!="main" {return Err("请在主窗口取消截图".into());}
    if question_id.is_empty() || question_id.len()>100 {return Err("截图问题 ID 无效".into());}
    state.screenshots.clear(Some(&question_id));Ok(())
}
#[tauri::command]
pub async fn interview_screenshot_preview_answer_start<R:tauri::Runtime>(app:tauri::AppHandle<R>,window:tauri::WebviewWindow<R>,state:State<'_,InterviewState>,context:PreviewContext,preview_id:String,revision:u32)->Result<(),String> {
    if window.label()!="main" {return Err("请在主窗口确认截图".into());}
    let hash=context.hash()?;
    if revision==0{return Err("回答请求版本无效".into());}
    // The immutable question/context hash is the image binding. The existing
    // foreground stream counter is allocated only on send, not during preview.
    crate::interview::start_answer(app,&state,context.question_id,revision,context.question,context.history,Some(context.dialogue),Some(AnswerImage::Preview(preview_id,hash)),true,Some(context.meeting_mode)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context()->PreviewContext {PreviewContext{question_id:"synthetic-question".into(),question:"分析合成题目中的 null 与 0".into(),history:vec![],dialogue:vec![],meeting_mode:"pair".into()}}
    const PNG:&[u8]=b"\x89PNG\r\n\x1a\nfixture";
    #[test]
    fn preview_is_single_use_and_bound_to_exact_context_screen_and_provider() {
        let now=Instant::now();let context=context();let hash=context.hash().unwrap();
        for mismatch in 0..4 {
            let store=Arc::new(PreviewStore::default());let lease=store.begin(&context,"screen".into(),"provider".into(),now).unwrap();
            store.finish(&lease.id,PNG.to_vec(),now).unwrap();
            let result=store.take(&lease.id,if mismatch==1{"changed"}else{&hash},if mismatch==2{"disconnected"}else{"screen"},if mismatch==3{"new-model"}else{"provider"},now);
            assert_eq!(result.is_ok(),mismatch==0);
            assert!(store.take(&lease.id,&hash,"screen","provider",now).is_err());
        }
    }
    #[test]
    fn cancellation_late_completion_and_old_discard_cannot_change_new_preview() {
        let store=Arc::new(PreviewStore::default());let mut context=context();let now=Instant::now();
        let first=store.begin(&context,"s".into(),"p".into(),now).unwrap();
        assert!(store.begin(&context,"s".into(),"p".into(),now).is_err());
        store.clear(None);assert!(first.cancel.is_cancelled());context.question_id="second".into();
        let second=store.begin(&context,"s".into(),"p".into(),now).unwrap();
        assert!(store.finish(&first.id,PNG.to_vec(),now).is_err());drop(first);
        store.clear(Some("synthetic-question"));assert!(!second.cancel.is_cancelled());
        store.finish(&second.id,PNG.to_vec(),now).unwrap();
        assert!(store.take(&second.id,&context.hash().unwrap(),"s","p",now).is_ok());
        context.question_id="late-create".into();store.clear(Some("late-create"));
        assert!(store.begin(&context,"s".into(),"p".into(),now).is_err());
    }
    #[test]
    fn dropped_capture_expiry_and_invalid_image_leave_no_sendable_asset() {
        let store=Arc::new(PreviewStore::default());let context=context();let now=Instant::now();
        {let lease=store.begin(&context,"s".into(),"p".into(),now).unwrap();assert!(store.finish(&lease.id,b"bad".to_vec(),now).is_err());}
        let lease=store.begin(&context,"s".into(),"p".into(),now).unwrap();store.finish(&lease.id,PNG.to_vec(),now).unwrap();
        assert!(store.take(&lease.id,&context.hash().unwrap(),"s","p",now+TTL).is_err());
        let mut oversized=context.clone();oversized.history=vec![interview_core::Turn{question:"q".into(),confirmed_answer:None,suggested_answer:None};4];assert!(oversized.hash().is_err());
    }
    #[test]
    fn display_window_cannot_capture_discard_or_send_main_preview() {
        let state=InterviewState::default();let ctx=context();let lease=state.screenshots.begin(&ctx,"s".into(),"p".into(),Instant::now()).unwrap();
        let app=tauri::test::mock_builder().manage(state).invoke_handler(tauri::generate_handler![interview_screenshot_preview_discard,interview_screenshot_preview_create,interview_screenshot_preview_answer_start]).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let window=tauri::WebviewWindowBuilder::new(&app,"display",Default::default()).build().unwrap();
        for (cmd,body) in [
            ("interview_screenshot_preview_discard",serde_json::json!({"questionId":ctx.question_id})),
            ("interview_screenshot_preview_create",serde_json::json!({"context":ctx,"displayId":1})),
            ("interview_screenshot_preview_answer_start",serde_json::json!({"context":ctx,"previewId":lease.id,"revision":2})),
        ] {
            let response=tauri::test::get_ipc_response(&window,tauri::webview::InvokeRequest{cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()});
            assert!(response.is_err());assert!(!lease.cancel.is_cancelled());assert!(app.state::<InterviewState>().answers_idle());
        }
    }
}
