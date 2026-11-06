//! Native-only credentials and source-isolated streaming capture adapter.
use crate::{interview::InterviewState,audio::{recording_state::{RecordingState,DeviceType},asr_queue::{AsrReceiver,AudioLease},microphone_gate::MicrophoneGate,transcription::worker::TranscriptUpdate}};
use interview_core::cloud_asr::{CloudAsrConfig,AsrMode,PreparedTask,AsrEvent};
use serde_json::{json,Value};
use std::{sync::{Arc,atomic::{AtomicU64,Ordering}},collections::HashMap,time::Duration};
use tauri::{AppHandle,Emitter,State,WebviewWindow,Runtime};
use tokio::sync::{Mutex,mpsc};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct Credentials(Mutex<Option<(String,String)>>);
impl Credentials {
    async fn key(&self,config:&CloudAsrConfig)->String{self.0.lock().await.as_ref().filter(|(workspace,_)|workspace==&config.workspace_id).map(|(_,key)|key.clone()).unwrap_or_default()}
}
fn owner<R:Runtime>(window:&WebviewWindow<R>)->Result<(),String>{if window.label()=="main"{Ok(())}else{Err("请在主窗口配置云转写".into())}}
#[tauri::command]
pub async fn interview_cloud_asr_get<R:Runtime>(window:WebviewWindow<R>,app:AppHandle<R>,state:State<'_,InterviewState>)->Result<Value,String>{
    owner(&window)?;let config=state.index(&app).await?.cloud_asr_config().await?;
    Ok(json!({"key_configured":!state.cloud_asr.key(&config).await.is_empty(),"config":config}))
}
#[tauri::command]
pub async fn interview_cloud_asr_save<R:Runtime>(window:WebviewWindow<R>,app:AppHandle<R>,state:State<'_,InterviewState>,config:CloudAsrConfig,key:Option<String>)->Result<(),String>{
    owner(&window)?;crate::interview_exit::ensure_not_exiting(&app)?;config.validate()?;
    let _guard=state.lifecycle.try_lock().map_err(|_|"请等待当前操作完成")?;
    if state.session.load(Ordering::SeqCst)||crate::audio::recording_commands::is_recording().await{return Err("请先结束会话再切换转写服务".into());}
    if key.as_ref().is_some_and(|k|k.len()>4096||k.chars().any(char::is_control)){return Err("云转写 API Key 格式无效".into());}
    state.index(&app).await?.set_cloud_asr_config(&config).await?;
    if let Some(key)=key{*state.cloud_asr.0.lock().await=Some((config.workspace_id,key.trim().into()));}
    Ok(())
}
pub struct CloudStart {config:CloudAsrConfig,key:String,language:String,terms:String,initial:[Option<PreparedTask>;2]}
impl CloudStart {
    pub async fn prepare<R:Runtime>(app:&AppHandle<R>,state:&InterviewState,allowed:bool,microphone:bool)->Result<Option<Self>,String>{
        let index=state.index(app).await?;let config=index.cloud_asr_config().await?;
        if config.mode==AsrMode::Local{return Ok(None);}
        if !allowed{return Err("请勾选允许将本次声音发送至百炼，或在音频设置切回本地转写".into());}
        let prefs=index.preferences().await?;
        let mut prepared=Self{key:state.cloud_asr.key(&config).await,config,language:prefs.transcription_language,terms:prefs.terminology,initial:[None,None]};
        let cancel=CancellationToken::new();
        prepared.initial[0]=Some(prepared.connect(&cancel).await?);
        if microphone{prepared.initial[1]=Some(prepared.connect(&cancel).await?);}
        Ok(Some(prepared))
    }
    async fn connect(&self,cancel:&CancellationToken)->Result<PreparedTask,String>{
        // A failed startup can be retried once. Audio is admitted only after ready.
        let mut error=String::new();
        for _ in 0..2 {
            match PreparedTask::connect(&self.config,&self.key,uuid::Uuid::new_v4().to_string(),48000,&self.language,&self.terms,cancel).await{
                Ok(task)=>return Ok(task),Err(e)=>error=e,
            }
            if cancel.is_cancelled()||error.contains("401")||error.contains("403")||error.contains("Key"){break;}
        }
        Err(error)
    }
}
struct Frame{pcm:Vec<u8>,_lease:AudioLease}
impl AsRef<[u8]> for Frame{fn as_ref(&self)->&[u8]{&self.pcm}}
struct Channel{sender:mpsc::Sender<Frame>,cancel:CancellationToken,epoch:u64,microphone_epoch:Option<u64>}
static NEXT_UTTERANCE:AtomicU64=AtomicU64::new(1);

