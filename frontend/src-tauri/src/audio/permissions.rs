// macOS audio permissions handling
use anyhow::Result;
use log::{info, warn, error};

#[cfg(target_os = "macos")]
use std::process::Command;

/// Check if the app has Audio Capture permission (required for Core Audio taps on macOS 14.4+)
///
/// Note: Core Audio taps require NSAudioCaptureUsageDescription in Info.plist.
/// When the app first attempts to create a Core Audio tap, macOS will automatically
/// show a permission dialog to the user. If permission is denied, the tap will return
/// silence (all zeros).
///
/// This function returns true because the actual permission prompt happens automatically
/// when AudioHardwareCreateProcessTap is called by the cidre library.
#[cfg(target_os = "macos")]
pub fn check_screen_recording_permission() -> bool {
    info!("ℹ️  Core Audio tap requires Audio Capture permission (macOS 14.4+)");
    info!("📍 Permission dialog will appear automatically when recording starts");
    info!("   If already granted: System Settings → Privacy & Security → Audio Capture");

    // Always return true - the actual permission dialog is triggered by Core Audio API
    true
}

#[cfg(not(target_os = "macos"))]
pub fn check_screen_recording_permission() -> bool {
    true // Not required on other platforms
}

/// Request Audio Capture permission from the user
/// This will open System Settings to the Privacy & Security page
#[cfg(target_os = "macos")]
pub fn request_screen_recording_permission() -> Result<()> {
    info!("🔐 Opening System Settings for Audio Capture permission...");

    // Open System Settings to Privacy & Security page
    // Note: There's no direct URL for Audio Capture, so we open the main Privacy page
    let result = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security")
        .spawn();

    match result {
        Ok(_) => {
            info!("✅ Opened System Settings - navigate to Privacy & Security → Audio Capture");
            info!("👉 Please enable Audio Capture permission and restart the app");
            Ok(())
        }
        Err(e) => {
            error!("❌ Failed to open System Settings: {}", e);
            Err(anyhow::anyhow!("Failed to open System Settings: {}", e))
        }
    }
}

#[cfg(windows)]
pub fn request_screen_recording_permission() -> Result<()> {
    std::process::Command::new("explorer.exe").arg("ms-settings:privacy-microphone").spawn()?;
    Ok(())
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn request_screen_recording_permission() -> Result<()> {
    Err(anyhow::anyhow!("请在系统设置中检查麦克风权限"))
}

/// Check and request Audio Capture permission if not granted
/// Returns true if permission is granted, false otherwise
pub fn ensure_screen_recording_permission() -> bool {
    if check_screen_recording_permission() {
        return true;
    }

    warn!("Audio Capture permission not granted - requesting...");

    if let Err(e) = request_screen_recording_permission() {
        error!("Failed to request Audio Capture permission: {}", e);
        return false;
    }

    false // Permission will be granted after restart
}

/// Tauri command to check Screen Recording permission
#[tauri::command]
pub async fn check_screen_recording_permission_command() -> bool {
    check_screen_recording_permission()
}

/// Tauri command to request Screen Recording permission
#[tauri::command]
pub async fn request_screen_recording_permission_command() -> Result<(), String> {
    request_screen_recording_permission()
        .map_err(|e| e.to_string())
}

/// Trigger system audio permission request and verify it was granted
/// Returns Ok(true) if permission granted (tap created successfully), Ok(false) if denied
#[cfg(target_os = "macos")]
pub fn trigger_system_audio_permission(device: Option<&str>) -> Result<bool> {
    info!("🔐 Triggering Audio Capture permission request...");

    // Try to create a Core Audio capture - this triggers the permission dialog
    // if NSAudioCaptureUsageDescription is present in Info.plist
    // NOTE: We only create the tap, don't start streaming - similar to mic permission approach
    match crate::audio::capture::CoreAudioCapture::for_output(device) {
        Ok(_capture) => {
            info!("✅ Core Audio tap created successfully");
            // Sleep briefly to allow permission dialog to appear (if shown)
            // Similar to microphone permission handling in discovery.rs
            std::thread::sleep(std::time::Duration::from_millis(500));
            info!("✅ Audio Capture permission appears to be granted");
            // Note: On macOS, even with permission denied, tap creation may succeed
            // but audio will be silence. For onboarding, we just check tap creation.
            Ok(true)
        }
        Err(e) => {
            let error_msg = e.to_string().to_lowercase();
            if error_msg.contains("permission") || error_msg.contains("denied") {
                info!("🔐 Audio Capture permission denied");
                info!("👉 Please grant Audio Capture permission in System Settings");
                return Ok(false);
            }
            warn!("⚠️ Failed to create Core Audio tap: {}", e);
            Err(e)
        }
    }
}

#[cfg(windows)]
pub fn trigger_system_audio_permission(name: Option<&str>) -> Result<bool> {
    use cpal::traits::{DeviceTrait,StreamTrait};
    use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
    let selected=match name {
        Some(name)=>super::devices::AudioDevice::new(name.into(),super::devices::DeviceType::Output),
        None=>super::devices::default_output_device()?,
    };
    let (device,config)=super::devices::platform::get_windows_device(&selected)?;
    let failed=Arc::new(AtomicBool::new(false));let failure=failed.clone();
    let stream=device.build_input_stream_raw(&config.config(),config.sample_format(),|_,_|{},move |_|{failure.store(true,Ordering::SeqCst);},None)?;
    stream.play()?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(stream);
    anyhow::ensure!(!failed.load(Ordering::SeqCst),"系统输出通道启动失败，请检查设备连接或独占使用状态");
    Ok(true)
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn trigger_system_audio_permission(_device: Option<&str>) -> Result<bool> {
    // System audio permissions not required on other platforms
    info!("System audio permissions not required on this platform");
    Ok(true)
}

/// Tauri command to trigger system audio permission request
/// Returns true if permission was granted (stream created), false if denied
#[tauri::command]
pub async fn trigger_system_audio_permission_command(output: Option<String>) -> Result<bool, String> {
    // Run in blocking task to avoid blocking the async runtime
    tokio::task::spawn_blocking(move || {
        let device = output.as_deref().map(super::devices::parse_audio_device).transpose()?;
        if let Some(ref device) = device {
            anyhow::ensure!(device.device_type == super::devices::DeviceType::Output, "请选择输出设备");
        }
        trigger_system_audio_permission(device.as_ref().map(|d| d.name.as_str()))
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_permission() {
        let has_permission = check_screen_recording_permission();
        println!("Has Screen Recording permission: {}", has_permission);
    }
}