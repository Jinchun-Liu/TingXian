//! Isolated real-filesystem + real-SQLite + macOS Trash regression. No user data.
use anyhow::{ensure, Result};
use app_lib::{database::manager::DatabaseManager, interview_history::{clear_history_with, clear_temporary_history_with, residue_status, move_to_trash}};
use serde_json::json;
use std::{fs, path::PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).expect("fixture directory required"));
    ensure!(!root.exists(), "fixture directory must be new");
    fs::create_dir_all(&root)?;
    let root=root.canonicalize()?;
    let recordings = root.join("recordings"); fs::create_dir(&recordings)?;
    let owned = recordings.join("Interview practice_test"); fs::create_dir(&owned)?;
    fs::write(owned.join("metadata.json"), r#"{"meeting_name":"Interview practice"}"#)?;
    fs::write(owned.join("transcripts.json"), r#"{"text":"synthetic transcript"}"#)?;
    let unrelated = recordings.join("Unrelated meeting"); fs::create_dir(&unrelated)?;
    fs::write(unrelated.join("keep.txt"), "not interview data")?;
    fs::write(root.join("profile.sqlite"), "personal index sentinel")?;
    fs::write(root.join("ggml-base.bin"), "model sentinel")?;
    let db = DatabaseManager::new(root.join("meetings.sqlite").to_str().unwrap(),root.join("missing-legacy.db").to_str().unwrap()).await?;
    let pool = db.pool();
    for (id,title,folder) in [("owned","Interview practice",&owned),("other","Unrelated meeting",&unrelated)] {
        sqlx::query("INSERT INTO meetings(id,title,created_at,updated_at,folder_path) VALUES(?,?,datetime('now'),datetime('now'),?)")
            .bind(id).bind(title).bind(folder.to_string_lossy().as_ref()).execute(pool).await?;
    }
    sqlx::query("INSERT INTO transcripts(id,meeting_id,transcript,timestamp) VALUES('segment','owned','synthetic transcript','00:01')").execute(pool).await?;
    sqlx::query("INSERT INTO summary_processes(meeting_id,status,created_at,updated_at,result) VALUES('owned','completed',datetime('now'),datetime('now'),'synthetic summary')").execute(pool).await?;
    sqlx::query("INSERT INTO transcript_chunks(meeting_id,transcript_text,model,model_name,created_at) VALUES('owned','synthetic chunk','test','test',datetime('now'))").execute(pool).await?;
    sqlx::query("INSERT INTO meeting_notes(meeting_id,notes_markdown,notes_json,created_at,updated_at) VALUES('owned','synthetic notes','{}',datetime('now'),datetime('now'))").execute(pool).await?;
    let saved:app_lib::interview_sessions::Snapshot=serde_json::from_value(json!({"version":1,"history":[{"id":"saved-r1","role":"remote","question":"synthetic question","suggestion":"synthetic full AI answer","confirmed_answer":null,"status":"done","sources":[],"time":"00:01","timing":""}],"transcripts":[],"spoken_dialogue":[],"current":{"question":"synthetic question","answer":"synthetic full AI answer"},"calls":1,"usage":"unknown"}))?;
    let saved=app_lib::interview_sessions::save(pool,saved,true).await.map_err(anyhow::Error::msg)?;
    let initial=residue_status(pool,&recordings).await.map_err(anyhow::Error::msg)?;
    ensure!(initial.recording_folders==1 && initial.unfinished_folders==1 && initial.database_meetings==1 && initial.saved_sessions==1 && initial.staged_archives==0,"initial residue scope incorrect");
    let before_settings:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings").fetch_one(pool).await?;
    let failure = clear_history_with(pool,&recordings,json!({"test":"unavailable Trash"}), |_|Err("simulated Trash failure".into())).await;
    ensure!(failure.as_ref().err().is_some_and(|e|e.contains("simulated Trash failure")) && owned.join("transcripts.json").exists(), "Trash failure branch not reached or recording lost: {failure:?}");
    ensure!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM meetings").fetch_one(pool).await? == 2, "Trash failure cleared DB");
    ensure!(app_lib::interview_sessions::load(pool,saved.id).await.is_ok(),"Trash failure removed saved answers");
    ensure!(residue_status(pool,&recordings).await.map_err(anyhow::Error::msg)?.staged_archives==1,"failed cleanup staging was not detected");
    println!("PASS: failed Trash restores recordings and leaves database untouched");

    sqlx::query("CREATE TRIGGER refuse_test_delete BEFORE DELETE ON meetings WHEN OLD.id='owned' BEGIN SELECT RAISE(ABORT,'test rejection'); END").execute(pool).await?;
    let failure = clear_history_with(pool,&recordings,json!({"test":"DB rollback"}),move_to_trash).await;
    ensure!(failure.as_ref().err().is_some_and(|e|e.contains("test rejection")) && owned.join("transcripts.json").exists(), "DB failure branch not reached or recording lost: {failure:?}");
    ensure!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM transcripts WHERE meeting_id='owned'").fetch_one(pool).await? == 1, "DB failure did not roll back transcripts");
    ensure!(app_lib::interview_sessions::load(pool,saved.id).await.is_ok(),"DB rollback lost saved answers");
    sqlx::query("DROP TRIGGER refuse_test_delete").execute(pool).await?;
    println!("PASS: database rejection rolls back rows and restores moved recordings from Trash");

    let failure=clear_history_with(pool,&recordings,json!({"test":"corrupted archive"}),|path|{
        let trashed=move_to_trash(path)?;
        fs::write(trashed.join("history.json"),"synthetic corrupted archive").map_err(|e|e.to_string())?;Ok(trashed)
    }).await;
    ensure!(failure.as_ref().err().is_some_and(|e|e.contains("校验失败")),"corrupt backup did not stop deletion");
    ensure!(app_lib::interview_sessions::load(pool,saved.id).await.is_ok() && owned.join("transcripts.json").exists(),"corrupt archive lost live records");
    println!("PASS: backup mismatch refuses database deletion and restores recordings");
    let report = clear_history_with(pool,&recordings,json!({"current_session":{"history":["synthetic answer"]},"recovery":{"meetings":["synthetic metadata"]}}),move_to_trash).await.map_err(anyhow::Error::msg)?;
    ensure!(report.recording_folders == 1 && report.database_meetings == 1, "wrong clear scope");
    ensure!(report.staged_archives==1,"failed cleanup staging was not archived with history");
    ensure!(report.saved_sessions==1,"saved session missing from clear report");
    let archive = PathBuf::from(&report.archive_path);
    let data:serde_json::Value=serde_json::from_slice(&fs::read(archive.join("history.json"))?)?;
    ensure!(data["database_rows"]["transcripts"][0]["transcript"] == "synthetic transcript", "DB backup missing content");
    let saved_payload:serde_json::Value=serde_json::from_str(data["database_rows"]["interview_sessions"][0]["payload"].as_str().unwrap())?;
    ensure!(saved_payload["history"][0]["suggestion"]=="synthetic full AI answer","saved answer missing from Trash");
    ensure!(app_lib::interview_sessions::list(pool,None).await.map_err(anyhow::Error::msg)?.sessions.is_empty(),"saved session survived clear");
    for table in ["summary_processes", "transcript_chunks", "meeting_notes"] {
        ensure!(data["database_rows"][table].as_array().unwrap().len() == 1, "related table missing from archive");
        ensure!(sqlx::query_scalar::<_,i64>(&format!("SELECT COUNT(*) FROM {table} WHERE meeting_id='owned'")).fetch_one(pool).await? == 0, "related history survived cleanup");
    }
    ensure!(data["session_and_recovery"]["current_session"]["history"][0] == "synthetic answer", "memory history backup missing");
    ensure!(archive.join("recordings/Interview practice_test/transcripts.json").exists() && !owned.exists(), "recording not moved to Trash");
    ensure!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM meetings WHERE id='owned'").fetch_one(pool).await? == 0, "meeting survived");
    ensure!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM transcripts WHERE meeting_id='owned'").fetch_one(pool).await? == 0, "transcript survived");
    ensure!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM meetings WHERE id='other'").fetch_one(pool).await? == 1, "unrelated meeting removed");
    ensure!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM settings").fetch_one(pool).await? == before_settings, "settings changed");
    ensure!(unrelated.join("keep.txt").exists() && fs::read_to_string(root.join("profile.sqlite"))? == "personal index sentinel" && fs::read_to_string(root.join("ggml-base.bin"))? == "model sentinel", "profile/model/unrelated files touched");
    let empty = clear_history_with(pool,&recordings,json!({"test":"clear again"}),move_to_trash).await.map_err(anyhow::Error::msg)?;
    ensure!(empty.recording_folders == 0 && empty.database_meetings == 0 && empty.saved_sessions == 0, "repeat clear touched unrelated records");
    println!("PASS: real macOS Trash contains complete recoverable history; only interview rows removed; profile, model, settings and unrelated meeting unchanged; repeat clear succeeds");
    fs::create_dir(&owned)?;fs::write(owned.join("metadata.json"),r#"{"meeting_name":"Interview practice","status":"recording"}"#)?;fs::write(owned.join("transcripts.json"),"new synthetic temporary speech")?;
    sqlx::query("INSERT INTO meetings(id,title,created_at,updated_at,folder_path) VALUES('temporary','Interview practice',datetime('now'),datetime('now'),?)").bind(owned.to_str().unwrap()).execute(pool).await?;
    let keep=app_lib::interview_sessions::save(pool,serde_json::from_value(saved_payload.clone())?,true).await.map_err(anyhow::Error::msg)?;
    let temporary=clear_temporary_history_with(pool,&recordings,json!({"recovery":{"transcripts":["synthetic temporary cache"]}}),move_to_trash).await.map_err(anyhow::Error::msg)?;
    ensure!(temporary.saved_sessions==0 && temporary.recording_folders==1 && temporary.database_meetings==1,"temporary cleanup scope incorrect");
    let kept=app_lib::interview_sessions::load(pool,keep.id).await.map_err(anyhow::Error::msg)?;
    ensure!(serde_json::to_value(kept)?==saved_payload,"temporary cleanup changed explicitly saved answers");
    let remaining=residue_status(pool,&recordings).await.map_err(anyhow::Error::msg)?;
    ensure!(remaining.recording_folders==0 && remaining.database_meetings==0 && remaining.staged_archives==0 && remaining.saved_sessions==1,"residue status after temporary cleanup incorrect");
    let temp_archive:serde_json::Value=serde_json::from_slice(&fs::read(PathBuf::from(&temporary.archive_path).join("history.json"))?)?;
    ensure!(temp_archive["includes_saved_sessions"]==false && temp_archive["database_rows"]["interview_sessions"].as_array().unwrap().is_empty(),"temporary cleanup included retained session payloads");
    println!("PASS: temporary-only Trash cleanup preserves explicitly saved conversation; status reports no residue and one saved session");
    pool.close().await;
    println!("Fixture retained at {}",root.display());
    Ok(())
}
