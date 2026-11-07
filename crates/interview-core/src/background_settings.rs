//! Reviewed background suggestions never silently replace user settings.
use crate::{Index,InterviewPreferences,Result};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};

#[derive(Clone,Debug,Default,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Suggestions {pub role:String,pub organization:String,pub context:String,pub terminology:String,pub call_names:String}
impl Suggestions {
    pub fn validate(&self)->Result<()> {
        let prefs=InterviewPreferences{role:self.role.clone(),organization:self.organization.clone(),context:self.context.clone(),terminology:self.terminology.clone(),..Default::default()};
        prefs.validate()?;
        if self.call_names.chars().count()>240||crate::contains_credential(&self.call_names)||self.call_names.chars().any(|c|c.is_control()&&!matches!(c,'\n'|'\t'|'\r')){return Err("建议称呼无效".into());}
        Ok(())
    }
}
#[derive(Clone,Debug,PartialEq,Eq,Hash,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum Field {Role,Organization,Context,Terminology}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {pub field:Field,pub expected:String,pub value:String}
fn preferences(raw:Option<String>)->Result<InterviewPreferences>{
    let prefs=match raw.filter(|s|!s.is_empty()){Some(raw)=>serde_json::from_str(&raw).map_err(|_|"场景设置无效")?,None=>InterviewPreferences::default()};
    prefs.validate()?;Ok(prefs)
}
impl Index {
    pub async fn background_settings_preview(&self)->Result<Value>{
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let raw:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='profile'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        let Some(raw)=raw else{return Ok(Value::Null)};
        let saved:Value=serde_json::from_str(&raw).map_err(|_|"背景资料状态无效")?;
        if saved["settings"].is_null(){return Ok(Value::Null);}
        let available:bool=sqlx::query_scalar(&format!("SELECT {}",crate::background_dependencies::available_sql())).fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
        if !available {return Err("背景建议的来源已变化，请重新整理后再核对设置".into());}
        let suggestions:Suggestions=serde_json::from_value(saved["settings"].clone()).map_err(|_|"背景设置建议无效")?;suggestions.validate()?;
        let current=preferences(sqlx::query_scalar("SELECT value FROM settings WHERE key='interview_preferences'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?)?;
        let version:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='version'").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
        Ok(json!({"version":version,"profile_hash":crate::digest(raw.as_bytes()),"current":current,"suggestions":suggestions}))
    }
    pub async fn apply_background_settings(&self,version:&str,profile_hash:&str,changes:&[Change])->Result<InterviewPreferences>{
        if changes.len()>4{return Err("设置字段数量无效".into());}
        let _guard=self.gate.lock().await;
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let active:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='version'").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
        let raw:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='profile'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        if active!=version||raw.as_deref().map(|v|crate::digest(v.as_bytes())).as_deref()!=Some(profile_hash){return Err("资料或背景已变化，请重新核对设置差异".into());}
        let available:bool=sqlx::query_scalar(&format!("SELECT {}",crate::background_dependencies::available_sql())).fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
        if !available {return Err("背景建议的来源已变化，未应用旧设置".into());}
        let mut current=preferences(sqlx::query_scalar("SELECT value FROM settings WHERE key='interview_preferences'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?)?;
        // A names-only UI application still verifies the source snapshot before
        // changing local reminder settings, but makes no preferences write.
        if changes.is_empty(){return Ok(current);}
        let mut seen=std::collections::HashSet::new();
        for change in changes {
            if !seen.insert(&change.field){return Err("重复的设置字段".into());}
            let field=match change.field{Field::Role=>&mut current.role,Field::Organization=>&mut current.organization,Field::Context=>&mut current.context,Field::Terminology=>&mut current.terminology};
            if *field!=change.expected{return Err("所选设置已被手动修改，已保留当前值；请重新核对差异".into());}
            *field=change.value.clone();
        }
        current.validate()?;
        sqlx::query("INSERT OR REPLACE INTO settings VALUES('interview_preferences',?)").bind(serde_json::to_string(&current).map_err(|e|e.to_string())?).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        tx.commit().await.map_err(|e|e.to_string())?;Ok(current)
    }
}
