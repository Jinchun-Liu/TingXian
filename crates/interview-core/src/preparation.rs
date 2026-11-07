//! Offline preparation is a derived, source-bound view, never personal facts.
use crate::{background::{Bundle, Doc, Profile}, digest, Index, Result};
use serde_json::{json, Value};
use sqlx::{Row, Sqlite, Transaction};
use std::collections::{BTreeMap, BTreeSet};

pub const LIMIT: usize = 16 * 1024 * 1024;
const SAVED_LIMIT:usize=32*1024*1024;
const HISTORY_LIMIT:i64=256*1024*1024;
const HISTORY_COUNT:i64=1000;
const VERSION_PREFIX:&str="preparation-version/";
const META_PREFIX:&str="preparation-meta/";
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
    let consolidated=candidate["format"]=="tingxian-preparation-v2";
    let mut fields=vec!["format","snapshot_hash","library_id","library_version","task","task_hash","role_view","stories","source_records","unselected_record_ids","accepted_inputs","ignored_stale_inputs","status","selection_status","execution_mode","model_calls_executed_by_tool","model_usage","actual_cost_cny","quality_pass","source_verified","application_imported","harness2_enabled","review_required","budget","candidate_hash"];
    if consolidated {fields.push("consolidation");}shape(candidate,&fields)?;
    let mut content=candidate.clone();content.as_object_mut().unwrap().remove("candidate_hash");
    check(hex(&candidate["candidate_hash"])?==hash(&content),"候选摘要不一致")?;
    check((consolidated || candidate["format"]=="tingxian-preparation-v1") && candidate["status"]=="candidate_requires_semantic_review" && candidate["selection_status"]==if consolidated {"consolidated_candidate"}else{"model_proposed"}, "候选包格式无效")?;
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
        for g in gaps {text(&g["text"],400,false)?;check((!consolidated && g["scope"]=="current_library") || (g["scope"]==if consolidated {"consolidated_selected"}else{"packet_only"} && input_ids.contains(hex(&g["input_hash"])?)),"缺口范围无效")?;}
    }
    let mut story_ids=BTreeSet::new();
    for story in list(&candidate["stories"],6144)? {
        let mut fields=vec!["title","activity","sections","missing","id","input_hash","scope"];if consolidated {fields.push("from_story_ids");}shape(story,&fields)?;
        text(&story["title"],120,false)?;check(story_ids.insert(hex(&story["id"])?),"重复故事卡")?;
        check(story["scope"]==if consolidated {"consolidated_group"}else{"packet_only"} && input_ids.contains(hex(&story["input_hash"])?),"故事卡范围无效")?;
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
    if consolidated {
        let parent=&candidate["consolidation"]["parent_candidate"];
        check(parent["format"]=="tingxian-preparation-v1","整合必须保留原始分块候选，不接受递归整合")?;
        validate(parent,scope)?;
        used.extend(validate_consolidation(candidate,&sources,&input_ids)?);
    }
    check(used==sources.keys().map(|s|s.to_string()).collect(),"候选包含未使用引用")?;
    Ok(())
}

