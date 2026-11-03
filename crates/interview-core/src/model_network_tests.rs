//! Real loopback HTTP only, synthetic data and key; production URL is unchanged.
use super::*;
use std::sync::{Arc,Mutex};
use tokio::io::{AsyncReadExt,AsyncWriteExt};

struct Reply {status:u16,header_delay:u64,parts:Vec<(u64,String)>}
fn ok(parts:Vec<(u64,String)>)->Reply {Reply{status:200,header_delay:0,parts}}
fn delta(text:&str)->String {format!("data: {}\n\n",json!({"choices":[{"delta":{"content":text}}]}))}
fn done()->String {"data: [DONE]\n\n".into()}
struct Server {url:String,requests:Arc<Mutex<Vec<(Value,String)>>>,task:tokio::task::JoinHandle<()>}
impl Drop for Server {fn drop(&mut self){self.task.abort();}}
async fn serve(replies:Vec<Reply>)->Server {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url=format!("http://{}/chat/completions",listener.local_addr().unwrap());
    let requests=Arc::new(Mutex::new(Vec::new()));let captured=requests.clone();
    let task=tokio::spawn(async move {
        for reply in replies {
            let (mut socket,_)=listener.accept().await.unwrap();let mut input=Vec::new();let mut buf=[0;4096];
            let end=loop {let n=socket.read(&mut buf).await.unwrap();assert!(n>0);input.extend_from_slice(&buf[..n]);if let Some(p)=input.windows(4).position(|w|w==b"\r\n\r\n"){break p+4;}assert!(input.len()<100_000);};
            let header=String::from_utf8(input[..end].to_vec()).unwrap();
            let len:usize=header.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|v|v.trim().parse().unwrap())).unwrap();
            while input.len()<end+len {let n=socket.read(&mut buf).await.unwrap();assert!(n>0);input.extend_from_slice(&buf[..n]);}
            captured.lock().unwrap().push((serde_json::from_slice(&input[end..end+len]).unwrap(),header));
            tokio::time::sleep(Duration::from_millis(reply.header_delay)).await;
            if reply.status!=200 {let _=socket.write_all(format!("HTTP/1.1 {} Synthetic\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",reply.status).as_bytes()).await;continue;}
            if socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.is_err(){continue;}
            for (delay,part) in reply.parts {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                if socket.write_all(format!("{:x}\r\n{}\r\n",part.len(),part).as_bytes()).await.is_err(){break;}
            }
            let _=socket.write_all(b"0\r\n\r\n").await;
        }
    });Server{url,requests,task}
}
fn config()->crate::RequestConfig {crate::RequestConfig{model:"synthetic-model".into(),connect_timeout_secs:1,first_answer_warning_secs:1,total_timeout_secs:5}}
async fn run(server:&Server,cancel:CancellationToken)->(Result<()>,Vec<StreamEvent>) {
    let key=format!("sk-{}","synthetic01234567".repeat(2));let mut events=Vec::new();
    let client=reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(1)).redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let result=run_stream(client,&server.url,&key,json!([{"role":"user","content":format!("null不等于0；测试值{key}")}]),cancel,&config(),|e|events.push(e)).await;
    for (body,header) in server.requests.lock().unwrap().iter(){assert_eq!(body["model"],"synthetic-model");assert_eq!(body["thinking"]["type"],"disabled");assert!(!body.to_string().contains(&key));assert!(header.contains(&key));}
    (result,events)
}
fn attempts(events:&[StreamEvent])->usize {events.iter().filter(|e|matches!(e,StreamEvent::RequestSent{..})).count()}
fn warnings(events:&[StreamEvent])->usize {events.iter().filter(|e|matches!(e,StreamEvent::Warning{..})).count()}
fn completed(events:&[StreamEvent])->bool {events.iter().any(|e|matches!(e,StreamEvent::Done{..}))}

