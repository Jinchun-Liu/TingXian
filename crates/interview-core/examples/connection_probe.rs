//! Explicit fixed-text live check; no profile/recording access and no raw response output.
#[tokio::main]
async fn main() {
    if std::env::args().skip(1).collect::<Vec<_>>()!=["--live-fixed-test"] {
        eprintln!("Usage: connection_probe --live-fixed-test (sends fixed text to DeepSeek and may incur usage)");std::process::exit(64);
    }
    let key=std::env::var("DEEPSEEK_API_KEY").unwrap_or_default();
    let result=interview_core::probe_connection(&key,&interview_core::RequestConfig::default(),tokio_util::sync::CancellationToken::new()).await;
    println!("{}",serde_json::to_string(&result).unwrap());
    if !result.success {std::process::exit(1);}
}