pub fn start<R:Runtime>(app:AppHandle<R>,mut input:AsrReceiver,state:Arc<RecordingState>,mut start:CloudStart)->tokio::task::JoinHandle<()> {
    tokio::spawn(async move{
        let mut guard=crate::audio::transcription::control::CancelOnDrop::new(state.transcription_control.clone());
        let cancel=CancellationToken::new();let drop_cancel=cancel.clone().drop_guard();
        let mut tasks=tokio::task::JoinSet::<Result<(),String>>::new();let mut channels:[Option<Channel>;2]=[None,None];
        let mut ticker=tokio::time::interval(Duration::from_millis(30));
        loop{
            tokio::select!{biased;
                _=state.transcription_control.cancelled()=>break,
                completed=tasks.join_next(),if !tasks.is_empty()=>{
                    if let Some(result)=completed{if let Err(error)=result.unwrap_or_else(|_|Err("云转写任务异常退出".into())){
                        fail(&app,&state,&error);break;
                    }}
                },
                _=ticker.tick()=>{
                    for (i,slot) in channels.iter_mut().enumerate(){
                        let stale=slot.as_ref().is_some_and(|c|c.epoch!=state.cloud_epoch()||(i==1&&c.microphone_epoch!=Some(state.microphone_gate().epoch))||(i==1&&!state.microphone_gate().enabled()));
                        if stale||state.is_paused(){if let Some(channel)=slot.take(){channel.cancel.cancel();}start.initial[i]=None;}
                    }
                },
                queued=input.recv()=>{
                    let Some(queued)=queued else{break};let chunk=queued.chunk;
                    if state.is_paused()||!crate::audio::microphone_gate::allowed(&chunk.microphone_gate)||chunk.speech_boundary.as_ref().map(|b|b.utterance_id)!=Some(state.cloud_epoch()){continue;}
                    if chunk.sample_rate!=48000||chunk.data.len()>48000||chunk.data.iter().any(|s|!s.is_finite()){fail(&app,&state,"云转写收到不支持的音频帧");break;}
                    let i=usize::from(chunk.device_type==DeviceType::Microphone);let epoch=state.cloud_epoch();let mic_epoch=chunk.microphone_gate.as_ref().map(|g|g.epoch);
                    if channels[i].as_ref().is_some_and(|c|c.epoch!=epoch||c.microphone_epoch!=mic_epoch){channels[i].take().unwrap().cancel.cancel();}
                    if channels[i].is_none(){
                        let (sender,receiver)=mpsc::channel(128);let child=cancel.child_token();
                        let config=CloudStart{config:start.config.clone(),key:start.key.clone(),language:start.language.clone(),terms:start.terms.clone(),initial:[None,None]};
                        let prepared=start.initial[i].take();let application=app.clone();let capture=state.clone();let token=child.clone();let gate=chunk.microphone_gate.clone();
                        let offset=(chunk.timestamp-chunk.data.len() as f64/48000.0).max(0.0);
                        tasks.spawn(async move{run_channel(application,capture,config,prepared,receiver,token,gate,i==0,epoch,offset).await});
                        channels[i]=Some(Channel{sender,cancel:child,epoch,microphone_epoch:mic_epoch});
                    }
                    let pcm=chunk.data.iter().flat_map(|s|((s.clamp(-1.0,1.0)*32767.0).round() as i16).to_le_bytes()).collect();
                    if channels[i].as_ref().unwrap().sender.try_send(Frame{pcm,_lease:queued.lease}).is_err(){fail(&app,&state,"云转写发送积压，已停止采集；可停止后切换本地模型");break;}
                }
            }
        }
        drop(channels); // Closed input sends finish-task and waits for the final text.
        if state.transcription_control.is_cancelled(){cancel.cancel();}
        while !tasks.is_empty(){
            tokio::select!{biased;
                _=state.transcription_control.cancelled(),if !cancel.is_cancelled()=>cancel.cancel(),
                result=tasks.join_next()=>{
                    if let Some(result)=result{if let Err(error)=result.unwrap_or_else(|_|Err("云转写任务异常退出".into())){if !cancel.is_cancelled(){fail(&app,&state,&error);cancel.cancel();}}}
                }
            }
        }
        drop(drop_cancel);guard.disarm();
    })
}
fn fail<R:Runtime>(app:&AppHandle<R>,state:&RecordingState,error:&str){
    state.stop_recording();state.transcription_control.cancel();
    // Transport errors are fixed local messages or sanitized provider codes;
    // never retain the provider payload, credentials or audio in diagnostics.
    log::warn!("Cloud ASR stopped: {}",error);
    let _=app.emit("interview-audio-fault",state.health.record_fault_detail("cloud_asr_failed",Some(error)));
    let _=app.emit("transcription-error",json!({"native_session_id":state.health.session_id,"error":error,"userMessage":format!("{error}。已保留文字，请停止后重试或切换本地转写。"),"actionable":false,"phase":"active"}));
}
struct ChannelOutput<R:Runtime>{
    app:AppHandle<R>,state:Arc<RecordingState>,cancel:CancellationToken,gate:Option<Arc<MicrophoneGate>>,source:&'static str,epoch:u64,offset:f64,ids:HashMap<u64,u64>,unfinished:HashMap<u64,u64>,
}
impl<R:Runtime> ChannelOutput<R>{
    fn event(&mut self,event:AsrEvent){
        if self.cancel.is_cancelled()||self.epoch!=self.state.cloud_epoch()||!crate::audio::microphone_gate::allowed(&self.gate){return;}
        if let AsrEvent::Sentence{id,text,final_result,begin_ms,end_ms,billed_seconds}=event{
            let utterance=*self.ids.entry(id).or_insert_with(||NEXT_UTTERANCE.fetch_add(1,Ordering::Relaxed));self.unfinished.insert(id,utterance);
            let start=self.offset+begin_ms as f64/1000.0;let end=self.offset+end_ms.unwrap_or(begin_ms) as f64/1000.0;
            if !text.trim().is_empty(){
                let update=TranscriptUpdate{text,timestamp:chrono::Local::now().format("%H:%M:%S").to_string(),source:self.source.into(),sequence_id:crate::audio::transcription::worker::next_sequence(),chunk_start_time:start,is_partial:!final_result,confidence:None,audio_start_time:start,audio_end_time:end,duration:(end-start).max(0.0),utterance_id:Some(utterance),microphone_epoch:self.gate.as_ref().map(|g|g.epoch),replaces_utterance:true,native_session_id:Some(self.state.health.session_id)};
                let _=self.app.emit("transcript-update",update);
            }
            if final_result{
                self.unfinished.remove(&id);
                // A provider timestamp ahead of the capture clock is unknown,
                // not a measured zero-millisecond response.
                let latency=self.state.get_recording_duration().filter(|now|now.is_finite()&&*now>=end).map(|now|(now-end)*1000.0);
                let _=self.app.emit("interview-utterance-end",json!({"source":self.source,"native_session_id":self.state.health.session_id,"utterance_id":utterance,"success":true,"timing":{"stable_text_unix_ms":chrono::Utc::now().timestamp_millis(),"audio_end_basis":"provider_estimate","audio_end_to_stable_ms":latency}}));
                let _=self.app.emit("interview-cloud-asr-usage",json!({"source":self.source,"native_session_id":self.state.health.session_id,"task_billed_seconds":billed_seconds}));
            }
        }
    }
    fn finish(&mut self){
        for utterance in self.unfinished.values(){let _=self.app.emit("interview-utterance-end",json!({"source":self.source,"native_session_id":self.state.health.session_id,"utterance_id":utterance,"success":false}));}
        self.unfinished.clear();
    }
}
async fn run_channel<R:Runtime>(app:AppHandle<R>,state:Arc<RecordingState>,config:CloudStart,prepared:Option<PreparedTask>,input:mpsc::Receiver<Frame>,cancel:CancellationToken,gate:Option<Arc<MicrophoneGate>>,system:bool,epoch:u64,offset:f64)->Result<(),String>{
    let mut output=ChannelOutput{app,state:state.clone(),cancel:cancel.clone(),gate:gate.clone(),source:if system{"System"}else{"Microphone"},epoch,offset,ids:HashMap::new(),unfinished:HashMap::new()};
    let result=async{
        let task=match prepared{Some(task)=>task,None=>config.connect(&cancel).await?};
        task.stream(input,cancel.clone(),|event|output.event(event)).await
    }.await;
    output.finish();
    if cancel.is_cancelled()||epoch!=state.cloud_epoch()||!crate::audio::microphone_gate::allowed(&gate){Ok(())}else{result}
}
#[cfg(test)]
mod tests{
    use super::*;use tauri::Listener;
    #[test]
    fn failure_detail_survives_stop_and_is_scoped_to_the_native_session(){
        let app=tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let observed=Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let events=observed.clone();app.listen("transcription-error",move|e|events.lock().unwrap().push(serde_json::from_str(e.payload()).unwrap()));
        let state=RecordingState::new();state.start_recording().unwrap();
        fail(app.handle(),&state,"云转写连接无响应，已暂停该通道");
        let snapshot=state.health.snapshot(true,false,false,true,false);
        assert_eq!(snapshot.fault.unwrap().detail.as_deref(),Some("云转写连接无响应，已暂停该通道"));
        assert_eq!(snapshot.restart_reason.as_deref(),Some("cloud_asr_failed"));
        assert!(state.transcription_control.is_cancelled());
        assert_eq!(observed.lock().unwrap()[0]["native_session_id"],state.health.session_id);
    }
    #[test]
    fn cloud_sources_partials_mute_pause_and_completion_stay_isolated(){
        let app=tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let observed=Arc::new(std::sync::Mutex::new(Vec::<(String,Value)>::new()));
        for name in ["transcript-update","interview-utterance-end"]{let events=observed.clone();app.listen(name,move|e|events.lock().unwrap().push((name.into(),serde_json::from_str(e.payload()).unwrap())));}
        let state=RecordingState::new();state.start_recording().unwrap();state.enable_cloud_streaming();
        let make=|source,gate|ChannelOutput{app:app.handle().clone(),state:state.clone(),cancel:CancellationToken::new(),gate,source,epoch:state.cloud_epoch(),offset:0.0,ids:HashMap::new(),unfinished:HashMap::new()};
        let mut remote=make("System",None);let gate=state.microphone_gate();let mut own=make("Microphone",Some(gate.clone()));
        let event=|final_result|AsrEvent::Sentence{id:1,text:"synthetic speech".into(),final_result,begin_ms:0,end_ms:if final_result{Some(800)}else{None},billed_seconds:None};
        remote.event(event(false));own.event(event(false));
        {let events=observed.lock().unwrap();assert_eq!(events.len(),2);assert!(events.iter().all(|(n,v)|n=="transcript-update"&&v["is_partial"]==true));assert_ne!(events[0].1["utterance_id"],events[1].1["utterance_id"]);}
        gate.revoke();own.event(event(true));assert_eq!(observed.lock().unwrap().len(),2,"muted microphone emitted late final");own.finish();
        remote.event(event(true));remote.finish();
        {let events=observed.lock().unwrap();assert_eq!(events.iter().filter(|(n,v)|n=="interview-utterance-end"&&v["success"]==true).count(),1);assert!(events.iter().any(|(n,v)|n=="interview-utterance-end"&&v["source"]=="Microphone"&&v["success"]==false));}
        let before=observed.lock().unwrap().len();state.pause_recording().unwrap();state.resume_recording().unwrap();remote.event(event(true));assert_eq!(observed.lock().unwrap().len(),before,"pre-pause socket emitted into resumed capture");
        let mut fresh=make("System",None);fresh.cancel.cancel();fresh.event(event(true));assert_eq!(observed.lock().unwrap().len(),before,"cancelled task emitted a final");
    }
}
