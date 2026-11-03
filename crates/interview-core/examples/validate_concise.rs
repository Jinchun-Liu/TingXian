//! Synthetic-only network check: no profile, document, database or transcript
//! is opened. Only public technical questions and fictional spoken context.
use interview_core::*;
use serde_json::json;
use std::io::Write;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("output JSON path required")?;
    let key = std::env::var("DEEPSEEK_API_KEY")?;
    let cases = [
        "请简述契约测试，为什么不能把 null 和零混用？",
        "企业员工培训的测试应该怎么做？",
        "假设一名候选人没有做过生产环境的 Doris 测试，应该怎么回答相关经历问题？",
        "我们先聊聊软件测试。",
        "FastAPI 返回 JSON 时，字段缺失、null、数字零和空字符串应该如何分别测试？JSON Schema 通过为什么仍可能有业务错误？",
        "你刚才提到的方法怎么测试？",
    ];
    let mut output = std::fs::OpenOptions::new().create_new(true).write(true).open(path)?;
    let mut results = vec![];
    for (i, input) in cases.iter().enumerate() {
        let dialogue = if i == 5 { vec![SpokenTurn { source: "Microphone".into(), text: "合成测试角色：我会使用幂等键避免重试时重复写入。".into() }] } else { vec![] };
        let messages = messages_with_dialogue(input, "本次全为合成测试，无真实候选人资料或已确认个人事实。", &[], &[], &dialogue)?;
        let mut answer = String::new(); let mut metrics = serde_json::Value::Null;
        let result = stream_answer(&key, messages, CancellationToken::new(), |event| match event {
            StreamEvent::Delta { text } => answer.push_str(&text),
            StreamEvent::Done { usage, elapsed_ms, first_text_ms } => metrics = json!({"usage":usage,"elapsed_ms":elapsed_ms,"first_text_ms":first_text_ms}),
            _ => {},
        }).await;
        println!("synthetic case {}: complete={}, characters={}", i+1, result.is_ok(), answer.chars().count());
        results.push(json!({"input":input,"answer":answer,"characters":answer.chars().count(),"metrics":metrics,"error":result.err()}));
    }
    output.write_all(serde_json::to_string_pretty(&json!({"scope":"synthetic only, no profile/database/real speech","results":results}))?.as_bytes())?;
    Ok(())
}
