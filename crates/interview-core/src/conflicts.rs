//! Bounded, local hints for explicit date/ownership discrepancies. These are
//! candidates for human review, not a semantic truth classifier.
use crate::{digest,Evidence,FactCard};
use regex::Regex;
use serde::{Deserialize,Serialize};
use std::{collections::HashSet,sync::OnceLock};

#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct ConflictSide {
    pub source_id:Option<String>, pub fact_version:Option<i64>,
    pub path:String, pub location:String, pub quote:String, pub value:String,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct SourceConflict {
    pub id:String, pub topic:String, pub category:String, pub reason:String,
    pub left:ConflictSide, pub right:ConflictSide,
}
#[derive(Debug,Clone,Serialize)]
pub struct RetrievalReview {pub evidence:Vec<Evidence>,pub conflicts:Vec<SourceConflict>}
#[derive(Clone)]
struct Claim {subject:String,field:String,value:String,date:Option<(i32,Option<u32>,Option<u32>)>,quote:String}
fn compact(text:&str)->String {text.chars().filter(|c|!c.is_whitespace() && !"*`[]【】".contains(*c)).collect::<String>().to_lowercase()}
fn subject(text:&str)->String {
    let mut s=compact(text).replace("预计","").replace("计划","");
    for prefix in ["候选人","本人","我的","我"] {if let Some(rest)=s.strip_prefix(prefix){s=rest.into();}}
    if let Some(rest)=s.strip_suffix("项目"){s=rest.into();}
    s.trim_matches(['的','：',':','-']).to_owned()
}
struct Patterns {date_label:Regex,date_after:Regex,ownership:Regex,no_ai:Regex,production_yes:Regex,production_no:Regex}
fn patterns()->&'static Patterns {
    static P:OnceLock<Patterns>=OnceLock::new();P.get_or_init(|| {
        let d=r"(?P<year>19\d{2}|20\d{2}|21\d{2})(?:[-/.年](?P<month>\d{1,2}))?(?:[-/.月](?P<day>\d{1,2}))?(?:日|月|年)?";
        Patterns {
            date_label:Regex::new(&format!(r"^(?P<subject>[^：:。；;]{{0,60}}?)(?P<field>毕业|入职|到岗|离职|入学|开始|结束)(?:时间|日期)?\s*[：:]\s*(?:预计|计划|为|是)?\s*{d}")).unwrap(),
            date_after:Regex::new(&format!(r"^(?P<subject>[^：:。；;]{{0,60}}?)(?:预计|计划|将于|于)\s*{d}\s*(?P<field>毕业|入职|到岗|离职|入学)" )).unwrap(),
            ownership:Regex::new(r"(?P<negative>不是|不曾|不能|并非|没有|未)?(?:由)?(?:本人|我)?(?:独立|自主)(?:手写|编写|实现|完成|开发).{0,8}(?:全部|所有|整个).{0,8}(?:代码|项目|系统)").unwrap(),
            no_ai:Regex::new(r"(?:没有|未|不是|并非|不)(?:使用|借助|采用)?ai辅助").unwrap(),
            production_yes:Regex::new(r"(?:曾|已|实际).{0,8}(?:负责|参与|执行|做过).{0,12}(?:生产|线上)").unwrap(),
            production_no:Regex::new(r"(?:没有|未|从未|不曾).{0,8}(?:生产|线上)").unwrap(),
        }
    })
}
fn date(c:&regex::Captures<'_>)->Option<(i32,Option<u32>,Option<u32>)> {
    let year=c.name("year")?.as_str().parse().ok()?;
    let month=c.name("month").map(|m|m.as_str().parse::<u32>()).transpose().ok()?;
    let day=c.name("day").map(|d|d.as_str().parse::<u32>()).transpose().ok()?;
    if day.is_some() && month.is_none(){return None;}
    chrono::NaiveDate::from_ymd_opt(year,month.unwrap_or(1),day.unwrap_or(1))?;
    Some((year,month,day))
}
fn claims(text:&str)->Vec<Claim> {
    let p=patterns();let mut result=vec![];let mut heading=String::new();
    for raw in text.lines() {
        let line=raw.trim().trim_start_matches(['-','•',' ']);
        if line.starts_with('#') {
            heading=subject(line.trim_start_matches('#'));
            if ["个人背景","个人信息","基本信息","教育背景","教育经历","项目经历","个人事实","简介"].contains(&heading.as_str()) {heading.clear();}
            continue;
        }
        for raw in line.split(['。','；',';','\n']) {
            let quote=raw.trim();if quote.is_empty() || quote.chars().count()>300 {continue;}
            if ["?","？","如果","假设","若","是否","如何","应该","建议","禁止","不能声称","不得","反例","例如"].iter().any(|s|quote.contains(s)) {continue;}
            let normalized=compact(quote);
            if let Some(c)=p.date_label.captures(&normalized).or_else(||p.date_after.captures(&normalized)) {
                if let Some(value)=date(&c) {
                    let mut scope=subject(c.name("subject").map(|s|s.as_str()).unwrap_or(""));if scope.is_empty(){scope=heading.clone();}if scope.is_empty(){if ["开始","结束"].contains(&c.name("field").unwrap().as_str()){continue;}scope="本人".into();}
                    let rendered=match value {(y,Some(m),Some(d))=>format!("{y:04}-{m:02}-{d:02}"),(y,Some(m),None)=>format!("{y:04}-{m:02}"),(y,_,_)=>format!("{y:04}")};
                    result.push(Claim {subject:scope,field:format!("{}日期",c.name("field").unwrap().as_str()),value:rendered,date:Some(value),quote:quote.into()});
                }
            }
            let (prefix,value)=normalized.split_once(['：',':']).unwrap_or(("",normalized.as_str()));
            let prefix=prefix.trim_end_matches("个人贡献").trim_end_matches("本人贡献").trim_end_matches("我的贡献").trim_end_matches("开发方式").trim_end_matches("实现方式").trim_end_matches("代码贡献").trim_end_matches("生产环境经验").trim_end_matches("生产测试经验").trim_end_matches("生产经验");
            let mut scope=subject(prefix);if scope.is_empty(){scope=heading.clone();}
            if scope.is_empty(){continue;} // No project/entity: do not compare unrelated contributions.
            let mut ownership=HashSet::new();
            for capture in p.ownership.captures_iter(value) {ownership.insert(if capture.name("negative").is_some(){"非独立完成全部代码"}else{"独立完成全部代码"});}
            let ownership_context=["贡献","开发","实现","代码","编码","编程"].iter().any(|s|normalized.contains(s));
            if ownership_context && value.contains("ai辅助") && !p.no_ai.is_match(value) {ownership.insert("非独立完成全部代码");}
            if ownership.len()==1 {result.push(Claim {subject:scope.clone(),field:"代码贡献".into(),value:ownership.into_iter().next().unwrap().into(),date:None,quote:quote.into()});}
            let explicit_production=normalized.split_once(['：',':']).is_some_and(|(label,_)|["生产环境经验","生产测试经验","生产经验"].iter().any(|s|label.ends_with(s)));
            let yes=p.production_yes.is_match(value)||(explicit_production && ["有","已有"].contains(&value));
            let no=p.production_no.is_match(value)||(explicit_production && ["无","没有","暂无"].contains(&value));
            if yes!=no {result.push(Claim {subject:scope,field:"生产环境经历".into(),value:if yes{"有生产环境经历"}else{"未有生产环境经历"}.into(),date:None,quote:quote.into()});}
            if result.len()>=16 {return result;}
        }
    }result
}
fn differs(a:&Claim,b:&Claim)->bool {
    if a.subject!=b.subject || a.field!=b.field {return false;}
    match (a.date,b.date) {
        (Some((ay,am,ad)),Some((by,bm,bd)))=>ay!=by || am.zip(bm).is_some_and(|(a,b)|a!=b) || ad.zip(bd).is_some_and(|(a,b)|a!=b),
        (None,None)=>a.value!=b.value,
        _=>false,
    }
}
pub fn detect_conflicts(evidence:&[Evidence],facts:Option<&FactCard>)->Vec<SourceConflict> {
    let mut all=vec![];
    for e in evidence.iter().take(40).filter(|e|["personal","evidence"].contains(&e.kind.as_str())) {
        for claim in claims(e.current_text()) {
            let side=ConflictSide {source_id:Some(e.source_id.clone()),fact_version:None,path:e.path.clone(),location:e.location.clone(),quote:claim.quote.clone(),value:claim.value.clone()};all.push((claim,side));
        }
    }
    if let Some(facts)=facts.filter(|f|f.confirmed_at.is_some()) {
        for claim in claims(&facts.text) {
            let side=ConflictSide {source_id:None,fact_version:Some(facts.version),path:"本人确认事实卡".into(),location:format!("v{}",facts.version),quote:claim.quote.clone(),value:claim.value.clone()};all.push((claim,side));
        }
    }
    let mut result=vec![];let mut seen=HashSet::new();
    for (i,(left,a)) in all.iter().enumerate() {
        for (right,b) in all.iter().skip(i+1) {
            if a.source_id==b.source_id && a.fact_version==b.fact_version || !differs(left,right) {continue;}
            let key=format!("{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{}",a.source_id,a.fact_version,b.source_id,b.fact_version,left.subject,left.field,left.value,right.value);
            if !seen.insert(key.clone()){continue;}
            result.push(SourceConflict {id:format!("C{}",&digest(key.as_bytes())[..16]),topic:format!("{} · {}",left.subject,left.field),category:if left.date.is_some(){"date_difference"}else{"contribution_difference"}.into(),reason:if left.date.is_some(){"同一主题出现不同日期，可能是计划或版本差异，请核对来源"}else{"同一主题的个人贡献或生产经历表述不同，请核对本人实际情况"}.into(),left:a.clone(),right:b.clone()});
            if result.len()==24 {return result;}
        }
    }result
}
pub(crate) fn choose_evidence(mut candidates:Vec<Evidence>,question:&str,facts:Option<&FactCard>)->RetrievalReview {
    // Experience questions need their factual boundaries as well as wording
    // examples. Reserve up to three relevant personal/evidence candidates;
    // hypothetical technical design questions keep the normal relevance order.
    let q=compact(question);
    let personal=["做过","经历","个人贡献","你的贡献","独立","在读","毕业","到岗","入职","你的项目","介绍项目","介绍一下项目","研究结果","论文指标"].iter().any(|s|q.contains(s))
        && !["假设","如果","会怎么","如何设计","怎么设计"].iter().any(|s|q.contains(s));
    if personal {
        let anchors=q.split(|c:char|!c.is_ascii_alphanumeric() && c!='_').filter(|s|s.len()>=3 && s.chars().any(char::is_alphabetic)).collect::<Vec<_>>();
        let priority=candidates.iter().filter(|e|["personal","evidence"].contains(&e.kind.as_str()) && (anchors.is_empty() || anchors.iter().any(|a|compact(e.current_text()).contains(a)))).take(3).map(|e|e.source_id.clone()).collect::<HashSet<_>>();
        candidates.sort_by_key(|e|!priority.contains(&e.source_id));
    }
    let conflicts=detect_conflicts(&candidates,facts);let query=compact(question);let mut selected=vec![];let mut ids=HashSet::new();
    for evidence in &candidates {
        if selected.len()==6 {break;}
        if !ids.insert(evidence.source_id.clone()){continue;}selected.push(evidence.clone());
        for conflict in &conflicts {
            let topic_subject=conflict.topic.split(" · ").next().unwrap_or("");
            let relevant=(!topic_subject.is_empty() && topic_subject!="本人" && query.contains(topic_subject)) || ["毕业","到岗","入职","学历","教育","背景","贡献","负责","经历"].iter().any(|s|query.contains(s));
            if !relevant {continue;}
            let partner=if conflict.left.source_id.as_deref()==Some(&evidence.source_id) {conflict.right.source_id.as_ref()} else if conflict.right.source_id.as_deref()==Some(&evidence.source_id) {conflict.left.source_id.as_ref()} else {None};
            if selected.len()<6 {if let Some(partner)=partner.and_then(|id|candidates.iter().find(|e|&e.source_id==id)) {if ids.insert(partner.source_id.clone()){selected.push(partner.clone());}}}
        }
    }
    let conflicts=detect_conflicts(&selected,facts);
    RetrievalReview {evidence:selected,conflicts}
}

/// Only expose differences whose original excerpts are included in this exact
/// request. No paths, private-only counterparts or hidden source counts.
pub(crate) fn model_conflicts(conflicts:&[SourceConflict],included:&HashSet<&str>)->Vec<serde_json::Value> {
    let visible=|s:&ConflictSide|s.source_id.as_deref().map(|id|included.contains(id)).unwrap_or(s.fact_version.is_some());
    let side=|s:&ConflictSide|serde_json::json!({"source_id":s.source_id,"fact_version":s.fact_version,"value":s.value});
    conflicts.iter().filter(|c|visible(&c.left)&&visible(&c.right)).take(6).map(|c|serde_json::json!({"topic":c.topic,"category":c.category,"left":side(&c.left),"right":side(&c.right),"status":"potential_difference_requires_review"})).collect()
}
