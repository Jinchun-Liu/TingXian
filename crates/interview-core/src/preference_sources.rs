//! Applied suggestions retain source ownership until an explicit manual edit.
use crate::{background_settings::{Change, Field}, Index, InterviewPreferences, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const KEY: &str = "interview_preference_sources";
const FORMAT: &str = "preference-sources-v1";
const FIELDS: [Field;4] = [Field::Role,Field::Organization,Field::Context,Field::Terminology];
type Tx<'a> = sqlx::Transaction<'a,sqlx::Sqlite>;

#[derive(Serialize,Deserialize)]
struct Binding { value_hash:String, snapshot:String }
#[derive(Serialize,Deserialize)]
struct Lineage {
    format:String,
    fields:BTreeMap<Field,Binding>,
    snapshots:BTreeMap<String,Value>,
}
impl Default for Lineage {
    fn default()->Self {Self {format:FORMAT.into(),fields:BTreeMap::new(),snapshots:BTreeMap::new()}}
}
impl Field {
    fn value(self,prefs:&InterviewPreferences)->&str {
        match self {Self::Role=>&prefs.role,Self::Organization=>&prefs.organization,Self::Context=>&prefs.context,Self::Terminology=>&prefs.terminology}
    }
    fn clear(self,prefs:&mut InterviewPreferences) {
        match self {Self::Role=>prefs.role.clear(),Self::Organization=>prefs.organization.clear(),Self::Context=>prefs.context.clear(),Self::Terminology=>prefs.terminology.clear()}
    }
}

#[derive(Debug,Serialize)]
pub struct PreferenceState {
    pub preferences:InterviewPreferences,
    pub blocked_fields:Vec<Field>,
}
impl PreferenceState {
    pub fn effective(mut self)->InterviewPreferences {
        for field in self.blocked_fields {field.clear(&mut self.preferences);}
        self.preferences
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualSetting {pub field:Field,pub expected:String}

async fn saved_preferences(tx:&mut Tx<'_>)->Result<InterviewPreferences> {
    let raw:Option<String>=sqlx::query_scalar("SELECT value FROM settings WHERE key='interview_preferences'")
        .fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
    let prefs:InterviewPreferences=match raw.filter(|v|!v.is_empty()) {
        Some(raw)=>serde_json::from_str(&raw).map_err(|_|"对话场景设置格式无效")?,None=>Default::default(),
    };
    prefs.validate()?;Ok(prefs)
}
async fn read(tx:&mut Tx<'_>)->Result<Lineage> {
    let raw:Option<String>=sqlx::query_scalar("SELECT value FROM settings WHERE key=?").bind(KEY)
        .fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
    let lineage:Lineage=match raw {Some(v)=>serde_json::from_str(&v).map_err(|_|"设置来源记录无效，未发送场景内容")?,None=>Default::default()};
    if lineage.format!=FORMAT {return Err("设置来源版本不受支持，未发送场景内容".into());}
    Ok(lineage)
}
async fn write(tx:&mut Tx<'_>,mut lineage:Lineage)->Result<()> {
    // At most four referenced snapshots. These are derived metadata; source
    // documents and their version history remain in the existing library.
    let used:HashSet<_>=lineage.fields.values().map(|b|b.snapshot.clone()).collect();
    lineage.snapshots.retain(|key,_|used.contains(key));
    let value=serde_json::to_string(&lineage).map_err(|e|e.to_string())?;
    sqlx::query("INSERT OR REPLACE INTO settings VALUES(?,?)").bind(KEY).bind(value)
        .execute(&mut **tx).await.map_err(|e|e.to_string())?;Ok(())
}
pub(crate) async fn bind(tx:&mut Tx<'_>,changes:&[Change])->Result<()> {
    let raw:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='profile_dependencies'")
        .fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
    let snapshot=match raw {
        Some(raw)=>{let mut v:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
            v.as_object_mut().ok_or("背景来源记录无效")?.remove("profile");v},
        None=>{let version:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='version'")
            .fetch_one(&mut **tx).await.map_err(|e|e.to_string())?;
            json!({"format":"legacy-settings-source-v1","library_version":version})},
    };
    let key=crate::digest(snapshot.to_string().as_bytes());
    let mut lineage=read(tx).await?;
    lineage.snapshots.insert(key.clone(),snapshot);
    for change in changes {
        if change.value.trim().is_empty() {lineage.fields.remove(&change.field);}
        else {lineage.fields.insert(change.field,Binding {value_hash:crate::digest(change.value.as_bytes()),snapshot:key.clone()});}
    }
    write(tx,lineage).await
}
async fn current(tx:&mut Tx<'_>,snapshot:&Value)->Result<bool> {
    if snapshot["format"]=="legacy-settings-source-v1" {
        let sql=format!("SELECT value=? AND NOT {} FROM settings WHERE key='version'",crate::background_dependencies::legacy_invalid_sql());
        return sqlx::query_scalar(&sql).bind(snapshot["library_version"].as_str().unwrap_or("invalid"))
            .fetch_optional(&mut **tx).await.map(|v|v.unwrap_or(false)).map_err(|e|e.to_string());
    }
    let sql=format!("WITH p AS (SELECT ? AS value) SELECT {} FROM p",crate::background_dependencies::sources_current_sql("p.value"));
    sqlx::query_scalar(&sql).bind(snapshot.to_string()).fetch_one(&mut **tx).await.map_err(|e|e.to_string())
}
impl Index {
    pub async fn preference_state(&self)->Result<PreferenceState> {
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let preferences=saved_preferences(&mut tx).await?;let lineage=read(&mut tx).await?;
        let mut checked=HashMap::new();let mut blocked_fields=vec![];
        for (field,binding) in &lineage.fields {
            let valid=if let Some(valid)=checked.get(&binding.snapshot) {*valid} else {
                let valid=match lineage.snapshots.get(&binding.snapshot) {Some(v)=>current(&mut tx,v).await?,None=>false};
                checked.insert(binding.snapshot.clone(),valid);valid
            };
            if !valid || binding.value_hash!=crate::digest(field.value(&preferences).as_bytes()) {blocked_fields.push(*field);}
        }
        Ok(PreferenceState {preferences,blocked_fields})
    }
    pub async fn request_preferences(&self)->Result<InterviewPreferences> {
        Ok(self.preference_state().await?.effective())
    }
    pub(crate) async fn save_manual_preferences(&self,value:&InterviewPreferences)->Result<()> {
        self.set_preferences_expected(value,None).await
    }
    pub async fn set_preferences_expected(&self,value:&InterviewPreferences,expected:Option<&InterviewPreferences>)->Result<()> {
        value.validate()?;
        let _gate=self.gate.lock().await;
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let previous=saved_preferences(&mut tx).await?;let mut lineage=read(&mut tx).await?;
        if expected.is_some_and(|v|v!=&previous) {return Err("设置已变化，已保留当前值与编辑草稿；请重新读取后核对".into());}
        // Saving a language change or the same text must not silently certify
        // stale background-derived values. An actual edit becomes user-owned.
        for field in FIELDS {if field.value(value).trim().is_empty()||field.value(&previous)!=field.value(value) {lineage.fields.remove(&field);}}
        write(&mut tx,lineage).await?;
        sqlx::query("INSERT OR REPLACE INTO settings VALUES('interview_preferences',?)")
            .bind(serde_json::to_string(value).map_err(|e|e.to_string())?).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        tx.commit().await.map_err(|e|e.to_string())
    }
    pub async fn confirm_manual_settings(&self,fields:&[ManualSetting])->Result<PreferenceState> {
        if fields.is_empty()||fields.len()>4 {return Err("请选择要转为手动的设置".into());}
        let _gate=self.gate.lock().await;
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let preferences=saved_preferences(&mut tx).await?;let mut lineage=read(&mut tx).await?;
        let mut seen=HashSet::new();
        for item in fields {
            if !seen.insert(item.field)||item.field.value(&preferences)!=item.expected {return Err("设置已变化，请重新读取后核对；未修改".into());}
            lineage.fields.remove(&item.field);
        }
        write(&mut tx,lineage).await?;tx.commit().await.map_err(|e|e.to_string())?;
        self.preference_state().await
    }
}