#[tokio::test]
async fn multipart_answer_streams_as_one_request_without_losing_numbered_points() {
    let parts = ["建议回答：分别验证约束和最终状态。\n", "1. 同租户重试只产生一份资源，不同租户独立。\n", "2. 缺失、null、零和空字符串分别断言。\n", "3. HTTP 504 后先查询最终状态，权限错误不重试。"];
    let mut frames: Vec<_> = parts.iter().map(|text| (0, delta(text))).collect();
    frames.push((0, done()));
    let server = serve(vec![ok(frames)]).await;
    let (result, events) = run(&server, CancellationToken::new()).await;
    assert!(result.is_ok()); assert!(completed(&events)); assert_eq!(attempts(&events), 1);
    let received: String = events.iter().filter_map(|event| match event { StreamEvent::Delta { text } => Some(text.as_str()), _ => None }).collect();
    assert_eq!(received, parts.concat());
    assert!(has_useful_answer(&received));
    assert_eq!(server.requests.lock().unwrap()[0].0["max_tokens"], 1600);
}

#[tokio::test]
async fn observed_usage_survives_truncation_broken_stream_and_cancel_without_false_completion() {
    let usage=format!("data: {}\n\n",json!({"choices":[],"usage":{"total_tokens":16}}));
    let length=format!("data: {}\n\n",json!({"choices":[{"finish_reason":"length"}],"usage":{"total_tokens":16}}));
    for (parts,cancel_after) in [
        (vec![(0,delta("建议回答：部分内容")),(0,usage.clone())],false),
        (vec![(0,delta("建议回答：部分内容")),(0,length),(0,done())],false),
        (vec![(0,delta("建议回答：部分内容")),(0,usage),(8000,done())],true),
    ] {
        let server=serve(vec![ok(parts)]).await;let cancel=CancellationToken::new();let trigger=cancel.clone();
        let task=tokio::spawn(async move {if cancel_after {tokio::time::sleep(Duration::from_millis(100)).await;trigger.cancel();}});
        let (result,events)=run(&server,cancel).await;task.await.unwrap();
        assert!(result.is_err());assert!(!completed(&events));assert_eq!(attempts(&events),1);
        assert!(events.iter().any(|e|matches!(e,StreamEvent::Usage{attempt:1,usage} if usage["total_tokens"]==16)));
    }
}

