use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestConfig {
    pub model: String,
    pub connect_timeout_secs: u64,
    pub first_answer_warning_secs: u64,
    pub total_timeout_secs: u64,
}
impl Default for RequestConfig {
    fn default() -> Self {
        Self {model:"deepseek-flash".into(),connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:20}
    }
}
impl RequestConfig {
    pub fn validate(&self) -> crate::Result<()> {
        if self.model.is_empty() || self.model.len()>64 || !self.model.bytes().all(|c|c.is_ascii_alphanumeric() || b"._-".contains(&c)) || crate::contains_credential(&self.model) {
            return Err("模型ID须为1–64个英文字母、数字、点、下划线或连字符，不能填写凭证或地址".into());
        }
        if !(1..=30).contains(&self.connect_timeout_secs) || !(5..=120).contains(&self.total_timeout_secs)
            || self.connect_timeout_secs>self.total_timeout_secs || self.first_answer_warning_secs==0 || self.first_answer_warning_secs>=self.total_timeout_secs {
            return Err("连接超时须为1–30秒且不超过总截止；总截止须为5–120秒；等待提示须大于0且早于总截止".into());
        }
        Ok(())
    }
}
impl crate::Index {
    pub async fn request_config(&self) -> crate::Result<RequestConfig> {
        let text=self.setting("request_config").await?;
        let config:RequestConfig=if text.is_empty(){RequestConfig::default()}else{serde_json::from_str(&text).map_err(|_|"请求配置格式无效，请在资料与设置中恢复默认后保存")?};
        config.validate()?;Ok(config)
    }
    pub async fn set_request_config(&self,config:&RequestConfig)->crate::Result<()> {
        config.validate()?;
        self.set("request_config",&serde_json::to_string(config).map_err(|_|"请求配置序列化失败")?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn configuration_defaults_persists_and_rejects_invalid_updates_without_replacing_previous() {
        let root=tempfile::Builder::new().prefix("interview-request-config-").tempdir().unwrap().keep();
        let db=root.join("profile.sqlite");let index=crate::Index::open(&db).await.unwrap();
        assert_eq!(index.request_config().await.unwrap(),RequestConfig::default());
        let custom=RequestConfig{model:"synthetic-model".into(),connect_timeout_secs:2,first_answer_warning_secs:3,total_timeout_secs:10};
        index.set_request_config(&custom).await.unwrap();
        for bad in [RequestConfig{model:"https://example.invalid".into(),..custom.clone()},RequestConfig{model:format!("sk-{}","synthetic".repeat(4)),..custom.clone()},RequestConfig{first_answer_warning_secs:10,..custom.clone()},RequestConfig{connect_timeout_secs:11,..custom.clone()},RequestConfig{total_timeout_secs:0,..custom.clone()}] {
            assert!(index.set_request_config(&bad).await.is_err());assert_eq!(index.request_config().await.unwrap(),custom);
        }
        drop(index);let index=crate::Index::open(&db).await.unwrap();assert_eq!(index.request_config().await.unwrap(),custom);
        index.set("request_config","{}").await.unwrap();assert!(index.request_config().await.is_err());
        index.set_request_config(&RequestConfig::default()).await.unwrap();assert_eq!(index.request_config().await.unwrap(),RequestConfig::default());
    }
}
