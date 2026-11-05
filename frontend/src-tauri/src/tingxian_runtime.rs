use std::path::PathBuf;
use tauri::{AppHandle, Manager};

fn resource_root(executable: &std::path::Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    { Some(executable.parent()?.parent()?.join("Resources")) }
    #[cfg(not(target_os = "macos"))]
    { Some(executable.parent()?.to_path_buf()) }
}
fn python_in(resources: &std::path::Path) -> PathBuf {
    #[cfg(windows)]
    { resources.join("python/python.exe") }
    #[cfg(not(windows))]
    { resources.join("python/bin/python3") }
}
pub fn document_python() -> String {
    let bundled = std::env::current_exe().ok().and_then(|exe| resource_root(&exe)).map(|root| python_in(&root));
    if let Some(path) = bundled.filter(|p| p.is_file()) { return path.to_string_lossy().into_owned(); }
    std::env::var("INTERVIEW_PYTHON").unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.into())
}

pub async fn prepare_model(app:&AppHandle)->Result<Option<&'static str>,String>{
    let resource = app.path().resource_dir().map_err(|e|e.to_string())?.join("whisper/ggml-small.bin");
    if !resource.is_file() {return Ok(None)}
    let models = app.path().app_data_dir().map_err(|e|e.to_string())?.join("models");
    tokio::task::spawn_blocking(move || seed_model(resource,models)).await.map_err(|e|e.to_string())??;
    Ok(Some("small"))
}
fn seed_model(source:PathBuf,models:PathBuf)->Result<(),String>{
    let io = |e:std::io::Error|format!("准备内置 Whisper 模型失败：{e}");
    std::fs::create_dir_all(&models).map_err(io)?;
    let dest=models.join("ggml-small.bin");
    if dest.is_file(){return Ok(())}
    let mut temp=tempfile::NamedTempFile::new_in(models).map_err(io)?;
    let mut file=std::fs::File::open(source).map_err(io)?;
    std::io::copy(&mut file,temp.as_file_mut()).map_err(io)?;
    temp.as_file().sync_all().map_err(io)?;
    match temp.persist_noclobber(dest) {
        Ok(_)=>Ok(()),
        Err(e) if e.error.kind()==std::io::ErrorKind::AlreadyExists=>Ok(()),
        Err(e)=>Err(io(e.error)),
    }
}

