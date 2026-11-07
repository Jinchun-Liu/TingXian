//! Local synthetic timing only. Never opens the application database or a model.
use interview_core::{background::{Bundle, Doc, Profile, Record}, Index};
use serde_json::{json, Value};
use std::time::Instant;

fn distribution(mut times: Vec<f64>) -> Value {
    times.sort_by(f64::total_cmp);
    json!({"samples":times.len(),"p50_ms":times[times.len()/2],
        "p95_ms":times[(times.len()*95).div_ceil(100)-1]})
}

async fn measure(index: &Index) -> Result<Value, String> {
    let mut gates = vec![];
    let mut queries = vec![];
    for sample in 0..35 {
        let started = Instant::now();
        if !index.profile_available().await? {return Err("synthetic profile unexpectedly stale".into());}
        let gate = started.elapsed().as_secs_f64()*1000.0;
        let started = Instant::now();
        if index.search("接口契约测试", "").await?.is_empty() {return Err("synthetic retrieval empty".into());}
        if sample >= 5 {
            gates.push(gate);
            queries.push(started.elapsed().as_secs_f64()*1000.0);
        }
    }
    Ok(json!({"profile_gate":distribution(gates),"fts_search":distribution(queries)}))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    if std::env::args().len()!=1 {return Err("no path/API flags; synthetic fixtures only".into());}
    let root = tempfile::Builder::new().prefix("tingxian-dependency-timing-")
        .tempdir().map_err(|e|e.to_string())?.keep();
    let mut results = vec![];
    for size in [10,100,1000,2000] {
        eprintln!("Preparing {size} synthetic sources");
        let index = Index::open(&root.join(format!("{size}.sqlite"))).await?;
        let bundle = Bundle {
            format:"meetily-background-v1".into(),
            documents:(0..size).map(|i|Doc {
                id:format!("{:064x}",i+1),name:format!("fictional-{i}.txt"),hash:"f".repeat(64),
                records:vec![Record {kind:"personal".into(),location:"行1".into(),
                    quote:format!("虚构资料 {i}：接口契约测试只在模拟环境验证，尚无生产经验。"),
                    note:"完全虚构，仅用于本地计时".into()}],
            }).collect(),
            profile:Profile {overview:"虚构综合背景：接口契约测试仍需核对，不能主张生产经历。".into(),role:"虚构测试岗".into(),aliases:String::new()},
            settings:None,
        };
        let import_start=Instant::now();
        index.apply_background(&bundle,&index.setting("version").await?,true,Value::Null,false).await?;
        let import_ms=import_start.elapsed().as_secs_f64()*1000.0;
        eprintln!("Measuring {size} synthetic sources; import {import_ms:.1} ms");
        // Same corpus and process. Temporarily select the legacy fallback only
        // in this new synthetic database; retain every row for reproducibility.
        sqlx::query("UPDATE background_library SET key='saved_dependencies' WHERE key='profile_dependencies'")
            .execute(&index.pool).await.map_err(|e|e.to_string())?;
        let legacy = measure(&index).await?;
        sqlx::query("UPDATE background_library SET key='profile_dependencies' WHERE key='saved_dependencies'")
            .execute(&index.pool).await.map_err(|e|e.to_string())?;
        let current = measure(&index).await?;
        results.push(json!({"source_documents":size,"synthetic_import_ms":import_ms,"legacy_fallback":legacy,"source_dependencies":current}));
    }
    println!("{}",json!({"schema":"background-dependency-timing-v1","fixture_root":root,
        "scope":"local warm SQLite, 5 warmups then 30 samples per mode; legacy first; one synthetic record per source; excludes extraction, process startup, ASR, network and model",
        "model_calls":0,"results":results}));
    Ok(())
}
