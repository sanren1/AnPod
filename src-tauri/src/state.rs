use crate::{audio::AudioActor, discovery::{self, Candidate, HostDiscoverySnapshot, HostOfferListener}, network::{PcmQueue}, network_interface::{self, Ipv4Interface}, p5c::{ReceiverCapabilities, SessionLifecycleState, SessionOwner}, protocol::PcmFormat};
use crate::receiver::{self, PlaybackDiagnostics, ReceiverStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    env,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::{process::{Child, Command}, sync::Mutex as AsyncMutex, time::timeout};
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

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default = "yes")]
    pub auto_connect_last_device: bool,
    #[serde(default)]
    pub last_device_id: Option<String>,
    #[serde(default = "yes")]
    pub minimize_to_tray: bool,
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default = "usb")]
    pub preferred_transport: String,
    #[serde(default)]
    pub wireless_protocol: String,
    #[serde(default)]
    pub audio_backend: String,
    #[serde(default = "yes")]
    pub auto_reconnect_wifi: bool,
    #[serde(default)]
    pub last_wireless_endpoint: Option<String>,
    #[serde(default = "balanced")]
    pub playback_mode: String,
    #[serde(default = "system")]
    pub appearance_theme: String,
}

fn schema_version() -> u32 { 1 }
fn yes() -> bool { true }
fn usb() -> String { "Usb".into() }
fn udp() -> String { "udp".into() }
fn legacy() -> String { "legacy".into() }
fn balanced() -> String { "Balanced".into() }
fn system() -> String { "System".into() }

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            auto_connect_last_device: true,
            last_device_id: None,
            minimize_to_tray: true,
            launch_at_login: false,
            preferred_transport: usb(),
            wireless_protocol: udp(),
            audio_backend: legacy(),
            auto_reconnect_wifi: true,
            last_wireless_endpoint: None,
            playback_mode: balanced(),
            appearance_theme: system(),
        }
    }
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
    pub session_transport: Option<String>,
    pub session_connection_transport: Option<String>,
    pub session_wireless_protocol: Option<String>,
    pub session_capabilities: Option<ReceiverCapabilities>,
    pub playback_diagnostics: Option<PlaybackDiagnostics>,
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
    pub status_detail: String,
    pub audio_bitrate_bps: Option<u64>,
    pub version: String,
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

struct Session {
    generation: u64,
    serial: String,
    transport: String,
    reverse: String,
    child: Option<Child>,
    installed_receiver: bool,
    output: String,
    output_tail: Arc<Mutex<VecDeque<String>>>,
    network_owner: Option<SessionOwner>,
    network_sink_queue: Option<Arc<PcmQueue>>,
    network_binding_ref: Option<String>,
}

struct ActiveOperation {
    generation: u64,
    token: CancellationToken,
}

struct ActiveQrOperation {
    generation: u64,
    token: CancellationToken,
}

struct ViewState {
    state: String,
    message: String,
    device_name: Option<String>,
    devices: Vec<Device>,
    selected_serial: Option<String>,
    session_serial: Option<String>,
    session_receiver_binding: Option<String>,
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
    network_recovery_seconds_remaining: Option<u64>,
}

pub struct AppState {
    view: Mutex<ViewState>,
    host_offer_listener: Mutex<Option<HostOfferListener>>,
    operation: Mutex<Option<ActiveOperation>>,
    operation_generation: AtomicU64,
    recovering: AtomicBool,
    wireless_probe_attempted: AtomicBool,
    failed_auto_connect: Mutex<Option<String>>,
    suppressed_auto_connect: Mutex<Option<String>>,
    qr_operation: Mutex<Option<ActiveQrOperation>>,
    qr_generation: AtomicU64,
    last_pairing_endpoint: Mutex<Option<String>>,
    gate: AsyncMutex<()>,
    session: AsyncMutex<Option<Session>>,
    pcm_rate_sample: Mutex<Option<(u64, u64, Instant)>>,
    receiver_health_sample: Mutex<Option<(u64, u32)>>,
    audio: AudioActor,
    data_dir: PathBuf,
    adb_path: PathBuf,
    server_path: PathBuf,
    receiver_apk_path: PathBuf,
    receiver_metadata_path: PathBuf,
}

