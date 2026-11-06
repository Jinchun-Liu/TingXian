// audio/recording_commands.rs
//
// Slim Tauri command layer for recording functionality.
// Delegates to transcription and recording modules for actual implementation.

use anyhow::Result;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::task::JoinHandle;

use super::{
    recording_manager::RecordingStartError,
    parse_audio_device,
    default_input_device,   // Get default microphone
    default_output_device,  // Get default system audio
    RecordingManager,
};
use super::device_monitor::{DeviceEvent, DeviceMonitorType};

// Import transcription modules
use super::transcription::{
    self,
    reset_speech_detected_flag,
};

// Re-export TranscriptUpdate for backward compatibility
pub use super::transcription::TranscriptUpdate;

// ============================================================================
// GLOBAL STATE
// ============================================================================

// Simple recording state tracking
static IS_RECORDING: AtomicBool = AtomicBool::new(false);

/// True for the whole `stop_recording` tail (manager taken -> `recording-stopped`
/// emitted). `IS_RECORDING` must stay true through the tail — the frontend polls
/// it to keep the stop UI up — so the mic-disconnect fallback checks this flag
/// too, otherwise a fallback queued before Stop retries against a taken manager
/// and surfaces a spurious "Microphone fallback failed" toast.
static IS_RECORDING_STOPPING: AtomicBool = AtomicBool::new(false);

/// Recording is live and not being torn down — the only state in which the
/// mic-disconnect fallback should run or report.
fn recording_live() -> bool {
    IS_RECORDING.load(Ordering::SeqCst) && !IS_RECORDING_STOPPING.load(Ordering::SeqCst)
}

/// Recording is live AND the global manager is still the session `s` belongs to.
/// Used by the mic-disconnect fallback to refuse acting on a *later* recording
/// after a Stop/Start swapped the manager out from under an in-flight task.
///
/// NOTE: this locks `RECORDING_MANAGER`. Never call it while already holding
/// that lock (e.g. inside a `RECORDING_MANAGER.lock()` scope) — the std Mutex
/// is non-reentrant and it would self-deadlock. All current callers invoke it
/// outside any held lock; keep it that way.
fn session_live(s: &Arc<super::RecordingState>) -> bool {
    recording_live() && s.is_recording()
        && RECORDING_MANAGER
            .lock()
            .unwrap()
            .as_ref()
            .map_or(false, |m| Arc::ptr_eq(m.get_state(), s))
}

/// RAII guard for the stop-tail flag. Sets `IS_RECORDING_STOPPING` true on
/// construction and clears it on Drop — including during unwind — so a panic
/// anywhere in the ~320-line stop tail can't leave the flag stuck true and
/// silently kill the mic-disconnect fallback for every later recording.
///
/// This unwind-clears behaviour depends on `panic = "unwind"` (the default).
/// If a release profile ever sets `panic = "abort"`, Drop won't run on panic
/// and the stuck-flag failure mode returns — add a start-time reset then.
struct StoppingGuard;
impl StoppingGuard {
    fn new() -> Self {
        IS_RECORDING_STOPPING.store(true, Ordering::SeqCst);
        StoppingGuard
    }
}
impl Drop for StoppingGuard {
    fn drop(&mut self) {
        IS_RECORDING_STOPPING.store(false, Ordering::SeqCst);
    }
}

/// Shared start-path finalize. Both start commands MUST call this so a new
/// start path can't silently ship with a per-session flag left unreset (e.g.
/// the mic-recovery budget already exhausted).
fn finalize_recording_start() {
    info!("🔍 Setting IS_RECORDING to true and resetting SPEECH_DETECTED_EMITTED");
    IS_RECORDING.store(true, Ordering::SeqCst);
    MIC_FALLBACK_FAILED_ATTEMPTS.store(0, Ordering::SeqCst); // fresh mic-recovery budget per session
    reset_speech_detected_flag(); // reset speech-detected emit latch for the new session
}

// Global recording manager and transcription task to keep them alive during recording
static RECORDING_MANAGER: Mutex<Option<RecordingManager>> = Mutex::new(None);
struct ActiveTranscription {
    session_id:u64,
    handle: JoinHandle<()>,
    control: Arc<transcription::control::TranscriptionControl>,
}
static TRANSCRIPTION_TASK: Mutex<Option<ActiveTranscription>> = Mutex::new(None);

// Listener ID for proper cleanup - prevents microphone from staying active after recording stops
static TRANSCRIPT_LISTENER_ID: Mutex<Option<tauri::EventId>> = Mutex::new(None);

const TRANSCRIPTION_RUNTIME_START_ERROR_CODE: &str =
    "TRANSCRIPTION_RUNTIME_INITIALIZATION_FAILED";
const TRANSCRIPTION_RUNTIME_USER_MESSAGE: &str = "本地语音引擎无法初始化，请重启听弦；若仍失败请重新安装。";

