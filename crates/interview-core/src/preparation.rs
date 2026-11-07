//! Offline preparation is a derived, source-bound view, never personal facts.
use crate::{background::{Bundle, Doc, Profile}, digest, Index, Result};
use serde_json::{json, Value};
use sqlx::{Row, Sqlite, Transaction};
use std::collections::{BTreeMap, BTreeSet};

pub const LIMIT: usize = 16 * 1024 * 1024;
const LIBRARY: &str = "preparation_library_id";
const SECTIONS: [&str; 5] = ["context", "task", "action", "result", "learning"];

// Python uses sort_keys=True. Feature-unified serde_json may preserve insertion
// order; an explicit recursive writer keeps cross-host identities stable.
pub fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map.iter().collect();
            format!("{{{}}}", sorted.into_iter().map(|(k,v)| format!("{}:{}", serde_json::to_string(k).unwrap(), canonical(v))).collect::<Vec<_>>().join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(",")),
        _ => value.to_string(),
    }
}
pub fn hash(value: &Value) -> String { digest(canonical(value).as_bytes()) }
fn check(ok: bool, message: &str) -> Result<()> { if ok {Ok(())} else {Err(message.into())} }
fn text(v: &Value, max: usize, empty: bool) -> Result<&str> {
    let s=v.as_str().ok_or("准备资料文本无效")?;
    check((empty || !s.trim().is_empty()) && s.chars().count()<=max && !crate::contains_credential(s)
        && !s.chars().any(|c|c.is_control() && !matches!(c,'\n'|'\t'|'\r')), "准备资料文本过长或包含无效内容")?;
    Ok(s)
}
fn list(v: &Value, max: usize) -> Result<&Vec<Value>> {
    let a=v.as_array().ok_or("准备资料列表无效")?; check(a.len()<=max,"准备资料列表过长")?; Ok(a)
}
fn shape(v: &Value, fields: &[&str]) -> Result<()> {
    let obj=v.as_object().ok_or("准备资料结构无效")?;
    check(obj.len()==fields.len() && fields.iter().all(|k|obj.contains_key(*k)),"准备资料包含未知或缺少字段")
}
fn hex(v: &Value) -> Result<&str> {
    let s=text(v,64,false)?;
    check(s.len()==64 && s.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),"准备资料摘要无效")?; Ok(s)
}

