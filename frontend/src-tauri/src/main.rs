#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use log;
use env_logger;

fn main() {
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref()==Some("--windows-platform-check") {
        if let Err(e)=app_lib::tingxian_runtime::windows_platform_check() {
            eprintln!("{e}");std::process::exit(1);
        }
        println!("OK: Windows recoverable archive and in-memory screenshot");
        return;
    }
    if std::env::args().nth(1).as_deref()==Some("--installation-check") {
        if let Err(e)=app_lib::tingxian_runtime::installation_check() {
            eprintln!("{e}");std::process::exit(1);
        }
        println!("OK: bundled Whisper small, document runtime and FFmpeg");
        return;
    }
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "warn");
    }
    env_logger::init();

    // Async logger will be initialized lazily when first needed (after Tauri runtime starts)
    log::info!("Starting application...");
    app_lib::run();
}
