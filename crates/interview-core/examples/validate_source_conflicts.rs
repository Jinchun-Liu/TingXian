//! Only fictional fixtures; never opens a profile, transcript or document.
use interview_core::*;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use std::io::Write;
fn evidence(id:&str,text:&str)->Evidence {Evidence {source_id:id.into(),path:format!("fictional-{id}.md"),root:"/fictional".into(),hash:format!("fictional-{id}"),location:"synthetic paragraph".into(),text:text.into(),kind:"personal".into(),allow_external:true,score:0.0,metadata:Default::default(),user_assertion:None}}
fn card(text:&str,confirmed:bool)->FactCard {FactCard {fact_id:"personal-fact-card".into(),version:1,previous_version:Some(0),text:text.into(),source_note:"Fictional test only".into(),sources:vec![],reason:"Synthetic test".into(),confirmed_at:confirmed.then(||"2026-11-03T00:00:00.000Z".into()),recorded_at:"2026-11-03T00:00:00.000Z".into()}}
#[tokio::main]
async fn main()->std::result::Result<(),Box<dyn std::error::Error>> {
    let path=std::env::args().nth(1).ok_or("new output JSON path required")?;
    let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(path)?;
    let key=std::env::var("DEEPSEEK_API_KEY")?;
    let cases=[
        ("虚构测试人物的本科毕业日期是什么？",card("本科毕业时间：2027年7月。",true),vec![evidence("A","本科毕业时间：2026年7月。"),evidence("B","本科毕业时间：2027年7月。")]),
        ("虚构的OrbitDemo项目是否全部由候选人独立编写代码？",card("OrbitDemo个人贡献：AI辅助编码，本人阅读、修改和验证代码，并非独立编写全部代码。",true),vec![evidence("A","OrbitDemo个人贡献：候选人独立编写全部代码。"),evidence("B","OrbitDemo个人贡献：AI辅助编码，本人阅读、修改和验证代码。")]),
        ("虚构测试人物的本科毕业日期是什么？",card("",false),vec![evidence("A","本科毕业时间：2026年7月。"),evidence("B","本科毕业时间：2027年7月。")]),
    ];
    let mut results=vec![];
    for (i,(question,facts,evidence)) in cases.into_iter().enumerate() {
        let request=messages_with_fact_card(question,&facts,&[],&evidence,&[])?;
        let mut answer=String::new();let mut metrics=serde_json::Value::Null;
        let result=stream_answer(&key,request,CancellationToken::new(),|event|match event {StreamEvent::Delta{text}=>answer.push_str(&text),StreamEvent::Done{usage,elapsed_ms,first_text_ms}=>metrics=json!({"usage":usage,"elapsed_ms":elapsed_ms,"first_text_ms":first_text_ms}),_=>{}}).await;
        println!("fictional case {}: complete={}, characters={}",i+1,result.is_ok(),answer.chars().count());
        results.push(json!({"question":question,"answer":answer,"characters":answer.chars().count(),"metrics":metrics,"error":result.err()}));
    }
    file.write_all(serde_json::to_string_pretty(&json!({"scope":"fictional fixtures only; no profile/database/document/transcript access","results":results}))?.as_bytes())?;Ok(())
}
