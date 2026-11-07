//! Query-only synthetic timing. No application DB, model or capture access.
use interview_core::{Index,source_policy::{EXCLUDED_SQL,EXCLUSION_INDEX_SQL}};
use serde_json::{json,Value};
use std::time::Instant;
fn stats(mut v:Vec<f64>)->Value {v.sort_by(f64::total_cmp);json!({"samples":v.len(),"p50_ms":v[v.len()/2],"p95_ms":v[(v.len()*95).div_ceil(100)-1]})}
#[tokio::main]
async fn main()->Result<(),String> {
    if std::env::args().len()!=1 {return Err("synthetic only; no database/API arguments".into());}
    let dir=tempfile::Builder::new().prefix("tingxian-exclusion-timing-").tempdir().map_err(|e|e.to_string())?.keep();
    let mut results=vec![];
    for (count,renamed) in [(0,false),(1,false),(10,false),(100,false),(100,true)] {
        let index=Index::open(&dir.join(format!("{count}-{renamed}.sqlite"))).await?;
        let mut tx=index.pool.begin().await.map_err(|e|e.to_string())?;
        for i in 1..=2000 {
            let prefix=format!(".背景/{i:064x}/");let name=if renamed&&i<=count{"renamed.md"}else{"source.md"};
            sqlx::query("INSERT INTO documents VALUES(?,'synthetic','personal',1)").bind(format!("{prefix}{name}")).execute(&mut *tx).await.map_err(|e|e.to_string())?;
            if i<=count {sqlx::query("INSERT INTO background_library VALUES(?,?)").bind(format!("excluded/legacy-{i}")).bind(json!({"root":"背景资料库","path":format!("{prefix}source.md"),"excluded":true}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;}
        }
        tx.commit().await.map_err(|e|e.to_string())?;
        let query=format!("SELECT d.path FROM documents d WHERE NOT {EXCLUDED_SQL}");let mut row=json!({"documents":2000,"exclusions":count,"renamed":renamed});
        // Identical correct predicate, identical DB. Compare only index use.
        for indexed in [false,true] {
            if indexed {sqlx::raw_sql(EXCLUSION_INDEX_SQL).execute(&index.pool).await.map_err(|e|e.to_string())?;} else {sqlx::query("DROP INDEX background_exclusion_identity").execute(&index.pool).await.map_err(|e|e.to_string())?;}
            let mut times=vec![];
            for sample in 0..16 {let start=Instant::now();let rows:Vec<String>=sqlx::query_scalar(&query).fetch_all(&index.pool).await.map_err(|e|e.to_string())?;if rows.len()!=2000-count{return Err("wrong source scope".into());}if sample>0{times.push(start.elapsed().as_secs_f64()*1000.0);}}
            row[if indexed{"indexed"}else{"unindexed"}]=stats(times);
        }
        results.push(row);index.pool.close().await;
    }
    println!("{}",json!({"scope":"Rust debug/sqlx warm SQLite: full 2000-document permission predicate only; unindexed first, 1 warmup + 15 samples; excludes FTS, IPC, ASR, model, import and process startup", "fixture_root":dir,"api_calls":0,"results":results}));Ok(())
}