fn id_set(value:&Value,limit:usize)->Result<BTreeSet<String>> {
    let rows=list(value,limit)?;let mut ids=BTreeSet::new();
    for row in rows {check(ids.insert(hex(row)?.to_owned()),"重复的整合标识")?;}Ok(ids)
}
fn validate_consolidation(candidate:&Value,sources:&BTreeMap<&str,&Value>,inputs:&BTreeSet<&str>)->Result<BTreeSet<String>> {
    let c=&candidate["consolidation"];
    shape(c,&["format","parent_candidate_hash","parent_candidate","parent_story_ids","parent_inputs","catalog_input_hash","grouping_hash","grouping","parts","conflicts","retained_source_ids","ignored_stale_inputs","budget","quality_pass"])?;
    let parent=&c["parent_candidate"];
    check(c["parent_candidate_hash"]==parent["candidate_hash"] && c["parent_inputs"]==parent["accepted_inputs"],"原始候选标识不一致")?;
    for field in ["source_records","unselected_record_ids","task","budget"] {check(candidate[field]==parent[field],"整合改变了原始选材范围或任务")?;}
    check(c["format"]=="preparation-consolidation-v1" && c["quality_pass"]==false,"整合不能自评通过")?;
    for field in ["parent_candidate_hash","catalog_input_hash","grouping_hash"] {hex(&c[field])?;}
    check(c["grouping_hash"]==hash(&json!({"format":"consolidation-groups-v1","input_hash":c["catalog_input_hash"],"groups":c["grouping"]})),"整合分组摘要无效")?;
    id_set(&c["parent_inputs"],512)?;id_set(&c["ignored_stale_inputs"],1024)?;
    let retained=id_set(&c["retained_source_ids"],32000000)?;
    check(retained==sources.keys().map(|x|x.to_string()).collect(),"整合原始证据范围不完整")?;
    let parents=id_set(&c["parent_story_ids"],6144)?;let mut grouped=BTreeSet::new();let mut group_sets=vec![];
    let original_stories=list(&parent["stories"],6144)?;
    check(parents==original_stories.iter().map(|s|s["id"].as_str().unwrap().to_owned()).collect(),"原始故事清单不一致")?;
    for g in list(&c["grouping"],512)? {
        shape(g,&["story_ids","reason"])?;text(&g["reason"],600,false)?;
        let ids=id_set(&g["story_ids"],6144)?;check(!ids.is_empty() && ids.is_subset(&parents) && ids.is_disjoint(&grouped),"故事分组重复或越界")?;
        grouped.extend(ids.clone());group_sets.push(ids);
    }
    check(grouped==parents,"整合分组遗漏故事")?;
    let b=&c["budget"];
    shape(b,&["format","parent_candidate_hash","grouping_hash","catalog_input_hash","limits","parts","catalog_input_upper","total_input_upper","planned_requests","model_calls_executed_by_tool","model_usage","actual_cost_cny","execution_profile","harness2_enabled","scope"])?;
    check(b["format"]=="consolidation-plan-v1" && b["execution_profile"]=="offline" && b["harness2_enabled"]==false && b["model_calls_executed_by_tool"]==0 && b["model_usage"].is_null() && b["actual_cost_cny"].is_null() && b["parent_candidate_hash"]==c["parent_candidate_hash"] && b["grouping_hash"]==c["grouping_hash"],"整合执行或预算状态无效")?;
    let limits=&b["limits"];shape(limits,&["context_tokens","input_upper_cap","output_reserved","safety_reserved"])?;
    let number=|v:&Value|->Result<u64>{v.as_u64().filter(|n|*n>0 && *n<=262144).ok_or("整合预算数字无效".into())};
    let context=number(&limits["context_tokens"])?;let cap=number(&limits["input_upper_cap"])?;
    check(number(&limits["output_reserved"])?+number(&limits["safety_reserved"])?+cap<=context && limits["safety_reserved"]==4096,"整合预算未保留输出或安全余量")?;
    check(b["catalog_input_hash"]==c["catalog_input_hash"] && number(&b["catalog_input_upper"])?<=cap,"分组输入预算无效")?;
    let planned=list(&b["parts"],512)?;let actual=list(&c["parts"],512)?;
    check(planned.len()==actual.len() && b["planned_requests"]==json!(planned.len()+1),"整合请求数无效")?;
    let mut total=number(&b["catalog_input_upper"])?;
    for (planned,actual) in planned.iter().zip(actual) {
        shape(planned,&["input_hash","kind","record_ids","input_upper"])?;
        for key in ["input_hash","kind","record_ids"] {check(planned[key]==actual[key],"整合计划与审计记录不一致")?;}
        let n=number(&planned["input_upper"])?;check(n<=cap,"整合输入超过声明上限")?;total+=n;
    }
    check(b["total_input_upper"]==json!(total),"整合累计估计不一致")?;
    let mut seen=BTreeSet::new();let mut covered=BTreeSet::new();let mut parent_seen=BTreeSet::new();let mut requirements=BTreeSet::new();let mut part_records=BTreeMap::new();let mut story_seen=BTreeSet::new();
    let role_view=list(&candidate["role_view"],30)?;let stories=list(&candidate["stories"],6144)?;
    for part in list(&c["parts"],512)? {
        shape(part,&["input_hash","kind","record_ids","omitted","requirement_id","input_gaps","gap_review","from_story_ids"])?;
        let h=hex(&part["input_hash"])?;check(inputs.contains(h) && seen.insert(h),"整合输入重复或无效")?;
        let available=id_set(&part["record_ids"],32000000)?;check(available.is_subset(&retained),"整合引用越界")?;
        let mut selected=BTreeSet::new();
        if part["kind"]=="requirement" {
            let id=text(&part["requirement_id"],40,false)?;check(requirements.insert(id),"重复岗位整合")?;
            let row=role_view.iter().find(|r|r["id"]==id).ok_or("未知岗位整合")?;
            let original=list(&parent["role_view"],30)?.iter().find(|r|r["id"]==id).ok_or("原始岗位要求不存在")?;
            let expected: BTreeSet<String>=list(&original["matches"],51200)?.iter().chain(list(&original["references"],51200)?).map(|r|r["record_id"].as_str().unwrap().to_owned()).collect();
            check(expected==available,"岗位整合未保留完整原始选材")?;
            let expected_gaps: BTreeSet<String>=list(&original["gaps"],6144)?.iter().enumerate().map(|(i,g)|hash(&json!([id,i,g]))).collect();
            let gap_inputs:Vec<Value>=list(&original["gaps"],6144)?.iter().enumerate().map(|(i,g)|json!({"id":hash(&json!([id,i,g])),"text":g["text"],"scope":g["scope"]})).collect();
            check(part["input_gaps"]==json!(gap_inputs),"原始缺口文字已变化")?;
            check(id_set(&part["from_story_ids"],6144)?.is_empty(),"岗位整合不能声明故事来源")?;
            for r in list(&row["matches"],51200)?.iter().chain(list(&row["references"],51200)?) {check(selected.insert(hex(&r["record_id"])?.to_owned()),"重复的整合选材")?;}
            for gap in list(&row["gaps"],6144)? {check(gap["input_hash"]==h,"岗位缺口来自不同整合输入")?;}
            let mut gap_ids=BTreeSet::new();
            for gap in list(&part["gap_review"],6144)? {
                shape(gap,&["id","status","reason","evidence_ids","output_gap"])?;check(gap_ids.insert(hex(&gap["id"])?),"重复缺口判断")?;text(&gap["reason"],600,false)?;
                let evidence=id_set(&gap["evidence_ids"],6144)?;check(evidence.is_subset(&selected),"缺口判断引用未保留证据")?;
                if gap["status"]=="retained" {let pos=gap["output_gap"].as_u64().ok_or("缺口位置无效")?;check(pos<(list(&row["gaps"],6144)?.len() as u64),"保留缺口不可消失")?;}
                else {check(gap["status"]=="resolved" && gap["output_gap"].is_null() && !evidence.is_empty() && evidence.iter().all(|rid|sources.get(rid.as_str()).is_some_and(|r|r["kind"]=="personal" || r["kind"]=="evidence")),"个人经历缺口不能由参考答案消除")?;}
            }
            check(gap_ids.iter().map(|s|s.to_string()).collect::<BTreeSet<_>>()==expected_gaps,"每个原始缺口都须明确处理")?;
        } else {
            check(part["kind"]=="story_group" && part["requirement_id"].is_null() && list(&part["gap_review"],6144)?.is_empty() && list(&part["input_gaps"],6144)?.is_empty(),"未知整合输入类型")?;
            let origins=id_set(&part["from_story_ids"],6144)?;check(group_sets.contains(&origins) && origins.is_disjoint(&parent_seen),"整合故事分组不一致")?;
            let mut expected=BTreeSet::new();
            for old in original_stories.iter().filter(|s|origins.contains(s["id"].as_str().unwrap())) {for key in SECTIONS {for f in list(&old["sections"][key],8)? {expected.extend(id_set(&f["record_ids"],12)?);}}}
            check(expected==available,"故事整合未保留完整原始证据")?;
            let mut mapped=BTreeSet::new();
            for story in stories.iter().filter(|s|s["input_hash"]==h) {
                check(story_seen.insert(hex(&story["id"])?),"故事属于多个整合输入")?;
                let ids=id_set(&story["from_story_ids"],6144)?;check(!ids.is_empty() && ids.is_subset(&origins) && ids.is_disjoint(&mapped),"故事映射缺失或重复")?;mapped.extend(ids);
                let ids=id_set(&story["from_story_ids"],6144)?;let mut allowed=BTreeSet::new();
                for old in original_stories.iter().filter(|s|ids.contains(s["id"].as_str().unwrap())) {
                    check(story["activity"]!="completed" || (old["activity"]!="planned" && old["activity"]!="in_progress"),"整合不能升级原故事完成状态")?;
                    for key in SECTIONS {for f in list(&old["sections"][key],8)? {allowed.extend(id_set(&f["record_ids"],12)?);}}
                }
                for key in SECTIONS {for f in list(&story["sections"][key],8)? {let ids=id_set(&f["record_ids"],12)?;check(ids.is_subset(&allowed),"故事借用了另一项目映射的证据")?;selected.extend(ids);}}
            }
            check(mapped==origins,"整合遗漏原故事")?;parent_seen.extend(origins);
        }
        check(selected.is_subset(&available),"整合内容借用其他输入证据")?;
        let mut omitted=BTreeSet::new();
        for row in list(&part["omitted"],6144)? {shape(row,&["record_id","reason"])?;let id=hex(&row["record_id"])?;check(available.contains(id) && !selected.contains(id) && omitted.insert(id.to_owned()),"省略证据标记无效")?;text(&row["reason"],600,false)?;}
        selected.extend(omitted);check(selected==available,"整合无理由遗漏证据")?;
        covered.extend(available.clone());part_records.insert(h,available);
    }
    check(seen==*inputs && covered==retained && parent_seen==parents && requirements.len()==role_view.len() && story_seen.len()==stories.len(),"整合范围不完整")?;
    for conflict in list(&c["conflicts"],51200)? {
        shape(conflict,&["text","record_ids","input_hash","status"])?;text(&conflict["text"],800,false)?;
        let ids=id_set(&conflict["record_ids"],6144)?;let available=part_records.get(hex(&conflict["input_hash"])?).ok_or("冲突来源输入无效")?;
        check(conflict["status"]=="unresolved" && ids.len()>=2 && ids.is_subset(available),"冲突须保留双方依据且不能自动裁定")?;
    }
    Ok(retained)
}

