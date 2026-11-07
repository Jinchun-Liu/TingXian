//! Main-window owned local memory. Audio capture never holds these locks and
//! network work runs outside them. Display windows cannot configure or call it.
use crate::interview::InterviewState;
use interview_core::{compaction::{self,Config,Scheduler},memory_checkpoint::{self as checkpoint,Reference},memory_store,session_memory::*,Index};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use sqlx::SqlitePool;
use std::{collections::BTreeMap,sync::Mutex as StdMutex};
use tauri::{Emitter,Manager,State,WebviewWindow};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handle {pub id:String,pub epoch:u64,pub generation:String}
struct Owned {handle:Handle,ledger:Ledger,scheduler:Scheduler,checkpoint:Option<Reference>,closed_through:u64,error:Option<String>,persistence_failed:bool}
#[derive(Default)]
pub(crate) struct Work {inner:Mutex<Option<Owned>>,cancel:StdMutex<Option<(String,CancellationToken,String)>>,pressure:StdMutex<interview_core::context_pressure::Tracker>,probe:crate::interview_connection::ConnectionState}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawInput {
    pub id:String,pub expected_revision:u64,pub source:String,pub text:String,
    pub capture_session:Option<String>,pub utterance_id:Option<String>,pub closed:bool,
    #[serde(default)]pub topic_closed:bool,
}
fn owner<R:tauri::Runtime>(window:&WebviewWindow<R>)->Result<(),String>{if window.label()=="main"{Ok(())}else{Err("请在主窗口管理会话记忆".into())}}
fn current<'a>(slot:&'a mut Option<Owned>,handle:&Handle)->Result<&'a mut Owned,String>{slot.as_mut().filter(|s|s.handle==*handle).ok_or("会话记忆已切换，旧操作未写入".into())}
async fn dependencies(index:&Index)->Result<Dependencies,String>{
    let (version,facts,models,preferences)=tokio::try_join!(index.setting("version"),index.fact_card(),index.provider_config(),index.setting("interview_preferences"))?;
    Ok(Dependencies{source_policy:interview_core::digest(json!({"version":version,"validity_day":chrono::Local::now().date_naive().to_string()}).to_string().as_bytes()),facts:interview_core::digest(serde_json::to_string(&facts).map_err(|e|e.to_string())?.as_bytes()),model:interview_core::digest(serde_json::to_string(&models).map_err(|e|e.to_string())?.as_bytes()),policy:interview_core::digest(format!("session-memory-v1:legacy:{preferences}").as_bytes())})
}
fn source_binding(d:&Dependencies)->BTreeMap<String,String>{BTreeMap::from([("request-sources".into(),interview_core::digest(serde_json::to_string(d).unwrap().as_bytes()))])}
impl Work {
    pub(crate) async fn has_session(&self)->bool {self.inner.lock().await.is_some()}
    pub(crate) fn cancel(&self){self.probe.cancel(None);if let Some((_,c,_))=&*self.cancel.lock().unwrap_or_else(|e|e.into_inner()){c.cancel();}}
    /// Keys are not quota boundaries. Requests to the same service host yield
    /// background work immediately; a separate service may continue in parallel.
    pub(crate) fn preempt(&self,endpoint:&str){if let Some((_,c,host))=&*self.cancel.lock().unwrap_or_else(|e|e.into_inner()){let next=service_host(endpoint);if host.is_empty()||next.is_empty()||*host==next{c.cancel();}}}
    pub(crate) async fn reset(&self){self.cancel();*self.inner.lock().await=None;*self.pressure.lock().unwrap_or_else(|e|e.into_inner())=Default::default();}
    pub(crate) fn observe_request(&self,m:&interview_core::request_budget::RequestMeasurement,active:usize){self.pressure.lock().unwrap_or_else(|e|e.into_inner()).observe(m,chrono::Utc::now().timestamp_millis().max(0) as u64,active);}
    async fn open(&self,pool:&SqlitePool,index:&Index,resume_id:Option<String>,missing_older_history:bool)->Result<Handle,String>{
        let mut slot=self.inner.lock().await;
        let deps=dependencies(index).await?;let id=resume_id.unwrap_or_else(||uuid::Uuid::new_v4().to_string());
        let session=memory_store::open(pool,&id,deps.clone(),missing_older_history).await?;
        let reference=checkpoint::latest(pool,&id).await?;
        let (mut ledger,scheduler)=if let Some(reference)=&reference {
            let(mut ledger,scheduler)=checkpoint::load(pool,reference).await?;let mut after=reference.last_record;
            while after<session.last_record {let rows=memory_store::page(pool,&id,session.epoch,after,64).await?;for r in rows {after=r.record_sequence;ledger.append(r.event)?;}}
            (ledger,scheduler)
        }else{(checkpoint::load_journal(pool,&session).await?,Scheduler::default())};
        ledger.update_dependencies(deps)?;
        let handle=Handle{id,epoch:session.epoch,generation:uuid::Uuid::new_v4().to_string()};
        *self.pressure.lock().unwrap_or_else(|e|e.into_inner())=Default::default();
        *slot=Some(Owned{handle:handle.clone(),ledger,scheduler,checkpoint:reference,closed_through:0,error:None,persistence_failed:false});Ok(handle)
    }
    async fn record(&self,pool:&SqlitePool,handle:&Handle,input:RawInput)->Result<Value,String>{
        let mut slot=self.inner.lock().await;let own=current(&mut slot,handle)?;
        let (kind,origin)=match input.source.as_str(){"System"=>(EventKind::RemoteSpeech,Origin::SystemAudio),"Microphone"=>(EventKind::SelfSpeech,Origin::Microphone),"Confirmed"=>(EventKind::ConfirmedSelf,Origin::UserText),_=>return Err("只接受实际对方/本人发言，不能从展示数据写入 AI 事实".into())};
        let old=own.ledger.event(&input.id);
        if input.expected_revision>20_000||!old.map_or(input.expected_revision==0,|e|e.revision==input.expected_revision||e.revision==input.expected_revision+1){return Err("发言已有新修订，旧输入未覆盖".into());}
        let event=Event{id:input.id,session_id:handle.id.clone(),epoch:handle.epoch,sequence:old.map_or_else(||own.ledger.next_sequence(),|e|e.sequence),revision:input.expected_revision+1,kind,origin,capture_session:input.capture_session,utterance_id:input.utterance_id,replaces_revision:(input.expected_revision>0).then_some(input.expected_revision),text:input.text,closed:input.closed,terminal:None,question:None,source_dependencies:BTreeMap::new(),asset_sha256:None};
        let record=own.ledger.plan_append(&event)?;
        if let Err(e)=memory_store::append(pool,&record).await {own.error=Some(e.clone());own.persistence_failed=true;return Err(format!("会话原文未保存：{e}；请保留窗口并暂停采集"));}
        own.ledger.append(event.clone())?;if input.topic_closed&&event.closed {own.closed_through=own.closed_through.max(event.sequence);}
        Ok(json!({"record_sequence":record.record_sequence,"event":event.reference()?,"persisted":true}))
    }
    async fn save(&self,pool:&SqlitePool,handle:&Handle)->Result<Reference,String>{
        let mut slot=self.inner.lock().await;let own=current(&mut slot,handle)?;
        if let Some(r)=&own.checkpoint {if r.last_record==own.ledger.journal().len() as u64 {let (ledger,scheduler)=checkpoint::load(pool,r).await?;if serde_json::to_value(checkpoint::Checkpoint::capture(&ledger,&scheduler)).ok()==serde_json::to_value(checkpoint::Checkpoint::capture(&own.ledger,&own.scheduler)).ok(){return Ok(r.clone());}}}
        let reference=checkpoint::save(pool,&checkpoint::Checkpoint::capture(&own.ledger,&own.scheduler),own.checkpoint.as_ref().map_or(0,|r|r.revision)).await?;
        own.checkpoint=Some(reference.clone());own.error=None;own.persistence_failed=false;Ok(reference)
    }
    async fn restore(&self,pool:&SqlitePool,index:&Index,reference:&checkpoint::Boundary)->Result<Handle,String>{
        let mut slot=self.inner.lock().await;let id=uuid::Uuid::new_v4().to_string();
        let (ledger,scheduler,reference)=checkpoint::fork_boundary(pool,reference,&id,dependencies(index).await?).await?;
        let handle=Handle{id,epoch:reference.epoch,generation:uuid::Uuid::new_v4().to_string()};
        *self.pressure.lock().unwrap_or_else(|e|e.into_inner())=Default::default();
        *slot=Some(Owned{handle:handle.clone(),ledger,scheduler,checkpoint:Some(reference),closed_through:0,error:None,persistence_failed:false});Ok(handle)
    }
    pub(crate) async fn request(&self,pool:&SqlitePool,index:&Index,question_id:&str,revision:u32,question:&str,screenshot_sha256:Option<String>)->Result<Option<(Handle,QuestionVersion,Workset,BTreeMap<String,String>)>,String>{
        if self.inner.lock().await.is_none(){return Ok(None);}
        let deps=dependencies(index).await?;let terms=index.tokens(question);
        let mut slot=self.inner.lock().await;let Some(own)=slot.as_mut()else{return Ok(None)};
        own.ledger.update_dependencies(deps.clone())?;let binding=source_binding(&deps);
        let pin=own.ledger.pin(question_id,u64::from(revision))?;
        let event=Event{id:format!("question-{question_id}-{revision}"),session_id:own.handle.id.clone(),epoch:own.handle.epoch,sequence:own.ledger.next_sequence(),revision:1,kind:EventKind::RequestQuestion,origin:Origin::Runtime,capture_session:None,utterance_id:None,replaces_revision:None,text:question.into(),closed:false,terminal:None,question:Some(pin.clone()),source_dependencies:BTreeMap::new(),asset_sha256:None};
        let record=own.ledger.plan_append(&event)?;
        if let Err(e)=memory_store::append(pool,&record).await{own.error=Some(e.clone());own.persistence_failed=true;return Err(e);}
        own.ledger.append(event)?;
        if let Some(hash)=screenshot_sha256 {
            // Keep an exact reference and the user-provided conditions, not
            // image bytes or invented OCR. The existing capture remains ephemeral.
            let event=Event{id:format!("screen-{question_id}-{revision}"),session_id:own.handle.id.clone(),epoch:own.handle.epoch,sequence:own.ledger.next_sequence(),revision:1,kind:EventKind::Screenshot,origin:Origin::Screen,capture_session:None,utterance_id:None,replaces_revision:None,text:format!("截图问题原文：{question}\n截图资产 SHA-256：{hash}。图像未持久化；此引用不代表图片内容，涉及未记录的图中条件时需重新截屏。"),closed:true,terminal:None,question:Some(pin.clone()),source_dependencies:BTreeMap::new(),asset_sha256:Some(hash)};
            let record=own.ledger.plan_append(&event)?;
            if let Err(e)=memory_store::append(pool,&record).await{own.error=Some(e.clone());own.persistence_failed=true;return Err(e);}
            own.ledger.append(event)?;
        }
        Ok(Some((own.handle.clone(),pin,own.ledger.workset_with_sources(&terms,&binding),binding)))
    }
    pub(crate) async fn finish_request(&self,pool:&SqlitePool,handle:&Handle,pin:&QuestionVersion,text:&str,status:Terminal,binding:&BTreeMap<String,String>)->Result<(),String>{
        let mut slot=self.inner.lock().await;let own=current(&mut slot,handle)?;
        let (answer,question)=text.split_once("<<<QUESTION>>>").map_or((text,None),|(a,q)|(a,Some(q)));
        let mut parts=vec![];
        if !answer.trim().is_empty(){parts.push(("advice",EventKind::Advice,Origin::Model,answer.trim().to_string()));}
        if status==Terminal::Complete {if let Some(question)=question.filter(|q|!q.trim().is_empty()){parts.push(("normalized",EventKind::NormalizedQuestion,Origin::Model,question.trim().to_string()));}}
        parts.push(("terminal",EventKind::RequestTerminal,Origin::Runtime,serde_json::to_string(&status).unwrap()));
        for (label,kind,origin,text) in parts {
            let event=Event{id:format!("{label}-{}-{}",pin.question_id,pin.revision),session_id:handle.id.clone(),epoch:handle.epoch,sequence:own.ledger.next_sequence(),revision:1,kind,origin,capture_session:None,utterance_id:None,replaces_revision:None,text,closed:true,terminal:Some(status.clone()),question:Some(pin.clone()),source_dependencies:binding.clone(),asset_sha256:None};
            let record=own.ledger.plan_append(&event)?;
            if let Err(e)=memory_store::append(pool,&record).await {own.error=Some(e.clone());own.persistence_failed=true;return Err(e);}
            own.ledger.append(event)?;
        }
        if let Some(question)=own.ledger.event(&format!("question-{}-{}",pin.question_id,pin.revision)).cloned(){
            let mut done=question;done.revision+=1;done.replaces_revision=Some(done.revision-1);done.closed=true;done.terminal=Some(status.clone());
            let record=own.ledger.plan_append(&done)?;
            if let Err(e)=memory_store::append(pool,&record).await {own.error=Some(e.clone());own.persistence_failed=true;return Err(e);}
            own.ledger.append(done)?;
        }
        if status==Terminal::Complete {own.closed_through=own.ledger.next_sequence()-1;}
        Ok(())
    }
    async fn status(&self)->Value {
        let slot=self.inner.lock().await;match slot.as_ref(){None=>json!({"active":false}),Some(o)=>json!({"active":true,"pressure":self.pressure.lock().unwrap_or_else(|e|e.into_inner()).view(),"handle":o.handle,"context_version":o.ledger.version(),"records":o.ledger.journal().len(),"covered_through":o.ledger.chunks().iter().filter(|c|c.valid).map(|c|c.snapshot.cutoff_sequence).max().unwrap_or(0),"missing_older_history":o.ledger.missing_older_history(),"running":o.scheduler.active(),"attempts":o.scheduler.attempts().len(),"accounted_units":o.scheduler.charged_units(),"last_attempt":o.scheduler.attempts().last().map(|a|json!({"status":a.status,"trigger":a.trigger,"request_sent":a.request_sent,"usage":a.usage,"measurement":a.measurement})),"error":o.error,"persistence_failed":o.persistence_failed,"checkpoint":o.checkpoint})}
    }
}

