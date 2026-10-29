mod helpers;
pub(crate) use helpers::*;
mod discovery_ops;
mod receiver_ops;

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
    truncated: bool,
    samples_omitted: u64,
    samples: VecDeque<DiagnosticsTimelineSample>,
}

const DIAGNOSTICS_INTERVAL: Duration = Duration::from_secs(2);
const DIAGNOSTICS_RETENTION: Duration = Duration::from_secs(60 * 60);
const DIAGNOSTICS_MAX_SAMPLES: usize = 1800;

impl DiagnosticsTimelineBuffer {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            last_sample_at: None,
            truncated: false,
            samples_omitted: 0,
            samples: VecDeque::with_capacity(DIAGNOSTICS_MAX_SAMPLES),
        }
    }

    fn push(&mut self, sample: DiagnosticsTimelineSample, now: Instant) {
        if self
            .last_sample_at
            .is_some_and(|last| now.saturating_duration_since(last) < DIAGNOSTICS_INTERVAL)
        {
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
        self.samples.push_back(sample);
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

/// UTC timestamp without pulling in a date library (Howard Hinnant's civil_from_days).
fn iso8601_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        (rest % 3_600) / 60,
        rest % 60
    )
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
        let mut slot = self.session_cancel.lock().expect("session cancel lock");
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
        let _slot = self.session_cancel.lock().expect("session cancel lock");
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
        let mut slot = self.session_cancel.lock().expect("session cancel lock");
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
        let mut slot = self.session_cancel.lock().expect("session cancel lock");
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
        let mut slot = self.session_cancel.lock().expect("session cancel lock");
        if self.current_session_generation.load(Ordering::Acquire) == 0 {
            slot.take();
        }
    }

    fn session_generation_is_current(&self, generation: u64) -> bool {
        generation != 0 && self.current_session_generation.load(Ordering::Acquire) == generation
    }

    fn cancel_active_session(&self, force_io: bool) {
        let slot = self.session_cancel.lock().expect("session cancel lock");
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

    #[cfg(not(windows))]
    pub async fn accept_wireless_connection_offer(
        &self,
        _app: &AppHandle,
        _offer_id: &str,
        _binding_ref: &str,
    ) -> Result<AppSnapshot, String> {
        Err("Windows 网络音频接收仅在 Windows 上可用。".into())
    }

    #[cfg(windows)]
    #[allow(clippy::too_many_arguments)]
    pub async fn start_network_audio(
        &self,
        app: &AppHandle,
        local_address: &str,
        peer: &str,
        bootstrap_serial: Option<&str>,
        receiver_address: Option<&str>,
        receiver_prefix: Option<u8>,
        selected_binding_ref: Option<&str>,
        background_authorization_required: bool,
    ) -> Result<AppSnapshot, String> {
        let result = self
            .start_network_audio_inner(
                app,
                local_address,
                peer,
                bootstrap_serial,
                receiver_address,
                receiver_prefix,
                selected_binding_ref,
                None,
                None,
                background_authorization_required,
            )
            .await;
        if result.is_ok() {
            self.remember_receiver_endpoint(peer);
            self.record_presence(peer, selected_binding_ref);
        }
        result
    }

    #[cfg(windows)]
    #[allow(clippy::too_many_arguments)]
    async fn start_network_audio_inner(
        &self,
        app: &AppHandle,
        local_address: &str,
        peer: &str,
        bootstrap_serial: Option<&str>,
        receiver_address: Option<&str>,
        receiver_prefix: Option<u8>,
        selected_binding_ref: Option<&str>,
        intent: Option<([u8; 16], bool, CancellationToken)>,
        reservation: Option<(u64, CancellationToken)>,
        background_authorization_required: bool,
    ) -> Result<AppSnapshot, String> {
        if background_authorization_required
            && (bootstrap_serial.is_some()
                || intent.is_some()
                || !selected_binding_ref.is_some_and(valid_binding_reference))
        {
            return Err("背景授权仅允许在明确选择已保存绑定、LAN 接口和 Receiver 目标时请求；USB/bootstrap 与手机 offer 使用原连接流程。".into());
        }
        use crate::security::{AudioParameters, BindingStore};
        let (generation, token) = reservation.unwrap_or_else(|| self.start_operation());
        let mut audio_initialized = false;
        let _gate = self.gate.lock().await;
        let result = async {
            token_check(&token)?;
            if intent.is_some() && self.usb_direct_configuration_active() {
                return Err("USB Direct 配置期间无线连接请求已暂停。".into());
            }
            let audio_quality = self.view.lock().expect("view lock").settings.audio_quality.clone();
            validate_audio_quality(&audio_quality)?;
            if self.session.lock().await.is_some() { return Err("AnPod 已有活动会话；请先断开后再启动网络接收。".into()); }
            {
                let mut listener = self.host_offer_listener.lock().expect("offer listener lock");
                if let Some((_, _, cancellation)) = &intent {
                    token_check(cancellation)?;
                    listener.as_mut().ok_or("无线接收已停止；旧连接请求无效。")?.attempt_cancel = Some(token.clone());
                } else { listener.take(); }
            }
            let interface = self.network_interfaces()?.into_iter().find(|item| item.address.to_string() == local_address)
                .ok_or_else(|| "所选 IPv4 接口已不可用；请重新读取接口列表。".to_string())?;
            audio_initialized = true;
            self.audio.initialize()?;
            let playback_mode = if self.view.lock().expect("view lock").settings.playback_mode.eq_ignore_ascii_case("lowlatency") { 1 } else { 0 };
            let (capture_format, profile, initial_float_queue, mut mix_format) = if audio_quality != "standard" {
                let profile_id = audio_quality_profile(&audio_quality)
                    .ok_or_else(|| "Invalid audio quality; select Standard, HiFi, or Hi-Res.".to_string())?;
                let format = AudioFormatV2::for_profile(profile_id, playback_mode)
                    .map_err(|_| "Unable to construct the authenticated Float32 format.".to_string())?;
                let native = self.audio.probe_float_format()?;
                crate::audio::validate_float_format_for_session(&native, format)?;
                let mix_format = windows_mix_format(native);
                let queue = Arc::new(FloatPcmQueue::new(playback_mode == 1));
                (
                    PcmFormat { sample_rate: native.capture_sample_rate, channels: native.capture_channels as u16 },
                    HostAudioProfile::AudioV2(format),
                    Some(queue),
                    Some(mix_format),
                )
            } else {
                (self.audio.pcm_format()?, HostAudioProfile::Standard, None, None)
            };
            let (binding_ref, binding_path, peer) = if let Some(serial) = bootstrap_serial {
                let address = receiver_address.ok_or_else(|| "USB/bootstrap 需要明确选择手机 IPv4 接口。".to_string())?;
                let prefix = receiver_prefix.ok_or_else(|| "USB/bootstrap 需要手机接口前缀长度。".to_string())?;
                let reference = receiver::binding_reference(&self.adb_path, serial, &token).await?;
                if selected_binding_ref.is_some_and(|selected| selected != reference) {
                    return Err("所选绑定与当前 USB 设备或 Android 用户不匹配。".into());
                }
                let version = receiver::network_status(&self.adb_path, serial, &self.receiver_apk_path,
                    &self.receiver_metadata_path, &token).await?;
                if version.network_audio_version != 1 {
                    return Err("RECEIVER_UPDATE_REQUIRED:Receiver 网络协议版本不兼容。".into());
                }
                let path = binding_store_path(&self.data_dir, &reference)?;
                receiver::provision(&self.adb_path, serial, &self.receiver_apk_path,
                    &self.receiver_metadata_path, &path, &token).await?;
                let status = receiver::start_network(&self.adb_path, serial, &self.receiver_apk_path,
                    &self.receiver_metadata_path, address, prefix, &token).await?;
                if status.network_audio_version != 1 || status.network_audio_address != address || status.network_audio_port == 0 {
                    return Err("Receiver 未报告本次 bootstrap 的有效监听地址和端口。".into());
                }
                if !peer.is_empty() {
                    let supplied = peer.parse::<std::net::SocketAddr>()
                        .map_err(|_| "USB/bootstrap 不接受手动猜测的 Receiver 端口。".to_string())?;
                    let actual = std::net::SocketAddr::new(status.network_audio_address.parse().map_err(|_| "Receiver 监听地址无效。".to_string())?, status.network_audio_port);
                    if supplied != actual { return Err("USB/bootstrap 端点必须来自本次 Receiver 状态查询。".into()); }
                }
                let address = status.network_audio_address.parse::<std::net::Ipv4Addr>()
                    .map_err(|_| "Receiver 监听地址不是 IPv4。".to_string())?;
                (reference, path, std::net::SocketAddr::V4(std::net::SocketAddrV4::new(address, status.network_audio_port)))
            } else {
                if receiver_address.is_some() || receiver_prefix.is_some() {
                    return Err("手机接口地址只允许与明确的 USB/bootstrap 设备一起提供。".into());
                }
                let reference = selected_binding_ref.filter(|reference| valid_binding_reference(reference))
                    .ok_or_else(|| "无 USB bootstrap 时必须明确选择已保存的 Receiver 绑定。".to_string())?.to_owned();
                let path = binding_store_path(&self.data_dir, &reference)?;
                if !path.is_file() { return Err("所选 Receiver 绑定不存在；请通过授权 USB/bootstrap 建立该绑定。".into()); }
                let peer = peer.parse::<std::net::SocketAddr>()
                    .map_err(|_| "接收端点必须是发现候选或明确的 IPv4 地址和端口。".to_string())?;
                (reference, path, peer)
            };
            let std::net::SocketAddr::V4(peer_v4) = peer else { return Err("网络接收仅支持 IPv4 端点。".into()); };
            if peer_v4.port() == 0 || !interface.contains(*peer_v4.ip()) { return Err("接收端点不在所选 IPv4 接口的本地网段内。".into()); }
            token_check(&token)?;
            let binding = BindingStore::load(&binding_path).map_err(|_| "本机 DPAPI 绑定材料不可用；请通过授权 USB/bootstrap 建立绑定。".to_string())?;
            // binding_ref is the host-side reference that names this DPAPI file, while the phone's
            // commitment is a hash of the material itself; comparing them rejected every valid
            // background start. The saved material is bound to the reference by its path, and the
            // P5-C handshake still proves the phone holds the same secret.
            let parameters = AudioParameters { sample_rate: capture_format.sample_rate, channels: capture_format.channels as u8,
                mode: if self.view.lock().expect("view lock").settings.playback_mode.eq_ignore_ascii_case("lowlatency") { 1 } else { 0 } };
            let connection_intent = intent.as_ref().map(|(nonce, required, _)| (*nonce, *required));
            let mut owner = SessionOwner::start_on_interface_with_profile(
                &interface,
                peer,
                binding,
                parameters,
                profile,
                initial_float_queue,
                token.clone(),
                connection_intent,
                background_authorization_required,
                self.startup_diagnostics.clone(),
            )?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            let queue = loop {
                token_check(&token)?;
                if let Some(queue) = owner.audio_queue() { break Some(queue); }
                let snapshot = owner.snapshot();
                if snapshot.state == SessionLifecycleState::Authenticated
                    && snapshot.capabilities.as_ref().is_some_and(|info| info.management_compatible && !info.compatible) {
                    break None;
                }
                if snapshot.state == SessionLifecycleState::Stopped || tokio::time::Instant::now() >= deadline {
                    owner.stop();
                    return Err(snapshot.last_error.unwrap_or_else(|| "手机未在认证窗口内确认接收；请在手机重新开始接收。".into()));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            };
            let management_only = queue.is_none();
            if management_only {
                self.audio.stop_and_cleanup()?;
                self.audio.restore_muted()?;
            } else {
                audio_initialized = true;
                let queue_ref = queue.as_ref().expect("audio-compatible session queue");
                let sink_result = match queue_ref {
                    SessionAudioQueue::Pcm16(queue) => self.audio.start_pcm_sink(queue.clone()).map(|()| None),
                    SessionAudioQueue::Float32(_, _) => self.start_session_audio_sink(queue_ref),
                };
                match sink_result {
                    Ok(Some(format)) => mix_format = Some(format),
                    Ok(None) => {}
                    Err(error) => {
                        let _ = self.audio.stop_and_cleanup();
                        let _ = self.audio.restore_muted();
                        owner.stop();
                        return Err(error);
                    }
                }
                if let Err(error) = self.audio.mute_active() {
                    let _ = self.audio.stop_pcm_sink();
                    let _ = self.audio.restore_muted();
                    owner.stop();
                    return Err(error);
                }
            }
            if let Err(error) = token_check(&token) {
                let _ = self.audio.stop_pcm_sink();
                let _ = self.audio.restore_muted();
                owner.stop();
                return Err(error);
            }
            let session_output = if management_only { "computer" } else { "phone" };
            if intent.is_some() {
                let mut listener = self.host_offer_listener.lock().expect("offer listener lock");
                token_check(&token)?;
                if let Some(current) = listener.as_mut() { current.attempt_cancel = None; }
                listener.take();
            }
            let session_generation = NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed);
            let owner_snapshot = owner.snapshot();
            let negotiated_format = owner_snapshot.negotiated_format.map(negotiated_v2_audio_format);
            let unsupported_v2_format = owner_snapshot
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("audio format"));
            let capabilities = owner_snapshot.capabilities;
            let cancel_signal = owner.cancel_signal();
            let failed_install_signal = cancel_signal.clone();
            *self.session.lock().await = Some(Session {
                generation: session_generation,
                serial: "network".into(), transport: "lan".into(), reverse: String::new(), child: None,
                installed_receiver: false, output: session_output.into(), output_tail: Arc::new(Mutex::new(VecDeque::new())),
                receiver_owner_id: None, receiver_generation: None,
                network_owner: Some(owner), network_sink_queue: queue, network_binding_ref: Some(binding_ref.clone()),
                aoa_owner: None,
                adb_stream_owner: None,
                aoa_target_id: None,
                windows_mix_format: mix_format.clone(),
            });
            if !self.install_session_cancel(0, session_generation, generation, &token, cancel_signal) {
                failed_install_signal();
                self.teardown(Some(app)).await?;
                return Err("无线会话已取消。".into());
            }
            {
                let mut view = self.view.lock().expect("view lock");
                view.session_active = true;
                view.session_serial = None;
                view.session_receiver_binding = Some(binding_ref);
                view.session_generation = Some(session_generation);
                view.session_transport = Some("lan".into());
                view.session_android_version = None;
                view.session_capabilities = capabilities;
                view.playback_diagnostics = None;
                view.negotiated_audio_format = negotiated_format;
                view.windows_mix_format = if management_only { None } else { mix_format.clone() };
                view.network_recovery_seconds_remaining = None;
                view.output = session_output.into();
                view.state = if management_only { "managementOnly" } else { "phoneReady" }.into();
                view.message = if unsupported_v2_format { "Receiver 不支持所选 V2 音频格式" } else if management_only { "Receiver 管理连接已认证，音频协议不兼容" } else { "手机接收已就绪" }.into();
                view.status_detail = if unsupported_v2_format { "V2 握手已认证，但 Receiver 拒绝所选音频描述符；未启动 BIND、START 或 PCM。请选择 Standard 或更新 Receiver。" }
                    else if management_only { "P5-A 与管理能力查询已认证；未启动 BIND、START 或 PCM。请通过授权 ADB 或手动安装兼容版本。" }
                    else { "P5-A 已认证；本机 PCM 已接入所选 LAN 接收会话。" }.into();
                view.audio_bitrate_bps = if management_only { None } else { Some(0) };
            }
            *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
            self.emit(app);
            Ok(self.snapshot())
        }.await;
        if result.is_err() && audio_initialized {
            let _ = self.audio.stop_pcm_sink();
            let _ = self.audio.stop_and_cleanup();
            if self.audio.restore_muted().is_err() {
                let mut view = self.view.lock().expect("app view lock");
                view.state = "error".into();
                view.message = "网络启动失败，电脑声音恢复失败".into();
                view.status_detail = "电脑声音恢复待处理；请通过托盘重试或断开连接。".into();
            }
        }
        self.finish_operation(generation);
        if result.is_err() {
            self.emit(app);
        }
        result
    }

    #[cfg(not(windows))]
    pub async fn start_network_audio(
        &self,
        _app: &AppHandle,
        _local_address: &str,
        _peer: &str,
        _bootstrap_serial: Option<&str>,
        _receiver_address: Option<&str>,
        _receiver_prefix: Option<u8>,
        _selected_binding_ref: Option<&str>,
        _background_authorization_required: bool,
    ) -> Result<AppSnapshot, String> {
        Err("Windows 网络音频接收仅在 Windows 上可用。".into())
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

    fn prepare_usb_direct_configuration_operation(&self) -> Result<(), String> {
        let mut current = self.operation.lock().expect("operation lock");
        match current.as_ref() {
            Some(operation) if !operation.background && !operation.token.is_cancelled() => {
                Err("AnPod 正在执行前台操作；请完成或取消后再配置 USB Direct。".into())
            }
            Some(operation) if operation.background => {
                operation.token.cancel();
                self.operation_generation.fetch_add(1, Ordering::AcqRel);
                *current = None;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub async fn begin_usb_direct_configuration(
        &self,
        disconnect_current: bool,
    ) -> Result<UsbDirectConfiguration, String> {
        let configuration_id = self
            .usb_direct_configuration_sequence
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
            .max(1);
        if self
            .usb_direct_configuration_id
            .compare_exchange(0, configuration_id, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("USB Direct 配置窗口已处于打开状态。".into());
        }
        let result = async {
            self.prepare_usb_direct_configuration_operation()?;
            let _gate = self.gate.lock().await;
            self.prepare_usb_direct_configuration_operation()?;
            let session_active = self.session.lock().await.is_some();
            if session_active && !disconnect_current {
                return Err("AnPod 有活动会话；请先确认断开后再配置 USB Direct。".into());
            }
            if session_active {
                let cleanup = self.teardown(None).await;
                let mut view = self.view.lock().expect("app view lock");
                view.output = "computer".into();
                view.busy = false;
                view.device_name = None;
                if let Err(error) = cleanup {
                    view.state = "error".into();
                    view.message = format!("已断开，但清理连接失败：{error}");
                    view.status_detail = "USB Direct 配置未启动；连接清理需要处理。".into();
                    return Err(format!("已断开，但清理连接失败：{error}"));
                }
                view.state = if view.devices.is_empty() {
                    "noDevice".into()
                } else {
                    "ready".into()
                };
                view.message = "已断开连接；正在配置 USB Direct".into();
                view.status_detail =
                    "当前自动连接已暂停。关闭配置窗口后，未触发 AOA 切换的情况下会恢复自动连接。"
                        .into();
            }
            let view = self.view.lock().expect("app view lock");
            if view.busy || view.session_active {
                return Err(
                    "AnPod 正在连接或已有活动会话；请先结束操作后再配置 USB Direct。".into(),
                );
            }
            drop(view);
            Ok(())
        }
        .await;
        if let Err(error) = result {
            let _ = self.usb_direct_configuration_id.compare_exchange(
                configuration_id,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            return Err(error);
        }
        Ok(UsbDirectConfiguration {
            snapshot: self.snapshot(),
            configuration_id,
        })
    }

    pub fn end_usb_direct_configuration(&self, configuration_id: u64) -> AppSnapshot {
        let _ = self.usb_direct_configuration_id.compare_exchange(
            configuration_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.snapshot()
    }

    fn start_qr_operation(&self) -> (u64, CancellationToken) {
        let mut current = self.qr_operation.lock().expect("qr operation lock");
        if let Some(previous) = current.as_ref() {
            previous.token.cancel();
        }
        let generation = self.qr_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let token = CancellationToken::new();
        *current = Some(ActiveQrOperation {
            generation,
            token: token.clone(),
        });
        (generation, token)
    }

    fn finish_qr_operation(&self, generation: u64) {
        let mut current = self.qr_operation.lock().expect("qr operation lock");
        if current
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            *current = None;
        }
    }

    pub fn initialize(&self, app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        fs::create_dir_all(self.data_dir.join("logs"))?;
        self.load_settings();
        self.recover_mute_state();
        self.log("AnPod Tauri starting");
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
        let view = self.view.lock().expect("app view lock");
        let transport = view.settings.preferred_transport.to_ascii_lowercase();
        let can_disconnect = view.busy
            || view.session_active
            || (view.state == "error" && view.device_name.is_some());
        let can_change_modes = !view.busy && !can_disconnect;
        let can_change_playback_mode = view.session_active
            && !view.busy
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
            busy: view.busy,
            can_disconnect,
            can_change_modes,
            can_change_playback_mode,
            status_detail: view.status_detail.clone(),
            audio_bitrate_bps: view.audio_bitrate_bps,
            presence: self.presence.lock().expect("presence lock").view(),
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
            let mut previous = self.pcm_rate_sample.lock().expect("PCM rate sample lock");
            let mut view = self.view.lock().expect("app view lock");
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
        let (queued_frames, dropped_frames) = queue
            .as_ref()
            .map(session_audio_queue_metrics)
            .unwrap_or((None, None));
        let now = Instant::now();
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
        };
        let mut timeline = self
            .diagnostics_timeline
            .lock()
            .expect("diagnostics timeline lock");
        let mut sample = sample;
        sample.elapsed_ms = now
            .saturating_duration_since(timeline.started_at)
            .as_millis() as u64;
        timeline.push(sample, now);
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
        *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
        let changed = {
            let mut view = self.view.lock().expect("app view lock");
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
                .expect("receiver health sample lock");
            let (diagnostics, next) = merge_usb_playback_diagnostics(
                diagnostics.unwrap_or_default(),
                format,
                generation,
                *sample,
            );
            *sample = next;
            diagnostics
        };
        let mut view = self.view.lock().expect("app view lock");
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
        let mut view = self.view.lock().expect("app view lock");
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
        self.view.lock().expect("app view lock").settings = settings;
    }

    fn remember_usb_direct_target(
        &self,
        target_id: &str,
        binding_reference: &str,
    ) -> Result<(), String> {
        let mut view = self.view.lock().expect("app view lock");
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
        let mut view = self.view.lock().expect("app view lock");
        let settings = merge_audio_preferences(&view.settings, requested);
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }

    fn log(&self, message: &str) {
        let path = self.data_dir.join("logs").join("anpod.log");
        let _ = fs::create_dir_all(path.parent().unwrap_or(&self.data_dir));
        let line = format!("{} {message}\r\n", chrono_stub_timestamp());
        use std::io::Write;
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(line.as_bytes());
        }
    }

    pub(crate) fn log_launch_at_login_error(&self, error: &str) {
        self.log(&format!("登录启动项同步失败：{error}"));
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
                    self.log("Recovered speaker mute state left by an unclean exit");
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

    pub async fn list_aoa_targets(&self) -> Result<Vec<AoaDeviceTarget>, String> {
        let targets = tokio::task::spawn_blocking(aoa_winusb::discover_targets)
            .await
            .map_err(|error| format!("USB Direct 设备枚举任务失败：{error}"))?
            .map_err(|error| format!("无法读取 USB Direct 设备：{error}"))?;
        Ok(targets
            .into_iter()
            .map(|target| AoaDeviceTarget {
                label: target.label(),
                id: target.id,
            })
            .collect())
    }

    pub async fn list_aoa_switch_targets(&self) -> Result<Vec<AoaDeviceTarget>, String> {
        let targets = tokio::task::spawn_blocking(aoa_winusb::discover_switch_targets)
            .await
            .map_err(|error| format!("AOA 切换目标枚举任务失败：{error}"))?
            .map_err(|error| format!("无法读取 iQOO 11 WinUSB 接口：{error}"))?;
        Ok(targets
            .into_iter()
            .map(|target| AoaDeviceTarget {
                label: target.label(),
                id: target.id,
            })
            .collect())
    }

    pub async fn switch_aoa_mode(
        &self,
        app: &AppHandle,
        target_id: &str,
        release_adb_server: bool,
    ) -> Result<AoaSwitchResult, String> {
        if !self.usb_direct_configuration_active() {
            return Err("USB Direct 配置窗口未处于活动状态；请重新打开后再切换。".into());
        }
        if !release_adb_server {
            return Err("请确认停止本机 ADB server 后再切换 USB 模式。".into());
        }
        let (generation, token) = self
            .try_start_exclusive_operation()
            .ok_or_else(|| "AnPod 正在处理其他操作；请稍后再切换 USB 模式。".to_string())?;
        let result: Result<AoaSwitchResult, String> = async {
            let _gate = self.gate.lock().await;
            token_check(&token)?;
            if !self.usb_direct_configuration_active() {
                return Err("USB Direct 配置窗口已关闭；请重新打开后再切换。".into());
            }
            if self.session.lock().await.is_some() {
                return Err("AnPod 有活动会话；请先断开后再切换 USB 模式。".into());
            }
            {
                let mut view = self.view.lock().expect("view lock");
                if view.busy || view.session_active {
                    return Err(
                        "AnPod 正在连接或已有活动会话；请先结束操作后再切换 USB 模式。".into(),
                    );
                }
                view.busy = true;
                view.state = "connecting".into();
                view.message = "正在停止 ADB server 并请求切换 USB 模式…".into();
                view.device_name = Some("iQOO 11 V2243A".into());
                view.status_detail = "正在停止本机 ADB server（会影响该服务上的全部调试连接），随后通过已确认的 WinUSB MI_02 接口发送 Android Open Accessory 请求。".into();
            }
            self.emit(app);

            let selected_id = target_id.to_owned();
            let fresh_targets = tokio::task::spawn_blocking(aoa_winusb::discover_switch_targets)
                .await
                .map_err(|error| format!("重新检查 iQOO 11 WinUSB 接口失败：{error}"))?
                .map_err(|error| format!("重新检查 iQOO 11 WinUSB 接口失败：{error}"))?;
            if !fresh_targets.iter().any(|target| target.id == selected_id) {
                return Err("所选 iQOO 11 WinUSB 接口已不可用或发生变化；请刷新设备列表后重试。".into());
            }

            with_confirmed_adb_release(release_adb_server, || async {
                let stopped = adb_run(
                    &self.adb_path,
                    None,
                    ADB_SERVER_KILL_ARGS,
                    Duration::from_secs(5),
                    Some(&token),
                )
                .await?;
                if stopped.success {
                    Ok(())
                } else {
                    Err(format!(
                        "无法停止本机 ADB server，未尝试打开 WinUSB 接口。请检查 ADB server 状态后重试。{}{}",
                        stopped.stderr.trim(),
                        stopped.stdout.trim()
                    ))
                }
            })
            .await?;

            let worker_token = token.clone();
            let mut worker = tokio::task::spawn_blocking(move || {
                aoa_winusb::switch_to_accessory(&selected_id, || worker_token.is_cancelled())
            });
            let switch_result = tokio::select! {
                result = &mut worker => result
                    .map_err(|error| format!("AOA 切换 worker 失败：{error}"))?
                    .map_err(|error| format!("AOA 切换失败：{error}")),
                _ = token.cancelled() => {
                    match worker.await
                        .map_err(|error| format!("取消 AOA 切换 worker 失败：{error}"))?
                    {
                        Ok(()) => Ok(()),
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                            Err("AOA 切换已取消。".into())
                        }
                        Err(error) => Err(format!("AOA 切换失败：{error}")),
                    }
                }
            };
            switch_result.map_err(|error| {
                format!(
                    "{error}。请确认该接口仍使用 WinUSB 驱动，并关闭其他正在使用此 USB 接口的工具后重试。"
                )
            })?;
            Ok(AoaSwitchResult {
                success: true,
                message: "已发送 AOA 切换请求；手机的 USB 连接将断开，等待手机重新枚举。".into(),
                requires_refresh: true,
            })
        }
        .await;
        self.finish_operation(generation);
        if operation_generation_matches(&self.operation_generation, generation) {
            let mut view = self.view.lock().expect("view lock");
            if result.is_ok() && view.message == "正在停止 ADB server 并请求切换 USB 模式…"
            {
                view.busy = false;
                view.state = "noDevice".into();
                view.message = "等待手机重新枚举".into();
                view.device_name = None;
                view.status_detail =
                    "已发送 AOA 切换请求；手机 USB 设备重新出现后可选择 USB Direct。".into();
            } else if result.is_err() && view.message == "正在停止 ADB server 并请求切换 USB 模式…"
            {
                view.busy = false;
                view.state = if view.devices.is_empty() {
                    "noDevice"
                } else {
                    "ready"
                }
                .into();
                view.message = "USB 模式切换未完成".into();
                view.device_name = None;
                view.status_detail = result.as_ref().unwrap_err().clone();
            }
        }
        self.emit(app);
        result
    }

    #[cfg(windows)]
    pub async fn start_aoa_audio(
        &self,
        app: &AppHandle,
        target_id: &str,
        binding_reference: &str,
    ) -> Result<AppSnapshot, String> {
        if !valid_binding_reference(binding_reference) {
            return Err("请选择有效的已保存 Receiver 绑定。".into());
        }
        let (generation, token) = self.start_operation();
        self.log(&format!(
            "USB Direct connect phase=waiting_for_gate generation={generation}"
        ));
        let _gate = self.gate.lock().await;
        self.log(&format!(
            "USB Direct connect phase=gate_acquired generation={generation}"
        ));
        if let Err(error) = token_check(&token) {
            self.finish_operation(generation);
            return Err(error);
        }
        if self.session.lock().await.is_some() {
            self.finish_operation(generation);
            return Err("AnPod 已有活动会话；请先断开后再启动 USB Direct。".into());
        }
        let label = match aoa_winusb::discover_targets()
            .map_err(|error| format!("无法读取 USB Direct 设备：{error}"))
        {
            Err(error) => {
                self.finish_operation(generation);
                return Err(error);
            }
            Ok(targets) => match targets.into_iter().find(|target| target.id == target_id) {
                Some(target) => target.label(),
                None => {
                    self.finish_operation(generation);
                    return Err("所选 USB Direct 设备已不可用；请重新读取设备列表。".into());
                }
            },
        };
        {
            let mut view = self.view.lock().expect("view lock");
            view.busy = true;
            view.state = "connecting".into();
            view.message = "正在连接 USB Direct Receiver…".into();
            view.device_name = Some(label.clone());
            view.output = "computer".into();
            view.session_serial = None;
            view.status_detail = "正在验证所选绑定并建立 USB Direct 会话。".into();
        }
        self.emit(app);
        let mut audio_initialized = true;
        let result = async {
            token_check(&token)?;
            let settings = self.view.lock().expect("view lock").settings.clone();
            if settings.preferred_transport != "Usb" || settings.wired_mode != "aoa" {
                return Err("请先选择 USB Direct 有线模式。".into());
            }
            validate_audio_quality(&settings.audio_quality)?;
            let mode = if settings.playback_mode.eq_ignore_ascii_case("lowlatency") {
                1
            } else {
                0
            };
            let profile = audio_quality_profile(&settings.audio_quality)
                .ok_or_else(|| "所选音质不可用。".to_string())?;
            let format = AudioFormatV2::for_profile(profile, mode)
                .map_err(|_| "无法建立所选 USB Direct 音频格式。".to_string())?;
            let (owner, queue, mix_format) = self
                .create_aoa_owner(target_id, binding_reference, format, &token)
                .await?;
            token_check(&token)?;
            let sink = match &queue {
                SessionAudioQueue::Pcm16(queue) => {
                    self.audio.start_pcm_sink(queue.clone()).map(|()| None)
                }
                SessionAudioQueue::Float32(_, _) => self.start_session_audio_sink(&queue),
            };
            let mix_format = match sink {
                Ok(Some(format)) => Some(format),
                Ok(None) => mix_format,
                Err(error) => {
                    let _ = owner.stop();
                    return Err(error);
                }
            };
            if let Err(error) = self.audio.mute_active() {
                let _ = self.audio.stop_pcm_sink();
                let _ = self.audio.restore_muted();
                let _ = owner.stop();
                audio_initialized = false;
                return Err(error);
            }
            token_check(&token)?;
            if let Err(error) = self.remember_usb_direct_target(target_id, binding_reference) {
                let _ = owner.stop();
                return Err(error);
            }
            let session_generation = NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed);
            let owner_snapshot = owner.snapshot();
            let cancel_signal = owner.cancel_signal();
            let failed_install_signal = cancel_signal.clone();
            *self.session.lock().await = Some(Session {
                generation: session_generation,
                serial: String::new(),
                transport: "aoa".into(),
                reverse: String::new(),
                child: None,
                installed_receiver: false,
                receiver_owner_id: None,
                receiver_generation: None,
                output: "phone".into(),
                output_tail: Arc::new(Mutex::new(VecDeque::new())),
                network_owner: None,
                aoa_owner: Some(owner),
                adb_stream_owner: None,
                aoa_target_id: Some(target_id.to_owned()),
                network_sink_queue: None,
                network_binding_ref: Some(binding_reference.to_owned()),
                windows_mix_format: mix_format.clone(),
            });
            if !self.install_session_cancel(
                0,
                session_generation,
                generation,
                &token,
                cancel_signal,
            ) {
                failed_install_signal();
                let _ = self.teardown(Some(app)).await;
                return Err("USB Direct 会话已取消。".into());
            }
            {
                let mut view = self.view.lock().expect("view lock");
                view.busy = false;
                view.session_active = true;
                view.session_serial = None;
                view.session_receiver_binding = Some(binding_reference.to_owned());
                view.session_generation = Some(session_generation);
                view.session_transport = Some("aoa".into());
                view.session_android_version = None;
                view.session_capabilities = Some(owner_snapshot.capabilities);
                view.playback_diagnostics = owner_snapshot.playback_progress;
                view.negotiated_audio_format = Some(negotiated_v2_audio_format(format));
                view.windows_mix_format = mix_format;
                view.network_recovery_seconds_remaining = None;
                view.output = "phone".into();
                view.state = "phoneReady".into();
                view.message = "USB Direct Receiver 已就绪".into();
                view.status_detail =
                    "已通过 USB Direct 完成 ANPB 绑定认证；音频使用独立 ANPD v2 会话。".into();
                view.audio_bitrate_bps = Some(0);
            }
            *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
            Ok(self.snapshot())
        }
        .await;
        let outcome = if result.is_ok() {
            "connected"
        } else if token.is_cancelled() || !self.operation_is_current(generation, &token) {
            "cancelled"
        } else {
            "failed"
        };
        self.log(&format!(
            "USB Direct connect operation generation={generation} outcome={outcome}"
        ));
        if result.is_err() && audio_initialized {
            let _ = self.audio.stop_pcm_sink();
            let _ = self.audio.stop_and_cleanup();
            let _ = self.audio.restore_muted();
        }
        self.finish_operation(generation);
        if result.is_err() && operation_generation_matches(&self.operation_generation, generation) {
            let mut view = self.view.lock().expect("view lock");
            view.busy = false;
            if !view.session_active {
                view.state = if view.devices.is_empty() {
                    "noDevice"
                } else {
                    "ready"
                }
                .into();
                view.message = "USB Direct 连接失败或已取消".into();
                view.status_detail = result.as_ref().err().cloned().unwrap_or_default();
                view.device_name = None;
                view.output = "computer".into();
            }
        }
        self.emit(app);
        result
    }

    #[cfg(windows)]
    async fn create_aoa_owner(
        &self,
        target_id: &str,
        binding_reference: &str,
        format: AudioFormatV2,
        token: &CancellationToken,
    ) -> Result<(AoaOwner, SessionAudioQueue, Option<WindowsMixFormat>), String> {
        let target = aoa_winusb::discover_targets()
            .map_err(|error| format!("无法读取 USB Direct 设备：{error}"))?
            .into_iter()
            .find(|target| target.id == target_id)
            .ok_or_else(|| "所选 USB Direct 设备已不可用；请重新读取设备列表。".to_string())?;
        let (queue, mix_format) = if format.profile == 1 {
            self.audio.initialize()?;
            let actual = self.audio.pcm_format()?;
            if actual
                != (PcmFormat {
                    sample_rate: format.sample_rate,
                    channels: format.channels as u16,
                })
            {
                return Err("Standard USB Direct 只支持当前 48 kHz stereo PCM16 采集格式。".into());
            }
            (
                SessionAudioQueue::Pcm16(Arc::new(PcmQueue::for_session(actual, format.mode == 1))),
                None,
            )
        } else {
            let native = self.audio.probe_float_format()?;
            crate::audio::validate_float_format_for_session(&native, format)?;
            let queue = Arc::new(FloatPcmQueue::new(format.mode == 1));
            queue.set_expected_format(format.sample_rate, 4096)?;
            (
                SessionAudioQueue::Float32(queue, format),
                Some(windows_mix_format(native)),
            )
        };
        let path = binding_store_path(&self.data_dir, binding_reference)?;
        if !path.is_file() {
            return Err("所选 Receiver 绑定不存在；请先通过配对建立该绑定。".into());
        }
        let material = BindingStore::load(&path)
            .map_err(|_| "本机 DPAPI Receiver 绑定材料不可用。".to_string())?;
        token_check(token)?;
        let cancel_slot = Arc::new(Mutex::new(None::<AoaCancel>));
        let worker_cancel_slot = cancel_slot.clone();
        let worker_token = token.clone();
        let worker_queue = queue.clone();
        let startup_diagnostics = self.startup_diagnostics.clone();
        let worker = tokio::task::spawn_blocking(move || {
            let stream = aoa_winusb::open_target(&target)
                .map_err(|error| format!("无法打开 USB Direct 设备：{error}"))?;
            let cancel = stream.cancel.clone();
            *worker_cancel_slot.lock().expect("AOA cancel slot") = Some(cancel.clone());
            if worker_token.is_cancelled() {
                cancel.cancel();
            }
            AoaOwner::connect(stream, material, format, worker_queue, startup_diagnostics)
        });
        let mut worker = worker;
        let owner = tokio::select! {
            result = &mut worker => result.map_err(|error| format!("USB Direct worker failed: {error}"))??,
            _ = token.cancelled() => {
                if let Some(cancel) = cancel_slot.lock().expect("AOA cancel slot").as_ref() {
                    cancel.cancel();
                }
                let _ = worker.await;
                return Err("USB Direct 连接已取消。".into());
            }
        };
        token_check(token)?;
        Ok((owner, queue, mix_format))
    }

    #[cfg(windows)]
    #[allow(clippy::too_many_arguments)]
    async fn restart_aoa_session_locked(
        &self,
        app: &AppHandle,
        operation_generation: u64,
        target_id: &str,
        binding_reference: &str,
        format: AudioFormatV2,
        output: &str,
        token: &CancellationToken,
    ) -> Result<(), String> {
        token_check(token)?;
        let mut session = self.session.lock().await;
        let active = session
            .as_mut()
            .filter(|session| session.aoa_owner.is_some())
            .ok_or_else(|| "USB Direct 会话已结束。".to_string())?;
        let previous_generation = active.generation;
        let old_owner = active.aoa_owner.take().expect("AOA session owner");
        let (owner, queue, mix_format) = match self
            .stop_owner_release_cancel_then(
                previous_generation,
                || old_owner.stop_for_restart(),
                || async {
                    self.audio
                        .stop_pcm_sink()
                        .and_then(|()| self.audio.restore_muted())?;
                    self.create_aoa_owner(target_id, binding_reference, format, token)
                        .await
                },
            )
            .await
        {
            Ok(connection) => connection,
            Err(error) => {
                let detail = self.clear_aoa_session_after_restart_failure(&mut session, error);
                drop(session);
                self.emit(app);
                return Err(detail);
            }
        };
        if output == "phone" {
            let sink = match &queue {
                SessionAudioQueue::Pcm16(queue) => {
                    self.audio.start_pcm_sink(queue.clone()).map(|()| None)
                }
                SessionAudioQueue::Float32(_, _) => self.start_session_audio_sink(&queue),
            };
            if let Err(error) = sink.and_then(|_| self.audio.mute_active()) {
                let _ = owner.stop();
                let detail = self.clear_aoa_session_after_restart_failure(&mut session, error);
                drop(session);
                self.emit(app);
                return Err(detail);
            }
        } else if let Err(error) = owner
            .pause()
            .and_then(|()| self.audio.stop_pcm_sink())
            .and_then(|()| self.audio.restore_muted())
        {
            let _ = owner.stop();
            let detail = self.clear_aoa_session_after_restart_failure(&mut session, error);
            drop(session);
            self.emit(app);
            return Err(detail);
        }
        if let Err(error) = token_check(token) {
            let _ = owner.stop();
            let detail = self.clear_aoa_session_after_restart_failure(&mut session, error);
            drop(session);
            self.emit(app);
            return Err(detail);
        }
        active.generation = NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed);
        let cancel_signal = owner.cancel_signal();
        active.aoa_owner = Some(owner);
        active.aoa_target_id = Some(target_id.to_owned());
        active.network_binding_ref = Some(binding_reference.to_owned());
        active.output = output.to_owned();
        active.windows_mix_format = mix_format.clone();
        let generation = active.generation;
        let failed_install_signal = cancel_signal.clone();
        if !self.install_session_cancel(
            previous_generation,
            generation,
            operation_generation,
            token,
            cancel_signal,
        ) {
            failed_install_signal();
            drop(session);
            self.clear_session_cancel(previous_generation);
            let _ = self.teardown(Some(app)).await;
            return Err("USB Direct 会话已取消。".into());
        }
        let snapshot = active.aoa_owner.as_ref().expect("new AOA owner").snapshot();
        let mut view = self.view.lock().expect("app view lock");
        view.session_generation = Some(generation);
        view.session_capabilities = Some(snapshot.capabilities);
        view.playback_diagnostics = snapshot.playback_progress;
        view.negotiated_audio_format = Some(negotiated_v2_audio_format(format));
        view.windows_mix_format = (output == "phone").then_some(mix_format).flatten();
        view.output = output.into();
        view.state = if output == "phone" {
            "phoneReady"
        } else {
            "connected"
        }
        .into();
        view.message = if output == "phone" {
            "USB Direct Receiver 已就绪".into()
        } else {
            "已连接 · 电脑播放".into()
        };
        view.status_detail = if output == "phone" {
            "USB Direct 已建立新的认证会话并应用所选播放模式。".into()
        } else {
            "USB Direct 新会话已按当前播放模式暂停；电脑声音已恢复。".into()
        };
        Ok(())
    }

    fn clear_aoa_session_after_restart_failure(
        &self,
        session: &mut Option<Session>,
        error: String,
    ) -> String {
        let retired_generation = session.as_ref().map(|active| active.generation);
        *session = None;
        if let Some(generation) = retired_generation {
            self.clear_session_cancel(generation);
        }
        let mut cleanup = Vec::new();
        if let Err(stop_error) = self.audio.stop_and_cleanup() {
            cleanup.push(stop_error);
        }
        if let Err(restore_error) = self.audio.restore_muted() {
            cleanup.push(restore_error);
        }
        let detail = if cleanup.is_empty() {
            error
        } else {
            format!("{error}；{}", cleanup.join("；"))
        };
        let mut view = self.view.lock().expect("app view lock");
        view.session_active = false;
        view.session_serial = None;
        view.session_receiver_binding = None;
        view.session_generation = None;
        view.session_transport = None;
        view.session_capabilities = None;
        view.playback_diagnostics = None;
        view.negotiated_audio_format = None;
        view.windows_mix_format = None;
        view.network_recovery_seconds_remaining = None;
        view.output = "computer".into();
        view.state = "error".into();
        view.message = "USB Direct 会话重启失败".into();
        view.status_detail = format!("本机音频和 USB Direct 会话已清理：{detail}");
        detail
    }

    #[cfg(not(windows))]
    pub async fn start_aoa_audio(
        &self,
        _app: &AppHandle,
        _target_id: &str,
        _binding_reference: &str,
    ) -> Result<AppSnapshot, String> {
        Err("USB Direct WinUSB 仅在 Windows 上可用。".into())
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
            let mut view = self.view.lock().expect("app view lock");
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

    pub async fn monitor_once(&self, app: &AppHandle) {
        if self.usb_direct_configuration_active() {
            return;
        }
        let Ok(_gate) = self.gate.try_lock() else {
            return;
        };
        if self.usb_direct_configuration_active() {
            return;
        }
        if self
            .operation
            .lock()
            .expect("operation lock")
            .as_ref()
            .is_some_and(|operation| !operation.token.is_cancelled())
        {
            return;
        }
        if self.monitor_aoa_session(app).await {
            return;
        }
        if self.monitor_network_session(app).await {
            return;
        }
        let before = self.snapshot();
        let snapshot = match self.refresh_devices().await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                self.log(&format!(
                    "ADB device scan failed during monitoring: {error}"
                ));
                None
            }
        };
        if let Some(snapshot) = &snapshot {
            let changed = before
                .devices
                .iter()
                .map(|d| (&d.serial, &d.state, &d.transport))
                .ne(snapshot
                    .devices
                    .iter()
                    .map(|d| (&d.serial, &d.state, &d.transport)))
                || before.selected_serial != snapshot.selected_serial
                || before.state != snapshot.state;
            if changed {
                self.emit(app);
            }
            if snapshot.busy {
                return;
            }
        }
        let session = self.session.lock().await.as_mut().map(|session| {
            let exited = session
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten())
                .is_some();
            (
                session.generation,
                session.serial.clone(),
                session.transport.clone(),
                session.output.clone(),
                session.output_tail.clone(),
                session.reverse.clone(),
                session.installed_receiver,
                session.receiver_owner_id.clone(),
                session.receiver_generation,
                exited,
            )
        });
        if let Some((
            session_generation,
            serial,
            transport,
            output,
            tail,
            session_key,
            installed_receiver,
            receiver_owner_id,
            receiver_generation,
            exited,
        )) = session
        {
            let device_present = snapshot.as_ref().map(|snapshot| {
                snapshot
                    .devices
                    .iter()
                    .any(|device| device.serial == serial && device.state == "device")
            });
            let connected = self.audio.is_connected().unwrap_or(false);
            let capture_error = self.audio.take_last_error().ok().flatten();
            let mut receiver_status = None;
            let mut receiver_failure = None;
            if installed_receiver && device_present != Some(false) {
                let Some((generation, token)) = self.start_preemptible_background_operation()
                else {
                    return;
                };
                let result = receiver::health(
                    &self.adb_path,
                    &serial,
                    &self.receiver_apk_path,
                    &self.receiver_metadata_path,
                    &token,
                )
                .await;
                let cancelled = !self.operation_is_current(generation, &token);
                self.finish_operation(generation);
                if cancelled || !self.operation_is_current(generation, &token) {
                    return;
                }
                if result.as_ref().is_ok_and(|status| {
                    status
                        .was_stopped_by_user_for(receiver_owner_id.as_deref(), receiver_generation)
                }) {
                    let device_name = self.snapshot().device_name;
                    if let Err(error) = self.teardown(Some(app)).await {
                        self.set_state(
                            app,
                            "error",
                            format!("手机已主动停止；清理连接失败：{error}"),
                            device_name,
                        );
                    } else {
                        self.set_state(app, "ready", "手机已主动停止音频会话", device_name);
                    }
                    self.emit(app);
                    return;
                }
                match result {
                    Ok(status) => {
                        receiver_failure = status.monitoring_error();
                        receiver_status = Some(status);
                    }
                    Err(error) => receiver_failure = Some(error),
                }
            }
            let session_lost = session_is_lost(device_present, exited, connected);
            if session_lost || capture_error.is_some() || receiver_failure.is_some() {
                let detail = if let Some(error) = capture_error {
                    error
                } else if let Some(error) = receiver_failure {
                    format!("Receiver 健康检查失败：{error}")
                } else if exited {
                    let output = safe_android_tail(
                        &tail
                            .lock()
                            .expect("output log lock")
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>(),
                    );
                    if output.is_empty() {
                        "手机端音频服务已停止".to_string()
                    } else {
                        format!("手机端音频服务已停止：\n{output}")
                    }
                } else if device_present == Some(false) {
                    "手机连接已断开".into()
                } else {
                    "手机端音频连接已中断".into()
                };
                self.log(&detail);
                if installed_receiver
                    && (receiver_owner_id.is_none() || receiver_generation.is_none())
                {
                    let _ = self.teardown(Some(app)).await;
                    let device_name = self.snapshot().device_name;
                    self.set_state(
                        app,
                        "error",
                        format!(
                            "{detail}；Receiver 缺少会话身份信息，请更新 Receiver 或手动重新连接。"
                        ),
                        device_name,
                    );
                    self.emit(app);
                    return;
                }
                self.recover_session_locked(
                    app,
                    session_generation,
                    serial,
                    transport,
                    output == "phone",
                    session_key,
                    detail,
                )
                .await;
                return;
            }
            if let Some(status) = receiver_status {
                let format = self.audio.pcm_format().ok();
                if self
                    .update_usb_playback_status(
                        session_generation,
                        &serial,
                        output == "phone",
                        status.audio_active,
                        status.playback_diagnostics,
                        format,
                    )
                    .await
                {
                    self.emit(app);
                }
            }
        }

        // Discovery updates the UI; it never establishes a connection.
    }

    fn start_session_audio_sink(
        &self,
        queue: &SessionAudioQueue,
    ) -> Result<Option<WindowsMixFormat>, String> {
        match queue {
            SessionAudioQueue::Pcm16(queue) => {
                self.audio.start_pcm_sink(queue.clone()).map(|()| None)
            }
            SessionAudioQueue::Float32(queue, expected) => {
                let format = self.audio.start_float_sink(queue.clone(), *expected)?;
                Ok(Some(windows_mix_format(format)))
            }
        }
    }

    async fn monitor_network_session(&self, app: &AppHandle) -> bool {
        let mut session = self.session.lock().await;
        let Some(active) = session
            .as_mut()
            .filter(|session| session.network_owner.is_some())
        else {
            return false;
        };
        let owner = active
            .network_owner
            .as_ref()
            .expect("network session owner");
        let snapshot = owner.snapshot();
        let needs_bootstrap = snapshot.state == SessionLifecycleState::Stopped;
        let management_only = snapshot.state == SessionLifecycleState::Authenticated
            && snapshot
                .capabilities
                .as_ref()
                .is_some_and(|info| info.management_compatible && !info.compatible);
        let queue = owner.audio_queue();
        let mut failure = self.audio.take_last_error().ok().flatten();
        if failure.is_none() && needs_bootstrap {
            failure = Some(
                snapshot
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "Receiver session stopped".into()),
            );
        }
        if failure.is_none() {
            if management_only {
                active.network_sink_queue = None;
                if let Err(error) = self.audio.stop_pcm_sink() {
                    failure = Some(error);
                }
                if let Err(error) = self.audio.restore_muted() {
                    failure.get_or_insert(error);
                }
            } else if active.output == "phone" {
                if let Some(queue) = queue.as_ref() {
                    if session_audio_queue_identity_changed(
                        active.network_sink_queue.as_ref(),
                        Some(queue),
                    ) {
                        match self.start_session_audio_sink(queue) {
                            Err(error) => failure = Some(error),
                            Ok(mix_format) => {
                                if let Err(error) = self.audio.mute_active() {
                                    let _ = self.audio.stop_pcm_sink();
                                    let _ = self.audio.restore_muted();
                                    failure = Some(error);
                                } else {
                                    active.network_sink_queue = Some(queue.clone());
                                    active.windows_mix_format = mix_format;
                                }
                            }
                        }
                    }
                } else if active.network_sink_queue.take().is_some() {
                    active.windows_mix_format = None;
                    if let Err(error) = self.audio.stop_pcm_sink() {
                        failure = Some(error);
                    }
                    if let Err(error) = self.audio.restore_muted() {
                        failure = Some(error);
                    }
                }
            } else if active.network_sink_queue.take().is_some() {
                if let Err(error) = self.audio.stop_pcm_sink() {
                    failure = Some(error);
                }
                if let Err(error) = self.audio.restore_muted() {
                    failure = Some(error);
                }
            }
        }
        if let Some(error) = failure {
            let mut cleanup_errors = Vec::new();
            if let Err(stop_error) = self.audio.stop_pcm_sink() {
                cleanup_errors.push(stop_error);
            }
            if let Err(restore_error) = self.audio.restore_muted() {
                cleanup_errors.push(restore_error);
            }
            if let Some(mut owner) = active.network_owner.take() {
                owner.stop();
            }
            active.network_sink_queue = None;
            active.output = "computer".into();
            let mut detail = if cleanup_errors.is_empty() {
                error
            } else {
                format!("{error}；{}", cleanup_errors.join("；"))
            };
            if needs_bootstrap {
                detail.push_str("；请重新通过授权 USB/bootstrap 启动 Receiver");
            }
            let retired_generation = active.generation;
            *session = None;
            self.clear_session_cancel(retired_generation);
            {
                let mut view = self.view.lock().expect("app view lock");
                view.session_active = false;
                view.session_serial = None;
                view.session_receiver_binding = None;
                view.session_generation = None;
                view.session_transport = None;
                view.session_capabilities = None;
                view.playback_diagnostics = None;
                view.negotiated_audio_format = None;
                view.windows_mix_format = None;
                view.network_recovery_seconds_remaining = None;
                view.session_android_version = None;
                view.output = "computer".into();
                view.state = "error".into();
                view.message = "LAN 音频会话已停止".into();
                view.status_detail = format!("本机音频已清理；{detail}");
                view.audio_bitrate_bps = None;
            }
            *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
            drop(session);
            self.emit(app);
            return true;
        }
        active.output = monitored_network_output(&active.output, management_only).into();
        let state = if management_only {
            "managementOnly"
        } else if snapshot.state == SessionLifecycleState::Paused {
            "paused"
        } else if active.output == "computer" {
            "connected"
        } else if queue.is_none() || snapshot.state == SessionLifecycleState::Reconnecting {
            "reconnecting"
        } else {
            match snapshot.state {
                SessionLifecycleState::Streaming => "streaming",
                SessionLifecycleState::Paused => "paused",
                _ => "phoneReady",
            }
        };
        let (message, detail) = if let Some(error) = failure {
            (
                "网络音频切换失败".to_string(),
                format!("本机 PCM sink 操作失败：{error}"),
            )
        } else if state == "reconnecting" {
            (
                "正在重新连接手机".into(),
                "网络接收暂时中断；电脑声音已恢复，正在尝试重新认证。".into(),
            )
        } else if management_only {
            ("Receiver 管理连接已认证，音频协议不兼容".into(), "已认证管理查询保持在线；未启动 BIND、START 或 PCM。请通过授权 ADB 或手动安装兼容版本。".into())
        } else if active.output == "computer" {
            (
                "已连接 · 电脑播放".into(),
                "网络会话保持暂停；电脑声音已恢复到本机扬声器。".into(),
            )
        } else if state == "streaming" {
            (
                "正在手机播放".into(),
                "Receiver 已确认播放引擎收到 PCM 音频。".into(),
            )
        } else if state == "paused" {
            ("手机播放已暂停".into(), "P5-C 会话已暂停。".into())
        } else {
            (
                "手机接收已就绪".into(),
                "P5-A 已认证；等待接收端播放引擎收到 PCM 音频。".into(),
            )
        };
        let changed = {
            let mut view = self.view.lock().expect("app view lock");
            let changed = view.state != state
                || view.message != message
                || view.status_detail != detail
                || view.playback_diagnostics != snapshot.playback_diagnostics
                || view.network_recovery_seconds_remaining != snapshot.recovery_seconds_remaining;
            view.state = state.into();
            view.message = message;
            view.status_detail = detail;
            view.output = active.output.clone();
            view.session_receiver_binding = active.network_binding_ref.clone();
            view.session_capabilities = snapshot.capabilities.clone();
            view.playback_diagnostics = snapshot.playback_diagnostics.clone();
            view.negotiated_audio_format =
                snapshot.negotiated_format.map(negotiated_v2_audio_format);
            if active.output == "computer" || management_only {
                view.windows_mix_format = None;
            } else {
                view.windows_mix_format = active.windows_mix_format.clone();
            }
            view.network_recovery_seconds_remaining = snapshot.recovery_seconds_remaining;
            changed
        };
        drop(session);
        if changed {
            self.emit(app);
        }
        true
    }

    async fn monitor_aoa_session(&self, app: &AppHandle) -> bool {
        let mut session = self.session.lock().await;
        let Some(active) = session.as_mut().filter(|session| {
            self.session_generation_is_current(session.generation)
                && (session.aoa_owner.is_some() || session.adb_stream_owner.is_some())
        }) else {
            return false;
        };
        let adb_stream = active.adb_stream_owner.is_some();
        let snapshot = active
            .aoa_owner
            .as_ref()
            .or(active.adb_stream_owner.as_ref())
            .expect("record stream owner")
            .snapshot();
        if !self.session_generation_is_current(active.generation) {
            return false;
        }
        let failure = self.audio.take_last_error().ok().flatten().or_else(|| {
            (snapshot.state == crate::p5c::ReceiverState::Stopped).then(|| {
                snapshot
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "AOA Receiver session stopped".into())
            })
        });
        match snapshot.terminal_cause {
            Some(TerminalCause::ReceiverUserStop) => {
                let generation = active.generation;
                drop(session);
                self.finish_aoa_terminal(generation, app, "手机已主动停止音频会话")
                    .await;
                return true;
            }
            Some(TerminalCause::TransportFailure) => {
                let generation = active.generation;
                if snapshot.capabilities.capability_bits & CAP_TERMINAL_CAUSE == 0 {
                    drop(session);
                    self.finish_aoa_terminal(
                        generation,
                        app,
                        "手机 Receiver 无法确认会话终止原因，请更新 Receiver 或手动重新连接。",
                    )
                    .await;
                    return true;
                }
                let target_id = active.aoa_target_id.clone();
                let binding = active.network_binding_ref.clone();
                let format = snapshot.format;
                let output = active.output.clone();
                let serial = active.serial.clone();
                let transport = active.transport.clone();
                let session_key = active.reverse.clone();
                let reason = snapshot
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "USB Direct 连接已中断".into());
                drop(session);
                if adb_stream {
                    self.recover_session_locked(
                        app,
                        generation,
                        serial,
                        transport,
                        output == "phone",
                        session_key,
                        reason,
                    )
                    .await;
                } else if let (Some(target_id), Some(binding)) = (target_id, binding) {
                    self.recover_aoa_session_locked(
                        app, generation, target_id, binding, format, output, reason,
                    )
                    .await;
                } else {
                    self.finish_aoa_terminal(
                        generation,
                        app,
                        "AOA 会话缺少恢复目标或 Receiver 绑定，请手动重新连接。",
                    )
                    .await;
                }
                return true;
            }
            _ => {}
        }
        if let Some(error) = failure {
            if adb_stream {
                drop(session);
                let cleanup = self.teardown(Some(app)).await;
                let detail = cleanup.map_or_else(
                    |cleanup_error| format!("{error}；清理连接失败：{cleanup_error}"),
                    |_| error.clone(),
                );
                self.set_state(app, "error", &detail, self.snapshot().device_name);
                self.emit(app);
                *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
                return true;
            }
            let _ = self.audio.stop_and_cleanup();
            let _ = self.audio.restore_muted();
            if let Some(owner) = active
                .aoa_owner
                .take()
                .or_else(|| active.adb_stream_owner.take())
            {
                let _ = owner.stop();
            }
            let retired_generation = active.generation;
            *session = None;
            self.clear_session_cancel(retired_generation);
            let mut view = self.view.lock().expect("view lock");
            view.session_active = false;
            view.session_serial = None;
            view.session_receiver_binding = None;
            view.session_generation = None;
            view.session_transport = None;
            view.session_capabilities = None;
            view.playback_diagnostics = None;
            view.negotiated_audio_format = None;
            view.windows_mix_format = None;
            view.output = "computer".into();
            view.state = "error".into();
            view.message = if adb_stream {
                "ADB 音频会话已停止"
            } else {
                "USB Direct 音频会话已停止"
            }
            .into();
            view.status_detail = format!("本机采集已清理；{error}");
            view.audio_bitrate_bps = None;
            drop(view);
            drop(session);
            self.emit(app);
            *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
            return true;
        }
        let next_state = match snapshot.state {
            crate::p5c::ReceiverState::Streaming => "streaming",
            crate::p5c::ReceiverState::Paused => "paused",
            crate::p5c::ReceiverState::ReceiverRunning => "phoneReady",
            _ => "connected",
        };
        let changed = {
            let mut view = self.view.lock().expect("view lock");
            let changed =
                view.state != next_state || view.playback_diagnostics != snapshot.playback_progress;
            view.state = next_state.into();
            let transport_label = if adb_stream { "ADB" } else { "USB Direct" };
            view.message = match next_state {
                "streaming" if adb_stream => "正在通过 ADB 播放".into(),
                "paused" if adb_stream => "ADB 播放已暂停".into(),
                "streaming" => "正在通过 USB Direct 播放".into(),
                "paused" => "USB Direct 播放已暂停".into(),
                _ if adb_stream => "ADB Receiver 已就绪".into(),
                _ => "USB Direct Receiver 已就绪".into(),
            };
            view.status_detail = snapshot
                .last_error
                .clone()
                .unwrap_or_else(|| format!("{transport_label} 已认证；音频通过 ANPD v2 流传输。"));
            view.session_capabilities = Some(snapshot.capabilities.clone());
            view.playback_diagnostics = snapshot.playback_progress.clone();
            view.negotiated_audio_format = Some(negotiated_v2_audio_format(snapshot.format));
            view.windows_mix_format = if active.output == "phone" {
                active.windows_mix_format.clone()
            } else {
                None
            };
            changed
        };
        drop(session);
        if changed {
            self.emit(app);
        }
        true
    }

    async fn finish_aoa_terminal(&self, generation: u64, app: &AppHandle, message: &str) {
        let mut session = self.session.lock().await;
        if !session
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            return;
        }
        if session
            .as_ref()
            .is_some_and(|active| active.adb_stream_owner.is_some())
        {
            drop(session);
            let cleanup = self.teardown(Some(app)).await;
            self.set_state(
                app,
                if cleanup.is_ok() { "ready" } else { "error" },
                message,
                None,
            );
            if let Err(error) = cleanup {
                self.view.lock().expect("app view lock").status_detail = error;
            }
            self.emit(app);
            return;
        }
        let mut cleanup = Vec::new();
        if let Err(error) = self.audio.stop_and_cleanup() {
            cleanup.push(error);
        }
        if let Err(error) = self.audio.restore_muted() {
            cleanup.push(error);
        }
        if let Some(owner) = session.as_mut().and_then(|active| {
            active
                .aoa_owner
                .take()
                .or_else(|| active.adb_stream_owner.take())
        }) {
            if let Err(error) = owner.stop() {
                cleanup.push(error);
            }
        }
        *session = None;
        self.clear_session_cancel(generation);
        let detail = if cleanup.is_empty() {
            message.to_string()
        } else {
            format!("{message}；清理失败：{}", cleanup.join("；"))
        };
        let mut view = self.view.lock().expect("view lock");
        view.session_active = false;
        view.session_serial = None;
        view.session_receiver_binding = None;
        view.session_generation = None;
        view.session_transport = None;
        view.session_capabilities = None;
        view.playback_diagnostics = None;
        view.negotiated_audio_format = None;
        view.windows_mix_format = None;
        view.output = "computer".into();
        view.state = if cleanup.is_empty() { "ready" } else { "error" }.into();
        view.message = detail.clone();
        view.status_detail = detail;
        view.audio_bitrate_bps = None;
        drop(view);
        drop(session);
        *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
        self.emit(app);
    }

    #[cfg(windows)]
    // Recovery inputs must stay frozen to the generation observed by the caller.
    #[allow(clippy::too_many_arguments)]
    async fn recover_aoa_session_locked(
        &self,
        app: &AppHandle,
        session_generation: u64,
        target_id: String,
        binding: String,
        format: AudioFormatV2,
        output: String,
        reason: String,
    ) {
        if !self.session_generation_is_current(session_generation) {
            return;
        }
        if self.recovering.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some((operation_generation, token)) = self.start_preemptible_background_operation()
        else {
            self.recovering.store(false, Ordering::Release);
            return;
        };
        if !self.session_generation_is_current(session_generation)
            || !self.operation_is_current(operation_generation, &token)
        {
            self.finish_operation(operation_generation);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        let mut session = self.session.lock().await;
        if !session.as_ref().is_some_and(|active| {
            active.generation == session_generation
                && self.session_generation_is_current(active.generation)
        }) {
            self.finish_operation(operation_generation);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        if let Some(owner) = session.as_mut().and_then(|active| active.aoa_owner.take()) {
            // The owner already recorded TransportFailure; joining it must not
            // turn that original link failure into a new cleanup failure.
            let _ = self
                .stop_owner_release_cancel_then(
                    session_generation,
                    || owner.stop(),
                    || async { Ok(()) },
                )
                .await;
        }
        let cleanup_result = self
            .audio
            .stop_and_cleanup()
            .and_then(|()| self.audio.restore_muted());
        if let Err(error) = cleanup_result {
            drop(session);
            if !token.is_cancelled() {
                self.finish_aoa_terminal(
                    session_generation,
                    app,
                    &format!("{reason}；本机音频清理失败：{error}"),
                )
                .await;
            }
            self.finish_operation(operation_generation);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        if token.is_cancelled() {
            self.finish_operation(operation_generation);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        drop(session);
        for attempt in 0..3 {
            if token.is_cancelled() || !self.session_generation_is_current(session_generation) {
                break;
            }
            self.set_state(
                app,
                "reconnecting",
                format!(
                    "{reason}，正在恢复 USB Direct 会话（{}/{})…",
                    attempt + 1,
                    3
                ),
                self.snapshot().device_name,
            );
            tokio::select! {
                _ = token.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis([1000, 2000, 5000][attempt])) => {}
            }
            if token.is_cancelled()
                || !self.session_generation_is_current(session_generation)
                || !operation_generation_matches(&self.operation_generation, operation_generation)
                || !self
                    .session
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|active| active.generation == session_generation)
            {
                break;
            }
            let Ok((owner, queue, mix_format)) = self
                .create_aoa_owner(&target_id, &binding, format, &token)
                .await
            else {
                continue;
            };
            let playback_result = if output == "phone" {
                self.start_session_audio_sink(&queue)
                    .and_then(|_| self.audio.mute_active())
            } else {
                owner.pause().and_then(|()| {
                    self.audio.stop_and_cleanup()?;
                    self.audio.restore_muted()
                })
            };
            if let Err(error) = playback_result {
                let _ = owner.stop();
                let _ = self.audio.stop_and_cleanup();
                let _ = self.audio.restore_muted();
                self.log(&format!("AOA recovery output setup failed: {error}"));
                continue;
            }
            if token.is_cancelled()
                || !self.session_generation_is_current(session_generation)
                || !operation_generation_matches(&self.operation_generation, operation_generation)
            {
                (owner.cancel_signal())();
                let _ = owner.stop();
                break;
            }
            let mut session = self.session.lock().await;
            let Some(active) = session.as_mut().filter(|active| {
                active.generation == session_generation
                    && self.session_generation_is_current(active.generation)
            }) else {
                (owner.cancel_signal())();
                let _ = owner.stop();
                break;
            };
            let cancel_signal = owner.cancel_signal();
            let recovered_generation = NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed);
            if !self.install_session_cancel(
                session_generation,
                recovered_generation,
                operation_generation,
                &token,
                cancel_signal.clone(),
            ) {
                cancel_signal();
                let _ = owner.stop();
                break;
            }
            if token.is_cancelled() || !self.session_generation_is_current(recovered_generation) {
                cancel_signal();
                let _ = owner.stop();
                self.clear_session_cancel(recovered_generation);
                break;
            }
            active.generation = recovered_generation;
            active.aoa_owner = Some(owner);
            active.aoa_target_id = Some(target_id);
            active.network_binding_ref = Some(binding);
            active.output = output.clone();
            active.windows_mix_format = mix_format.clone();
            let snapshot = active
                .aoa_owner
                .as_ref()
                .expect("recovered AOA owner")
                .snapshot();
            let mut view = self.view.lock().expect("view lock");
            view.session_generation = Some(active.generation);
            view.session_capabilities = Some(snapshot.capabilities);
            view.playback_diagnostics = snapshot.playback_progress;
            view.negotiated_audio_format = Some(negotiated_v2_audio_format(format));
            view.windows_mix_format = (output == "phone").then_some(mix_format).flatten();
            view.output = output.clone();
            view.state = if output == "phone" {
                "phoneReady"
            } else {
                "connected"
            }
            .into();
            view.message = if output == "phone" {
                "USB Direct Receiver 已恢复"
            } else {
                "已连接 · 电脑播放"
            }
            .into();
            view.status_detail = "USB Direct 已在同一目标和格式上重新认证。".into();
            drop(view);
            drop(session);
            self.finish_operation(operation_generation);
            self.emit(app);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        self.finish_operation(operation_generation);
        if !token.is_cancelled() {
            self.finish_aoa_terminal(
                session_generation,
                app,
                "USB Direct 恢复失败，请手动重新连接。",
            )
            .await;
        }
        self.recovering.store(false, Ordering::Release);
    }

    // Recovery inputs must stay frozen to the generation observed by the caller.
    #[allow(clippy::too_many_arguments)]
    async fn recover_session_locked(
        &self,
        app: &AppHandle,
        session_generation: u64,
        serial: String,
        transport: String,
        was_playing: bool,
        session_key: String,
        reason: String,
    ) {
        if !self.session_generation_is_current(session_generation) {
            return;
        }
        if self.recovering.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some((generation, token)) = self.start_preemptible_background_operation() else {
            self.recovering.store(false, Ordering::Release);
            return;
        };
        if !self.session_generation_is_current(session_generation)
            || !self.operation_is_current(generation, &token)
        {
            self.finish_operation(generation);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        let (same_session, adb_stream) = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|session| {
                (
                    session.generation == session_generation
                        && self.session_generation_is_current(session.generation)
                        && session.serial == serial
                        && session.reverse == session_key,
                    session.adb_stream_owner.is_some(),
                )
            })
            .unwrap_or((false, false));
        if !same_session {
            self.finish_operation(generation);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        // monitor_once holds gate and already confirmed session loss. Keep the
        // old remote owner pending until the retry loop has freshly confirmed
        // this same target and transport online.
        let device_name = self.snapshot().device_name;
        let teardown_result = if adb_stream {
            self.teardown_adb_for_recovery(app, session_generation, &session_key)
                .await
        } else {
            self.teardown(Some(app))
                .await
                .map(|()| AdbCleanupOutcome::Complete)
        };
        let cleanup_outcome = match teardown_result {
            Ok(outcome) => outcome,
            Err(error) => {
                if !token.is_cancelled() {
                    self.set_state(
                        app,
                        "error",
                        format!("{reason}；清理连接失败：{error}"),
                        device_name,
                    );
                }
                self.finish_operation(generation);
                self.recovering.store(false, Ordering::Release);
                return;
            }
        };
        if !self.operation_is_current(generation, &token) {
            self.finish_operation(generation);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        if cleanup_outcome == AdbCleanupOutcome::ReceiverUserStopped {
            self.finish_operation(generation);
            self.set_state(app, "ready", "手机已主动停止音频会话", None);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        let mut last_cleanup_error = None;
        for attempt in 0..3 {
            if !self.operation_is_current(generation, &token) {
                break;
            }
            self.set_state(
                app,
                "reconnecting",
                format!("{reason}，正在重连（{}/{})…", attempt + 1, 3),
                device_name.clone(),
            );
            tokio::select! { _ = token.cancelled() => break, _ = tokio::time::sleep(Duration::from_millis([1000, 2000, 5000][attempt])) => {} }
            if !self.operation_is_current(generation, &token) {
                break;
            }
            if adb_stream {
                let ops = AppStateAdbRecoveryAttemptOps {
                    state: self,
                    app,
                    operation_generation: generation,
                    was_playing,
                };
                match run_adb_recovery_attempt(&ops, &serial, &transport, &token).await {
                    AdbRecoveryAttempt::Cancelled => break,
                    AdbRecoveryAttempt::Offline | AdbRecoveryAttempt::Failed => {}
                    AdbRecoveryAttempt::ReceiverUserStopped => {
                        self.finish_operation(generation);
                        self.set_state(app, "ready", "手机已主动停止音频会话", None);
                        self.recovering.store(false, Ordering::Release);
                        return;
                    }
                    AdbRecoveryAttempt::CleanupFailed(error) => {
                        last_cleanup_error = Some(error);
                    }
                    AdbRecoveryAttempt::Connected => {
                        self.finish_operation(generation);
                        self.recovering.store(false, Ordering::Release);
                        return;
                    }
                }
            } else {
                if transport == "wifi" && serial.contains(':') {
                    let _ = checked_adb(
                        &self.adb_path,
                        None,
                        &["connect", &serial],
                        Duration::from_secs(8),
                        Some(&token),
                    )
                    .await;
                }
                if token.is_cancelled() {
                    break;
                }
                let _ = self.refresh_devices().await;
                if let Some(device) = self
                    .snapshot()
                    .devices
                    .iter()
                    .find(|device| {
                        device.serial == serial
                            && device.transport == transport
                            && device.connectable
                    })
                    .cloned()
                {
                    if let Ok(result) = self
                        .connect_locked(app, &device.serial, generation, &token, was_playing)
                        .await
                    {
                        if result.success {
                            if !was_playing {
                                let _ = self
                                    .set_output_locked(app, "computer", generation, &token)
                                    .await;
                                self.emit(app);
                            }
                            self.finish_operation(generation);
                            self.recovering.store(false, Ordering::Release);
                            return;
                        }
                    }
                    let _ = self.teardown(Some(app)).await;
                }
            }
        }
        self.finish_operation(generation);
        if !token.is_cancelled()
            && operation_generation_matches(&self.operation_generation, generation)
        {
            let message = last_cleanup_error.map_or_else(
                || "重连失败，请手动连接。".to_string(),
                |error| format!("ADB 旧会话清理失败，未启动新流：{error}"),
            );
            self.set_state(app, "error", message, device_name);
        }
        self.recovering.store(false, Ordering::Release);
    }

    pub async fn connect_device(
        &self,
        app: &AppHandle,
        serial: String,
    ) -> Result<OperationResult, String> {
        self.connect_device_mode(app, serial, true).await
    }

    pub async fn connect_device_only(
        &self,
        app: &AppHandle,
        serial: String,
    ) -> Result<OperationResult, String> {
        self.connect_device_mode(app, serial, false).await
    }

    pub async fn switch_device(
        &self,
        app: &AppHandle,
        serial: String,
    ) -> Result<OperationResult, String> {
        let (generation, token) = self.start_operation();
        let _guard = self.gate.lock().await;
        let result = async {
            if !self.operation_is_current(generation, &token) {
                return Ok(OperationResult {
                    success: false,
                    message: "设备切换已取消。".into(),
                    snapshot: self.snapshot(),
                });
            }
            if self.session.lock().await.as_ref().is_some_and(|session| {
                session.network_owner.is_some() || session.aoa_owner.is_some()
            }) {
                return Err("当前是直接连接会话；请先断开后再切换手机。".into());
            }
            let old_session = self
                .session
                .lock()
                .await
                .as_ref()
                .map(|session| (session.serial.clone(), session.output == "phone"));
            if old_session
                .as_ref()
                .is_some_and(|(old_serial, _)| old_serial == &serial)
            {
                return Ok(OperationResult {
                    success: true,
                    message: "当前已连接这部手机。".into(),
                    snapshot: self.snapshot(),
                });
            }
            let (device_available, cleanup_pending) = {
                let view = self.view.lock().expect("app view lock");
                let preferred = view.settings.preferred_transport.to_ascii_lowercase();
                (
                    view.devices.iter().any(|device| {
                        device.serial == serial
                            && device.connectable
                            && (device.transport == preferred || device.transport == "unknown")
                    }),
                    view.state == "error" && view.device_name.is_some() && !view.session_active,
                )
            };
            if !device_available {
                return Err("此手机当前不可连接，请刷新设备列表。".into());
            }
            if old_session.is_some() || cleanup_pending {
                if let Err(error) = self.teardown(Some(app)).await {
                    let mut view = self.view.lock().expect("app view lock");
                    view.state = "error".into();
                    view.message = format!("无法切换手机：{error}");
                    view.status_detail = "连接清理未完成；请断开连接后重试。".into();
                    drop(view);
                    self.emit(app);
                    return Ok(OperationResult {
                        success: false,
                        message: format!("无法切换手机：{error}"),
                        snapshot: self.snapshot(),
                    });
                }
            }
            let play_on_phone = old_session.as_ref().is_some_and(|(_, play)| *play);
            let switched = match self
                .connect_locked(app, &serial, generation, &token, play_on_phone)
                .await
            {
                Ok(result) => result,
                Err(error) => OperationResult {
                    success: false,
                    message: error,
                    snapshot: self.snapshot(),
                },
            };
            Ok(switched)
        }
        .await;
        self.finish_operation(generation);
        result
    }

    async fn connect_device_mode(
        &self,
        app: &AppHandle,
        serial: String,
        play_on_phone: bool,
    ) -> Result<OperationResult, String> {
        let (generation, token) = self.start_operation();
        let _guard = self.gate.lock().await;
        let result = if !self.operation_is_current(generation, &token) {
            Ok(OperationResult {
                success: false,
                message: "连接已取消。".into(),
                snapshot: self.snapshot(),
            })
        } else if self.session.lock().await.is_some() {
            Ok(OperationResult {
                success: false,
                message: "已有手机连接；请从设备卡片确认后切换。".into(),
                snapshot: self.snapshot(),
            })
        } else {
            self.connect_locked(app, &serial, generation, &token, play_on_phone)
                .await
        };
        self.finish_operation(generation);
        result
    }

    async fn connect_locked(
        &self,
        app: &AppHandle,
        serial: &str,
        generation: u64,
        token: &CancellationToken,
        play_on_phone: bool,
    ) -> Result<OperationResult, String> {
        if play_on_phone {
            let settings = self.view.lock().expect("view lock").settings.clone();
            if settings.preferred_transport == "Usb" && settings.wired_mode == "aoa" {
                return Err(
                    "USB Direct 有线模式已选；请从 USB Direct 入口连接，或切回 ADB。".into(),
                );
            }
            let quality = settings.audio_quality;
            validate_audio_quality(&quality)?;
            if settings.audio_backend == "legacy" && quality != "standard" {
                return Err("ADB连接仅支持标准音质。".into());
            }
        }
        if !self.operation_is_current(generation, token) {
            return Ok(OperationResult {
                success: false,
                message: "连接已取消。".into(),
                snapshot: self.snapshot(),
            });
        }
        let device = {
            let view = self.view.lock().expect("app view lock");
            view.devices
                .iter()
                .find(|d| d.serial == serial && d.connectable)
                .cloned()
                .ok_or_else(|| "此手机当前不可连接，请刷新设备列表。".to_string())?
        };
        self.view.lock().expect("view lock").selected_serial = Some(device.serial.clone());
        self.set_state(
            app,
            "connecting",
            "正在建立音频连接…",
            Some(device.display_name.clone()),
        );
        let result = self
            .connect_inner(app, &device, generation, token, play_on_phone)
            .await;
        if !self.operation_is_current(generation, token) {
            let cleanup = self.teardown(Some(app)).await;
            let message = cleanup.as_ref().err().map_or_else(
                || "连接已取消。".into(),
                |error| format!("连接已取消，但清理连接失败：{error}"),
            );
            let idle_state = if cleanup.is_err() {
                "error"
            } else if self.view.lock().expect("view lock").devices.is_empty() {
                "noDevice"
            } else {
                "ready"
            };
            self.set_state(app, idle_state, &message, None);
            self.view.lock().expect("view lock").output = "computer".into();
            self.emit(app);
            return Ok(OperationResult {
                success: false,
                message,
                snapshot: self.snapshot(),
            });
        }
        match result {
            Ok(message) => Ok(OperationResult {
                success: true,
                message,
                snapshot: self.snapshot(),
            }),
            Err(error) => {
                let cleanup_error = self.teardown(Some(app)).await.err();
                let message = cleanup_error.map_or(error.clone(), |cleanup| {
                    format!("{error}；清理连接失败：{cleanup}")
                });
                self.set_state(app, "error", &message, Some(device.display_name.clone()));
                Ok(OperationResult {
                    success: false,
                    message,
                    snapshot: self.snapshot(),
                })
            }
        }
    }

    async fn connect_adb_stream_inner(
        &self,
        app: &AppHandle,
        device: &Device,
        operation_generation: u64,
        token: &CancellationToken,
        play_on_phone: bool,
        settings: AppSettings,
    ) -> Result<String, String> {
        use crate::aoa_owner::AoaOwner;
        use crate::protocol::AudioFormatV2;

        let serial = device.serial.as_str();
        token_check(token)?;
        self.audio.initialize()?;
        self.audio.set_streaming(false)?;
        let quality = settings.audio_quality.as_str();
        let profile = audio_quality_profile(quality)
            .filter(|profile| *profile >= 4)
            .ok_or_else(|| "ADB Float32 audio requires HiFi or Hi-Res quality.".to_string())?;
        let mode = if settings.playback_mode.eq_ignore_ascii_case("lowlatency") {
            1
        } else {
            0
        };
        let format = AudioFormatV2::for_profile(profile, mode)
            .map_err(|_| "Unable to construct the authenticated Float32 format.".to_string())?;
        let native = self.audio.probe_float_format()?;
        crate::audio::validate_float_format_for_session(&native, format)?;
        let queue = Arc::new(FloatPcmQueue::new(mode == 1));
        queue.set_expected_format(format.sample_rate, 4096)?;
        let source = SessionAudioQueue::Float32(queue, format);
        let (binding_reference, material) = receiver::adb_binding_material(
            &self.adb_path,
            serial,
            if device.transport == "wifi" {
                "wifi"
            } else {
                "usb"
            },
            &self.data_dir,
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
            token,
        )
        .await?;
        let host_id = material
            .host_id()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let socket_name = format!("anpod_adb_{}", uuid_token()?);
        checked_adb(
            &self.adb_path,
            Some(serial),
            &[
                "reverse",
                &format!("localabstract:{socket_name}"),
                &format!("tcp:{port}"),
            ],
            Duration::from_secs(5),
            Some(token),
        )
        .await?;

        let generation = NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed);
        if !self.register_session_generation(0, generation, operation_generation, token) {
            return Err("ADB audio session was cancelled.".into());
        }
        *self.session.lock().await = Some(Session {
            generation,
            serial: serial.into(),
            transport: device.transport.clone(),
            reverse: socket_name.clone(),
            child: None,
            installed_receiver: true,
            receiver_owner_id: None,
            receiver_generation: None,
            output: "computer".into(),
            output_tail: Arc::new(Mutex::new(VecDeque::new())),
            network_owner: None,
            aoa_owner: None,
            adb_stream_owner: None,
            aoa_target_id: None,
            network_sink_queue: Some(source.clone()),
            network_binding_ref: Some(binding_reference.clone()),
            windows_mix_format: Some(windows_mix_format(native)),
        });
        {
            let mut view = self.view.lock().expect("app view lock");
            view.session_generation = Some(generation);
            view.session_active = true;
            view.session_serial = Some(serial.into());
            view.session_receiver_binding = Some(binding_reference.clone());
            view.session_transport = Some(device.transport.clone());
            view.session_capabilities = None;
            view.playback_diagnostics = None;
            view.negotiated_audio_format = None;
            view.windows_mix_format = None;
            view.network_recovery_seconds_remaining = None;
            view.session_android_version =
                (!device.android_version.is_empty()).then(|| device.android_version.clone());
            view.audio_bitrate_bps = Some(0);
        }
        *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;

        let accept_cancel = token.child_token();
        let mut accepted = tokio::task::spawn_blocking({
            let token = accept_cancel.clone();
            move || loop {
                if token.is_cancelled() {
                    return Err("ADB audio connection was cancelled.".to_string());
                }
                match listener.accept() {
                    Ok((stream, _peer)) => return Ok(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
        });
        let dispatch = receiver::start_adb_stream_audio(
            &self.adb_path,
            serial,
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
            &socket_name,
            &host_id,
            if device.transport == "wifi" {
                "wifi"
            } else {
                "usb"
            },
            token,
        )
        .await;
        if let Err(error) = dispatch {
            accept_cancel.cancel();
            let _ = accepted.await;
            return Err(error);
        }
        let stream = tokio::select! {
            result = &mut accepted => result.map_err(|error| format!("ADB accept worker failed: {error}"))??,
            _ = token.cancelled() => {
                accept_cancel.cancel();
                let _ = accepted.await;
                return Err("ADB audio connection was cancelled.".into());
            },
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
                accept_cancel.cancel();
                let _ = accepted.await;
                return Err("Receiver did not connect its ADB audio stream within 30 seconds.".into());
            },
        };
        token_check(token)?;
        let owner_queue = source.clone();
        let startup_diagnostics = self.startup_diagnostics.clone();
        let diagnostic_transport = if device.transport == "wifi" {
            "wifi-adb"
        } else {
            "usb-adb"
        };
        let owner = tokio::task::spawn_blocking(move || {
            AoaOwner::connect_adb_stream(
                stream,
                material,
                format,
                owner_queue,
                diagnostic_transport,
                startup_diagnostics,
            )
        })
        .await
        .map_err(|error| format!("ADB record owner failed: {error}"))??;
        token_check(token)?;
        let receiver_status = receiver::status(
            &self.adb_path,
            serial,
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
            token,
        )
        .await?;
        let receiver_owner_id = receiver_status
            .operation_owner_id
            .clone()
            .filter(|owner_id| !owner_id.is_empty())
            .ok_or_else(|| "ADB Receiver did not report its operation owner.".to_string())?;
        let receiver_generation = receiver_status
            .operation_generation
            .ok_or_else(|| "ADB Receiver did not report its operation generation.".to_string())?;
        if !receiver_status.service_active || !receiver_status.healthy {
            let _ = owner.stop();
            return Err(receiver_status
                .monitoring_error()
                .unwrap_or_else(|| "ADB Receiver is not healthy after P5-C startup.".into()));
        }
        let mut mix_format = Some(windows_mix_format(native));
        if play_on_phone {
            mix_format = self.start_session_audio_sink(&source)?;
            self.audio.mute_active()?;
        } else {
            owner.pause()?;
        }
        let mut session = self.session.lock().await;
        let active = session.as_ref().ok_or("ADB audio session was cancelled.")?;
        if active.generation != generation {
            let _ = owner.stop();
            return Err("ADB audio session generation changed during startup.".into());
        }
        let session_generation = active.generation;
        let cancel_signal = owner.cancel_signal();
        if !self.install_session_cancel(
            session_generation,
            session_generation,
            operation_generation,
            token,
            cancel_signal.clone(),
        ) {
            cancel_signal();
            drop(session);
            let _ = owner.stop();
            return Err("ADB audio session was cancelled.".into());
        }
        if token.is_cancelled() || !self.session_generation_is_current(session_generation) {
            cancel_signal();
            drop(session);
            let _ = owner.stop();
            return Err("ADB audio session was cancelled.".into());
        }
        if let Some(active) = session.as_mut() {
            active.adb_stream_owner = Some(owner);
            active.receiver_owner_id = Some(receiver_owner_id);
            active.receiver_generation = Some(receiver_generation);
            active.output = if play_on_phone { "phone" } else { "computer" }.into();
            active.windows_mix_format = mix_format.clone();
        }
        drop(session);
        {
            let mut view = self.view.lock().expect("app view lock");
            view.output = if play_on_phone { "phone" } else { "computer" }.into();
            view.settings.last_device_id = Some(serial.into());
            view.busy = false;
            view.settings.last_wireless_endpoint = if device.transport == "wifi" {
                Some(serial.into())
            } else {
                view.settings.last_wireless_endpoint.clone()
            };
            let settings = view.settings.clone();
            drop(view);
            let _ = self.save_settings(&settings);
        }
        let snapshot = self
            .session
            .lock()
            .await
            .as_ref()
            .and_then(|session| session.adb_stream_owner.as_ref())
            .map(AoaOwner::snapshot)
            .ok_or("ADB stream owner was not installed.")?;
        let mut view = self.view.lock().expect("app view lock");
        view.session_capabilities = Some(snapshot.capabilities);
        view.playback_diagnostics = snapshot.playback_progress;
        view.negotiated_audio_format = Some(negotiated_v2_audio_format(format));
        view.windows_mix_format = if play_on_phone { mix_format } else { None };
        view.state = if play_on_phone {
            "phoneReady"
        } else {
            "connected"
        }
        .into();
        view.message = if play_on_phone {
            "手机接收已就绪"
        } else {
            "已连接 · 电脑播放"
        }
        .into();
        view.status_detail = if play_on_phone {
            "Receiver 已认证并开始等待音频播放。".into()
        } else {
            "Receiver 已认证；手机输出保持暂停。".into()
        };
        drop(view);
        self.set_state(
            app,
            if play_on_phone {
                "phoneReady"
            } else {
                "connected"
            },
            if play_on_phone {
                "手机接收已就绪"
            } else {
                "已连接 · 电脑播放"
            },
            Some(device.display_name.clone()),
        );
        Ok(if play_on_phone {
            "手机接收端已就绪，等待音频数据。"
        } else {
            "已连接，声音继续从电脑播放。"
        }
        .into())
    }

    async fn connect_inner(
        &self,
        app: &AppHandle,
        device: &Device,
        operation_generation: u64,
        token: &CancellationToken,
        play_on_phone: bool,
    ) -> Result<String, String> {
        let serial = device.serial.as_str();
        token_check(token)?;
        self.cleanup_pending_adb_for_target(serial, &device.transport, token)
            .await?;
        let settings = self.view.lock().expect("app view lock").settings.clone();
        if settings.audio_backend == "receiver" && settings.audio_quality != "standard" {
            return self
                .connect_adb_stream_inner(
                    app,
                    device,
                    operation_generation,
                    token,
                    play_on_phone,
                    settings,
                )
                .await;
        }
        let installed_receiver =
            self.view.lock().expect("view lock").settings.audio_backend == "receiver";
        self.audio.initialize()?;
        self.audio.set_streaming(false)?;
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| e.to_string())?
            .local_addr()
            .map_err(|e| e.to_string())?
            .port();
        let generation = NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let reverse = format!("anpod_{generation}");
        let token_text = uuid_token()?;
        self.audio.listen(port, &token_text)?;
        let remote = format!("localabstract:{reverse}");
        let port_arg = format!("tcp:{port}");
        checked_adb(
            &self.adb_path,
            Some(serial),
            &["reverse", &remote, &port_arg],
            Duration::from_secs(5),
            Some(token),
        )
        .await?;
        let session_generation = NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed);
        if !self.register_session_generation(0, session_generation, operation_generation, token) {
            return Err("ADB session was cancelled.".into());
        }
        *self.session.lock().await = Some(Session {
            generation: session_generation,
            serial: serial.into(),
            transport: device.transport.clone(),
            reverse: reverse.clone(),
            child: None,
            installed_receiver,
            receiver_owner_id: None,
            receiver_generation: None,
            output: "computer".into(),
            output_tail: Arc::new(Mutex::new(VecDeque::new())),
            network_owner: None,
            aoa_owner: None,
            adb_stream_owner: None,
            aoa_target_id: None,
            network_sink_queue: None,
            network_binding_ref: None,
            windows_mix_format: None,
        });
        self.view.lock().expect("view lock").session_generation = Some(session_generation);
        {
            let mut view = self.view.lock().expect("view lock");
            view.session_active = true;
            view.session_serial = Some(device.serial.clone());
            view.session_receiver_binding = None;
            view.session_transport = Some(device.transport.clone());
            view.session_capabilities = None;
            view.playback_diagnostics = None;
            view.negotiated_audio_format = None;
            view.windows_mix_format = None;
            view.network_recovery_seconds_remaining = None;
            view.session_android_version =
                (!device.android_version.is_empty()).then(|| device.android_version.clone());
            view.audio_bitrate_bps = Some(0);
        }
        *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
        let mode = self
            .view
            .lock()
            .expect("view lock")
            .settings
            .playback_mode
            .to_ascii_lowercase();
        let receiver_status = if installed_receiver {
            let playback_mode = if mode == "lowlatency" || mode == "low_latency" {
                "lowlatency"
            } else {
                "balanced"
            };
            Some(
                receiver::start_audio(
                    &self.adb_path,
                    serial,
                    &self.receiver_apk_path,
                    &self.receiver_metadata_path,
                    &reverse,
                    &token_text,
                    playback_mode,
                    token,
                )
                .await?,
            )
        } else {
            checked_adb(
                &self.adb_path,
                Some(serial),
                &[
                    "push",
                    self.server_path.to_string_lossy().as_ref(),
                    "/data/local/tmp/anpod_server",
                ],
                Duration::from_secs(15),
                Some(token),
            )
            .await?;
            let mut command = Command::new(&self.adb_path);
            command
                .args([
                    "-s",
                    serial,
                    "shell",
                    "app_process",
                    "-Djava.class.path=/data/local/tmp/anpod_server",
                    "/data/local/tmp",
                    "com.anpod.server.Main",
                ])
                .arg(format!("socketName={reverse}"))
                .arg(format!("connectCode={token_text}"))
                .arg(format!(
                    "playbackMode={}",
                    if mode == "lowlatency" || mode == "low_latency" {
                        "low-latency"
                    } else {
                        "balanced"
                    }
                ))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let mut child = command
                .spawn()
                .map_err(|_| "无法启动手机端音频服务。".to_string())?;
            let output_tail = self
                .session
                .lock()
                .await
                .as_ref()
                .expect("session created before server")
                .output_tail
                .clone();
            let log_path = self.data_dir.join("logs").join("anpod.log");
            if let Some(stdout) = child.stdout.take() {
                tokio::spawn(capture_child_output(
                    stdout,
                    "stdout",
                    output_tail.clone(),
                    log_path.clone(),
                ));
            }
            if let Some(stderr) = child.stderr.take() {
                tokio::spawn(capture_child_output(
                    stderr,
                    "stderr",
                    output_tail.clone(),
                    log_path,
                ));
            }
            if let Some(session) = self.session.lock().await.as_mut() {
                session.child = Some(child);
            }
            None
        };
        if let Some(status) = receiver_status {
            if let Some(session) = self.session.lock().await.as_mut() {
                session.receiver_owner_id = status.operation_owner_id;
                session.receiver_generation = status.operation_generation;
            }
        }
        token_check(token)?;
        let handshake = self
            .audio
            .wait_handshake(&token_text, Duration::from_secs(10), token)
            .await?;
        token_check(token)?;
        if handshake != token_text {
            return Err("握手校验失败，已拒绝连接。".into());
        }
        if !installed_receiver
            && self
                .session
                .lock()
                .await
                .as_mut()
                .and_then(|session| session.child.as_mut())
                .is_some_and(|child| child.try_wait().map_or(true, |status| status.is_some()))
        {
            return Err("手机端音频服务已停止。".into());
        }
        if !self.audio.is_connected()? {
            return Err("手机端音频连接已中断。".into());
        }
        token_check(token)?;
        if play_on_phone && !installed_receiver {
            self.audio.mute_active()?;
        }
        self.audio.set_streaming(play_on_phone)?;
        token_check(token)?;
        self.audio.start()?;
        token_check(token)?;
        if play_on_phone && installed_receiver {
            self.audio.mute_active()?;
        }
        let output = if play_on_phone { "phone" } else { "computer" }.to_string();
        if let Some(session) = self.session.lock().await.as_mut() {
            session.output = output.clone();
        }
        {
            let mut view = self.view.lock().expect("view lock");
            view.output = output;
            view.settings.last_device_id = Some(serial.into());
            view.busy = false;
            view.settings.last_wireless_endpoint = if device.transport == "wifi" {
                Some(serial.into())
            } else {
                view.settings.last_wireless_endpoint.clone()
            };
            view.status_detail = if play_on_phone && installed_receiver {
                "手机接收端已就绪；电脑有声音发送后将开始播放。".into()
            } else if play_on_phone {
                format!(
                    "电脑声音正在通过 {} 传到手机。",
                    device.transport.to_uppercase()
                )
            } else {
                "手机连接保持中；声音继续从电脑播放。".into()
            };
            let settings = view.settings.clone();
            drop(view);
            let _ = self.save_settings(&settings);
        }
        let state = if play_on_phone && installed_receiver {
            "phoneReady"
        } else if play_on_phone {
            "streaming"
        } else {
            "connected"
        };
        let message = if state == "phoneReady" {
            "手机接收已就绪"
        } else if play_on_phone {
            "正在手机播放"
        } else {
            "已连接 · 电脑播放"
        };
        self.set_state(app, state, message, Some(device.display_name.clone()));
        self.log("Audio session connected");
        Ok(if state == "phoneReady" {
            "手机接收端已就绪，等待音频数据。".into()
        } else {
            "已连接并开始播放。".into()
        })
    }

    pub async fn disconnect(&self, app: &AppHandle) -> Result<OperationResult, String> {
        self.cancel_active_operation();
        self.cancel_qr_pairing();
        let _guard = match self.gate.try_lock() {
            Ok(guard) => {
                self.cancel_active_session(false);
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
        let mut view = self.view.lock().expect("view lock");
        view.state = if cleanup.is_err() {
            "error".into()
        } else if view.devices.is_empty() {
            "noDevice".into()
        } else {
            "ready".into()
        };
        view.message = cleanup.as_ref().err().map_or_else(
            || "已断开连接".into(),
            |error| format!("已断开，但清理连接失败：{error}"),
        );
        if cleanup.is_ok() {
            view.device_name = None;
        }
        view.output = "computer".into();
        view.busy = false;
        view.status_detail = if cleanup.is_ok()
            || cleanup
                .as_ref()
                .err()
                .is_some_and(|error| !has_restore_failure(error))
        {
            "电脑声音已恢复。选择手机卡片可重新连接。".into()
        } else {
            "电脑声音恢复待处理；可从托盘菜单选择“断开连接”重试，或重新启动 AnPod。".into()
        };
        drop(view);
        self.log("Audio session disconnected by user");
        self.emit(app);
        match cleanup {
            Ok(()) => Ok(OperationResult {
                success: true,
                message: "已断开连接。".into(),
                snapshot: self.snapshot(),
            }),
            Err(error) => Ok(OperationResult {
                success: false,
                message: format!("已断开，但清理连接失败：{error}"),
                snapshot: self.snapshot(),
            }),
        }
    }

    pub fn select_device(&self, serial: &str) -> Result<AppSnapshot, String> {
        let mut view = self.view.lock().expect("view lock");
        if view.session_transport.as_deref() == Some("aoa") {
            return Err("USB Direct 会话没有 ADB serial；请先断开后再选择 ADB 设备。".into());
        }
        if view.busy {
            return Err("连接期间无法更换手机。请先断开连接。".into());
        }
        if let Some(active) = view.session_serial.as_deref() {
            if active == serial {
                drop(view);
                return Ok(self.snapshot());
            }
            return Err("切换已连接的手机需要先确认中断当前连接。".into());
        }
        if view.state == "error" && view.device_name.is_some() {
            return Err("连接清理待完成；请先断开连接后再选择手机。".into());
        }
        if !view.devices.iter().any(|device| {
            device.serial == serial
                && (device.transport == view.settings.preferred_transport.to_ascii_lowercase()
                    || device.transport == "unknown")
        }) {
            return Err("所选手机不在当前连接方式的设备列表中。".into());
        }
        if view.selected_serial.as_deref() == Some(serial) {
            drop(view);
            return Ok(self.snapshot());
        }
        view.selected_serial = Some(serial.to_owned());
        drop(view);
        Ok(self.snapshot())
    }

    pub async fn reconfigure_session(
        &self,
        app: &AppHandle,
        kind: &str,
        value: &str,
    ) -> Result<AppSnapshot, String> {
        let selection = reconfigure_selection(kind, value, &self.snapshot().settings)?;
        let (generation, token) = self.start_operation();
        let result = self
            .reconfigure_locked(app, selection, kind, value, generation, &token)
            .await;
        self.finish_operation(generation);
        result
    }

    pub async fn apply_audio_settings(
        &self,
        app: &AppHandle,
        audio_quality: &str,
        playback_mode: &str,
    ) -> Result<AppSnapshot, String> {
        let (generation, token) = self.start_operation();
        let result = async {
            let _guard = self.gate.lock().await;
            token_check(&token)?;
            let old_settings = self.view.lock().expect("app view lock").settings.clone();
            let candidate = prepare_audio_settings(&old_settings, audio_quality, playback_mode)?;
            if candidate.audio_quality == old_settings.audio_quality
                && candidate.playback_mode == old_settings.playback_mode
            {
                return Ok(self.snapshot());
            }

            let p5c_active = self
                .session
                .lock()
                .await
                .as_ref()
                .is_some_and(|active| active.network_owner.is_some());
            if p5c_active {
                let mode = u8::from(candidate.playback_mode == "LowLatency");
                let (profile, parameters, expected_format) =
                    if candidate.audio_quality == "standard" {
                        (
                            HostAudioProfile::Standard,
                            AudioParameters {
                                sample_rate: 48_000,
                                channels: 2,
                                mode,
                            },
                            None,
                        )
                    } else {
                        let profile_id = audio_quality_profile(&candidate.audio_quality)
                            .ok_or("所选音质不可用。")?;
                        let format = AudioFormatV2::for_profile(profile_id, mode)
                            .map_err(|_| "所选音质不支持该播放模式。".to_string())?;
                        (
                            HostAudioProfile::AudioV2(format),
                            AudioParameters {
                                sample_rate: format.sample_rate,
                                channels: format.channels,
                                mode,
                            },
                            Some(format),
                        )
                    };
                let (
                    baseline_generation,
                    previous_session_id,
                    session_generation,
                    output,
                    audio_v2_known_unsupported,
                ) = {
                    let session = self.session.lock().await;
                    let active = session
                        .as_ref()
                        .ok_or_else(|| "无线接收会话已结束。".to_string())?;
                    let owner = active
                        .network_owner
                        .as_ref()
                        .ok_or_else(|| "无线接收会话已结束。".to_string())?;
                    let snapshot = owner.snapshot();
                    (
                        snapshot.generation,
                        snapshot.session_id,
                        active.generation,
                        active.output.clone(),
                        snapshot.capabilities.as_ref().is_some_and(|capabilities| {
                            capabilities.capability_bits & CAP_AUDIO_V2 == 0
                        }),
                    )
                };
                if expected_format.is_some() && audio_v2_known_unsupported {
                    return Err("接收端当前会话不支持此音频配置。".into());
                }
                if let Err(error) = self
                    .audio
                    .stop_pcm_sink()
                    .and_then(|()| self.audio.restore_muted())
                {
                    return Err(match self.teardown(Some(app)).await {
                        Ok(()) => error,
                        Err(cleanup) => format!("{error}；连接清理未完成：{cleanup}"),
                    });
                }
                let command_result = {
                    let mut session = self.session.lock().await;
                    let active = session
                        .as_mut()
                        .filter(|active| active.network_owner.is_some())
                        .ok_or_else(|| "无线接收会话已结束。".to_string())?;
                    active.network_sink_queue = None;
                    active.windows_mix_format = None;
                    active
                        .network_owner
                        .as_ref()
                        .expect("network session owner")
                        .reconfigure_audio(profile, parameters, output == "computer")
                };
                if let Err(error) = command_result {
                    return Err(match self.teardown(Some(app)).await {
                        Ok(()) => error,
                        Err(cleanup) => format!("{error}；连接清理未完成：{cleanup}"),
                    });
                }
                {
                    let mut view = self.view.lock().expect("app view lock");
                    view.negotiated_audio_format = None;
                    view.playback_diagnostics = None;
                    view.windows_mix_format = None;
                    view.state = "reconnecting".into();
                    view.message = "正在应用音频配置".into();
                    view.status_detail = "正在建立并确认新的无线音频会话。".into();
                }
                self.emit(app);
                let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
                let mut ready_audio = None;
                let mut failure = None;
                while tokio::time::Instant::now() < deadline {
                    if token.is_cancelled() {
                        failure = Some("音频配置应用已取消。".to_string());
                        break;
                    }
                    let session = self.session.lock().await;
                    let Some(owner) = session
                        .as_ref()
                        .and_then(|active| active.network_owner.as_ref())
                    else {
                        failure = Some("无线接收会话已结束。".to_string());
                        break;
                    };
                    let owner_snapshot = owner.snapshot();
                    if owner_snapshot.generation > baseline_generation
                        && owner_snapshot.session_id.is_some()
                        && owner_snapshot.session_id != previous_session_id
                    {
                        let expected_state = if output == "computer" {
                            owner_snapshot.state == SessionLifecycleState::Paused
                                && owner_snapshot.paused
                        } else {
                            matches!(
                                owner_snapshot.state,
                                SessionLifecycleState::ReceiverRunning
                                    | SessionLifecycleState::Streaming
                            ) && !owner_snapshot.paused
                        };
                        let expected_queue =
                            owner
                                .audio_queue()
                                .filter(|queue| match (expected_format, queue) {
                                    (None, SessionAudioQueue::Pcm16(_)) => true,
                                    (Some(format), SessionAudioQueue::Float32(_, actual)) => {
                                        format == *actual
                                    }
                                    _ => false,
                                });
                        if owner_snapshot.negotiated_format == expected_format
                            && expected_state
                            && expected_queue.is_some()
                        {
                            ready_audio =
                                expected_queue.map(|queue| (queue, owner_snapshot.clone()));
                            break;
                        }
                    }
                    if owner_snapshot.state == SessionLifecycleState::Stopped {
                        failure =
                            Some(owner_snapshot.last_error.unwrap_or_else(|| {
                                "Receiver 未能以所选音频配置重新建立连接。".into()
                            }));
                        break;
                    }
                    drop(session);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                let Some((queue, confirmed_owner)) = ready_audio else {
                    let reason =
                        failure.unwrap_or_else(|| "等待 Receiver 确认新音频配置超时。".into());
                    return Err(match self.teardown(Some(app)).await {
                        Ok(()) => reason,
                        Err(cleanup) => format!("{reason}；连接清理未完成：{cleanup}"),
                    });
                };
                let mix_format = if output == "phone" {
                    match self
                        .start_session_audio_sink(&queue)
                        .and_then(|mix| self.audio.mute_active().map(|()| mix))
                    {
                        Ok(mix) => mix,
                        Err(error) => {
                            return Err(match self.teardown(Some(app)).await {
                                Ok(()) => error,
                                Err(cleanup) => {
                                    format!("{error}；新会话清理未完成：{cleanup}")
                                }
                            });
                        }
                    }
                } else {
                    if let Err(error) = self
                        .audio
                        .stop_pcm_sink()
                        .and_then(|()| self.audio.restore_muted())
                    {
                        return Err(match self.teardown(Some(app)).await {
                            Ok(()) => error,
                            Err(cleanup) => format!("{error}；新会话清理未完成：{cleanup}"),
                        });
                    }
                    None
                };
                let owner_still_matches = {
                    let mut session = self.session.lock().await;
                    session.as_mut().is_some_and(|active| {
                        if !self.session_generation_is_current(active.generation) {
                            return false;
                        }
                        let Some(owner) = active.network_owner.as_ref() else {
                            return false;
                        };
                        let snapshot = owner.snapshot();
                        let queue_matches =
                            owner
                                .audio_queue()
                                .is_some_and(|actual| match (&queue, actual) {
                                    (
                                        SessionAudioQueue::Pcm16(expected),
                                        SessionAudioQueue::Pcm16(actual),
                                    ) => Arc::ptr_eq(expected, &actual),
                                    (
                                        SessionAudioQueue::Float32(expected, expected_format),
                                        SessionAudioQueue::Float32(actual, actual_format),
                                    ) => {
                                        Arc::ptr_eq(expected, &actual)
                                            && *expected_format == actual_format
                                    }
                                    _ => false,
                                });
                        let lifecycle_matches = if output == "computer" {
                            snapshot.state == SessionLifecycleState::Paused && snapshot.paused
                        } else {
                            matches!(
                                snapshot.state,
                                SessionLifecycleState::ReceiverRunning
                                    | SessionLifecycleState::Streaming
                            ) && !snapshot.paused
                        };
                        let confirmed = active.generation == session_generation
                            && snapshot.generation == confirmed_owner.generation
                            && snapshot.session_id == confirmed_owner.session_id
                            && snapshot.negotiated_format == expected_format
                            && lifecycle_matches
                            && snapshot.terminal_cause.is_none()
                            && queue_matches;
                        if confirmed {
                            active.network_sink_queue =
                                (output == "phone").then_some(queue.clone());
                            active.windows_mix_format = mix_format.clone();
                        }
                        confirmed
                    })
                };
                if !owner_still_matches || !self.operation_is_current(generation, &token) {
                    let reason = if token.is_cancelled() {
                        "音频配置应用已取消。"
                    } else {
                        "新音频会话在应用前失效。"
                    };
                    return Err(match self.teardown(Some(app)).await {
                        Ok(()) => reason.into(),
                        Err(cleanup) => format!("{reason}；新会话清理未完成：{cleanup}"),
                    });
                }
                let save_result = self.save_audio_preferences(&candidate);
                if let Err(error) = save_result {
                    return Err(match self.teardown(Some(app)).await {
                        Ok(()) => error,
                        Err(cleanup) => format!("{error}；新会话清理未完成：{cleanup}"),
                    });
                }
                {
                    let mut view = self.view.lock().expect("app view lock");
                    view.session_active = true;
                    view.session_generation = Some(session_generation);
                    view.session_capabilities = confirmed_owner.capabilities.clone();
                    view.playback_diagnostics = confirmed_owner.playback_diagnostics.clone();
                    view.negotiated_audio_format = confirmed_owner
                        .negotiated_format
                        .map(negotiated_v2_audio_format);
                    view.windows_mix_format =
                        (output == "phone").then(|| mix_format.clone()).flatten();
                    view.output = output.clone();
                    view.state = if output == "computer" {
                        "paused"
                    } else if confirmed_owner.state == SessionLifecycleState::Streaming {
                        "streaming"
                    } else {
                        "phoneReady"
                    }
                    .into();
                    view.message = if output == "computer" {
                        "已连接 · 电脑播放"
                    } else if confirmed_owner.state == SessionLifecycleState::Streaming {
                        "正在手机播放"
                    } else {
                        "手机接收已就绪"
                    }
                    .into();
                    view.status_detail = if output == "computer" {
                        "网络会话保持暂停；电脑声音已恢复到本机扬声器。".into()
                    } else if confirmed_owner.state == SessionLifecycleState::Streaming {
                        "Receiver 已确认播放引擎收到 PCM 音频。".into()
                    } else {
                        "P5-A 已认证；等待接收端播放引擎收到 PCM 音频。".into()
                    };
                }
                self.emit(app);
                return Ok(self.snapshot());
            }

            let (has_session, serial, transport, output, aoa_restart) = {
                let session = self.session.lock().await;
                match session.as_ref() {
                    Some(active) => {
                        let aoa_restart = active.aoa_owner.as_ref().map(|owner| {
                            let snapshot = owner.snapshot();
                            (
                                active.aoa_target_id.clone(),
                                active.network_binding_ref.clone(),
                                snapshot.capabilities.capability_bits,
                            )
                        });
                        (
                            true,
                            Some(active.serial.clone()),
                            active.transport.clone(),
                            active.output.clone(),
                            aoa_restart,
                        )
                    }
                    None => (false, None, String::new(), "computer".into(), None),
                }
            };

            if !has_session {
                if !self.operation_is_current(generation, &token) {
                    return Err("音频配置应用已取消。".into());
                }
                self.save_audio_preferences(&candidate)?;
                return Ok(self.snapshot());
            }

            if let Some((target_id, binding, capabilities)) = aoa_restart {
                if capabilities & CAP_AOA_SESSION_RESTART == 0 {
                    return Err(
                        "手机 Receiver 不支持在连接中应用这些音频设置；请断开后重试。".into(),
                    );
                }
                let target_id = target_id.ok_or("USB Direct 目标已失效。")?;
                let binding = binding.ok_or("USB Direct 绑定已失效。")?;
                let profile =
                    audio_quality_profile(&candidate.audio_quality).ok_or("所选音质不可用。")?;
                let mode = u8::from(candidate.playback_mode == "LowLatency");
                let format = AudioFormatV2::for_profile(profile, mode)
                    .map_err(|_| "所选音质不支持该播放模式。".to_string())?;
                self.restart_aoa_session_locked(
                    app, generation, &target_id, &binding, format, &output, &token,
                )
                .await?;
                if !self.operation_is_current(generation, &token) {
                    let reason = "音频配置应用已取消。";
                    return Err(match self.teardown(Some(app)).await {
                        Ok(()) => reason.into(),
                        Err(cleanup) => format!("{reason}；新会话清理未完成：{cleanup}"),
                    });
                }
                let save_result = self.save_audio_preferences(&candidate);
                if let Err(error) = save_result {
                    let cleanup = self.teardown(Some(app)).await;
                    return Err(match cleanup {
                        Ok(()) => error,
                        Err(cleanup_error) => {
                            format!("{error}；新会话清理未完成：{cleanup_error}")
                        }
                    });
                }
                self.emit(app);
                return Ok(self.snapshot());
            }

            self.teardown(Some(app)).await?;
            {
                let mut view = self.view.lock().expect("app view lock");
                view.settings = merge_audio_preferences(&view.settings, &candidate);
            }
            let reconnectable = matches!(transport.as_str(), "usb" | "wifi")
                && serial.as_deref().is_some_and(|serial| !serial.is_empty());
            let reconnect_result = if reconnectable {
                let serial = serial
                    .as_deref()
                    .expect("reconnectable session has a serial");
                match self
                    .connect_locked(app, serial, generation, &token, output == "phone")
                    .await
                {
                    Ok(result) if result.success => Ok(()),
                    Ok(result) => Err(result.message),
                    Err(error) => Err(error),
                }
            } else {
                Ok(())
            };
            if let Err(error) = reconnect_result {
                let cleanup = self.teardown(Some(app)).await;
                let restore = {
                    let mut view = self.view.lock().expect("app view lock");
                    let settings = merge_audio_preferences(&view.settings, &old_settings);
                    let saved = self.save_settings(&settings);
                    view.settings = settings;
                    saved
                };
                return Err(append_audio_apply_failures(error, cleanup, restore));
            }
            if !self.operation_is_current(generation, &token) {
                let cleanup = self.teardown(Some(app)).await;
                let restore = {
                    let mut view = self.view.lock().expect("view lock");
                    let settings = merge_audio_preferences(&view.settings, &old_settings);
                    let saved = self.save_settings(&settings);
                    view.settings = settings;
                    saved
                };
                return Err(append_audio_apply_failures(
                    "音频配置应用已取消。".into(),
                    cleanup,
                    restore,
                ));
            }
            let save_result = self.save_audio_preferences(&candidate);
            if let Err(error) = save_result {
                let cleanup = self.teardown(Some(app)).await;
                let restore = {
                    let mut view = self.view.lock().expect("app view lock");
                    let settings = merge_audio_preferences(&view.settings, &old_settings);
                    let saved = self.save_settings(&settings);
                    view.settings = settings;
                    saved
                };
                return Err(append_audio_apply_failures(error, cleanup, restore));
            }
            self.emit(app);
            Ok(self.snapshot())
        }
        .await;
        self.finish_operation(generation);
        result
    }

    async fn reconfigure_locked(
        &self,
        app: &AppHandle,
        selection: ReconfigureSelection,
        kind: &str,
        value: &str,
        generation: u64,
        token: &CancellationToken,
    ) -> Result<AppSnapshot, String> {
        let _guard = self.gate.lock().await;
        token_check(token)?;
        let aoa_session = {
            let session = self.session.lock().await;
            session.as_ref().and_then(|active| {
                let owner = active.aoa_owner.as_ref()?;
                let binding = active.network_binding_ref.as_ref()?;
                Some((
                    active.aoa_target_id.clone()?,
                    binding.clone(),
                    owner.snapshot().format,
                    active.output.clone(),
                    owner.snapshot().capabilities.capability_bits,
                ))
            })
        };
        let adb_stream_session = {
            let session = self.session.lock().await;
            session.as_ref().and_then(|active| {
                active.adb_stream_owner.as_ref().map(|owner| {
                    (
                        active.serial.clone(),
                        active.output.clone(),
                        owner.snapshot().format,
                    )
                })
            })
        };
        if let Some((serial, output, current_format)) = adb_stream_session {
            if kind == "playback" {
                let mode = match value {
                    "balanced" => 0,
                    "lowLatency" => 1,
                    _ => return Err("未知的播放模式。".into()),
                };
                AudioFormatV2::for_profile(current_format.profile, mode).map_err(|_| {
                    "ADB audio format does not support the selected playback mode.".to_string()
                })?;
                self.update_mode_setting(kind, value)?;
                self.teardown(Some(app)).await?;
                let generation = self.operation_generation.load(Ordering::Acquire);
                let result = self
                    .connect_locked(app, &serial, generation, token, output == "phone")
                    .await?;
                if !result.success {
                    return Err(result.message);
                }
                self.emit(app);
                return Ok(self.snapshot());
            }
        }
        if let Some((target_id, binding, current_format, output, capabilities)) = aoa_session {
            if kind != "playback" {
                return Err("USB Direct 会话连接方式和音质已锁定；请先断开。".into());
            }
            if capabilities & CAP_AOA_SESSION_RESTART == 0 {
                return Err(
                    "手机 Receiver 不支持在连接中切换播放模式；请更新 Receiver 或先断开。".into(),
                );
            }
            let mode = match value {
                "balanced" => 0,
                "lowLatency" => 1,
                _ => return Err("未知的播放模式。".into()),
            };
            let format = AudioFormatV2::for_profile(current_format.profile, mode)
                .map_err(|_| "USB Direct 音频格式不支持所选播放模式。".to_string())?;
            self.update_mode_setting(kind, value)?;
            self.restart_aoa_session_locked(
                app, generation, &target_id, &binding, format, &output, token,
            )
            .await?;
            self.emit(app);
            return Ok(self.snapshot());
        }
        {
            let session = self.session.lock().await;
            if let Some(active) = session
                .as_ref()
                .filter(|session| session.network_owner.is_some())
            {
                if kind != "playback" {
                    drop(session);
                } else {
                    let mode = match value {
                        "balanced" => 0,
                        "lowLatency" => 1,
                        _ => return Err("未知的播放模式。".into()),
                    };
                    let previous = self.snapshot().settings.playback_mode;
                    self.update_mode_setting(kind, value)?;
                    let owner = active
                        .network_owner
                        .as_ref()
                        .expect("network session owner");
                    if let Err(error) = owner
                        .set_mode(mode)
                        .and_then(|_| owner.set_paused(active.output == "computer"))
                    {
                        let rollback_mode = if previous.eq_ignore_ascii_case("lowlatency") {
                            "lowLatency"
                        } else {
                            "balanced"
                        };
                        let rollback = self.update_mode_setting("playback", rollback_mode);
                        return Err(rollback.err().map_or(error.clone(), |rollback| {
                            format!("{error}；设置回滚失败：{rollback}")
                        }));
                    }
                    {
                        let mut view = self.view.lock().expect("app view lock");
                        view.state = "reconnecting".into();
                        view.message = "正在应用播放模式".into();
                        view.status_detail = if active.output == "computer" {
                            "电脑输出保持暂停；Receiver 将使用新的认证会话应用播放模式。".into()
                        } else {
                            "正在建立新的认证会话应用播放模式；电脑输出在确认 PCM 格式前保持静音状态。".into()
                        };
                    }
                    self.emit(app);
                    return Ok(self.snapshot());
                }
            }
        }
        let (had_session, serial, old_output, old_transport, old_device) = {
            let session = self.session.lock().await;
            let view = self.view.lock().expect("view lock");
            if let Some(session) = session.as_ref() {
                (
                    true,
                    Some(session.serial.clone()),
                    session.output.clone(),
                    session.transport.clone(),
                    view.devices
                        .iter()
                        .find(|device| device.serial == session.serial)
                        .cloned(),
                )
            } else {
                (false, None, "computer".into(), "unknown".into(), None)
            }
        };
        if !had_session {
            self.update_mode_setting(kind, value)?;
            let _ = self.refresh_devices().await?;
            let guidance = match selection {
                ReconfigureSelection::WirelessUdp => {
                    "已选择无线 UDP；请提供本地接口、已保存绑定和接收端点后启动网络音频。"
                }
                ReconfigureSelection::WirelessAdb => "已选择无线 ADB；请选择无线调试设备后连接。",
                ReconfigureSelection::Usb => "连接方式已切换到 USB；请连接并授权手机。",
                ReconfigureSelection::Playback => "播放模式已保存。",
                ReconfigureSelection::AudioBackend => "接收后端已保存。",
            };
            self.view.lock().expect("view lock").output = "computer".into();
            self.set_state(app, "ready", guidance, None);
            self.emit(app);
            return Ok(self.snapshot());
        }

        self.set_state(
            app,
            "connecting",
            "正在有序切换连接…",
            old_device
                .as_ref()
                .map(|device| device.display_name.clone()),
        );
        if let Err(error) = self.teardown(Some(app)).await {
            self.set_state(
                app,
                "error",
                format!("切换前清理连接失败：{error}"),
                old_device
                    .as_ref()
                    .map(|device| device.display_name.clone()),
            );
            return Err(error);
        }
        {
            let mut view = self.view.lock().expect("view lock");
            view.state = "ready".into();
            view.busy = false;
        }
        token_check(token)?;
        self.update_mode_setting(kind, value)?;

        if old_transport == "wifi"
            && self.snapshot().settings.wireless_protocol == "adb"
            && matches!(
                selection,
                ReconfigureSelection::WirelessAdb
                    | ReconfigureSelection::Playback
                    | ReconfigureSelection::AudioBackend
            )
        {
            if let Some(endpoint) = serial.as_deref().filter(|endpoint| endpoint.contains(':')) {
                let _ = checked_adb(
                    &self.adb_path,
                    None,
                    &["connect", endpoint],
                    Duration::from_secs(10),
                    Some(token),
                )
                .await;
            }
        }
        token_check(token)?;
        self.refresh_devices().await?;

        let settings = self.snapshot().settings;
        let devices = self.snapshot().devices;
        let target = match selection {
            ReconfigureSelection::WirelessUdp => None,
            ReconfigureSelection::WirelessAdb => {
                let explicit = serial.clone().or(self.snapshot().selected_serial);
                explicit
                    .as_ref()
                    .and_then(|serial| {
                        devices.iter().find(|device| {
                            &device.serial == serial
                                && device.connectable
                                && device.transport == "wifi"
                        })
                    })
                    .map(|device| device.serial.clone())
                    .or_else(|| {
                        old_device.as_ref().and_then(|previous| {
                            let matching: Vec<_> = devices
                                .iter()
                                .filter(|device| {
                                    device.connectable
                                        && device.transport == "wifi"
                                        && same_physical_device(previous, device)
                                })
                                .collect();
                            (matching.len() == 1).then(|| matching[0].serial.clone())
                        })
                    })
            }
            ReconfigureSelection::Usb => old_device.as_ref().and_then(|previous| {
                let matching: Vec<_> = devices
                    .iter()
                    .filter(|device| {
                        device.connectable
                            && device.transport == "usb"
                            && same_physical_device(previous, device)
                    })
                    .collect();
                (matching.len() == 1).then(|| matching[0].serial.clone())
            }),
            ReconfigureSelection::Playback | ReconfigureSelection::AudioBackend => {
                if old_transport == "lan"
                    || (old_transport == "wifi" && !wireless_adb_enabled(&settings))
                {
                    None
                } else {
                    serial
                        .as_ref()
                        .and_then(|serial| {
                            devices
                                .iter()
                                .find(|device| &device.serial == serial && device.connectable)
                        })
                        .map(|device| device.serial.clone())
                }
            }
        };

        if let Some(serial) = target {
            let result = self
                .connect_locked(app, &serial, generation, token, old_output == "phone")
                .await?;
            if !result.success {
                return Err(result.message);
            }
            if old_output == "computer" {
                self.set_output_locked(app, "computer", generation, token)
                    .await?;
            }
        } else {
            let guidance = match selection {
                ReconfigureSelection::WirelessUdp => {
                    "已选择无线 UDP；请提供本地接口、已保存绑定和接收端点后启动网络音频。"
                }
                ReconfigureSelection::WirelessAdb
                    if devices.iter().any(|device| device.transport == "wifi") =>
                {
                    "已选择无线 ADB；请选择无线调试设备后连接。"
                }
                ReconfigureSelection::WirelessAdb => {
                    "已选择无线 ADB；请添加手机或搜索无线调试设备。"
                }
                ReconfigureSelection::Usb => "连接方式已切换到 USB；请连接并授权手机。",
                ReconfigureSelection::Playback => "播放模式已保存；请从设备卡片连接手机。",
                ReconfigureSelection::AudioBackend => "接收后端已保存；请从设备卡片连接手机。",
            };
            self.view.lock().expect("view lock").output = "computer".into();
            self.set_state(app, "ready", guidance, None);
        }
        Ok(self.snapshot())
    }

    fn update_mode_setting(&self, kind: &str, value: &str) -> Result<(), String> {
        let (key, normalized) = match (kind, value) {
            ("transport", "usb") => ("preferredTransport", "Usb"),
            ("transport", "wireless") => ("preferredTransport", "Wifi"),
            ("transport", "wifi") => ("preferredTransport", "Wifi"),
            ("audioBackend", "receiver") => ("audioBackend", "receiver"),
            ("audioBackend", "legacy") => ("audioBackend", "legacy"),
            ("wirelessProtocol", "udp") => ("wirelessProtocol", "udp"),
            ("wirelessProtocol", "adb") => ("wirelessProtocol", "adb"),
            ("playback", "balanced") => ("playbackMode", "Balanced"),
            ("playback", "lowLatency") => ("playbackMode", "LowLatency"),
            _ => return Err("未知的连接或播放模式。".into()),
        };
        let mut view = self.view.lock().expect("view lock");
        let mut settings = view.settings.clone();
        match key {
            "preferredTransport" => settings.preferred_transport = normalized.into(),
            "audioBackend" => settings.audio_backend = normalized.into(),
            "wirelessProtocol" => {
                settings.wireless_protocol = normalized.into();
                settings.preferred_transport = "Wifi".into();
            }
            "playbackMode" => settings.playback_mode = normalized.into(),
            _ => unreachable!(),
        }
        let settings = normalize_settings(settings);
        self.save_settings(&settings)?;
        self.host_offer_listener
            .lock()
            .expect("offer listener lock")
            .take();
        view.settings = settings;
        Ok(())
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
            .expect("diagnostics timeline lock")
            .export();
        let mut export = diagnostics_export(&self.snapshot(), timeline);
        export.startup_diagnostics = self
            .startup_diagnostics
            .as_ref()
            .map(StartupDiagnosticsCapture::export);
        let directory = self.data_dir.join("exports");
        fs::create_dir_all(&directory).map_err(|e| format!("无法创建诊断导出目录：{e}"))?;
        let path = directory.join(format!("diagnostics-{}.json", unix_seconds()));
        let body =
            serde_json::to_vec_pretty(&export).map_err(|e| format!("无法序列化诊断数据：{e}"))?;
        fs::write(&path, body).map_err(|e| format!("无法写入诊断文件：{e}"))?;
        self.log("Exported sanitized diagnostics snapshot");
        Ok(path.to_string_lossy().into_owned())
    }

    fn remember_pending_adb_cleanup(&self, cleanup: PendingAdbCleanup) {
        let mut pending = self
            .pending_adb_cleanup
            .lock()
            .expect("pending ADB cleanup lock");
        if let Some(existing) = pending.iter_mut().find(|item| {
            item.serial == cleanup.serial
                && item.transport == cleanup.transport
                && item.socket_name == cleanup.socket_name
        }) {
            existing.installed_receiver |= cleanup.installed_receiver;
            existing.receiver_stopped |= cleanup.receiver_stopped;
            existing.receiver_user_stopped |= cleanup.receiver_user_stopped;
            existing.reverse_removed |= cleanup.reverse_removed;
            if existing.receiver_owner_id.is_none() {
                existing.receiver_owner_id = cleanup.receiver_owner_id;
            }
            if existing.receiver_generation.is_none() {
                existing.receiver_generation = cleanup.receiver_generation;
            }
        } else {
            pending.push(cleanup);
        }
    }

    async fn cleanup_pending_adb_for_target(
        &self,
        serial: &str,
        transport: &str,
        cancel: &CancellationToken,
    ) -> Result<AdbCleanupOutcome, String> {
        let mut pending = self
            .pending_adb_cleanup
            .lock()
            .expect("pending ADB cleanup lock")
            .clone();
        let mut outcome = AdbCleanupOutcome::Complete;
        for cleanup in pending
            .iter_mut()
            .filter(|item| item.serial == serial && item.transport == transport)
        {
            if cleanup.receiver_user_stopped {
                outcome = AdbCleanupOutcome::ReceiverUserStopped;
            }
            let result = self.cleanup_adb_remote(cleanup, cancel).await;
            if let Some(stored) = self
                .pending_adb_cleanup
                .lock()
                .expect("pending ADB cleanup lock")
                .iter_mut()
                .find(|item| {
                    item.serial == cleanup.serial
                        && item.transport == cleanup.transport
                        && item.socket_name == cleanup.socket_name
                })
            {
                stored.receiver_stopped = cleanup.receiver_stopped;
                stored.receiver_user_stopped = cleanup.receiver_user_stopped;
                stored.reverse_removed = cleanup.reverse_removed;
            }
            if result? == AdbCleanupOutcome::ReceiverUserStopped {
                outcome = AdbCleanupOutcome::ReceiverUserStopped;
            }
        }
        let completed: Vec<_> = pending
            .iter()
            .filter(|item| {
                item.serial == serial
                    && item.transport == transport
                    && (!item.installed_receiver || item.receiver_stopped)
                    && item.reverse_removed
            })
            .map(|item| {
                (
                    item.serial.clone(),
                    item.transport.clone(),
                    item.socket_name.clone(),
                )
            })
            .collect();
        if !completed.is_empty() {
            self.pending_adb_cleanup
                .lock()
                .expect("pending ADB cleanup lock")
                .retain(|item| {
                    !completed.contains(&(
                        item.serial.clone(),
                        item.transport.clone(),
                        item.socket_name.clone(),
                    ))
                });
        }
        Ok(outcome)
    }

    async fn cleanup_adb_remote(
        &self,
        cleanup: &mut PendingAdbCleanup,
        cancel: &CancellationToken,
    ) -> Result<AdbCleanupOutcome, String> {
        let (stop, receiver_user_stopped) = if cleanup.installed_receiver
            && !cleanup.receiver_stopped
        {
            if let (Some(owner_id), Some(generation)) = (
                cleanup.receiver_owner_id.as_deref(),
                cleanup.receiver_generation,
            ) {
                match receiver::status(
                    &self.adb_path,
                    &cleanup.serial,
                    &self.receiver_apk_path,
                    &self.receiver_metadata_path,
                    cancel,
                )
                .await
                {
                    Ok(status)
                        if status.was_stopped_by_user_for(Some(owner_id), Some(generation)) =>
                    {
                        (Ok(()), true)
                    }
                    Ok(status) if !status.service_active => (Ok(()), false),
                    Ok(status)
                        if status.operation_owner_id.as_deref() != Some(owner_id)
                            || status.operation_generation != Some(generation) =>
                    {
                        (
                            Err(
                                "Receiver owner changed; refusing to stop a different session."
                                    .into(),
                            ),
                            false,
                        )
                    }
                    Ok(_) => match receiver::stop_if_owner(
                        &self.adb_path,
                        &cleanup.serial,
                        &self.receiver_apk_path,
                        &self.receiver_metadata_path,
                        owner_id,
                        generation,
                        &cleanup.socket_name,
                        cancel,
                    )
                    .await
                    {
                        Ok(status)
                            if status.was_stopped_by_user_for(Some(owner_id), Some(generation)) =>
                        {
                            (Ok(()), true)
                        }
                        Ok(_) => (Ok(()), false),
                        Err(error) => (Err(error), false),
                    },
                    Err(error) => (Err(error), false),
                }
            } else {
                match receiver::status(
                    &self.adb_path,
                    &cleanup.serial,
                    &self.receiver_apk_path,
                    &self.receiver_metadata_path,
                    cancel,
                )
                .await
                {
                    Ok(status) if !status.service_active => (Ok(()), false),
                    Ok(_) => (
                        Err(
                            "ADB Receiver owner is unknown; refusing to stop an unowned session."
                                .into(),
                        ),
                        false,
                    ),
                    Err(error) => (Err(error), false),
                }
            }
        } else {
            (Ok(()), false)
        };
        let reverse = if !cleanup.reverse_removed {
            adb_run(
                &self.adb_path,
                Some(&cleanup.serial),
                &[
                    "reverse",
                    "--remove",
                    &format!("localabstract:{}", cleanup.socket_name),
                ],
                Duration::from_secs(5),
                Some(cancel),
            )
            .await
            .and_then(|result| adb_reverse_remove_result(result, &cleanup.socket_name))
        } else {
            Ok(())
        };
        record_adb_cleanup_attempt(cleanup, stop, receiver_user_stopped, reverse)
    }

    async fn teardown_adb_for_recovery(
        &self,
        app: &AppHandle,
        session_generation: u64,
        session_key: &str,
    ) -> Result<AdbCleanupOutcome, String> {
        let (cleanup, owner) = {
            let mut session = self.session.lock().await;
            let active = session
                .as_mut()
                .filter(|active| {
                    active.generation == session_generation
                        && active.reverse == session_key
                        && active.adb_stream_owner.is_some()
                })
                .ok_or_else(|| "ADB recovery target changed before local cleanup.".to_string())?;
            let cleanup = PendingAdbCleanup {
                serial: active.serial.clone(),
                transport: active.transport.clone(),
                socket_name: active.reverse.clone(),
                installed_receiver: active.installed_receiver,
                receiver_owner_id: active.receiver_owner_id.clone(),
                receiver_generation: active.receiver_generation,
                receiver_stopped: !active.installed_receiver,
                receiver_user_stopped: false,
                reverse_removed: active.reverse.is_empty(),
            };
            active.reverse.clear();
            active.installed_receiver = false;
            (cleanup, active.adb_stream_owner.take())
        };
        self.remember_pending_adb_cleanup(cleanup);
        let owner_error = owner.and_then(|owner| {
            if owner.snapshot().terminal_cause == Some(TerminalCause::TransportFailure) {
                owner.cancel_after_transport_failure().err()
            } else {
                owner.stop().err()
            }
        });
        let local_result = self.teardown(Some(app)).await;
        match (owner_error, local_result) {
            (None, Ok(())) => Ok(AdbCleanupOutcome::Complete),
            (Some(error), Ok(())) => Err(format!("ADB record owner cleanup failed: {error}")),
            (None, Err(error)) => Err(error),
            (Some(owner_error), Err(local_error)) => Err(format!(
                "ADB record owner cleanup failed: {owner_error}; {local_error}"
            )),
        }
    }

    async fn teardown(&self, app: Option<&AppHandle>) -> Result<(), String> {
        self.cancel_active_session(false);
        self.host_offer_listener
            .lock()
            .expect("offer listener lock")
            .take();
        let mut errors = Vec::new();
        let retired_generation = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|session| session.generation);
        if let Some(mut session) = self.session.lock().await.take() {
            let adb_stream =
                session.adb_stream_owner.is_some() || session.reverse.starts_with("anpod_adb_");
            {
                let mut view = self.view.lock().expect("view lock");
                view.session_active = false;
                view.session_serial = None;
                view.session_receiver_binding = None;
                view.session_generation = None;
                view.session_transport = None;
                view.session_capabilities = None;
                view.playback_diagnostics = None;
                view.negotiated_audio_format = None;
                view.windows_mix_format = None;
                view.network_recovery_seconds_remaining = None;
                view.session_android_version = None;
                view.audio_bitrate_bps = None;
            }
            if let Some(mut owner) = session.network_owner.take() {
                owner.stop();
            }
            if let Some(owner) = session.aoa_owner.take() {
                if let Err(error) = owner.stop() {
                    errors.push(format!("USB Direct 会话停止失败：{error}"));
                }
            }
            if let Some(owner) = session.adb_stream_owner.take() {
                if let Err(error) = owner.stop() {
                    errors.push(format!("ADB audio stream stop failed: {error}"));
                }
            }
            if let Err(error) = self.audio.stop_and_cleanup() {
                errors.push(format!("本机音频采集停止失败：{error}"));
            }
            if let Err(error) = self.audio.restore_muted() {
                errors.push(format!("电脑声音恢复失败：{error}"));
            }
            if adb_stream {
                let mut cleanup = PendingAdbCleanup {
                    serial: session.serial.clone(),
                    transport: session.transport.clone(),
                    socket_name: session.reverse.clone(),
                    installed_receiver: session.installed_receiver,
                    receiver_owner_id: session.receiver_owner_id.clone(),
                    receiver_generation: session.receiver_generation,
                    receiver_stopped: !session.installed_receiver,
                    receiver_user_stopped: false,
                    reverse_removed: session.reverse.is_empty(),
                };
                if let Err(error) = self
                    .cleanup_adb_remote(&mut cleanup, &CancellationToken::new())
                    .await
                {
                    errors.push(format!("ADB remote cleanup failed: {error}"));
                    self.remember_pending_adb_cleanup(cleanup);
                }
            } else if session.installed_receiver {
                if let Err(error) = receiver::stop(
                    &self.adb_path,
                    &session.serial,
                    &self.receiver_apk_path,
                    &self.receiver_metadata_path,
                    &CancellationToken::new(),
                )
                .await
                {
                    errors.push(format!("Receiver 停止失败：{error}"));
                }
            }
            if let Some(child) = session.child.as_mut() {
                let tail = safe_android_tail(
                    &session
                        .output_tail
                        .lock()
                        .expect("output log lock")
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                );
                if !tail.is_empty() {
                    self.log(&format!(
                        "Android audio process output before teardown:\n{tail}"
                    ));
                }
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            if !adb_stream && !session.reverse.is_empty() {
                if let Err(error) = adb_run(
                    &self.adb_path,
                    Some(&session.serial),
                    &[
                        "reverse",
                        "--remove",
                        &format!("localabstract:{}", session.reverse),
                    ],
                    Duration::from_secs(5),
                    None,
                )
                .await
                .and_then(|result| {
                    if result.success {
                        Ok(())
                    } else {
                        Err("无法移除本次音频连接映射。".into())
                    }
                }) {
                    errors.push(format!("ADB 映射清理失败：{error}"));
                }
            }
            self.clear_session_cancel(retired_generation.expect("retired session generation"));
        } else {
            self.clear_invalidated_session_cancel();
            if let Err(error) = self.audio.stop_and_cleanup() {
                errors.push(format!("本机音频采集停止失败：{error}"));
            }
            if let Err(error) = self.audio.restore_muted() {
                errors.push(format!("电脑声音恢复失败：{error}"));
            }
        }
        *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
        {
            let mut view = self.view.lock().expect("app view lock");
            view.output = "computer".into();
            view.audio_bitrate_bps = None;
        }
        if let Some(app) = app {
            self.emit(app);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }

    pub async fn set_output(&self, app: &AppHandle, output: &str) -> Result<AppSnapshot, String> {
        if output != "computer" && output != "phone" {
            return Err("未知输出位置。".into());
        }
        let (generation, token) = self.start_operation();
        let result = self
            .set_output_operation(app, output, generation, &token)
            .await;
        self.finish_operation(generation);
        if result.is_err() {
            self.emit(app);
        }
        result
    }

    async fn set_output_operation(
        &self,
        app: &AppHandle,
        output: &str,
        generation: u64,
        token: &CancellationToken,
    ) -> Result<AppSnapshot, String> {
        let _guard = self.gate.lock().await;
        token_check(token)?;
        let session_up = self.session.lock().await.is_some();
        if !session_up && output == "computer" {
            self.emit(app);
            return Ok(self.snapshot());
        }
        if !session_up && output == "phone" {
            let serial = self
                .snapshot()
                .selected_serial
                .ok_or_else(|| "请先选择可连接的手机。".to_string())?;
            let result = self
                .connect_locked(app, &serial, generation, token, true)
                .await?;
            if !result.success {
                return Err(result.message);
            }
            return Ok(result.snapshot);
        }
        self.set_output_locked(app, output, generation, token)
            .await?;
        self.emit(app);
        Ok(self.snapshot())
    }

    async fn set_output_locked(
        &self,
        app: &AppHandle,
        output: &str,
        generation: u64,
        token: &CancellationToken,
    ) -> Result<(), String> {
        let adb_stream_serial = {
            let session = self.session.lock().await;
            session.as_ref().and_then(|active| {
                (active.adb_stream_owner.is_some() && active.output != output)
                    .then(|| active.serial.clone())
            })
        };
        if let Some(serial) = adb_stream_serial {
            token_check(token)?;
            self.teardown(Some(app)).await?;
            let generation = self.operation_generation.load(Ordering::Acquire);
            let result = self
                .connect_locked(app, &serial, generation, token, output == "phone")
                .await?;
            if result.success {
                if output == "computer" {
                    let mut view = self.view.lock().expect("app view lock");
                    view.state = "paused".into();
                    view.message = "ADB 播放已暂停".into();
                    view.status_detail = "ADB 音频会话保持暂停；电脑声音已恢复。".into();
                }
                return Ok(());
            }
            return Err(result.message);
        }
        if output == "phone" {
            let restart = {
                let session = self.session.lock().await;
                session.as_ref().and_then(|active| {
                    if active.output != "computer" {
                        return None;
                    }
                    let owner = active.aoa_owner.as_ref()?;
                    let snapshot = owner.snapshot();
                    Some((
                        active.aoa_target_id.clone()?,
                        active.network_binding_ref.clone()?,
                        snapshot.format,
                        snapshot.capabilities.capability_bits,
                    ))
                })
            };
            if let Some((target_id, binding, format, capabilities)) = restart {
                if capabilities & CAP_AOA_SESSION_RESTART == 0 {
                    return Err(
                        "手机 Receiver 不支持恢复 USB Direct 播放；请更新 Receiver 或重新连接。"
                            .into(),
                    );
                }
                self.restart_aoa_session_locked(
                    app, generation, &target_id, &binding, format, "phone", token,
                )
                .await?;
                return Ok(());
            }
        }
        {
            let mut session = self.session.lock().await;
            if let Some(active) = session
                .as_mut()
                .filter(|session| session.aoa_owner.is_some())
            {
                token_check(token)?;
                let owner = active.aoa_owner.as_ref().expect("AOA session owner");
                let snapshot = owner.snapshot();
                if output == "phone" {
                    let quality = self
                        .view
                        .lock()
                        .expect("view lock")
                        .settings
                        .audio_quality
                        .clone();
                    let expected_profile = audio_quality_profile(&quality)
                        .ok_or_else(|| "所选音质不可用。".to_string())?;
                    if snapshot.format.profile != expected_profile {
                        return Err(
                            "USB Direct 会话格式与当前音质设置不符；请断开后重新连接。".into()
                        );
                    }
                }
                if active.output == output
                    && !(output == "phone" && snapshot.state == crate::p5c::ReceiverState::Paused)
                {
                    return Ok(());
                }
                let changed = if output == "computer" {
                    owner
                        .pause()
                        .and_then(|()| self.audio.stop_pcm_sink())
                        .and_then(|()| self.audio.restore_muted())
                } else {
                    let queue = owner.audio_queue();
                    let sink = match &queue {
                        SessionAudioQueue::Pcm16(queue) => {
                            self.audio.start_pcm_sink(queue.clone()).map(|()| None)
                        }
                        SessionAudioQueue::Float32(_, _) => self.start_session_audio_sink(&queue),
                    };
                    sink.map(|mix| {
                        active.windows_mix_format =
                            mix.or_else(|| active.windows_mix_format.clone());
                    })
                    .and_then(|()| self.audio.mute_active())
                    .and_then(|()| owner.resume())
                };
                if let Err(error) = changed {
                    let retired_generation = active.generation;
                    let _ = self.audio.stop_pcm_sink();
                    let _ = self.audio.restore_muted();
                    if let Some(owner) = active.aoa_owner.take() {
                        let _ = owner.stop();
                    }
                    *session = None;
                    self.clear_session_cancel(retired_generation);
                    let mut view = self.view.lock().expect("view lock");
                    view.session_active = false;
                    view.session_serial = None;
                    view.session_receiver_binding = None;
                    view.session_generation = None;
                    view.session_transport = None;
                    view.session_capabilities = None;
                    view.playback_diagnostics = None;
                    view.negotiated_audio_format = None;
                    view.windows_mix_format = None;
                    view.output = "computer".into();
                    view.state = "error".into();
                    view.message = "USB Direct 输出切换失败".into();
                    view.status_detail = format!("本机音频和 USB Direct 会话已清理：{error}");
                    view.audio_bitrate_bps = None;
                    return Err(error);
                }
                active.output = output.into();
                let mut view = self.view.lock().expect("view lock");
                view.output = output.into();
                view.state = if output == "phone" {
                    "phoneReady"
                } else {
                    "paused"
                }
                .into();
                view.message = if output == "phone" {
                    "USB Direct Receiver 已就绪"
                } else {
                    "USB Direct 播放已暂停"
                }
                .into();
                view.status_detail = if output == "phone" {
                    "USB Direct 已恢复；ANPD v2 音频继续传输。"
                } else {
                    "USB Direct 会话保持暂停；电脑声音已恢复。"
                }
                .into();
                view.windows_mix_format = if output == "phone" {
                    active.windows_mix_format.clone()
                } else {
                    None
                };
                return Ok(());
            }
        }
        if output == "phone" {
            let quality = self
                .view
                .lock()
                .expect("view lock")
                .settings
                .audio_quality
                .clone();
            validate_audio_quality(&quality)?;
            let requested_quality_matches_session = self
                .session
                .lock()
                .await
                .as_ref()
                .and_then(|session| session.network_owner.as_ref())
                .map(|owner| owner.matches_audio_quality(&quality))
                .unwrap_or(quality == "standard");
            if !requested_quality_matches_session {
                return Err("The active session does not match the requested audio quality; reconnect to negotiate the selected format. USB/ADB TCP playback supports Standard only.".into());
            }
        }
        {
            let mut session = self.session.lock().await;
            if let Some(active) = session
                .as_mut()
                .filter(|session| session.network_owner.is_some())
            {
                token_check(token)?;
                let owner = active
                    .network_owner
                    .as_ref()
                    .expect("network session owner");
                if output == "phone"
                    && owner
                        .snapshot()
                        .capabilities
                        .as_ref()
                        .is_some_and(|info| !info.compatible)
                {
                    return Err("Receiver 管理连接已认证，但音频协议不兼容；请通过授权 ADB 或手动安装兼容版本。".into());
                }
                let change = async {
                    if output == "phone" {
                        let queue = if active.output == "computer" {
                            let old_queue = owner.audio_queue();
                            owner.reconnect()?;
                            owner.set_paused(false)?;
                            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
                            loop {
                                token_check(token)?;
                                let snapshot = owner.snapshot();
                                if snapshot.state == SessionLifecycleState::Stopped {
                                    return Err(snapshot
                                        .last_error
                                        .unwrap_or_else(|| "Receiver session stopped".into()));
                                }
                                if let Some(queue) = owner.audio_queue() {
                                    let is_fresh = session_audio_queue_identity_changed(old_queue.as_ref(), Some(&queue));
                                    if is_fresh
                                        && matches!(
                                            snapshot.state,
                                            SessionLifecycleState::ReceiverRunning
                                                | SessionLifecycleState::Streaming
                                                | SessionLifecycleState::Paused
                                        )
                                    {
                                        break queue;
                                    }
                                }
                                if tokio::time::Instant::now() >= deadline {
                                    return Err("手机未在 30 秒内恢复认证会话；请重新连接。".into());
                                }
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                        } else {
                            owner.set_paused(false)?;
                            let Some(queue) = owner.audio_queue() else {
                                return Ok(());
                            };
                            queue
                        };
                        active.windows_mix_format = self.start_session_audio_sink(&queue)?;
                        self.audio.mute_active()?;
                        active.network_sink_queue = Some(queue);
                    } else {
                        owner.set_paused(true)?;
                        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                        loop {
                            token_check(token)?;
                            match owner.snapshot().state {
                                SessionLifecycleState::Paused => break,
                                SessionLifecycleState::Stopped => {
                                    return Err("Receiver session stopped while pausing phone output.".into());
                                }
                                _ if tokio::time::Instant::now() >= deadline => {
                                    return Err("Receiver did not confirm pause before stopping local capture.".into());
                                }
                                _ => tokio::time::sleep(Duration::from_millis(10)).await,
                            }
                        }
                        self.audio.stop_pcm_sink()?;
                        self.audio.restore_muted()?;
                        active.network_sink_queue = None;
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = change {
                    let retired_generation = active.generation;
                    let mut cleanup = Vec::new();
                    if let Err(stop_error) = self.audio.stop_pcm_sink() {
                        cleanup.push(stop_error);
                    }
                    if let Err(restore_error) = self.audio.restore_muted() {
                        cleanup.push(restore_error);
                    }
                    if let Some(mut owner) = active.network_owner.take() {
                        owner.stop();
                    }
                    *session = None;
                    self.clear_session_cancel(retired_generation);
                    let detail = if cleanup.is_empty() {
                        error.clone()
                    } else {
                        format!("{error}；{}", cleanup.join("；"))
                    };
                    let mut view = self.view.lock().expect("app view lock");
                    view.session_active = false;
                    view.session_serial = None;
                    view.session_receiver_binding = None;
                    view.session_generation = None;
                    view.session_transport = None;
                    view.session_capabilities = None;
                    view.playback_diagnostics = None;
                    view.negotiated_audio_format = None;
                    view.windows_mix_format = None;
                    view.network_recovery_seconds_remaining = None;
                    view.session_android_version = None;
                    view.output = "computer".into();
                    view.state = "error".into();
                    view.message = "LAN 输出切换失败".into();
                    view.status_detail = format!("本机音频已清理；Receiver 会话停止：{detail}");
                    view.audio_bitrate_bps = None;
                    return Err(detail);
                }
                active.output = output.into();
                let active_snapshot = active
                    .network_owner
                    .as_ref()
                    .expect("network session owner")
                    .snapshot();
                let mut view = self.view.lock().expect("view lock");
                view.output = output.into();
                view.negotiated_audio_format = if output == "phone" {
                    active_snapshot
                        .negotiated_format
                        .map(negotiated_v2_audio_format)
                } else {
                    None
                };
                view.windows_mix_format = if output == "phone" {
                    active.windows_mix_format.clone()
                } else {
                    None
                };
                view.playback_diagnostics = if output == "phone" {
                    active_snapshot.playback_diagnostics
                } else {
                    None
                };
                view.state = if output == "phone" {
                    "phoneReady".into()
                } else {
                    "paused".into()
                };
                view.message = if output == "phone" {
                    "手机接收已就绪".into()
                } else {
                    "已连接 · 电脑播放".into()
                };
                view.status_detail = if output == "phone" {
                    "P5-C 会话已恢复；正在接入手机音频。".into()
                } else {
                    "网络会话保持暂停；电脑声音已恢复到本机扬声器。".into()
                };
                return Ok(());
            }
        }
        let receiver_session = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|session| (session.serial.clone(), session.installed_receiver));
        let installed_receiver = receiver_session
            .as_ref()
            .is_some_and(|(_, installed)| *installed);
        if output == "phone" && installed_receiver {
            let (serial, _) = receiver_session
                .as_ref()
                .expect("installed Receiver session");
            let status = receiver::health(
                &self.adb_path,
                serial,
                &self.receiver_apk_path,
                &self.receiver_metadata_path,
                token,
            )
            .await?;
            if !status.healthy {
                return Err(status.message);
            }
        }
        if output == "phone" {
            if let Err(mute_error) = self.audio.mute_active() {
                return match self.audio.restore_muted() {
                    Ok(()) => Err(format!(
                        "无法静音电脑播放端点，已回滚本次静音：{mute_error}"
                    )),
                    Err(restore_error) => {
                        let mut view = self.view.lock().expect("view lock");
                        view.state = "error".into();
                        view.message =
                            format!("切换到手机输出失败，且电脑声音恢复失败：{restore_error}");
                        view.status_detail = "电脑声音恢复待处理；可断开连接或从托盘重试。".into();
                        Err(format!("{mute_error}；恢复电脑声音失败：{restore_error}"))
                    }
                };
            }
        }
        if let Err(error) = self.audio.set_streaming(output == "phone") {
            let restore = if output == "phone" {
                self.audio.restore_muted().err()
            } else {
                None
            };
            let message = restore.as_ref().map_or_else(
                || format!("无法切换音频输出：{error}"),
                |restore| format!("无法切换音频输出，恢复电脑声音也失败：{restore}"),
            );
            let mut view = self.view.lock().expect("app view lock");
            view.state = "error".into();
            view.message = message;
            view.status_detail = "音频输出状态异常；请断开连接以恢复电脑声音。".into();
            return Err(restore.map_or(error.clone(), |restore| {
                format!("{error}；恢复电脑声音失败：{restore}")
            }));
        }
        if output == "computer" {
            if let Err(error) = self.audio.restore_muted() {
                let mut view = self.view.lock().expect("view lock");
                view.state = "error".into();
                view.message = format!("电脑声音恢复失败：{error}");
                view.status_detail = "电脑声音恢复待处理；可断开连接或从托盘重试。".into();
                return Err(error);
            }
        }
        {
            let mut view = self.view.lock().expect("view lock");
            view.output = output.into();
            view.status_detail = if output == "phone" && installed_receiver {
                "手机接收端已就绪；电脑有声音发送后将开始播放。".into()
            } else if output == "phone" {
                "电脑声音正在传到手机。".into()
            } else {
                "连接保持中；电脑声音已恢复到本机扬声器。".into()
            };
            view.state = if output == "phone" && installed_receiver {
                "phoneReady".into()
            } else if output == "phone" {
                "streaming".into()
            } else {
                "paused".into()
            };
            view.message = if output == "phone" && installed_receiver {
                "手机接收已就绪".into()
            } else if output == "phone" {
                "正在手机播放".into()
            } else {
                "手机播放已暂停".into()
            };
        }
        if let Some(session) = self.session.lock().await.as_mut() {
            session.output = output.into();
        }
        Ok(())
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
        self.log(&format!(
            "cancel_operation state={state} cleanup={}",
            if cleanup.is_ok() { "ok" } else { "failed" }
        ));
        let mut view = self.view.lock().expect("view lock");
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
        let mut view = self.view.lock().expect("view lock");
        if view.busy {
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
                .expect("offer listener lock")
                .take();
        }
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }

    pub async fn connect_wifi(&self, endpoint: &str) -> Result<OperationResult, String> {
        let _guard = self.gate.lock().await;
        let endpoint = normalize_endpoint(endpoint)?;
        if self
            .last_pairing_endpoint
            .lock()
            .expect("pair endpoint lock")
            .as_deref()
            == Some(endpoint.as_str())
        {
            return Err(
                "这是配对端口，不能用于连接；请搜索设备或使用手机无线调试主页上的连接端口。".into(),
            );
        }
        let result = checked_adb(
            &self.adb_path,
            None,
            &["connect", &endpoint],
            Duration::from_secs(10),
            None,
        )
        .await;
        let success = result.is_ok();
        let message = result
            .err()
            .unwrap_or_else(|| format!("已连接到 {endpoint}。"));
        if success {
            self.persist_wireless_endpoint(&endpoint)?;
            let _ = self.refresh_devices().await;
        }
        Ok(OperationResult {
            success,
            message,
            snapshot: self.snapshot(),
        })
    }

    pub async fn pair_wifi(&self, endpoint: &str, code: &str) -> Result<OperationResult, String> {
        let _guard = self.gate.lock().await;
        let endpoint = normalize_endpoint(endpoint)?;
        if code.len() != 6 || !code.bytes().all(|c| c.is_ascii_digit()) {
            return Err("配对码应为 6 位数字。".into());
        }
        let result = adb_run(
            &self.adb_path,
            None,
            &["pair", &endpoint, code],
            Duration::from_secs(20),
            None,
        )
        .await?;
        let success = result.success
            && result
                .stdout
                .to_ascii_lowercase()
                .contains("successfully paired");
        let mut message = if success {
            "配对成功。正在查找连接端口…".into()
        } else {
            "配对失败。请确认配对端口和 6 位配对码仍有效。".into()
        };
        if success {
            *self
                .last_pairing_endpoint
                .lock()
                .expect("pair endpoint lock") = Some(endpoint.clone());
            if let Ok(discovery) = self.discover_wifi().await {
                let host = endpoint
                    .rsplit_once(':')
                    .map(|(host, _)| host)
                    .unwrap_or("");
                let matches: Vec<_> = discovery["services"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|service| {
                        service["kind"] == "connect"
                            && service["endpoint"]
                                .as_str()
                                .unwrap_or("")
                                .starts_with(&format!("{host}:"))
                    })
                    .collect();
                if matches.len() == 1 {
                    if let Some(connect_endpoint) = matches[0]["endpoint"].as_str() {
                        if checked_adb(
                            &self.adb_path,
                            None,
                            &["connect", connect_endpoint],
                            Duration::from_secs(10),
                            None,
                        )
                        .await
                        .is_ok()
                        {
                            self.persist_wireless_endpoint(connect_endpoint)?;
                            let _ = self.refresh_devices().await;
                            message = "配对成功，并已连接手机。".into();
                        }
                    }
                } else if matches.len() > 1 {
                    message = "配对成功，但发现多个连接端口；请确认后手动选择。".into();
                } else {
                    message = "配对成功；未发现连接端口，请检查手机无线调试是否仍开启。".into();
                }
            } else {
                message = "配对成功；mDNS 搜索不可用，请手动输入无线调试连接端口。".into();
            }
        }
        self.log(if success {
            "Wireless pairing succeeded"
        } else {
            "Wireless pairing failed"
        });
        Ok(OperationResult {
            success,
            message,
            snapshot: self.snapshot(),
        })
    }

    fn persist_wireless_endpoint(&self, endpoint: &str) -> Result<(), String> {
        let mut view = self.view.lock().expect("view lock");
        if view.session_active {
            return Ok(());
        }
        let mut settings = view.settings.clone();
        settings.last_wireless_endpoint = Some(endpoint.to_owned());
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }

    pub async fn discover_wifi(&self) -> Result<Value, String> {
        self.discover_wifi_with_cancel(None).await
    }

    async fn discover_wifi_with_cancel(
        &self,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, String> {
        let check = adb_run(
            &self.adb_path,
            None,
            &["mdns", "check"],
            Duration::from_secs(5),
            cancel,
        )
        .await?;
        let combined = format!("{}\n{}", check.stdout, check.stderr).to_ascii_lowercase();
        if combined.contains("unavailable")
            || combined.contains("not running")
            || !combined.contains("mdns daemon version")
        {
            return Err("ADB mDNS 搜索不可用；请手动填写手机的 IP:连接端口。".into());
        }
        let services = adb_run(
            &self.adb_path,
            None,
            &["mdns", "services"],
            Duration::from_secs(6),
            cancel,
        )
        .await?;
        if !services.success {
            return Err("读取 ADB mDNS 设备失败。".into());
        }
        let found: Vec<Value> = services.stdout.lines().filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let endpoint = fields.iter().rev().find(|item| normalize_endpoint(item).is_ok())?;
            let line_l = line.to_ascii_lowercase();
            let kind = if line_l.contains("_adb-tls-pairing") { "pairing" } else if line_l.contains("_adb-tls-connect") || line_l.contains("_adb._tcp") { "connect" } else { return None; };
            Some(serde_json::json!({"name": fields.first().copied().unwrap_or(*endpoint), "endpoint": endpoint, "kind": kind}))
        }).collect();
        Ok(serde_json::json!({"services": found, "available": true}))
    }

    pub fn qr_credentials(&self) -> Result<Value, String> {
        use getrandom::fill;
        const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
        let mut bytes = [0u8; 30];
        fill(&mut bytes).map_err(|_| "无法生成配对凭据。".to_string())?;
        let instance: String = bytes[..10]
            .iter()
            .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
            .collect();
        let password: String = bytes[10..]
            .iter()
            .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
            .collect();
        let name = format!("studio-{instance}");
        let payload = format!("WIFI:T:ADB;S:{name};P:{password};;");
        Ok(serde_json::json!({"instanceName":name,"password":password,"payload":payload}))
    }

    pub async fn pair_qr(
        &self,
        app: &AppHandle,
        instance: String,
        password: String,
    ) -> Result<OperationResult, String> {
        if instance.len() != 17
            || !instance.starts_with("studio-")
            || !instance
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || password.len() != 20
            || !password.bytes().all(|c| c.is_ascii_alphanumeric())
        {
            return Err("二维码配对凭据格式无效。".into());
        }
        let (generation, token) = self.start_qr_operation();
        let result = self
            .pair_qr_with_token(app, &instance, &password, &token)
            .await;
        self.finish_qr_operation(generation);
        result
    }

    async fn pair_qr_with_token(
        &self,
        app: &AppHandle,
        instance: &str,
        password: &str,
        token: &CancellationToken,
    ) -> Result<OperationResult, String> {
        let _guard = self.gate.lock().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        let endpoint = loop {
            token_check(token)?;
            let found = self.discover_wifi_with_cancel(Some(token)).await?;
            let services = found
                .get("services")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if let Some(pairing) = services.into_iter().find(|s| {
                s["kind"] == "pairing"
                    && normalize_mdns_instance(s["name"].as_str().unwrap_or("")) == instance
            }) {
                break pairing["endpoint"]
                    .as_str()
                    .ok_or("未收到配对端口")?
                    .to_owned();
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("等待扫码超时，请重新生成二维码。".into());
            }
            let _ = app.emit("anpod://pairing-progress", "等待手机扫码…");
            tokio::select! { _ = token.cancelled() => return Err("二维码配对已取消。".into()), _ = tokio::time::sleep(Duration::from_millis(800)) => {} }
        };
        *self
            .last_pairing_endpoint
            .lock()
            .expect("pair endpoint lock") = Some(endpoint.clone());
        let _ = app.emit("anpod://pairing-progress", "已识别手机，正在配对…");
        let pair = adb_run(
            &self.adb_path,
            None,
            &["pair", &endpoint, password],
            Duration::from_secs(20),
            Some(token),
        )
        .await?;
        if !pair.success
            || !pair
                .stdout
                .to_ascii_lowercase()
                .contains("successfully paired")
        {
            return Err("二维码配对失败，请重新生成后重试。".into());
        }
        self.log("Wireless QR pairing succeeded");
        let host = endpoint.rsplit_once(':').map(|(h, _)| h).unwrap_or("");
        let connect_deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let (connect, connect_name) = loop {
            token_check(token)?;
            let services = self.discover_wifi_with_cancel(Some(token)).await?;
            let found = services["services"].as_array().cloned().unwrap_or_default();
            let matches: Vec<_> = found
                .iter()
                .filter(|service| {
                    service["kind"] == "connect"
                        && service["endpoint"]
                            .as_str()
                            .is_some_and(|value| value.starts_with(&format!("{host}:")))
                })
                .collect();
            if matches.len() == 1 {
                break (
                    matches[0]["endpoint"]
                        .as_str()
                        .ok_or("缺少二维码手机的无线连接端口")?
                        .to_owned(),
                    matches[0]["name"]
                        .as_str()
                        .ok_or("缺少二维码手机的连接实例名")?
                        .to_owned(),
                );
            }
            if matches.len() > 1 {
                return Err("二维码手机广播了多个连接端口；请手动选择无线调试连接地址。".into());
            }
            if tokio::time::Instant::now() >= connect_deadline {
                return Err(
                    "手机已配对，但没有发现对应的无线连接端口。请确认无线调试仍开启后重试。".into(),
                );
            }
            let _ = app.emit(
                "anpod://pairing-progress",
                "配对完成，正在等待手机广播连接端口…",
            );
            tokio::select! { _ = token.cancelled() => return Err("二维码配对已取消。".into()), _ = tokio::time::sleep(Duration::from_millis(700)) => {} }
        };
        checked_adb(
            &self.adb_path,
            None,
            &["connect", &connect],
            Duration::from_secs(10),
            Some(token),
        )
        .await?;
        let device_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        let device = loop {
            token_check(token)?;
            let snapshot = self.refresh_devices_with_cancel(Some(token)).await?;
            if let Some(device) = snapshot.devices.into_iter().find(|device| {
                device.transport == "wifi"
                    && device.state == "device"
                    && device.connectable
                    && (device.serial == connect
                        || normalize_mdns_instance(&device.serial)
                            == normalize_mdns_instance(&connect_name))
            }) {
                break device;
            }
            if tokio::time::Instant::now() >= device_deadline {
                return Err("无线连接命令已完成，但 ADB 未显示这部手机为已授权设备。请在设备列表中检查连接状态。".into());
            }
            tokio::select! { _ = token.cancelled() => return Err("二维码配对已取消。".into()), _ = tokio::time::sleep(Duration::from_millis(500)) => {} }
        };
        token_check(token)?;
        self.persist_wireless_endpoint(&connect)?;
        let mut view = self.view.lock().expect("view lock");
        if !view.session_active {
            view.selected_serial = Some(device.serial);
        }
        drop(view);
        Ok(OperationResult {
            success: true,
            message: "配对并连接完成。".into(),
            snapshot: self.snapshot(),
        })
    }

    pub fn cancel_qr_pairing(&self) {
        if let Some(active) = self.qr_operation.lock().expect("qr operation lock").take() {
            active.token.cancel();
        }
    }
}

#[cfg(test)]
#[path = "../state_tests.rs"]
mod tests;
