mod helpers;
pub(crate) use helpers::*;
mod aoa_ops;
mod audio_ops;
mod connect_ops;
mod diagnostics_log;
mod discovery_ops;
mod monitor_ops;
mod network_ops;
mod receiver_ops;
mod teardown_ops;
mod wifi_ops;

use crate::receiver::{
    self, PlaybackDiagnostics, ReceiverStatus, StartupDiagnosticsCapture, StartupDiagnosticsExport,
};
use crate::{
    aoa_owner::AoaOwner,
    aoa_winusb::{self, AoaCancel},
    audio::{AudioActor, CaptureFormatV2},
    discovery::{self, Candidate, HostDiscoverySnapshot, HostOfferListener},
    network::{FloatPcmQueue, PcmQueue},
    network_interface::{self, Ipv4Interface},
    p5c::{
        BackgroundTrustSnapshot, HostAudioProfile, HostSession, ReceiverCapabilities,
        SessionAudioQueue, SessionLifecycleState, SessionOwner, TerminalCause,
        CAP_AOA_SESSION_RESTART, CAP_AUDIO_V2, CAP_TERMINAL_CAUSE,
    },
    presence::{PresenceTracker, PresenceView},
    protocol::{AudioFormatV2, PcmFormat},
    security::{AudioParameters, BindingStore},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    env, fs,
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::Manager;
use tauri::{AppHandle, Emitter};
use tokio::{
    process::{Child, Command},
    sync::Mutex as AsyncMutex,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

static NEXT_SESSION_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub serial: String,
    pub physical_id: Option<String>,
    pub transport: String,
    pub state: String,
    pub manufacturer: String,
    pub model: String,
    pub android_version: String,
    pub api_level: String,
    pub display_name: String,
    pub status_text: String,
    pub connectable: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AoaDeviceTarget {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AoaSwitchResult {
    pub success: bool,
    pub message: String,
    pub requires_refresh: bool,
}

const ADB_SERVER_KILL_ARGS: &[&str] = &["kill-server"];

async fn with_confirmed_adb_release<F, Fut>(confirmed: bool, run: F) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    if !confirmed {
        return Err("请确认停止本机 ADB server 后再切换 USB 模式。".into());
    }
    run().await
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub last_device_id: Option<String>,
    #[serde(default = "yes")]
    pub minimize_to_tray: bool,
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default = "usb")]
    pub preferred_transport: String,
    #[serde(default = "aoa")]
    pub wired_mode: String,
    #[serde(default)]
    pub wireless_protocol: String,
    #[serde(default = "receiver")]
    pub audio_backend: String,
    #[serde(default)]
    pub last_wireless_endpoint: Option<String>,
    /// Last Receiver control endpoint that accepted a session; probed first on the
    /// next wireless search so a known phone answers without a broadcast.
    #[serde(default)]
    pub last_receiver_endpoint: Option<String>,
    #[serde(default)]
    pub last_usb_direct_target_id: Option<String>,
    #[serde(default)]
    pub last_usb_direct_binding_reference: Option<String>,
    #[serde(default = "standard")]
    pub audio_quality: String,
    #[serde(default = "balanced")]
    pub playback_mode: String,
    #[serde(default = "system")]
    pub appearance_theme: String,
}

fn schema_version() -> u32 {
    1
}
fn yes() -> bool {
    true
}
fn usb() -> String {
    "Usb".into()
}
fn udp() -> String {
    "udp".into()
}