#[tauri::command]
pub async fn interview_memory_config_get<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>)->Result<Value,String>{owner(&window)?;let config=state.index(window.app_handle()).await?.compaction_config().await?;Ok(json!({"key_configured":!state.provider_key(&config.profile).await.is_empty(),"config":config,"probe":state.memory.probe.snapshot()}))}
#[tauri::command]
pub async fn interview_memory_test<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>)->Result<interview_core::ProbeOutcome,String>{
    owner(&window)?;crate::interview_exit::ensure_not_exiting(window.app_handle())?;
    let _lifecycle=state.lifecycle.try_lock().map_err(|_|"请等待当前操作完成")?;
    if state.session.load(std::sync::atomic::Ordering::SeqCst)||crate::audio::recording_commands::is_recording().await{return Err("请结束会话后测试压缩接口".into());}
    let network=state.source_network();let _network=network.try_write().map_err(|_|"请求正在结束")?;
    let config=state.index(window.app_handle()).await?.compaction_config().await?;let key=state.provider_key(&config.profile).await;
    if key.is_empty()&&!config.profile.accepts_empty_key(){return Err("请先保存独立压缩接口的 API Key".into());}
    let check_id=uuid::Uuid::new_v4().to_string();let cancel=state.memory.probe.begin(&check_id,&config.profile.request.model)?;
    let hash=interview_core::digest(b"synthetic-connection-check");
    let snapshot=Snapshot{schema_version:1,session_id:"connection-check".into(),epoch:1,base_context_version:0,dependencies:Dependencies{source_policy:hash.clone(),facts:hash.clone(),model:hash.clone(),policy:hash},from_sequence:1,cutoff_sequence:1,events:vec![Event{id:"fixed-test".into(),session_id:"connection-check".into(),epoch:1,sequence:1,revision:1,kind:EventKind::RemoteSpeech,origin:Origin::SystemAudio,capture_session:Some("synthetic".into()),utterance_id:Some("1".into()),replaces_revision:None,text:"合成连接测试：null 不等于数字零。".into(),closed:true,terminal:None,question:None,source_dependencies:BTreeMap::new(),asset_sha256:None}]};
    let started=std::time::Instant::now();let outcome=compaction::run(&config.profile,&key,&snapshot,cancel).await;
    let result=if let Some(error)=outcome.error{Err(error)}else{outcome.candidate.as_ref().ok_or("未返回完整候选".to_string()).and_then(|c|validate_candidate(&snapshot,c))};
    let report=interview_core::ProbeOutcome{success:result.is_ok(),message:interview_core::redact_credentials(&result.err().unwrap_or_else(||"固定文字连接通过；未验证长会话质量、完整窗口或费用收益".into()),Some(&key)),elapsed_ms:started.elapsed().as_millis() as u64,attempts:u8::from(outcome.transport_started),total_tokens:outcome.usage.map(|u|u.total())};
    state.memory.probe.finish(&check_id,report.clone())?;Ok(report)
}
#[tauri::command]
pub async fn interview_memory_config_save<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,config:Config,key:Option<String>)->Result<(),String>{
    owner(&window)?;config.validate()?;crate::interview_exit::ensure_not_exiting(window.app_handle())?;
    if key.as_ref().is_some_and(|k|k.len()>4096||k.chars().any(char::is_control)){return Err("接口凭证格式无效".into());}
    state.memory.cancel();let _lifecycle=state.lifecycle.lock().await;
    if state.session.load(std::sync::atomic::Ordering::SeqCst)||crate::audio::recording_commands::is_recording().await {return Err("请结束当前会话后修改压缩接口".into());}
    let network=state.source_network();let _network=tokio::time::timeout(std::time::Duration::from_secs(35),network.write()).await.map_err(|_|"压缩请求仍在收尾，尚未保存配置")?;
    state.index(window.app_handle()).await?.set_compaction_config(&config).await?;
    state.store_provider_key(&config.profile,key).await;state.memory.probe.invalidate();Ok(())
}
#[tauri::command]
pub async fn interview_memory_open<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,resume_id:Option<String>,missing_older_history:bool)->Result<Option<Handle>,String>{
    owner(&window)?;crate::interview_exit::ensure_not_exiting(window.app_handle())?;
    state.memory.cancel();let _lifecycle=state.lifecycle.lock().await;
    if state.session.load(std::sync::atomic::Ordering::SeqCst)||crate::audio::recording_commands::is_recording().await{return Err("请结束当前会话后切换会话记忆".into());}
    let network=state.source_network();let _network=tokio::time::timeout(std::time::Duration::from_secs(35),network.write()).await.map_err(|_|"上一会话请求仍在结束")?;
    let index=state.index(window.app_handle()).await?;let config=index.compaction_config().await?;
    if !config.enabled&&resume_id.is_none(){state.memory.reset().await;return Ok(None);}
    let db=window.app_handle().state::<crate::state::AppState>();state.memory.open(db.db_manager.pool(),index,resume_id,missing_older_history).await.map(Some)
}
#[tauri::command]
pub async fn interview_memory_append<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,handle:Handle,input:RawInput)->Result<Value,String>{
    owner(&window)?; // Existing queued raw writes may drain during the exit checkpoint.
    let db=window.app_handle().state::<crate::state::AppState>();state.memory.record(db.db_manager.pool(),&handle,input).await
}
#[tauri::command]
pub async fn interview_memory_checkpoint<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,handle:Handle)->Result<Reference,String>{
    owner(&window)?;let db=window.app_handle().state::<crate::state::AppState>();state.memory.save(db.db_manager.pool(),&handle).await
}
#[tauri::command]
pub async fn interview_memory_status<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>)->Result<Value,String>{owner(&window)?;Ok(state.memory.status().await)}

