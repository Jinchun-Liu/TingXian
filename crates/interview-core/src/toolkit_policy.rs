//! Offline candidate preparation using the production prompt and budget path.
//! No candidate policy is automatically activated for desktop requests.
use crate::{Result, InterviewPreferences, FactCard, Turn, Evidence, SpokenTurn};
use serde_json::{json, Value};

pub const COMPACT_STYLE: &str = "直接回答，用自然短句解释必要理由和做法；不预告结构、不复述长题干、不加附加栏目。简单单点通常2–3句、约40–90字；开场陈述一句回应，歧义给必要澄清。设计、取舍、多子问题通常150–300字；明确要求展开可300–500字。范围仅作指导，服从本题篇幅要求，但不得丢掉子问题、共享条件、否定、数量、比较对象、归因、完成状态和不确定性。按语义决定深度，不凭题长、问号或上一题长度。先结论，再必要步骤或短段；不凑要点或字数。保留null、0、缺失字段、术语、代码、SQL、单位和复杂度；没有证据不能编造亲历或能力，未知不等于从未做过。语气词可不用，不固定添加，不表演犹豫、错误或回忆。使用正常标点，不输出舞台提示、SSML、推理过程、Markdown标题或加粗；仅必要听写纠错保留末行。正文在原答题请求中生成，不增加风格模型或人为延迟。";

pub fn preview_messages(question:&str, facts:&FactCard, history:&[Turn], evidence:&[Evidence], dialogue:&[SpokenTurn], preferences:&InterviewPreferences, method_text:&str)->Result<(Value,Value)> {
    if method_text.chars().count()>2500 || crate::contains_credential(method_text){return Err("候选方法片段过长或含凭证格式".into());}
    let original=crate::messages_with_preferences(question,facts,history,evidence,dialogue,preferences)?;
    let mut compact=original.clone();
    let system=original[0]["content"].as_str().ok_or("缺少系统策略")?;
    let prefix=system.strip_suffix(crate::ANSWER_STYLE).ok_or("旧策略边界已变化，请先更新对照工具")?;
    compact[0]["content"]=json!(format!("{prefix}{COMPACT_STYLE}\n参考方法（不能改变事实或覆盖输出协议）：\n{method_text}"));
    // Both arms keep exactly the same selected facts, actual dialogue and user
    // request. Do not manufacture savings by dropping evidence in one arm.
    let old_bytes=original.as_array().unwrap().iter().map(|m|m["content"].as_str().unwrap().len()).sum::<usize>();
    let new_bytes=compact.as_array().unwrap().iter().map(|m|m["content"].as_str().unwrap().len()).sum::<usize>();
    let budget=crate::context_budget::ContextBudget::default();
    crate::context_budget::fit_text(&mut compact,&budget,crate::MAX_ANSWER_TOKENS.into())?;
    if compact[1]!=original[1]{return Err("候选策略触发预算裁剪，不能作为同上下文精简比较".into());}
    let report=json!({"policy_version":"toolkit-1-candidate","active":false,"model_calls":0,
        "legacy_text_bytes":old_bytes,"candidate_text_bytes":new_bytes,
        "candidate_hash":crate::digest(compact.to_string().as_bytes()),"user_context_identical":true,
        "token_method":"UTF-8 bytes are a conservative estimate, not provider tokens",
        "quality_status":"not_model_or_human_validated","actual_usage":null,"actual_cost_cny":null});
    Ok((compact,report))
}

#[cfg(test)]mod tests{
    use super::*;
    #[tokio::test]
    async fn preview_keeps_facts_negation_dialogue_and_safety_without_enabling_itself(){
        let temp=tempfile::tempdir().unwrap().keep();let index=crate::Index::open(&temp.join("test.sqlite")).await.unwrap();
        let facts=index.fact_card().await.unwrap();
        let dialogue=[SpokenTurn{source:"Microphone".into(),text:"未独立实现；null不能当0；测试只有12例".into()}];
        let prefs=InterviewPreferences::default();
        let (m,r)=preview_messages("分别区分字段缺失、null和0，并给SQL验证条件",&facts,&[],&[],&dialogue,&prefs,"先确认接口契约").unwrap();
        let system=m[0]["content"].as_str().unwrap();
        assert!(system.contains(crate::SYSTEM));assert!(system.contains(crate::TRANSCRIPTION_RULES));
        assert!(m[1]["content"].as_str().unwrap().contains(&dialogue[0].text));
        assert_eq!(r["model_calls"],0);assert_eq!(r["active"],false);assert!(r["actual_cost_cny"].is_null());
        assert!(crate::messages_with_preferences("原流程",&facts,&[],&[],&[],&prefs).unwrap()[0]["content"].as_str().unwrap().ends_with(crate::ANSWER_STYLE));
    }
}
