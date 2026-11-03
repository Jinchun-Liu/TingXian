//! Explicit synthetic API check for the production answer-style policy.
//! No profile, personal system prompt, documents, database or speech is read.
use interview_core::{stream_answer, StreamEvent, ANSWER_STYLE};
use serde_json::json;
use std::io::Write;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("output JSON path required")?;
    let key = std::env::var("DEEPSEEK_API_KEY")?;
    let cases = [
        ("simple", "JSON 中的 null 和数字零有什么区别？"),
        ("multipart", "假设一个多租户资源创建接口通过幂等键处理重复提交，返回 JSON。请分别说明：同一租户相同幂等键并发请求和不同租户相同幂等键如何测试；响应中的字段缺失、null、零和空字符串如何区分结构校验和业务断言；事务可能已经提交但客户端收到 HTTP 504 时，如何查询最终状态并验证重试。最大重试两次，不包括首次请求，权限错误不得重试；查询结果可能有短暂延迟。最后说明怎样通过延迟、断连与重复提交的故障注入验证这些约束。"),
        ("short_complex", "设计一个支持多租户隔离和幂等重试的资源创建接口测试方案。"),
        ("detailed", "详细比较乐观锁和悲观锁。请逐步说明实现方式、适用场景、并发冲突后的处理，并分别给一个测试例子。"),
        ("long_single", "假设我们准备测试一个云产品，涉及资源创建、幂等键、租户隔离、异步任务、重试上限、数据库事务和最终一致性。测试环境里还会注入网络延迟、连接中断、重复提交与查询结果延迟，同时记录 P95、P99、错误率和吞吐量。后续还要覆盖 JSON Schema、null、零、缺失字段和空字符串。现在只问一个判断：收到 HTTP 504 能否证明资源创建失败？请直接回答这个判断，不展开上述测试方案。"),
        ("explicit_brief", "对于多租户资源创建接口，请考虑幂等重试、null与零的语义、HTTP 504后业务状态未知、最多两次重试和权限错误不重试。请仅用两句话概括最关键的测试原则。"),
    ];
    let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(path)?;
    let mut results = vec![];
    let mut failures = 0;
    for (name, question) in cases {
        // Only the same production style policy is reused. The rest is public,
        // fictional technical content, with no actual candidate background.
        let messages = json!([
            {"role":"system","content":format!("这是合成技术回答测试。只提供通用建议，不声称任何候选人实际做过。以‘建议回答：’开头，不输出推理过程。\n{ANSWER_STYLE}")},
            {"role":"user","content":question},
        ]);
        let mut answer = String::new();
        let mut metrics = serde_json::Value::Null;
        let mut attempts = 0;
        let result = stream_answer(&key, messages, CancellationToken::new(), |event| match event {
            StreamEvent::Delta { text } => answer.push_str(&text),
            StreamEvent::RequestSent { .. } => attempts += 1,
            StreamEvent::Done { usage, elapsed_ms, first_text_ms } => metrics = json!({"usage":usage,"elapsed_ms":elapsed_ms,"first_text_ms":first_text_ms}),
            _ => {},
        }).await;
        let characters = answer.chars().count();
        let chinese_characters = answer.chars().filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c)).count();
        let depth_check = match name {
            "simple" | "long_single" => (20..=180).contains(&characters),
            "multipart" | "short_complex" => (120..=420).contains(&chinese_characters) && characters <= 750,
            "detailed" => (250..=650).contains(&chinese_characters) && characters <= 1200,
            "explicit_brief" => (20..=240).contains(&characters),
            _ => false,
        };
        let format_check = answer.starts_with("建议回答：") && !answer.contains("**") && !answer.contains('`');
        if result.is_err() || !depth_check || !format_check { failures += 1; }
        println!("{name}: complete={}, characters={characters}, chinese={chinese_characters}, depth_check={depth_check}, format_check={format_check}, attempts={attempts}", result.is_ok());
        results.push(json!({"case":name,"question":question,"answer":answer,"characters":characters,"chinese_characters":chinese_characters,"depth_check":depth_check,"format_check":format_check,"attempts":attempts,"metrics":metrics,"error":result.err()}));
    }
    file.write_all(serde_json::to_string_pretty(&json!({"scope":"public synthetic questions and production ANSWER_STYLE only; no personal context; length checks are not semantic acceptance","results":results}))?.as_bytes())?;
    if failures > 0 { return Err(format!("{failures} cases require review").into()); }
    Ok(())
}
