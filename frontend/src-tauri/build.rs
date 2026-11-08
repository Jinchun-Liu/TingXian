#[path = "build/ffmpeg.rs"]
mod ffmpeg;
#[path = "build/onnxruntime.rs"]
mod onnxruntime;
#[path = "../../crates/video-worker-control/src/bundle_contract.rs"]
mod bundle_contract;

fn main() {
    pin_video_worker();
    pin_media_bundle();
    // GPU Acceleration Detection and Build Guidance
    detect_and_report_gpu_capabilities();

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=Cocoa");
        println!("cargo:rustc-link-lib=framework=Foundation");

        // Let the enhanced_macos crate handle its own Swift compilation
        // The swift-rs crate build will be handled in the enhanced_macos crate's build.rs
    }

    // Download and bundle FFmpeg binary at build-time to eliminate runtime download delays
    ffmpeg::ensure_ffmpeg_binary();
    onnxruntime::ensure_onnxruntime_runtime();

    tauri_build::build()
}

fn pin_media_bundle() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref()!=Ok("windows") {return;}
    use sha2::{Digest,Sha256};
    let root=std::path::Path::new("binaries/video-worker/media");
    let manifest=root.join(bundle_contract::MANIFEST);
    println!("cargo:rerun-if-changed={}",root.display());
    if !manifest.is_file() {
        assert_ne!(std::env::var("PROFILE").as_deref(),Ok("release"),"Stage native media adapters before release: scripts/stage-windows-video-worker.ps1");
        println!("cargo:rustc-env=TINGXIAN_MEDIA_MANIFEST_SHA256=");return;
    }
    let text=std::fs::read_to_string(&manifest).expect("Cannot read staged media manifest");
    let pins=bundle_contract::Pins::parse(&text).expect("Invalid staged media manifest");
    for entry in std::fs::read_dir(root).expect("Cannot list staged media components") {
        let entry=entry.expect("Cannot inspect staged media entry");
        assert!(entry.file_name().to_str().is_some_and(bundle_contract::allowed_name),"Unreviewed media stage entry");
        assert!(entry.file_type().expect("Cannot inspect media entry type").is_file(),"Media component must be a regular file");
    }
    for (i,name) in bundle_contract::FILES.iter().enumerate() {
        let bytes=std::fs::read(root.join(name)).expect("Staged media component missing");
        assert!(bundle_contract::x64_dll(&bytes),"Staged media component must be an x64 DLL: {name}");
        assert_eq!(format!("{:x}",Sha256::digest(&bytes)),pins.0[i],"Staged media component digest differs: {name}");
    }
    println!("cargo:rustc-env=TINGXIAN_MEDIA_MANIFEST_SHA256={:x}",Sha256::digest(text.as_bytes()));
}

fn pin_video_worker() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref()!=Ok("windows") {return;}
    use sha2::{Digest,Sha256};
    let worker=std::path::Path::new("binaries/video-worker/tingxian-video-worker.exe");
    let pin=std::path::Path::new("binaries/video-worker/worker-sha256.txt");
    println!("cargo:rerun-if-changed={}",worker.display());
    println!("cargo:rerun-if-changed={}",pin.display());
    if !worker.is_file() {
        assert_ne!(std::env::var("PROFILE").as_deref(),Ok("release"),"Stage video worker before release: scripts/stage-windows-video-worker.ps1");
        println!("cargo:rustc-env=TINGXIAN_VIDEO_WORKER_SHA256=");return;
    }
    let expected=std::fs::read_to_string(pin).expect("Staged video worker pin is missing");
    let bytes=std::fs::read(worker).expect("Cannot read staged video worker");
    let actual=format!("{:x}",Sha256::digest(bytes));
    assert_eq!(expected.trim(),actual,"Staged video worker digest differs");
    println!("cargo:rustc-env=TINGXIAN_VIDEO_WORKER_SHA256={actual}");
}

/// Detects GPU acceleration capabilities and provides build guidance
fn detect_and_report_gpu_capabilities() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    println!("cargo:warning=🚀 Building Meetily for: {}", target_os);

    match target_os.as_str() {
        "macos" => {
            println!("cargo:warning=✅ macOS: Metal GPU acceleration ENABLED by default");
            #[cfg(feature = "coreml")]
            println!("cargo:warning=✅ CoreML acceleration ENABLED");
        }
        "windows" => {
            if cfg!(feature = "cuda") {
                println!("cargo:warning=✅ Windows: CUDA GPU acceleration ENABLED");
            } else if cfg!(feature = "vulkan") {
                println!("cargo:warning=✅ Windows: Vulkan GPU acceleration ENABLED");
            } else if cfg!(feature = "openblas") {
                println!("cargo:warning=✅ Windows: OpenBLAS CPU optimization ENABLED");
            } else {
                println!("cargo:warning=⚠️  Windows: Using CPU-only mode (no GPU or BLAS acceleration)");
                println!("cargo:warning=💡 For NVIDIA GPU: cargo build --release --features cuda");
                println!("cargo:warning=💡 For AMD/Intel GPU: cargo build --release --features vulkan");
                println!("cargo:warning=💡 For CPU optimization: cargo build --release --features openblas");

                // Try to detect NVIDIA GPU
                if which::which("nvidia-smi").is_ok() {
                    println!("cargo:warning=🎯 NVIDIA GPU detected! Consider rebuilding with --features cuda");
                }
            }
        }
        "linux" => {
            if cfg!(feature = "cuda") {
                println!("cargo:warning=✅ Linux: CUDA GPU acceleration ENABLED");
            } else if cfg!(feature = "vulkan") {
                println!("cargo:warning=✅ Linux: Vulkan GPU acceleration ENABLED");
            } else if cfg!(feature = "hipblas") {
                println!("cargo:warning=✅ Linux: AMD ROCm (HIP) acceleration ENABLED");
            } else if cfg!(feature = "openblas") {
                println!("cargo:warning=✅ Linux: OpenBLAS CPU optimization ENABLED");
            } else {
                println!("cargo:warning=⚠️  Linux: Using CPU-only mode (no GPU or BLAS acceleration)");
                println!("cargo:warning=💡 For NVIDIA GPU: cargo build --release --features cuda");
                println!("cargo:warning=💡 For AMD GPU: cargo build --release --features hipblas");
                println!("cargo:warning=💡 For other GPUs: cargo build --release --features vulkan");
                println!("cargo:warning=💡 For CPU optimization: cargo build --release --features openblas");

                // Try to detect NVIDIA GPU
                if which::which("nvidia-smi").is_ok() {
                    println!("cargo:warning=🎯 NVIDIA GPU detected! Consider rebuilding with --features cuda");
                }

                // Try to detect AMD GPU
                if which::which("rocm-smi").is_ok() {
                    println!("cargo:warning=🎯 AMD GPU detected! Consider rebuilding with --features hipblas");
                }
            }
        }
        _ => {
            println!("cargo:warning=ℹ️  Unknown platform: {}", target_os);
        }
    }

    // Performance guidance
    if !cfg!(feature = "cuda") && !cfg!(feature = "vulkan") && !cfg!(feature = "hipblas") && !cfg!(feature = "openblas") && target_os != "macos" {
        println!("cargo:warning=📊 Performance: CPU-only builds are significantly slower than GPU/BLAS builds");
        println!("cargo:warning=📚 See README.md for GPU/BLAS setup instructions");
    }
}