struct Scope { export: Value, records: BTreeMap<String,Value>, snapshot: String }
fn source_names(scope: &Scope) -> Value {
    let names: BTreeMap<_,_>=scope.export["bundle"]["documents"].as_array().unwrap().iter().map(|d|(d["id"].as_str().unwrap(),d["name"].as_str().unwrap())).collect();
    json!(names)
}
async fn scope(tx: &mut Transaction<'_,Sqlite>) -> Result<Scope> {
    let id: String=sqlx::query_scalar("SELECT value FROM settings WHERE key=?").bind(LIBRARY).fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?.ok_or("请先从本机导出岗位准备资料")?;
    let current=crate::source_policy::CURRENT_SQL; let excluded=crate::source_policy::EXCLUDED_SQL;
    let rows=sqlx::query(&format!("SELECT b.value,d.hash FROM background_library b JOIN documents d ON d.path='.背景/'||b.key||'/'||json_extract(b.value,'$.name') WHERE length(b.key)=64 AND b.key NOT GLOB '*[^a-f0-9]*' AND {current} AND NOT {excluded} ORDER BY b.key"))
        .fetch_all(&mut **tx).await.map_err(|e|e.to_string())?;
    let mut docs=vec![]; let mut native_hashes=vec![];
    for row in rows {
        let doc: Doc=serde_json::from_str(&row.get::<String,_>("value")).map_err(|_|"当前来源不可读")?;
        let native: String=row.get("hash");
        check(digest(&serde_json::to_vec(&doc).map_err(|e|e.to_string())?)==native,"当前提取版本不一致")?;
        docs.push(doc);native_hashes.push(native);
    }
    let bundle=Bundle{format:"meetily-background-v1".into(),documents:docs,profile:Profile{overview:"当前有效来源；岗位准备候选，非已确认事实。".into(),role:String::new(),aliases:String::new()},settings:None};
    if !bundle.documents.is_empty() {bundle.validate()?;}
    let bundle=serde_json::to_value(bundle).map_err(|e|e.to_string())?;
    let mut records=vec![];let mut revised=vec![];let mut revisions=vec![];
    for (doc,native) in bundle["documents"].as_array().unwrap().iter().zip(native_hashes) {
        let path=format!(".背景/{}/{}",doc["id"].as_str().unwrap(),doc["name"].as_str().unwrap());
        for (position,r) in doc["records"].as_array().unwrap().iter().enumerate() {
            let rid=hash(&json!([id,doc["id"],doc["hash"],position,r]));
            let mut record=json!({"record_id":rid,"namespace":if r["kind"]=="personal" || r["kind"]=="evidence" {"user_facts"} else {"task_context"},"kind":r["kind"],"claim":r["quote"],"extraction_note":r["note"],"claim_status":"source_asserted","activity_status":"unknown","source_refs":[{"source_id":doc["id"],"source_hash":doc["hash"],"extraction_revision":hash(doc),"record_index":position,"locator":r["location"],"quote":r["quote"]}],"excluded":false,"manual_revision":null});
            let chunk=format!("B{}",&digest(format!("{path}:{native}:{position}").as_bytes())[..24]);
            let raw: Option<String>=sqlx::query_scalar(&format!("SELECT a.value FROM chunks c JOIN documents d ON d.path=c.path {} WHERE c.id=?",crate::assertions::JOIN)).bind(chunk).fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?.flatten();
            if let Some(a)=crate::assertions::parse(raw)? {
                let revision=json!({"record_id":rid,"revision_id":format!("{}:{}",a.record_id,a.revision),"claim":a.claim,"basis":a.reason,"activity":a.activity});
                record["superseded"]=json!(true);
                let mut updated=record.clone();
                updated["record_id"]=json!(hash(&json!([rid,revision])));
                updated["claim"]=revision["claim"].clone();updated["claim_status"]=json!("user_asserted");
                updated["activity_status"]=revision["activity"].clone();updated["superseded"]=json!(false);
                updated["manual_revision"]=json!({"id":revision["revision_id"],"basis":revision["basis"],"supersedes":rid});
                revisions.push(revision);revised.push(updated);
            }
            records.push(record);
        }
    }
    records.extend(revised);
    let snapshot=hash(&json!({"schema_version":"tingxian-fact-view-2","library_id":id,"library_version":hash(&bundle),"validity_hash":hash(&json!([])),"records":records}));
    let records=records.into_iter().filter(|r|r["superseded"]!=true).map(|r|(r["record_id"].as_str().unwrap().to_owned(),r)).collect();
    let version: String=sqlx::query_scalar("SELECT value FROM settings WHERE key='version'").fetch_one(&mut **tx).await.map_err(|e|e.to_string())?;
    let export=json!({"format":"tingxian-preparation-scope-v1","library_id":id,"index_version":version,"bundle":bundle,"revisions":revisions,"excluded":[]});
    check(export.to_string().len()<=LIMIT,"准备资料超过16 MiB，请先缩小资料范围")?;
    Ok(Scope{export,records,snapshot})
}