impl AppState {
    pub async fn receiver_update_candidate(&self) -> Result<Option<ReceiverUpdateCandidate>, String> {
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
                if !matches!(snapshot.state, SessionLifecycleState::Authenticated | SessionLifecycleState::TransportConnected
                        | SessionLifecycleState::ReceiverRunning | SessionLifecycleState::Streaming | SessionLifecycleState::Paused) {
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
            (deps.join("adb.exe"), deps.join("server"), deps.join("receiver.apk"), deps.join("receiver.json"), deps.join("audio_capture.dll"))
        };
        #[cfg(not(debug_assertions))]
        let (adb_path, server_path, receiver_apk_path, receiver_metadata_path, audio_path) = {
            let app_dir = env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
            (app_dir.join("adb.exe"), app_dir.join("server"), app_dir.join("receiver.apk"), app_dir.join("receiver.json"), app_dir.join("audio_capture.dll"))
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
                network_recovery_seconds_remaining: None,
            }),
            host_offer_listener: Mutex::new(None),
            operation: Mutex::new(None),
            operation_generation: AtomicU64::new(0),
            recovering: AtomicBool::new(false),
            wireless_probe_attempted: AtomicBool::new(false),
            failed_auto_connect: Mutex::new(None),
            suppressed_auto_connect: Mutex::new(None),
            qr_operation: Mutex::new(None),
            qr_generation: AtomicU64::new(0),
            last_pairing_endpoint: Mutex::new(None),
            gate: AsyncMutex::new(()),
            session: AsyncMutex::new(None),
            pcm_rate_sample: Mutex::new(None),
            receiver_health_sample: Mutex::new(None),
            audio: AudioActor::new(audio_path, data_dir.join("muted-endpoints.txt")),
            data_dir,
            adb_path,
            server_path,
            receiver_apk_path,
            receiver_metadata_path,
        }
    }

    async fn begin_receiver_operation<'a>(&'a self, reject_active_audio: bool) -> Result<(u64, CancellationToken, tokio::sync::MutexGuard<'a, ()>), String> {
        let (generation, token) = begin_operation_slot(&self.operation_generation, &self.operation, true)
            .ok_or_else(|| "AnPod 正在执行其他设备操作。".to_string())?;
        let guard = self.gate.lock().await;
        if !self.operation_is_current(generation, &token) {
            self.finish_operation(generation);
            return Err("Receiver 管理操作已取消。".into());
        }
        let audio_active = self.session.try_lock().map_or(true, |session| session.is_some());
        if reject_active_audio && audio_active {
            self.finish_operation(generation);
            return Err("AnPod 音频会话正在使用此设备；Receiver 管理不会打断它。".into());
        }
        Ok((generation, token, guard))
    }

    pub async fn receiver_status(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = receiver::status(&self.adb_path, serial, &self.receiver_apk_path, &self.receiver_metadata_path, &token).await;
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_install(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = async {
            receiver::validate_usb_install(&self.adb_path, serial, &self.receiver_apk_path,
                &self.receiver_metadata_path, &token).await?;
            self.ensure_receiver_operation_target(serial, None).await?;
            let has_session = self.session.lock().await.is_some();
            if token.is_cancelled() { return Err("Receiver 管理操作已取消。".into()); }
            if has_session { self.teardown(None).await?; }
            receiver::install(&self.adb_path, serial, &self.receiver_apk_path,
                &self.receiver_metadata_path, &token).await
        }.await;
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_upgrade_usb(&self, serial: &str, binding_reference: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = async {
            let path = binding_store_path(&self.data_dir, binding_reference)?;
            let actual_reference = receiver::binding_reference(&self.adb_path, serial, &token).await?;
            if actual_reference != binding_reference {
                return Err("USB 设备序列号或 Android 用户与所选 Receiver 绑定不匹配；未执行升级。".into());
            }
            receiver::validate_usb_upgrade(&self.adb_path, serial, binding_reference,
                &self.receiver_apk_path, &path, &self.receiver_metadata_path, &token).await?;
            self.ensure_receiver_operation_target(serial, Some(binding_reference)).await?;
            let has_session = self.session.lock().await.is_some();
            if token.is_cancelled() { return Err("Receiver 管理操作已取消。".into()); }
            if has_session { self.teardown(None).await?; }
            receiver::upgrade_usb(&self.adb_path, serial, binding_reference, &path, &self.receiver_apk_path,
                &self.receiver_metadata_path, &token).await
        }.await;
        self.finish_operation(generation);
        result
    }

    async fn ensure_receiver_operation_target(&self, serial: &str, binding_reference: Option<&str>) -> Result<(), String> {
        let session = self.session.lock().await;
        let Some(active) = session.as_ref() else { return Ok(()); };
        let matches = receiver_target_matches(&active.serial, active.network_owner.is_some(),
            active.network_binding_ref.as_deref(), serial, binding_reference);
        if matches { Ok(()) }
        else { Err("AnPod 正在使用另一台设备；未中断当前会话或执行 Receiver 操作。".into()) }
    }

    pub async fn receiver_start(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(true).await?;
        let result = receiver::start(&self.adb_path, serial, &self.receiver_apk_path, &self.receiver_metadata_path, &token).await;
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_health(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = receiver::health(&self.adb_path, serial, &self.receiver_apk_path, &self.receiver_metadata_path, &token).await;
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_stop(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(true).await?;
        let result = receiver::stop(&self.adb_path, serial, &self.receiver_apk_path, &self.receiver_metadata_path, &token).await;
        self.finish_operation(generation);
        result
    }

    pub fn network_interfaces(&self) -> Result<Vec<Ipv4Interface>, String> {
        network_interface::enumerate_private_ipv4()
    }

    pub async fn receiver_network_interfaces(&self, serial: &str) -> Result<Vec<receiver::ReceiverInterface>, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = receiver::network_interfaces(&self.adb_path, serial, &self.receiver_apk_path,
            &self.receiver_metadata_path, &token).await;
        self.finish_operation(generation);
        result
    }

    pub fn receiver_binding_references(&self) -> Result<Vec<String>, String> {
        let directory = self.data_dir.join("receiver-bindings");
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err("无法读取已保存的 Receiver 绑定列表。".into()),
        };
        let mut references = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|_| "无法读取已保存的 Receiver 绑定列表。".to_string())?;
            if entry.path().extension().is_some_and(|extension| extension == "dat") {
                if let Some(reference) = entry.path().file_stem().and_then(|stem| stem.to_str()) {
                    if valid_binding_reference(reference) { references.push(reference.to_owned()); }
                }
            }
        }
        references.sort();
        Ok(references)
    }

    pub async fn receiver_binding_reset_preview(&self, serial: &str) -> Result<receiver::BindingResetPreview, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        #[cfg(windows)]
        let result = receiver::binding_reset_preview(&self.adb_path, serial, &self.data_dir,
            &self.receiver_apk_path, &self.receiver_metadata_path, &token).await;
        #[cfg(not(windows))]
        let result = Err("USB 绑定重置仅在 Windows 上可用。".into());
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_reset_binding(&self, expected: receiver::BindingResetPreview)
        -> Result<receiver::BindingResetResult, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = async {
            let before = self.receiver_binding_reset_preview_under_gate(&expected.serial, &token).await?;
            validate_receiver_reset_preview(&before, &expected)?;
            let has_session = self.session.lock().await.is_some();
            if has_session {
                self.ensure_receiver_operation_target(&expected.serial, Some(&expected.binding_reference)).await?;
            } else if before.receiver_busy {
                return Err("手机仍有活动的接收或播放会话；请先在手机上停止接收，再重新预览。未删除绑定。".into());
            }
            if !self.operation_is_current(generation, &token) {
                return Err("绑定重置已取消；尚未更改任何绑定。".into());
            }
            if has_session {
                self.teardown(None).await?;
                if before.receiver_busy {
                    if !self.operation_is_current(generation, &token) {
                        return Err("会话已停止，但绑定重置随后取消；绑定仍保留。".into());
                    }
                    receiver::stop(&self.adb_path, &expected.serial, &self.receiver_apk_path,
                        &self.receiver_metadata_path, &token).await?;
                }
            }
            if !self.operation_is_current(generation, &token) {
                return Err("绑定重置已取消；绑定仍保留。".into());
            }
            let after_stop = self.receiver_binding_reset_preview_under_gate(&expected.serial, &token).await?;
            validate_receiver_reset_preview(&after_stop, &expected)?;
            if after_stop.receiver_busy {
                return Err("手机仍报告活动接收或播放状态；请在手机上停止接收后重试。绑定未删除。".into());
            }
            let is_current = || self.operation_is_current(generation, &token);
            receiver::reset_binding(&self.adb_path, &after_stop, &self.data_dir,
                &self.receiver_apk_path, &self.receiver_metadata_path, &token, &is_current).await
        }.await;
        if let Ok(reset) = &result {
            if !reset_preserves_wireless_listener(reset, &expected) {
                let dropped = {
                    let mut listener = self.host_offer_listener.lock().expect("offer listener lock");
                    if listener.as_ref().is_some_and(|current|
                        wireless_listener_matches_binding(current.automatic_binding.as_deref(), &expected.binding_reference)) {
                        listener.take()
                    } else { None }
                };
                drop(dropped);
            }
        }
        self.finish_operation(generation);
        result
    }

    async fn receiver_binding_reset_preview_under_gate(&self, serial: &str, token: &CancellationToken)
        -> Result<receiver::BindingResetPreview, String> {
        #[cfg(windows)]
        { receiver::binding_reset_preview(&self.adb_path, serial, &self.data_dir,
            &self.receiver_apk_path, &self.receiver_metadata_path, token).await }
        #[cfg(not(windows))]
        { let _ = (serial, token); Err("USB 绑定重置仅在 Windows 上可用。".into()) }
    }

    pub async fn discover_network_receivers(&self, local_address: &str) -> Result<Vec<Candidate>, String> {
        let interface = self.network_interfaces()?.into_iter().find(|item| item.address.to_string() == local_address)
            .ok_or_else(|| "所选 IPv4 接口已不可用；请重新读取接口列表。".to_string())?;
        let cancellation = CancellationToken::new();
        let targets = discovery::probe_targets(&interface);
        tokio::task::spawn_blocking(move || discovery::discover_on_interface(&interface, &targets,
            Duration::from_secs(2), &cancellation)).await.map_err(|_| "网络发现任务已停止。".to_string())?
    }

    pub fn set_wireless_connection_offers(&self, local_address: &str, allowed: bool,
        selected_binding: Option<&str>) -> Result<HostDiscoverySnapshot, String> {
        if !allowed {
            self.host_offer_listener.lock().expect("offer listener lock").take();
            return Ok(HostDiscoverySnapshot::default());
        }
        let (interface, settings, active, busy) = {
            let view = self.view.lock().expect("app view lock");
            (self.network_interfaces()?.into_iter().find(|item| item.address.to_string() == local_address),
                view.settings.clone(), view.session_active, view.busy)
        };
        if !settings.preferred_transport.eq_ignore_ascii_case("wifi") || settings.wireless_protocol != "udp" {
            return Err("请先将无线连接方式明确设为 UDP，再允许 Receiver 发起连接请求。".into());
        }
        if active { return Err("活动会话期间不能更改无线连接请求监听。".into()); }
        if busy { return Err("AnPod 正在处理连接操作；请稍后再更改连接请求监听。".into()); }
        if self.operation.lock().expect("operation lock").is_some() {
            return Err("连接操作尚未结束；请稍后再允许手机连接。".into());
        }
        let references = self.receiver_binding_references()?;
        let binding = automatic_reception_binding(&references, selected_binding)?;
        let interface = interface.ok_or_else(|| "所选 IPv4 接口已不可用；请重新读取接口列表。".to_string())?;
        let mut listener = self.host_offer_listener.lock().expect("offer listener lock");
        if let Some(current) = listener.as_ref() {
            let snapshot = current.snapshot();
            if snapshot.active && snapshot.interface_address == Some(interface.address)
                && snapshot.interface_prefix_length == Some(interface.prefix_length) {
                listener.as_mut().expect("current listener").automatic_binding = binding;
                return Ok(snapshot);
            }
            listener.take();
        }
        *listener = Some(HostOfferListener::start(&interface)?);
        listener.as_mut().expect("listener was just installed").automatic_binding = binding;
        Ok(listener.as_ref().expect("listener was just installed").snapshot())
    }

    pub fn wireless_connection_offers(&self) -> HostDiscoverySnapshot {
        let Some((Some(address), Some(prefix))) = self.host_offer_listener.lock().expect("offer listener lock").as_ref()
            .map(|listener| (listener.interface_address(), listener.snapshot().interface_prefix_length)) else {
            return HostDiscoverySnapshot::default();
        };
        let active_interface_valid = match self.network_interfaces() {
            Ok(interfaces) => interfaces.iter().any(|interface| interface.address == address && interface.prefix_length == prefix),
            Err(_) => false,
        };
        if !active_interface_valid {
            self.host_offer_listener.lock().expect("offer listener lock").take();
            return HostDiscoverySnapshot::default();
        }
        self.host_offer_listener.lock().expect("offer listener lock").as_ref()
            .map(HostOfferListener::snapshot).unwrap_or_default()
    }

    #[cfg(windows)]
    pub async fn accept_wireless_connection_offer(&self, app: &AppHandle, offer_id: &str,
        binding_ref: &str) -> Result<AppSnapshot, String> {
        if !valid_binding_reference(binding_ref) { return Err("必须明确选择已保存的 Receiver 绑定。".into()); }
        {
            let view = self.view.lock().expect("app view lock");
            if !view.settings.preferred_transport.eq_ignore_ascii_case("wifi") || view.settings.wireless_protocol != "udp" {
                return Err("无线连接方式已更改；请先明确选择 UDP 并重新允许连接。".into());
            }
            if view.session_active { return Err("已有活动会话；请先断开后再接受连接请求。".into()); }
            if view.busy { return Err("AnPod 正在处理连接操作；请稍后再接受连接请求。".into()); }
        }
        let Some(reservation) = self.start_background_operation() else {
            return Err("AnPod 正在处理连接操作；请稍后再接受连接请求。".into());
        };
        let offer_result = (|| {
            let listener = self.host_offer_listener.lock().expect("offer listener lock");
            let listener = listener.as_ref().ok_or_else(|| "无线连接请求监听已关闭。".to_string())?;
            let snapshot = listener.snapshot();
            let address = snapshot.interface_address.ok_or_else(|| "监听接口已不可用；请重新选择网络接口。".to_string())?;
            if !self.network_interfaces()?.iter().any(|item| item.address == address
                && Some(item.prefix_length) == snapshot.interface_prefix_length) {
                return Err("监听接口已变化；旧连接请求已失效，请重新选择接口并允许连接。".into());
            }
            let offer = listener.take_offer(offer_id)
                .ok_or_else(|| "连接请求已过期或已处理，请让 Receiver 重新搜索。".to_string())?;
            Ok((offer, address, listener.cancellation()))
        })();
        let (offer, address, cancellation) = match offer_result {
            Ok(offer) => offer,
            Err(error) => { self.finish_operation(reservation.0); return Err(error); }
        };
        self.start_network_audio_inner(app, &address.to_string(), &offer.endpoint.to_string(), None,
            None, None, Some(binding_ref), Some((offer.nonce, false, cancellation)), Some(reservation)).await
    }

    #[cfg(not(windows))]
    pub async fn accept_wireless_connection_offer(&self, _app: &AppHandle, _offer_id: &str,
        _binding_ref: &str) -> Result<AppSnapshot, String> {
        Err("Windows 网络音频接收仅在 Windows 上可用。".into())
    }

    #[cfg(windows)]
    pub async fn start_network_audio(&self, app: &AppHandle, local_address: &str, peer: &str,
        bootstrap_serial: Option<&str>, receiver_address: Option<&str>, receiver_prefix: Option<u8>,
        selected_binding_ref: Option<&str>) -> Result<AppSnapshot, String> {
        self.start_network_audio_inner(app, local_address, peer, bootstrap_serial, receiver_address,
            receiver_prefix, selected_binding_ref, None, None).await
    }

    #[cfg(windows)]
    async fn start_network_audio_inner(&self, app: &AppHandle, local_address: &str, peer: &str,
        bootstrap_serial: Option<&str>, receiver_address: Option<&str>, receiver_prefix: Option<u8>,
        selected_binding_ref: Option<&str>, intent: Option<([u8; 16], bool, CancellationToken)>,
        reservation: Option<(u64, CancellationToken)>) -> Result<AppSnapshot, String> {
        use crate::security::{AudioParameters, BindingStore};
        let (generation, token) = reservation.unwrap_or_else(|| self.start_operation());
        let mut audio_initialized = false;
        let _gate = self.gate.lock().await;
        let result = async {
            token_check(&token)?;
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
            let capture_format = self.audio.pcm_format()?;
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
            let parameters = AudioParameters { sample_rate: capture_format.sample_rate, channels: capture_format.channels as u8,
                mode: if self.view.lock().expect("view lock").settings.playback_mode.eq_ignore_ascii_case("lowlatency") { 1 } else { 0 } };
            let mut owner = if let Some((nonce, required, _)) = &intent {
                SessionOwner::start_on_interface_with_intent(&interface, peer, binding, parameters, token.clone(), *nonce, *required)?
            } else { SessionOwner::start_on_interface(&interface, peer, binding, parameters, token.clone())? };
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            let queue = loop {
                token_check(&token)?;
                if let Some(queue) = owner.pcm_queue() { break Some(queue); }
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
                if let Err(error) = self.audio.start_pcm_sink(queue_ref.clone()) {
                    let _ = self.audio.stop_and_cleanup();
                    let _ = self.audio.restore_muted();
                    owner.stop();
                    return Err(error);
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
            *self.session.lock().await = Some(Session {
                generation: NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed),
                serial: "network".into(), transport: "lan".into(), reverse: String::new(), child: None,
                installed_receiver: false, output: session_output.into(), output_tail: Arc::new(Mutex::new(VecDeque::new())),
                network_owner: Some(owner), network_sink_queue: queue, network_binding_ref: Some(binding_ref.clone()),
            });
            let capabilities = self.session.lock().await.as_ref().and_then(|session| session.network_owner.as_ref())
                .and_then(|owner| owner.snapshot().capabilities);
            {
                let mut view = self.view.lock().expect("view lock");
                view.session_active = true;
                view.session_serial = None;
                view.session_receiver_binding = Some(binding_ref);
                view.session_transport = Some("lan".into());
                view.session_android_version = None;
                view.session_capabilities = capabilities;
                view.playback_diagnostics = None;
                view.network_recovery_seconds_remaining = None;
                view.output = session_output.into();
                view.state = if management_only { "managementOnly" } else { "phoneReady" }.into();
                view.message = if management_only { "Receiver 管理连接已认证，音频协议不兼容" } else { "手机接收已就绪" }.into();
                view.status_detail = if management_only { "P5-A 与管理能力查询已认证；未启动 BIND、START 或 PCM。请通过授权 ADB 或手动安装兼容版本。" }
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
        if result.is_err() { self.emit(app); }
        result
    }

    #[cfg(not(windows))]
    pub async fn start_network_audio(&self, _app: &AppHandle, _local_address: &str, _peer: &str,
        _bootstrap_serial: Option<&str>, _receiver_address: Option<&str>, _receiver_prefix: Option<u8>,
        _selected_binding_ref: Option<&str>) -> Result<AppSnapshot, String> {
        Err("Windows 网络音频接收仅在 Windows 上可用。".into())
    }

    fn start_operation(&self) -> (u64, CancellationToken) {
        begin_operation_slot(&self.operation_generation, &self.operation, false).expect("foreground operation always starts")
    }

    fn start_background_operation(&self) -> Option<(u64, CancellationToken)> {
        begin_operation_slot(&self.operation_generation, &self.operation, true)
    }

    fn cancel_active_operation(&self) {
        cancel_operation_slot(&self.operation_generation, &self.operation);
    }

    fn operation_is_current(&self, generation: u64, token: &CancellationToken) -> bool {
        self.operation_generation.load(Ordering::Acquire) == generation && !token.is_cancelled()
    }

    fn finish_operation(&self, generation: u64) {
        finish_operation_slot(&self.operation, generation);
    }

    fn start_qr_operation(&self) -> (u64, CancellationToken) {
        let mut current = self.qr_operation.lock().expect("qr operation lock");
        if let Some(previous) = current.as_ref() { previous.token.cancel(); }
        let generation = self.qr_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let token = CancellationToken::new();
        *current = Some(ActiveQrOperation { generation, token: token.clone() });
        (generation, token)
    }

    fn finish_qr_operation(&self, generation: u64) {
        let mut current = self.qr_operation.lock().expect("qr operation lock");
        if current.as_ref().is_some_and(|active| active.generation == generation) { *current = None; }
    }

    pub fn initialize(&self, app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        fs::create_dir_all(self.data_dir.join("logs"))?;
        self.load_settings();
        self.recover_mute_state();
        self.log("AnPod Tauri starting");
        let _ = app;
        Ok(())
    }

    pub fn snapshot(&self) -> AppSnapshot {
        let view = self.view.lock().expect("app view lock");
        let transport = view.settings.preferred_transport.to_ascii_lowercase();
        let can_disconnect = view.busy || view.session_active || (view.state == "error" && view.device_name.is_some());
        let can_change_modes = !view.busy && !can_disconnect;
        AppSnapshot {
            state: view.state.clone(),
            message: view.message.clone(),
            device_name: view.device_name.clone(),
            devices: view.devices.clone(),
            selected_serial: view.selected_serial.clone(),
            session_serial: view.session_serial.clone(),
            session_receiver_binding: view.session_receiver_binding.clone(),
            session_transport: view.session_transport.clone(),
            session_connection_transport: view.session_transport.as_deref().and_then(session_connection_transport).map(str::to_owned),
            session_wireless_protocol: view.session_transport.as_deref().and_then(session_wireless_protocol).map(str::to_owned),
            session_capabilities: view.session_capabilities.clone(),
            playback_diagnostics: view.playback_diagnostics.clone(),
            network_recovery_seconds_remaining: view.network_recovery_seconds_remaining,
            wireless_connection_offers: self.wireless_connection_offers(),
            session_android_version: view.session_android_version.clone(),
            connection_transport: selected_connection_transport(&view.settings.preferred_transport).into(),
            transport,
            output: view.output.clone(),
            settings: view.settings.clone(),
            busy: view.busy,
            can_disconnect,
            can_change_modes,
            status_detail: view.status_detail.clone(),
            audio_bitrate_bps: view.audio_bitrate_bps,
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
        } else if let Ok(bytes) = self.audio.sent_pcm_bytes() { bytes } else { return; };
        let now = Instant::now();
        let changed = {
            let mut previous = self.pcm_rate_sample.lock().expect("PCM rate sample lock");
            let mut view = self.view.lock().expect("app view lock");
            commit_pcm_bitrate_sample(Some(generation), generation, &mut previous,
                &mut view.audio_bitrate_bps, bytes, now)
        };
        drop(session);
        if changed { self.emit(app); }
    }

    async fn clear_audio_bitrate_if_session(&self, expected_session: Option<&str>, app: &AppHandle) {
        let session = self.session.lock().await;
        if expected_session.is_some() || session.is_some() { return; }
        *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
        let changed = {
            let mut view = self.view.lock().expect("app view lock");
            if view.audio_bitrate_bps.is_none() { false }
            else { view.audio_bitrate_bps = None; true }
        };
        drop(session);
        if changed { self.emit(app); }
    }

    fn emit(&self, app: &AppHandle) {
        let _ = app.emit("anpod://snapshot", self.snapshot());
    }

    async fn update_usb_playback_status(&self, generation: u64, serial: &str, phone_output: bool,
        audio_active: bool, diagnostics: Option<PlaybackDiagnostics>, format: Option<PcmFormat>) -> bool {
        let session = self.session.lock().await;
        let Some(_active) = session.as_ref().filter(|active| active.generation == generation
            && active.serial == serial && active.network_owner.is_none() && active.output == "phone") else { return false; };
        if !phone_output { return false; }
        let diagnostics = {
            let mut sample = self.receiver_health_sample.lock().expect("receiver health sample lock");
            let (diagnostics, next) = merge_usb_playback_diagnostics(
                diagnostics.unwrap_or_default(), format, generation, *sample);
            *sample = next;
            diagnostics
        };
        let mut view = self.view.lock().expect("app view lock");
        let (state, message, detail) = if audio_active {
            ("streaming", "正在手机播放", "Receiver 已确认 AudioTrack 正在播放。")
        } else {
            ("phoneReady", "手机接收已就绪", "手机接收端已就绪；电脑有声音发送后将开始播放。")
        };
        let changed = view.state != state || view.message != message || view.status_detail != detail
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

    fn set_state(&self, app: &AppHandle, state: &str, message: impl Into<String>, device: Option<String>) {
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
            .unwrap_or_else(|| {
                let mut settings = AppSettings::default();
                if std::env::var("ANPOD_AUDIO_BACKEND").is_ok_and(|value| value == "installed-receiver") {
                    settings.audio_backend = "receiver".into();
                }
                settings
            });
        self.view.lock().expect("app view lock").settings = settings;
    }

    fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        let path = self.data_dir.join("settings.json");
        let temp = self.data_dir.join("settings.json.tmp");
        let bytes = serde_json::to_vec_pretty(&normalize_settings(settings.clone())).map_err(|e| e.to_string())?;
        fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        atomic_replace(&temp, &path).map_err(|e| e.to_string())
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

    fn recover_mute_state(&self) {
        let marker = self.data_dir.join("muted-endpoints.txt");
        if let Ok(contents) = fs::read_to_string(&marker) {
            let ids: Vec<String> = contents.lines().filter(|s| !s.is_empty()).map(str::to_owned).collect();
            if self.audio.restore_ids(ids.clone()).is_ok() {
                let _ = fs::remove_file(marker);
                if !ids.is_empty() { self.log("Recovered speaker mute state left by an unclean exit"); }
            }
        }
    }

    pub async fn refresh_devices(&self) -> Result<AppSnapshot, String> {
        self.refresh_devices_with_cancel(None).await
    }

    async fn refresh_devices_with_cancel(&self, cancel: Option<&CancellationToken>) -> Result<AppSnapshot, String> {
        let result = adb_run(&self.adb_path, None, &["devices", "-l"], Duration::from_secs(8), cancel).await?;
        if !result.success { return Err("无法读取 ADB 设备列表；请检查 platform-tools。".into()); }
        let mut devices = parse_devices(&result.stdout);
        for device in &mut devices {
            if device.transport == "unknown" || device.model.is_empty() || device.manufacturer.is_empty() || device.android_version.is_empty() {
                let props = adb_run(&self.adb_path, Some(&device.serial), &["shell", "getprop"], Duration::from_secs(4), cancel).await;
                if let Some(props) = props.ok().filter(|r| r.success) {
                    let map = parse_getprop(&props.stdout);
                    if device.model.is_empty() { device.model = map.get("ro.product.model").cloned().unwrap_or_default(); }
                    if device.manufacturer.is_empty() { device.manufacturer = map.get("ro.product.manufacturer").cloned().unwrap_or_default(); }
                    if device.android_version.is_empty() { device.android_version = map.get("ro.build.version.release").cloned().unwrap_or_default(); }
                    if device.api_level.is_empty() { device.api_level = map.get("ro.build.version.sdk").cloned().unwrap_or_default(); }
                    device.physical_id = map.get("ro.serialno").filter(|id| !id.is_empty()).cloned();
                    if device.transport == "unknown" && map.get("sys.usb.state").is_some_and(|state| state.split(',').any(|part| part.trim().eq_ignore_ascii_case("adb"))) {
                        device.transport = "usb".into();
                        device.connectable = device.state == "device";
                        device.status_text = if device.connectable { "可连接" } else { device.status_text.as_str() }.into();
                    }
                    device.display_name = if device.model.is_empty() { device.serial.clone() } else if device.manufacturer.is_empty() { device.model.clone() } else { format!("{} {}", device.manufacturer, device.model) };
                }
            }
        }
        {
            let mut view = self.view.lock().expect("app view lock");
            let preferred = view.settings.preferred_transport.to_ascii_lowercase();
            let previous = view.selected_serial.clone().or_else(|| view.settings.last_device_id.clone());
            view.devices = devices;
            view.selected_serial = choose_selected_serial(view.session_serial.clone(), previous, &view.devices, &preferred);
            if view.state == "noDevice" || view.state == "ready" || view.state == "unauthorized" {
                let selected = view.devices.iter().find(|d| Some(&d.serial) == view.selected_serial.as_ref());
                view.state = if selected.is_some_and(|d| d.connectable) { "ready".into() }
                    else if view.devices.iter().any(|d| d.state == "unauthorized") { "unauthorized".into() }
                    else { "noDevice".into() };
                view.message = if view.state == "ready" { "准备就绪".into() } else if view.state == "unauthorized" { "请在手机上允许 USB 调试授权".into() } else { "尚未连接手机".into() };
                view.status_detail = if view.state == "ready" { "已发现可连接手机。选择输出卡片即可连接。".into() } else { "连接 USB 手机或切换到 Wi-Fi 添加手机。".into() };
            }
        }
        Ok(self.snapshot())
    }

    #[cfg(windows)]
    async fn connect_pending_wireless_offer(&self, app: &AppHandle) -> bool {
        {
            let view = self.view.lock().expect("app view lock");
            if view.session_active || view.busy || !view.settings.preferred_transport.eq_ignore_ascii_case("wifi")
                || view.settings.wireless_protocol != "udp" { return false; }
        }
        let snapshot = self.wireless_connection_offers();
        // Several pending targets require explicit selection; hints never establish identity.
        if !snapshot.active || snapshot.offers.len() != 1 { return false; }
        let candidate = {
            let listener = self.host_offer_listener.lock().expect("offer listener lock");
            listener.as_ref().and_then(|current| current.automatic_binding.clone()
                .zip(snapshot.interface_address).map(|(binding, address)| (binding, address, current.cancellation())))
        };
        let Some((binding, address, cancellation)) = candidate else { return false; };
        let Some(reservation) = self.start_background_operation() else { return false; };
        let offer = self.host_offer_listener.lock().expect("offer listener lock").as_ref()
            .and_then(|current| if cancellation.is_cancelled() { None } else { current.take_offer(&snapshot.offers[0].id) });
        let Some(offer) = offer else { self.finish_operation(reservation.0); return false; };
        let generation = reservation.0;
        let result = self.start_network_audio_inner(app, &address.to_string(), &offer.endpoint.to_string(),
            None, None, None, Some(&binding), Some((offer.nonce, true, cancellation)), Some(reservation)).await;
        if let Err(error) = result {
            self.log(&format!("Wireless reception intent failed: {error}"));
            let mut view = self.view.lock().expect("app view lock");
            if !view.session_active && self.operation_generation.load(Ordering::Acquire) == generation {
                view.state = "error".into();
                view.device_name = None;
                view.message = "手机自动连接未完成".into();
                view.status_detail = error;
            }
            drop(view);
            self.emit(app);
        }
        true
    }

    pub async fn monitor_once(&self, app: &AppHandle) {
        #[cfg(windows)]
        if self.connect_pending_wireless_offer(app).await { return; }
        let Ok(_gate) = self.gate.try_lock() else { return; };
        if self.operation.lock().expect("operation lock").as_ref().is_some_and(|operation| !operation.token.is_cancelled()) { return; }
        if self.monitor_network_session(app).await { return; }
        let before = self.snapshot();
        let snapshot = match self.refresh_devices().await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => { self.log(&format!("ADB device scan failed during monitoring: {error}")); None }
        };
        if let Some(snapshot) = &snapshot {
            let changed = before.devices.iter().map(|d| (&d.serial, &d.state, &d.transport)).ne(snapshot.devices.iter().map(|d| (&d.serial, &d.state, &d.transport))) || before.selected_serial != snapshot.selected_serial || before.state != snapshot.state;
            if changed { self.emit(app); }
            if snapshot.busy { return; }
        }
        let session = self.session.lock().await.as_mut().map(|session| {
            let exited = session.child.as_mut().and_then(|child| child.try_wait().ok().flatten()).is_some();
            (session.generation, session.serial.clone(), session.transport.clone(), session.output.clone(), session.output_tail.clone(), session.reverse.clone(), session.installed_receiver, exited)
        });
        if let Some((session_generation, serial, transport, output, tail, session_key, installed_receiver, exited)) = session {
            let device_present = snapshot.as_ref().map(|snapshot| snapshot.devices.iter().any(|device| device.serial == serial && device.state == "device"));
            let connected = self.audio.is_connected().unwrap_or(false);
            let capture_error = self.audio.take_last_error().ok().flatten();
            if session_is_lost(device_present, exited, connected) || capture_error.is_some() {
                let detail = if let Some(error) = capture_error {
                    error
                } else if exited {
                    let output = safe_android_tail(&tail.lock().expect("output log lock").iter().cloned().collect::<Vec<_>>());
                    if output.is_empty() { "手机端音频服务已停止".to_string() } else { format!("手机端音频服务已停止：\n{output}") }
                } else if device_present == Some(false) { "手机连接已断开".into() } else { "手机端音频连接已中断".into() };
                self.log(&detail);
                self.recover_session_locked(app, serial, transport, output == "phone", session_key, detail).await;
                return;
            }
            if receiver_health_needed(installed_receiver, device_present, exited, connected) {
                let Some((generation, token)) = self.start_background_operation() else { return; };
                let result = receiver::health(&self.adb_path, &serial, &self.receiver_apk_path,
                    &self.receiver_metadata_path, &token).await;
                let cancelled = !self.operation_is_current(generation, &token);
                self.finish_operation(generation);
                if cancelled || !self.operation_is_current(generation, &token) { return; }
                let receiver_failure = match result {
                    Ok(status) => match status.monitoring_error() {
                        Some(error) => Some(error),
                        None => {
                            let format = self.audio.pcm_format().ok();
                            if self.update_usb_playback_status(session_generation, &serial, output == "phone",
                                status.audio_active, status.playback_diagnostics, format).await { self.emit(app); }
                            None
                        }
                    },
                    Err(error) => Some(error),
                };
                if let Some(error) = receiver_failure {
                    let detail = format!("Receiver 健康检查失败：{error}");
                    self.log(&detail);
                    self.recover_session_locked(app, serial, transport, output == "phone", session_key, detail).await;
                    return;
                }
            }
            return;
        }

        let Some(snapshot) = snapshot else { return; };
        let settings = snapshot.settings.clone();
        let suppressed = self.suppressed_auto_connect.lock().expect("auto-connect suppression lock").clone();
        if let Some(suppressed) = suppressed {
            if snapshot.devices.iter().all(|device| device.serial != suppressed) {
                *self.suppressed_auto_connect.lock().expect("auto-connect suppression lock") = None;
            }
        }
        if snapshot.settings.last_wireless_endpoint.as_ref().is_some_and(|endpoint| snapshot.devices.iter().any(|device| &device.serial == endpoint)) {
            self.wireless_probe_attempted.store(false, Ordering::Release);
        }
        if !settings.auto_connect_last_device || !selected_transport_allows_auto_adb(&settings) { return; }
        let target = settings.last_device_id.as_deref();
        let suppressed = self.suppressed_auto_connect.lock().expect("auto-connect suppression lock").clone();
        let failed = self.failed_auto_connect.lock().expect("auto-connect failure lock").clone();
        if failed.as_ref().is_some_and(|serial| snapshot.devices.iter().all(|device| &device.serial != serial)) {
            *self.failed_auto_connect.lock().expect("auto-connect failure lock") = None;
        }
        if let Some(device) = snapshot.devices.iter().find(|device| Some(device.serial.as_str()) == target && Some(device.serial.as_str()) != suppressed.as_deref() && Some(device.serial.as_str()) != failed.as_deref() && device.connectable && device.transport == snapshot.transport) {
            if !self.recovering.swap(true, Ordering::AcqRel) {
                let serial = device.serial.clone();
                let Some((generation, token)) = self.start_background_operation() else { self.recovering.store(false, Ordering::Release); return; };
                let result = self.connect_locked(app, &serial, generation, &token, false).await;
                self.finish_operation(generation);
                if let Ok(result) = result {
                    if !result.success { *self.failed_auto_connect.lock().expect("auto-connect failure lock") = Some(serial); self.log(&format!("Auto-connect failed: {}", result.message)); }
                } else if let Err(error) = result { self.log(&format!("Auto-connect failed: {error}")); }
                self.recovering.store(false, Ordering::Release);
            }
            return;
        }
        if snapshot.transport == "wifi" && wireless_adb_enabled(&settings) && settings.auto_reconnect_wifi {
            if let Some(endpoint) = settings.last_wireless_endpoint.filter(|endpoint| endpoint.contains(':')) {
                if self.wireless_probe_attempted.swap(true, Ordering::AcqRel) { return; }
                if checked_adb(&self.adb_path, None, &["connect", &endpoint], Duration::from_secs(8), None).await.is_ok() {
                    let _ = self.refresh_devices().await;
                }
            }
        }
    }

    async fn monitor_network_session(&self, app: &AppHandle) -> bool {
        let mut session = self.session.lock().await;
        let Some(active) = session.as_mut().filter(|session| session.network_owner.is_some()) else { return false; };
        let owner = active.network_owner.as_ref().expect("network session owner");
        let snapshot = owner.snapshot();
        let needs_bootstrap = snapshot.state == SessionLifecycleState::Stopped;
        let management_only = snapshot.state == SessionLifecycleState::Authenticated
            && snapshot.capabilities.as_ref().is_some_and(|info| info.management_compatible && !info.compatible);
        let queue = owner.pcm_queue();
        let mut failure = self.audio.take_last_error().ok().flatten();
        if failure.is_none() && needs_bootstrap {
            failure = Some(snapshot.last_error.clone().unwrap_or_else(|| "Receiver session stopped".into()));
        }
        if failure.is_none() {
            if management_only {
                active.network_sink_queue = None;
                if let Err(error) = self.audio.stop_pcm_sink() { failure = Some(error); }
                if let Err(error) = self.audio.restore_muted() { failure.get_or_insert(error); }
            } else if active.output == "phone" {
                if let Some(queue) = queue.as_ref() {
                    if pcm_queue_identity_changed(active.network_sink_queue.as_ref(), Some(queue)) {
                        if let Err(error) = self.audio.start_pcm_sink(queue.clone()) { failure = Some(error); }
                        else if let Err(error) = self.audio.mute_active() {
                            let _ = self.audio.stop_pcm_sink();
                            let _ = self.audio.restore_muted();
                            failure = Some(error);
                        } else { active.network_sink_queue = Some(queue.clone()); }
                    }
                } else if active.network_sink_queue.take().is_some() {
                    if let Err(error) = self.audio.stop_pcm_sink() { failure = Some(error); }
                    if let Err(error) = self.audio.restore_muted() { failure = Some(error); }
                }
            } else if active.network_sink_queue.take().is_some() {
                if let Err(error) = self.audio.stop_pcm_sink() { failure = Some(error); }
                if let Err(error) = self.audio.restore_muted() { failure = Some(error); }
            }
        }
        if let Some(error) = failure {
            let mut cleanup_errors = Vec::new();
            if let Err(stop_error) = self.audio.stop_pcm_sink() { cleanup_errors.push(stop_error); }
            if let Err(restore_error) = self.audio.restore_muted() { cleanup_errors.push(restore_error); }
            if let Some(mut owner) = active.network_owner.take() { owner.stop(); }
            active.network_sink_queue = None;
            active.output = "computer".into();
            let mut detail = if cleanup_errors.is_empty() { error } else { format!("{error}；{}", cleanup_errors.join("；")) };
            if needs_bootstrap { detail.push_str("；请重新通过授权 USB/bootstrap 启动 Receiver"); }
            *session = None;
            {
                let mut view = self.view.lock().expect("app view lock");
                view.session_active = false;
                view.session_serial = None;
                view.session_receiver_binding = None;
                view.session_transport = None;
                view.session_capabilities = None;
                view.playback_diagnostics = None;
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
        let state = if management_only { "managementOnly" }
            else if active.output == "computer" { "connected" }
            else if queue.is_none() || snapshot.state == SessionLifecycleState::Reconnecting { "reconnecting" }
            else { match snapshot.state {
                SessionLifecycleState::Streaming => "streaming",
                SessionLifecycleState::Paused => "paused",
                _ => "phoneReady",
            }};
        let (message, detail) = if let Some(error) = failure {
            ("网络音频切换失败".to_string(), format!("本机 PCM sink 操作失败：{error}"))
        } else if state == "reconnecting" {
            ("正在重新连接手机".into(), "网络接收暂时中断；电脑声音已恢复，正在尝试重新认证。".into())
        } else if management_only {
            ("Receiver 管理连接已认证，音频协议不兼容".into(), "已认证管理查询保持在线；未启动 BIND、START 或 PCM。请通过授权 ADB 或手动安装兼容版本。".into())
        } else if active.output == "computer" {
            ("已连接 · 电脑播放".into(), "网络会话保持暂停；电脑声音已恢复到本机扬声器。".into())
        } else if state == "streaming" {
            ("正在手机播放".into(), "Receiver 已确认 AudioTrack 收到 PCM 音频。".into())
        } else if state == "paused" {
            ("手机播放已暂停".into(), "P5-C 会话已暂停。".into())
        } else {
            ("手机接收已就绪".into(), "P5-A 已认证；等待 Receiver AudioTrack 收到 PCM 音频。".into())
        };
        let changed = {
            let mut view = self.view.lock().expect("app view lock");
            let changed = view.state != state || view.message != message || view.status_detail != detail
                || view.playback_diagnostics != snapshot.playback_diagnostics
                || view.network_recovery_seconds_remaining != snapshot.recovery_seconds_remaining;
            view.state = state.into();
            view.message = message;
            view.status_detail = detail;
            view.output = active.output.clone();
            view.session_receiver_binding = active.network_binding_ref.clone();
            view.session_capabilities = snapshot.capabilities.clone();
            view.playback_diagnostics = snapshot.playback_diagnostics.clone();
            view.network_recovery_seconds_remaining = snapshot.recovery_seconds_remaining;
            changed
        };
        drop(session);
        if changed { self.emit(app); }
        true
    }

    async fn recover_session_locked(&self, app: &AppHandle, serial: String, transport: String, was_playing: bool, session_key: String, reason: String) {
        if self.recovering.swap(true, Ordering::AcqRel) { return; }
        let same_session = self.session.lock().await.as_ref()
            .is_some_and(|session| session.serial == serial && session.reverse == session_key);
        if !same_session {
            self.recovering.store(false, Ordering::Release);
            return;
        }
        // monitor_once holds gate and already confirmed session loss. A socket
        // still reporting connected must not override a missing ADB device.
        let device_name = self.snapshot().device_name;
        if let Err(error) = self.teardown(Some(app)).await {
            self.set_state(app, "error", format!("{reason}；清理连接失败：{error}"), device_name);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        let suppressed = self.suppressed_auto_connect.lock().expect("auto-connect suppression lock")
            .as_deref() == Some(serial.as_str());
        if suppressed {
            self.set_state(app, "error", reason, device_name);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        let settings = self.snapshot().settings;
        if !settings.auto_connect_last_device || (transport == "wifi"
            && (!settings.auto_reconnect_wifi || !wireless_adb_enabled(&settings))) {
            self.set_state(app, "error", reason, device_name);
            self.recovering.store(false, Ordering::Release);
            return;
        }
        let Some((generation, token)) = self.start_background_operation() else {
            self.recovering.store(false, Ordering::Release);
            return;
        };
        for attempt in 0..3 {
            if token.is_cancelled() { break; }
            self.set_state(app, "reconnecting", format!("{reason}，正在重连（{}/{})…", attempt + 1, 3), device_name.clone());
            tokio::select! { _ = token.cancelled() => break, _ = tokio::time::sleep(Duration::from_millis([1000, 2000, 5000][attempt])) => {} }
            if transport == "wifi" {
                let endpoint = settings.last_wireless_endpoint.as_deref().unwrap_or(&serial);
                let _ = checked_adb(&self.adb_path, None, &["connect", endpoint], Duration::from_secs(8), Some(&token)).await;
            }
            if token.is_cancelled() { break; }
            let _ = self.refresh_devices().await;
            if let Some(device) = self.snapshot().devices.iter().find(|device| device.serial == serial && device.connectable).cloned() {
                if let Ok(result) = self.connect_locked(app, &device.serial, generation, &token, was_playing).await {
                    if result.success {
                        if !was_playing { let _ = self.set_output_locked("computer", &token).await; self.emit(app); }
                        self.finish_operation(generation);
                        self.recovering.store(false, Ordering::Release);
                        return;
                    }
                }
                let _ = self.teardown(Some(app)).await;
            }
        }
        self.finish_operation(generation);
        if !token.is_cancelled() {
            *self.failed_auto_connect.lock().expect("auto-connect failure lock") = Some(serial);
            self.set_state(app, "error", "重连失败，请手动连接。", device_name);
        }
        self.recovering.store(false, Ordering::Release);
    }

    pub async fn connect_device(&self, app: &AppHandle, serial: String) -> Result<OperationResult, String> {
        self.connect_device_mode(app, serial, true).await
    }

    pub async fn connect_device_only(&self, app: &AppHandle, serial: String) -> Result<OperationResult, String> {
        self.connect_device_mode(app, serial, false).await
    }

    pub async fn switch_device(&self, app: &AppHandle, serial: String) -> Result<OperationResult, String> {
        let (generation, token) = self.start_operation();
        let _guard = self.gate.lock().await;
        let result = async {
            if !self.operation_is_current(generation, &token) {
                return Ok(OperationResult { success: false, message: "设备切换已取消。".into(), snapshot: self.snapshot() });
            }
            if self.session.lock().await.as_ref().is_some_and(|session| session.network_owner.is_some()) {
                return Err("当前是 LAN 直连会话；请先断开后再切换手机。".into());
            }
            let old_session = self.session.lock().await.as_ref().map(|session| (session.serial.clone(), session.output == "phone"));
            if old_session.as_ref().is_some_and(|(old_serial, _)| old_serial == &serial) {
                return Ok(OperationResult { success: true, message: "当前已连接这部手机。".into(), snapshot: self.snapshot() });
            }
            let (old_selected, device_available, cleanup_pending) = {
                let view = self.view.lock().expect("app view lock");
                let preferred = view.settings.preferred_transport.to_ascii_lowercase();
                (
                    view.selected_serial.clone(),
                    view.devices.iter().any(|device| device.serial == serial && device.connectable && (device.transport == preferred || device.transport == "unknown")),
                    view.state == "error" && view.device_name.is_some() && !view.session_active,
                )
            };
            if !device_available { return Err("此手机当前不可连接，请刷新设备列表。".into()); }
            if old_session.is_some() || cleanup_pending {
                if let Err(error) = self.teardown(Some(app)).await {
                    let mut view = self.view.lock().expect("app view lock");
                    view.selected_serial = old_selected;
                    view.state = "error".into();
                    view.message = format!("无法切换手机：{error}");
                    view.status_detail = "连接清理未完成；请断开连接后重试。".into();
                    drop(view);
                    self.emit(app);
                    return Ok(OperationResult { success: false, message: format!("无法切换手机：{error}"), snapshot: self.snapshot() });
                }
            }
            let play_on_phone = old_session.as_ref().is_some_and(|(_, play)| *play);
            let switched = match self.connect_locked(app, &serial, generation, &token, play_on_phone).await {
                Ok(result) => result,
                Err(error) => OperationResult { success: false, message: error, snapshot: self.snapshot() },
            };
            if switched.success || old_session.is_none() { return Ok(switched); }
            let failure = switched.message;
            let (old_serial, _) = old_session.expect("old session checked above");
            if token.is_cancelled() {
                let mut view = self.view.lock().expect("app view lock");
                view.selected_serial = old_selected;
                drop(view);
                return Ok(OperationResult { success: false, message: "设备切换已取消，连接已停止，电脑声音已恢复。".into(), snapshot: self.snapshot() });
            }
            let restored = self.connect_locked(app, &old_serial, generation, &token, play_on_phone).await;
            Ok(match restored {
                Ok(restored) if restored.success => OperationResult {
                    success: false,
                    message: if token.is_cancelled() { "设备切换已取消，原手机连接与输出已恢复。".into() } else { format!("切换失败，已恢复原手机连接：{failure}") },
                    snapshot: restored.snapshot,
                },
                Ok(restored) => OperationResult {
                    success: false,
                    message: format!("切换失败：{failure}；恢复原手机也失败：{}", restored.message),
                    snapshot: restored.snapshot,
                },
                Err(error) => OperationResult { success: false, message: format!("切换失败：{failure}；恢复原手机也失败：{error}"), snapshot: self.snapshot() },
            })
        }.await;
        self.finish_operation(generation);
        result
    }

    async fn connect_device_mode(&self, app: &AppHandle, serial: String, play_on_phone: bool) -> Result<OperationResult, String> {
        *self.failed_auto_connect.lock().expect("auto-connect failure lock") = None;
        let (generation, token) = self.start_operation();
        let _guard = self.gate.lock().await;
        let result = if !self.operation_is_current(generation, &token) {
            Ok(OperationResult { success: false, message: "连接已取消。".into(), snapshot: self.snapshot() })
        } else if self.session.lock().await.is_some() {
            Ok(OperationResult { success: false, message: "已有手机连接；请从设备卡片确认后切换。".into(), snapshot: self.snapshot() })
        } else {
            self.connect_locked(app, &serial, generation, &token, play_on_phone).await
        };
        self.finish_operation(generation);
        result
    }

    async fn connect_locked(&self, app: &AppHandle, serial: &str, generation: u64, token: &CancellationToken, play_on_phone: bool) -> Result<OperationResult, String> {
        if !self.operation_is_current(generation, token) {
            return Ok(OperationResult { success: false, message: "连接已取消。".into(), snapshot: self.snapshot() });
        }
        let device = {
            let view = self.view.lock().expect("app view lock");
            view.devices.iter().find(|d| d.serial == serial && d.connectable).cloned()
                .ok_or_else(|| "此手机当前不可连接，请刷新设备列表。".to_string())?
        };
        self.view.lock().expect("view lock").selected_serial = Some(device.serial.clone());
        self.set_state(app, "connecting", "正在建立音频连接…", Some(device.display_name.clone()));
        let result = self.connect_inner(app, &device, token, play_on_phone).await;
        if !self.operation_is_current(generation, token) {
            let cleanup = self.teardown(Some(app)).await;
            let message = cleanup.as_ref().err().map_or_else(|| "连接已取消。".into(), |error| format!("连接已取消，但清理连接失败：{error}"));
            let idle_state = if cleanup.is_err() { "error" } else if self.view.lock().expect("view lock").devices.is_empty() { "noDevice" } else { "ready" };
            self.set_state(app, idle_state, &message, None);
            self.view.lock().expect("view lock").output = "computer".into();
            self.emit(app);
            return Ok(OperationResult { success: false, message, snapshot: self.snapshot() });
        }
        match result {
            Ok(message) => Ok(OperationResult { success: true, message, snapshot: self.snapshot() }),
            Err(error) => {
                let cleanup_error = self.teardown(Some(app)).await.err();
                let message = cleanup_error.map_or(error.clone(), |cleanup| format!("{error}；清理连接失败：{cleanup}"));
                self.set_state(app, "error", &message, Some(device.display_name.clone()));
                Ok(OperationResult { success: false, message, snapshot: self.snapshot() })
            }
        }
    }

    async fn connect_inner(&self, app: &AppHandle, device: &Device, token: &CancellationToken, play_on_phone: bool) -> Result<String, String> {
        let serial = device.serial.as_str();
        token_check(&token)?;
        let installed_receiver = self.view.lock().expect("view lock").settings.audio_backend == "receiver";
        self.audio.initialize()?;
        self.audio.set_streaming(false)?;
        let port = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?.local_addr().map_err(|e| e.to_string())?.port();
        let generation = NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let reverse = format!("anpod_{generation}");
        let token_text = uuid_token()?;
        self.audio.listen(port, &token_text)?;
        let remote = format!("localabstract:{reverse}");
        let port_arg = format!("tcp:{port}");
        checked_adb(&self.adb_path, Some(serial), &["reverse", &remote, &port_arg], Duration::from_secs(5), Some(&token)).await?;
        *self.session.lock().await = Some(Session { generation: NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed), serial: serial.into(), transport: device.transport.clone(), reverse: reverse.clone(), child: None, installed_receiver, output: "computer".into(), output_tail: Arc::new(Mutex::new(VecDeque::new())), network_owner: None, network_sink_queue: None, network_binding_ref: None });
        {
            let mut view = self.view.lock().expect("view lock");
            view.session_active = true;
            view.session_serial = Some(device.serial.clone());
            view.session_receiver_binding = None;
            view.session_transport = Some(device.transport.clone());
            view.session_capabilities = None;
            view.playback_diagnostics = None;
            view.network_recovery_seconds_remaining = None;
            view.session_android_version = (!device.android_version.is_empty()).then(|| device.android_version.clone());
            view.audio_bitrate_bps = Some(0);
        }
        *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
        let mode = self.view.lock().expect("view lock").settings.playback_mode.to_ascii_lowercase();
        if installed_receiver {
            let playback_mode = if mode == "lowlatency" || mode == "low_latency" { "lowlatency" } else { "balanced" };
            receiver::start_audio(&self.adb_path, serial, &self.receiver_apk_path, &self.receiver_metadata_path,
                &reverse, &token_text, playback_mode, token).await?;
        } else {
            checked_adb(&self.adb_path, Some(serial), &["push", self.server_path.to_string_lossy().as_ref(), "/data/local/tmp/anpod_server"], Duration::from_secs(15), Some(&token)).await?;
            let mut command = Command::new(&self.adb_path);
            command.args(["-s", serial, "shell", "app_process", "-Djava.class.path=/data/local/tmp/anpod_server", "/data/local/tmp", "com.anpod.server.Main"])
                .arg(format!("socketName={reverse}"))
                .arg(format!("connectCode={token_text}"))
                .arg(format!("playbackMode={}", if mode == "lowlatency" || mode == "low_latency" { "low-latency" } else { "balanced" }))
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let mut child = command.spawn().map_err(|_| "无法启动手机端音频服务。".to_string())?;
            let output_tail = self.session.lock().await.as_ref().expect("session created before server").output_tail.clone();
            let log_path = self.data_dir.join("logs").join("anpod.log");
            if let Some(stdout) = child.stdout.take() { tokio::spawn(capture_child_output(stdout, "stdout", output_tail.clone(), log_path.clone())); }
            if let Some(stderr) = child.stderr.take() { tokio::spawn(capture_child_output(stderr, "stderr", output_tail.clone(), log_path)); }
            if let Some(session) = self.session.lock().await.as_mut() { session.child = Some(child); }
        }
        token_check(&token)?;
        let handshake = self.audio.wait_handshake(&token_text, Duration::from_secs(10), &token).await?;
        token_check(&token)?;
        if handshake != token_text { return Err("握手校验失败，已拒绝连接。".into()); }
        if !installed_receiver && self.session.lock().await.as_mut().and_then(|session| session.child.as_mut()).is_some_and(|child| child.try_wait().map_or(true, |status| status.is_some())) { return Err("手机端音频服务已停止。".into()); }
        if !self.audio.is_connected()? { return Err("手机端音频连接已中断。".into()); }
        token_check(&token)?;
        if play_on_phone && !installed_receiver { self.audio.mute_active()?; }
        self.audio.set_streaming(play_on_phone)?;
        token_check(&token)?;
        self.audio.start()?;
        token_check(&token)?;
        if play_on_phone && installed_receiver {
            self.audio.mute_active()?;
        }
        let output = if play_on_phone { "phone" } else { "computer" }.to_string();
        if let Some(session) = self.session.lock().await.as_mut() { session.output = output.clone(); }
        {
            let mut view = self.view.lock().expect("view lock");
            view.output = output;
            view.settings.last_device_id = Some(serial.into());
            view.busy = false;
            view.settings.last_wireless_endpoint = if device.transport == "wifi" { Some(serial.into()) } else { view.settings.last_wireless_endpoint.clone() };
            view.status_detail = if play_on_phone && installed_receiver {
                "手机接收端已就绪；电脑有声音发送后将开始播放。".into()
            } else if play_on_phone {
                format!("电脑声音正在通过 {} 传到手机。", device.transport.to_uppercase())
            } else { "手机连接保持中；声音继续从电脑播放。".into() };
            let settings = view.settings.clone();
            drop(view);
            let _ = self.save_settings(&settings);
        }
        let state = if play_on_phone && installed_receiver { "phoneReady" } else if play_on_phone { "streaming" } else { "connected" };
        let message = if state == "phoneReady" { "手机接收已就绪" } else if play_on_phone { "正在手机播放" } else { "已连接 · 电脑播放" };
        self.set_state(app, state, message, Some(device.display_name.clone()));
        self.log("Audio session connected");
        Ok(if state == "phoneReady" { "手机接收端已就绪，等待音频数据。".into() } else { "已连接并开始播放。".into() })
    }

    pub async fn disconnect(&self, app: &AppHandle) -> Result<OperationResult, String> {
        self.cancel_active_operation();
        self.cancel_qr_pairing();
        let serial = self.session.lock().await.as_ref().map(|session| session.serial.clone())
            .or_else(|| self.view.lock().expect("app view lock").selected_serial.clone());
        *self.suppressed_auto_connect.lock().expect("auto-connect suppression lock") = serial;
        let _guard = self.gate.lock().await;
        let cleanup = self.teardown(Some(app)).await;
        let mut view = self.view.lock().expect("view lock");
        view.state = if cleanup.is_err() { "error".into() } else if view.devices.is_empty() { "noDevice".into() } else { "ready".into() };
        view.message = cleanup.as_ref().err().map_or_else(|| "已断开连接".into(), |error| format!("已断开，但清理连接失败：{error}"));
        if cleanup.is_ok() { view.device_name = None; }
        view.output = "computer".into();
        view.busy = false;
        view.status_detail = if cleanup.is_ok() || cleanup.as_ref().err().is_some_and(|error| !has_restore_failure(error)) {
            "电脑声音已恢复。选择手机卡片可重新连接。".into()
        } else {
            "电脑声音恢复待处理；可从托盘菜单选择“断开连接”重试，或重新启动 AnPod。".into()
        };
        drop(view);
        self.log("Audio session disconnected by user");
        self.emit(app);
        match cleanup {
            Ok(()) => Ok(OperationResult { success: true, message: "已断开连接。".into(), snapshot: self.snapshot() }),
            Err(error) => Ok(OperationResult { success: false, message: format!("已断开，但清理连接失败：{error}"), snapshot: self.snapshot() }),
        }
    }

    pub fn select_device(&self, serial: &str) -> Result<AppSnapshot, String> {
        let mut view = self.view.lock().expect("view lock");
        if view.busy {
            return Err("连接期间无法更换手机。请先断开连接。".into());
        }
        if let Some(active) = view.session_serial.as_deref() {
            if active == serial { drop(view); return Ok(self.snapshot()); }
            return Err("切换已连接的手机需要先确认中断当前连接。".into());
        }
        if view.state == "error" && view.device_name.is_some() {
            return Err("连接清理待完成；请先断开连接后再选择手机。".into());
        }
        if !view.devices.iter().any(|device| device.serial == serial && (device.transport == view.settings.preferred_transport.to_ascii_lowercase() || device.transport == "unknown")) {
            return Err("所选手机不在当前连接方式的设备列表中。".into());
        }
        if view.selected_serial.as_deref() == Some(serial) { drop(view); return Ok(self.snapshot()); }
        view.selected_serial = Some(serial.to_owned());
        drop(view);
        Ok(self.snapshot())
    }

    pub async fn reconfigure_session(&self, app: &AppHandle, kind: &str, value: &str) -> Result<AppSnapshot, String> {
        let selection = reconfigure_selection(kind, value, &self.snapshot().settings)?;
        let (generation, token) = self.start_operation();
        let result = self.reconfigure_locked(app, selection, kind, value, generation, &token).await;
        self.finish_operation(generation);
        result
    }

    async fn reconfigure_locked(&self, app: &AppHandle, selection: ReconfigureSelection, kind: &str, value: &str, generation: u64, token: &CancellationToken) -> Result<AppSnapshot, String> {
        let _guard = self.gate.lock().await;
        token_check(token)?;
        {
            let session = self.session.lock().await;
            if let Some(active) = session.as_ref().filter(|session| session.network_owner.is_some()) {
                if kind != "playback" {
                    drop(session);
                } else {
                let mode = match value { "balanced" => 0, "lowLatency" => 1,
                    _ => return Err("未知的播放模式。".into()) };
                let previous = self.snapshot().settings.playback_mode;
                self.update_mode_setting(kind, value)?;
                let owner = active.network_owner.as_ref().expect("network session owner");
                if let Err(error) = owner.set_mode(mode).and_then(|_| owner.set_paused(active.output == "computer")) {
                    let rollback_mode = if previous.eq_ignore_ascii_case("lowlatency") { "lowLatency" } else { "balanced" };
                    let rollback = self.update_mode_setting("playback", rollback_mode);
                    return Err(rollback.err().map_or(error.clone(), |rollback| format!("{error}；设置回滚失败：{rollback}")));
                }
                {
                    let mut view = self.view.lock().expect("app view lock");
                    view.state = "reconnecting".into();
                    view.message = "正在应用播放模式".into();
                    view.status_detail = if active.output == "computer" {
                        "电脑输出保持暂停；Receiver 将使用新的认证会话应用播放模式。".into()
                    } else { "正在建立新的认证会话应用播放模式；电脑输出在确认 PCM 格式前保持静音状态。".into() };
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
                (true, Some(session.serial.clone()), session.output.clone(), session.transport.clone(), view.devices.iter().find(|device| device.serial == session.serial).cloned())
            } else { (false, None, "computer".into(), "unknown".into(), None) }
        };
        if !had_session {
            self.update_mode_setting(kind, value)?;
            let _ = self.refresh_devices().await?;
            let guidance = match selection {
                ReconfigureSelection::WirelessUdp => "已选择无线 UDP；请提供本地接口、已保存绑定和接收端点后启动网络音频。",
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

        self.set_state(app, "connecting", "正在有序切换连接…", old_device.as_ref().map(|device| device.display_name.clone()));
        if let Err(error) = self.teardown(Some(app)).await {
            self.set_state(app, "error", format!("切换前清理连接失败：{error}"), old_device.as_ref().map(|device| device.display_name.clone()));
            return Err(error);
        }
        {
            let mut view = self.view.lock().expect("view lock");
            view.state = "ready".into();
            view.busy = false;
        }
        token_check(token)?;
        self.update_mode_setting(kind, value)?;

        if old_transport == "wifi" && self.snapshot().settings.wireless_protocol == "adb"
            && self.snapshot().settings.auto_reconnect_wifi && matches!(selection, ReconfigureSelection::WirelessAdb | ReconfigureSelection::Playback | ReconfigureSelection::AudioBackend) {
            if let Some(endpoint) = serial.as_deref().filter(|endpoint| endpoint.contains(':')) {
                let _ = checked_adb(&self.adb_path, None, &["connect", endpoint], Duration::from_secs(10), Some(token)).await;
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
                explicit.as_ref().and_then(|serial| devices.iter().find(|device| &device.serial == serial && device.connectable && device.transport == "wifi"))
                    .map(|device| device.serial.clone())
                    .or_else(|| old_device.as_ref().and_then(|previous| {
                        let matching: Vec<_> = devices.iter().filter(|device| device.connectable && device.transport == "wifi" && same_physical_device(previous, device)).collect();
                        (matching.len() == 1).then(|| matching[0].serial.clone())
                    }))
            }
            ReconfigureSelection::Usb => old_device.as_ref().and_then(|previous| {
                let matching: Vec<_> = devices.iter().filter(|device| device.connectable && device.transport == "usb" && same_physical_device(previous, device)).collect();
                (matching.len() == 1).then(|| matching[0].serial.clone())
            }),
            ReconfigureSelection::Playback | ReconfigureSelection::AudioBackend => {
                if old_transport == "lan" || (old_transport == "wifi" && !wireless_adb_enabled(&settings)) { None }
                else { serial.as_ref().and_then(|serial| devices.iter().find(|device| &device.serial == serial && device.connectable)).map(|device| device.serial.clone()) }
            }
        };

        if let Some(serial) = target {
            let result = self.connect_locked(app, &serial, generation, token, old_output == "phone").await?;
            if !result.success { return Err(result.message); }
            if old_output == "computer" { self.set_output_locked("computer", token).await?; }
        } else {
            let guidance = match selection {
                ReconfigureSelection::WirelessUdp => "已选择无线 UDP；请提供本地接口、已保存绑定和接收端点后启动网络音频。",
                ReconfigureSelection::WirelessAdb if devices.iter().any(|device| device.transport == "wifi") => "已选择无线 ADB；请选择无线调试设备后连接。",
                ReconfigureSelection::WirelessAdb => "已选择无线 ADB；请添加手机或搜索无线调试设备。",
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
        self.save_settings(&settings)?;
        self.host_offer_listener.lock().expect("offer listener lock").take();
        view.settings = settings;
        Ok(())
    }

    pub fn open_logs(&self) -> Result<(), String> {
        let directory = self.data_dir.join("logs");
        std::process::Command::new("explorer.exe").arg(directory).spawn().map(|_| ()).map_err(|_| "无法打开日志文件夹。".into())
    }

    async fn teardown(&self, app: Option<&AppHandle>) -> Result<(), String> {
        self.host_offer_listener.lock().expect("offer listener lock").take();
        let mut errors = Vec::new();
        if let Some(mut session) = self.session.lock().await.take() {
            let network_session = session.network_owner.is_some();
            { let mut view = self.view.lock().expect("view lock"); view.session_active = false; view.session_serial = None; view.session_receiver_binding = None; view.session_transport = None; view.session_capabilities = None; view.playback_diagnostics = None; view.network_recovery_seconds_remaining = None; view.session_android_version = None; view.audio_bitrate_bps = None; }
            if let Err(error) = self.audio.stop_and_cleanup() {
                errors.push(format!("本机音频采集停止失败：{error}"));
            }
            if let Err(error) = self.audio.restore_muted() {
                errors.push(format!("电脑声音恢复失败：{error}"));
            }
            if let Some(mut owner) = session.network_owner.take() { owner.stop(); }
            if session.installed_receiver {
                if let Err(error) = receiver::stop(&self.adb_path, &session.serial, &self.receiver_apk_path,
                    &self.receiver_metadata_path, &CancellationToken::new()).await {
                    errors.push(format!("Receiver 停止失败：{error}"));
                }
            }
            if let Some(child) = session.child.as_mut() {
                let tail = safe_android_tail(&session.output_tail.lock().expect("output log lock").iter().cloned().collect::<Vec<_>>());
                if !tail.is_empty() { self.log(&format!("Android audio process output before teardown:\n{tail}")); }
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            if !network_session {
                if let Err(error) = adb_run(&self.adb_path, Some(&session.serial), &["reverse", "--remove", &format!("localabstract:{}", session.reverse)], Duration::from_secs(5), None).await.and_then(|result| if result.success { Ok(()) } else { Err("无法移除本次音频连接映射。".into()) }) {
                    errors.push(format!("ADB 映射清理失败：{error}"));
                }
            }
        } else {
            if let Err(error) = self.audio.stop_and_cleanup() {
                errors.push(format!("本机音频采集停止失败：{error}"));
            }
            if let Err(error) = self.audio.restore_muted() {
                errors.push(format!("电脑声音恢复失败：{error}"));
            }
        }
        *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
        { let mut view = self.view.lock().expect("app view lock"); view.output = "computer".into(); view.audio_bitrate_bps = None; }
        if let Some(app) = app { self.emit(app); }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("；")) }
    }

    pub async fn set_output(&self, app: &AppHandle, output: &str) -> Result<AppSnapshot, String> {
        if output != "computer" && output != "phone" { return Err("未知输出位置。".into()); }
        let (generation, token) = self.start_operation();
        let result = self.set_output_operation(app, output, generation, &token).await;
        self.finish_operation(generation);
        if result.is_err() { self.emit(app); }
        result
    }

    async fn set_output_operation(&self, app: &AppHandle, output: &str, generation: u64, token: &CancellationToken) -> Result<AppSnapshot, String> {
        let _guard = self.gate.lock().await;
        token_check(token)?;
        let session_up = self.session.lock().await.is_some();
        if !session_up && output == "computer" {
            self.emit(app);
            return Ok(self.snapshot());
        }
        if !session_up && output == "phone" {
            let serial = self.snapshot().selected_serial.ok_or_else(|| "请先选择可连接的手机。".to_string())?;
            let result = self.connect_locked(app, &serial, generation, token, true).await?;
            if !result.success { return Err(result.message); }
            return Ok(result.snapshot);
        }
        self.set_output_locked(output, token).await?;
        self.emit(app);
        Ok(self.snapshot())
    }

    async fn set_output_locked(&self, output: &str, token: &CancellationToken) -> Result<(), String> {
        {
            let mut session = self.session.lock().await;
            if let Some(active) = session.as_mut().filter(|session| session.network_owner.is_some()) {
                token_check(token)?;
                let owner = active.network_owner.as_ref().expect("network session owner");
                if output == "phone" && owner.snapshot().capabilities.as_ref().is_some_and(|info| !info.compatible) {
                    return Err("Receiver 管理连接已认证，但音频协议不兼容；请通过授权 ADB 或手动安装兼容版本。".into());
                }
                let change = async {
                    if output == "phone" {
                        let queue = if active.output == "computer" {
                            let old_queue = owner.pcm_queue();
                            owner.reconnect()?;
                            owner.set_paused(false)?;
                            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
                            loop {
                                token_check(token)?;
                                let snapshot = owner.snapshot();
                                if snapshot.state == SessionLifecycleState::Stopped {
                                    return Err(snapshot.last_error.unwrap_or_else(|| "Receiver session stopped".into()));
                                }
                                if let Some(queue) = owner.pcm_queue() {
                                    let is_fresh = old_queue.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &queue));
                                    if is_fresh && matches!(snapshot.state, SessionLifecycleState::ReceiverRunning
                                        | SessionLifecycleState::Streaming | SessionLifecycleState::Paused) {
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
                            let Some(queue) = owner.pcm_queue() else { return Ok(()); };
                            queue
                        };
                        self.audio.start_pcm_sink(queue.clone())?;
                        self.audio.mute_active()?;
                        active.network_sink_queue = Some(queue);
                    } else {
                        owner.set_paused(true)?;
                        self.audio.stop_pcm_sink()?;
                        self.audio.restore_muted()?;
                        active.network_sink_queue = None;
                    }
                    Ok(())
                }.await;
                if let Err(error) = change {
                    let mut cleanup = Vec::new();
                    if let Err(stop_error) = self.audio.stop_pcm_sink() { cleanup.push(stop_error); }
                    if let Err(restore_error) = self.audio.restore_muted() { cleanup.push(restore_error); }
                    if let Some(mut owner) = active.network_owner.take() { owner.stop(); }
                    *session = None;
                    let detail = if cleanup.is_empty() { error.clone() } else { format!("{error}；{}", cleanup.join("；")) };
                    let mut view = self.view.lock().expect("app view lock");
                    view.session_active = false;
                    view.session_serial = None;
                    view.session_receiver_binding = None;
                    view.session_transport = None;
                    view.session_capabilities = None;
                    view.playback_diagnostics = None;
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
                let mut view = self.view.lock().expect("view lock");
                view.output = output.into();
                view.state = if output == "phone" { "phoneReady".into() } else { "connected".into() };
                view.message = if output == "phone" { "手机接收已就绪".into() } else { "已连接 · 电脑播放".into() };
                view.status_detail = if output == "phone" { "P5-C 会话已恢复；正在接入手机音频。".into() }
                    else { "网络会话保持暂停；电脑声音已恢复到本机扬声器。".into() };
                return Ok(());
            }
        }
        let receiver_session = self.session.lock().await.as_ref()
            .map(|session| (session.serial.clone(), session.installed_receiver));
        let installed_receiver = receiver_session.as_ref().is_some_and(|(_, installed)| *installed);
        if output == "phone" && installed_receiver {
            let (serial, _) = receiver_session.as_ref().expect("installed Receiver session");
            let status = receiver::health(&self.adb_path, serial, &self.receiver_apk_path,
                &self.receiver_metadata_path, token).await?;
            if !status.healthy { return Err(status.message); }
        }
        if output == "phone" {
            if let Err(mute_error) = self.audio.mute_active() {
                return match self.audio.restore_muted() {
                    Ok(()) => Err(format!("无法静音电脑播放端点，已回滚本次静音：{mute_error}")),
                    Err(restore_error) => {
                        let mut view = self.view.lock().expect("view lock");
                        view.state = "error".into();
                        view.message = format!("切换到手机输出失败，且电脑声音恢复失败：{restore_error}");
                        view.status_detail = "电脑声音恢复待处理；可断开连接或从托盘重试。".into();
                        Err(format!("{mute_error}；恢复电脑声音失败：{restore_error}"))
                    }
                };
            }
        }
        if let Err(error) = self.audio.set_streaming(output == "phone") {
            let restore = if output == "phone" { self.audio.restore_muted().err() } else { None };
            let message = restore.as_ref().map_or_else(|| format!("无法切换音频输出：{error}"), |restore| format!("无法切换音频输出，恢复电脑声音也失败：{restore}"));
            let mut view = self.view.lock().expect("app view lock");
            view.state = "error".into();
            view.message = message;
            view.status_detail = "音频输出状态异常；请断开连接以恢复电脑声音。".into();
            return Err(restore.map_or(error.clone(), |restore| format!("{error}；恢复电脑声音失败：{restore}")));
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
            } else if output == "phone" { "电脑声音正在传到手机。".into() }
            else { "连接保持中；电脑声音已恢复到本机扬声器。".into() };
            view.state = if output == "phone" && installed_receiver { "phoneReady".into() }
                else if output == "phone" { "streaming".into() } else { "connected".into() };
            view.message = if output == "phone" && installed_receiver { "手机接收已就绪".into() }
                else if output == "phone" { "正在手机播放".into() } else { "已连接 · 电脑播放".into() };
        }
        if let Some(session) = self.session.lock().await.as_mut() { session.output = output.into(); }
        Ok(())
    }

    pub async fn cancel_operation(&self, app: &AppHandle) -> AppSnapshot {
        self.cancel_active_operation();
        let _guard = self.gate.lock().await;
        if self.session.try_lock().map_or(true, |session| session.is_none()) {
            let mut view = self.view.lock().expect("view lock");
            if view.busy {
                view.state = if view.devices.is_empty() { "noDevice".into() } else { "ready".into() };
                view.message = "连接已取消".into();
                view.device_name = None;
                view.output = "computer".into();
                view.busy = false;
            }
        }
        self.emit(app);
        self.snapshot()
    }

    pub fn update_setting(&self, key: &str, value: Value) -> Result<(), String> {
        let mut view = self.view.lock().expect("view lock");
        if view.busy {
            return Err("正在连接期间无法更改设置。".into());
        }
        if matches!(key, "transport" | "preferredTransport" | "wirelessProtocol" | "audioBackend" | "playbackMode") && self.session.try_lock().map_or(true, |session| session.is_some()) {
            return Err("请使用连接方式或播放模式控件切换当前会话。".into());
        }
        let mut settings = view.settings.clone();
        match key {
            "autoConnectLastDevice" => settings.auto_connect_last_device = value.as_bool().ok_or("无效设置值")?,
            "minimizeToTray" => settings.minimize_to_tray = value.as_bool().ok_or("无效设置值")?,
            "launchAtLogin" => settings.launch_at_login = value.as_bool().ok_or("无效设置值")?,
            "autoReconnectWifi" => settings.auto_reconnect_wifi = value.as_bool().ok_or("无效设置值")?,
            "preferredTransport" => settings.preferred_transport = normalize_transport(value.as_str().ok_or("无效连接方式")?).into(),
            "transport" => settings.preferred_transport = match value.as_str().ok_or("无效连接方式")? {
                "usb" => "Usb".into(),
                "wireless" => "Wifi".into(),
                _ => return Err("transport 必须为 usb 或 wireless。".into()),
            },
            "wirelessProtocol" => settings.wireless_protocol = match value.as_str().ok_or("无效无线协议")? {
                value if value.eq_ignore_ascii_case("udp") => "udp".into(),
                value if value.eq_ignore_ascii_case("adb") => "adb".into(),
                _ => return Err("无线协议必须为 udp 或 adb。".into()),
            },
            "audioBackend" => settings.audio_backend = match value.as_str().ok_or("无效 Receiver 后端")? {
                value if value.eq_ignore_ascii_case("receiver") => "receiver".into(),
                value if value.eq_ignore_ascii_case("legacy") => "legacy".into(),
                _ => return Err("Receiver 后端必须为 receiver 或 legacy。".into()),
            },
            "playbackMode" => settings.playback_mode = normalize_playback(value.as_str().ok_or("无效播放模式")?).into(),
            "appearanceTheme" => settings.appearance_theme = normalize_theme(value.as_str().ok_or("无效主题")?).into(),
            _ => return Err("未知设置项。".into()),
        }
        if matches!(key, "transport" | "preferredTransport" | "wirelessProtocol") {
            self.host_offer_listener.lock().expect("offer listener lock").take();
        }
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }

    pub async fn connect_wifi(&self, endpoint: &str) -> Result<OperationResult, String> {
        let _guard = self.gate.lock().await;
        let endpoint = normalize_endpoint(endpoint)?;
        if self.last_pairing_endpoint.lock().expect("pair endpoint lock").as_deref() == Some(endpoint.as_str()) {
            return Err("这是配对端口，不能用于连接；请搜索设备或使用手机无线调试主页上的连接端口。".into());
        }
        let result = checked_adb(&self.adb_path, None, &["connect", &endpoint], Duration::from_secs(10), None).await;
        let success = result.is_ok();
        let message = result.err().unwrap_or_else(|| format!("已连接到 {endpoint}。"));
        if success {
            self.persist_wireless_endpoint(&endpoint)?;
            let _ = self.refresh_devices().await;
        }
        Ok(OperationResult { success, message, snapshot: self.snapshot() })
    }

    pub async fn pair_wifi(&self, endpoint: &str, code: &str) -> Result<OperationResult, String> {
        let _guard = self.gate.lock().await;
        let endpoint = normalize_endpoint(endpoint)?;
        if code.len() != 6 || !code.bytes().all(|c| c.is_ascii_digit()) { return Err("配对码应为 6 位数字。".into()); }
        let result = adb_run(&self.adb_path, None, &["pair", &endpoint, code], Duration::from_secs(20), None).await?;
        let success = result.success && result.stdout.to_ascii_lowercase().contains("successfully paired");
        let mut message = if success { "配对成功。正在查找连接端口…".into() } else { "配对失败。请确认配对端口和 6 位配对码仍有效。".into() };
        if success {
            *self.last_pairing_endpoint.lock().expect("pair endpoint lock") = Some(endpoint.clone());
            if let Ok(discovery) = self.discover_wifi().await {
                let host = endpoint.rsplit_once(':').map(|(host, _)| host).unwrap_or("");
                let matches: Vec<_> = discovery["services"].as_array().into_iter().flatten()
                    .filter(|service| service["kind"] == "connect" && service["endpoint"].as_str().unwrap_or("").starts_with(&format!("{host}:")))
                    .collect();
                if matches.len() == 1 {
                    if let Some(connect_endpoint) = matches[0]["endpoint"].as_str() {
                        if checked_adb(&self.adb_path, None, &["connect", connect_endpoint], Duration::from_secs(10), None).await.is_ok() {
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
        self.log(if success { "Wireless pairing succeeded" } else { "Wireless pairing failed" });
        Ok(OperationResult { success, message, snapshot: self.snapshot() })
    }

    fn persist_wireless_endpoint(&self, endpoint: &str) -> Result<(), String> {
        let mut view = self.view.lock().expect("view lock");
        if view.session_active { return Ok(()); }
        let mut settings = view.settings.clone();
        settings.last_wireless_endpoint = Some(endpoint.to_owned());
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }

    pub async fn discover_wifi(&self) -> Result<Value, String> {
        self.discover_wifi_with_cancel(None).await
    }

    async fn discover_wifi_with_cancel(&self, cancel: Option<&CancellationToken>) -> Result<Value, String> {
        let check = adb_run(&self.adb_path, None, &["mdns", "check"], Duration::from_secs(5), cancel).await?;
        let combined = format!("{}\n{}", check.stdout, check.stderr).to_ascii_lowercase();
        if combined.contains("unavailable") || combined.contains("not running") || !combined.contains("mdns daemon version") {
            return Err("ADB mDNS 搜索不可用；请手动填写手机的 IP:连接端口。".into());
        }
        let services = adb_run(&self.adb_path, None, &["mdns", "services"], Duration::from_secs(6), cancel).await?;
        if !services.success { return Err("读取 ADB mDNS 设备失败。".into()); }
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
        let instance: String = bytes[..10].iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect();
        let password: String = bytes[10..].iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect();
        let name = format!("studio-{instance}");
        let payload = format!("WIFI:T:ADB;S:{name};P:{password};;");
        Ok(serde_json::json!({"instanceName":name,"password":password,"payload":payload}))
    }

    pub async fn pair_qr(&self, app: &AppHandle, instance: String, password: String) -> Result<OperationResult, String> {
        if instance.len() != 17 || !instance.starts_with("studio-") || !instance.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') ||
            password.len() != 20 || !password.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err("二维码配对凭据格式无效。".into());
        }
        let (generation, token) = self.start_qr_operation();
        let result = self.pair_qr_with_token(app, &instance, &password, &token).await;
        self.finish_qr_operation(generation);
        result
    }

    async fn pair_qr_with_token(&self, app: &AppHandle, instance: &str, password: &str, token: &CancellationToken) -> Result<OperationResult, String> {
        let _guard = self.gate.lock().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        let endpoint = loop {
            token_check(&token)?;
            let found = self.discover_wifi_with_cancel(Some(token)).await?;
            let services = found.get("services").and_then(Value::as_array).cloned().unwrap_or_default();
            if let Some(pairing) = services.into_iter().find(|s| s["kind"] == "pairing" && normalize_mdns_instance(s["name"].as_str().unwrap_or("")) == instance) {
                break pairing["endpoint"].as_str().ok_or("未收到配对端口")?.to_owned();
            }
            if tokio::time::Instant::now() >= deadline { return Err("等待扫码超时，请重新生成二维码。".into()); }
            let _ = app.emit("anpod://pairing-progress", "等待手机扫码…");
            tokio::select! { _ = token.cancelled() => return Err("二维码配对已取消。".into()), _ = tokio::time::sleep(Duration::from_millis(800)) => {} }
        };
        *self.last_pairing_endpoint.lock().expect("pair endpoint lock") = Some(endpoint.clone());
        let _ = app.emit("anpod://pairing-progress", "已识别手机，正在配对…");
        let pair = adb_run(&self.adb_path, None, &["pair", &endpoint, &password], Duration::from_secs(20), Some(&token)).await?;
        if !pair.success || !pair.stdout.to_ascii_lowercase().contains("successfully paired") { return Err("二维码配对失败，请重新生成后重试。".into()); }
        self.log("Wireless QR pairing succeeded");
        let host = endpoint.rsplit_once(':').map(|(h,_)| h).unwrap_or("");
        let connect_deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let (connect, connect_name) = loop {
            token_check(token)?;
            let services = self.discover_wifi_with_cancel(Some(token)).await?;
            let found = services["services"].as_array().cloned().unwrap_or_default();
            let matches: Vec<_> = found.iter().filter(|service|
                service["kind"] == "connect"
                    && service["endpoint"].as_str().is_some_and(|value| value.starts_with(&format!("{host}:")))
            ).collect();
            if matches.len() == 1 {
                break (
                    matches[0]["endpoint"].as_str().ok_or("缺少二维码手机的无线连接端口")?.to_owned(),
                    matches[0]["name"].as_str().ok_or("缺少二维码手机的连接实例名")?.to_owned(),
                );
            }
            if matches.len() > 1 { return Err("二维码手机广播了多个连接端口；请手动选择无线调试连接地址。".into()); }
            if tokio::time::Instant::now() >= connect_deadline { return Err("手机已配对，但没有发现对应的无线连接端口。请确认无线调试仍开启后重试。".into()); }
            let _ = app.emit("anpod://pairing-progress", "配对完成，正在等待手机广播连接端口…");
            tokio::select! { _ = token.cancelled() => return Err("二维码配对已取消。".into()), _ = tokio::time::sleep(Duration::from_millis(700)) => {} }
        };
        checked_adb(&self.adb_path, None, &["connect", &connect], Duration::from_secs(10), Some(token)).await?;
        let device_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        let device = loop {
            token_check(token)?;
            let snapshot = self.refresh_devices_with_cancel(Some(token)).await?;
            if let Some(device) = snapshot.devices.into_iter().find(|device|
                device.transport == "wifi"
                    && device.state == "device"
                    && device.connectable
                    && (device.serial == connect || normalize_mdns_instance(&device.serial) == normalize_mdns_instance(&connect_name))
            ) {
                break device;
            }
            if tokio::time::Instant::now() >= device_deadline { return Err("无线连接命令已完成，但 ADB 未显示这部手机为已授权设备。请在设备列表中检查连接状态。".into()); }
            tokio::select! { _ = token.cancelled() => return Err("二维码配对已取消。".into()), _ = tokio::time::sleep(Duration::from_millis(500)) => {} }
        };
        token_check(token)?;
        self.persist_wireless_endpoint(&connect)?;
        let mut view = self.view.lock().expect("view lock");
        if !view.session_active { view.selected_serial = Some(device.serial); }
        drop(view);
        Ok(OperationResult { success: true, message: "配对并连接完成。".into(), snapshot: self.snapshot() })
    }

    pub fn cancel_qr_pairing(&self) {
        if let Some(active) = self.qr_operation.lock().expect("qr operation lock").take() { active.token.cancel(); }
    }
}

static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn begin_operation_slot(generation: &AtomicU64, slot: &Mutex<Option<ActiveOperation>>, background: bool) -> Option<(u64, CancellationToken)> {
    let mut current = slot.lock().expect("operation lock");
    if background && current.is_some() { return None; }
    let next_generation = generation.fetch_add(1, Ordering::AcqRel) + 1;
    let token = CancellationToken::new();
    if let Some(previous) = current.as_ref() { previous.token.cancel(); }
    *current = Some(ActiveOperation { generation: next_generation, token: token.clone() });
    Some((next_generation, token))
}

fn cancel_operation_slot(generation: &AtomicU64, slot: &Mutex<Option<ActiveOperation>>) {
    let mut current = slot.lock().expect("operation lock");
    generation.fetch_add(1, Ordering::AcqRel);
    if let Some(operation) = current.as_ref() { operation.token.cancel(); }
    *current = None;
}

fn finish_operation_slot(slot: &Mutex<Option<ActiveOperation>>, generation: u64) {
    let mut current = slot.lock().expect("operation lock");
    if current.as_ref().is_some_and(|active| active.generation == generation) { *current = None; }
}

fn session_is_lost(device_present: Option<bool>, process_exited: bool, audio_connected: bool) -> bool {
    process_exited || !audio_connected || device_present == Some(false)
}

fn receiver_health_needed(installed_receiver: bool, device_present: Option<bool>, process_exited: bool, audio_connected: bool) -> bool {
    installed_receiver && !session_is_lost(device_present, process_exited, audio_connected)
}

fn merge_usb_playback_diagnostics(mut diagnostics: PlaybackDiagnostics, format: Option<PcmFormat>,
    generation: u64, previous: Option<(u64, u32)>) -> (PlaybackDiagnostics, Option<(u64, u32)>) {
    if let Some(format) = format {
        diagnostics.sample_rate = Some(format.sample_rate);
        diagnostics.channels = u8::try_from(format.channels).ok();
    }
    diagnostics.underrun_increment = match (previous, diagnostics.underrun_count) {
        (Some((sample_generation, previous)), Some(current))
            if sample_generation == generation && current >= previous => Some(u64::from(current - previous)),
        _ => None,
    };
    let next = diagnostics.underrun_count.map(|count| (generation, count));
    (diagnostics, next)
}

fn commit_pcm_bitrate_sample(current_session: Option<u64>, expected_session: u64,
    previous: &mut Option<(u64, u64, Instant)>, current_rate: &mut Option<u64>, bytes: u64, now: Instant) -> bool {
    if current_session != Some(expected_session) { return false; }
    let bitrate = calculate_pcm_bitrate(previous.as_ref().map(|(id, count, at)| (*id, *count, *at)),
        expected_session, bytes, now);
    *previous = Some((expected_session, bytes, now));
    if *current_rate == Some(bitrate) { false }
    else { *current_rate = Some(bitrate); true }
}

fn calculate_pcm_bitrate(previous: Option<(u64, u64, Instant)>, session_id: u64, bytes: u64, now: Instant) -> u64 {
    let Some((previous_session, previous_bytes, previous_at)) = previous else { return 0; };
    if previous_session != session_id { return 0; }
    let elapsed_nanos = now.saturating_duration_since(previous_at).as_nanos();
    if elapsed_nanos == 0 { return 0; }
    let delta = bytes.saturating_sub(previous_bytes) as u128;
    (delta.saturating_mul(8_000_000_000) / elapsed_nanos).min(u64::MAX as u128) as u64
}

fn same_physical_device(left: &Device, right: &Device) -> bool {
    left.physical_id.as_ref().is_some_and(|id| right.physical_id.as_ref() == Some(id))
}

fn receiver_target_matches(active_serial: &str, active_network: bool, active_binding: Option<&str>,
    target_serial: &str, target_binding: Option<&str>) -> bool {
    if active_network {
        target_binding.is_some() && active_binding == target_binding
    } else {
        active_serial == target_serial
    }
}

fn wireless_listener_matches_binding(listener_binding: Option<&str>, target_binding: &str) -> bool {
    listener_binding == Some(target_binding)
}

fn reset_preserves_wireless_listener(result: &receiver::BindingResetResult,
    expected: &receiver::BindingResetPreview) -> bool {
    result.serial == expected.serial && result.android_user == expected.android_user
        && result.binding_reference == expected.binding_reference
        && result.phone_state == "saved" && result.phone_commitment == expected.phone_commitment
        && result.pc_state == "saved" && result.pc_commitment == expected.pc_commitment
}

fn validate_receiver_reset_preview(actual: &receiver::BindingResetPreview,
    expected: &receiver::BindingResetPreview) -> Result<(), String> {
    if actual.serial != expected.serial || actual.android_user != expected.android_user
        || actual.binding_reference != expected.binding_reference
        || actual.phone_state != expected.phone_state || actual.phone_commitment != expected.phone_commitment
        || actual.pc_state != expected.pc_state || actual.pc_commitment != expected.pc_commitment {
        return Err("重置预览已过期（设备、Android user 或绑定摘要发生变化）；未停止会话或删除绑定，请重新检查。".into());
    }
    if actual.phone_state == "pending" || actual.phone_state == "failed" {
        return Err("手机绑定状态正在写入或不可读取；请完成/检查绑定操作后重新预览。未停止会话或删除绑定。".into());
    }
    Ok(())
}

fn valid_binding_reference(reference: &str) -> bool {
    reference.len() == 64 && reference.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn automatic_reception_binding(references: &[String], selected: Option<&str>) -> Result<Option<String>, String> {
    match selected.filter(|value| !value.is_empty()) {
        Some(value) if references.iter().any(|item| item == value) => Ok(Some(value.to_owned())),
        Some(_) => Err("所选 Receiver 绑定已不可用；请重新选择。".into()),
        None if references.len() == 1 => Ok(references.first().cloned()),
        None => Ok(None),
    }
}

fn pcm_queue_identity_changed(current: Option<&Arc<PcmQueue>>, next: Option<&Arc<PcmQueue>>) -> bool {
    match (current, next) {
        (Some(current), Some(next)) => !Arc::ptr_eq(current, next),
        (None, None) => false,
        _ => true,
    }
}

fn monitored_network_output(current: &str, management_only: bool) -> &str {
    if management_only { "computer" } else { current }
}

fn binding_store_path(data_dir: &Path, reference: &str) -> Result<PathBuf, String> {
    if !valid_binding_reference(reference) { return Err("Receiver 绑定引用格式无效。".into()); }
    Ok(data_dir.join("receiver-bindings").join(format!("{}.dat", reference.to_ascii_lowercase())))
}

fn choose_selected_serial(session_serial: Option<String>, previous: Option<String>, devices: &[Device], preferred: &str) -> Option<String> {
    if session_serial.is_some() { return session_serial; }
    let visible: Vec<_> = devices.iter().filter(|device| device.transport == preferred || device.transport == "unknown").collect();
    previous.filter(|serial| visible.iter().any(|device| &device.serial == serial))
        .or_else(|| { let candidates: Vec<_> = visible.iter().filter(|device| device.connectable).collect(); (candidates.len() == 1).then(|| candidates[0].serial.clone()) })
}

#[derive(Default)]
pub(crate) struct AdbResult { pub(crate) success: bool, pub(crate) stdout: String, pub(crate) stderr: String }

pub(crate) async fn adb_run(adb: &Path, serial: Option<&str>, args: &[&str], limit: Duration, cancel: Option<&CancellationToken>) -> Result<AdbResult, String> {
    if !adb.is_file() { return Err(format!("找不到 ADB：{}", adb.display())); }
    let mut command = Command::new(adb);
    if let Some(serial) = serial { command.args(["-s", serial]); }
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let child = command.spawn().map_err(|_| "无法启动 ADB。".to_string())?;
    let wait = child.wait_with_output();
    let output = if let Some(cancel) = cancel {
        tokio::select! {
            _ = cancel.cancelled() => return Err("操作已取消。".into()),
            result = timeout(limit, wait) => result.map_err(|_| "ADB 命令超时。".to_string())?.map_err(|_| "ADB 命令失败。".to_string())?,
        }
    } else {
        timeout(limit, wait).await.map_err(|_| "ADB 命令超时。".to_string())?.map_err(|_| "ADB 命令失败。".to_string())?
    };
    Ok(AdbResult { success: output.status.success(), stdout: String::from_utf8_lossy(&output.stdout).into_owned(), stderr: String::from_utf8_lossy(&output.stderr).into_owned() })
}

pub(crate) async fn adb_run_with_stdin(adb: &Path, serial: Option<&str>, args: &[&str], input: &[u8],
    limit: Duration, cancel: &CancellationToken) -> Result<AdbResult, String> {
    use tokio::io::AsyncWriteExt;
    use tokio::time::{timeout_at, Instant as TokioInstant};

    if !adb.is_file() { return Err("找不到 ADB。".into()); }
    let mut command = Command::new(adb);
    if let Some(serial) = serial { command.args(["-s", serial]); }
    command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|_| "无法启动 ADB。".to_string())?;
    let mut stdin = child.stdin.take().ok_or_else(|| "无法打开 ADB 私密输入管道。".to_string())?;
    let stdout = child.stdout.take().ok_or_else(|| "无法读取 ADB 命令状态。".to_string())?;
    let stderr = child.stderr.take().ok_or_else(|| "无法读取 ADB 命令状态。".to_string())?;
    let mut output_tasks = [
        Some(tokio::spawn(drain_output_presence(stdout))),
        Some(tokio::spawn(drain_output_presence(stderr))),
    ];
    let deadline = TokioInstant::now() + limit;

    let status = tokio::select! {
        _ = cancel.cancelled() => {
            stop_adb_child(&mut child, &mut output_tasks).await;
            return Err("操作已取消。".into());
        }
        result = timeout_at(deadline, async {
            stdin.write_all(input).await?;
            drop(stdin);
            child.wait().await
        }) => match result {
            Ok(Ok(status)) => status,
            Ok(Err(_)) => {
                stop_adb_child(&mut child, &mut output_tasks).await;
                return Err("ADB 私密输入传输失败。".into());
            }
            Err(_) => {
                stop_adb_child(&mut child, &mut output_tasks).await;
                return Err("ADB 私密输入操作超时。".into());
            }
        }
    };
    let (stdout_present, stderr_present) = match drain_output_tasks(&mut output_tasks, deadline, cancel).await {
        Ok(presence) => presence,
        Err(DrainError::Cancelled) => {
            stop_adb_child(&mut child, &mut output_tasks).await;
            return Err("操作已取消。".into());
        }
        Err(DrainError::TimedOut) => {
            stop_adb_child(&mut child, &mut output_tasks).await;
            return Err("ADB 命令超时。".into());
        }
        Err(DrainError::Failed) => {
            stop_adb_child(&mut child, &mut output_tasks).await;
            return Err("无法读取 ADB 命令状态。".into());
        }
    };
    let stdout = if stdout_present { "[output present]" } else { "" };
    let stderr = if stderr_present { "[output present]" } else { "" };
    Ok(AdbResult { success: status.success(), stdout: stdout.into(), stderr: stderr.into() })
}

async fn stop_adb_child(child: &mut Child, output_tasks: &mut [Option<tokio::task::JoinHandle<bool>>; 2]) {
    let _ = child.start_kill();
    let _ = timeout(Duration::from_secs(2), child.wait()).await;
    stop_output_tasks(output_tasks).await;
}

enum DrainError { Cancelled, TimedOut, Failed }

async fn drain_output_tasks(tasks: &mut [Option<tokio::task::JoinHandle<bool>>; 2],
    deadline: tokio::time::Instant, cancel: &CancellationToken) -> Result<(bool, bool), DrainError> {
    let stdout = await_output_task(&mut tasks[0], deadline, cancel).await?;
    let stderr = await_output_task(&mut tasks[1], deadline, cancel).await?;
    Ok((stdout, stderr))
}

async fn await_output_task(task: &mut Option<tokio::task::JoinHandle<bool>>,
    deadline: tokio::time::Instant, cancel: &CancellationToken) -> Result<bool, DrainError> {
    tokio::select! {
        _ = cancel.cancelled() => Err(DrainError::Cancelled),
        result = tokio::time::timeout_at(deadline, join_output_task(task)) => match result {
            Ok(Ok(presence)) => Ok(presence),
            Ok(Err(())) => Err(DrainError::Failed),
            Err(_) => Err(DrainError::TimedOut),
        },
    }
}

async fn join_output_task(task: &mut Option<tokio::task::JoinHandle<bool>>) -> Result<bool, ()> {
    let Some(handle) = task.as_mut() else { return Err(()); };
    let result = handle.await.map_err(|_| ());
    task.take();
    result
}

async fn stop_output_tasks(tasks: &mut [Option<tokio::task::JoinHandle<bool>>; 2]) {
    for task in tasks.iter().flatten() { task.abort(); }
    for task in tasks.iter_mut() {
        if let Some(task) = task.take() { let _ = task.await; }
    }
}

async fn drain_output_presence<R: tokio::io::AsyncRead + Unpin>(mut reader: R) -> bool {
    use tokio::io::AsyncReadExt;
    let mut chunk = [0u8; 1024];
    let mut present = false;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(count) => present |= count > 0,
        }
    }
    present
}

async fn capture_child_output<R>(reader: R, stream: &'static str, tail: Arc<Mutex<VecDeque<String>>>, log_path: PathBuf)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let entry = format!("{stream}: {line}");
        if safe_android_diagnostic(&entry) { append_app_log(&log_path, &format!("Android server: {line}")); }
        let mut tail = tail.lock().expect("output log lock");
        tail.push_back(entry);
        while tail.len() > 100 { tail.pop_front(); }
    }
}

fn safe_android_diagnostic(line: &str) -> bool {
    let line = line.strip_prefix("stdout: ").or_else(|| line.strip_prefix("stderr: ")).unwrap_or(line);
    ["ANPOD AUDIO ", "ANPOD BUFFER_", "ANPOD START_THRESHOLD", "ANPOD LOW_LATENCY", "ANPOD PLAYING ", "ANPOD UNDERRUNS "].iter().any(|prefix| line.starts_with(prefix))
}

fn has_restore_failure(error: &str) -> bool {
    error.contains("电脑声音恢复失败：")
}

fn safe_android_tail(lines: &[String]) -> String { lines.iter().filter(|line| safe_android_diagnostic(line)).cloned().collect::<Vec<_>>().join("\n") }

fn append_app_log(path: &Path, message: &str) {
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} {message}", chrono_stub_timestamp());
    }
}

async fn checked_adb(adb: &Path, serial: Option<&str>, args: &[&str], limit: Duration, cancel: Option<&CancellationToken>) -> Result<(), String> {
    let result = adb_run(adb, serial, args, limit, cancel).await?;
    let text = format!("{}\n{}", result.stdout, result.stderr).to_ascii_lowercase();
    if !result.success || ["failed", "failure", "unable to", "cannot", "connection refused", "timed out", "error:"].iter().any(|m| text.contains(m)) {
        return Err("ADB 未能完成此操作；请检查手机状态和连接地址。".into());
    }
    Ok(())
}

fn token_check(token: &CancellationToken) -> Result<(), String> { if token.is_cancelled() { Err("操作已取消。".into()) } else { Ok(()) } }

pub(crate) fn parse_devices(text: &str) -> Vec<Device> {
    text.lines().skip_while(|line| !line.starts_with("List of devices")).skip(1).filter_map(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        let serial = (*fields.first()?).to_owned();
        let state_index = if fields.get(1) == Some(&"no") && fields.get(2) == Some(&"permissions") { 3 } else { 2 };
        let state_raw = *fields.get(1)?;
        if serial.starts_with('*') { return None; }
        let tags: HashMap<_, _> = fields.iter().skip(state_index).filter_map(|field| field.split_once(':')).collect();
        let state = match state_raw { "device" => "device", "unauthorized" => "unauthorized", "offline" => "offline", "no" if state_index == 3 => "noPermissions", _ => "unknown" };
        let transport = if serial.contains(':') || serial.contains("_adb-tls-") { "wifi" } else if tags.contains_key("usb") { "usb" } else { "unknown" };
        let manufacturer = tags.get("manufacturer").copied().unwrap_or("").replace('_', " ");
        let model = tags.get("model").copied().unwrap_or("").replace('_', " ");
        let display_name = if model.is_empty() { serial.clone() } else if manufacturer.is_empty() { model.clone() } else { format!("{manufacturer} {model}") };
        let status_text = if transport == "unknown" && state == "device" { "连接方式待确认" } else { match state { "device" => "可连接", "unauthorized" => "等待手机授权", "offline" => "设备暂时离线", "noPermissions" => "缺少 USB 驱动权限", _ => "状态未知" } };
        Some(Device { serial, physical_id: None, transport: transport.into(), state: state.into(), manufacturer, model, android_version: String::new(), api_level: String::new(), display_name, status_text: status_text.into(), connectable: state == "device" && transport != "unknown" })
    }).collect()
}

fn parse_getprop(text: &str) -> HashMap<String, String> {
    text.lines().filter_map(|line| {
        let line = line.trim();
        let close = line.find("]:")?;
        Some((line.get(1..close)?.into(), line.get(close + 2..)?.trim().trim_start_matches('[').trim_end_matches(']').into()))
    }).collect()
}

fn normalize_endpoint(value: &str) -> Result<String, String> {
    let value = value.trim().trim_start_matches("tcp:");
    let (host, port) = value.rsplit_once(':').ok_or("地址格式应为 IPv4:端口。")?;
    host.parse::<std::net::Ipv4Addr>().map_err(|_| "地址格式应为 IPv4:端口。")?;
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) { return Err("端口需为 1–65535。".into()); }
    let port: u16 = port.parse().map_err(|_| "端口需为 1–65535。")?;
    if port == 0 { return Err("端口需为 1–65535。".into()); }
    Ok(format!("{host}:{port}"))
}

fn normalize_mdns_instance(value: &str) -> String {
    value.trim().trim_end_matches('.').replace("._adb-tls-pairing._tcp", "").replace("._adb-tls-connect._tcp", "").replace("._adb._tcp", "")
}

fn normalize_settings(mut settings: AppSettings) -> AppSettings {
    settings.schema_version = 1;
    settings.preferred_transport = match settings.preferred_transport.to_ascii_lowercase().as_str() { "wifi" => "Wifi".into(), _ => "Usb".into() };
    settings.wireless_protocol = if settings.wireless_protocol.is_empty() {
        if settings.preferred_transport == "Wifi" { "adb" } else { "udp" }.into()
    } else { normalize_wireless_protocol(&settings.wireless_protocol).into() };
    settings.audio_backend = if settings.audio_backend.is_empty()
        && std::env::var("ANPOD_AUDIO_BACKEND").is_ok_and(|value| value == "installed-receiver") {
        "receiver".into()
    } else { normalize_audio_backend(&settings.audio_backend).into() };
    settings.playback_mode = match settings.playback_mode.to_ascii_lowercase().as_str() { "lowlatency" | "low_latency" => "LowLatency".into(), _ => "Balanced".into() };
    settings.appearance_theme = match settings.appearance_theme.to_ascii_lowercase().as_str() { "light" => "Light".into(), "dark" => "Dark".into(), _ => "System".into() };
    settings
}

fn normalize_transport(value: &str) -> &'static str { if value.eq_ignore_ascii_case("wifi") { "Wifi" } else { "Usb" } }
fn normalize_wireless_protocol(value: &str) -> &'static str { if value.eq_ignore_ascii_case("adb") { "adb" } else { "udp" } }
fn normalize_audio_backend(value: &str) -> &'static str { if value.eq_ignore_ascii_case("receiver") { "receiver" } else { "legacy" } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReconfigureSelection { Playback, AudioBackend, Usb, WirelessAdb, WirelessUdp }
fn wireless_adb_enabled(settings: &AppSettings) -> bool { settings.wireless_protocol == "adb" }
fn selected_transport_allows_auto_adb(settings: &AppSettings) -> bool {
    !settings.preferred_transport.eq_ignore_ascii_case("wifi") || wireless_adb_enabled(settings)
}
fn reconfigure_selection(kind: &str, value: &str, settings: &AppSettings) -> Result<ReconfigureSelection, String> {
    match (kind, value) {
        ("playback", "balanced" | "lowLatency") => Ok(ReconfigureSelection::Playback),
        ("audioBackend", "receiver" | "legacy") => Ok(ReconfigureSelection::AudioBackend),
        ("transport", "usb") => Ok(ReconfigureSelection::Usb),
        ("transport", "wireless" | "wifi") => Ok(if settings.wireless_protocol == "adb" {
            ReconfigureSelection::WirelessAdb
        } else { ReconfigureSelection::WirelessUdp }),
        ("wirelessProtocol", "udp") => Ok(ReconfigureSelection::WirelessUdp),
        ("wirelessProtocol", "adb") => Ok(ReconfigureSelection::WirelessAdb),
        _ => Err("未知的连接或播放模式。".into()),
    }
}
fn selected_connection_transport(preferred: &str) -> &'static str { if preferred.eq_ignore_ascii_case("wifi") { "wireless" } else { "usb" } }
fn session_connection_transport(transport: &str) -> Option<&'static str> {
    match transport { "usb" => Some("usb"), "wifi" | "lan" => Some("wireless"), _ => None }
}
fn session_wireless_protocol(transport: &str) -> Option<&'static str> {
    match transport { "wifi" => Some("adb"), "lan" => Some("udp"), _ => None }
}
fn normalize_playback(value: &str) -> &'static str { if value.eq_ignore_ascii_case("lowLatency") { "LowLatency" } else { "Balanced" } }
fn normalize_theme(value: &str) -> &'static str { match value.to_ascii_lowercase().as_str() { "light" => "Light", "dark" => "Dark", _ => "System" } }
fn uuid_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成本次连接的安全握手令牌。".to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn chrono_stub_timestamp() -> String { format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()) }

pub fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" { fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32; }
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
        let ok = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) };
        if ok == 0 { return Err(std::io::Error::last_os_error()); }
        Ok(())
    }
    #[cfg(not(windows))]
    { std::fs::rename(source, destination) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_playback_diagnostics_use_session_scoped_health_intervals() {
        let mut receiver = PlaybackDiagnostics { receiver_sample_rate: Some(48_000), receiver_channels: Some(2),
            pc_dropped_frames: None, pc_queued_frames: None, underrun_count: Some(12), ..PlaybackDiagnostics::default() };
        let (first, sample) = merge_usb_playback_diagnostics(receiver.clone(),
            Some(PcmFormat { sample_rate: 48_000, channels: 2 }), 7, None);
        assert_eq!(first.sample_rate, Some(48_000));
        assert_eq!(first.channels, Some(2));
        assert_eq!(first.pc_dropped_frames, None);
        assert_eq!(first.pc_queued_frames, None);
        assert_eq!(first.underrun_increment, None);
        assert_eq!(sample, Some((7, 12)));

        receiver.underrun_count = Some(15);
        let (next, sample) = merge_usb_playback_diagnostics(receiver.clone(), None, 7, sample);
        assert_eq!(next.underrun_increment, Some(3));
        assert_eq!(sample, Some((7, 15)));

        receiver.underrun_count = Some(1);
        let (new_session, sample) = merge_usb_playback_diagnostics(receiver.clone(), None, 8, sample);
        assert_eq!(new_session.underrun_increment, None);
        assert_eq!(sample, Some((8, 1)));
        receiver.underrun_count = None;
        let (unknown, sample) = merge_usb_playback_diagnostics(receiver, None, 8, sample);
        assert_eq!(unknown.underrun_increment, None);
        assert_eq!(sample, None);
    }

    struct SignalOnDrop(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for SignalOnDrop {
        fn drop(&mut self) {
            if let Some(signal) = self.0.take() { let _ = signal.send(()); }
        }
    }

    #[test]
    #[ignore]
    fn adb_stdin_child_helper() {
        use std::{io::Write, thread};
        let mut stdout = std::io::stdout().lock();
        let mut stderr = std::io::stderr().lock();
        loop {
            let _ = stdout.write_all(&[b'x'; 1024]);
            let _ = stderr.write_all(&[b'y'; 1024]);
            let _ = stdout.flush();
            let _ = stderr.flush();
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[tokio::test]
    async fn adb_stdin_cancellation_reaps_child_and_output_drains() {
        let executable = env::current_exe().unwrap();
        let args = ["--ignored", "--exact", "state::tests::adb_stdin_child_helper", "--nocapture"];
        let cancel = CancellationToken::new();
        let child_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            adb_run_with_stdin(&executable, None, &args, b"synthetic", Duration::from_secs(20), &child_cancel).await
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel.cancel();
        assert!(matches!(task.await.unwrap(), Err(error) if error == "操作已取消。"));

        let (stderr_started, mut stderr_started_rx) = tokio::sync::oneshot::channel();
        let (stderr_dropped, stderr_dropped_rx) = tokio::sync::oneshot::channel();
        let stdout_task = tokio::spawn(async { true });
        let stderr_task = tokio::spawn(async move {
            let _signal_on_drop = SignalOnDrop(Some(stderr_dropped));
            let _ = stderr_started.send(());
            std::future::pending::<bool>().await
        });
        let mut drains = [Some(stdout_task), Some(stderr_task)];
        let drain_cancel = CancellationToken::new();
        let (stdout_result, stderr_result) = {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            let stdout_result = await_output_task(&mut drains[0], deadline, &drain_cancel).await;
            let stderr_future = await_output_task(&mut drains[1], deadline, &drain_cancel);
            tokio::pin!(stderr_future);
            tokio::select! {
                _ = &mut stderr_started_rx => drain_cancel.cancel(),
                result = &mut stderr_future => panic!("blocked stderr drain completed unexpectedly: {}", result.is_ok()),
            }
            (stdout_result, stderr_future.await)
        };
        assert!(matches!(stdout_result, Ok(true)));
        assert!(matches!(stderr_result, Err(DrainError::Cancelled)));
        assert!(drains[0].is_none(), "completed stdout handle must be consumed once");
        assert!(drains[1].is_some(), "blocked stderr handle must remain available for cleanup");
        stop_output_tasks(&mut drains).await;
        assert!(stderr_dropped_rx.await.is_ok(), "aborted stderr drain must be joined and dropped");
    }

    #[test]
    fn pcm_bitrate_uses_successful_byte_deltas_and_resets_between_sessions() {
        let started = Instant::now();
        assert_eq!(calculate_pcm_bitrate(Some((1, 0, started)), 1, 192_000, started + Duration::from_secs(1)), 1_536_000);
        assert_eq!(calculate_pcm_bitrate(Some((1, 900_000, started)), 2, 192_000, started + Duration::from_secs(1)), 0);
        assert_eq!(calculate_pcm_bitrate(Some((1, 192_000, started)), 1, 192_000, started + Duration::from_secs(1)), 0);
    }

    #[test]
    fn stale_pcm_sample_is_rejected_after_session_changes() {
        let started = Instant::now();
        let mut previous = Some((2, 10, started));
        let mut current_rate = None;
        assert!(!commit_pcm_bitrate_sample(Some(2), 1, &mut previous,
            &mut current_rate, 192_010, started + Duration::from_secs(1)));
        assert_eq!(previous.as_ref().map(|(id, count, _)| (*id, *count)), Some((2, 10)));
        assert_eq!(current_rate, None);
        assert!(commit_pcm_bitrate_sample(Some(2), 2, &mut previous,
            &mut current_rate, 192_010, started + Duration::from_secs(1)));
        assert_eq!(previous.as_ref().map(|(id, count, _)| (*id, *count)), Some((2, 192_010)));
        assert_eq!(current_rate, Some(1_536_000));
    }

    #[test]
    fn fresh_network_queue_is_handed_off_even_without_observing_none() {
        let old = Arc::new(PcmQueue::new());
        let new = Arc::new(PcmQueue::new());
        assert!(!pcm_queue_identity_changed(Some(&old), Some(&old)));
        assert!(pcm_queue_identity_changed(Some(&old), Some(&new)));
    }

    #[test]
    fn management_only_recovery_selects_computer_output() {
        assert_eq!(monitored_network_output("phone", true), "computer");
    }

    #[test]
    fn parses_and_filters_adb_device_rows() {
        let rows = "List of devices attached\nZX1\tdevice product:oriole model:Pixel_8 manufacturer:Google usb:1-2\n192.0.2.5:37123 device product:a model:Phone\nZX2 unauthorized usb:1-3\n";
        let devices = parse_devices(rows);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].transport, "usb");
        assert_eq!(devices[1].transport, "wifi");
        assert_eq!(devices[2].status_text, "等待手机授权");
    }

    #[test]
    fn settings_keep_schema_one_names_and_defaults() {
        let value = serde_json::to_value(AppSettings::default()).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["preferredTransport"], "Usb");
        assert_eq!(value["wirelessProtocol"], "udp");
        assert_eq!(value["audioBackend"], "legacy");
        assert_eq!(value["playbackMode"], "Balanced");
        let settings = normalize_settings(serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Wifi",
            "wirelessProtocol": "adb", "audioBackend": "receiver"
        })).unwrap());
        assert_eq!(settings.preferred_transport, "Wifi");
        assert_eq!(settings.wireless_protocol, "adb");
        assert_eq!(settings.audio_backend, "receiver");
        let old_saved = normalize_settings(serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Wifi"
        })).unwrap());
        assert_eq!(old_saved.preferred_transport, "Wifi");
        assert_eq!(old_saved.wireless_protocol, "adb");
        assert_eq!(old_saved.audio_backend, "legacy");
        let new_saved = normalize_settings(serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Usb", "wirelessProtocol": "udp"
        })).unwrap());
        assert_eq!(new_saved.wireless_protocol, "udp");
        assert_eq!(selected_transport_allows_auto_adb(&old_saved), true);
        assert_eq!(selected_transport_allows_auto_adb(&new_saved), true);
        let explicit_udp_wifi = normalize_settings(serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Wifi", "wirelessProtocol": "udp"
        })).unwrap());
        assert_eq!(explicit_udp_wifi.wireless_protocol, "udp");
        assert!(!selected_transport_allows_auto_adb(&explicit_udp_wifi));
        assert_eq!(reconfigure_selection("transport", "wireless", &old_saved), Ok(ReconfigureSelection::WirelessAdb));
        assert_eq!(reconfigure_selection("wirelessProtocol", "udp", &old_saved), Ok(ReconfigureSelection::WirelessUdp));
        assert!(reconfigure_selection("wirelessProtocol", "invalid", &old_saved).is_err());
        let state = AppState::new();
        state.view.lock().unwrap().settings.preferred_transport = "Wifi".into();
        assert_eq!(state.snapshot().connection_transport, "wireless");
        assert_eq!(state.snapshot().transport, "wifi");
        assert_eq!(session_connection_transport("lan"), Some("wireless"));
        assert_eq!(session_wireless_protocol("lan"), Some("udp"));
        assert_eq!(session_connection_transport("wifi"), Some("wireless"));
        assert_eq!(session_wireless_protocol("wifi"), Some("adb"));
        assert_eq!(normalize_endpoint("192.0.2.2:5555").unwrap(), "192.0.2.2:5555");
        assert!(normalize_endpoint("192.0.2.2:0").is_err());
    }

    #[test]
    fn operation_owner_cannot_be_cleared_by_an_older_completion() {
        let generation = AtomicU64::new(0);
        let slot = Mutex::new(None);
        let (first_generation, first_token) = begin_operation_slot(&generation, &slot, false).unwrap();
        let (current_generation, current_token) = begin_operation_slot(&generation, &slot, false).unwrap();
        assert!(first_token.is_cancelled());
        finish_operation_slot(&slot, first_generation);
        assert!(slot.lock().unwrap().as_ref().is_some_and(|active| active.generation == current_generation && !active.token.is_cancelled()));
        assert!(!current_token.is_cancelled());
    }

    #[test]
    fn wireless_protocol_switch_rejects_completion_from_previous_owner() {
        let generation = AtomicU64::new(0);
        let slot = Mutex::new(None);
        let (old_generation, old_token) = begin_operation_slot(&generation, &slot, false).unwrap();
        let (new_generation, new_token) = begin_operation_slot(&generation, &slot, false).unwrap();
        assert!(old_token.is_cancelled());
        finish_operation_slot(&slot, old_generation);
        assert!(slot.lock().unwrap().as_ref().is_some_and(|operation| operation.generation == new_generation));
        assert!(!new_token.is_cancelled());
    }

    #[test]
    fn background_operation_does_not_preempt_a_foreground_request() {
        let generation = AtomicU64::new(0);
        let slot = Mutex::new(None);
        let (foreground_generation, foreground_token) = begin_operation_slot(&generation, &slot, false).unwrap();
        assert!(begin_operation_slot(&generation, &slot, true).is_none());
        assert!(slot.lock().unwrap().as_ref().is_some_and(|active| active.generation == foreground_generation));
        assert!(!foreground_token.is_cancelled());
        assert_eq!(automatic_reception_binding(&[], None).unwrap(), None);
        let references = vec!["saved-a".to_string(), "saved-b".to_string()];
        assert_eq!(automatic_reception_binding(&references[..1], None).unwrap(), Some("saved-a".into()));
        assert_eq!(automatic_reception_binding(&references, None).unwrap(), None, "several saved identities need selection");
        assert_eq!(automatic_reception_binding(&references, Some("saved-b")).unwrap(), Some("saved-b".into()));
        assert!(automatic_reception_binding(&references, Some("untrusted-hint")).is_err());
    }

    #[test]
    fn native_disconnect_is_detected_when_adb_scan_fails() {
        assert!(session_is_lost(None, false, false));
        assert!(!session_is_lost(None, false, true));
        assert!(session_is_lost(Some(false), false, true));
    }

    #[test]
    fn receiver_health_monitor_skips_known_local_or_transport_failures() {
        assert!(receiver_health_needed(true, Some(true), false, true));
        assert!(!receiver_health_needed(false, Some(true), false, true));
        assert!(!receiver_health_needed(true, Some(false), false, true));
        assert!(!receiver_health_needed(true, Some(true), true, true));
        assert!(!receiver_health_needed(true, Some(true), false, false));
    }

    #[test]
    fn active_device_selection_is_idempotent_and_cannot_change_session() {
        let state = AppState::new();
        let make = |serial: &str| Device {
            serial: serial.into(), physical_id: None, transport: "usb".into(), state: "device".into(),
            manufacturer: "Google".into(), model: "Pixel".into(), android_version: "15".into(), api_level: "35".into(),
            display_name: serial.into(), status_text: "可连接".into(), connectable: true,
        };
        {
            let mut view = state.view.lock().unwrap();
            view.devices = vec![make("TEST-A"), make("TEST-B")];
            view.selected_serial = Some("TEST-A".into());
            view.session_serial = Some("TEST-A".into());
            view.session_active = true;
        }

        assert_eq!(state.select_device("TEST-A").unwrap().selected_serial.as_deref(), Some("TEST-A"));
        assert!(state.select_device("TEST-B").is_err());
        let snapshot = state.snapshot();
        assert_eq!(snapshot.session_serial.as_deref(), Some("TEST-A"));
        assert_eq!(snapshot.selected_serial.as_deref(), Some("TEST-A"));
        let scanned_devices = [make("TEST-B")];
        assert_eq!(choose_selected_serial(snapshot.session_serial, snapshot.selected_serial, &scanned_devices, "usb").as_deref(), Some("TEST-A"));
    }

    #[test]
    fn adding_wireless_device_does_not_replace_active_session_reconnect_target() {
        let state = AppState::new();
        {
            let mut view = state.view.lock().unwrap();
            view.session_active = true;
            view.session_serial = Some("TEST-A".into());
            view.selected_serial = Some("TEST-A".into());
            view.settings.last_device_id = Some("TEST-A".into());
            view.settings.last_wireless_endpoint = Some("192.0.2.1:5555".into());
        }

        state.persist_wireless_endpoint("192.0.2.2:5555").unwrap();
        let snapshot = state.snapshot();
        assert_eq!(snapshot.selected_serial.as_deref(), Some("TEST-A"));
        assert_eq!(snapshot.settings.last_device_id.as_deref(), Some("TEST-A"));
        assert_eq!(snapshot.settings.last_wireless_endpoint.as_deref(), Some("192.0.2.1:5555"));
    }

    #[test]
    fn transport_switch_requires_a_proven_physical_identity() {
        let make = |serial: &str, physical_id: Option<&str>| Device {
            serial: serial.into(), physical_id: physical_id.map(str::to_owned), transport: "usb".into(), state: "device".into(),
            manufacturer: "Google".into(), model: "Pixel".into(), android_version: "15".into(), api_level: "35".into(),
            display_name: "Google Pixel".into(), status_text: "可连接".into(), connectable: true,
        };
        assert!(same_physical_device(&make("USB-1", Some("SERIAL")), &make("192.0.2.1:5555", Some("SERIAL"))));
        assert!(!same_physical_device(&make("USB-1", None), &make("192.0.2.1:5555", None)));
        assert!(!same_physical_device(&make("USB-1", Some("SERIAL-A")), &make("192.0.2.1:5555", Some("SERIAL-B"))));
    }

    #[test]
    fn receiver_reset_cannot_stop_a_session_for_another_target() {
        assert!(receiver_target_matches("USB-A", false, None, "USB-A", Some("reference-a")));
        assert!(!receiver_target_matches("USB-B", false, None, "USB-A", Some("reference-a")));
        assert!(receiver_target_matches("USB-A", true, Some("reference-a"), "USB-Z", Some("reference-a")));
        assert!(!receiver_target_matches("USB-A", true, Some("reference-b"), "USB-A", Some("reference-a")));
        assert!(!receiver_target_matches("USB-A", true, Some("reference-a"), "USB-A", None));
    }

    #[test]
    fn operation_slot_keeps_reset_and_offer_reservations_exclusive() {
        let generation = AtomicU64::new(0);
        let slot = Mutex::new(None);
        let (reset_generation, reset_token) = begin_operation_slot(&generation, &slot, true).unwrap();
        assert!(begin_operation_slot(&generation, &slot, true).is_none());
        assert!(!reset_token.is_cancelled());
        finish_operation_slot(&slot, reset_generation);

        let (offer_generation, offer_token) = begin_operation_slot(&generation, &slot, true).unwrap();
        assert!(begin_operation_slot(&generation, &slot, true).is_none());
        assert!(!offer_token.is_cancelled());
        finish_operation_slot(&slot, offer_generation);
    }

    #[test]
    fn binding_reset_preserves_only_the_matching_listener_and_unchanged_saved_binding() {
        let expected = receiver::BindingResetPreview {
            serial: "USB-A".into(), android_user: "10".into(), binding_reference: "reference-a".into(),
            phone_state: "saved".into(), phone_commitment: Some("a".repeat(64)),
            pc_state: "saved".into(), pc_commitment: Some("b".repeat(64)), receiver_busy: false,
        };
        assert!(wireless_listener_matches_binding(Some("reference-a"), "reference-a"));
        assert!(!wireless_listener_matches_binding(Some("reference-b"), "reference-a"));
        assert!(!wireless_listener_matches_binding(None, "reference-a"));
        let mut result = receiver::BindingResetResult {
            outcome: "phoneRetained".into(), serial: expected.serial.clone(), android_user: expected.android_user.clone(),
            binding_reference: expected.binding_reference.clone(), phone_state: "saved".into(),
            phone_commitment: expected.phone_commitment.clone(), pc_state: "saved".into(),
            pc_commitment: expected.pc_commitment.clone(), reason: String::new(), next_action: String::new(),
        };
        assert!(reset_preserves_wireless_listener(&result, &expected));
        for (phone_state, pc_state) in [("empty", "missing"), ("unknown", "saved"), ("saved", "missing")] {
            result.phone_state = phone_state.into();
            result.pc_state = pc_state.into();
            assert!(!reset_preserves_wireless_listener(&result, &expected));
        }
        result.phone_state = "saved".into();
        result.pc_state = "saved".into();
        result.pc_commitment = Some("c".repeat(64));
        assert!(!reset_preserves_wireless_listener(&result, &expected));
    }

    #[test]
    fn receiver_reset_preflight_rejects_pending_or_failed_phone_state_before_stop() {
        let preview = receiver::BindingResetPreview {
            serial: "USB-A".into(), android_user: "10".into(), binding_reference: "reference-a".into(),
            phone_state: "saved".into(), phone_commitment: Some("a".repeat(64)),
            pc_state: "saved".into(), pc_commitment: Some("b".repeat(64)), receiver_busy: false,
        };
        assert!(validate_receiver_reset_preview(&preview, &preview).is_ok());
        for state in ["pending", "failed"] {
            let invalid = receiver::BindingResetPreview { phone_state: state.into(), phone_commitment: None,
                ..preview.clone() };
            assert!(validate_receiver_reset_preview(&invalid, &invalid).is_err(),
                "{state} must be rejected before any session teardown");
        }
        let changed = receiver::BindingResetPreview { android_user: "11".into(), ..preview.clone() };
        assert!(validate_receiver_reset_preview(&changed, &preview).is_err());
    }
}
