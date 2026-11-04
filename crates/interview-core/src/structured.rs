//! Explicit, snapshot-bound selection of JSON/YAML fields. No directory crawl
//! parses structured content, and no selected value is treated as an instruction.
use crate::{digest, safe_path, Result, Segment};
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StructuredSelection { pub root:String, pub file_hash:String, pub pointers:Vec<String> }
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StructuredField { pub pointer:String, pub kind:String, pub preview:String, pub characters:usize, pub children:usize }
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StructuredPreview { pub root:String, pub path:String, pub file_hash:String, pub fields:Vec<StructuredField> }
#[derive(Deserialize)]
struct ParsedField { pointer:String, kind:String, value:Option<String>, children:usize }

pub fn structured_format(path:&Path)->Option<&'static str> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {"json"=>Some("json"),"yaml"|"yml"=>Some("yaml"),_=>None}
}
async fn parse(bytes:Vec<u8>,format:&str,python:&str)->Result<Vec<ParsedField>> {
    tokio::time::timeout(Duration::from_secs(30),async {
        let mut child=tokio::process::Command::new(python).arg("-B").arg("-c").arg(include_str!("structured.py")).arg(format)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true)
            .spawn().map_err(|e|format!("结构化解析工具不可用：{e}"))?;
        let mut stdin=child.stdin.take().ok_or("解析输入通道不可用")?;
        stdin.write_all(&bytes).await.map_err(|e|format!("读取结构化快照失败：{e}"))?;drop(stdin);
        let output=child.wait_with_output().await.map_err(|e|e.to_string())?;
        if !output.status.success(){return Err(String::from_utf8_lossy(&output.stderr).chars().take(300).collect());}
        serde_json::from_slice(&output.stdout).map_err(|_|"结构化解析结果无效".into())
    }).await.map_err(|_|"结构化解析超时（30 秒）")?
}
pub async fn preview_structured(root:&str,path:&str,python:&str)->Result<StructuredPreview> {
    let root=tokio::fs::canonicalize(root).await.map_err(|e|e.to_string())?;
    let source=safe_path(&root,path)?;
    let format=structured_format(&source).ok_or("该文件不是 JSON/YAML")?;
    if tokio::fs::metadata(&source).await.map_err(|e|e.to_string())?.len()>50*1024*1024 {return Err("文件超过 50 MB".into());}
    let bytes=tokio::fs::read(source).await.map_err(|e|e.to_string())?;
    let file_hash=digest(&bytes);
    let parsed=parse(bytes,format,python).await?;
    let fields=parsed.into_iter().map(|field|{let text=field.value.unwrap_or_default();StructuredField {pointer:field.pointer,kind:field.kind,characters:text.chars().count(),preview:text.chars().take(160).collect(),children:field.children}}).collect();
    Ok(StructuredPreview {root:root.to_string_lossy().into_owned(),path:path.into(),file_hash,fields})
}
pub(crate) fn selected_pointers(selection:&StructuredSelection,root:&str,file_hash:&str)->Result<Vec<String>> {
    if selection.root!=root || selection.file_hash!=file_hash {return Err("文件或资料目录已变化，请重新预览并选择字段".into());}
    if selection.pointers.is_empty() || selection.pointers.len()>20_000 {return Err("请明确选择结构化字段或整份文件".into());}
    if selection.pointers.iter().any(|p|p.len()>8192 || (!p.is_empty() && !p.starts_with('/'))) {return Err("字段路径无效，请重新预览".into());}
    let mut pointers=selection.pointers.clone();pointers.sort();pointers.dedup();
    let mut selected:Vec<String>=vec![];
    for pointer in pointers {if !selected.iter().any(|p|contains(p,&pointer)){selected.push(pointer);}}
    Ok(selected)
}
fn contains(parent:&str,child:&str)->bool {parent.is_empty() || child==parent || child.strip_prefix(parent).is_some_and(|s|s.starts_with('/'))}
pub(crate) fn selection_hash(file_hash:&str,pointers:&[String])->String {
    // v2 keeps numeric literals exact instead of binary-float conversion.
    digest(serde_json::to_vec(&("structured-fields-v2",file_hash,pointers)).unwrap().as_slice())
}
pub(crate) async fn structured_segments(bytes:Vec<u8>,format:&str,python:&str,pointers:&[String])->Result<Vec<u8>> {
    let fields=parse(bytes,format,python).await?;
    for p in pointers {if !fields.iter().any(|f|&f.pointer==p){return Err("所选字段已不存在，请重新预览".into());}}
    let mut output=vec![];
    for field in fields {
        if !pointers.iter().any(|p|contains(p,&field.pointer)){continue;}
        if let Some(value)=field.value {
            let kind=match field.kind.as_str(){"null"=>"null 空值","number"=>"数字","boolean"=>"布尔值","string"=>"字符串","object"=>"对象","array"=>"数组",_=>return Err("字段类型无效".into())};
            let location=if field.pointer.is_empty(){"字段（根）".into()}else{format!("字段 {}",field.pointer)};
            let segment=Segment {text:format!("{location}\n类型：{kind}\n值：{value}"),location};
            serde_json::to_writer(&mut output,&segment).map_err(|e|e.to_string())?;output.push(b'\n');
        }
    }
    if output.is_empty(){return Err("所选范围没有可导入字段".into());}Ok(output)
}