#[test]
fn useful_text_requires_answer_body_not_headings_corrections_or_punctuation() {
    for input in ["","建议回答：","建议回答：……","听写纠正：企业→契约","建议回答：\n听写纠正：企业→契约"] {assert!(!has_useful_answer(input),"{input}");}
    for input in ["建议回答：null与0不同。","听写理解：原词\n## **建议回答**：使用幂等键。"] {assert!(has_useful_answer(input));}
}
#[tokio::test]
async fn warning_arrives_before_delayed_response_headers_and_success_retains_usage() {
    let usage=format!("data: {}\n\n",json!({"choices":[],"usage":{"total_tokens":9}}));
    let mut reply=ok(vec![(0,delta("建议回答：null不等于0。")),(0,usage),(0,done())]);reply.header_delay=1300;
    let server=serve(vec![reply]).await;let (result,events)=run(&server,CancellationToken::new()).await;
    assert!(result.is_ok(),"{result:?}");assert_eq!(warnings(&events),1);assert_eq!(attempts(&events),1);
    let warning=events.iter().position(|e|matches!(e,StreamEvent::Warning{..})).unwrap();let text=events.iter().position(|e|matches!(e,StreamEvent::Delta{..})).unwrap();assert!(warning<text);
    assert!(events.iter().any(|e|matches!(e,StreamEvent::Done{usage:Some(value),..} if value["total_tokens"]==9)));
}
#[tokio::test]
async fn streamed_title_does_not_suppress_warning_but_useful_body_does() {
    let server=serve(vec![ok(vec![(0,delta("建议")),(0,delta("回答：")),(1300,delta("通过幂等键避免重复写入。")),(0,done())])]).await;
    let (result,events)=run(&server,CancellationToken::new()).await;assert!(result.is_ok());assert_eq!(warnings(&events),1);
    let server=serve(vec![ok(vec![(0,delta("建议回答：通过幂等键避免重复写入。")),(1300,done())])]).await;
    let (result,events)=run(&server,CancellationToken::new()).await;assert!(result.is_ok());assert_eq!(warnings(&events),0);
}
#[tokio::test]
async fn deadline_preserves_partial_output_and_cancel_can_interrupt_header_wait() {
    let server=serve(vec![ok(vec![(0,delta("建议回答：部分答案")),(8000,done())])]).await;
    let started=Instant::now();let (result,events)=run(&server,CancellationToken::new()).await;
    assert!(result.unwrap_err().contains("5 秒"));assert!(started.elapsed()<Duration::from_secs(7));assert!(!completed(&events));assert_eq!(attempts(&events),1);
    assert!(events.iter().any(|e|matches!(e,StreamEvent::Delta{text} if text.contains("部分答案"))));
    let mut reply=ok(vec![(0,done())]);reply.header_delay=8000;let server=serve(vec![reply]).await;
    let cancel=CancellationToken::new();let trigger=cancel.clone();let task=tokio::spawn(async move {tokio::time::sleep(Duration::from_millis(100)).await;trigger.cancel();});
    let started=Instant::now();let (result,events)=run(&server,cancel).await;task.await.unwrap();
    assert_eq!(result.unwrap_err(),"已取消");assert!(started.elapsed()<Duration::from_secs(2));assert!(!completed(&events));
}
#[tokio::test]
async fn retry_is_limited_to_transient_pre_answer_failures_and_uses_one_deadline() {
    for status in [429,503] {
        let server=serve(vec![Reply{status,header_delay:0,parts:vec![]},ok(vec![(0,delta("建议回答：使用幂等键。")),(0,done())])]).await;
        let (result,events)=run(&server,CancellationToken::new()).await;assert!(result.is_ok());assert_eq!(attempts(&events),2);
    }
    for status in [400,401,402] {
        let server=serve(vec![Reply{status,header_delay:0,parts:vec![]}]).await;
        let (result,events)=run(&server,CancellationToken::new()).await;assert!(result.unwrap_err().contains(&status.to_string()));assert_eq!(attempts(&events),1);assert!(!completed(&events));
    }
    let server=serve(vec![ok(vec![(0,delta("建议回答：已到达部分答案"))])]).await;
    let (result,events)=run(&server,CancellationToken::new()).await;assert!(result.is_err());assert_eq!(attempts(&events),1);assert!(!completed(&events));
    let mut delayed=ok(vec![(0,delta("建议回答：已经太晚的答案")),(0,done())]);delayed.header_delay=3000;
    let server=serve(vec![Reply{status:503,header_delay:2300,parts:vec![]},delayed]).await;
    let started=Instant::now();let (result,events)=run(&server,CancellationToken::new()).await;
    assert!(result.unwrap_err().contains("5 秒"));assert!(started.elapsed()<Duration::from_millis(6500));assert_eq!(attempts(&events),2);assert_eq!(warnings(&events),1);assert!(!completed(&events));
}
#[tokio::test]
async fn heading_only_length_cutoff_and_bad_frames_never_report_completed() {
    for parts in [vec![(0,delta("建议回答：")),(0,done())],vec![(0,delta("建议回答：部分答案")),(0,format!("data: {}\n\n",json!({"choices":[{"finish_reason":"length"}]}))),(0,done())],vec![(0,"data: not-json\n\n".into())]] {
        let server=serve(vec![ok(parts)]).await;let (result,events)=run(&server,CancellationToken::new()).await;assert!(result.is_err());assert!(!completed(&events));assert_eq!(attempts(&events),1);
    }
    let body="通过幂等键和唯一约束验证重试只写入一次。";
    assert!(!has_useful_answer(body));assert!(has_response_body(body));
    let server=serve(vec![ok(vec![(0,delta(body)),(0,done())])]).await;
    let (result,events)=run(&server,CancellationToken::new()).await;assert!(result.is_ok());assert!(completed(&events));assert_eq!(attempts(&events),1);
    for text in ["","……","建议","建议回答","听写纠正：企业→契约"] {assert!(!has_response_body(text));}
}
