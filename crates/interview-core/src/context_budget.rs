//! Text estimates are UTF-8 byte bounds, never claimed as provider tokenization.
use serde::{Deserialize,Serialize};
use serde_json::Value;
use crate::Result;
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextBudget {pub context_tokens:u64,pub max_output_tokens:u64,pub safety_tokens:u64}
impl Default for ContextBudget {fn default()->Self{Self{context_tokens:262144,max_output_tokens:8192,safety_tokens:2048}}}
impl ContextBudget {
    pub fn window(&self)->u64 {self.context_tokens.min(262144)}
    pub fn output(&self,requested:u64)->u64 {requested.min(self.max_output_tokens)}
    pub fn validate(&self)->Result<()> {
        if !(4096..=2_097_152).contains(&self.context_tokens)||!(16..=65536).contains(&self.max_output_tokens)||!(256..=16384).contains(&self.safety_tokens)
            || self.max_output_tokens+self.safety_tokens>=self.window(){return Err("上下文、输出与余量配置无效：输出预留和余量必须小于模型窗口及256K应用上限".into());}Ok(())
    }
    pub fn fits(&self,input:u64,requested_output:u64)->bool{input.saturating_add(self.output(requested_output)).saturating_add(self.safety_tokens)<=self.window()}
}

/// Drop low-ranked evidence first, then old conversation. Never crop the current
/// question, confirmed facts, conflict descriptions or the newest spoken turns.
pub fn fit_text(messages:&mut Value,budget:&ContextBudget,output:u64)->Result<bool>{
    budget.validate()?;
    let rows=messages.as_array_mut().ok_or("模型消息格式无效")?;
    let mut images=false;
    for row in rows.iter_mut(){
        if let Some(parts)=row["content"].as_array(){
            if parts.iter().any(|p|p["type"]=="image_url"){images=true;}
        }
    }
    let size=|rows:&Vec<Value>|->u64{
        rows.iter().map(|r|match &r["content"]{
            Value::String(s)=>s.len() as u64,
            Value::Array(parts)=>parts.iter().filter_map(|p|p["text"].as_str()).map(|s|s.len() as u64).sum(),
            _=>0,
        }+64).sum::<u64>()+128
    };
    loop{
        if budget.fits(size(rows),output){return Ok(images);}
        let mut trimmed=false;
        for row in rows.iter_mut().rev(){
            if row["role"]!="user"{continue;}
            // Only our structured conversation payload is eligible for pruning.
            let content=if row["content"].is_string(){Some(&mut row["content"])}else{row["content"].as_array_mut().and_then(|parts|parts.iter_mut().find(|p|p["type"]=="text")).map(|p|&mut p["text"])};
            let Some(content)=content else{continue};
            let Some(mut context)=content.as_str().and_then(|s|serde_json::from_str::<Value>(s).ok()).filter(|c|c["current_question"].is_string())else{continue};
            if let Some(items)=context["evidence"].as_array_mut(){
                let before=items.len();let mut seen=std::collections::HashSet::new();
                items.retain(|item|seen.insert(item.to_string()));trimmed=before!=items.len();
            }
            for (field,keep,last) in [("evidence",0,true),("recent_turns",0,false),("recent_spoken_dialogue",2,false)]{
                if trimmed{break;}
                if let Some(items)=context[field].as_array_mut(){if items.len()>keep{if last{items.pop();}else{items.remove(0);}trimmed=true;break;}}
            }
            if trimmed{*content=Value::String(context.to_string());break;}
        }
        if !trimmed{return Err("当前问题、必要背景和输出预留超过模型窗口；请调整模型上下文/输出预算或精简输入，未发送".into());}
    }
}

#[cfg(test)]mod tests{
    use super::*;use serde_json::json;
    #[test]fn small_window_prunes_only_optional_context_and_preserves_constraints(){
        let budget=ContextBudget{context_tokens:8192,max_output_tokens:1024,safety_tokens:512};
        let question="不能把 null 和 0 混用；分三种失败状态验证。";
        let mut input=json!([{"role":"system","content":"规则"},{"role":"user","content":json!({"current_question":question,"user_confirmed_facts":"仅参与模拟测试","source_conflicts":["新旧材料职责不一致"],"evidence":[{"text":"参考".repeat(3000)}],"recent_turns":[],"recent_spoken_dialogue":[{"text":"我的实际方法"}]}).to_string()}]);
        assert!(!fit_text(&mut input,&budget,4096).unwrap());
        let user:Value=serde_json::from_str(input[1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(user["current_question"],question);assert_eq!(user["user_confirmed_facts"],"仅参与模拟测试");assert_eq!(user["evidence"],json!([]));assert_eq!(user["source_conflicts"][0],"新旧材料职责不一致");
        input[1]["content"]=json!({"current_question":"限制".repeat(10000)}).to_string().into();assert!(fit_text(&mut input,&budget,4096).is_err());
    }
    #[test]fn application_ceiling_and_unknown_images_are_separate(){
        let b=ContextBudget{context_tokens:1_048_576,..Default::default()};assert_eq!(b.window(),262144);assert!(!b.fits(262144,1));
        let edge=b.window()-b.output(4096)-b.safety_tokens;assert!(b.fits(edge,4096));assert!(!b.fits(edge+1,4096));
        let mut input=json!([{"role":"user","content":[{"type":"text","text":"图片中的条件"},{"type":"image_url","image_url":{"url":"data:image/png;base64,synthetic"}}]}]);
        assert!(fit_text(&mut input,&b,4096).unwrap());assert_eq!(input[0]["content"][1]["type"],"image_url");
    }
}
