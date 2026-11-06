//! User-reviewed capability metadata. A text probe never certifies model limits or vision.
use crate::{RequestConfig,Result};
use serde::{Deserialize,Serialize};

#[derive(Clone,Copy,Debug,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum CapabilitySupport {#[default] Unknown, Supported, Unsupported}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCapabilities {
    pub text:CapabilitySupport, pub image:CapabilitySupport,
    pub streaming:CapabilitySupport, pub usage:CapabilitySupport,
    pub checked_on:String, pub reference:String,
    pub model:String, pub endpoint:String,
    pub context_tokens:u64, pub max_output_tokens:u64,
}
impl ModelCapabilities {
    pub fn validate(&self,request:&RequestConfig)->Result<()> {
        crate::providers::validate_endpoint(&self.endpoint)?;
        if self.model!=request.model || self.context_tokens!=request.budget.context_tokens || self.max_output_tokens!=request.budget.max_output_tokens {
            return Err("模型或窗口/输出上限已变化，请清除旧能力记录并重新核对".into());
        }
        let date=chrono::NaiveDate::parse_from_str(&self.checked_on,"%Y-%m-%d").map_err(|_|"能力核对日期须为 YYYY-MM-DD")?;
        if self.checked_on!=date.format("%Y-%m-%d").to_string() || date>chrono::Local::now().date_naive() {
            return Err("能力核对日期无效或晚于今天".into());
        }
        if self.reference.trim().is_empty() || self.reference.chars().count()>500 || self.reference.chars().any(char::is_control) || crate::contains_credential(&self.reference) {
            return Err("请填写500字内的能力核对来源，不含凭证或控制字符".into());
        }
        Ok(())
    }
    pub fn endpoint_matches(&self,endpoint:&str)->Result<()> {
        if self.endpoint!=endpoint {return Err("接口地址已变化，请清除旧能力记录并重新核对".into());}Ok(())
    }
}
impl RequestConfig {
    pub fn check_capabilities(&self,endpoint:&str,streaming:bool,image:bool)->Result<()> {
        if let Some(c)=&self.capabilities {
            c.validate(self)?;c.endpoint_matches(endpoint)?;
            if c.text==CapabilitySupport::Unsupported {return Err("此模型标记为不支持文字输入，请调整模型能力或选择其他接口；未发送".into());}
            if streaming && c.streaming==CapabilitySupport::Unsupported {return Err("此模型标记为不支持流式响应，请选择支持流式的回答接口；未发送".into());}
            if image && c.image==CapabilitySupport::Unsupported {return Err("此模型标记为不支持图片输入，请选择视觉接口；未发送".into());}
        }
        Ok(())
    }
    pub fn usage_unsupported(&self)->bool {self.capabilities.as_ref().is_some_and(|c|c.usage==CapabilitySupport::Unsupported)}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reviewed(request:&RequestConfig)->ModelCapabilities {
        ModelCapabilities{text:CapabilitySupport::Supported,image:CapabilitySupport::Unknown,streaming:CapabilitySupport::Supported,usage:CapabilitySupport::Unknown,checked_on:"2020-01-02".into(),reference:"虚构测试规格；不是服务商实际能力".into(),model:request.model.clone(),endpoint:crate::DEEPSEEK_ENDPOINT.into(),context_tokens:request.budget.context_tokens,max_output_tokens:request.budget.max_output_tokens}
    }
    #[test]
    fn review_is_bound_to_identity_limits_and_valid_source_without_certifying_unknowns(){
        let mut request=RequestConfig::default();request.capabilities=Some(reviewed(&request));request.validate().unwrap();
        assert!(!request.usage_unsupported());
        let mut timeout=request.clone();timeout.total_timeout_secs=30;timeout.budget.safety_tokens=1024;timeout.validate().unwrap();
        let mut changed=request.clone();changed.model="different".into();assert!(changed.validate().is_err());
        changed=request.clone();changed.budget.context_tokens=65536;assert!(changed.validate().is_err());
        changed=request.clone();changed.budget.max_output_tokens=4096;assert!(changed.validate().is_err());
        assert!(request.check_capabilities("https://other.invalid/chat/completions",true,false).is_err());
        for date in ["2020-02-30","2020-1-2","9999-01-01",""]{let mut bad=request.clone();bad.capabilities.as_mut().unwrap().checked_on=date.into();assert!(bad.validate().is_err());}
        for reference in [String::new(),"a".repeat(501),format!("sk-{}","synthetic".repeat(4)),"line\nbreak".into()]{let mut bad=request.clone();bad.capabilities.as_mut().unwrap().reference=reference;assert!(bad.validate().is_err());}
        request.capabilities.as_mut().unwrap().streaming=CapabilitySupport::Unsupported;
        assert!(request.check_capabilities(crate::DEEPSEEK_ENDPOINT,true,false).is_err());
        request.check_capabilities(crate::DEEPSEEK_ENDPOINT,false,false).unwrap(); // background uses non-streaming JSON
        request.capabilities.as_mut().unwrap().text=CapabilitySupport::Unsupported;
        assert!(request.check_capabilities(crate::DEEPSEEK_ENDPOINT,false,false).is_err());
    }
    #[tokio::test]
    async fn legacy_unknown_and_reviewed_records_persist_without_probe_certification(){
        let dir=tempfile::tempdir().unwrap().keep();let db=dir.join("caps.sqlite");let index=crate::Index::open(&db).await.unwrap();
        index.set("request_config",r#"{"model":"legacy","connect_timeout_secs":5,"first_answer_warning_secs":8,"total_timeout_secs":20}"#).await.unwrap();
        assert!(index.request_config().await.unwrap().capabilities.is_none());
        let mut config=index.provider_config().await.unwrap();assert!(config.answers.iter().all(|p|p.request.capabilities.is_none()));
        let request=&mut config.answers[0].request;request.capabilities=Some(reviewed(request));
        index.set_provider_config(&config).await.unwrap();
        let reopened=crate::Index::open(&db).await.unwrap();assert_eq!(reopened.provider_config().await.unwrap(),config);
        let mut stale=config.clone();stale.answers[0].endpoint="https://other.invalid/chat/completions".into();
        assert!(index.set_provider_config(&stale).await.is_err());assert_eq!(index.provider_config().await.unwrap(),config);
        stale.answers[0].request.capabilities=None;index.set_provider_config(&stale).await.unwrap();
        assert!(index.provider_config().await.unwrap().answers[0].request.capabilities.is_none());
    }
}
