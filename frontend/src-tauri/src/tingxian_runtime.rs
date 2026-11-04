use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub fn document_python() -> String {
    // Finder launches without the developer's shell environment.
    let bundled = std::env::current_exe().ok().and_then(|exe| {
        Some(exe.parent()?.parent()?.join("Resources/python/bin/python3"))
    });
    if let Some(path) = bundled.filter(|p|p.is_file()) {return path.to_string_lossy().into_owned()}
    std::env::var("INTERVIEW_PYTHON").unwrap_or_else(|_|"python3".into())
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
    let resources=macos.parent().ok_or("missing Contents")?.join("Resources");
    let python=resources.join("python/bin/python3");
    if !python.is_file() || PathBuf::from(document_python())!=python{return Err("bundled Python missing".into())}
    let test="from io import BytesIO;from docx import Document;from pypdf import PdfReader,PdfWriter;import yaml;d=Document();d.add_paragraph('sample');b=BytesIO();d.save(b);b.seek(0);assert Document(b).paragraphs[0].text=='sample';w=PdfWriter();w.add_blank_page(width=50,height=50);p=BytesIO();w.write(p);p.seek(0);assert len(PdfReader(p).pages)==1 and yaml.safe_load('n: 2')['n']==2";
    if !Command::new(python).args(["-I","-B","-c",test]).status().map_err(|e|e.to_string())?.success(){return Err("bundled document runtime failed".into())}
    if !Command::new(macos.join("ffmpeg")).args(["-v","error","-f","lavfi","-i","anullsrc","-t","0.05","-f","null","-"]).stdin(Stdio::null()).status().map_err(|e|e.to_string())?.success(){return Err("bundled media runtime failed".into())}
    let model=resources.join("whisper/ggml-small.bin");
    let mut config=whisper_rs::WhisperContextParameters::default();config.use_gpu(false);
    let _model=whisper_rs::WhisperContext::new_with_params(model.to_str().ok_or("invalid model path")?,config).map_err(|e|format!("bundled ASR model: {e}"))?;
    tauri::async_runtime::block_on(database_installation_check()).map_err(|e|format!("安装数据库检查：{e}"))?;
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
    ensure!(std::path::Path::new(&(legacy.clone()+"-wal")).exists());
    for _ in 0..2 {
        let db=DatabaseManager::new(&modern,&legacy).await?;
        let meeting=MeetingsRepository::get_meeting(db.pool(),"sample").await?.context("旧会议缺失")?;
        ensure!(meeting.transcripts.len()==1 && meeting.transcripts[0].text=="合成转写");
        let recovered=interview_sessions::load(db.pool(),saved.id).await.map_err(anyhow::Error::msg)?;
        ensure!(recovered.history[0]["suggestion"]=="合成回答");
        let config=SettingsRepository::get_transcript_config(db.pool()).await?.context("模型设置缺失")?;
        ensure!(config.provider=="localWhisper" && config.model=="small");
        db.pool().close().await;
    }
    ensure!(MeetingsRepository::get_meeting(pool,"sample").await?.is_some());
    pool.close().await;
    Ok(())
}