// ============================================================================
// PUBLIC TYPES
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct RecordingArgs {
    pub save_path: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct TranscriptionStatus {
    pub chunks_in_queue: usize,
    pub is_processing: bool,
    pub last_activity_ms: u64,
}

fn map_recording_start_error<R: Runtime>(
    app: &AppHandle<R>,
    error: RecordingStartError,
) -> String {
    crate::tray::update_tray_menu(app);

    match error {
        RecordingStartError::TranscriptionRuntime(source) => {
            error!("Failed to initialize speech recognition: {source:#}");
            let error = RecordingStartError::TranscriptionRuntime(source);
            if let Err(emit_error) = app.emit("transcription-error", serde_json::json!({
                "error": error.to_string(),
                "userMessage": TRANSCRIPTION_RUNTIME_USER_MESSAGE,
                "actionable": false,
                "phase": "startup"
            })) {
                error!("Failed to emit transcription runtime startup error: {emit_error}");
            }
            TRANSCRIPTION_RUNTIME_START_ERROR_CODE.to_string()
        }
        RecordingStartError::Other(error) => format!("Failed to start recording: {error}"),
    }
}

// ============================================================================
// DEVICE RESOLUTION
// ============================================================================

/// Resolve the microphone to record with: requested device (if it actually
/// enumerates) → system default → none (system-audio-only recording).
///
/// The device picker has no "no microphone" option: choosing "Default
/// Microphone" sends `None`, so `None` here means "use the system default",
/// NOT "record without a mic". A specifically-requested mic that isn't in
/// cpal's current enumeration (a stale saved device, or a Continuity
/// "iPhone Microphone" that isn't available right now) is downgraded to the
/// system default — the same `default_input_device()` helper the
/// mid-recording disconnect path uses — so start never hard-fails with
/// "Device not found".
///
/// Emits at most one event per call:
/// - `mic-device-switched` — a specific mic was requested but unavailable,
///   and we fell back to the default (reuses the existing frontend listener).
/// - `mic-unavailable` — no usable mic at all; recording proceeds with
///   system audio only. If system audio is also unavailable, start_streams'
///   own guard reports it.
/// Resolving `None` to the default is the user's actual choice, so it's silent.
///
/// ponytail: sync pre-flight substitution (matches Pro), not catch-and-retry —
/// stream.rs keeps its hard-fail as the last line of defense. cpal calls
/// block briefly either way.
fn resolve_mic_or_default<R: Runtime>(
    app: &AppHandle<R>,
    requested_name: Option<&str>,
) -> Option<Arc<super::AudioDevice>> {
    // Explicit opt-out used only by the interview system-audio entry point.
    // None retains upstream semantics: use the default microphone.
    if requested_name == Some("__interview_disabled__") { return None; }
    use cpal::traits::{DeviceTrait, HostTrait};

    let requested_specific = requested_name.is_some();

    if let Some(name) = requested_name {
        match parse_audio_device(name) {
            Ok(device) => {
                #[cfg(target_os = "macos")]
                let exists = super::devices::platform::configure_macos_audio()
                    .map(|items| items.iter().any(|d| d.name == device.name && d.device_type == super::devices::DeviceType::Input))
                    .unwrap_or(false);
                #[cfg(not(target_os = "macos"))]
                let exists = cpal::default_host()
                    .input_devices()
                    .map(|mut it| it.any(|d| d.name().map(|n| n == device.name).unwrap_or(false)))
                    .unwrap_or(false);
                if exists {
                    info!("✅ Using requested microphone: '{}'", device.name);
                    return Some(Arc::new(device));
                }
                warn!(
                    "⚠️ Requested mic '{}' not enumerated — falling back to system default",
                    device.name
                );
            }
            Err(e) => {
                warn!(
                    "⚠️ Requested mic '{}' not available: {} — falling back to system default",
                    name, e
                );
            }
        }
    }

    match default_input_device() {
        Ok(device) => {
            info!("✅ Using default microphone: '{}'", device.name);
            if requested_specific {
                // Tell the user their selected mic wasn't available and which
                // mic is actually recording. Reuses the mic-device-switched
                // listener the disconnect path wires up.
                let _ = app.emit(
                    "mic-device-switched",
                    serde_json::json!({ "device_name": device.name }),
                );
            }
            Some(Arc::new(device))
        }
        Err(e) => {
            warn!("❌ No microphone available: {} — recording system audio only", e);
            let _ = app.emit("mic-unavailable", serde_json::json!({}));
            None
        }
    }
}

/// System-audio analog of `resolve_mic_or_default`: `Some(name)` -> parse it,
/// falling back to the default output if unparseable; `None` ("Default System
/// Audio" in the UI) -> default output. Returns `None` only when no output
/// device exists — system audio is optional, mic-only recording proceeds.
///
/// ponytail: no cpal enumeration check (unlike the mic helper) — Linux system
/// devices are Pulse/ALSA monitor *inputs* tagged Output, so output_devices()
/// would false-negative them. stream.rs still hard-fails on a missing device.
fn resolve_system_or_default(requested_name: Option<&str>) -> Option<Arc<super::AudioDevice>> {
    if let Some(name) = requested_name {
        match parse_audio_device(name) {
            Ok(device) => {
                info!("✅ Using requested system audio: '{}'", device.name);
                #[cfg(target_os = "macos")]
                let device = super::AudioDevice { pin_output: true, ..device };
                return Some(Arc::new(device));
            }
            Err(e) => warn!(
                "⚠️ Requested system audio '{}' not available: {} — falling back to system default",
                name, e
            ),
        }
    }

    match default_output_device() {
        Ok(device) => {
            info!("✅ Using default system audio: '{}'", device.name);
            Some(Arc::new(device))
        }
        Err(e) => {
            warn!("⚠️ No system audio available: {} — recording will continue with microphone only", e);
            None
        }
    }
}

/// Wake idle audio hardware before checking microphone callbacks, and finish
/// validation before creating any recording resources.
#[cfg(target_os = "macos")]
async fn prepare_audio_for_recording(
    system_device: Option<&super::AudioDevice>,
    microphone_requested: bool,
) -> Result<(), String> {
    run_microphone_preflight(microphone_requested, || async {
    use cpal::traits::{DeviceTrait, HostTrait};

    let wake_name = system_device
        .map(|s| s.name.clone())
        .or_else(|| {
            cpal::default_host()
                .default_output_device()
                .and_then(|d| d.name().ok())
        });
    if let Some(name) = wake_name {
        if let Err(e) = super::recording_manager::wake_audio_connection(&name).await {
            warn!("[AUDIO_WAKE] Wake failed: {} — proceeding anyway", e);
        }
    }

    if let Err(e) = super::devices::verify_microphone_access().await {
        error!("Microphone access verification failed: {}", e);
        return Err(format!("Microphone access required: {}", e));
    }
    Ok(())
    }).await
}

#[cfg(target_os = "macos")]
async fn run_microphone_preflight<F, Fut>(requested: bool, check: F) -> Result<(), String>
where F: FnOnce() -> Fut, Fut: std::future::Future<Output = Result<(), String>> {
    // System-only capture must not wake a CPAL stream, access the microphone,
    // or request its permission as a side effect of pressing Start.
    if !requested { return Ok(()); }
    check().await
}

#[cfg(all(test, target_os = "macos"))]
mod interview_audio_preflight_tests {
    use super::*;

    #[tokio::test]
    async fn system_only_never_constructs_a_microphone_or_wake_probe() {
        let result = run_microphone_preflight(false, || -> std::future::Ready<Result<(), String>> {
            panic!("microphone-off must skip the whole native preflight")
        }).await;
        assert!(result.is_ok());
        // Exercise the production entry point with no output selection. It
        // must return before even querying a default device from the host.
        assert!(prepare_audio_for_recording(None, false).await.is_ok());
    }

    #[tokio::test]
    async fn requested_microphone_preserves_probe_failure() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let result = run_microphone_preflight(true, || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Err("synthetic microphone denied".to_string()))
        }).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(result.unwrap_err(), "synthetic microphone denied");
    }
}


fn store_transcript_update(update:TranscriptUpdate,transcript_session:u64,transcript_buffer:&Arc<Mutex<Vec<super::recording_saver::TranscriptSegment>>>) {
    if update.native_session_id!=Some(transcript_session) {return;}
    // Create structured transcript segment
    let segment = crate::audio::recording_saver::TranscriptSegment {
        id: format!("seg_{}", update.sequence_id),
        text: update.text.clone(),
        audio_start_time: update.audio_start_time,
        audio_end_time: update.audio_end_time,
        duration: update.duration,
        display_time: update.timestamp.clone(), // Use wall-clock timestamp for display
        confidence: update.confidence,
        sequence_id: update.sequence_id,
        source: Some(update.source.clone()),
        utterance_id: update.utterance_id,
        microphone_epoch: update.microphone_epoch,
        replaces_utterance: update.replaces_utterance,
    };

    // Save to recording manager
    if let Ok(manager_guard) = RECORDING_MANAGER.lock() {
        if let Some(manager) = manager_guard.as_ref() {
            if update.native_session_id == Some(manager.get_state().health.session_id) {
                manager.add_transcript_segment(segment);
                return;
            }
        }
    }
    // The manager is temporarily local while devices/pipeline
    // stop. Keep this final text in its original session buffer;
    // final save reads the same Arc, never the next recording.
    if let Ok(mut buffer)=transcript_buffer.lock() {
        if let Some(existing)=buffer.iter_mut().find(|s|s.sequence_id==segment.sequence_id) {*existing=segment;}
        else {buffer.push(segment);}
    }
}

// ============================================================================
// RECORDING COMMANDS
// ============================================================================

/// Start recording with default devices
pub async fn start_recording<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    start_recording_with_meeting_name(app, None).await
}

