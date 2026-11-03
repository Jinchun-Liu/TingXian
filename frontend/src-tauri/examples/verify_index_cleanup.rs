//! Uses only a newly created synthetic fixture, then the real macOS Trash API.
use anyhow::{ensure,Result};
use app_lib::{interview_index::clear_index_with,interview_history::move_to_trash};
use interview_core::{Index,Selection};
use std::{fs,path::PathBuf};
#[tokio::main]
async fn main()->Result<()> {
    let root=PathBuf::from(std::env::args().nth(1).expect("new synthetic fixture directory required"));
    ensure!(!root.exists(),"fixture directory must be new");
    fs::create_dir_all(root.join("source"))?;fs::create_dir(root.join("app"))?;
    fs::write(root.join("source/synthetic.md"),"syntheticneedle 契约测试原文件")?;
    fs::write(root.join("app/session.json"),"synthetic history remains")?;
    let index=Index::open(&root.join("app/profile.sqlite")).await.map_err(anyhow::Error::msg)?;
    let selection=vec![Selection {path:"synthetic.md".into(),kind:"technical".into(),allow_external:false,structured:None}];
    index.import(root.join("source").to_str().unwrap(),selection.clone(),"unused").await.map_err(anyhow::Error::msg)?;
    index.correct_facts(interview_core::FactCorrection {expected_version:0,text:"synthetic confirmed fact".into(),source_note:"Synthetic fixture, not real personal data".into(),sources:vec![],reason:"Synthetic fixture setup".into(),confirmed:true}).await.map_err(anyhow::Error::msg)?;
    let doc=index.documents().await.map_err(anyhow::Error::msg)?.remove(0);
    index.update_document(interview_core::DocumentUpdate {path:doc.path,expected_root:doc.root,expected_hash:doc.hash,confirmed:true,valid_from:None,valid_until:Some("2026-12-31".into())}).await.map_err(anyhow::Error::msg)?;
    let report=clear_index_with(&index,&root.join("app"),move_to_trash).await.map_err(anyhow::Error::msg)?;
    ensure!(report.documents==1 && report.chunks==1 && report.metadata_records==1 && report.version==3,"unexpected clear scope/version");
    let archive=PathBuf::from(report.archive_path.unwrap());
    let backup:serde_json::Value=serde_json::from_slice(&fs::read(archive.join("index.json"))?)?;
    ensure!(backup["kind"]=="meetily-profile-index-backup" && backup["database_rows"]["chunks"].as_array().unwrap().len()==1,"missing archived content");
    ensure!(backup["format_version"]==2 && backup["database_rows"]["document_metadata"][0]["confirmed"]==1,"missing archived review");
    ensure!(index.search("syntheticneedle","").await.map_err(anyhow::Error::msg)?.is_empty(),"retrieval cache survived");
    ensure!(index.setting("facts").await.map_err(anyhow::Error::msg)?=="synthetic confirmed fact","facts removed");
    ensure!(fs::read_to_string(root.join("source/synthetic.md"))?=="syntheticneedle 契约测试原文件","source changed");
    ensure!(fs::read_to_string(root.join("app/session.json"))?=="synthetic history remains","history changed");
    let rebuilt=index.import(root.join("source").to_str().unwrap(),selection,"unused").await.map_err(anyhow::Error::msg)?;
    ensure!(rebuilt.success==1 && rebuilt.unchanged==0 && rebuilt.version==4,"rebuild did not repopulate cache");
    ensure!(index.documents().await.map_err(anyhow::Error::msg)?[0].metadata.confirmation==interview_core::Confirmation::Unreviewed,"cleared review leaked into re-import");
    println!("PASS: real macOS Trash backup verified; retrieval cache cleared; original, facts and history preserved; re-import succeeded");
    println!("Synthetic backup retained at {}",archive.display());
    println!("Synthetic fixture retained at {}",root.display());
    Ok(())
}