fn aoa() -> String {
    "aoa".into()
}
fn receiver() -> String {
    "receiver".into()
}
fn standard() -> String {
    "standard".into()
}
fn balanced() -> String {
    "Balanced".into()
}
fn system() -> String {
    "System".into()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            last_device_id: None,
            minimize_to_tray: true,
            launch_at_login: false,
            preferred_transport: usb(),
            wired_mode: "aoa".into(),
            wireless_protocol: udp(),
            audio_backend: receiver(),
            last_wireless_endpoint: None,
            last_receiver_endpoint: None,
            last_usb_direct_target_id: None,
            last_usb_direct_binding_reference: None,
            audio_quality: standard(),
            playback_mode: balanced(),
            appearance_theme: system(),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NegotiatedAudioFormat {
    pub codec_version: u8,
    pub sample_format: String,
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub valid_bits_per_sample: u8,
    pub bytes_per_sample: u8,
    pub frame_bytes: u8,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsMixFormat {
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub sample_format: Option<String>,
    pub container_bits: Option<u32>,
    pub valid_bits: Option<u32>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidOutputFormat {
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub encoding: Option<String>,
}

type SessionCancelSignal = Arc<dyn Fn() + Send + Sync>;

fn negotiated_v2_audio_format(format: AudioFormatV2) -> NegotiatedAudioFormat {
    NegotiatedAudioFormat {
        codec_version: 2,
        sample_format: match format.sample_format {
            2 => "pcm24le",
            3 => "float32le",
            _ => "pcm16le",
        }
        .into(),
        sample_rate_hz: format.sample_rate,
        channels: format.channels,
        valid_bits_per_sample: format.valid_bits,
        bytes_per_sample: format.bytes_per_sample,
        frame_bytes: format.frame_bytes as u8,
    }
}

fn android_output_format(diagnostics: Option<&PlaybackDiagnostics>) -> Option<AndroidOutputFormat> {
    let diagnostics = diagnostics?;
    let encoding = match diagnostics.output_format? {
        1 => "pcm16",
        2 => "pcm24Packed",
        3 => "float32",
        _ => return None,
    };
    Some(AndroidOutputFormat {
        sample_rate: diagnostics.output_sample_rate,
        channels: diagnostics.opened_track_channels,
        encoding: Some(encoding.into()),
    })
}

fn windows_mix_format(format: CaptureFormatV2) -> WindowsMixFormat {
    WindowsMixFormat {
        sample_rate: (format.mix_sample_rate != 0).then_some(format.mix_sample_rate),
        channels: (format.mix_channels != 0).then_some(format.mix_channels),
        sample_format: match format.mix_sample_format {
            1 => Some("integer".into()),
            2 => Some("float".into()),
            _ => None,
        },
        container_bits: (format.mix_container_bits != 0).then_some(format.mix_container_bits),
        valid_bits: (format.mix_valid_bits != 0).then_some(format.mix_valid_bits),
    }
}

fn negotiated_audio_format(
    session_active: bool,
    output: &str,
    diagnostics: Option<&PlaybackDiagnostics>,
) -> Option<NegotiatedAudioFormat> {
    if !session_active || output != "phone" {
        return None;
    }
    diagnostics
        .filter(|diagnostics| {
            diagnostics.sample_rate == Some(48_000) && diagnostics.channels == Some(2)
        })
        .map(|_| NegotiatedAudioFormat {
            codec_version: 1,
            sample_format: "pcm16le".into(),
            sample_rate_hz: 48_000,
            channels: 2,
            valid_bits_per_sample: 16,
            bytes_per_sample: 2,
            frame_bytes: 4,
        })
}

fn snapshot_negotiated_audio_format(
    session_active: bool,
    output: &str,
    diagnostics: Option<&PlaybackDiagnostics>,
    negotiated: Option<&NegotiatedAudioFormat>,
) -> Option<NegotiatedAudioFormat> {
    if !session_active || output != "phone" {
        return None;
    }
    negotiated
        .cloned()
        .or_else(|| negotiated_audio_format(true, output, diagnostics))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub state: String,
    pub message: String,
    pub device_name: Option<String>,
    pub devices: Vec<Device>,
    pub selected_serial: Option<String>,
    pub session_serial: Option<String>,
    pub session_receiver_binding: Option<String>,
    pub session_generation: Option<u64>,
    pub session_transport: Option<String>,
    pub session_connection_transport: Option<String>,
    pub session_wireless_protocol: Option<String>,
    pub session_capabilities: Option<ReceiverCapabilities>,
    pub playback_diagnostics: Option<PlaybackDiagnostics>,
    pub negotiated_audio_format: Option<NegotiatedAudioFormat>,
    pub windows_mix_format: Option<WindowsMixFormat>,
    pub android_output_format: Option<AndroidOutputFormat>,
    pub network_recovery_seconds_remaining: Option<u64>,
    pub wireless_connection_offers: HostDiscoverySnapshot,
    pub session_android_version: Option<String>,
    /// Canonical v2 choice; `transport` remains the `usb`/`wifi` frontend alias.
    pub connection_transport: String,
    pub transport: String,
    pub output: String,
    pub settings: AppSettings,
    pub busy: bool,
    pub can_disconnect: bool,
    pub can_change_modes: bool,
    pub can_change_playback_mode: bool,
    pub status_detail: String,
    pub audio_bitrate_bps: Option<u64>,
    pub presence: Vec<PresenceView>,
    pub version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDirectConfiguration {
    pub snapshot: AppSnapshot,
    pub configuration_id: u64,
}

/// Sanitised snapshot written by `export_diagnostics`.
///
/// Deliberately excludes device serials, display names, binding references,
/// network addresses and free-form messages, so the file can be shared when
/// reporting a problem. Only structured counters and negotiated capabilities
/// are kept.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsExport {
    pub schema_version: u32,
    pub exported_at: String,
    pub app_version: String,
    pub state: String,
    pub transport: String,
    pub connection_transport: String,
    pub session_transport: Option<String>,
    pub session_wireless_protocol: Option<String>,
    pub output: String,
    pub playback_mode: String,
    pub audio_quality: String,
    pub negotiated_audio_format: Option<NegotiatedAudioFormat>,
    pub windows_mix_format: Option<WindowsMixFormat>,
    pub android_output_format: Option<AndroidOutputFormat>,
    pub audio_backend: String,
    pub audio_bitrate_bps: Option<u64>,
    pub session_capabilities: Option<ReceiverCapabilities>,
    pub playback: Option<PlaybackDiagnostics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_diagnostics: Option<StartupDiagnosticsExport>,
    pub network_recovery_seconds_remaining: Option<u64>,
    pub timeline: DiagnosticsTimeline,
    /// Newest log lines; free-text details are stripped (see applog).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_log: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsTimelineSample {
    pub sampled_at_utc: String,
    pub elapsed_ms: u64,
    pub session_generation: Option<u64>,
    pub state: String,
    pub transport: String,
    pub session_transport: Option<String>,
    pub session_wireless_protocol: Option<String>,
    pub output: String,
    pub playback_mode: String,
    pub audio_quality: String,
    pub negotiated_audio_format: Option<NegotiatedAudioFormat>,
    pub windows_mix_format: Option<WindowsMixFormat>,
    pub android_output_format: Option<AndroidOutputFormat>,
    pub pc_queued_frames: Option<u64>,
    pub pc_dropped_frames: Option<u64>,
    pub drift_target_frames: Option<u32>,
    pub drift_observed_frames: Option<u32>,
    pub drift_error_frames: Option<i32>,
    pub drift_correction_ppm: Option<i32>,
    pub drift_reset_reason: Option<u8>,
    pub playback: Option<PlaybackDiagnostics>,
    /// PC capture interval, queue wait and send duration since the previous
    /// sample of this session's queue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_send_timing: Option<crate::timing::CaptureSendTimingSummary>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsTimeline {
    pub interval_ms: u32,
    pub retention_ms: u64,
    pub truncated: bool,
    pub samples_omitted: u64,
    pub samples: Vec<DiagnosticsTimelineSample>,
}

struct DiagnosticsTimelineBuffer {
    started_at: Instant,
    last_sample_at: Option<Instant>,
    /// Generation and first sample time of the current session, for the session-start file.
    session_start: Option<(u64, Instant)>,
    truncated: bool,
    samples_omitted: u64,
    samples: VecDeque<DiagnosticsTimelineSample>,
}

const DIAGNOSTICS_INTERVAL: Duration = Duration::from_secs(2);
const SESSION_START_WINDOW: Duration = Duration::from_secs(10 * 60);
const DIAGNOSTICS_RETENTION: Duration = Duration::from_secs(60 * 60);
const DIAGNOSTICS_MAX_SAMPLES: usize = 1800;
const DIAGNOSTICS_EXPORT_LOG_LINES: usize = 500;

impl DiagnosticsTimelineBuffer {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            last_sample_at: None,
            session_start: None,
            truncated: false,
            samples_omitted: 0,
            samples: VecDeque::with_capacity(DIAGNOSTICS_MAX_SAMPLES),
        }
    }

    fn is_due(&self, now: Instant) -> bool {
        self.last_sample_at
            .is_none_or(|last| now.saturating_duration_since(last) >= DIAGNOSTICS_INTERVAL)
    }

    fn push(&mut self, sample: DiagnosticsTimelineSample, now: Instant) {
        if !self.is_due(now) {
            return;
        }
        self.last_sample_at = Some(now);
        let elapsed_ms = now.saturating_duration_since(self.started_at).as_millis() as u64;
        while self.samples.front().is_some_and(|oldest| {
            elapsed_ms.saturating_sub(oldest.elapsed_ms) > DIAGNOSTICS_RETENTION.as_millis() as u64
        }) {
            self.samples.pop_front();
            self.truncated = true;
            self.samples_omitted = self.samples_omitted.saturating_add(1);
        }
        if self.samples.len() == DIAGNOSTICS_MAX_SAMPLES {
            self.samples.pop_front();
            self.truncated = true;
            self.samples_omitted = self.samples_omitted.saturating_add(1);
        }
        for event in diagnostics_log::timeline_events(self.samples.back(), &sample) {
            crate::applog::write(event.level, event.module, event.event, event.detail);
        }
        let session_opening = self.in_session_opening(sample.session_generation, now);
        if let Ok(json) = serde_json::to_string(&sample) {
            crate::applog::timeline(&json);
            if session_opening {
                crate::applog::session_start_timeline(&json);
            }
        }
        self.samples.push_back(sample);
    }

    /// True for samples in the first ten minutes of a session; a recovered session gets a new
    /// generation and therefore its own opening.
    fn in_session_opening(&mut self, generation: Option<u64>, now: Instant) -> bool {
        let Some(generation) = generation else {
            self.session_start = None;
            return false;
        };
        if self.session_start.map(|(current, _)| current) != Some(generation) {
            self.session_start = Some((generation, now));
        }
        self.session_start
            .is_some_and(|(_, at)| now.saturating_duration_since(at) < SESSION_START_WINDOW)
    }

    fn export(&self) -> DiagnosticsTimeline {
        DiagnosticsTimeline {
            interval_ms: DIAGNOSTICS_INTERVAL.as_millis() as u32,
            retention_ms: DIAGNOSTICS_RETENTION.as_millis() as u64,
            truncated: self.truncated,
            samples_omitted: self.samples_omitted,
            samples: self.samples.iter().cloned().collect(),
        }
    }
}

fn diagnostics_export(snapshot: &AppSnapshot, timeline: DiagnosticsTimeline) -> DiagnosticsExport {
    DiagnosticsExport {
        schema_version: 2,
        exported_at: iso8601_utc(unix_seconds()),
        app_version: snapshot.version.clone(),
        state: snapshot.state.clone(),
        transport: snapshot.transport.clone(),
        connection_transport: snapshot.connection_transport.clone(),
        session_transport: snapshot.session_transport.clone(),
        session_wireless_protocol: snapshot.session_wireless_protocol.clone(),
        output: snapshot.output.clone(),
        playback_mode: snapshot.settings.playback_mode.clone(),
        audio_quality: snapshot.settings.audio_quality.clone(),
        negotiated_audio_format: snapshot.negotiated_audio_format.clone(),
        windows_mix_format: snapshot.windows_mix_format.clone(),
        android_output_format: snapshot.android_output_format.clone(),
        audio_backend: snapshot.settings.audio_backend.clone(),
        audio_bitrate_bps: snapshot.audio_bitrate_bps,
        session_capabilities: snapshot.session_capabilities.clone(),
        playback: snapshot.playback_diagnostics.clone(),
        startup_diagnostics: None,
        network_recovery_seconds_remaining: snapshot.network_recovery_seconds_remaining,
        timeline,
        recent_log: Vec::new(),
    }
}

fn diagnostics_generation_is_consistent(
    active_generation: Option<u64>,
    snapshot_generation: Option<u64>,
) -> bool {
    active_generation == snapshot_generation
}

fn session_audio_queue_metrics(queue: &SessionAudioQueue) -> (Option<u64>, Option<u64>) {
    match queue {
        SessionAudioQueue::Pcm16(queue) => {
            (Some(queue.queued_frames()), Some(queue.dropped_frames()))
        }
        SessionAudioQueue::Float32(queue, _) => {
            (Some(queue.queued_frames()), Some(queue.dropped_frames()))
        }
    }
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `YYYY-MM-DDTHH:MM:SSZ`, sharing the log's calendar conversion.
fn iso8601_utc(seconds: u64) -> String {
    let millis = crate::applog::utc_timestamp(seconds.saturating_mul(1000));
    format!("{}Z", &millis[..19])
}

#[derive(Deserialize)]
#[serde(
    tag = "target",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BackgroundTrustTarget {
    ActiveSession {
        binding_reference: String,
        session_generation: u64,
    },
    Receiver {
        binding_reference: String,
        local_address: String,
        peer: String,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTrustStatus {
    pub target: String,
    pub binding_reference: String,
    pub session_generation: Option<u64>,
    pub kind: String,
    pub persistence: String,
    pub remaining_ms: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub success: bool,
    pub message: String,
    pub snapshot: AppSnapshot,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiverUpdateCandidate {
    pub version_code: u32,
    pub version_name: String,
    pub size_bytes: u64,
    pub current_version_code: Option<u32>,
    pub binding_reference: Option<String>,
}

pub(crate) struct Session {
    generation: u64,
    serial: String,
    transport: String,
    reverse: String,
    child: Option<Child>,
    installed_receiver: bool,
    receiver_owner_id: Option<String>,
    receiver_generation: Option<u64>,
    output: String,
    output_tail: Arc<Mutex<VecDeque<String>>>,
    network_owner: Option<SessionOwner>,
    aoa_owner: Option<AoaOwner>,
    adb_stream_owner: Option<AoaOwner>,
    aoa_target_id: Option<String>,
    network_sink_queue: Option<SessionAudioQueue>,
    network_binding_ref: Option<String>,
    windows_mix_format: Option<WindowsMixFormat>,
}

#[derive(Clone)]
struct PendingAdbCleanup {
    serial: String,
    transport: String,
    socket_name: String,
    installed_receiver: bool,
    receiver_owner_id: Option<String>,
    receiver_generation: Option<u64>,
    receiver_stopped: bool,
    receiver_user_stopped: bool,
    reverse_removed: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AdbCleanupOutcome {
    #[default]
    Complete,
    ReceiverUserStopped,
}

enum AdbRecoveryAttempt {
    Cancelled,
    Offline,
    ReceiverUserStopped,
    CleanupFailed(String),
    Connected,
    Failed,
}

trait AdbRecoveryAttemptOps {
    fn find_device<'a>(
        &'a self,
        serial: &'a str,
        transport: &'a str,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Vec<Device>> + Send + 'a>>;
    fn cleanup_pending<'a>(
        &'a self,
        serial: &'a str,
        transport: &'a str,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<AdbCleanupOutcome, String>> + Send + 'a>>;
    fn bootstrap<'a>(
        &'a self,
        device: Device,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>;
}

async fn run_adb_recovery_attempt(
    ops: &impl AdbRecoveryAttemptOps,
    serial: &str,
    transport: &str,
    cancel: &CancellationToken,
) -> AdbRecoveryAttempt {
    if cancel.is_cancelled() {
        return AdbRecoveryAttempt::Cancelled;
    }
    let device = ops
        .find_device(serial, transport, cancel)
        .await
        .into_iter()
        .find(|device| {
            device.serial == serial && device.transport == transport && device.connectable
        });
    let Some(device) = device else {
        return if cancel.is_cancelled() {
            AdbRecoveryAttempt::Cancelled
        } else {
            AdbRecoveryAttempt::Offline
        };
    };
    if cancel.is_cancelled() {
        return AdbRecoveryAttempt::Cancelled;
    }
    match ops.cleanup_pending(serial, transport, cancel).await {
        Ok(AdbCleanupOutcome::ReceiverUserStopped) => {
            return AdbRecoveryAttempt::ReceiverUserStopped;
        }
        Err(error) => return AdbRecoveryAttempt::CleanupFailed(error),
        Ok(AdbCleanupOutcome::Complete) => {}
    }
    if cancel.is_cancelled() {
        return AdbRecoveryAttempt::Cancelled;
    }
    if ops.bootstrap(device, cancel).await {
        AdbRecoveryAttempt::Connected
    } else if cancel.is_cancelled() {
        AdbRecoveryAttempt::Cancelled
    } else {
        AdbRecoveryAttempt::Failed
    }
}

struct AppStateAdbRecoveryAttemptOps<'a> {
    state: &'a AppState,
    app: &'a AppHandle,
    operation_generation: u64,
    was_playing: bool,
}

impl AdbRecoveryAttemptOps for AppStateAdbRecoveryAttemptOps<'_> {
    fn find_device<'a>(
        &'a self,
        serial: &'a str,
        transport: &'a str,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Vec<Device>> + Send + 'a>> {
        Box::pin(async move {
            if transport == "wifi" && serial.contains(':') {
                let _ = checked_adb(
                    &self.state.adb_path,
                    None,
                    &["connect", serial],
                    Duration::from_secs(8),
                    Some(cancel),
                )
                .await;
            }
            if cancel.is_cancelled() {
                return Vec::new();
            }
            if self.state.refresh_devices().await.is_err() {
                return Vec::new();
            }
            self.state.snapshot().devices
        })
    }

    fn cleanup_pending<'a>(
        &'a self,
        serial: &'a str,
        transport: &'a str,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<AdbCleanupOutcome, String>> + Send + 'a>> {
        Box::pin(
            self.state
                .cleanup_pending_adb_for_target(serial, transport, cancel),
        )
    }

    fn bootstrap<'a>(
        &'a self,
        device: Device,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            match self
                .state
                .connect_locked(
                    self.app,
                    &device.serial,
                    self.operation_generation,
                    cancel,
                    self.was_playing,
                )
                .await
            {
                Ok(result) if result.success => {
                    if !self.was_playing {
                        let _ = self
                            .state
                            .set_output_locked(
                                self.app,
                                "computer",
                                self.operation_generation,
                                cancel,
                            )
                            .await;
                        self.state.emit(self.app);
                    }
                    true
                }
                _ => {
                    let _ = self.state.teardown(Some(self.app)).await;
                    false
                }
            }
        })
    }
}

fn record_adb_cleanup_attempt(
    cleanup: &mut PendingAdbCleanup,
    stop: Result<(), String>,
    receiver_user_stopped: bool,
    reverse: Result<(), String>,
) -> Result<AdbCleanupOutcome, String> {
    let mut errors = Vec::new();
    if receiver_user_stopped {
        cleanup.receiver_user_stopped = true;
    }
    match stop {
        Ok(()) => cleanup.receiver_stopped = true,
        Err(error) => errors.push(error),
    }
    match reverse {
        Ok(()) => cleanup.reverse_removed = true,
        Err(error) => errors.push(error),
    }
    if errors.is_empty() {
        Ok(if cleanup.receiver_user_stopped {
            AdbCleanupOutcome::ReceiverUserStopped
        } else {
            AdbCleanupOutcome::Complete
        })
    } else {
        Err(errors.join("; "))
    }
}

fn adb_reverse_remove_result(result: AdbResult, socket_name: &str) -> Result<(), String> {
    let stderr = result.stderr.trim();
    let absent_listener = format!("adb: error: listener 'localabstract:{socket_name}' not found");
    if result.success || stderr == absent_listener {
        Ok(())
    } else {
        Err("Unable to remove the ADB audio reverse mapping.".into())
    }
}

pub(crate) struct ActiveOperation {
    generation: u64,
    token: CancellationToken,
    background: bool,
    receiver_preemptible: bool,
}

pub(crate) struct ActiveQrOperation {
    generation: u64,
    token: CancellationToken,
}

impl ViewState {
    /// `busy` only counts while connecting or reconnecting, so a terminal path that sets the
    /// state directly cannot leave the UI stuck in "processing" with every action refused.
    fn is_busy(&self) -> bool {
        self.busy && matches!(self.state.as_str(), "connecting" | "reconnecting")
    }
}

pub(crate) struct ViewState {
    state: String,
    message: String,
    device_name: Option<String>,
    devices: Vec<Device>,
    selected_serial: Option<String>,
    session_serial: Option<String>,
    session_receiver_binding: Option<String>,
    session_generation: Option<u64>,
    session_transport: Option<String>,
    session_android_version: Option<String>,
    output: String,
    settings: AppSettings,
    status_detail: String,
    busy: bool,
    session_active: bool,
    audio_bitrate_bps: Option<u64>,
    session_capabilities: Option<ReceiverCapabilities>,
    playback_diagnostics: Option<PlaybackDiagnostics>,
    negotiated_audio_format: Option<NegotiatedAudioFormat>,
    windows_mix_format: Option<WindowsMixFormat>,
    network_recovery_seconds_remaining: Option<u64>,
}

pub struct AppState {
    view: Mutex<ViewState>,
    host_offer_listener: Mutex<Option<HostOfferListener>>,
    operation: Mutex<Option<ActiveOperation>>,
    operation_generation: AtomicU64,
    usb_direct_configuration_id: AtomicU64,
    usb_direct_configuration_sequence: AtomicU64,
    recovering: AtomicBool,
    pending_adb_cleanup: Mutex<Vec<PendingAdbCleanup>>,
    qr_operation: Mutex<Option<ActiveQrOperation>>,
    qr_generation: AtomicU64,
    last_pairing_endpoint: Mutex<Option<String>>,
    gate: AsyncMutex<()>,
    session: AsyncMutex<Option<Session>>,
    session_cancel: Mutex<Option<(u64, SessionCancelSignal)>>,
    current_session_generation: AtomicU64,
    pcm_rate_sample: Mutex<Option<(u64, u64, Instant)>>,
    receiver_health_sample: Mutex<Option<(u64, u32)>>,
    diagnostics_timeline: Mutex<DiagnosticsTimelineBuffer>,
    last_binding_version: Mutex<Option<(String, u32)>>,
    startup_diagnostics: Option<StartupDiagnosticsCapture>,
    audio: AudioActor,
    data_dir: PathBuf,
    adb_path: PathBuf,
    server_path: PathBuf,
    receiver_apk_path: PathBuf,
    receiver_metadata_path: PathBuf,
    presence: Mutex<PresenceTracker>,
}

fn prepare_audio_settings(
    current: &AppSettings,
    audio_quality: &str,
    playback_mode: &str,
) -> Result<AppSettings, String> {
    validate_audio_quality(audio_quality)?;
    if !matches!(playback_mode, "Balanced" | "LowLatency") {
        return Err("未知的播放模式。".into());
    }
    let mut candidate = current.clone();
    candidate.audio_quality = audio_quality.into();
    candidate.playback_mode = playback_mode.into();
    if adb_transport_enabled(&candidate)
        && candidate.audio_backend == "legacy"
        && candidate.audio_quality != "standard"
    {
        return Err("ADB 连接仅支持标准音质。".into());
    }
    let candidate = normalize_settings(candidate);
    if candidate.audio_quality != "standard" {
        let profile = audio_quality_profile(&candidate.audio_quality).ok_or("所选音质不可用。")?;
        let mode = u8::from(candidate.playback_mode == "LowLatency");
        AudioFormatV2::for_profile(profile, mode)
            .map_err(|_| "所选音质不支持该播放模式。".to_string())?;
    }
    Ok(candidate)
}

fn merge_audio_preferences(current: &AppSettings, requested: &AppSettings) -> AppSettings {
    let mut settings = current.clone();
    settings.audio_quality = requested.audio_quality.clone();
    settings.playback_mode = requested.playback_mode.clone();
    normalize_settings(settings)
}

fn append_audio_apply_failures(
    mut message: String,
    cleanup: Result<(), String>,
    restore: Result<(), String>,
) -> String {
    if let Err(error) = cleanup {
        message.push_str(&format!("；新会话清理未完成：{error}"));
    }
    if let Err(error) = restore {
        message.push_str(&format!("；恢复原音频设置失败：{error}"));
    }
    message
}

impl AppState {
    fn install_session_cancel(
        &self,
        expected_generation: u64,
        generation: u64,
        operation_generation: u64,
        token: &CancellationToken,
        signal: SessionCancelSignal,
    ) -> bool {
        let mut slot = self
            .session_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.operation_is_current(operation_generation, token)
            || self.current_session_generation.load(Ordering::Acquire) != expected_generation
        {
            return false;
        }
        self.current_session_generation
            .store(generation, Ordering::Release);
        *slot = Some((generation, signal));
        true
    }

    fn register_session_generation(
        &self,
        expected_generation: u64,
        generation: u64,
        operation_generation: u64,
        token: &CancellationToken,
    ) -> bool {
        let _slot = self
            .session_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.operation_is_current(operation_generation, token)
            || self.current_session_generation.load(Ordering::Acquire) != expected_generation
        {
            return false;
        }
        self.current_session_generation
            .store(generation, Ordering::Release);
        true
    }

    fn clear_session_cancel(&self, generation: u64) {
        let mut slot = self
            .session_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let matches = slot
            .as_ref()
            .is_some_and(|(current, _)| *current == generation);
        if matches {
            slot.take();
        }
        let _ = self.current_session_generation.compare_exchange(
            generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn release_session_cancel_signal(&self, generation: u64) {
        let mut slot = self
            .session_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot
            .as_ref()
            .is_some_and(|(current, _)| *current == generation)
        {
            slot.take();
        }
    }

    async fn stop_owner_release_cancel_then<T, Stop, Reopen, Open>(
        &self,
        generation: u64,
        stop_and_join: Stop,
        reopen: Reopen,
    ) -> Result<T, String>
    where
        Stop: FnOnce() -> Result<(), String>,
        Reopen: FnOnce() -> Open,
        Open: std::future::Future<Output = Result<T, String>>,
    {
        let result = stop_and_join();
        self.release_session_cancel_signal(generation);
        result?;
        reopen().await
    }

    fn clear_invalidated_session_cancel(&self) {
        let mut slot = self
            .session_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.current_session_generation.load(Ordering::Acquire) == 0 {
            slot.take();
        }
    }

    fn session_generation_is_current(&self, generation: u64) -> bool {
        generation != 0 && self.current_session_generation.load(Ordering::Acquire) == generation
    }

    fn cancel_active_session(&self, force_io: bool) {
        let slot = self
            .session_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.current_session_generation.store(0, Ordering::Release);
        let signal = slot.as_ref().map(|(_, signal)| signal.clone());
        drop(slot);
        if force_io {
            if let Some(signal) = signal {
                signal();
            }
        }
    }

    pub async fn receiver_update_candidate(
        &self,
    ) -> Result<Option<ReceiverUpdateCandidate>, String> {
        let (version_code, version_name, size_bytes) = receiver::local_upgrade_candidate(
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
        )?;
        let current = {
            let session = self.session.lock().await;
            session.as_ref().and_then(|active| {
                let owner = active.network_owner.as_ref()?;
                let binding_reference = active.network_binding_ref.as_ref()?;
                let snapshot = owner.snapshot();
                if !matches!(
                    snapshot.state,
                    SessionLifecycleState::Authenticated
                        | SessionLifecycleState::TransportConnected
                        | SessionLifecycleState::ReceiverRunning
                        | SessionLifecycleState::Streaming
                        | SessionLifecycleState::Paused
                ) {
                    return None;
                }
                let capabilities = snapshot.capabilities?;
                if !capabilities.management_compatible || capabilities.version_code == 0 {
                    return None;
                }
                Some((capabilities.version_code, binding_reference.clone()))
            })
        };
        let (current_version_code, binding_reference) = current
            .map(|(version, binding)| (Some(version), Some(binding)))
            .unwrap_or((None, None));
        Ok(Some(ReceiverUpdateCandidate {
            version_code,
            version_name,
            size_bytes,
            current_version_code,
            binding_reference,
        }))
    }

    pub fn new() -> Self {
        let data_dir = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("AnPod");
        #[cfg(debug_assertions)]
        let (adb_path, server_path, receiver_apk_path, receiver_metadata_path, audio_path) = {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let deps = root.join("../deps");
            (
                deps.join("adb.exe"),
                deps.join("server"),
                deps.join("receiver.apk"),
                deps.join("receiver.json"),
                deps.join("audio_capture.dll"),
            )
        };
        #[cfg(not(debug_assertions))]
        let (adb_path, server_path, receiver_apk_path, receiver_metadata_path, audio_path) = {
            let app_dir = env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_default();
            (
                app_dir.join("adb.exe"),
                app_dir.join("server"),
                app_dir.join("receiver.apk"),
                app_dir.join("receiver.json"),
                app_dir.join("audio_capture.dll"),
            )
        };
        Self {
            view: Mutex::new(ViewState {
                state: "noDevice".into(),
                message: "正在检查设备…".into(),
                device_name: None,
                devices: Vec::new(),
                selected_serial: None,
                session_serial: None,
                session_receiver_binding: None,
                session_generation: None,
                session_transport: None,
                session_android_version: None,
                output: "computer".into(),
                settings: AppSettings::default(),
                status_detail: "连接 USB 手机或切换到 Wi-Fi 添加手机。".into(),
                busy: false,
                session_active: false,
                audio_bitrate_bps: None,
                session_capabilities: None,
                playback_diagnostics: None,
                negotiated_audio_format: None,
                windows_mix_format: None,
                network_recovery_seconds_remaining: None,
            }),
            host_offer_listener: Mutex::new(None),
            operation: Mutex::new(None),
            operation_generation: AtomicU64::new(0),
            usb_direct_configuration_id: AtomicU64::new(0),
            usb_direct_configuration_sequence: AtomicU64::new(0),
            recovering: AtomicBool::new(false),
            pending_adb_cleanup: Mutex::new(Vec::new()),
            qr_operation: Mutex::new(None),
            qr_generation: AtomicU64::new(0),
            last_pairing_endpoint: Mutex::new(None),
            gate: AsyncMutex::new(()),
            session: AsyncMutex::new(None),
            session_cancel: Mutex::new(None),
            current_session_generation: AtomicU64::new(0),
            pcm_rate_sample: Mutex::new(None),
            receiver_health_sample: Mutex::new(None),
            diagnostics_timeline: Mutex::new(DiagnosticsTimelineBuffer::new()),
            last_binding_version: Mutex::new(None),
            startup_diagnostics: (env::var_os("ANPOD_STARTUP_DIAGNOSTICS").as_deref()
                == Some(std::ffi::OsStr::new("1")))
            .then(StartupDiagnosticsCapture::new),
            audio: AudioActor::new(audio_path, data_dir.join("muted-endpoints.txt")),
            data_dir,
            adb_path,
            server_path,
            receiver_apk_path,
            receiver_metadata_path,
            presence: Mutex::new(PresenceTracker::default()),
        }
    }

    fn start_operation(&self) -> (u64, CancellationToken) {
        begin_operation_slot(&self.operation_generation, &self.operation, false)
            .expect("foreground operation always starts")
    }

    fn start_background_operation(&self) -> Option<(u64, CancellationToken)> {
        begin_operation_slot(&self.operation_generation, &self.operation, true)
    }

    fn try_start_exclusive_operation(&self) -> Option<(u64, CancellationToken)> {
        try_begin_exclusive_operation(&self.operation_generation, &self.operation)
    }

    fn start_preemptible_background_operation(&self) -> Option<(u64, CancellationToken)> {
        begin_preemptible_background_operation(&self.operation_generation, &self.operation)
    }

    fn cancel_active_operation(&self) {
        cancel_operation_slot(&self.operation_generation, &self.operation);
    }

    fn operation_is_current(&self, generation: u64, token: &CancellationToken) -> bool {
        operation_generation_matches(&self.operation_generation, generation)
            && !token.is_cancelled()
    }

    pub(crate) fn usb_direct_configuration_active(&self) -> bool {
        self.usb_direct_configuration_id.load(Ordering::Acquire) != 0
    }

    fn finish_operation(&self, generation: u64) {
        finish_operation_slot(&self.operation, generation);
    }

    pub fn initialize(&self, app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        fs::create_dir_all(self.data_dir.join("logs"))?;
        crate::applog::init(&self.data_dir.join("logs"));
        self.load_settings();
        self.recover_mute_state();
        crate::applog::info(
            "state",
            "app.start",
            format_args!("version={}", env!("CARGO_PKG_VERSION")),
        );
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                handle.state::<AppState>().probe_presence().await;
            }
        });
        Ok(())
    }

    pub fn snapshot(&self) -> AppSnapshot {
        let view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transport = view.settings.preferred_transport.to_ascii_lowercase();
        let can_disconnect = view.is_busy()
            || view.session_active
            || (view.state == "error" && view.device_name.is_some());
        let can_change_modes = !view.is_busy() && !can_disconnect;
        let can_change_playback_mode = view.session_active
            && !view.is_busy()
            && match view.session_transport.as_deref() {
                Some("usb" | "wifi" | "lan") => true,
                Some("aoa") => view
                    .session_capabilities
                    .as_ref()
                    .is_some_and(|capabilities| {
                        capabilities.capability_bits & CAP_AOA_SESSION_RESTART != 0
                    }),
                _ => false,
            };
        AppSnapshot {
            state: view.state.clone(),
            message: view.message.clone(),
            device_name: view.device_name.clone(),
            devices: view.devices.clone(),
            selected_serial: view.selected_serial.clone(),
            session_serial: view.session_serial.clone(),
            session_receiver_binding: view.session_receiver_binding.clone(),
            session_generation: view.session_generation,
            session_transport: view.session_transport.clone(),
            session_connection_transport: view
                .session_transport
                .as_deref()
                .and_then(session_connection_transport)
                .map(str::to_owned),
            session_wireless_protocol: view
                .session_transport
                .as_deref()
                .and_then(session_wireless_protocol)
                .map(str::to_owned),
            session_capabilities: view.session_capabilities.clone(),
            playback_diagnostics: view.playback_diagnostics.clone(),
            negotiated_audio_format: snapshot_negotiated_audio_format(
                view.session_active,
                &view.output,
                view.playback_diagnostics.as_ref(),
                view.negotiated_audio_format.as_ref(),
            ),
            windows_mix_format: (view.session_active && view.output == "phone")
                .then(|| view.windows_mix_format.clone())
                .flatten(),
            android_output_format: (view.session_active && view.output == "phone")
                .then(|| android_output_format(view.playback_diagnostics.as_ref()))
                .flatten(),
            network_recovery_seconds_remaining: view.network_recovery_seconds_remaining,
            wireless_connection_offers: self.wireless_connection_offers(),
            session_android_version: view.session_android_version.clone(),
            connection_transport: selected_connection_transport(&view.settings.preferred_transport)
                .into(),
            transport,
            output: view.output.clone(),
            settings: view.settings.clone(),
            busy: view.is_busy(),
            can_disconnect,
            can_change_modes,
            can_change_playback_mode,
            status_detail: view.status_detail.clone(),
            audio_bitrate_bps: view.audio_bitrate_bps,
            presence: self
                .presence
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .view(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    pub async fn sample_audio_bitrate(&self, app: &AppHandle) {
        let session = self.session.lock().await;
        let Some(active) = session.as_ref() else {
            drop(session);
            self.clear_audio_bitrate_if_session(None, app).await;
            return;
        };
        let generation = active.generation;
        let bytes = if let Some(owner) = active.network_owner.as_ref() {
            owner.pcm_payload_bytes_sent()
        } else if let Some(owner) = active.aoa_owner.as_ref() {
            owner.snapshot().pcm_payload_bytes_sent
        } else if let Some(owner) = active.adb_stream_owner.as_ref() {
            owner.snapshot().pcm_payload_bytes_sent
        } else if let Ok(bytes) = self.audio.sent_pcm_bytes() {
            bytes
        } else {
            return;
        };
        let now = Instant::now();
        let changed = {
            let mut previous = self
                .pcm_rate_sample
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            commit_pcm_bitrate_sample(
                Some(generation),
                generation,
                &mut previous,
                &mut view.audio_bitrate_bps,
                bytes,
                now,
            )
        };
        drop(session);
        if changed {
            self.emit(app);
        }
    }

    pub async fn sample_diagnostics_timeline(&self) {
        let session = self.session.lock().await;
        let snapshot = self.snapshot();
        self.remember_binding_version(&snapshot);
        let (session_generation, session_transport, session_wireless_protocol, playback, queue) =
            if let Some(active) = session.as_ref() {
                if !diagnostics_generation_is_consistent(
                    Some(active.generation),
                    snapshot.session_generation,
                ) {
                    return;
                }
                let (playback, queue) = if let Some(owner) = active.aoa_owner.as_ref() {
                    let owner = owner.snapshot();
                    (
                        owner.playback_progress,
                        active.aoa_owner.as_ref().map(AoaOwner::audio_queue),
                    )
                } else if let Some(owner) = active.adb_stream_owner.as_ref() {
                    let owner = owner.snapshot();
                    (
                        owner.playback_progress,
                        active.adb_stream_owner.as_ref().map(AoaOwner::audio_queue),
                    )
                } else if let Some(owner) = active.network_owner.as_ref() {
                    let owner_snapshot = owner.snapshot();
                    (owner_snapshot.playback_diagnostics, owner.audio_queue())
                } else {
                    (
                        snapshot.playback_diagnostics.clone(),
                        active.network_sink_queue.clone(),
                    )
                };
                (
                    Some(active.generation),
                    snapshot.session_transport.clone(),
                    snapshot.session_wireless_protocol.clone(),
                    playback,
                    queue,
                )
            } else {
                if !diagnostics_generation_is_consistent(None, snapshot.session_generation) {
                    return;
                }
                (None, None, None, None, None)
            };
        drop(session);
        let now = Instant::now();
        // Timing histograms are drained on read, so only read them for a
        // sample the timeline will keep.
        if !self
            .diagnostics_timeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_due(now)
        {
            return;
        }
        let capture_send_timing = queue
            .as_ref()
            .and_then(|queue| queue.timing().take_summary());
        let (queued_frames, dropped_frames) = queue
            .as_ref()
            .map(session_audio_queue_metrics)
            .unwrap_or((None, None));
        let sample = DiagnosticsTimelineSample {
            sampled_at_utc: iso8601_utc(unix_seconds()),
            elapsed_ms: 0,
            session_generation,
            state: snapshot.state,
            transport: snapshot.transport,
            session_transport,
            session_wireless_protocol,
            output: snapshot.output,
            playback_mode: snapshot.settings.playback_mode,
            audio_quality: snapshot.settings.audio_quality,
            negotiated_audio_format: snapshot.negotiated_audio_format,
            windows_mix_format: snapshot.windows_mix_format,
            android_output_format: snapshot.android_output_format,
            pc_queued_frames: queued_frames
                .or_else(|| playback.as_ref().and_then(|p| p.pc_queued_frames)),
            pc_dropped_frames: dropped_frames
                .or_else(|| playback.as_ref().and_then(|p| p.pc_dropped_frames)),
            drift_target_frames: playback.as_ref().and_then(|p| p.drift_target_frames),
            drift_observed_frames: playback.as_ref().and_then(|p| p.drift_observed_frames),
            drift_error_frames: playback.as_ref().and_then(|p| p.drift_error_frames),
            drift_correction_ppm: playback.as_ref().and_then(|p| p.drift_correction_ppm),
            drift_reset_reason: playback.as_ref().and_then(|p| p.drift_reset_reason),
            playback,
            capture_send_timing,
        };
        let mut timeline = self
            .diagnostics_timeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut sample = sample;
        sample.elapsed_ms = now
            .saturating_duration_since(timeline.started_at)
            .as_millis() as u64;
        timeline.push(sample, now);
    }

    fn remember_binding_version(&self, snapshot: &AppSnapshot) {
        let (Some(reference), Some(capabilities)) = (
            snapshot.session_receiver_binding.as_ref(),
            snapshot.session_capabilities.as_ref(),
        ) else {
            return;
        };
        if capabilities.version_code == 0 {
            return;
        }
        let current = Some((reference.clone(), capabilities.version_code));
        let mut last = self
            .last_binding_version
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *last == current {
            return;
        }
        let version = capabilities.version_code;
        match update_binding_profile(&self.data_dir, reference, |profile| {
            profile.version_code = Some(version)
        }) {
            Ok(()) => *last = current,
            Err(error) => crate::applog::warn("binding", "profile.save_failed", &error),
        }
    }

    async fn clear_audio_bitrate_if_session(
        &self,
        expected_session: Option<&str>,
        app: &AppHandle,
    ) {
        let session = self.session.lock().await;
        if expected_session.is_some() || session.is_some() {
            return;
        }
        *self
            .pcm_rate_sample
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let changed = {
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if view.audio_bitrate_bps.is_none() {
                false
            } else {
                view.audio_bitrate_bps = None;
                true
            }
        };
        drop(session);
        if changed {
            self.emit(app);
        }
    }

    fn emit(&self, app: &AppHandle) {
        let _ = app.emit("anpod://snapshot", self.snapshot());
    }

    async fn update_usb_playback_status(
        &self,
        generation: u64,
        serial: &str,
        phone_output: bool,
        audio_active: bool,
        diagnostics: Option<PlaybackDiagnostics>,
        format: Option<PcmFormat>,
    ) -> bool {
        let session = self.session.lock().await;
        let Some(_active) = session.as_ref().filter(|active| {
            self.session_generation_is_current(active.generation)
                && active.generation == generation
                && active.serial == serial
                && active.network_owner.is_none()
                && active.aoa_owner.is_none()
                && active.adb_stream_owner.is_none()
                && active.output == "phone"
        }) else {
            return false;
        };
        if !phone_output {
            return false;
        }
        let diagnostics = {
            let mut sample = self
                .receiver_health_sample
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let (diagnostics, next) = merge_usb_playback_diagnostics(
                diagnostics.unwrap_or_default(),
                format,
                generation,
                *sample,
            );
            *sample = next;
            diagnostics
        };
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, message, detail) = if audio_active {
            (
                "streaming",
                "正在手机播放",
                "Receiver 已确认播放引擎正在播放。",
            )
        } else {
            (
                "phoneReady",
                "手机接收已就绪",
                "手机接收端已就绪；电脑有声音发送后将开始播放。",
            )
        };
        let changed = view.state != state
            || view.message != message
            || view.status_detail != detail
            || view.playback_diagnostics.as_ref() != Some(&diagnostics);
        if view.state != state || view.message != message || view.status_detail != detail {
            view.state = state.into();
            view.message = message.into();
            view.status_detail = detail.into();
        }
        view.playback_diagnostics = Some(diagnostics);
        drop(session);
        changed
    }

    fn set_state(
        &self,
        app: &AppHandle,
        state: &str,
        message: impl Into<String>,
        device: Option<String>,
    ) {
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        view.state = state.into();
        view.message = message.into();
        view.device_name = device;
        view.busy = matches!(state, "connecting" | "reconnecting");
        drop(view);
        self.emit(app);
    }

    fn load_settings(&self) {
        let path = self.data_dir.join("settings.json");
        let settings = fs::read_to_string(path)
            .ok()
            .and_then(|json| serde_json::from_str::<AppSettings>(&json).ok())
            .filter(|s| s.schema_version == 1)
            .map(normalize_settings)
            .unwrap_or_default();
        self.view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .settings = settings;
    }

    fn remember_usb_direct_target(
        &self,
        target_id: &str,
        binding_reference: &str,
    ) -> Result<(), String> {
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut settings = view.settings.clone();
        settings.last_usb_direct_target_id = Some(target_id.into());
        settings.last_usb_direct_binding_reference = Some(binding_reference.into());
        let settings = normalize_settings(settings);
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }

    fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        let path = self.data_dir.join("settings.json");
        let temp = self.data_dir.join("settings.json.tmp");
        let bytes = serde_json::to_vec_pretty(&normalize_settings(settings.clone()))
            .map_err(|e| e.to_string())?;
        fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        atomic_replace(&temp, &path).map_err(|e| e.to_string())
    }

    fn save_audio_preferences(&self, requested: &AppSettings) -> Result<(), String> {
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let settings = merge_audio_preferences(&view.settings, requested);
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }

    pub(crate) fn log_launch_at_login_error(&self, error: &str) {
        crate::applog::warn("state", "launch_at_login.sync_failed", error);
    }

    fn recover_mute_state(&self) {
        let marker = self.data_dir.join("muted-endpoints.txt");
        if let Ok(contents) = fs::read_to_string(&marker) {
            let ids: Vec<String> = contents
                .lines()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if self.audio.restore_ids(ids.clone()).is_ok() {
                let _ = fs::remove_file(marker);
                if !ids.is_empty() {
                    crate::applog::warn("audio", "mute.recovered_after_unclean_exit", "");
                }
            }
        }
    }

    pub async fn refresh_devices(&self) -> Result<AppSnapshot, String> {
        if self.usb_direct_configuration_active() {
            return Ok(self.snapshot());
        }
        if self.session.lock().await.as_ref().is_some_and(|session| {
            session.aoa_owner.is_some() || session.adb_stream_owner.is_some()
        }) {
            return Ok(self.snapshot());
        }
        self.refresh_devices_with_cancel(None).await
    }

    pub async fn refresh_devices_from_ui(&self) -> Result<AppSnapshot, String> {
        let _gate = self.gate.lock().await;
        if self.usb_direct_configuration_active() {
            return Ok(self.snapshot());
        }
        self.refresh_devices_with_cancel(None).await
    }

    async fn refresh_devices_with_cancel(
        &self,
        cancel: Option<&CancellationToken>,
    ) -> Result<AppSnapshot, String> {
        if self.usb_direct_configuration_active() {
            return Ok(self.snapshot());
        }
        let result = adb_run(
            &self.adb_path,
            None,
            &["devices", "-l"],
            Duration::from_secs(8),
            cancel,
        )
        .await?;
        if !result.success {
            return Err("无法读取 ADB 设备列表；请检查 platform-tools。".into());
        }
        let mut devices = parse_devices(&result.stdout);
        for device in &mut devices {
            if self.usb_direct_configuration_active() {
                break;
            }
            if device.transport == "unknown"
                || device.model.is_empty()
                || device.manufacturer.is_empty()
                || device.android_version.is_empty()
            {
                let props = adb_run(
                    &self.adb_path,
                    Some(&device.serial),
                    &["shell", "getprop"],
                    Duration::from_secs(4),
                    cancel,
                )
                .await;
                if let Some(props) = props.ok().filter(|r| r.success) {
                    let map = parse_getprop(&props.stdout);
                    if device.model.is_empty() {
                        device.model = map.get("ro.product.model").cloned().unwrap_or_default();
                    }
                    if device.manufacturer.is_empty() {
                        device.manufacturer = map
                            .get("ro.product.manufacturer")
                            .cloned()
                            .unwrap_or_default();
                    }
                    if device.android_version.is_empty() {
                        device.android_version = map
                            .get("ro.build.version.release")
                            .cloned()
                            .unwrap_or_default();
                    }
                    if device.api_level.is_empty() {
                        device.api_level =
                            map.get("ro.build.version.sdk").cloned().unwrap_or_default();
                    }
                    device.physical_id =
                        map.get("ro.serialno").filter(|id| !id.is_empty()).cloned();
                    if device.transport == "unknown"
                        && map.get("sys.usb.state").is_some_and(|state| {
                            state
                                .split(',')
                                .any(|part| part.trim().eq_ignore_ascii_case("adb"))
                        })
                    {
                        device.transport = "usb".into();
                        device.connectable = device.state == "device";
                        device.status_text = if device.connectable {
                            "可连接"
                        } else {
                            device.status_text.as_str()
                        }
                        .into();
                    }
                    device.display_name = if device.model.is_empty() {
                        device.serial.clone()
                    } else if device.manufacturer.is_empty() {
                        device.model.clone()
                    } else {
                        format!("{} {}", device.manufacturer, device.model)
                    };
                }
            }
        }
        {
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let preferred = view.settings.preferred_transport.to_ascii_lowercase();
            let previous = view
                .selected_serial
                .clone()
                .or_else(|| view.settings.last_device_id.clone());
            view.devices = devices;
            view.selected_serial = choose_selected_serial(
                view.session_serial.clone(),
                previous,
                &view.devices,
                &preferred,
            );
            if view.state == "noDevice" || view.state == "ready" || view.state == "unauthorized" {
                let selected = view
                    .devices
                    .iter()
                    .find(|d| Some(&d.serial) == view.selected_serial.as_ref());
                view.state = if selected.is_some_and(|d| d.connectable) {
                    "ready".into()
                } else if view.devices.iter().any(|d| d.state == "unauthorized") {
                    "unauthorized".into()
                } else {
                    "noDevice".into()
                };
                view.message = if view.state == "ready" {
                    "准备就绪".into()
                } else if view.state == "unauthorized" {
                    "请在手机上允许 USB 调试授权".into()
                } else {
                    "尚未连接手机".into()
                };
                view.status_detail = if view.state == "ready" {
                    "已发现可连接手机。选择输出卡片即可连接。".into()
                } else {
                    "连接 USB 手机或切换到 Wi-Fi 添加手机。".into()
                };
            }
        }
        Ok(self.snapshot())
    }

    pub fn open_logs(&self) -> Result<(), String> {
        let directory = self.data_dir.join("logs");
        std::process::Command::new("explorer.exe")
            .arg(directory)
            .spawn()
            .map(|_| ())
            .map_err(|_| "无法打开日志文件夹。".into())
    }

    /// Writes a sanitised diagnostics snapshot and returns its absolute path.
    pub fn export_diagnostics(&self) -> Result<String, String> {
        let timeline = self
            .diagnostics_timeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .export();
        let mut export = diagnostics_export(&self.snapshot(), timeline);
        export.startup_diagnostics = self
            .startup_diagnostics
            .as_ref()
            .map(StartupDiagnosticsCapture::export);
        export.recent_log = crate::applog::recent_export_lines(DIAGNOSTICS_EXPORT_LOG_LINES);
        let directory = self.data_dir.join("exports");
        fs::create_dir_all(&directory).map_err(|e| format!("无法创建诊断导出目录：{e}"))?;
        let path = directory.join(format!("diagnostics-{}.json", unix_seconds()));
        let body =
            serde_json::to_vec_pretty(&export).map_err(|e| format!("无法序列化诊断数据：{e}"))?;
        fs::write(&path, body).map_err(|e| format!("无法写入诊断文件：{e}"))?;
        crate::applog::info("diagnostics", "exported", "");
        Ok(path.to_string_lossy().into_owned())
    }

    pub async fn cancel_operation(&self, app: &AppHandle) -> AppSnapshot {
        let state = self.snapshot().state;
        let force_session_io = matches!(state.as_str(), "connecting" | "reconnecting");
        self.cancel_active_operation();
        let _guard = match self.gate.try_lock() {
            Ok(guard) => {
                self.cancel_active_session(force_session_io);
                guard
            }
            Err(_) => {
                self.cancel_active_session(true);
                self.gate.lock().await
            }
        };
        self.cancel_active_operation();
        self.cancel_active_session(false);
        let cleanup = self.teardown(Some(app)).await;
        crate::applog::info(
            "session",
            "operation.cancelled",
            format_args!(
                "state={state} cleanup={}",
                if cleanup.is_ok() { "ok" } else { "failed" }
            ),
        );
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        view.state = if cleanup.is_err() {
            "error".into()
        } else if view.devices.is_empty() {
            "noDevice".into()
        } else {
            "ready".into()
        };
        view.message = cleanup.as_ref().err().map_or_else(
            || "连接已取消".into(),
            |error| format!("取消连接时清理失败：{error}"),
        );
        if cleanup.is_ok() {
            view.device_name = None;
        }
        view.output = "computer".into();
        view.busy = false;
        drop(view);
        self.emit(app);
        self.snapshot()
    }

    pub fn update_setting(&self, key: &str, value: Value) -> Result<(), String> {
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if view.is_busy() {
            return Err("正在连接期间无法更改设置。".into());
        }
        if matches!(
            key,
            "transport"
                | "preferredTransport"
                | "wiredMode"
                | "wirelessProtocol"
                | "audioBackend"
                | "playbackMode"
                | "audioQuality"
        ) && self
            .session
            .try_lock()
            .map_or(true, |session| session.is_some())
        {
            return Err("请使用连接方式或播放模式控件切换当前会话。".into());
        }
        let mut settings = view.settings.clone();
        match key {
            "minimizeToTray" => settings.minimize_to_tray = value.as_bool().ok_or("无效设置值")?,
            "launchAtLogin" => settings.launch_at_login = value.as_bool().ok_or("无效设置值")?,
            "preferredTransport" => {
                settings.preferred_transport =
                    normalize_transport(value.as_str().ok_or("无效连接方式")?).into()
            }
            "wiredMode" => {
                settings.wired_mode = match value.as_str().ok_or("无效有线模式")? {
                    value if value.eq_ignore_ascii_case("adb") => "adb".into(),
                    value if value.eq_ignore_ascii_case("aoa") => "aoa".into(),
                    _ => return Err("有线模式必须为 adb 或 aoa。".into()),
                }
            }
            "transport" => {
                settings.preferred_transport = match value.as_str().ok_or("无效连接方式")? {
                    "usb" => "Usb".into(),
                    "wireless" => "Wifi".into(),
                    _ => return Err("transport 必须为 usb 或 wireless。".into()),
                }
            }
            "wirelessProtocol" => {
                settings.wireless_protocol = match value.as_str().ok_or("无效无线协议")? {
                    value if value.eq_ignore_ascii_case("udp") => "udp".into(),
                    value if value.eq_ignore_ascii_case("adb") => "adb".into(),
                    _ => return Err("无线协议必须为 udp 或 adb。".into()),
                }
            }
            "audioBackend" => {
                settings.audio_backend = match value.as_str().ok_or("无效 Receiver 后端")? {
                    value if value.eq_ignore_ascii_case("receiver") => "receiver".into(),
                    value if value.eq_ignore_ascii_case("legacy") => "legacy".into(),
                    _ => return Err("Receiver 后端必须为 receiver 或 legacy。".into()),
                }
            }
            "audioQuality" => {
                let quality = value
                    .as_str()
                    .ok_or("无效音质设置 / Invalid audio quality")?;
                validate_audio_quality(quality)?;
                if adb_transport_enabled(&settings)
                    && settings.audio_backend == "legacy"
                    && quality != "standard"
                {
                    return Err("ADB连接仅支持标准音质。".into());
                }
                settings.audio_quality = quality.into();
            }
            "playbackMode" => {
                settings.playback_mode =
                    normalize_playback(value.as_str().ok_or("无效播放模式")?).into()
            }
            "appearanceTheme" => {
                settings.appearance_theme =
                    normalize_theme(value.as_str().ok_or("无效主题")?).into()
            }
            _ => return Err("未知设置项。".into()),
        }
        if adb_transport_enabled(&settings) && settings.audio_backend == "legacy" {
            settings.audio_quality = "standard".into();
        }
        let settings = normalize_settings(settings);
        if matches!(
            key,
            "transport" | "preferredTransport" | "wiredMode" | "wirelessProtocol"
        ) {
            self.host_offer_listener
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
        }
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../state_tests.rs"]
mod tests;