/// Start recording with default devices and optional meeting name
pub async fn start_recording_with_meeting_name<R: Runtime>(
    app: AppHandle<R>,
    meeting_name: Option<String>,
) -> Result<(), String> {
    info!(
        "Starting recording with default devices, meeting: {:?}",
        meeting_name
    );

    crate::interview_exit::ensure_not_exiting(&app)?;
    let _engine_lifecycle_guard = super::common::acquire_engine_lifecycle_lock().await;
    crate::interview_exit::ensure_not_exiting(&app)?;

    // Check if already recording
    let current_recording_state = IS_RECORDING.load(Ordering::SeqCst);
    info!("🔍 IS_RECORDING state check: {}", current_recording_state);
    if current_recording_state {
        return Err("采集已经开始".to_string());
    }

    if let Err(error) = crate::ensure_onnx_runtime_available() {
        return Err(map_recording_start_error(
            &app,
            RecordingStartError::TranscriptionRuntime(error),
        ));
    }

    // Validate that transcription models are available before starting recording
    info!("🔍 Validating transcription model availability before starting recording...");
    if let Err(validation_error) = transcription::validate_transcription_model_ready(&app).await {
        error!("Model validation failed: {}", validation_error);

        // Emit error event for frontend - actionable: false to show toast instead of modal
        // (download progress is already shown in top-right toast)
        let _ = app.emit("transcription-error", serde_json::json!({
            "error": validation_error,
            "userMessage": format!("无法开始采集：{}", validation_error),
            "actionable": false,
            "phase": "startup"
        }));

        return Err(validation_error);
    }
    info!("✅ Transcription model validation passed");

    // Notify frontend that startup has begun (surfaces STARTING state)
    app.emit("recording-starting", serde_json::json!({
        "message": "Recording initialization started"
    })).map_err(|e| e.to_string())?;

    // Load recording preferences to get auto_save AND device preferences
    let (auto_save, preferred_mic_name, preferred_system_name) =
        match super::recording_preferences::load_recording_preferences(&app).await {
            Ok(prefs) => {
                info!("📋 Loaded recording preferences: auto_save={}, preferred_mic={:?}, preferred_system={:?}",
                      prefs.auto_save, prefs.preferred_mic_device, prefs.preferred_system_device);
                (prefs.auto_save, prefs.preferred_mic_device, prefs.preferred_system_device)
            }
            Err(e) => {
                warn!("Failed to load recording preferences, using defaults: {}", e);
                (true, None, None)
            }
        };

    #[cfg(not(target_os = "macos"))]
    let microphone_device = resolve_mic_or_default(&app, preferred_mic_name.as_deref());

    let system_device = resolve_system_or_default(preferred_system_name.as_deref());

    #[cfg(target_os = "macos")]
    prepare_audio_for_recording(system_device.as_deref(), preferred_mic_name.as_deref() != Some("__interview_disabled__")).await?;

    #[cfg(target_os = "macos")]
    let microphone_device = resolve_mic_or_default(&app, preferred_mic_name.as_deref());

    // Async-first approach - no more blocking operations!
    info!("🚀 Starting async recording initialization");

    // Create new recording manager only after startup validation succeeds
    let mut manager = RecordingManager::new();

    // Always ensure a meeting name is set so incremental saver initializes
    let effective_meeting_name = meeting_name.clone().unwrap_or_else(|| {
        // Example: Meeting 2025-10-03_08-25-23
        let now = chrono::Local::now();
        format!(
            "Meeting {}",
            now.format("%Y-%m-%d_%H-%M-%S")
        )
    });
    manager.set_meeting_name(Some(effective_meeting_name));

    // Set up error callback
    let app_for_error = app.clone();
    let error_health = manager.get_state().health.clone();
    manager.set_error_callback(move |error| {
        let code = if matches!(error, super::recording_state::AudioError::TranscriptionBacklog) { "asr_overflow" } else { "capture_error" };
        super::interview_health::report(&app_for_error, &error_health, code);
        let _ = app_for_error.emit("recording-error", error.user_message());
    });

    // Start recording with resolved devices (replaces start_recording_with_defaults_and_auto_save call)
    let transcription_receiver = manager
        .start_recording(microphone_device, system_device, auto_save)
        .await
        .map_err(|error| map_recording_start_error(&app, error))?;

    // Take the device event receiver BEFORE storing manager globally.
    // A background task will process device events (hot-swap) without frontend polling.
    let device_event_receiver = manager.take_device_event_receiver();
    let session = manager.get_state().clone();
    let transcript_buffer=manager.transcript_buffer();
    let transcript_session=session.health.session_id;

    // Store the manager globally to keep it alive
    {
        let mut global_manager = RECORDING_MANAGER.lock().unwrap();
        *global_manager = Some(manager);
    }

    // Spawn background device event processor (mic-disconnect fallback).
    if let Some(receiver) = device_event_receiver {
        spawn_device_event_processor(app.clone(), receiver, session.clone());
    }

    // Flip recording live + reset per-session flags (speech-detected latch,
    // mic-recovery budget). Shared with the other start path — see helper.
    finalize_recording_start();

    // CRITICAL: Listen for transcript-update events and save to recording manager
    // This enables transcript history persistence for page reload sync
    // Store listener ID for cleanup during stop_recording to ensure microphone is released
    {
        use tauri::Listener;
        let listener_id = app.listen("transcript-update", move |event: tauri::Event| {
            // Parse the transcript update from the event payload
            if let Ok(update) = serde_json::from_str::<TranscriptUpdate>(event.payload()) {
                store_transcript_update(update,transcript_session,&transcript_buffer);
            }
        });
        let mut global_listener = TRANSCRIPT_LISTENER_ID.lock().unwrap();
        *global_listener = Some(listener_id);
        info!("✅ Transcript-update event listener registered for history persistence");
    }

    // Start optimized parallel transcription task and store handle
    let task_handle = transcription::start_transcription_task(app.clone(), transcription_receiver, session.health.clone(), session.transcription_control.clone());
    {
        let mut global_task = TRANSCRIPTION_TASK.lock().unwrap();
        *global_task = Some(ActiveTranscription {session_id:session.health.session_id,handle:task_handle,control:session.transcription_control.clone()});
    }

    // Emit success event
    app.emit("recording-started", serde_json::json!({
        "message": "Recording started successfully with parallel processing",
        "devices": ["Default Microphone", "Default System Audio"],
        "workers": 3
    })).map_err(|e| e.to_string())?;

    // Update tray menu to reflect recording state
    crate::tray::update_tray_menu(&app);

    info!("✅ Recording started successfully with async-first approach");

    Ok(())
}

/// Start recording with specific devices
pub async fn start_recording_with_devices<R: Runtime>(
    app: AppHandle<R>,
    mic_device_name: Option<String>,
    system_device_name: Option<String>,
) -> Result<(), String> {
    start_recording_with_devices_and_meeting(app, mic_device_name, system_device_name, None).await
}

/// Start recording with specific devices and optional meeting name
pub async fn start_recording_with_devices_and_meeting<R: Runtime>(
    app: AppHandle<R>,
    mic_device_name: Option<String>,
    system_device_name: Option<String>,
    meeting_name: Option<String>,
) -> Result<(), String> {
    start_recording_with_asr(app,mic_device_name,system_device_name,meeting_name,None).await
}
pub(crate) async fn start_recording_with_asr<R:Runtime>(app:AppHandle<R>,mic_device_name:Option<String>,system_device_name:Option<String>,meeting_name:Option<String>,cloud:Option<crate::interview_cloud_asr::CloudStart>)->Result<(),String> {
    info!(
        "Starting recording with specific devices: mic={:?}, system={:?}, meeting={:?}",
        mic_device_name, system_device_name, meeting_name
    );

    crate::interview_exit::ensure_not_exiting(&app)?;
    let _engine_lifecycle_guard = super::common::acquire_engine_lifecycle_lock().await;
    crate::interview_exit::ensure_not_exiting(&app)?;

    // Check if already recording
    let current_recording_state = IS_RECORDING.load(Ordering::SeqCst);
    info!("🔍 IS_RECORDING state check: {}", current_recording_state);
    if current_recording_state {
        return Err("采集已经开始".to_string());
    }

    if let Err(error) = crate::ensure_onnx_runtime_available() {
        return Err(map_recording_start_error(
            &app,
            RecordingStartError::TranscriptionRuntime(error),
        ));
    }

    // Validate that transcription models are available before starting recording
    info!("🔍 Validating transcription model availability before starting recording...");
    if cloud.is_none() {
    if let Err(validation_error) = transcription::validate_transcription_model_ready(&app).await {
        error!("Model validation failed: {}", validation_error);

        // Emit error event for frontend - actionable: false to show toast instead of modal
        // (download progress is already shown in top-right toast)
        let _ = app.emit("transcription-error", serde_json::json!({
            "error": validation_error,
            "userMessage": format!("无法开始采集：{}", validation_error),
            "actionable": false,
            "phase": "startup"
        }));

        return Err(validation_error);
    }
    }
    info!("✅ Transcription model validation passed");

    // Notify frontend that startup has begun (surfaces STARTING state)
    app.emit("recording-starting", serde_json::json!({
        "message": "Recording initialization started"
    })).map_err(|e| e.to_string())?;

    #[cfg(not(target_os = "macos"))]
    let mic_device = resolve_mic_or_default(&app, mic_device_name.as_deref());

    let system_device = resolve_system_or_default(system_device_name.as_deref());

    #[cfg(target_os = "macos")]
    prepare_audio_for_recording(system_device.as_deref(), mic_device_name.as_deref() != Some("__interview_disabled__")).await?;

    #[cfg(target_os = "macos")]
    let mic_device = resolve_mic_or_default(&app, mic_device_name.as_deref());

    // Async-first approach for custom devices - no more blocking operations!
    info!("🚀 Starting async recording initialization with custom devices");

    // Create new recording manager
    let mut manager = RecordingManager::new();
    if cloud.is_some(){manager.get_state().enable_cloud_streaming();}

    // Load recording preferences to check auto_save setting
    let auto_save = match super::recording_preferences::load_recording_preferences(&app).await {
        Ok(prefs) => {
            info!("📋 Loaded recording preferences: auto_save={}", prefs.auto_save);
            prefs.auto_save
        }
        Err(e) => {
            warn!("Failed to load recording preferences, defaulting to auto_save=true: {}", e);
            true // Default to saving if preferences can't be loaded
        }
    };

    // Always ensure a meeting name is set so incremental saver initializes
    let effective_meeting_name = meeting_name.clone().unwrap_or_else(|| {
        let now = chrono::Local::now();
        format!(
            "Meeting {}",
            now.format("%Y-%m-%d_%H-%M-%S")
        )
    });
    manager.set_meeting_name(Some(effective_meeting_name));
    if meeting_name.as_deref() == Some("Interview practice") {
        let config = crate::api::api::api_get_transcript_config(app.clone(), app.state(), None).await?;
        let refine = config.as_ref().map_or(false, |c| c.provider == "localWhisper" && c.model == "small");
        manager.get_state().set_utterance_refinement(refine);
    }


    // Set up error callback
    let app_for_error = app.clone();
    let error_health = manager.get_state().health.clone();
    manager.set_error_callback(move |error| {
        let code = if matches!(error, super::recording_state::AudioError::TranscriptionBacklog) { "asr_overflow" } else { "capture_error" };
        super::interview_health::report(&app_for_error, &error_health, code);
        let _ = app_for_error.emit("recording-error", error.user_message());
    });

    // Start recording with specified devices and auto_save setting
    let transcription_receiver = manager
        .start_recording(mic_device, system_device, auto_save)
        .await
        .map_err(|error| map_recording_start_error(&app, error))?;

    // Take the device event receiver BEFORE storing manager globally.
    // A background task will process device events (hot-swap) without frontend polling.
    let device_event_receiver = manager.take_device_event_receiver();
    let session = manager.get_state().clone();
    let transcript_buffer=manager.transcript_buffer();
    let transcript_session=session.health.session_id;

    // Store the manager globally to keep it alive
    {
        let mut global_manager = RECORDING_MANAGER.lock().unwrap();
        *global_manager = Some(manager);
    }

    // Spawn background device event processor (mic-disconnect fallback).
    if let Some(receiver) = device_event_receiver {
        spawn_device_event_processor(app.clone(), receiver, session.clone());
    }

    // Flip recording live + reset per-session flags (speech-detected latch,
    // mic-recovery budget). Shared with the other start path — see helper.
    finalize_recording_start();

    // CRITICAL: Listen for transcript-update events and save to recording manager
    // This enables transcript history persistence for page reload sync
    // Store listener ID for cleanup during stop_recording to ensure microphone is released
    {
        use tauri::Listener;
        let listener_id = app.listen("transcript-update", move |event: tauri::Event| {
            // Parse the transcript update from the event payload
            if let Ok(update) = serde_json::from_str::<TranscriptUpdate>(event.payload()) {
                store_transcript_update(update,transcript_session,&transcript_buffer);
            }
        });
        let mut global_listener = TRANSCRIPT_LISTENER_ID.lock().unwrap();
        *global_listener = Some(listener_id);
        info!("✅ Transcript-update event listener registered for history persistence");
    }

    // Start optimized parallel transcription task and store handle
    let task_handle = if let Some(cloud)=cloud {crate::interview_cloud_asr::start(app.clone(),transcription_receiver,session.clone(),cloud)}else{transcription::start_transcription_task(app.clone(), transcription_receiver, session.health.clone(), session.transcription_control.clone())};
    {
        let mut global_task = TRANSCRIPTION_TASK.lock().unwrap();
        *global_task = Some(ActiveTranscription {session_id:session.health.session_id,handle:task_handle,control:session.transcription_control.clone()});
    }

    // Emit success event
    app.emit("recording-started", serde_json::json!({
        "message": "Recording started with custom devices and parallel processing",
        "devices": [
            mic_device_name.unwrap_or_else(|| "Default Microphone".to_string()),
            system_device_name.unwrap_or_else(|| "Default System Audio".to_string())
        ],
        "workers": 3
    })).map_err(|e| e.to_string())?;

    // Update tray menu to reflect recording state
    crate::tray::update_tray_menu(&app);

    info!("✅ Recording started with custom devices using async-first approach");

    Ok(())
}