#[tauri::command]
pub async fn interview_memory_restore<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,reference:checkpoint::Boundary)->Result<Handle,String>{
    owner(&window)?;crate::interview_exit::ensure_not_exiting(window.app_handle())?;state.memory.cancel();let _lifecycle=state.lifecycle.lock().await;
    if state.session.load(std::sync::atomic::Ordering::SeqCst)||crate::audio::recording_commands::is_recording().await{return Err("请结束当前会话后恢复会话记忆".into());}
    let network=state.source_network();let _network=tokio::time::timeout(std::time::Duration::from_secs(35),network.write()).await.map_err(|_|"旧请求仍在结束")?;
    let db=window.app_handle().state::<crate::state::AppState>();state.memory.restore(db.db_manager.pool(),state.index(window.app_handle()).await?,&reference).await
}

#[tauri::command]
pub async fn interview_memory_boundary<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,handle:Handle)->Result<checkpoint::Boundary,String>{owner(&window)?;let mut slot=state.memory.inner.lock().await;let own=current(&mut slot,&handle)?;if own.persistence_failed{return Err("原文尚未完整保存，不能声明记忆边界".into());}checkpoint::Boundary::capture(&own.ledger,own.checkpoint.clone())}

#[tauri::command]
pub async fn interview_memory_raw_get<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,handle:Handle,id:String)->Result<RawInput,String>{owner(&window)?;let mut slot=state.memory.inner.lock().await;let own=current(&mut slot,&handle)?;let e=own.ledger.event(&id).ok_or("这条原文不存在")?;let source=match e.kind{EventKind::RemoteSpeech=>"System",EventKind::SelfSpeech=>"Microphone",EventKind::ConfirmedSelf=>"Confirmed",_=>return Err("AI 派生信息不能改成本人原文".into())};Ok(RawInput{id:e.id.clone(),expected_revision:e.revision,source:source.into(),text:e.text.clone(),capture_session:e.capture_session.clone(),utterance_id:e.utterance_id.clone(),closed:e.closed,topic_closed:e.closed})}

