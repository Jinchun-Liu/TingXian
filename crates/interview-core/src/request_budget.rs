//! Admission for the final wire payload. Serialized UTF-8 bytes are a local
//! conservative proxy, not a provider tokenizer or certification of its window.
use crate::{context_budget::ContextBudget, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum WindowBasis { ConfiguredUnknown, UserReviewed }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestMeasurement {
    pub estimator: String,
    pub window_basis: WindowBasis,
    pub window_tokens: u64,
    pub serialized_text_bytes: u64,
    pub image_count: u64,
    pub image_reserve: Option<u64>,
    pub output_tokens: u64,
    pub safety_tokens: u64,
    pub hard_input_budget: u64,
    pub complete_local_estimate: bool,
    pub within_known_budget: bool,
}

/// `image_reserve` is an explicit total estimate for all images, never the
/// base64 byte count. None means unknown and must remain visible to callers.
pub fn measure(payload: &Value, budget: &ContextBudget, basis: WindowBasis, image_reserve: Option<u64>) -> Result<RequestMeasurement> {
    budget.validate()?;
    let output=payload["max_tokens"].as_u64().filter(|n|*n>0 && *n<=budget.max_output_tokens)
        .ok_or("请求的实际输出预留与配置不一致；未发送")?;
    let mut counted=payload.clone();
    let mut images=0;
    let rows=counted["messages"].as_array_mut().ok_or("模型消息格式无效")?;
    for row in rows {
        if let Some(parts)=row["content"].as_array_mut() {
            for part in parts {
                if part["type"]=="image_url" {
                    if !part["image_url"]["url"].is_string() {return Err("图片输入格式无效；未发送".into());}
                    images+=1;
                    part["image_url"]["url"]=Value::String("[image accounted separately]".into());
                }
            }
        }
    }
    // Includes role/name, tools, JSON schema, escaping, model and protocol fields.
    // The safety reserve covers the provider's hidden framing and proxy error.
    let bytes=serde_json::to_vec(&counted).map_err(|e|e.to_string())?.len() as u64;
    let reserve=if images==0 {Some(0)} else {image_reserve};
    if images>0 && reserve==Some(0) {return Err("图片不能以零用量通过预算检查".into());}
    let hard=budget.window().checked_sub(output).and_then(|n|n.checked_sub(budget.safety_tokens))
        .ok_or("生成预留和安全余量超过模型窗口；未发送")?;
    Ok(RequestMeasurement {
        estimator:"serialized-utf8-bytes-v1-not-tokenizer".into(), window_basis:basis,
        window_tokens:budget.window(), serialized_text_bytes:bytes, image_count:images,
        image_reserve:reserve, output_tokens:output, safety_tokens:budget.safety_tokens,
        hard_input_budget:hard, complete_local_estimate:reserve.is_some(),
        within_known_budget:bytes.saturating_add(reserve.unwrap_or(0))<=hard,
    })
}

pub fn admit(m:&RequestMeasurement)->Result<()> {
    if m.serialized_text_bytes.saturating_add(m.image_reserve.unwrap_or(0))>m.hard_input_budget {return Err("当前问题、必要背景与生成预留超过本地估算窗口；未发送，请减少输入或调整模型".into());}
    if !m.complete_local_estimate && m.serialized_text_bytes>=m.hard_input_budget.saturating_mul(3)/4 {
        return Err("图片用量未知，文字已接近输入预算；未发送，请减少可选上下文或核对图片预算".into());
    }
    Ok(())
}

/// Only optional evidence/history is removed. The current question, facts,
/// conflicts, newest two spoken turns, and every image remain byte-for-byte.
pub fn fit(payload:&mut Value,budget:&ContextBudget,basis:WindowBasis,image_reserve:Option<u64>)->Result<RequestMeasurement> {
    for _ in 0..1024 {
        let m=measure(payload,budget,basis.clone(),image_reserve)?;
        if admit(&m).is_ok() {return Ok(m);}
        if !prune_optional(&mut payload["messages"]) {admit(&m)?;}
    }
    Err("可选上下文过多，预算裁剪未收敛；未发送".into())
}

pub(crate) fn prune_optional(messages:&mut Value)->bool {
    let Some(rows)=messages.as_array_mut() else{return false;};
    for row in rows.iter_mut().rev() {
        if row["role"]!="user" {continue;}
        let content=if row["content"].is_string() {Some(&mut row["content"])} else {
            row["content"].as_array_mut().and_then(|parts|parts.iter_mut().find(|p|p["type"]=="text")).map(|p|&mut p["text"])
        };
        let Some(content)=content else {continue;};
        let Some(mut context)=content.as_str().and_then(|s|serde_json::from_str::<Value>(s).ok()).filter(|c|c["current_question"].is_string()) else {continue;};
        for (field,keep,last) in [("evidence",0,true),("session_memory",0,true),("retrieved_dialogue",0,true),("recent_turns",0,false),("recent_spoken_dialogue",2,false)] {
            if let Some(items)=context[field].as_array_mut() {
                if items.len()>keep {
                    if last {items.pop();} else {items.remove(0);}
                    *content=Value::String(context.to_string());return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;use serde_json::json;
    fn request()->Value {json!({"model":"synthetic","max_tokens":1024,"messages":[{"role":"system","content":"规则"},{"role":"user","content":"null ≠ 0；不要删除条件\n\"quoted\""}],"tools":[{"type":"function","function":{"name":"unused","parameters":{"type":"object"}}}],"response_format":{"type":"json_object"}})}
    #[test] fn full_serialization_and_actual_output_are_counted() {
        let p=request();let b=ContextBudget::default();let m=measure(&p,&b,WindowBasis::ConfiguredUnknown,None).unwrap();
        assert_eq!(m.serialized_text_bytes,serde_json::to_vec(&p).unwrap().len() as u64);
        assert_eq!(m.output_tokens,1024);assert_eq!(m.image_reserve,Some(0));assert!(m.complete_local_estimate);
        let mut p=p;p["tools"][0]["description"]="术语".repeat(100000).into();assert!(admit(&measure(&p,&b,WindowBasis::ConfiguredUnknown,None).unwrap()).is_err());
        p["max_tokens"]=8193.into();assert!(measure(&p,&b,WindowBasis::ConfiguredUnknown,None).is_err());
    }
    #[test] fn exact_boundary_and_model_switch_recompute() {
        let p=request();let b=ContextBudget{context_tokens:8192,max_output_tokens:1024,safety_tokens:256};
        let mut edge=p.clone();edge["padding"]="".into();let m=measure(&edge,&b,WindowBasis::UserReviewed,None).unwrap();
        edge["padding"]="x".repeat((m.hard_input_budget-m.serialized_text_bytes) as usize).into();
        let m=measure(&edge,&b,WindowBasis::UserReviewed,None).unwrap();assert_eq!(m.serialized_text_bytes,m.hard_input_budget);admit(&m).unwrap();
        edge["padding"]=format!("{}x",edge["padding"].as_str().unwrap()).into();assert!(admit(&measure(&edge,&b,WindowBasis::UserReviewed,None).unwrap()).is_err());
        let mut p=p;p["padding"]="x".repeat(7000).into();let wide=ContextBudget::default();
        admit(&measure(&p,&wide,WindowBasis::ConfiguredUnknown,None).unwrap()).unwrap();assert!(admit(&measure(&p,&b,WindowBasis::UserReviewed,None).unwrap()).is_err());
        let b=ContextBudget{context_tokens:1_048_576,..wide};assert_eq!(measure(&request(),&b,WindowBasis::ConfiguredUnknown,None).unwrap().window_tokens,262144);
    }
    #[test] fn images_unknown_near_limit_are_blocked_without_mutating_inputs() {
        let mut p=request();p["messages"][1]["content"]=json!([{"type":"text","text":"条件"},{"type":"image_url","image_url":{"url":format!("data:image/png;base64,{}","A".repeat(200000))}}]);
        let b=ContextBudget{context_tokens:8192,max_output_tokens:1024,safety_tokens:256};let original=p.clone();
        let m=fit(&mut p,&b,WindowBasis::ConfiguredUnknown,None).unwrap();assert!(!m.complete_local_estimate);assert!(m.serialized_text_bytes<2000);assert_eq!(p,original);
        p["messages"][1]["content"][0]["text"]="x".repeat(5300).into();let original=p.clone();assert!(fit(&mut p,&b,WindowBasis::ConfiguredUnknown,None).is_err());assert_eq!(p,original);
        assert!(fit(&mut p,&b,WindowBasis::UserReviewed,Some(10000)).is_err());
    }
    #[test] fn optional_pruning_keeps_current_conditions_and_images() {
        let mut p=request();let text=json!({"current_question":"不得混用 null、零和缺失字段","user_confirmed_facts":"只做过模拟","source_conflicts":["未确认"],"evidence":["x".repeat(9000)],"recent_spoken_dialogue":["不要忽略租户","金额 10 元"],"session_memory":["可检索旧话题"]}).to_string();
        p["messages"][1]["content"]=text.into();let b=ContextBudget{context_tokens:8192,max_output_tokens:1024,safety_tokens:256};fit(&mut p,&b,WindowBasis::ConfiguredUnknown,None).unwrap();
        let c:Value=serde_json::from_str(p["messages"][1]["content"].as_str().unwrap()).unwrap();assert_eq!(c["current_question"],"不得混用 null、零和缺失字段");assert_eq!(c["source_conflicts"],json!(["未确认"]));assert_eq!(c["recent_spoken_dialogue"].as_array().unwrap().len(),2);assert_eq!(c["evidence"],json!([]));
    }
}