fn validate(candidate: &Value, scope: &Scope) -> Result<()> {
    check(candidate.to_string().len()<=LIMIT,"候选包超过16 MiB")?;
    check(!crate::contains_credential(&candidate.to_string()),"候选包含凭证格式")?;
    shape(candidate,&["format","snapshot_hash","library_id","library_version","task","task_hash","role_view","stories","source_records","unselected_record_ids","accepted_inputs","ignored_stale_inputs","status","selection_status","execution_mode","model_calls_executed_by_tool","model_usage","actual_cost_cny","quality_pass","source_verified","application_imported","harness2_enabled","review_required","budget","candidate_hash"])?;
    let mut content=candidate.clone();content.as_object_mut().unwrap().remove("candidate_hash");
    check(hex(&candidate["candidate_hash"])?==hash(&content),"候选摘要不一致")?;
    check(candidate["format"]=="tingxian-preparation-v1" && candidate["status"]=="candidate_requires_semantic_review" && candidate["selection_status"]=="model_proposed", "候选包格式无效")?;
    check(candidate["library_id"]==scope.export["library_id"] && candidate["library_version"]==hash(&scope.export["bundle"]) && candidate["snapshot_hash"]==scope.snapshot,"资料库、来源或本人修订已变化；请重新导出并整理")?;
    for key in ["quality_pass","source_verified","application_imported","harness2_enabled"] {check(candidate[key]==false,"候选包不能自行确认事实或启用优化")?;}
    check(candidate["budget"]["execution_profile"]=="offline" && candidate["budget"]["harness2_enabled"]==false
        && candidate["budget"]["model_calls_executed_by_tool"]==0 && candidate["budget"]["snapshot_hash"]==candidate["snapshot_hash"],"候选预算或执行范围无效")?;
    check(["fixture","host_model","unreported"].contains(&candidate["execution_mode"].as_str().unwrap_or("")) && candidate["model_calls_executed_by_tool"]==0 && candidate["model_usage"].is_null() && candidate["actual_cost_cny"].is_null(),"候选执行状态无效")?;
    let task=&candidate["task"];shape(task,&["role","organization","requirements"])?;
    text(&task["role"],120,false)?;text(&task["organization"],120,true)?;
    check(candidate["task_hash"]==hash(task),"岗位目标摘要无效")?;
    let requirements=list(&task["requirements"],30)?;check(!requirements.is_empty(),"岗位要求为空")?;
    let mut requirement_text=BTreeSet::new();
    for (i,r) in requirements.iter().enumerate() {shape(r,&["id","text"])?;check(r["id"]==format!("req-{}",i+1),"岗位要求编号无效")?;check(requirement_text.insert(text(&r["text"],600,false)?),"岗位要求重复")?;}
    let mut sources=BTreeMap::new();
    for r in list(&candidate["source_records"],32000000)? {
        let id=hex(&r["record_id"])?;
        check(scope.records.get(id)==Some(r),"引用与本机当前原文或本人修订不一致")?;
        check(sources.insert(id,r).is_none(),"重复引用记录")?;
    }
    let mut partition: BTreeSet<&str>=sources.keys().copied().collect();
    for id in list(&candidate["unselected_record_ids"],32000000)? {check(partition.insert(hex(id)?),"重复的未选记录")?;}
    check(partition==scope.records.keys().map(String::as_str).collect(),"候选没有完整说明当前资料范围")?;
    let mut used=BTreeSet::new();
    let mut cite=|value:&Value,personal:bool|->Result<Value>{let id=text(value,64,false)?;let r=sources.get(id).ok_or("未找到候选引用")?;check(!personal || r["kind"]=="personal" || r["kind"]=="evidence","练习或参考资料不能成为个人经历")?;used.insert(id.to_owned());Ok((*r).clone())};
    let inputs=list(&candidate["accepted_inputs"],512)?;let mut input_ids=BTreeSet::new();
    for id in inputs {check(input_ids.insert(hex(id)?),"重复的整理块")?;}
    for id in list(&candidate["ignored_stale_inputs"],1024)? {hex(id)?;}
    for r in list(&candidate["review_required"],12)? {text(r,400,false)?;}
    let view=list(&candidate["role_view"],30)?;check(view.len()==requirements.len(),"岗位要求未完整覆盖")?;
    for (row,requirement) in view.iter().zip(requirements) {
        shape(row,&["id","text","matches","references","gaps"])?;
        check(row["id"]==requirement["id"] && row["text"]==requirement["text"],"岗位视图与目标不一致")?;
        let matches=list(&row["matches"],51200)?;let mut seen=BTreeSet::new();
        for m in matches {shape(m,&["record_id","relevance","reason"])?;cite(&m["record_id"],true)?;check(seen.insert(hex(&m["record_id"])?),"重复匹配")?;check(["direct","supporting"].contains(&m["relevance"].as_str().unwrap_or("")),"相关性标记无效")?;text(&m["reason"],600,false)?;}
        for r in list(&row["references"],51200)? {shape(r,&["record_id","purpose"])?;cite(&r["record_id"],false)?;text(&r["purpose"],600,false)?;}
        let gaps=list(&row["gaps"],6144)?;check(!matches.is_empty() || !gaps.is_empty(),"缺少个人证据时须列出缺口")?;
        for g in gaps {text(&g["text"],400,false)?;check(g["scope"]=="current_library" || (g["scope"]=="packet_only" && input_ids.contains(hex(&g["input_hash"])?)),"缺口范围无效")?;}
    }
    let mut story_ids=BTreeSet::new();
    for story in list(&candidate["stories"],6144)? {
        shape(story,&["title","activity","sections","missing","id","input_hash","scope"])?;
        text(&story["title"],120,false)?;check(story_ids.insert(hex(&story["id"])?),"重复故事卡")?;
        check(story["scope"]=="packet_only" && input_ids.contains(hex(&story["input_hash"])?),"故事卡范围无效")?;
        check(["unknown","planned","in_progress","completed"].contains(&story["activity"].as_str().unwrap_or("")),"故事完成状态无效")?;
        shape(&story["sections"],&SECTIONS)?;
        let missing=story["missing"].as_object().ok_or("故事缺口格式无效")?;
        check(missing.keys().all(|k|SECTIONS.contains(&k.as_str())),"未知故事字段")?;
        let mut activities=BTreeSet::new();
        for section in SECTIONS {
            let fragments=list(&story["sections"][section],8)?;
            if fragments.is_empty() {text(&story["missing"][section],400,false)?;} else {check(!missing.contains_key(section),"故事字段与缺口冲突")?;}
            for fragment in fragments {
                shape(fragment,&["text","record_ids"])?;text(&fragment["text"],800,false)?;
                let refs=list(&fragment["record_ids"],12)?;check(!refs.is_empty(),"故事片段缺少引用")?;
                for id in refs {activities.insert(cite(id,true)?["activity_status"].as_str().unwrap().to_owned());}
            }
        }
        check(!activities.is_empty(),"空故事不能作为经历")?;
        check(story["activity"]!="completed" || (!activities.contains("planned") && !activities.contains("in_progress")),"未完成经历不能升为完成")?;
        check(story["activity"]=="unknown" || activities!=BTreeSet::from(["unknown".into()]),"未知完成状态须保留")?;
    }
    check(used==sources.keys().map(|s|s.to_string()).collect(),"候选包含未使用引用")?;
    Ok(())
}

