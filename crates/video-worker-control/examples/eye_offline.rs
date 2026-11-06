//! Explicit local diagnostic, not shipped or callable from a webview.
use std::{path::PathBuf,sync::Arc};
use video_worker_control::{BundledWorker,Controller,EyeJob,EyeProvider,Phase};
fn main() {
    let args:Vec<_>=std::env::args_os().collect();
    if args.len()!=8 { eprintln!("eye_offline <verified-exe> <sha256> <width> <height> <frames> <input.bgr> <new-output.bgr>"); std::process::exit(2); }
    let number=|i:usize|args[i].to_str().and_then(|v|v.parse::<u32>().ok()).unwrap_or(0);
    let worker=BundledWorker{path:PathBuf::from(&args[1]),sha256:args[2].to_string_lossy().into_owned()};
    let job=EyeJob{input:PathBuf::from(&args[6]),output:PathBuf::from(&args[7]),width:number(3),height:number(4),frames:number(5),provider:EyeProvider::Ar087};
    let state=Arc::new(Controller::default());
    let result=state.begin_eye_if_current(Some(worker),job,0).expect("explicit initial admission").expect("worker available").execute();
    println!("{}",serde_json::to_string(&result).expect("status serialization"));
    if result.phase!=Phase::Completed { std::process::exit(1); }
}
