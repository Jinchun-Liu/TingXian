//! Isolated real-filesystem + real-SQLite + macOS Trash regression. No user data.
use anyhow::{ensure, Result};
use app_lib::{database::manager::DatabaseManager, interview_history::{clear_history_with, move_to_trash}};
use serde_json::json;
use std::{fs, path::PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).expect("fixture directory required"));
    ensure!(!root.exists(), "fixture directory must be new");
    fs::create_dir_all(&root)?;
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
    let before_settings:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings").fetch_one(pool).await?;
    let failure = clear_history_with(pool,&recordings,json!({"test":"unavailable Trash"}), |_|Err("simulated Trash failure".into())).await;
    ensure!(failure.is_err() && owned.join("transcripts.json").exists(), "Trash failure lost recording");
    ensure!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM meetings").fetch_one(pool).await? == 2, "Trash failure cleared DB");
    println!("PASS: failed Trash restores recordings and leaves database untouched");

    sqlx::query("CREATE TRIGGER refuse_test_delete BEFORE DELETE ON meetings WHEN OLD.id='owned' BEGIN SELECT RAISE(ABORT,'test rejection'); END").execute(pool).await?;
    let failure = clear_history_with(pool,&recordings,json!({"test":"DB rollback"}),move_to_trash).await;
    ensure!(failure.is_err() && owned.join("transcripts.json").exists(), "DB failure did not restore recording");
    ensure!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM transcripts WHERE meeting_id='owned'").fetch_one(pool).await? == 1, "DB failure did not roll back transcripts");
    sqlx::query("DROP TRIGGER refuse_test_delete").execute(pool).await?;
    println!("PASS: database rejection rolls back rows and restores moved recordings from Trash");

    let report = clear_history_with(pool,&recordings,json!({"current_session":{"history":["synthetic answer"]},"recovery":{"meetings":["synthetic metadata"]}}),move_to_trash).await.map_err(anyhow::Error::msg)?;
    ensure!(report.recording_folders == 1 && report.database_meetings == 1, "wrong clear scope");
    let archive = PathBuf::from(&report.archive_path);
    let data:serde_json::Value=serde_json::from_slice(&fs::read(archive.join("history.json"))?)?;
    ensure!(data["database_rows"]["transcripts"][0]["transcript"] == "synthetic transcript", "DB backup missing content");
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
    ensure!(empty.recording_folders == 0 && empty.database_meetings == 0, "repeat clear touched unrelated records");
    println!("PASS: real macOS Trash contains complete recoverable history; only interview rows removed; profile, model, settings and unrelated meeting unchanged; repeat clear succeeds");
    pool.close().await;
    println!("Fixture retained at {}",root.display());
    Ok(())
}