/// Offline post-install smoke check, without opening devices or the user's database.
pub fn installation_check()->Result<(),String>{
    use std::process::{Command,Stdio};
    let executable=std::env::current_exe().map_err(|e|e.to_string())?;
    let macos=executable.parent().ok_or("invalid bundle path")?;
    let resources=resource_root(&executable).ok_or("missing resource directory")?;
    let python=python_in(&resources);
    if !python.is_file() || PathBuf::from(document_python())!=python{return Err("bundled Python missing".into())}
    let test="from io import BytesIO;from docx import Document;from pypdf import PdfReader,PdfWriter;import yaml;d=Document();d.add_paragraph('中文契约测试');b=BytesIO();d.save(b);b.seek(0);assert Document(b).paragraphs[0].text=='中文契约测试';w=PdfWriter();w.add_blank_page(width=50,height=50);p=BytesIO();w.write(p);p.seek(0);assert len(PdfReader(p).pages)==1 and yaml.safe_load('n: 2')['n']==2";
    if !Command::new(python).args(["-I","-X","utf8","-B","-c",test]).status().map_err(|e|e.to_string())?.success(){return Err("bundled document runtime failed".into())}
    if !Command::new(macos.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" })).args(["-v","error","-f","lavfi","-i","anullsrc","-t","0.05","-f","null","-"]).stdin(Stdio::null()).status().map_err(|e|e.to_string())?.success(){return Err("bundled media runtime failed".into())}
    let model=resources.join("whisper/ggml-small.bin");
    let mut config=whisper_rs::WhisperContextParameters::default();config.use_gpu(false);
    let _model=whisper_rs::WhisperContext::new_with_params(model.to_str().ok_or("invalid model path")?,config).map_err(|e|format!("bundled ASR model: {e}"))?;
    #[cfg(windows)]
    {
        ort::init_from(resources.join("onnxruntime.dll").to_string_lossy().into_owned())
            .with_telemetry(false).commit().map_err(|e| format!("bundled ONNX Runtime: {e}"))?;
        let mut vad=crate::audio::vad::ContinuousVadProcessor::new(16000,1500).map_err(|e|format!("bundled VAD: {e}"))?;
        vad.process_audio(&vec![0.0;16000]).map_err(|e|format!("bundled VAD inference: {e}"))?;
    }
    tauri::async_runtime::block_on(database_installation_check()).map_err(|e|format!("安装数据库检查：{e}"))?;
    Ok(())
}

/// Explicit CI/diagnostic command. Captures a monitor into memory and recycles
/// only a newly created synthetic archive. It never opens the user's database.
#[cfg(windows)]
pub fn windows_platform_check()->Result<(),String>{
    let root=tempfile::tempdir().map_err(|e|e.to_string())?;
    let archive=root.path().join(format!("TingXian-platform-check-{}",uuid::Uuid::new_v4()));
    std::fs::create_dir(&archive).map_err(|e|e.to_string())?;
    let content=b"synthetic recoverable archive";
    std::fs::write(archive.join("check.txt"),content).map_err(|e|e.to_string())?;
    let recycled=crate::interview_history::move_to_trash(&archive)?;
    if std::fs::read(recycled.join("check.txt")).map_err(|e|e.to_string())?!=content {
        return Err("recycled archive verification failed".into());
    }
    let screens=crate::interview_screenshot::interview_screenshot_displays()?;
    if screens.is_empty(){return Err("no monitor available for screenshot check".into());}
    let png=tauri::async_runtime::block_on(crate::interview_screenshot::capture(None,tokio_util::sync::CancellationToken::new()))?;
    if !png.starts_with(b"\x89PNG\r\n\x1a\n"){return Err("invalid screenshot PNG".into());}
    Ok(())
}

/// Rehearse a fresh database, a live WAL upgrade and restart with synthetic data only.
async fn database_installation_check() -> anyhow::Result<()> {
    use crate::{database::{manager::DatabaseManager,repositories::{meeting::MeetingsRepository,setting::SettingsRepository}},interview_sessions};
    use anyhow::{ensure,Context};
    let root=tempfile::tempdir()?;
    let name=|s:&str|root.path().join(s).to_string_lossy().into_owned();
    let legacy=name("meeting_minutes.db");
    let modern=name("meeting_minutes.sqlite");
    let first=DatabaseManager::new(&legacy,&name("absent.db")).await?;
    let pool=first.pool();
    ensure!(MeetingsRepository::get_meetings(pool).await?.is_empty());
    sqlx::query("PRAGMA journal_mode=WAL").execute(pool).await?;
    sqlx::query("INSERT INTO meetings(id,title,created_at,updated_at) VALUES('sample','合成旧会议',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)").execute(pool).await?;
    sqlx::query("INSERT INTO transcripts(id,meeting_id,transcript,timestamp) VALUES('segment','sample','合成转写','00:01')").execute(pool).await?;
    SettingsRepository::save_transcript_config(pool,"localWhisper","small").await?;
    let snapshot=serde_json::from_value(serde_json::json!({"version":1,"history":[{"id":"q","role":"remote","question":"合成问题","suggestion":"合成回答","confirmed_answer":null,"status":"done","sources":[],"time":"00:01","timing":""}],"transcripts":[],"spoken_dialogue":[],"current":{"question":"","answer":""},"calls":1,"usage":"unknown"}))?;
    let saved=interview_sessions::save(pool,snapshot,true).await.map_err(anyhow::Error::msg)?;
    let empty=serde_json::json!({"version":1,"history":[],"transcripts":[],"spoken_dialogue":[],"current":{"question":"","answer":""},"calls":0,"usage":"未知"});
    ensure!(interview_sessions::save(pool,serde_json::from_value(empty.clone())?,true).await.is_err(),"empty snapshot accepted");
    let mut draft=empty.clone();
    draft["input_drafts"]=serde_json::json!({"question":"StockMarketAgent：null、零、缺失字段","confirmed_answer":"尚未确认的合成回答","self_edit":null});
    let draft_snapshot=serde_json::from_value(draft.clone())?;
    ensure!(interview_sessions::save(pool,serde_json::from_value(draft.clone())?,false).await.is_err(),"draft saved without explicit request");
    let draft_saved=interview_sessions::save(pool,draft_snapshot,true).await.map_err(anyhow::Error::msg)?;
    ensure!(draft_saved.turns==0 && draft_saved.calls==0);
    let mut edit=draft.clone();
    edit["history"]=serde_json::json!([{"id":"self","role":"self","question":"合成原文","suggestion":"","confirmed_answer":null,"status":"done","sources":[],"time":"00:01","timing":""}]);
    edit["input_drafts"]["self_edit"]=serde_json::json!({"id":"self","text":"合成原文"});
    let synthetic_key="synthetic-input-redaction-token";
    edit["input_drafts"]["question"]=format!("合成修正 {synthetic_key}").into();
    let edit_saved=interview_sessions::save_with_key(pool,serde_json::from_value(edit)?,true,synthetic_key).await.map_err(anyhow::Error::msg)?;
    let mut invalid=draft.clone();invalid["input_drafts"]["question"]=0.into();
    ensure!(serde_json::from_value::<interview_sessions::Snapshot>(invalid).is_err(),"malformed input accepted");
    ensure!(std::path::Path::new(&(legacy.clone()+"-wal")).exists());
    for _ in 0..2 {
        let db=DatabaseManager::new(&modern,&legacy).await?;
        let meeting=MeetingsRepository::get_meeting(db.pool(),"sample").await?.context("旧会议缺失")?;
        ensure!(meeting.transcripts.len()==1 && meeting.transcripts[0].text=="合成转写");
        let recovered=interview_sessions::load(db.pool(),saved.id).await.map_err(anyhow::Error::msg)?;
        ensure!(recovered.history[0]["suggestion"]=="合成回答");
        ensure!(recovered.input_drafts.is_none(),"legacy snapshot acquired invented input");
        let recovered_draft=interview_sessions::load(db.pool(),draft_saved.id).await.map_err(anyhow::Error::msg)?;
        ensure!(serde_json::to_value(recovered_draft)?==draft,"input-only snapshot lost content during migration/reopen");
        let recovered_edit=interview_sessions::load(db.pool(),edit_saved.id).await.map_err(anyhow::Error::msg)?;
        ensure!(!serde_json::to_string(&recovered_edit)?.contains(synthetic_key),"input draft retained configured credential");
        ensure!(recovered_edit.history[0]["question"]=="合成原文" && recovered_edit.history[0]["confirmed_answer"].is_null());
        ensure!(recovered_edit.spoken_dialogue.is_empty() && recovered_edit.calls==0,"unsubmitted input became actual dialogue");
        let edit_input=recovered_edit.input_drafts.context("input draft missing")?;
        let edit_base=edit_input.self_edit.context("self edit missing")?;
        ensure!(edit_base.id=="self" && edit_base.text=="合成原文");
        ensure!(edit_input.confirmed_answer=="尚未确认的合成回答");
        let config=SettingsRepository::get_transcript_config(db.pool()).await?.context("模型设置缺失")?;
        ensure!(config.provider=="localWhisper" && config.model=="small");
        db.pool().close().await;
    }
    ensure!(MeetingsRepository::get_meeting(pool,"sample").await?.is_some());
    pool.close().await;
    eprintln!("PASS: legacy sessions and unsubmitted input survive migration/reopen; empty/invalid/unconfirmed saves rejected; credentials redacted; no draft promoted to dialogue");
    Ok(())
}