/// Stop capture and drain admitted transcription work before cleanup.
pub async fn stop_recording<R: Runtime>(
    app: AppHandle<R>,
    _args: RecordingArgs,
) -> Result<(), String> {
    info!(
        "Stopping capture and draining admitted transcription work"
    );

    // Serialize with starts/imports and with another Stop invocation.
    let _engine_lifecycle=super::common::acquire_engine_lifecycle_lock().await;
    // Check if recording is active
    if !IS_RECORDING.load(Ordering::SeqCst) {
        info!("Recording was not active");
        return Ok(());
    }

    // Emit shutdown progress to frontend
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "stopping_audio",
            "message": "Stopping audio capture...",
            "progress": 20
        }),
    );

    // Step 1: Stop audio capture immediately (no more new chunks) with proper error handling
    let manager_for_cleanup = {
        let mut global_manager = RECORDING_MANAGER.lock().unwrap();
        global_manager.take()
    };

    // Mark the stop tail as in progress so a mic-disconnect fallback that was
    // queued before Stop short-circuits instead of retrying against the taken
    // manager. IS_RECORDING itself stays true until the tail completes — the
    // frontend polls it to keep the stop UI up. RAII so a panic in the tail
    // below can't leave the flag stuck true.
    let _stopping_guard = StoppingGuard::new();

    let stop_result = if let Some(mut manager) = manager_for_cleanup {
        // Use FORCE FLUSH to immediately process all accumulated audio - eliminates 30s delay!
        info!("🚀 Using FORCE FLUSH to eliminate pipeline accumulation delays");
        let result = manager.stop_streams_and_force_flush().await;
        // Store manager back for later cleanup
        let manager_for_cleanup = Some(manager);
        (result, manager_for_cleanup)
    } else {
        warn!("No recording manager found to stop");
        (Ok(()), None)
    };

    let (stop_result, manager_for_cleanup) = stop_result;

    match stop_result {
        Ok(_) => {
            info!("✅ Audio streams stopped successfully - no more chunks will be created");
        }
        Err(e) => {
            error!("❌ Failed to stop audio streams: {}", e);
            *RECORDING_MANAGER.lock().unwrap()=manager_for_cleanup;
            return Err(format!("Failed to stop audio streams: {}", e)); // _stopping_guard clears on return
        }
    }

    // Keep this session reachable while ASR drains so its final transcripts
    // are saved by the existing scoped listener. START stays blocked by
    // IS_RECORDING; fallback/mic changes stay blocked by the stopping guard.
    *RECORDING_MANAGER.lock().unwrap()=manager_for_cleanup;

    // Step 2: Drain admitted work. Earlier admission/ASR errors remain failures.
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "processing_transcripts",
            "message": "Processing remaining transcript chunks...",
            "progress": 40
        }),
    );

    // Wait for transcription work with progress reporting and the deadline below.
    let transcription_task = {
        let mut global_task = TRANSCRIPTION_TASK.lock().unwrap();
        global_task.take()
    };

    let mut cancelled_session=None;
    if let Some(mut active) = transcription_task {
        let result=transcription::control::drain_or_cancel(
            &mut active.handle,&active.control,
            std::time::Duration::from_secs(5),std::time::Duration::from_secs(5),
        ).await;
        match result {
            Some(Ok(()))=>info!("Transcription task and its workers have exited"),
            Some(Err(error))=>warn!("Transcription task exited with error: {}",error),
            None=>{
                // A timeout is not proof of termination. Retain ownership and
                // refuse model unload/new capture; the next Stop can retry.
                *TRANSCRIPTION_TASK.lock().unwrap()=Some(active);
                return Err("语音识别仍未退出，尚未完成停止；已请求中断并保留记录。请稍后再次点击停止采集，持续无响应时重启应用。".into());
            }
        }
        if active.control.is_cancelled() {cancelled_session=Some(active.session_id);}
    }

    if RECORDING_MANAGER.lock().unwrap().as_ref().map_or(false,|manager|manager.get_state().health.has_pending_work()) {
        return Err("识别任务仍在释放资源，尚未完成停止；请稍后再点击停止采集。".into());
    }

    if let Some(session_id)=cancelled_session {
        let _=app.emit("interview-transcription-cancelled",serde_json::json!({"native_session_id":session_id,"message":"未完成的语音转写已取消，已显示文字保留。"}));
    }

    // Remove the listener only after final deliveries/work have finished.
    // Unlisten transcript-update event to prevent lingering references
    {
        use tauri::Listener;
        if let Some(listener_id) = TRANSCRIPT_LISTENER_ID.lock().unwrap().take() {
            app.unlisten(listener_id);
            info!("✅ Transcript-update listener removed");
        }
    }

    let manager_for_cleanup=RECORDING_MANAGER.lock().unwrap().take();

    // Step 3: Unload only after the task and owned workers have actually exited.
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "unloading_model",
            "message": "Unloading speech recognition model...",
            "progress": 70
        }),
    );

    info!("Transcription workers exited. Unloading the transcription model...");

    // Determine which provider was used and unload the appropriate model (with timeout)
    let config = match tokio::time::timeout(
        tokio::time::Duration::from_secs(30), // 30 seconds max for DB operation
        crate::api::api::api_get_transcript_config(
            app.clone(),
            app.clone().state(),
            None,
        )
    )
    .await
    {
        Ok(Ok(Some(config))) => Some(config.provider),
        Ok(Ok(None)) => None,
        Ok(Err(e)) => {
            warn!("⚠️ Failed to get transcript config: {:?}", e);
            None
        }
        Err(_) => {
            warn!("⏱️ Transcript config timeout (30s), continuing shutdown");
            None
        }
    };

    match config.as_deref() {
        Some("parakeet") => {
            info!("🦜 Unloading Parakeet model...");
            let engine_clone = {
                let engine_guard = crate::parakeet_engine::commands::PARAKEET_ENGINE
                    .lock()
                    .unwrap();
                engine_guard.as_ref().cloned()
            };

            if let Some(engine) = engine_clone {
                let current_model = engine
                    .get_current_model()
                    .await
                    .unwrap_or_else(|| "unknown".to_string());
                info!("Current Parakeet model before unload: '{}'", current_model);

                if engine.unload_model().await {
                    info!("✅ Parakeet model '{}' unloaded successfully", current_model);
                } else {
                    warn!("⚠️ Failed to unload Parakeet model '{}'", current_model);
                }
            } else {
                warn!("⚠️ No Parakeet engine found to unload model");
            }
        }
        _ => {
            // Default to Whisper
            info!("🎤 Unloading Whisper model...");
            let engine_clone = {
                let engine_guard = crate::whisper_engine::commands::WHISPER_ENGINE
                    .lock()
                    .unwrap();
                engine_guard.as_ref().cloned()
            };

            if let Some(engine) = engine_clone {
                let current_model = engine
                    .get_current_model()
                    .await
                    .unwrap_or_else(|| "unknown".to_string());
                info!("Current Whisper model before unload: '{}'", current_model);

                if engine.unload_model().await {
                    info!("✅ Whisper model '{}' unloaded successfully", current_model);
                } else {
                    warn!("⚠️ Failed to unload Whisper model '{}'", current_model);
                }
            } else {
                warn!("⚠️ No Whisper engine found to unload model");
            }
        }
    }

    // Step 3.5: Track meeting ended analytics with privacy-safe metadata
    // Extract all data from manager BEFORE any async operations to avoid Send issues
    let analytics_data = if let Some(ref manager) = manager_for_cleanup {
        let state = manager.get_state();
        let stats = state.get_stats();

        Some((
            manager.get_recording_duration(),
            manager.get_active_recording_duration().unwrap_or(0.0),
            manager.get_total_pause_duration(),
            manager.get_transcript_segments().len() as u64,
            state.has_fatal_error(),
            state.get_microphone_device().map(|d| d.name.clone()),
            state.get_system_device().map(|d| d.name.clone()),
            stats.chunks_processed,
        ))
    } else {
        None
    };

    // Now perform async analytics tracking without holding manager reference
    if let Some((total_duration, active_duration, pause_duration, transcript_segments_count, had_fatal_error, mic_device_name, sys_device_name, chunks_processed)) = analytics_data {
        info!("📊 Collecting analytics for meeting end");

        // Helper function to classify device type from device name (privacy-safe)
        fn classify_device_type(device_name: &str) -> &'static str {
            let name_lower = device_name.to_lowercase();
            // Check for Bluetooth keywords
            if name_lower.contains("bluetooth")
                || name_lower.contains("airpods")
                || name_lower.contains("beats")
                || name_lower.contains("headphones")
                || name_lower.contains("bt ")
                || name_lower.contains("wireless") {
                "Bluetooth"
            } else {
                "Wired"
            }
        }

        // Get transcription model info (already loaded above for model unload)
        let transcription_config = match crate::api::api::api_get_transcript_config(
            app.clone(),
            app.clone().state(),
            None,
        )
        .await
        {
            Ok(Some(config)) => Some((config.provider, config.model)),
            _ => None,
        };

        let (transcription_provider, transcription_model) = transcription_config
            .unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()));

        // Get summary model info from API
        let summary_config = match crate::api::api::api_get_model_config(
            app.clone(),
            app.clone().state(),
            None,
        )
        .await
        {
            Ok(Some(config)) => Some((config.provider, config.model)),
            _ => None,
        };

        let (summary_provider, summary_model) = summary_config
            .unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()));

        // Classify device types (privacy-safe)
        let microphone_device_type = mic_device_name
            .as_ref()
            .map(|name| classify_device_type(name))
            .unwrap_or("Unknown");

        let system_audio_device_type = sys_device_name
            .as_ref()
            .map(|name| classify_device_type(name))
            .unwrap_or("Unknown");

        // Track meeting ended event with privacy-safe data
        match crate::analytics::commands::track_meeting_ended(
            transcription_provider.clone(),
            transcription_model.clone(),
            summary_provider.clone(),
            summary_model.clone(),
            total_duration,
            active_duration,
            pause_duration,
            microphone_device_type.to_string(),
            system_audio_device_type.to_string(),
            chunks_processed,
            transcript_segments_count,
            had_fatal_error,
        )
        .await
        {
            Ok(_) => info!("✅ Analytics tracked successfully for meeting end"),
            Err(e) => warn!("⚠️ Failed to track analytics: {}", e),
        }
    }

    // Step 4: Finalize recording state and cleanup resources safely
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "finalizing",
            "message": "Finalizing recording and cleaning up resources...",
            "progress": 90
        }),
    );

    // Perform final cleanup with the manager if available
    let (meeting_folder, meeting_name) = if let Some(mut manager) = manager_for_cleanup {
        info!("🧹 Performing final cleanup and saving recording data");

        // Extract meeting info BEFORE async operations
        let meeting_folder = manager.get_meeting_folder();
        let meeting_name = manager.get_meeting_name();

        match tokio::time::timeout(
            tokio::time::Duration::from_secs(300), // 5 minutes max for file I/O
            manager.save_recording_only(&app)
        ).await {
            Ok(Ok(_)) => {
                info!("✅ Recording data saved successfully during cleanup");
            }
            Ok(Err(e)) => {
                warn!(
                    "⚠️ Error during recording cleanup (transcripts preserved): {}",
                    e
                );
                // Don't fail shutdown - transcripts are already preserved
            }
            Err(_) => {
                warn!("⏱️ File I/O timeout (5 minutes) reached during save, continuing shutdown");
                // Don't fail shutdown - transcripts are already preserved
            }
        }

        (meeting_folder, meeting_name)
    } else {
        info!("ℹ️ No recording manager available for cleanup");
        (None, None)
    };

    // Set recording flag to false
    info!("🔍 Setting IS_RECORDING to false");
    IS_RECORDING.store(false, Ordering::SeqCst);
    // IS_RECORDING_STOPPING is cleared by _stopping_guard on scope exit.

    // Step 4.5: Prepare metadata for frontend (NO database save)
    // NOTE: We do NOT save to database here. The frontend will save after all transcripts are displayed.
    // This ensures the user sees all transcripts streaming in before the database save happens.
    let (folder_path_str, meeting_name_str) = match (&meeting_folder, &meeting_name) {
        (Some(path), Some(name)) => (
            Some(path.to_string_lossy().to_string()),
            Some(name.clone()),
        ),
        _ => (None, None),
    };

    info!("📤 Preparing recording metadata for frontend save");
    info!("   folder_path: {:?}", folder_path_str);
    info!("   meeting_name: {:?}", meeting_name_str);

    // Database save removed - frontend will handle this after receiving all transcripts
    info!("ℹ️ Skipping database save in Rust - frontend will save after all transcripts received");

    // Step 5: Complete shutdown
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "complete",
            "message": "Recording stopped successfully",
            "progress": 100
        }),
    );

    // Emit final stop event with folder_path and meeting_name for frontend to save
    app.emit(
        "recording-stopped",
        serde_json::json!({
            "message": "Recording stopped - frontend will save after all transcripts received",
            "folder_path": folder_path_str,
            "meeting_name": meeting_name_str
        }),
    )
    .map_err(|e| e.to_string())?;

    // Update tray menu to reflect stopped state
    crate::tray::update_tray_menu(&app);

    info!("Recording stopped; completed transcripts are retained and any cancelled work was reported");
    Ok(())
}