impl Index {
    pub async fn preparation_export(&self) -> Result<Value> {
        let _guard=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        sqlx::query("INSERT OR IGNORE INTO settings VALUES(?,lower(hex(randomblob(32))))").bind(LIBRARY).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        let result=scope(&mut tx).await?;
        check(!result.records.is_empty(),"请先导入有效背景资料")?;
        tx.commit().await.map_err(|e|e.to_string())?;Ok(result.export)
    }
    pub async fn preparation_preview(&self, candidate: &Value) -> Result<Value> {
        let _guard=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let current=scope(&mut tx).await?;validate(candidate,&current)?;
        let previous: Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='preparation'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        Ok(json!({"candidate":candidate,"source_names":source_names(&current),"index_version":current.export["index_version"],"previous_hash":previous.map(|v|digest(v.as_bytes())),"source_binding_valid":true}))
    }
    pub async fn preparation_save(&self,candidate:&Value,version:&str,previous_hash:Option<&str>) -> Result<()> {
        let _guard=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let current=scope(&mut tx).await?;
        check(current.export["index_version"]==version,"资料库已变化，请重新核对")?;validate(candidate,&current)?;
        let previous: Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='preparation'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        check(previous.as_ref().map(|v|digest(v.as_bytes())).as_deref()==previous_hash,"已保存的准备资料已变化，请重新核对")?;
        let saved=json!({"candidate":candidate,"source_names":source_names(&current),"saved_at":chrono::Utc::now().to_rfc3339(),"index_version":version});
        sqlx::query("INSERT OR REPLACE INTO background_library VALUES('preparation',?)").bind(saved.to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        tx.commit().await.map_err(|e|e.to_string())
    }
    pub async fn preparation_view(&self) -> Result<Value> {
        let _guard=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let raw: Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='preparation'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        let Some(raw)=raw else{return Ok(Value::Null)};
        let saved: Value=serde_json::from_str(&raw).map_err(|_|"准备资料不可读")?;
        let valid=match scope(&mut tx).await {Ok(s)=>validate(&saved["candidate"],&s).is_ok(),Err(_)=>false};
        // Retain old local history for inspection; never register it in FTS,
        // fact cards, settings or model input. Eligibility is checked afresh.
        Ok(json!({"candidate":saved["candidate"],"source_names":saved["source_names"],"saved_at":saved["saved_at"],"source_binding_valid":valid,"local_only":true,"personal_facts_confirmed":false}))
    }
}
