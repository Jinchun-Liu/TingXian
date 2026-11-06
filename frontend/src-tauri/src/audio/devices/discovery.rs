use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::error;

use super::configuration::AudioDevice;
#[cfg(not(target_os = "macos"))]
use super::configuration::DeviceType;
use super::platform;

/// List all available audio devices on the system
pub async fn list_audio_devices() -> Result<Vec<AudioDevice>> {
    static SCAN: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    bounded_discovery(
        SCAN.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)))
            .clone(),
        std::time::Duration::from_secs(3),
        list_audio_devices_sync,
    )
    .await
}

// Driver calls can block indefinitely. Keep them off the async executor and
// retain the permit in the blocking worker even if its caller stops waiting.
// A stuck driver therefore cannot accumulate workers on every refresh/poll.
async fn bounded_discovery<F>(
    gate: std::sync::Arc<tokio::sync::Semaphore>,
    deadline: std::time::Duration,
    scan: F,
) -> Result<Vec<AudioDevice>>
where
    F: FnOnce() -> Result<Vec<AudioDevice>> + Send + 'static,
{
    bounded_device_operation(gate, deadline, "读取音频设备超时，请稍后刷新；已保存的设备选择保留", scan).await
}

/// Keep a driver operation's slot until the native call actually finishes,
/// even when its caller times out or stops waiting.
pub(crate) async fn bounded_device_operation<T, F>(
    gate: std::sync::Arc<tokio::sync::Semaphore>,
    deadline: std::time::Duration,
    timeout_message: &'static str,
    operation: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::time::timeout(deadline, async move {
        let permit = gate.acquire_owned().await?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        })
        .await
        .map_err(|e| anyhow::anyhow!("音频设备任务失败：{e}"))?
    })
    .await
    .map_err(|_| anyhow::anyhow!(timeout_message))?
}

fn list_audio_devices_sync() -> Result<Vec<AudioDevice>> {
    #[cfg(target_os = "macos")]
    {
        platform::configure_macos_audio()
    }

    #[cfg(not(target_os = "macos"))]
    {
        let host = cpal::default_host();

        // Platform-specific device enumeration
        let mut devices = {
            #[cfg(target_os = "windows")]
            {
                platform::configure_windows_audio(&host)?
            }
            #[cfg(target_os = "linux")]
            {
                platform::configure_linux_audio(&host)?
            }
        };

        // Add any additional devices from the default host
        if let Ok(other_devices) = host.devices() {
            for device in other_devices {
                if let Ok(name) = device.name() {
                    if !devices.iter().any(|d| d.name == name) {
                        devices.push(AudioDevice::new(name, DeviceType::Output));
                    }
                }
            }
        }

        Ok(devices)
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use std::{
        sync::{atomic::{AtomicUsize, Ordering}, Arc},
        time::Duration,
    };

    #[tokio::test]
    async fn stalled_driver_times_out_without_stalling_runtime_or_spawning_more_workers() {
        let gate = Arc::new(tokio::sync::Semaphore::new(1));
        let calls = Arc::new(AtomicUsize::new(0));
        let (release, wait) = std::sync::mpsc::channel();
        let (started, ready) = tokio::sync::oneshot::channel();
        let count = calls.clone();
        let first_gate = gate.clone();
        let first = tokio::spawn(async move {
            bounded_discovery(first_gate, Duration::from_secs(1), move || {
                count.fetch_add(1, Ordering::SeqCst);
                let _ = started.send(());
                let _ = wait.recv_timeout(Duration::from_secs(5));
                Ok(vec![])
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), ready)
            .await
            .expect("the fake driver must have started")
            .unwrap();
        let first = first.await.unwrap();
        assert!(first.unwrap_err().to_string().contains("超时"));
        let count = calls.clone();
        let second = bounded_discovery(gate.clone(), Duration::from_millis(30), move || {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        })
        .await;
        assert!(second.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        release.send(()).unwrap();
        let next = bounded_discovery(gate, Duration::from_secs(1), || Ok(vec![])).await;
        assert!(next.is_ok(), "a completed driver call must release the scan slot");
    }
}

/// Trigger audio permission request on platforms that require it
/// Returns Ok(true) if permission is granted, Ok(false) if denied, Err if something went wrong
pub fn trigger_audio_permission() -> Result<bool> {
    use log::info;

    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            info!("[trigger_audio_permission] No default input device found - permission likely denied");
            return Ok(false);
        }
    };

    let config = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            info!("[trigger_audio_permission] Failed to get input config: {} - permission likely denied", e);
            return Ok(false);
        }
    };

    // Build and start an input stream to trigger the permission request
    let stream = match device.build_input_stream_raw(
        &config.config(), config.sample_format(),
        |_data: &cpal::Data, _: &cpal::InputCallbackInfo| {
            // Do nothing, we just want to trigger the permission request
        },
        |err| error!("Error in audio stream: {}", err),
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            info!("[trigger_audio_permission] Failed to build input stream: {} - permission likely denied", e);
            return Ok(false);
        }
    };

    // Start the stream to actually trigger the permission dialog
    if let Err(e) = stream.play() {
        info!("[trigger_audio_permission] Failed to play stream: {} - permission likely denied", e);
        return Ok(false);
    }

    // Sleep briefly to allow the permission dialog to appear and for stream to actually work
    std::thread::sleep(std::time::Duration::from_millis(500));

    // If we got here, permission was granted
    info!("[trigger_audio_permission] Stream played successfully - permission granted");

    // Stop the stream
    drop(stream);

    Ok(true)
}