/// Check if recording is active
pub async fn is_recording() -> bool {
    IS_RECORDING.load(Ordering::SeqCst)
}

/// Get recording statistics
pub async fn get_transcription_status() -> TranscriptionStatus {
    TranscriptionStatus {
        chunks_in_queue: 0,
        is_processing: IS_RECORDING.load(Ordering::SeqCst),
        last_activity_ms: 0,
    }
}

/// Pause the current recording
#[tauri::command]
pub async fn pause_recording<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    info!("Pausing recording");

    // Check if currently recording
    if !IS_RECORDING.load(Ordering::SeqCst) {
        return Err("当前没有正在进行的采集".to_string());
    }

    // Access the recording manager and pause it
    let manager_guard = RECORDING_MANAGER.lock().unwrap();
    if let Some(manager) = manager_guard.as_ref() {
        manager.pause_recording().map_err(|e| e.to_string())?;

        // Emit pause event to frontend
        app.emit(
            "recording-paused",
            serde_json::json!({
                "message": "Recording paused"
            }),
        )
        .map_err(|e| e.to_string())?;

        // Update tray menu to reflect paused state
        crate::tray::update_tray_menu(&app);

        info!("Recording paused successfully");
        Ok(())
    } else {
        Err("No recording manager found".to_string())
    }
}

