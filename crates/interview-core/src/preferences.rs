//! User-controlled interview context. Empty defaults carry no personal facts.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InterviewPreferences {
    pub role: String,
    pub organization: String,
    pub context: String,
    pub terminology: String,
    pub answer_language: String,
    pub transcription_language: String,
}
impl Default for InterviewPreferences {
    fn default() -> Self {
        Self { role:String::new(), organization:String::new(), context:String::new(), terminology:String::new(), answer_language:"auto".into(), transcription_language:"auto".into() }
    }
}
impl InterviewPreferences {
    pub fn validate(&self) -> crate::Result<()> {
        for (name,value,max) in [("目标岗位",&self.role,120),("目标组织",&self.organization,120),("面试背景",&self.context,1000),("领域术语",&self.terminology,1000)] {
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
        let value:InterviewPreferences=if text.is_empty(){Default::default()}else{serde_json::from_str(&text).map_err(|_|"面试场景设置格式无效，请恢复默认并保存")?};
        value.validate()?;Ok(value)
    }
    pub async fn set_preferences(&self,value:&InterviewPreferences) -> crate::Result<()> {
        value.validate()?;
        self.set("interview_preferences",&serde_json::to_string(value).map_err(|_|"面试场景设置序列化失败")?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(data["interview_context"]["role"],"策展助理");
        assert_eq!(data["user_confirmed_facts"],"");
        assert!(messages[0]["content"].as_str().unwrap().contains("英文"));
        assert!(!messages[0]["content"].as_str().unwrap().contains("策展助理"));
    }
}
