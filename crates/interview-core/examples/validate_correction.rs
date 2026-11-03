//! Real API regression for context-aware interpretation. Output remains local.
use interview_core::*;
use serde_json::json;
use std::path::Path;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 { return Err("Usage: validate_correction INDEX.sqlite OUTPUT.json".into()); }
    let index = Index::open(Path::new(&args[1])).await?;
    let key = std::env::var("DEEPSEEK_API_KEY")?;
    let cases = [
        "请介绍一下demo project中的企业测试以及为什么不能把闹和灵魂用",
        "企业员工培训的测试应该怎么做？",
        "我没有做过生产环境的 Doris 测试，面试时应该怎么说？",
        "我们先聊聊软件测试。",
        "FastAPI 返回 JSON 时，字段缺失、null、数字零和空字符串应该如何分别测试？JSON Schema 通过为什么仍可能有业务错误？",
    ];
    let mut results = vec![];
    for input in cases {
        let evidence = index.search(input, DEFAULT_ALIASES).await?;
        let mut answer = String::new();
        let mut metrics = serde_json::Value::Null;
        let result = stream_answer(&key, messages(input, "", &[], &evidence)?, CancellationToken::new(), |event| match event {
            StreamEvent::Delta { text } => answer.push_str(&text),
            StreamEvent::Done { usage, elapsed_ms, first_text_ms } => metrics = json!({"usage":usage,"elapsed_ms":elapsed_ms,"first_text_ms":first_text_ms}),
            _ => {},
        }).await;
        println!("case {}: {} evidence chunks; complete={}", results.len()+1, evidence.len(), result.is_ok());
        results.push(json!({"input":input,"answer":answer,"source_ids":evidence.iter().map(|e|&e.source_id).collect::<Vec<_>>(),"metrics":metrics,"error":result.err()}));
    }
    std::fs::write(&args[2], serde_json::to_string_pretty(&results)?)?;
    Ok(())
}