/// Resume the current recording
#[tauri::command]
pub async fn resume_recording<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    info!("Resuming recording");

    // Check if currently recording
    if !IS_RECORDING.load(Ordering::SeqCst) {
        return Err("当前没有正在进行的采集".to_string());
    }

    // Access the recording manager and resume it
    let manager_guard = RECORDING_MANAGER.lock().unwrap();
    if let Some(manager) = manager_guard.as_ref() {
        manager.resume_recording().map_err(|e| e.to_string())?;

        // Emit resume event to frontend
        app.emit(
            "recording-resumed",
            serde_json::json!({
                "message": "Recording resumed"
            }),
        )
        .map_err(|e| e.to_string())?;

        // Update tray menu to reflect resumed state
        crate::tray::update_tray_menu(&app);

        info!("Recording resumed successfully");
        Ok(())
    } else {
        Err("No recording manager found".to_string())
    }
}

/// Check if recording is currently paused
#[tauri::command]
pub async fn is_recording_paused() -> bool {
    let manager_guard = RECORDING_MANAGER.lock().unwrap();
    if let Some(manager) = manager_guard.as_ref() {
        manager.is_paused()
    } else {
        false
    }
}

/// Get detailed recording state
#[tauri::command]
pub async fn get_recording_state() -> serde_json::Value {
    let is_recording = IS_RECORDING.load(Ordering::SeqCst);
    let manager_guard = RECORDING_MANAGER.lock().unwrap();

    if let Some(manager) = manager_guard.as_ref() {
        serde_json::json!({
            "is_recording": is_recording,
            "is_paused": manager.is_paused(),
            "is_active": manager.is_active(),
            "recording_duration": manager.get_recording_duration(),
            "active_duration": manager.get_active_recording_duration(),
            "total_pause_duration": manager.get_total_pause_duration(),
            "current_pause_duration": manager.get_current_pause_duration()
        })
    } else {
        serde_json::json!({
            "is_recording": is_recording,
            "is_paused": false,
            "is_active": false,
            "recording_duration": null,
            "active_duration": null,
            "total_pause_duration": 0.0,
            "current_pause_duration": null
        })
    }
}