/// Verify the microphone is actually delivering audio (macOS permission check).
///
/// A macOS TCC denial can still let an input stream be *created*, but the audio
/// callback then never fires. This builds a brief test stream and waits for the
/// first callback: permission granted → fires in <100ms; denied → times out at
/// 5s and returns Err. Building + playing the stream also triggers the system
/// permission dialog if the state is notDetermined.
///
/// A denied mic fails the record-start path loudly instead of recording dead
/// audio. Deliberately minimal: it checks the default
/// input device and treats the first callback as "granted" (no sample-content
/// inspection, no per-selected-device check).
pub async fn verify_microphone_access() -> anyhow::Result<()> {
    use log::{info, warn};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    static VERIFY: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    bounded_device_operation(
        VERIFY.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1))).clone(),
        std::time::Duration::from_secs(8),
        "麦克风预检超时，请检查设备与权限；也可关闭“记录我的声音”后仅采集系统音频",
        || {
        let host = cpal::default_host();
        let Some(device) = host.default_input_device() else {
            // Let device resolution handle the existing system-audio-only fallback.
            info!("[verify_microphone_access] No microphone device found; skipping access check");
            return Ok(());
        };

        let config = device
            .default_input_config()
            .map_err(|e| anyhow::anyhow!("Failed to get microphone config: {}", e))?;

        let callback_fired = Arc::new(AtomicBool::new(false));
        let callback_fired_clone = callback_fired.clone();

        let stream = device
            .build_input_stream_raw(
                &config.config(), config.sample_format(),
                move |_data: &cpal::Data, _: &cpal::InputCallbackInfo| {
                    callback_fired_clone.store(true, Ordering::SeqCst);
                },
                |err| warn!("[verify_microphone_access] test stream error: {}", err),
                None,
            )
            .map_err(|e| anyhow::anyhow!("Cannot access microphone: {}", e))?;

        stream
            .play()
            .map_err(|e| anyhow::anyhow!("Cannot start microphone: {}", e))?;

        // Wait up to 5s for the first callback.
        for _ in 0..50 {
            if callback_fired.load(Ordering::SeqCst) {
                drop(stream);
                info!("[verify_microphone_access] microphone access verified");
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        drop(stream);
        Err(anyhow::anyhow!(
            "Microphone permission not granted. Please allow microphone access in \
             System Settings > Privacy & Security > Microphone, then try again."
        ))
    }).await
}
