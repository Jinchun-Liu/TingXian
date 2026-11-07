//! User-controlled conversation context. Empty defaults carry no personal facts.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ConversationScenario { #[default] General, Interview, Learning, Meeting }
fn legacy_scenario()->ConversationScenario { ConversationScenario::Interview }
impl ConversationScenario {
    pub fn instructions(self)->&'static str {
        match self {
            Self::General=>"当前场景：通用对话。直接回应当前问题或要求，结合上下文澄清与解释，不默认用户正在应聘。历史AI建议仅用于衔接讨论，可能出错，不是用户事实或外部证据。",
            Self::Interview=>"当前场景：面试。提供适合口头表达的候选人建议回答，不代替本人发言。对方的完整问题或要求需要回应，开场陈述简短接话。不得把练习或AI建议当作本人经历。",
            Self::Learning=>"当前场景：学习问答。作为学习助手解释概念、步骤、例子与常见误区；根据用户基础和当前问题调整深度。可以参考标为assistant_suggestion的历史讲解来理解‘第二步’等追问，但应重新核对其正确性，不默认它是事实。用户未表达已掌握时，不声称其已学会。",
            Self::Meeting=>"当前场景：会议。提供讨论分析、澄清和按需摘要，不生成候选人式应聘答案。整理最近讨论时，只根据本次提供的实际发言区分明确结论、待确认事项与行动项；未明确的负责人和截止日期写‘未指定’，建议不等于决议。检索资料和历史AI建议仅为背景，不能冒充会上发言或承诺。上下文有长度限制，称为‘最近讨论摘要’，不能声称覆盖整场会议。",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InterviewPreferences {
    #[serde(default="legacy_scenario")]
    pub scenario: ConversationScenario,
    pub role: String,
    pub organization: String,
    pub context: String,
    pub terminology: String,
    pub answer_language: String,
    pub transcription_language: String,
}
impl Default for InterviewPreferences {
    fn default() -> Self {
        Self { scenario:ConversationScenario::General, role:String::new(), organization:String::new(), context:String::new(), terminology:String::new(), answer_language:"auto".into(), transcription_language:"auto".into() }
    }
}
impl InterviewPreferences {
    pub fn validate(&self) -> crate::Result<()> {
        for (name,value,max) in [("角色或目标岗位",&self.role,120),("组织",&self.organization,120),("场景背景",&self.context,1000),("领域术语",&self.terminology,1000)] {
            if value.chars().count()>max {return Err(format!("{name}不能超过{max}个字符"));}
            if crate::contains_credential(value) {return Err(format!("{name}含凭证格式，请使用专用API Key输入框"));}
        }
        if !["auto","zh","en"].contains(&self.answer_language.as_str()) {return Err("不支持的回答语言".into());}
        if !["auto","zh","en","ja","ko","fr","de","es"].contains(&self.transcription_language.as_str()) {return Err("不支持的转写语言".into());}
        Ok(())
    }
}
impl crate::Index {
    pub async fn preferences(&self) -> crate::Result<InterviewPreferences> {
        let text=self.setting("interview_preferences").await?;
        let value:InterviewPreferences=if text.is_empty(){Default::default()}else{serde_json::from_str(&text).map_err(|_|"对话场景设置格式无效，请恢复默认并保存")?};
        value.validate()?;Ok(value)
    }
    pub async fn set_preferences(&self,value:&InterviewPreferences) -> crate::Result<()> {
        value.validate()?;
        self.save_manual_preferences(value).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn new_install_is_general_and_legacy_saved_preferences_remain_interview() {
        let root=tempfile::tempdir().unwrap().keep();
        let index=crate::Index::open(&root.join("profile.sqlite")).await.unwrap();
        assert_eq!(index.preferences().await.unwrap().scenario,ConversationScenario::General);
        index.set("interview_preferences",r#"{"role":"legacy role","answer_language":"en"}"#).await.unwrap();
        let legacy=index.preferences().await.unwrap();
        assert_eq!(legacy.scenario,ConversationScenario::Interview);
        let turn:crate::Turn=serde_json::from_str(r#"{"question":"legacy question","confirmed_answer":null}"#).unwrap();
        assert!(turn.suggested_answer.is_none());
        assert_eq!(legacy.role,"legacy role");assert_eq!(legacy.answer_language,"en");
        let mut updated=legacy.clone();updated.scenario=ConversationScenario::Meeting;
        index.set_preferences(&updated).await.unwrap();
        assert_eq!(index.preferences().await.unwrap(),updated);
        assert!(serde_json::from_str::<InterviewPreferences>(r#"{"scenario":"unknown"}"#).is_err());
    }
    #[tokio::test]
    async fn scenes_keep_generated_history_distinct_from_actual_speech_and_confirmed_facts() {
        let root=tempfile::tempdir().unwrap().keep();
        let index=crate::Index::open(&root.join("profile.sqlite")).await.unwrap();
        let facts=index.fact_card().await.unwrap();
        let history=[crate::Turn{question:"解释两步方法".into(),confirmed_answer:Some("我想先澄清概念".into()),suggested_answer:Some("AI_SYNTHETIC_SUGGESTION".repeat(150))}];
        for scenario in [ConversationScenario::General,ConversationScenario::Interview,ConversationScenario::Learning,ConversationScenario::Meeting] {
            let preferences=InterviewPreferences{scenario,..Default::default()};
            let messages=crate::messages_with_preferences("第二步是什么意思？",&facts,&history,&[],&[],&preferences).unwrap();
            let system=messages[0]["content"].as_str().unwrap();
            let user=messages[1]["content"].as_str().unwrap();
            let body:serde_json::Value=serde_json::from_str(user).unwrap();
            assert!(system.contains(scenario.instructions()));
            assert_eq!(body["conversation_context"]["scenario"],serde_json::to_value(scenario).unwrap());
            assert_eq!(body["recent_turns"][0]["confirmed_actual_answer"],"我想先澄清概念");
            assert_eq!(body["user_confirmed_facts"],"");
            assert!(body["recent_spoken_dialogue"].as_array().unwrap().is_empty());
            let suggestion=&body["recent_turns"][0]["assistant_suggestion"];
            if scenario==ConversationScenario::Interview {assert!(suggestion.is_null());}
            else {assert_eq!(suggestion.as_str().unwrap().chars().count(),1000);}
            assert!(system.chars().count()+user.chars().count()<=12_000);
        }
    }
    #[tokio::test]
    async fn empty_defaults_persist_cross_industry_context_and_reject_bad_updates() {
        let root=tempfile::tempdir().unwrap().keep();
        let index=crate::Index::open(&root.join("profile.sqlite")).await.unwrap();
        assert_eq!(index.preferences().await.unwrap(),InterviewPreferences::default());
        assert_eq!(crate::DEFAULT_ALIASES,"");
        let value=InterviewPreferences{role:"品牌策划".into(),organization:"示例工作室".into(),context:"讨论零售消费品行业".into(),terminology:"品牌资产，品牌认知，复购率".into(),answer_language:"en".into(),..Default::default()};
        index.set_preferences(&value).await.unwrap();
        assert_eq!(index.preferences().await.unwrap(),value);
        let mut bad=value.clone();bad.role="长".repeat(121);
        assert!(index.set_preferences(&bad).await.is_err());
        bad=value.clone();bad.answer_language="unsupported".into();
        assert!(index.set_preferences(&bad).await.is_err());
        bad=value.clone();bad.context=format!("sk-{}","x".repeat(40));
        assert!(index.set_preferences(&bad).await.is_err());
        assert_eq!(index.preferences().await.unwrap(),value);
        assert!(serde_json::from_str::<InterviewPreferences>(r#"{"unexpected":true}"#).is_err());
    }
    #[tokio::test]
    async fn configured_context_is_data_not_personal_evidence() {
        let root=tempfile::tempdir().unwrap().keep();
        let index=crate::Index::open(&root.join("profile.sqlite")).await.unwrap();
        let facts=index.fact_card().await.unwrap();
        let value=InterviewPreferences{role:"策展助理".into(),terminology:"策展叙事，观众研究".into(),answer_language:"en".into(),..Default::default()};
        let messages=crate::messages_with_preferences("如何理解观众研究？",&facts,&[],&[],&[],&value).unwrap();
        let data:serde_json::Value=serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(data["conversation_context"]["role"],"策展助理");
        assert_eq!(data["user_confirmed_facts"],"");
        assert!(messages[0]["content"].as_str().unwrap().contains("英文"));
        assert!(!messages[0]["content"].as_str().unwrap().contains("策展助理"));
    }
}
