//! Narrow credential handling for selected text, requests and exports. This is
//! not a general personal-data detector; original files are never rewritten.
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

pub const HIDDEN_CREDENTIAL:&str="[已隐藏API凭证]";
fn pattern()->&'static Regex {
    static KEY:OnceLock<Regex>=OnceLock::new();
    // ASCII boundary matters: in Chinese prose “密钥为sk-...” has no Unicode
    // word boundary before sk, despite being a separate credential token.
    KEY.get_or_init(||Regex::new(r"(?i)(?-u:\b)sk-[A-Za-z0-9_-]{16,}").unwrap())
}
pub fn contains_credential(text:&str)->bool {pattern().is_match(text)}
pub fn redact_credentials(text:&str,configured_key:Option<&str>)->String {
    let exact=match configured_key.map(str::trim).filter(|key|!key.is_empty()) {
        Some(key)=>text.replace(key,HIDDEN_CREDENTIAL),None=>text.to_owned(),
    };
    pattern().replace_all(&exact,HIDDEN_CREDENTIAL).into_owned()
}
pub fn redact_json_credentials(value:&mut Value,configured_key:Option<&str>)->crate::Result<bool> {
    fn walk(value:&mut Value,key:Option<&str>,depth:usize)->crate::Result<bool> {
        if depth>64{return Err("内容结构过深，未发送或导出".into());}
        let mut changed=false;
        match value {
            Value::String(text)=>{let redacted=redact_credentials(text,key);changed=redacted!=*text;*text=redacted;},
            Value::Array(items)=>for item in items {changed|=walk(item,key,depth+1)?;},
            Value::Object(items)=>{
                if items.keys().any(|name|redact_credentials(name,key)!=*name){return Err("字段名包含凭证格式，请先移除后重试".into());}
                for item in items.values_mut(){changed|=walk(item,key,depth+1)?;}
            },_=>{},
        }Ok(changed)
    }
    walk(value,configured_key,0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_adjacent_keys_and_exact_configured_key_are_hidden_without_changing_technical_values() {
        let fake=format!("sk-{}","synthetic01234567".repeat(2));
        let input=format!("密钥为{fake}，Authorization: Bearer {fake}。null、0、空字符串与缺失字段不能混用。");
        assert!(contains_credential(&input));let result=redact_credentials(&input,None);
        assert!(!result.contains(&fake));assert!(result.contains("null、0、空字符串与缺失字段不能混用"));
        assert!(!contains_credential("task-market-agent，sk-示例，DemoProject、pytest、JSON Schema"));
        let known="synthetic-configured-token";
        assert_eq!(redact_credentials(known,Some(known)),HIDDEN_CREDENTIAL);
        let mut data=serde_json::json!({"q":input,"list":[known,null,0,false,""]});
        assert!(redact_json_credentials(&mut data,Some(known)).unwrap());
        assert_eq!(data["list"][1],Value::Null);assert_eq!(data["list"][2],0);assert_eq!(data["list"][3],false);assert_eq!(data["list"][4],"");
    }
}
