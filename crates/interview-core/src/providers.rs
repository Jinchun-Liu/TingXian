//! Public endpoint metadata only. Credentials are held by the native process.
use serde::{Deserialize,Serialize};
use crate::{Index,RequestConfig,Result};

pub const DEEPSEEK_ENDPOINT:&str="https://api.deepseek.com/chat/completions";
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ProviderProtocol { Deepseek, Openai, Qwen }
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    pub id:String, pub name:String, pub enabled:bool,
    pub endpoint:String, pub protocol:ProviderProtocol, pub request:RequestConfig,
}
impl ProviderProfile {
    pub fn validate(&self)->Result<()> {
        if !["answer-1","answer-2","answer-3","vision","background"].contains(&self.id.as_str()) {return Err("未知模型位置".into());}
        if self.name.trim().is_empty() || self.name.chars().count()>40 || crate::contains_credential(&self.name) {return Err("接口名称须为1–40字，不能包含凭证".into());}
        self.request.validate()?;
        let url=reqwest::Url::parse(&self.endpoint).map_err(|_|"请填写完整 chat/completions 接口地址")?;
        let local=matches!(url.host_str(),Some("localhost"|"127.0.0.1"|"[::1]"|"::1"));
        if !(url.scheme()=="https" || (url.scheme()=="http" && local)) || url.host_str().is_none()
            || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some()
            || !url.path().ends_with("/chat/completions") || crate::contains_credential(&self.endpoint) {
            return Err("接口须为 HTTPS 的完整 /chat/completions 地址（本机 localhost 可用 HTTP），不能包含凭证、查询参数或片段".into());
        }
        Ok(())
    }
    pub fn accepts_empty_key(&self)->bool {self.endpoint.starts_with("http://")}
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig { pub answers:Vec<ProviderProfile>, pub vision:ProviderProfile, #[serde(default="background_profile")] pub background:ProviderProfile }
fn background_profile()->ProviderProfile {
    ProviderProfile { id:"background".into(), name:"背景整理".into(), enabled:false, endpoint:DEEPSEEK_ENDPOINT.into(), protocol:ProviderProtocol::Openai,
        request:RequestConfig{model:"deepseek-v4-pro".into(),total_timeout_secs:120,..Default::default()} }
}
impl ProviderConfig {
    pub fn initial(request:RequestConfig)->Self {
        let make=|id:&str,name:&str,enabled|ProviderProfile{id:id.into(),name:name.into(),enabled,endpoint:DEEPSEEK_ENDPOINT.into(),protocol:ProviderProtocol::Deepseek,request:request.clone()};
        Self {background:background_profile(),answers:vec![make("answer-1","DeepSeek",true),make("answer-2","AI 2",false),make("answer-3","AI 3",false)],
            vision:ProviderProfile{protocol:ProviderProtocol::Openai,name:"视觉模型".into(),..make("vision","视觉模型",false)}}
    }
    pub fn validate(&self)->Result<()> {
        if self.answers.len()!=3 || self.answers.iter().enumerate().any(|(i,p)|p.id!=format!("answer-{}",i+1)) || self.vision.id!="vision" || self.background.id!="background" {return Err("须保留三个回答位置和一个视觉模型位置".into());}
        for profile in self.answers.iter().chain([&self.vision,&self.background]) {profile.validate()?;}
        if !self.answers.iter().any(|p|p.enabled) {return Err("至少启用一个回答接口".into());}Ok(())
    }
}
impl Index {
    pub async fn provider_config(&self)->Result<ProviderConfig> {
        let stored=self.setting("provider_config").await?;
        let config=if stored.is_empty(){ProviderConfig::initial(self.request_config().await?)}else{serde_json::from_str(&stored).map_err(|_|"多接口配置无效，请重新保存")?};
        config.validate()?;Ok(config)
    }
    pub async fn set_provider_config(&self,config:&ProviderConfig)->Result<()> {
        config.validate()?;self.set("provider_config",&serde_json::to_string(config).map_err(|_|"接口配置序列化失败")?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_validation_and_migration_keep_credentials_out_of_metadata(){
        let config=ProviderConfig::initial(RequestConfig::default());config.validate().unwrap();
        let mut p=config.answers[0].clone();
        for bad in ["http://example.com/v1/chat/completions","https://user:password@example.com/v1/chat/completions","https://example.com/v1/chat/completions?key=x","https://example.com/v1/chat/completions#token","https://example.com/v1"] {
            p.endpoint=bad.into();assert!(p.validate().is_err(),"{bad}");
        }
        p.endpoint="http://127.0.0.1:1234/v1/chat/completions".into();p.request.model="Qwen/local:latest".into();p.validate().unwrap();assert!(p.accepts_empty_key());
        let value=serde_json::to_value(config).unwrap();assert!(value["answers"][0].get("key").is_none());
    }
}