#[tauri::command]
pub async fn interview_memory_tick<R:tauri::Runtime>(window:WebviewWindow<R>,state:State<'_,InterviewState>,handle:Handle,send_allowed:bool)->Result<bool,String>{
    owner(&window)?;crate::interview_exit::ensure_not_exiting(window.app_handle())?;
    if !send_allowed||!state.answers_idle(){return Ok(false);}
    let Ok(_lifecycle)=state.lifecycle.try_lock() else{return Ok(false)};
    let network=match state.source_network().try_read_owned(){Ok(n)=>n,Err(_)=>return Ok(false)};
    let index=state.index(window.app_handle()).await?.clone();let config=index.compaction_config().await?;
    if !config.enabled{return Ok(false);}
    let key=state.provider_key(&config.profile).await;
    if key.is_empty()&&!config.profile.accepts_empty_key(){return Ok(false);}
    let deps=dependencies(&index).await?;let app=window.app_handle().clone();let pool=app.state::<crate::state::AppState>().db_manager.pool().clone();
    let mut slot=state.memory.inner.lock().await;let own=current(&mut slot,&handle)?;
    if own.persistence_failed{return Ok(false);}
    own.ledger.update_dependencies(deps)?;
    let trigger=if state.memory.pressure.lock().unwrap_or_else(|e|e.into_inner()).view().active{compaction::Trigger::Capacity}else{compaction::Trigger::Coverage};
    let attempt=match own.scheduler.begin_for(&own.ledger,&config,true,!state.answers_idle(),own.closed_through,chrono::Utc::now().timestamp_millis().max(0) as u64,trigger){Ok(Some(a))=>a,Ok(None)=>return Ok(false),Err(e)=>{own.error=Some(e);return Ok(false)}};
    own.scheduler.mark_sent(&attempt.id)?;
    let persisted=checkpoint::save(&pool,&checkpoint::Checkpoint::capture(&own.ledger,&own.scheduler),own.checkpoint.as_ref().map_or(0,|r|r.revision)).await;
        match persisted {Ok(r)=>own.checkpoint=Some(r),Err(e)=>{own.scheduler.reject_unsent(&attempt.id,&e);own.persistence_failed=true;own.error=Some(e.clone());return Err(e);}}
    let cancel=CancellationToken::new();*state.memory.cancel.lock().unwrap_or_else(|e|e.into_inner())=Some((attempt.id.clone(),cancel.clone(),service_host(&config.profile.endpoint)));
    drop(slot);drop(_lifecycle);
    tauri::async_runtime::spawn(async move{
        let started=std::time::Instant::now();
        let mut outcome=compaction::run(&config.profile,&key,&attempt.snapshot,cancel.clone()).await;
        if cancel.is_cancelled(){outcome.candidate=None;outcome.error=Some("后台整理已取消，保留原上下文".into());outcome.cancelled=true;}
        let fresh_config=index.compaction_config().await;let fresh_dependencies=dependencies(&index).await;
        let state=app.state::<InterviewState>();let mut slot=state.memory.inner.lock().await;
        if let Ok(own)=current(&mut slot,&handle){
            if outcome.candidate.is_some()&&!outcome.cancelled{state.memory.pressure.lock().unwrap_or_else(|e|e.into_inner()).completed(started.elapsed().as_millis() as u64);}
            let mut ledger=own.ledger.clone();let mut scheduler=own.scheduler.clone();
            scheduler.record_outcome_accounting(&attempt.id,&outcome);
            let result=async {
                ledger.update_dependencies(fresh_dependencies?)?;
                scheduler.finish(&attempt.id,&mut ledger,&fresh_config?,outcome)?;
                checkpoint::save(&pool,&checkpoint::Checkpoint::capture(&ledger,&scheduler),own.checkpoint.as_ref().map_or(0,|r|r.revision)).await
            }.await;
            match result {Ok(reference)=>{own.ledger=ledger;own.scheduler=scheduler;own.checkpoint=Some(reference);own.error=own.scheduler.attempts().last().and_then(|a|a.error.clone());},Err(e)=>{scheduler.reject_commit(&attempt.id,&e);own.scheduler=scheduler;own.error=Some(format!("会话记忆未保存：{e}"));own.persistence_failed=true;}}
        }
        drop(slot);
        let mut active=state.memory.cancel.lock().unwrap_or_else(|e|e.into_inner());if active.as_ref().is_some_and(|(id,_,_)|id==&attempt.id){*active=None;}drop(active);
        drop(network);let _=app.emit_to("main","interview-memory-status-changed",());
    });Ok(true)
}
fn service_host(endpoint:&str)->String {reqwest::Url::parse(endpoint).ok().and_then(|u|u.host_str().map(|s|s.to_lowercase())).unwrap_or_default()}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    type Runtime=tauri::test::MockRuntime;
    fn app()->(tauri::App<Runtime>,WebviewWindow<Runtime>,WebviewWindow<Runtime>){
        let root=std::env::temp_dir().join(format!("tingxian-memory-ipc-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let(index,db)=tauri::async_runtime::block_on(async{(Index::open(&root.join("profile.sqlite")).await.unwrap(),crate::database::manager::DatabaseManager::new(root.join("sessions.sqlite").to_str().unwrap(),root.join("no-legacy.sqlite").to_str().unwrap()).await.unwrap())});
        let app=tauri::test::mock_builder().manage(InterviewState::with_test_index(index)).manage(crate::state::AppState{db_manager:db}).manage(crate::interview_exit::ExitState::default())
            .invoke_handler(tauri::generate_handler![interview_memory_config_get,interview_memory_config_save,interview_memory_open,interview_memory_append,interview_memory_checkpoint,interview_memory_status,interview_memory_tick,interview_memory_restore,interview_memory_boundary,interview_memory_raw_get,interview_memory_test]).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let main=tauri::WebviewWindowBuilder::new(&app,"main",Default::default()).build().unwrap();let display=tauri::WebviewWindowBuilder::new(&app,"display",Default::default()).build().unwrap();(app,main,display)
    }
    fn call(w:&WebviewWindow<Runtime>,cmd:&str,body:Value)->Result<Value,String>{tauri::test::get_ipc_response(w,tauri::webview::InvokeRequest{cmd:cmd.into(),callback:tauri::ipc::CallbackFn(0),error:tauri::ipc::CallbackFn(1),url:"tauri://localhost".parse().unwrap(),body:tauri::ipc::InvokeBody::Json(body),headers:Default::default(),invoke_key:tauri::test::INVOKE_KEY.to_string()}).map_err(|e|format!("{e:?}"))?.deserialize::<Value>().map_err(|e|e.to_string())}
    fn input(n:u64)->Value{json!({"id":format!("raw-{n}"),"expected_revision":0,"source":"Microphone","text":format!("第{n}条实际发言：HTTP 504 后状态未知，我只做过模拟。"),"capture_session":"fixture-capture","utterance_id":format!("utterance-{n}"),"closed":true,"topic_closed":true})}
    fn open(w:&WebviewWindow<Runtime>)->Handle{serde_json::from_value(call(w,"interview_memory_open",json!({"resumeId":null,"missingOlderHistory":false})).unwrap()).unwrap()}
    fn config(w:&WebviewWindow<Runtime>,endpoint:Option<String>){let mut c=Config{enabled:true,..Default::default()};if let Some(e)=endpoint{c.profile.endpoint=e;}call(w,"interview_memory_config_save",json!({"config":c,"key":"synthetic-credential-not-real"})).unwrap();}

    #[test]fn memory_ipc_is_main_only_default_off_and_credentials_stay_native(){
        let(app,main,display)=app();let raw=call(&main,"interview_memory_config_get",json!({})).unwrap();assert_eq!(raw["config"]["enabled"],false);
        assert!(call(&main,"interview_memory_open",json!({"resumeId":null,"missingOlderHistory":false})).unwrap().is_null());
        let pool=app.state::<crate::state::AppState>().db_manager.pool().clone();assert_eq!(tauri::async_runtime::block_on(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM sqlite_master WHERE name='tingxian_memory_sessions'").fetch_one(&pool)).unwrap(),0);
        config(&main,None);let h=open(&main);
        for(cmd,body)in[("interview_memory_config_get",json!({})),("interview_memory_config_save",json!({"config":Config::default(),"key":null})),("interview_memory_open",json!({"resumeId":null,"missingOlderHistory":false})),("interview_memory_append",json!({"handle":h,"input":input(1)})),("interview_memory_checkpoint",json!({"handle":h})),("interview_memory_status",json!({})),("interview_memory_tick",json!({"handle":h,"sendAllowed":true}))]{assert!(call(&display,cmd,body).is_err(),"display called {cmd}");}
        let shown=call(&main,"interview_memory_config_get",json!({})).unwrap();assert_eq!(shown["key_configured"],true);assert!(!shown.to_string().contains("synthetic-credential-not-real"));
        let state=app.state::<InterviewState>();let cfg=tauri::async_runtime::block_on(state.index(app.handle())).unwrap();assert_eq!(tauri::async_runtime::block_on(cfg.provider_config()).unwrap().answers.len(),3);
        let key_rows=tauri::async_runtime::block_on(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM settings WHERE value LIKE '%synthetic-credential-not-real%'").fetch_one(&cfg.pool)).unwrap();assert_eq!(key_rows,0);
        tauri::async_runtime::block_on(pool.close());
    }
    #[test]fn raw_ack_is_idempotent_failure_does_not_advance_and_old_generation_is_fenced(){
        let(app,main,_)=app();config(&main,None);let h=open(&main);let body=json!({"handle":h,"input":input(1)});let first=call(&main,"interview_memory_append",body.clone()).unwrap();assert_eq!(first["persisted"],true);assert_eq!(call(&main,"interview_memory_append",body).unwrap(),first);
        let pool=app.state::<crate::state::AppState>().db_manager.pool().clone();
        tauri::async_runtime::block_on(sqlx::query("CREATE TRIGGER synthetic_full BEFORE INSERT ON tingxian_memory_events BEGIN SELECT RAISE(ABORT,'synthetic disk full'); END").execute(&pool)).unwrap();assert!(call(&main,"interview_memory_append",json!({"handle":h,"input":input(2)})).is_err());
        let status=call(&main,"interview_memory_status",json!({})).unwrap();assert_eq!(status["records"],1);assert_eq!(status["persistence_failed"],true);
        tauri::async_runtime::block_on(sqlx::query("DROP TRIGGER synthetic_full").execute(&pool)).unwrap();call(&main,"interview_memory_append",json!({"handle":h,"input":input(2)})).unwrap();call(&main,"interview_memory_checkpoint",json!({"handle":h})).unwrap();
        let changed:Handle=serde_json::from_value(call(&main,"interview_memory_open",json!({"resumeId":h.id,"missingOlderHistory":false})).unwrap()).unwrap();assert_ne!(h.generation,changed.generation);assert_eq!(call(&main,"interview_memory_status",json!({})).unwrap()["records"],2);assert!(call(&main,"interview_memory_append",json!({"handle":h,"input":input(3)})).is_err());
        let mut forged=input(3);forged["source"]="AI".into();assert!(call(&main,"interview_memory_append",json!({"handle":changed,"input":forged})).is_err());tauri::async_runtime::block_on(pool.close());
    }
    #[test]fn saved_v2_boundary_restores_exact_raw_prefix_and_new_commands_reject_display(){
        let(app,main,display)=app();config(&main,None);let h=open(&main);
        for i in 1..=2{call(&main,"interview_memory_append",json!({"handle":h,"input":input(i)})).unwrap();}
        let boundary=call(&main,"interview_memory_boundary",json!({"handle":h})).unwrap();
        let snapshot:crate::interview_sessions::Snapshot=serde_json::from_value(json!({"version":2,"memory":boundary,"history":[{"id":"saved","role":"self","question":"我只做过模拟","suggestion":"","confirmed_answer":null,"status":"done","sources":[],"time":"00:01","timing":""}],"transcripts":[],"spoken_dialogue":[],"current":{"question":"","answer":""},"calls":0,"usage":"未知"})).unwrap();
        let pool=app.state::<crate::state::AppState>().db_manager.pool().clone();let saved=tauri::async_runtime::block_on(crate::interview_sessions::save(&pool,snapshot,true)).unwrap();
        call(&main,"interview_memory_append",json!({"handle":h,"input":input(3)})).unwrap();
        let snapshot=tauri::async_runtime::block_on(crate::interview_sessions::load(&pool,saved.id)).unwrap();assert_eq!(snapshot.memory.as_ref().unwrap().last_record,2);
        for (cmd,args) in [("interview_memory_restore",json!({"reference":boundary})),("interview_memory_boundary",json!({"handle":h})),("interview_memory_raw_get",json!({"handle":h,"id":"raw-1"})),("interview_memory_test",json!({}))]{assert!(call(&display,cmd,args).is_err());}
        let restored:Handle=serde_json::from_value(call(&main,"interview_memory_restore",json!({"reference":snapshot.memory})).unwrap()).unwrap();assert_ne!(restored.id,h.id);assert_eq!(call(&main,"interview_memory_status",json!({})).unwrap()["records"],2);
        assert!(call(&main,"interview_memory_append",json!({"handle":h,"input":input(4)})).is_err());assert!(call(&main,"interview_memory_raw_get",json!({"handle":restored,"id":"raw-3"})).is_err());
        tauri::async_runtime::block_on(pool.close());
    }
    #[test]fn screen_reference_keeps_original_conditions_without_claiming_image_recovery(){
        let(app,main,_)=app();config(&main,None);let h=open(&main);
        let state=app.state::<InterviewState>();let pool=app.state::<crate::state::AppState>().db_manager.pool().clone();let index=tauri::async_runtime::block_on(state.index(app.handle())).unwrap();
        tauri::async_runtime::block_on(async{
            let hash=interview_core::digest(b"synthetic-image-reference-only");let question="图中 SQL 不允许把 null 转成 0；金额单位是分。";
            let(_,pin,_,binding)=state.memory.request(&pool,index,"screen-question",1,question,Some(hash.clone())).await.unwrap().unwrap();
            state.memory.finish_request(&pool,&h,&pin,"先核对 SQL 条件。",Terminal::Complete,&binding).await.unwrap();
            let slot=state.memory.inner.lock().await;let own=slot.as_ref().unwrap();let image=own.ledger.event("screen-screen-question-1").unwrap();
            assert_eq!(image.asset_sha256.as_ref(),Some(&hash));assert!(image.text.contains(question));assert!(image.text.contains("图像未持久化"));assert_eq!(image.origin,Origin::Screen);assert_eq!(image.question.as_ref(),Some(&pin));assert!(!image.text.contains("data:image"));drop(slot);
            pool.close().await;
        });
    }
    #[test]fn explicit_connection_probe_uses_fixed_text_and_does_not_enable_or_create_memory(){
        let(app,main,_)=app();let(url,server,_)=tauri::async_runtime::block_on(server(0));let mut c=Config::default();c.profile.endpoint=url;
        call(&main,"interview_memory_config_save",json!({"config":c,"key":"synthetic-credential-not-real"})).unwrap();let result=call(&main,"interview_memory_test",json!({})).unwrap();assert_eq!(result["success"],true);assert_eq!(result["attempts"],1);assert_eq!(result["total_tokens"],60);
        assert_eq!(call(&main,"interview_memory_config_get",json!({})).unwrap()["config"]["enabled"],false);assert_eq!(call(&main,"interview_memory_status",json!({})).unwrap()["active"],false);tauri::async_runtime::block_on(server).unwrap();tauri::async_runtime::block_on(app.state::<crate::state::AppState>().db_manager.pool().close());
    }
    #[test]fn three_foreground_versions_complete_out_of_order_during_background_commit(){
        let(app,main,_)=app();let(url,server,received)=tauri::async_runtime::block_on(server(250));config(&main,Some(url));let h=open(&main);
        for i in 1..=40 {call(&main,"interview_memory_append",json!({"handle":h,"input":input(i)})).unwrap();}
        assert_eq!(call(&main,"interview_memory_tick",json!({"handle":h,"sendAllowed":true})).unwrap(),true);
        tauri::async_runtime::block_on(async{tokio::time::timeout(std::time::Duration::from_secs(5),received.notified()).await.unwrap();});let state=app.state::<InterviewState>();state.memory.preempt("https://independent.synthetic.invalid/chat/completions");
        let pool=app.state::<crate::state::AppState>().db_manager.pool().clone();let index=tauri::async_runtime::block_on(state.index(app.handle())).unwrap();let jobs=state.test_answer_jobs();let profiles=vec!["answer-1".into(),"answer-2".into(),"answer-3".into()];
        let leases:Vec<_>=(1..=3).map(|i|jobs.reserve(&format!("parallel-{i}"),1,&profiles).unwrap()).collect();
        let pinned=tauri::async_runtime::block_on(async{let mut v=vec![];for i in 1..=3{v.push(state.memory.request(&pool,index,&format!("parallel-{i}"),1,"HTTP 504 后状态未知",None).await.unwrap().unwrap());}v});
        for(_,pin,_,_)in &pinned{assert_eq!(pin.context_version,0);}
        for i in 41..=48{call(&main,"interview_memory_append",json!({"handle":h,"input":input(i)})).unwrap();}
        tauri::async_runtime::block_on(async{for i in [2,0]{let(h,p,_,b)=&pinned[i];state.memory.finish_request(&pool,h,p,"先查状态。<<<QUESTION>>>504 后如何核查？",Terminal::Complete,b).await.unwrap();}});
        for _ in 0..100{if call(&main,"interview_memory_status",json!({})).unwrap()["running"]==false{break;}std::thread::sleep(std::time::Duration::from_millis(50));}
        let status=call(&main,"interview_memory_status",json!({})).unwrap();assert_eq!(status["context_version"],1);assert_eq!(status["last_attempt"]["status"],"committed");assert_eq!(status["covered_through"],28);
        tauri::async_runtime::block_on(async{let(h,p,_,b)=&pinned[1];state.memory.finish_request(&pool,h,p,"保留未知状态。",Terminal::Complete,b).await.unwrap();let(_,pin,_,_)=state.memory.request(&pool,index,"after-promotion",1,"HTTP 504",None).await.unwrap().unwrap();assert_eq!(pin.context_version,1);});
        assert!(leases.iter().all(|l|!l.cancel.is_cancelled()));drop(leases);assert!(jobs.is_empty());tauri::async_runtime::block_on(server).unwrap();tauri::async_runtime::block_on(pool.close());
    }
    async fn server(delay_ms:u64)->(String,tokio::task::JoinHandle<()>,std::sync::Arc<tokio::sync::Notify>){
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let url=format!("http://{}/chat/completions",listener.local_addr().unwrap());let received=std::sync::Arc::new(tokio::sync::Notify::new());let signal=received.clone();
        let task=tokio::spawn(async move{let(mut socket,_)=listener.accept().await.unwrap();let mut bytes=vec![];let mut buf=[0;4096];let end=loop{let n=socket.read(&mut buf).await.unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);if let Some(p)=bytes.windows(4).position(|w|w==b"\r\n\r\n"){break p+4;}};let header=String::from_utf8_lossy(&bytes[..end]);let length:usize=header.lines().find_map(|s|s.to_lowercase().strip_prefix("content-length:").map(|s|s.trim().parse().unwrap())).unwrap();while bytes.len()<end+length{let n=socket.read(&mut buf).await.unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);}
            let request:Value=serde_json::from_slice(&bytes[end..end+length]).unwrap();assert!(!request.to_string().contains("synthetic-credential-not-real"));let data:Value=serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();signal.notify_one();
            if delay_ms>0{tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;}
            let items:Vec<_>=data["events"].as_array().unwrap().iter().map(|v|json!({"kind":v["event"]["kind"],"origin":v["event"]["origin"],"quote":v["event"]["text"],"refs":[v["ref"]]})).collect();
            let candidate=json!({"schema_version":1,"snapshot_id":data["snapshot_id"],"finish_reason":"stop","items":items,"omitted":[]});let body=json!({"choices":[{"finish_reason":"stop","message":{"content":candidate.to_string()}}],"usage":{"prompt_tokens":40,"completion_tokens":20}}).to_string();let _=socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
        });(url,task,received)
    }
    #[test]fn native_singleflight_respects_foreground_and_publishes_only_persisted_memory(){
        let(app,main,_)=app();let(url,server,_)=tauri::async_runtime::block_on(server(0));config(&main,Some(url));let h=open(&main);for i in 1..=40{call(&main,"interview_memory_append",json!({"handle":h,"input":input(i)})).unwrap();}
        let state=app.state::<InterviewState>();let lease=state.test_answer_jobs().reserve("foreground",1,&["answer-1".into()]).unwrap();assert_eq!(call(&main,"interview_memory_tick",json!({"handle":h,"sendAllowed":true})).unwrap(),false);drop(lease);
        assert_eq!(call(&main,"interview_memory_tick",json!({"handle":h,"sendAllowed":false})).unwrap(),false);assert_eq!(call(&main,"interview_memory_tick",json!({"handle":h,"sendAllowed":true})).unwrap(),true);
        tauri::async_runtime::block_on(async{server.await.unwrap();tokio::time::timeout(std::time::Duration::from_secs(5),async{loop{if !state.memory.status().await["running"].as_bool().unwrap(){break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}}).await.unwrap();});
        let status=call(&main,"interview_memory_status",json!({})).unwrap();assert_eq!(status["covered_through"],28);assert_eq!(status["last_attempt"]["status"],"committed");assert_eq!(status["checkpoint"]["revision"],2);assert_eq!(status["accounted_units"],60);
        let pool=app.state::<crate::state::AppState>().db_manager.pool().clone();let index=tauri::async_runtime::block_on(state.index(app.handle())).unwrap();
        tauri::async_runtime::block_on(async{let(_,pin,work,binding)=state.memory.request(&pool,index,"followup",1,"HTTP 504",None).await.unwrap().unwrap();assert_eq!(pin.context_version,1);assert!(!work.memory.is_empty());state.memory.finish_request(&pool,&h,&pin,"先查状态。<<<QUESTION>>>HTTP 504 后怎样判断？",Terminal::Complete,&binding).await.unwrap();index.set("version","999").await.unwrap();let(_,new_pin,work,_)=state.memory.request(&pool,index,"after-withdrawal",1,"HTTP 504",None).await.unwrap().unwrap();assert!(new_pin.context_version>pin.context_version);assert!(work.memory.is_empty());pool.close().await;});
    }
    #[test]fn cancellation_keeps_unknown_usage_reservation_and_does_not_promote(){
        let(app,main,_)=app();let(url,server,received)=tauri::async_runtime::block_on(server(10_000));config(&main,Some(url));let h=open(&main);for i in 1..=40{call(&main,"interview_memory_append",json!({"handle":h,"input":input(i)})).unwrap();}assert_eq!(call(&main,"interview_memory_tick",json!({"handle":h,"sendAllowed":true})).unwrap(),true);
        let state=app.state::<InterviewState>();tauri::async_runtime::block_on(async{tokio::time::timeout(std::time::Duration::from_secs(5),received.notified()).await.unwrap();state.memory.cancel();tokio::time::timeout(std::time::Duration::from_secs(5),async{loop{if !state.memory.status().await["running"].as_bool().unwrap(){break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}}).await.unwrap();});server.abort();
        let status=call(&main,"interview_memory_status",json!({})).unwrap();assert_eq!(status["covered_through"],0);assert_eq!(status["last_attempt"]["status"],"cancelled");assert_eq!(status["last_attempt"]["usage"],Value::Null);assert!(status["accounted_units"].as_u64().unwrap()>0);
        tauri::async_runtime::block_on(app.state::<crate::state::AppState>().db_manager.pool().close());
    }
}
