//! Synthetic-only checks for local discrepancy hints and request boundaries.
use interview_core::*;
fn source(id:&str,text:&str)->Evidence {Evidence {source_id:id.into(),path:format!("{id}.md"),root:"/synthetic/private/root".into(),hash:format!("hash-{id}"),location:"段落1".into(),text:text.into(),kind:"personal".into(),allow_external:true,score:0.0,metadata:Default::default()}}
fn fact(text:&str,confirmed:bool)->FactCard {FactCard {fact_id:"personal-fact-card".into(),version:4,previous_version:Some(3),text:text.into(),source_note:"local-only note".into(),sources:vec![],reason:"synthetic".into(),confirmed_at:confirmed.then(||"2026-11-03T00:00:00.000Z".into()),recorded_at:"2026-11-03T00:00:00.000Z".into()}}
#[test]
fn dates_compare_same_subject_field_and_shared_precision_only() {
    let a=source("A","本科预计2027年7月毕业。");let b=source("B","本科毕业时间：2026-07。");
    let conflict=detect_conflicts(&[a.clone(),b],None);assert_eq!(conflict.len(),1);assert_eq!(conflict[0].left.value,"2027-07");assert_eq!(conflict[0].right.value,"2026-07");assert_eq!(conflict[0].topic,"本科 · 毕业日期");
    for text in ["本科毕业日期：2027年","本科毕业日期：2027/07/01","硕士毕业日期：2026-07","本科入学时间：2026-07","本科毕业时间：2027年13月","本科毕业日期：2027月5","假设本科毕业时间：2026-07","本科毕业时间：2026-07？"] {
        assert!(detect_conflicts(&[a.clone(),source("B",text)],None).is_empty(),"false date conflict for {text}");
    }
    let mut practice=source("practice","本科毕业时间：2026-07");practice.kind="practice".into();assert!(detect_conflicts(&[a.clone(),practice],None).is_empty());
    let confirmed=fact("本科毕业日期：2028-07",true);let c=detect_conflicts(&[a.clone()],Some(&confirmed));assert_eq!(c.len(),1);assert_eq!(c[0].right.fact_version,Some(4));assert!(c[0].right.source_id.is_none());
    assert!(detect_conflicts(&[a],Some(&fact("本科毕业日期：2028-07",false))).is_empty(),"legacy card is not a newly confirmed counterclaim");
}
#[test]
fn explicit_ownership_and_production_claims_preserve_subject_and_negation() {
    let a=source("A","# OrbitDemo\n个人贡献：主要使用 AI 辅助编码，阅读、修改和验证代码。");
    let b=source("B","OrbitDemo个人贡献：本人独立编写全部代码。");
    let c=detect_conflicts(&[a.clone(),b],None);assert_eq!(c.len(),1);assert_eq!(c[0].category,"contribution_difference");assert_eq!(c[0].topic,"orbitdemo · 代码贡献");
    for text in ["OrbitDemo：并非独立编写全部代码，而是AI辅助开发。","OtherDemo：独立编写全部代码。","如果由我负责OrbitDemo：独立编写全部代码。","OrbitDemo：不是AI辅助编码。","OrbitDemo：禁止声称独立编写全部代码。"] {
        assert!(detect_conflicts(&[a.clone(),source("B",text)],None).is_empty(),"false ownership conflict for {text}");
    }
    let no=source("N","OrbitDB生产环境经验：没有做过生产环境测试。");let yes=source("Y","OrbitDB生产环境经验：曾负责生产环境测试。");
    let c=detect_conflicts(&[no,yes],None);assert_eq!(c.len(),1);assert_eq!(c[0].topic,"orbitdb · 生产环境经历");
    let unrelated=source("U","# OtherDemo\n个人贡献：AI辅助编码。");assert!(detect_conflicts(&[a,unrelated],None).is_empty());
}
#[test]
fn private_or_budget_excluded_counterparts_do_not_leak_into_model_hints() {
    let public=source("public","本科毕业时间：2027-07");let mut private=source("private","本科毕业时间：2026-07");private.allow_external=false;
    assert_eq!(detect_conflicts(&[public.clone(),private.clone()],None).len(),1,"local review should retain both original sources");
    let payload=messages("本科毕业时间","",&[],&[public.clone(),private]).unwrap();let body=payload[1]["content"].as_str().unwrap();
    assert!(!body.contains("2026-07"));assert!(!body.contains("private"));let data:serde_json::Value=serde_json::from_str(body).unwrap();assert!(data["source_conflicts"].as_array().unwrap().is_empty());
    let oversized=source("large",&format!("{}\n本科毕业时间：2026-07","填".repeat(6000)));
    let payload=messages("本科毕业时间","",&[],&[public.clone(),oversized]).unwrap();let data:serde_json::Value=serde_json::from_str(payload[1]["content"].as_str().unwrap()).unwrap();assert!(data["source_conflicts"].as_array().unwrap().is_empty());
    let payload=messages_with_fact_card("本科毕业时间",&fact("本科毕业时间：2028-07",true),&[],&[public],&[]).unwrap();let data:serde_json::Value=serde_json::from_str(payload[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(data["source_conflicts"].as_array().unwrap().len(),1);assert_eq!(data["source_conflicts"][0]["right"]["fact_version"],4);
    assert!(!data.to_string().contains("/synthetic"));assert!(!data.to_string().contains("local-only note"));assert!(!data.to_string().contains("hash-"));
}
#[tokio::test]
async fn broad_aliases_cannot_displace_the_only_direct_personal_boundary() {
    let root=tempfile::Builder::new().prefix("interview-alias-weight-").tempdir().unwrap().keep();std::fs::create_dir(root.join("source")).unwrap();
    let index=Index::open(&root.join("index.sqlite")).await.unwrap();let mut selected=vec![];
    std::fs::write(root.join("source/personal.md"),"OrionDB 生产测试：尚无已核实的生产经验。其他背景资料供核对。").unwrap();selected.push(Selection {path:"personal.md".into(),kind:"personal".into(),allow_external:true,structured:None});
    let brands=(0..8).map(|i|format!("cloudbrand{i}")).collect::<Vec<_>>();
    for (i,brand) in brands.iter().enumerate(){let path=format!("practice-{i}.md");std::fs::write(root.join("source").join(&path),format!("OrionDB {} 你做过生产测试吗？我做过很多测试练习，通用建议。",format!("{brand} ").repeat(12))).unwrap();selected.push(Selection {path,kind:"practice".into(),allow_external:true,structured:None});}
    for i in 0..20 {let path=format!("unrelated-{i}.md");std::fs::write(root.join("source").join(&path),format!("unrelatedsubject{i} unrelated material")).unwrap();selected.push(Selection {path,kind:"technical".into(),allow_external:true,structured:None});}
    index.import(root.join("source").to_str().unwrap(),selected,"unused").await.unwrap();
    let alias=format!("OrionDB,{}",brands.join(","));
    let hits=index.search("你做过OrionDB生产测试吗",&alias).await.unwrap();assert_eq!(hits[0].path,"personal.md");assert!(hits[0].text.contains("尚无已核实的生产经验"));assert!(hits.len()<=6);
    let expanded_only=index.search("unknownproduct","unknownproduct,OrionDB").await.unwrap();assert!(expanded_only.iter().any(|s|s.path=="personal.md"),"alias-only queries must still retrieve the direct evidence");
}
#[tokio::test]
async fn retrieval_brings_a_matching_counterpart_into_top_six_without_changing_files_or_facts() {
    let root=tempfile::Builder::new().prefix("interview-conflicts-").tempdir().unwrap().keep();std::fs::create_dir_all(root.join("source")).unwrap();
    let index=Index::open(&root.join("index.sqlite")).await.unwrap();let mut selections=vec![];
    for i in 0..8 {let path=format!("current-{i}.md");std::fs::write(root.join("source").join(&path),format!("本科毕业时间：2027年7月。\n个人记录编号{i}。")).unwrap();selections.push(Selection {path,kind:"personal".into(),allow_external:true,structured:None});}
    let old=format!("本科毕业时间：2026年7月。\n{}","其他背景内容".repeat(90));std::fs::write(root.join("source/old.md"),&old).unwrap();selections.push(Selection {path:"old.md".into(),kind:"personal".into(),allow_external:true,structured:None});
    index.import(root.join("source").to_str().unwrap(),selections,"unused").await.unwrap();
    let rank:Vec<String>=sqlx::query_scalar("SELECT c.path FROM search JOIN chunks c ON c.id=search.id WHERE search MATCH '\"本科\" OR \"毕业\" OR \"时间\"' ORDER BY bm25(search)").fetch_all(&index.pool).await.unwrap();assert!(rank.iter().position(|p|p=="old.md").unwrap()>=6);
    let version=index.setting("version").await.unwrap();let review=index.search_review("本科毕业时间","",Some(&index.fact_card().await.unwrap())).await.unwrap();
    assert!(review.evidence.len()<=6);assert!(review.evidence.iter().any(|e|e.path=="old.md"));assert!(!review.conflicts.is_empty());
    for c in &review.conflicts {for side in [&c.left,&c.right] {assert!(review.evidence.iter().any(|e|Some(&e.source_id)==side.source_id.as_ref()));}}
    assert_eq!(index.setting("version").await.unwrap(),version);assert_eq!(index.fact_card().await.unwrap().version,0);assert_eq!(std::fs::read_to_string(root.join("source/old.md")).unwrap(),old);
}