/// Get the meeting folder path for the current recording
/// Returns the path if a meeting name was set and folder structure initialized
#[tauri::command]
pub async fn get_meeting_folder_path() -> Result<Option<String>, String> {
    let manager_guard = RECORDING_MANAGER.lock().unwrap();
    if let Some(manager) = manager_guard.as_ref() {
        Ok(manager.get_meeting_folder().map(|p| p.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

/// Get accumulated transcript segments from current recording session
/// Used for syncing frontend state after page reload during active recording
#[tauri::command]
pub async fn get_transcript_history() -> Result<Vec<crate::audio::recording_saver::TranscriptSegment>, String> {
    let manager_guard = RECORDING_MANAGER.lock().unwrap();

    if let Some(manager) = manager_guard.as_ref() {
        Ok(manager.get_transcript_segments())
    } else {
        Ok(Vec::new()) // No recording active, return empty
    }
}

/// Get meeting name from current recording session
/// Used for syncing frontend state after page reload during active recording
#[tauri::command]
pub async fn get_recording_meeting_name() -> Result<Option<String>, String> {
    let manager_guard = RECORDING_MANAGER.lock().unwrap();

    if let Some(manager) = manager_guard.as_ref() {
        Ok(manager.get_meeting_name())
    } else {
        Ok(None)
    }
}

// ============================================================================
// DEVICE MONITORING COMMANDS (AirPods/Bluetooth disconnect/reconnect support)
// ============================================================================

/// Get information about the active audio output device
/// Used to warn users about Bluetooth playback issues
#[tauri::command]
pub async fn get_active_audio_output() -> Result<super::playback_monitor::AudioOutputInfo, String> {
    super::playback_monitor::get_active_audio_output()
        .await
        .map_err(|e| format!("Failed to get audio output info: {}", e))
}


// ============================================================================
// MIC HOT-SWAP (disconnect recovery)
// ============================================================================

// Guard against concurrent mic hot-swap tasks. Only used by the disconnect
// fallback path (trigger_mic_fallback_to_default) — the "chase the new
// default" auto-swap has been removed.
static MIC_SWAP_IN_PROGRESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Serialize)]
pub struct MicrophoneStatus { pub recording: bool, pub enabled: bool, pub epoch: Option<u64>, pub health:Option<super::interview_health::AudioHealthSnapshot> }

pub fn microphone_status() -> MicrophoneStatus {
    let guard = RECORDING_MANAGER.lock().unwrap();
    let gate = guard.as_ref().map(|m|m.get_state().microphone_gate());
    let recording=recording_live();
    let enabled=recording && gate.as_ref().map_or(false, |g|g.enabled());
    let health=guard.as_ref().map(|m| {let state=m.get_state();state.health.snapshot(recording,state.is_recording(),state.is_paused(),state.get_system_device().is_some(),enabled)});
    MicrophoneStatus { recording, enabled, epoch: gate.map(|g|g.epoch),health }
}

pub async fn set_microphone_enabled<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> Result<MicrophoneStatus, String> {
    if MIC_SWAP_IN_PROGRESS.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err("麦克风设备正在切换，请稍后重试".into());
    }
    struct Guard;
    impl Drop for Guard { fn drop(&mut self) { MIC_SWAP_IN_PROGRESS.store(false, Ordering::SeqCst); } }
    let _guard = Guard;
    let session = {
        let manager = RECORDING_MANAGER.lock().unwrap();
        if !recording_live() { return Err("请先开始采集".into()); }
        manager.as_ref().ok_or("采集正在停止")?.get_state().clone()
    };
    if !session.is_recording() {return Err("采集正在结束，请先完成停止操作".into());}
    if session.microphone_gate().enabled() == enabled { return Ok(microphone_status()); }
    if !enabled {
        // Revoke before releasing the device: queued/callback/in-flight audio
        // cannot escape into transcripts while CPAL is being torn down.
        session.disable_microphone();
        let stream = {
            let mut manager = RECORDING_MANAGER.lock().unwrap();
            manager.as_mut().filter(|m|Arc::ptr_eq(m.get_state(), &session)).and_then(|m|m.take_mic_stream_for_swap())
        };
        if let Some(stream) = stream {
            tokio::task::spawn_blocking(move || stream.stop()).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
        }
    } else {
        let prefs = super::recording_preferences::load_recording_preferences(app).await.map_err(|e|e.to_string())?;
        let device = resolve_mic_or_default(app, prefs.preferred_mic_device.as_deref()).ok_or("没有可用的麦克风")?;
        if !session_live(&session) { return Err("采集已停止".into()); }
        session.renew_microphone();
        if let Err(error) = do_mic_swap(&device.name, &session).await {
            session.disable_microphone();
            return Err(error);
        }
    }
    let status = microphone_status();
    let _ = app.emit("interview-microphone-state", &status);
    Ok(status)
}

// Bounded retry budget for the disconnect fallback (P1 #2). Counts COMPLETED
// failed attempts; MIC_SWAP_IN_PROGRESS still prevents overlapping swaps.
static MIC_FALLBACK_FAILED_ATTEMPTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const MAX_MIC_FALLBACK_ATTEMPTS: u32 = 3;

/// Perform mic hot-swap using phased locking — never holds RECORDING_MANAGER during I/O
/// except the brief mic-stream stop in Phase 1.
/// If CPAL hangs during stream creation, only this task blocks; stop flow stays unblocked.
async fn perform_mic_hot_swap_task<R: Runtime>(
    new_device_name: String,
    session: &Arc<super::RecordingState>,
    app: AppHandle<R>,
) -> Result<(), String> {
    info!("[HOT_SWAP] Starting mic hot-swap to '{}'", new_device_name);

    match do_mic_swap(&new_device_name, session).await {
        Ok(()) => {
            info!("[HOT_SWAP] Mic switched to '{}'", new_device_name);
            let _ = app.emit("mic-device-switched", serde_json::json!({
                "device_name": new_device_name
            }));
            Ok(())
        }
        Err(e) => {
            if !session_live(session) {
                return Err(e);
            }
            warn!("[HOT_SWAP] First attempt failed: {} — retrying in 500ms", e);
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            match do_mic_swap(&new_device_name, session).await {
                Ok(()) => {
                    info!("[HOT_SWAP] Mic switched to '{}' on retry", new_device_name);
                    let _ = app.emit("mic-device-switched", serde_json::json!({
                        "device_name": new_device_name
                    }));
                    Ok(())
                }
                Err(e) => {
                    error!("[HOT_SWAP] Mic swap failed after retry: {}", e);
                    if session_live(session) {
                        let _ = app.emit("mic-swap-failed", serde_json::json!({
                            "error": e,
                            "device_name": new_device_name
                        }));
                    }
                    Err(e)
                }
            }
        }
    }
}

/// Phased mic swap — lock is never held during async I/O.
async fn do_mic_swap(device_name: &str, session: &Arc<super::RecordingState>) -> Result<(), String> {
    if !session.microphone_gate().enabled() { return Err("本软件麦克风已关闭".into()); }
    // Phase 1: Lock briefly — verify identity, take old stream OUT (no teardown under lock)
    let old_mic = {
        let mut guard = RECORDING_MANAGER.lock().unwrap();
        let manager = guard.as_mut().ok_or_else(|| "Recording manager not available".to_string())?;
        if !manager.is_recording() {
            return Err("Recording stopped — aborting mic hot-swap".to_string());
        }
        if !Arc::ptr_eq(manager.get_state(), session) {
            return Err("Session changed before hot-swap — aborting".to_string());
        }
        manager.take_mic_stream_for_swap()
    }; // lock released

    // Tear down the dead mic OUTSIDE the lock — cpal stop()/drop on a
    // disconnected BT device can stall on the CoreAudio HAL lock; doing it
    // under RECORDING_MANAGER would freeze stop_recording (deep-review #2).
    // Non-fatal: the replacement stream is created next regardless, so a
    // teardown error/stall on the already-dead device must not abort the swap.
    if let Some(s) = old_mic {
        if let Err(e) = s.stop() {
            warn!("[HOT_SWAP] Failed to stop old mic stream (proceeding): {}", e);
        }
    }

    // Phase 2: Async I/O WITHOUT lock — may be slow, that's OK
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Build the AudioDevice directly from the name — the caller
    // (trigger_mic_fallback_to_default) already resolved it via
    // default_input_device(). Skipping list_audio_devices() here avoids a
    // full cpal enumeration on the exact BT-transition hot path where it's
    // known to hang 100+ s (see H2 in PR-175 review). The real device
    // validation happens inside AudioStream::create → get_device_and_config
    // which does a targeted host.input_devices() lookup by name.
    let device_arc = std::sync::Arc::new(super::AudioDevice::new(
        device_name.to_string(),
        super::DeviceType::Input,
    ));

    info!("[HOT_SWAP] Creating new mic stream for '{}' (lock released)", device_name);
    let new_stream = super::stream::AudioStream::create(
        device_arc.clone(),
        session.clone(),
        super::recording_state::DeviceType::Microphone,
        None,
    ).await.map_err(|e| format!("Failed to create mic stream: {}", e))?;

    // Resolve the current default output OUTSIDE the lock — a CoreAudio stall here
    // must not block stop_recording (which needs RECORDING_MANAGER). (P1 #1)
    let system_name = default_output_device().ok().map(|d| d.name);

    // Phase 3: Lock briefly — install ONLY if still the same session
    {
        let mut guard = RECORDING_MANAGER.lock().unwrap();
        match guard.as_mut() {
            Some(manager) if Arc::ptr_eq(manager.get_state(), session) && recording_live() && session.microphone_gate().enabled() => {
                manager.set_mic_stream_after_swap(new_stream, device_arc, system_name);
                info!("[HOT_SWAP] Mic hot-swap to '{}' completed", device_name);
            }
            Some(_) => {
                return Err("Session changed during hot-swap — discarding stale mic stream".to_string());
            }
            None => {
                return Err("Recording manager gone during hot-swap".to_string());
            }
        }
    } // lock released

    Ok(())
}

/// Background processor for device monitor events during a recording session.
///
/// The ONLY mid-recording mic switch that is allowed is the fallback from a
/// dead device to the system default, triggered by the device monitor's
/// DeviceDisconnected event. Any other device event is explicitly ignored —
/// recording stays on whatever device was picked at start time until the
/// meeting ends.
///
/// Rationale: auto-swapping to a freshly-connected BT device during recording
/// triggers a reliable hang inside cpal's stream creation on macOS. Locking
/// the device at start eliminates that hang and also makes the recording
/// session predictable.
///
/// The task stops automatically when the receiver is dropped (recording
/// ends / monitor stops).
fn spawn_device_event_processor<R: Runtime>(
    app: AppHandle<R>,
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<DeviceEvent>,
    session: Arc<super::RecordingState>,
) {
    tokio::spawn(async move {
        info!("[DEVICE_EVENTS] Background event processor started");

        while let Some(event) = receiver.recv().await {
            // Skip if recording has stopped
            if !session_live(&session) {
                info!("[DEVICE_EVENTS] Recording stopped — ignoring event: {:?}", event);
                continue;
            }

            match event {
                DeviceEvent::DeviceDisconnected { ref device_name, ref device_type } => {
                    info!("[DEVICE_EVENTS] Device disconnected: '{}' ({:?})", device_name, device_type);
                    // The only automatic mid-recording mic change allowed:
                    // when the active microphone dies, fall back to the
                    // system default input. Triggered after the device
                    // monitor's polling threshold fires.
                    if matches!(device_type, DeviceMonitorType::SystemAudio) {
                        super::interview_health::report(&app, &session.health, "system_disconnected");
                    }
                    if matches!(device_type, DeviceMonitorType::Microphone) && session.microphone_gate().enabled() {
                        super::interview_health::report(&app, &session.health, "microphone_disconnected");
                        let name = device_name.clone();
                        let app_clone = app.clone();
                        let session = session.clone();
                        tokio::spawn(async move {
                            trigger_mic_fallback_to_default(app_clone, name, session).await;
                        });
                    }
                }
                DeviceEvent::DeviceReconnected { ref device_name, ref device_type } => {
                    // Per product decision: once we have fallen back to the
                    // built-in mic we stay there for the rest of the meeting.
                    // This is intentional — just log and do nothing.
                    info!("[DEVICE_EVENTS] Device reconnected: '{}' ({:?}) — staying on current mic (fallback is sticky)", device_name, device_type);
                }
                DeviceEvent::DeviceListChanged => {
                    debug!("[DEVICE_EVENTS] Device list changed");
                }
            }
        }
        info!("[DEVICE_EVENTS] Background event processor stopped (channel closed)");
    });
}

/// Disconnect fallback: swap the active mic to the system default input
/// device. Triggered from the background device event processor after the
/// device monitor's polling threshold (3 × 2s) fires `DeviceDisconnected`
/// for the active microphone.
///
/// `disconnected_name` is the device that just died. We keep it to detect
/// the edge case where macOS hasn't yet updated the system default input
/// away from the dead device — we wait and retry in that case rather than
/// swapping back to the same broken device.
///
/// This function takes the MIC_SWAP_IN_PROGRESS guard itself; the caller
/// must NOT already hold it. If a swap is somehow already running this
/// returns immediately.
async fn trigger_mic_fallback_to_default<R: Runtime>(
    app: AppHandle<R>,
    disconnected_name: String,
    session: Arc<super::RecordingState>,
) {
    if !session_live(&session) || !session.microphone_gate().enabled() {
        info!(
            "[MIC_FALLBACK] Not recording — skipping fallback for '{}'",
            disconnected_name
        );
        return;
    }

    if MIC_FALLBACK_FAILED_ATTEMPTS.load(Ordering::SeqCst) >= MAX_MIC_FALLBACK_ATTEMPTS {
        warn!(
            "[MIC_FALLBACK] {} failed attempts reached — giving up on '{}' (terminal event already announced)",
            MAX_MIC_FALLBACK_ATTEMPTS, disconnected_name
        );
        return;
    }

    if MIC_SWAP_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        info!(
            "[MIC_FALLBACK] Swap already in progress — skipping fallback for '{}'",
            disconnected_name
        );
        return;
    }

    // Guard that clears MIC_SWAP_IN_PROGRESS on any return path below so a
    // panic or early return can't leave the flag stuck.
    struct SwapGuard;
    impl Drop for SwapGuard {
        fn drop(&mut self) {
            MIC_SWAP_IN_PROGRESS.store(false, Ordering::SeqCst);
        }
    }
    let _guard = SwapGuard;

    info!(
        "[MIC_FALLBACK] Starting fallback from disconnected device '{}'",
        disconnected_name
    );

    // Let macOS finish swapping the system default input away from the dead
    // device. 150ms is enough in practice for the built-in mic to become the
    // default when an explicitly-selected BT device disconnects.
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // A Stop-A/Start-B during the sleep swapped our session out. Bail silently —
    // emitting or spending the recovery budget here would fire against B with
    // A's device. Covers the default_input_device() error branch below.
    if !session_live(&session) {
        info!("[MIC_FALLBACK] Session no longer live after wait — aborting fallback for '{}'", disconnected_name);
        return;
    }

    // Query the current system default input. If it still reports the
    // disconnected device, back off once more and re-query — this handles
    // the edge case where the OS hasn't propagated the change yet.
    let fallback_name = match default_input_device() {
        Ok(dev) => dev.name,
        Err(e) => {
            error!("[MIC_FALLBACK] Failed to query default input device: {}", e);
            let _ = app.emit(
                "mic-swap-failed",
                serde_json::json!({
                    "error": format!("Failed to query default input: {}", e),
                    "device_name": disconnected_name,
                }),
            );
            let n = MIC_FALLBACK_FAILED_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
            if n == MAX_MIC_FALLBACK_ATTEMPTS {
                let _ = app.emit(
                    "mic-recovery-exhausted",
                    serde_json::json!({ "device_name": disconnected_name }),
                );
            }
            return;
        }
    };

    let fallback_name = if fallback_name == disconnected_name {
        warn!(
            "[MIC_FALLBACK] Default input still reports disconnected device '{}' — retrying after 300ms",
            disconnected_name
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        if !session_live(&session) {
            info!("[MIC_FALLBACK] Session no longer live after retry wait — aborting fallback for '{}'", disconnected_name);
            return;
        }
        match default_input_device() {
            Ok(dev) if dev.name != disconnected_name => dev.name,
            Ok(dev) => {
                error!(
                    "[MIC_FALLBACK] Default input still '{}' after retry — aborting fallback",
                    dev.name
                );
                let _ = app.emit(
                    "mic-swap-failed",
                    serde_json::json!({
                        "error": "System default input still reports disconnected device after retry",
                        "device_name": disconnected_name,
                    }),
                );
                let n = MIC_FALLBACK_FAILED_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
                if n == MAX_MIC_FALLBACK_ATTEMPTS {
                    let _ = app.emit(
                        "mic-recovery-exhausted",
                        serde_json::json!({ "device_name": disconnected_name }),
                    );
                }
                return;
            }
            Err(e) => {
                error!("[MIC_FALLBACK] Failed to re-query default input device: {}", e);
                let _ = app.emit(
                    "mic-swap-failed",
                    serde_json::json!({
                        "error": format!("Failed to re-query default input: {}", e),
                        "device_name": disconnected_name,
                    }),
                );
                let n = MIC_FALLBACK_FAILED_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
                if n == MAX_MIC_FALLBACK_ATTEMPTS {
                    let _ = app.emit(
                        "mic-recovery-exhausted",
                        serde_json::json!({ "device_name": disconnected_name }),
                    );
                }
                return;
            }
        }
    } else {
        fallback_name
    };

    info!(
        "[MIC_FALLBACK] Falling back '{}' → '{}'",
        disconnected_name, fallback_name
    );

    // macOS Core Audio pre-wake for the hot-swap path — before we call the
    // rebuild path (which internally calls `AudioDeviceStart` on the new
    // mic), play 150ms of digital silence through the current system
    // output device to force the Core Audio hardware unit out of its idle
    // power state. Without this, `AudioDeviceStart` can return `noErr` but
    // the IO proc will not fire for 10-30 seconds until some other audio
    // nudges the hardware awake — the "backend idle until you play YouTube"
    // symptom from earlier testing.
    //
    // `wake_audio_connection_for_swap` has a built-in fallback: if the
    // current system device name doesn't enumerate (e.g. the BT output just
    // disappeared), it plays through `default_output_device()` instead,
    // which on macOS will now be the built-in speakers — exactly the
    // hardware unit we want to wake for the fallback mic.
    //
    // Non-fatal: on error we log and proceed to the swap anyway. A failed
    // wake is strictly better than no wake.
    #[cfg(target_os = "macos")]
    {
        // Read from the captured session directly (no manager lock) — this
        // stays correct even if the global manager has since been swapped by
        // a Stop/Start of a different session.
        let sys_device_name = session.get_system_device().map(|d| d.name.clone());
        if let Some(name) = sys_device_name {
            match super::recording_manager::wake_audio_connection_for_swap(&name).await {
                Ok(()) => info!("[MIC_FALLBACK] Pre-swap audio wake completed"),
                Err(e) => warn!(
                    "[MIC_FALLBACK] Pre-swap audio wake failed: {} — proceeding anyway",
                    e
                ),
            }
        } else {
            log::debug!("[MIC_FALLBACK] No system device recorded — skipping pre-swap wake");
        }
    }

    // Stop may have started during the sleeps above — bail before touching
    // the (possibly already taken) manager.
    if !session_live(&session) {
        info!("[MIC_FALLBACK] Recording stopping — aborting fallback for '{}'", disconnected_name);
        return;
    }

    // perform_mic_hot_swap_task performs its own retry-once logic on failure
    // and emits the mic-device-switched / mic-swap-failed events, so we can
    // just delegate here. It does NOT touch MIC_SWAP_IN_PROGRESS internally.
    match perform_mic_hot_swap_task(fallback_name.clone(), &session, app.clone()).await {
        Ok(()) => {
            info!(
                "[MIC_FALLBACK] Fallback complete: now recording via '{}'",
                fallback_name
            );
            MIC_FALLBACK_FAILED_ATTEMPTS.store(0, Ordering::SeqCst);
        }
        Err(e) => {
            error!("[MIC_FALLBACK] Fallback swap failed: {}", e);
            if !session_live(&session) {
                return;
            }
            let n = MIC_FALLBACK_FAILED_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
            if n == MAX_MIC_FALLBACK_ATTEMPTS {
                let _ = app.emit(
                    "mic-recovery-exhausted",
                    serde_json::json!({ "device_name": disconnected_name }),
                );
            }
        }
    }
}

#[cfg(test)]
mod interview_transcript_tail_tests {
    use super::*;
    fn update(session:u64,text:&str)->TranscriptUpdate {
        serde_json::from_value(serde_json::json!({"text":text,"timestamp":"00:01","source":"System","sequence_id":1,
          "chunk_start_time":0.0,"is_partial":false,"confidence":null,"audio_start_time":0.0,"audio_end_time":1.0,"duration":1.0,"native_session_id":session})).unwrap()
    }
    #[tokio::test]
    async fn final_text_reaches_original_saver_while_manager_is_out_of_global_slot() {
        let manager=RecordingManager::new();let id=manager.get_state().health.session_id;
        let buffer=manager.transcript_buffer();
        store_transcript_update(update(id,"已完成的尾段"),id,&buffer);
        store_transcript_update(update(id+1,"另一轮迟到的文字"),id,&buffer);
        store_transcript_update(update(id,"尾段修订"),id,&buffer);
        let segments=manager.get_transcript_segments();assert_eq!(segments.len(),1);assert_eq!(segments[0].text,"尾段修订");
        assert!(segments[0].confidence.is_none());
        let saved=serde_json::to_value(&segments[0]).unwrap();
        assert!(saved["confidence"].is_null(),"saved unknown confidence must not become zero/default");
        assert!(manager.get_meeting_folder().is_none());
    }
    #[tokio::test]
    async fn missing_and_legacy_confidence_do_not_block_transcript_persistence() {
        let manager=RecordingManager::new();let id=manager.get_state().health.session_id;
        let buffer=manager.transcript_buffer();
        let mut payload=serde_json::to_value(update(id,"缺少置信度的短句")).unwrap();
        payload.as_object_mut().unwrap().remove("confidence");
        store_transcript_update(serde_json::from_value(payload.clone()).unwrap(),id,&buffer);
        assert!(manager.get_transcript_segments()[0].confidence.is_none());
        payload["sequence_id"]=2.into();payload["confidence"]=serde_json::json!(0.85);
        store_transcript_update(serde_json::from_value(payload).unwrap(),id,&buffer);
        let saved=manager.get_transcript_segments();
        assert_eq!(saved.len(),2);assert_eq!(saved[1].confidence,Some(0.85));
        // Historical bytes remain readable; UI must not promote them to accuracy.
        let roundtrip:Vec<super::super::recording_saver::TranscriptSegment>=serde_json::from_value(serde_json::to_value(saved).unwrap()).unwrap();
        assert!(roundtrip[0].confidence.is_none());assert_eq!(roundtrip[1].confidence,Some(0.85));
    }
}