fn read_saved(raw:&str)->Result<Value>{
    check(raw.len()<=SAVED_LIMIT,"已存准备版本超过32 MiB")?;
    let value:Value=serde_json::from_str(raw).map_err(|_|"已存准备版本不可读；保留原记录")?;
    check(value["candidate"].is_object() && value["source_names"].is_object() && value["saved_at"].is_string() && value["index_version"].is_string(),"已存准备版本格式无效；保留原记录")?;
    Ok(value)
}
async fn latest(tx:&mut Transaction<'_,Sqlite>)->Result<Option<String>>{
    sqlx::query_scalar("SELECT value FROM background_library WHERE key='preparation'").fetch_optional(&mut **tx).await.map_err(|e|e.to_string())
}
async fn history_size(tx:&mut Transaction<'_,Sqlite>,limit:i64)->Result<i64>{
    let bytes:i64=sqlx::query_scalar("SELECT coalesce(sum(length(CAST(value AS BLOB))),0) FROM background_library WHERE key GLOB 'preparation-version/*' OR key GLOB 'preparation-meta/*'").fetch_one(&mut **tx).await.map_err(|e|e.to_string())?;
    check(bytes<=limit,"岗位准备历史达到256 MiB上限，本次未保存；旧版本保留，请先备份整理")?;Ok(bytes)
}
fn history_summary(raw:&str,saved:&Value)->Value{
    json!({"id":digest(raw.as_bytes()),"saved_at":saved["saved_at"],"role":saved["candidate"]["task"]["role"],"organization":saved["candidate"]["task"]["organization"],"candidate_hash":saved["candidate"]["candidate_hash"],"restored_from":saved["restored_from"]})
}
async fn archive_saved(tx:&mut Transaction<'_,Sqlite>,raw:&str)->Result<()> {
    let saved=read_saved(raw)?;let id=digest(raw.as_bytes());let key=format!("{VERSION_PREFIX}{id}");
    let existing:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key=?").bind(&key).fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
    if let Some(existing)=existing {check(existing==raw,"旧准备版本摘要不一致，本次未保存")?;return Ok(());}
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM background_library WHERE key GLOB 'preparation-meta/*'").fetch_one(&mut **tx).await.map_err(|e|e.to_string())?;
    check(count<HISTORY_COUNT,"岗位准备已达1000个版本，本次未保存；旧版本保留")?;
    let last:Option<String>=sqlx::query_scalar("SELECT max(key) FROM background_library WHERE key GLOB 'preparation-meta/*'").fetch_one(&mut **tx).await.map_err(|e|e.to_string())?;
    let serial=match last {Some(key)=>key.strip_prefix(META_PREFIX).and_then(|v|v.parse::<u64>().ok()).ok_or("历史序号无效")?.checked_add(1).ok_or("历史序号已达上限")?,None=>1};
    sqlx::query("INSERT INTO background_library VALUES(?,?)").bind(key).bind(raw).execute(&mut **tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO background_library VALUES(?,?)").bind(format!("{META_PREFIX}{serial:020}")).bind(history_summary(raw,&saved).to_string()).execute(&mut **tx).await.map_err(|e|e.to_string())?;
    history_size(tx,HISTORY_LIMIT).await?;Ok(())
}
async fn version_raw(tx:&mut Transaction<'_,Sqlite>,id:&str)->Result<String>{
    hex(&json!(id))?;
    let archived:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key=?").bind(format!("{VERSION_PREFIX}{id}")).fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
    let raw=match archived {Some(raw)=>raw,None=>latest(tx).await?.ok_or("没有找到准备版本")?};
    check(digest(raw.as_bytes())==id,"准备版本缺失或摘要不一致")?;read_saved(&raw)?;Ok(raw)
}
async fn saved_view(tx:&mut Transaction<'_,Sqlite>,raw:&str)->Result<Value>{
    let saved=read_saved(raw)?;let current=scope(tx).await;
    let valid=current.as_ref().is_ok_and(|s|validate(&saved["candidate"],s).is_ok());
    let previous=latest(tx).await?;let previous_hash=previous.as_ref().map(|s|digest(s.as_bytes()));let id=digest(raw.as_bytes());
    Ok(json!({"candidate":saved["candidate"],"source_names":saved["source_names"],"saved_at":saved["saved_at"],"id":id,"current":previous_hash.as_deref()==Some(id.as_str()),"restored_from":saved["restored_from"],
        "index_version":current.as_ref().ok().map(|s|s.export["index_version"].clone()),"previous_hash":previous_hash,"source_binding_valid":valid,"local_only":true,"personal_facts_confirmed":false}))
}
async fn save_current(tx:&mut Transaction<'_,Sqlite>,candidate:&Value,current:&Scope,previous:Option<&str>,restored_from:Option<&str>)->Result<()> {
    if let Some(raw)=previous {
        let old=read_saved(raw)?;
        // A repeated save is idempotent; old UI snapshots still fail CAS first.
        if old["candidate"]==*candidate {return Ok(());}
        archive_saved(tx,raw).await?;
    }
    let saved=json!({"candidate":candidate,"source_names":source_names(current),"saved_at":chrono::Utc::now().to_rfc3339(),"index_version":current.export["index_version"],"restored_from":restored_from}).to_string();
    archive_saved(tx,&saved).await?;
    sqlx::query("INSERT OR REPLACE INTO background_library VALUES('preparation',?)").bind(saved).execute(&mut **tx).await.map_err(|e|e.to_string())?;Ok(())
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
        save_current(&mut tx,candidate,&current,previous.as_deref(),None).await?;
        tx.commit().await.map_err(|e|e.to_string())
    }
    pub async fn preparation_view(&self) -> Result<Value> {
        let _guard=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let raw: Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='preparation'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        let Some(raw)=raw else{return Ok(Value::Null)};
        // Retain old local history for inspection; never register it in FTS,
        // fact cards, settings or model input. Eligibility is checked afresh.
        saved_view(&mut tx,&raw).await
    }
    pub async fn preparation_history(&self,before:Option<&str>)->Result<Value>{
        if let Some(cursor)=before {check(cursor.len()==20 && cursor.bytes().all(|b|b.is_ascii_digit()),"历史分页标识无效")?;}
        let _guard=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let current=latest(&mut tx).await?;let current_hash=current.as_ref().map(|raw|digest(raw.as_bytes()));
        let rows=sqlx::query("SELECT key,value FROM background_library WHERE key GLOB 'preparation-meta/*' AND (? IS NULL OR key < ?) ORDER BY key DESC LIMIT 21").bind(before).bind(before.map(|v|format!("{META_PREFIX}{v}"))).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
        let more=rows.len()>20;let cursor=rows.get(19).map(|r|r.get::<String,_>("key").strip_prefix(META_PREFIX).map(str::to_owned).ok_or("历史分页无效")).transpose()?;let mut values=vec![];
        for row in rows.into_iter().take(20){let raw:String=row.get("value");let mut value:Value=serde_json::from_str(&raw).map_err(|_|"版本摘要不可读")?;value["current"]=json!(value["id"].as_str()==current_hash.as_deref());values.push(value);}
        let legacy=if before.is_none() {if let Some(raw)=&current {
            let present:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM background_library WHERE key=?)").bind(format!("{VERSION_PREFIX}{}",current_hash.as_ref().unwrap())).fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
            if present {Value::Null}else{let mut summary=history_summary(raw,&read_saved(raw)?);summary["current"]=true.into();summary}
        } else {Value::Null}}else{Value::Null};
        let bytes=history_size(&mut tx,i64::MAX).await?;
        Ok(json!({"rows":values,"legacy_current":legacy,"next_before":if more {cursor}else{None},"bytes":bytes,"limit_bytes":HISTORY_LIMIT}))
    }
    pub async fn preparation_version_view(&self,id:&str)->Result<Value>{
        let _guard=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let raw=version_raw(&mut tx,id).await?;saved_view(&mut tx,&raw).await
    }
    pub async fn preparation_version_export(&self,id:&str)->Result<Value>{
        let _guard=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        // Export for inspection is allowed even when its sources are stale.
        let raw=version_raw(&mut tx,id).await?;Ok(read_saved(&raw)?["candidate"].clone())
    }
    pub async fn preparation_restore(&self,id:&str,version:&str,previous_hash:Option<&str>)->Result<Value>{
        let _guard=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let raw=version_raw(&mut tx,id).await?;let saved=read_saved(&raw)?;let current=scope(&mut tx).await?;
        check(current.export["index_version"]==version,"资料库已变化，请重新查看版本")?;
        validate(&saved["candidate"],&current)?;
        let previous=latest(&mut tx).await?;check(previous.as_ref().map(|s|digest(s.as_bytes())).as_deref()==previous_hash,"当前准备已变化，请重新查看版本")?;
        save_current(&mut tx,&saved["candidate"],&current,previous.as_deref(),Some(id)).await?;
        let restored=latest(&mut tx).await?.ok_or("恢复版本不存在")?;let view=saved_view(&mut tx,&restored).await?;
        tx.commit().await.map_err(|e|e.to_string())?;Ok(view)
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    #[tokio::test]
    async fn storage_limit_is_bytes_not_character_count(){
        let dir=tempfile::tempdir().unwrap();let index=Index::open(&dir.path().join("index.sqlite")).await.unwrap();let mut tx=index.pool.begin().await.unwrap();
        sqlx::query("INSERT INTO background_library VALUES('preparation-version/test','中文')").execute(&mut *tx).await.unwrap();
        assert!(history_size(&mut tx,5).await.is_err());assert_eq!(history_size(&mut tx,6).await.unwrap(),6);
        tx.rollback().await.unwrap();assert!(index.preparation_history(None).await.unwrap()["rows"].as_array().unwrap().is_empty());
    }
}
