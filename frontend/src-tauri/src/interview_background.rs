//! Native picker -> local Python harness -> review -> transactional FTS update.
use crate::interview::InterviewState;
use interview_core::background::{Bundle,Profile};
use serde_json::{json,Value};
use std::{path::{Path,PathBuf}, sync::{Mutex,atomic::Ordering}};
use tauri::{AppHandle,State,Window};
use tauri_plugin_dialog::DialogExt;
use tokio::io::{AsyncWriteExt,AsyncBufReadExt,AsyncReadExt,BufReader};
use tokio_util::sync::CancellationToken;
const HARNESS:&str=include_str!("../../../interview/skills/interview-background/scripts/background.py");
const SKILL:&str=include_str!("../../../interview/skills/interview-background/SKILL.md");
const LIMIT:usize=16*1024*1024;
#[derive(Default)]
pub struct Work(Mutex<Inner>);
#[derive(Default)]
struct Inner{selection:Option<(String,Vec<PathBuf>,bool)>,running:Option<CancellationToken>,progress:String,draft:Option<Draft>}
struct Draft{id:String,version:String,bundle:Bundle,metrics:Value,failures:Value}
impl Work{
    pub fn cancel(&self){if let Some(t)=&self.0.lock().unwrap().running{t.cancel()}}
    fn view(&self)->Value{
        let s=self.0.lock().unwrap();
        json!({"busy":s.running.is_some(),"progress":s.progress,"draft":s.draft.as_ref().map(|d|json!({
            "id":d.id,"profile":d.bundle.profile,"documents":d.bundle.documents.iter().map(|f|json!({"id":f.id,"name":f.name,"count":f.records.len()})).collect::<Vec<_>>(),
            "metrics":d.metrics,"failures":d.failures}))})
    }
}
fn owner(window:&Window)->Result<(),String>{if window.label()!="main"{return Err("请在主操作窗口整理资料".into())}Ok(())}
async fn guard<'a>(window:&Window,app:&AppHandle,state:&'a InterviewState)->Result<tokio::sync::MutexGuard<'a,()>,String>{
    owner(window)?;crate::interview_exit::ensure_not_exiting(app)?;
    let guard=state.lifecycle.try_lock().map_err(|_|"请等待当前操作完成")?;
    if state.session.load(Ordering::SeqCst)||crate::audio::recording_commands::is_recording().await{return Err("请结束面试会话后整理资料".into())}
    Ok(guard)
}
#[tauri::command]
pub async fn background_status(window:Window,app:AppHandle,state:State<'_,InterviewState>)->Result<Value,String>{
    owner(&window)?;let index=state.index(&app).await?;let cfg=index.provider_config().await?.background;
    Ok(json!({"library":index.background_view().await?,"work":state.background.view(),"model":cfg.request.model,
        "ready":cfg.enabled&&(cfg.accepts_empty_key()||!state.provider_key(&cfg).await.is_empty())}))
}
#[tauri::command]
pub async fn background_choose(window:Window,app:AppHandle,state:State<'_,InterviewState>,mode:String)->Result<Value,String>{
    let _guard=guard(&window,&app,&state).await?;
    let folder=mode=="folder";let is_pack=mode=="bundle";
    if !folder&&!is_pack&&mode!="files"{return Err("未知选择方式".into())}
    let picker=app.clone();
    let paths=tauri::async_runtime::spawn_blocking(move||{
        let f=picker.dialog().file();
        let list=if folder{f.blocking_pick_folder().map(|v|vec![v])}
            else if is_pack{f.add_filter("Meetily 资料包",&["json"]).blocking_pick_file().map(|v|vec![v])}
            else{f.add_filter("背景资料",&["md","txt","pdf","docx","json","yaml","yml"]).blocking_pick_files()};
        list.unwrap_or_default().into_iter().map(|p|p.into_path().map_err(|e|e.to_string())).collect::<Result<Vec<_>,_>>()
    }).await.map_err(|e|e.to_string())??;
    if paths.is_empty(){return Ok(Value::Null)}
    let id=uuid::Uuid::new_v4().to_string();
    if is_pack{
        let f=&paths[0];let meta=tokio::fs::symlink_metadata(f).await.map_err(|e|e.to_string())?;
        if !meta.is_file()||meta.len()>LIMIT as u64{return Err("请选择不超过 16 MiB 的普通 JSON 文件".into())}
        let bytes=tokio::fs::read(f).await.map_err(|e|e.to_string())?;
        if bytes.len()>LIMIT{return Err("资料包过大".into())}
        let bundle:Bundle=serde_json::from_slice(&bytes).map_err(|_|"不是有效的 Meetily 背景资料包")?;bundle.validate()?;
        let version=state.index(&app).await?.setting("version").await?;
        state.background.0.lock().unwrap().draft=Some(Draft{id,version,bundle,metrics:Value::Null,failures:json!([])});
        return Ok(json!({"draft":true}))
    }
    let label=paths.iter().take(3).filter_map(|p|p.file_name()).map(|p|p.to_string_lossy()).collect::<Vec<_>>().join("、");
    let count=paths.len();
    state.background.0.lock().unwrap().selection=Some((id.clone(),paths,folder));
    Ok(json!({"id":id,"label":label,"count":count,"folder":folder}))
}
#[tauri::command]
pub fn background_progress(window:Window,state:State<'_,InterviewState>)->Result<Value,String>{owner(&window)?;let w=state.background.0.lock().unwrap();Ok(json!({"busy":w.running.is_some(),"progress":w.progress}))}
#[tauri::command]
pub fn background_cancel(window:Window,state:State<'_,InterviewState>)->Result<(),String>{owner(&window)?;state.background.cancel();Ok(())}
#[tauri::command]
pub async fn background_generate(window:Window,app:AppHandle,state:State<'_,InterviewState>,selection:String,include_library:bool)->Result<(),String>{
    let _guard=guard(&window,&app,&state).await?;
    let measure=selection=="measure";
    let index=state.index(&app).await?;let p=index.provider_config().await?.background;
    let key=state.provider_key(&p).await;
    if !p.enabled||(!p.accepts_empty_key()&&key.is_empty()){return Err("请先配置独立的背景整理模型".into())}
    let (paths,folder)=if measure{(vec![],false)}else{let work=state.background.0.lock().unwrap();
        let (_,paths,folder)=work.selection.as_ref().filter(|s|s.0==selection).ok_or("请重新选择文件夹或重点文件")?;(paths.clone(),*folder)};
    let previous=if !measure && include_library{index.background_inputs(false).await?}else{vec![]};
    let version=index.setting("version").await?;
    let mut req=json!({"api":{"model":p.request.model,"endpoint":p.endpoint,"key":key,"timeout":p.request.total_timeout_secs},"previous":previous});
    if measure{req["measure"]=json!(true)}else if folder{req["folder"]=json!(paths[0])}else{req["files"]=json!(paths)}
    let token=CancellationToken::new();
    {let mut w=state.background.0.lock().unwrap();w.running=Some(token.clone());w.progress="读取资料".into();}
    let job=async{
        if !measure{return worker(&req,&state.background).await}
        let (mut cursor,mut chunks,mut calls,mut total,mut peak,mut bound)=(0,0usize,0u64,0u64,0u64,0u64);
        loop {
            let (next,batch)=index.background_input_page(true,cursor).await?;
            if batch.is_empty(){break} cursor=next;chunks+=batch.len();req["previous"]=json!(batch);
            let result=worker(&req,&state.background).await?;let m=&result["metrics"];
            calls+=m["calls"].as_u64().ok_or("计量请求数无效")?;
            total+=m["sum_prompt_tokens"].as_u64().ok_or("接口未返回输入 token 数")?;
            peak=peak.max(m["peak_prompt_tokens"].as_u64().unwrap_or(0));bound=bound.max(m["peak_budget"].as_u64().unwrap_or(0));
        }
        if chunks==0{return Err("资料库中没有允许外发的片段".into())}
        Ok(json!({"chunks":chunks,"metrics":{"calls":calls,"sum_prompt_tokens":total,"peak_prompt_tokens":peak,"peak_budget":bound}}))
    };
    let result=tokio::select!{biased;_=token.cancelled()=>Err("已取消，资料库未改变".into()),result=job=>result};
    {let mut w=state.background.0.lock().unwrap();w.running=None;w.progress.clear();}
    let mut result=result?;
    if measure{
        let data=json!({"model":p.request.model,"endpoint":p.endpoint,"version":version,"metrics":result["metrics"],"chunks":result["chunks"]});
        sqlx::query("INSERT OR REPLACE INTO background_library VALUES('measurement',?)").bind(data.to_string()).execute(&index.pool).await.map_err(|e|e.to_string())?;
        return Ok(())
    }
    let bundle:Bundle=serde_json::from_value(result["bundle"].take()).map_err(|_|"模型整理结果不符合资料格式")?;
    bundle.validate()?;
    state.background.0.lock().unwrap().draft=Some(Draft{id:uuid::Uuid::new_v4().to_string(),version,bundle,metrics:result["metrics"].take(),failures:result["failures"].take()});
    Ok(())
}
async fn worker(req:&Value,work:&Work)->Result<Value,String>{
    use std::process::Stdio;
    let python=std::env::var("INTERVIEW_PYTHON").unwrap_or_else(|_|"python3".into());
    let mut child=tokio::process::Command::new(python).args(["-I","-u","-c",HARNESS,"desktop"]).env_remove("DEEPSEEK_API_KEY")
        .kill_on_drop(true).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e|e.to_string())?;
    let mut input=child.stdin.take().ok_or("资料输入不可用")?;
    input.write_all(&serde_json::to_vec(req).map_err(|e|e.to_string())?).await.map_err(|e|e.to_string())?;drop(input);
    let stderr=child.stderr.take().ok_or("资料错误通道不可用")?;
    let mut reader=BufReader::new(child.stdout.take().ok_or("资料输出不可用")?);
    let output=async{
        let mut result=None;
        loop{
            let mut line=Vec::new();
            let n=(&mut reader).take((LIMIT+1024*1024) as u64+1).read_until(b'\n',&mut line).await.map_err(|e|e.to_string())?;
            if n==0{break} if n>LIMIT+1024*1024{return Err("资料包过大，请分批".to_string())}
            let v:Value=serde_json::from_slice(&line).map_err(|_|"整理工具输出无效".to_string())?;
            if let Some(s)=v["progress"].as_str(){work.0.lock().unwrap().progress=s.chars().take(140).collect()}else{result=Some(v)}
        }
        Ok::<_,String>(result)
    };
    let errors=async{let mut e=String::new();stderr.take(4096).read_to_string(&mut e).await.map_err(|e|e.to_string())?;Ok(e)};
    let (result,e)=tokio::try_join!(output,errors)?;
    if !child.wait().await.map_err(|e|e.to_string())?.success(){return Err(interview_core::redact_credentials(e.trim(),req["api"]["key"].as_str()))}
    result.ok_or_else(||"整理工具没有返回资料包".into())
}
#[tauri::command]
pub fn background_preview(window:Window,state:State<'_,InterviewState>,draft:String,document:String,offset:usize)->Result<Value,String>{
    owner(&window)?;let w=state.background.0.lock().unwrap();
    let file=w.draft.as_ref().filter(|d|d.id==draft).and_then(|d|d.bundle.documents.iter().find(|f|f.id==document)).ok_or("草稿或文件已改变")?;
    let rows=file.records.iter().skip(offset).take(20).collect::<Vec<_>>();
    Ok(json!({"rows":rows,"more":offset.saturating_add(rows.len())<file.records.len()}))
}
#[tauri::command]
pub async fn background_fact_candidates(window:Window,app:AppHandle,state:State<'_,InterviewState>)->Result<Vec<Value>,String>{
    owner(&window)?;use sqlx::Row;
    let index=state.index(&app).await?;
    let rows=sqlx::query("WITH eligible AS (SELECT c.*,d.kind,d.hash,row_number() OVER(PARTITION BY c.path ORDER BY c.rowid) AS n FROM chunks c JOIN documents d ON d.path=c.path WHERE c.path GLOB '.背景/*' AND d.kind IN ('personal','evidence')) SELECT * FROM eligible ORDER BY n,path LIMIT 6").fetch_all(&index.pool).await.map_err(|e|e.to_string())?;
    Ok(rows.iter().map(|r|json!({"source_id":r.get::<String,_>("id"),"root":"背景资料库","path":r.get::<String,_>("path"),"hash":r.get::<String,_>("hash"),"text":r.get::<String,_>("text"),"kind":r.get::<String,_>("kind"),"location":r.get::<String,_>("location")})).collect())
}
#[tauri::command]
pub async fn background_edit(window:Window,app:AppHandle,state:State<'_,InterviewState>,version:String,profile:Profile,share:bool,merge_terms:bool)->Result<(),String>{
    let _guard=guard(&window,&app,&state).await?;
    let index=state.index(&app).await?;
    let library=index.background_view().await?;
    if library["state"].is_null(){return Err("暂无可编辑背景".into())}
    index.edit_background(profile,&version,share,library["state"]["metrics"].clone(),merge_terms).await
}
#[tauri::command]
pub async fn background_apply(window:Window,app:AppHandle,state:State<'_,InterviewState>,id:String,profile:Profile,share:bool,merge_terms:bool)->Result<(),String>{
    let _guard=guard(&window,&app,&state).await?;
    let (mut bundle,version,metrics)={let w=state.background.0.lock().unwrap();let d=w.draft.as_ref().filter(|d|d.id==id).ok_or("草稿已变化，请重新加载")?;(d.bundle.clone(),d.version.clone(),d.metrics.clone())};
    bundle.profile=profile;state.index(&app).await?.apply_background(&bundle,&version,share,metrics,merge_terms).await?;
    state.background.0.lock().unwrap().draft=None;Ok(())
}
fn export_into(parent:&Path)->Result<String,String>{
    let dir=parent.join("interview-background");
    std::fs::create_dir(&dir).map_err(|e|format!("不能创建技能文件夹（已有目录不会覆盖）：{e}"))?;
    std::fs::create_dir(dir.join("scripts")).map_err(|e|e.to_string())?;
    for (name,text) in [("SKILL.md",SKILL),("scripts/background.py",HARNESS)]{
        use std::io::Write;
        let mut f=std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join(name)).map_err(|e|e.to_string())?;
        f.write_all(text.as_bytes()).and_then(|_|f.sync_all()).map_err(|e|e.to_string())?;
    }
    Ok(dir.display().to_string())
}
#[tauri::command]
pub async fn background_export_skill(window:Window,app:AppHandle)->Result<Option<String>,String>{
    owner(&window)?;
    tauri::async_runtime::spawn_blocking(move||{
        let Some(p)=app.dialog().file().blocking_pick_folder() else{return Ok(None)};
        export_into(&p.into_path().map_err(|e|e.to_string())?).map(Some)
    }).await.map_err(|e|e.to_string())?
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn exported_skill_is_self_contained_and_does_not_overwrite(){
        let dir=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());std::fs::create_dir(&dir).unwrap();
        let out=PathBuf::from(export_into(&dir).unwrap());assert_eq!(std::fs::read_to_string(out.join("SKILL.md")).unwrap(),SKILL);
        assert_eq!(std::fs::read_to_string(out.join("scripts/background.py")).unwrap(),HARNESS);
        assert!(export_into(&dir).is_err());
    }
}
