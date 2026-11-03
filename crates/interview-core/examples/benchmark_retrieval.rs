//! Bounded synthetic benchmark of the same retrieval library used by Tauri.
//! Generation and measurement run separately; no user files, audio or network.
use interview_core::*;
use serde_json::{json,Value};
use std::{collections::HashSet,fs,io::Write,path::Path,time::Instant};
type BenchResult<T>=std::result::Result<T,Box<dyn std::error::Error>>;
const DOCS:usize=2_000;
const PER_DOC:usize=10;
const ROOT:&str="/synthetic/benchmark-profile";
const MARKER:&str="meetily-synthetic-retrieval-20000-v1";
const TOPICS:[&str;10]=[
    "DemoProject 契约测试使用 FastAPI、pytest 与 JSON Schema，分别检查缺失字段、null、数字零和空字符串，不把练习解释为真实项目贡献。",
    "Doris 与 ExampleDB 云产品测试包含数据正确性、存算分离、FE/BE、Tablet、副本一致性和故障恢复；技术说明不能证明实际生产经验。",
    "接口重试通过幂等键避免重复写入，需要验证请求超时、网络抖动、并发冲突、重复提交和唯一约束，检查返回值与最终业务状态。",
    "SQL 用 LEFT JOIN、GROUP BY 和窗口函数验证聚合结果，注意事务隔离、脏读、不可重复读、幻读、死锁以及回滚后的数据一致性。",
    "离线扩缩容研究 autoscaling 比较 P95 延迟、吞吐量、资源利用率与成本，保留异常值、基线、时间泄漏和实验限制。",
    "测试性能需要记录 QPS、TPS、P50/P95/P99、并发数、CPU、内存、IO、缓存命中率，并区分响应慢和排队时间。",
    "JSON Schema 通过只代表结构与类型符合约束；仍应检查业务语义、时间顺序、币种单位、负数、空值、精度和关联字段一致性。",
    "故障注入覆盖节点重启、网络分区、复制延迟、进程退出和磁盘空间不足，恢复时检查幂等性、重试次数及告警信息。",
    "权限测试验证用户角色、读写范围、越权、会话过期、访问拒绝与审计记录；测试输入必须使用独立的合成账号与数据。",
    "日志排查结合请求ID、状态码、时间戳、错误堆栈与资源指标，先重现问题，再检查最小失败条件，避免把相关性当作因果。",
];
fn template(topic:&str)->String {
    let filler="接口测试先明确输入、前置条件、预期结果与检查位置。边界场景覆盖空值、重复请求、乱序数据、超时和部分失败；结果需要可重现，并保留观察记录。数据校验应区分格式、业务规则和状态变化，测试说明不代表真实经历。";
    let mut text=topic.to_owned();
    while text.chars().count()<700 {text.push('\n');text.push_str(filler);}text
}
fn write_new(path:&Path,value:&Value)->BenchResult<()> {
    let mut file=fs::OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;file.sync_all()?;Ok(())
}
async fn generate(root:&Path)->BenchResult<()> {
    if root.exists(){return Err("generation directory must be new; old fixtures are retained".into());}fs::create_dir(root)?;
    let started=Instant::now();let index=Index::open(&root.join("profile.sqlite")).await?;
    let templates=TOPICS.iter().map(|t|template(t)).collect::<Vec<_>>();
    let tokens=templates.iter().map(|t|index.tokens(t).join(" ")).collect::<Vec<_>>();
    let mut total_chars=0usize;let mut tx=index.pool.begin().await?;
    for doc in 0..DOCS {
        let path=format!("doc-{doc:06}.md");let hash=digest(format!("{MARKER}:document:{doc}").as_bytes());
        let kind=match doc%10 {0=>"personal",1=>"evidence",2=>"practice",3=>"job",_=>"technical"};
        sqlx::query("INSERT INTO documents(path,hash,kind,allow_external) VALUES(?,?,?,0)").bind(&path).bind(&hash).bind(kind).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO document_metadata(path,root,reviewed_hash,confirmed,confirmed_at,valid_from,valid_until,updated_at) VALUES(?,?,?,?,?,'2026-01-01',?,'2026-11-03T00:00:00.000Z')")
            .bind(&path).bind(ROOT).bind(&hash).bind(doc%2==0).bind(if doc%2==0{Some("2026-11-03T00:00:00.000Z")}else{None}).bind(if doc%4==0{"2025-12-31"}else{"2026-12-31"}).execute(&mut *tx).await?;
        for part in 0..PER_DOC {
            let topic=(doc+part)%TOPICS.len();let id=format!("SB{doc:06}_{part:02}");
            let mut header=format!("# BenchProject{doc:06}\n合成记录{part}，不代表真实个人经历。");
            if ["personal","evidence"].contains(&kind) {header.push_str(&format!("\n本科毕业时间：{}年7月。\nOrionDB生产环境经验：没有做过生产环境测试。",2026+(part%2)));}
            let text=format!("{header}\n{}",templates[topic]);let indexed=format!("{} {}",index.tokens(&header).join(" "),tokens[topic]);
            if doc<2 {assert_eq!(indexed,index.tokens(&text).join(" "),"cached template tokenization differs from production tokenizer");}
            total_chars+=text.chars().count();
            sqlx::query("INSERT INTO chunks(id,path,location,text) VALUES(?,?,?,?)").bind(&id).bind(&path).bind(format!("synthetic segment {}",part+1)).bind(text).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO search(id,tokens) VALUES(?,?)").bind(&id).bind(indexed).execute(&mut *tx).await?;
        }
        if (doc+1)%200==0 {println!("synthetic corpus: {} / {} chunks",(doc+1)*PER_DOC,DOCS*PER_DOC);}
    }
    sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES('root',?),('version','1'),('aliases',?)").bind(ROOT).bind("示例项目,DemoProject\n扩缩容,autoscaling").execute(&mut *tx).await?;
    tx.commit().await?;
    let mut fact_text="本科毕业时间：2028年7月。\nOrionDB生产环境经验：没有做过生产环境测试。\n".to_string();while fact_text.chars().count()<2000{fact_text.push_str("这是独立合成基准资料，只用于性能测量，不代表任何真实人物、项目或工作经历。\n");}
    let hash=digest(format!("{MARKER}:document:0").as_bytes());
    let sources=(0..6).map(|part|FactSourceRef{source_id:format!("SB000000_{part:02}"),root:ROOT.into(),path:"doc-000000.md".into(),hash:hash.clone()}).collect();
    index.correct_facts(FactCorrection {expected_version:0,text:fact_text,source_note:"Synthetic benchmark provenance; never user data".into(),sources,reason:"Synthetic benchmark setup".into(),confirmed:true}).await?;
    let integrity:String=sqlx::query_scalar("PRAGMA integrity_check").fetch_one(&index.pool).await?;if integrity!="ok"{return Err(integrity.into());}
    index.pool.close().await;
    write_new(&root.join("benchmark.json"),&json!({"marker":MARKER,"documents":DOCS,"chunks":DOCS*PER_DOC,"total_characters":total_chars,"mean_chunk_characters":total_chars/(DOCS*PER_DOC),"database_bytes":fs::metadata(root.join("profile.sqlite"))?.len(),"generation_ms":started.elapsed().as_millis(),"scope":"synthetic SQL corpus using production tokenizer; does not measure directory import performance"}))?;
    println!("PASS: generated {} synthetic chunks; fixture retained",DOCS*PER_DOC);Ok(())
}
fn distribution(samples:&[f64])->Value {
    let mut sorted=samples.to_vec();sorted.sort_by(f64::total_cmp);
    let percentile=|p:f64|sorted[((sorted.len() as f64*p).ceil() as usize).saturating_sub(1).min(sorted.len()-1)];
    json!({"count":sorted.len(),"p50_ms":percentile(0.5),"p95_ms":percentile(0.95),"p99_ms":percentile(0.99),"max_ms":sorted.last(),"mean_ms":sorted.iter().sum::<f64>()/sorted.len() as f64})
}
async fn measure(root:&Path,output:&Path)->BenchResult<()> {
    let manifest:Value=serde_json::from_slice(&fs::read(root.join("benchmark.json"))?)?;
    if manifest["marker"]!=MARKER || !root.join("profile.sqlite").is_file(){return Err("not a completed synthetic benchmark fixture".into());}
    if output.exists(){return Err("output must be new; previous measurements are retained".into());}
    let started=Instant::now();let index=Index::open(&root.join("profile.sqlite")).await?;let open_ms=started.elapsed().as_secs_f64()*1000.0;
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM chunks").fetch_one(&index.pool).await?;assert_eq!(count,20_000);
    let cases=[
        ("exact_project","BenchProject001234"),
        ("common_terms","接口 测试"),
        ("alias_mixed","示例项目 契约测试"),
        ("cloud_alias","Doris 云产品数据正确性测试"),
        ("technical_negation","JSON Schema 通过为什么仍不能证明业务结果正确"),
        ("personal_boundary","你做过 OrionDB 生产环境测试吗"),
        ("date_conflicts","本科毕业时间"),
        ("sql_literals","接口测试 \" OR * -- (超时)"),
        ("no_match","zzzzunmatchabletoken"),
        ("wide_query","Doris ExampleDB DemoProject FastAPI pytest JSON Schema SQL null 幂等键 接口 契约测试 重试 超时 并发 隔离 副本 一致性 节点 网络 CPU 内存 IO 磁盘 缓存 窗口函数 GROUP BY LEFT JOIN QPS TPS P50 P95 P99 autoscaling 吞吐量 资源 权限 日志 事务 回滚 负数 精度 空字符串"),
    ];
    let mut records=vec![];
    for round in 0..17 { // first round separate; 160 subsequent queries
        for offset in 0..cases.len() {
            let case=(round*3+offset)%cases.len();let (name,question)=cases[case];assert!(question.chars().count()<=1500);
            let now=Instant::now();let aliases=index.setting("aliases").await?;let facts=index.fact_card().await?;
            let review=index.search_review(question,&aliases,Some(&facts)).await?;
            let serialized=serde_json::to_vec(&review)?;let ms=now.elapsed().as_secs_f64()*1000.0;
            assert!(review.evidence.len()<=6);assert!(review.conflicts.len()<=24);
            let ids=review.evidence.iter().map(|e|e.source_id.as_str()).collect::<HashSet<_>>();
            for conflict in &review.conflicts {for side in [&conflict.left,&conflict.right] {if let Some(id)=&side.source_id {assert!(ids.contains(id.as_str()));}else{assert_eq!(side.fact_version,Some(facts.version));}}}
            match name {
                "exact_project"=>assert!(review.evidence.iter().all(|e|e.path=="doc-001234.md") && !review.evidence.is_empty()),
                "no_match"=>assert!(review.evidence.is_empty()),
                "alias_mixed"=>assert!(review.evidence.iter().any(|e|e.text.contains("DemoProject"))),
                "personal_boundary"=>assert!(review.evidence.iter().any(|e|["personal","evidence"].contains(&e.kind.as_str())&&e.text.contains("没有做过生产环境测试"))),
                "date_conflicts"=>assert!(!review.conflicts.is_empty()),
                _=>assert!(!review.evidence.is_empty()),
            }
            records.push(json!({"round":round,"case":name,"elapsed_ms":ms,"evidence_count":review.evidence.len(),"evidence_ids":review.evidence.iter().map(|e|&e.source_id).collect::<Vec<_>>(),"conflicts":review.conflicts.len(),"response_bytes":serialized.len()}));
        }
        if round%4==0 {println!("retrieval rounds: {} / 17",round+1);}
    }
    let all=records.iter().map(|r|r["elapsed_ms"].as_f64().unwrap()).collect::<Vec<_>>();
    let steady=records.iter().filter(|r|r["round"]!=0).map(|r|r["elapsed_ms"].as_f64().unwrap()).collect::<Vec<_>>();
    let first=records.iter().filter(|r|r["round"]==0).map(|r|r["elapsed_ms"].as_f64().unwrap()).collect::<Vec<_>>();
    let groups=cases.iter().map(|(name,_)|{let values=records.iter().filter(|r|r["case"]==*name && r["round"]!=0).map(|r|r["elapsed_ms"].as_f64().unwrap()).collect::<Vec<_>>();((*name).to_owned(),distribution(&values))}).collect::<serde_json::Map<String,Value>>();
    let summary=distribution(&all);let met=summary["p95_ms"].as_f64().unwrap()<=200.0;
    write_new(output,&json!({"created_at":chrono::Utc::now().to_rfc3339(),"architecture":std::env::consts::ARCH,"os":std::env::consts::OS,"debug_build":cfg!(debug_assertions),"scope":"synthetic standalone retrieval; includes alias/fact reads, exact core search_review and serialization; no Tauri event transport, ASR, model request or Feishu load; OS file cache is not flushed","manifest":manifest,"open_ms":open_ms,"all_queries":summary,"first_process_round":distribution(&first),"subsequent_queries":distribution(&steady),"per_case_subsequent":groups,"p95_target_ms":200,"target_met_for_this_workload":met,"records":records}))?;
    println!("queries={}, P50={:.2} ms, P95={:.2} ms, target_met={met}",all.len(),summary["p50_ms"].as_f64().unwrap(),summary["p95_ms"].as_f64().unwrap());Ok(())
}
#[tokio::main]
async fn main()->BenchResult<()> {
    let args=std::env::args().collect::<Vec<_>>();match args.get(1).map(String::as_str) {
        Some("generate") if args.len()==3=>generate(Path::new(&args[2])).await,
        Some("measure") if args.len()==4=>measure(Path::new(&args[2]),Path::new(&args[3])).await,
        _=>Err("Usage: benchmark_retrieval generate NEW_DIRECTORY | measure SYNTHETIC_DIRECTORY NEW_RESULT_JSON".into()),
    }
}
