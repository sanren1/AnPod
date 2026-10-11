use crate::audio::AudioActor;
use crate::receiver::{self, ReceiverStatus};
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
    pub session_transport: Option<String>,
    pub session_android_version: Option<String>,
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

struct Session {
    serial: String,
    transport: String,
    reverse: String,
    child: Option<Child>,
    installed_receiver: bool,
    output: String,
    output_tail: Arc<Mutex<VecDeque<String>>>,
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
    session_transport: Option<String>,
    session_android_version: Option<String>,
    output: String,
    settings: AppSettings,
    status_detail: String,
    busy: bool,
    session_active: bool,
    audio_bitrate_bps: Option<u64>,
}

pub struct AppState {
    view: Mutex<ViewState>,
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
    pcm_rate_sample: Mutex<Option<(String, u64, Instant)>>,
    audio: AudioActor,
    data_dir: PathBuf,
    adb_path: PathBuf,
    server_path: PathBuf,
    receiver_apk_path: PathBuf,
    receiver_metadata_path: PathBuf,
}

impl AppState {
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
                session_transport: None,
                session_android_version: None,
                output: "computer".into(),
                settings: AppSettings::default(),
                status_detail: "连接 USB 手机或切换到 Wi-Fi 添加手机。".into(),
                busy: false,
                session_active: false,
                audio_bitrate_bps: None,
            }),
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
        let (generation, token, _guard) = self.begin_receiver_operation(true).await?;
        let result = receiver::install(&self.adb_path, serial, &self.receiver_apk_path, &self.receiver_metadata_path, &token).await;
        self.finish_operation(generation);
        result
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
            session_transport: view.session_transport.clone(),
            session_android_version: view.session_android_version.clone(),
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
        let session_id = self.session.lock().await.as_ref().map(|session| session.reverse.clone());
        let Some(session_id) = session_id else {
            self.clear_audio_bitrate_if_session(None, app).await;
            return;
        };
        let Ok(bytes) = self.audio.sent_pcm_bytes() else {
            self.clear_audio_bitrate_if_session(Some(&session_id), app).await;
            return;
        };
        let now = Instant::now();
        let session = self.session.lock().await;
        let Some(current) = session.as_ref().filter(|session| session.reverse == session_id) else { return; };
        let changed = {
            let mut previous = self.pcm_rate_sample.lock().expect("PCM rate sample lock");
            let mut view = self.view.lock().expect("app view lock");
            commit_pcm_bitrate_sample(Some(&current.reverse), &session_id, &mut previous,
                &mut view.audio_bitrate_bps, bytes, now)
        };
        drop(session);
        if changed { self.emit(app); }
    }

    async fn clear_audio_bitrate_if_session(&self, expected_session: Option<&str>, app: &AppHandle) {
        let session = self.session.lock().await;
        if session.as_ref().map(|session| session.reverse.as_str()) != expected_session { return; }
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

    fn update_receiver_playback_status(&self, phone_output: bool, audio_active: bool) -> bool {
        if !phone_output { return false; }
        let mut view = self.view.lock().expect("app view lock");
        let (state, message, detail) = if audio_active {
            ("streaming", "正在手机播放", "Receiver 已确认 AudioTrack 正在播放。")
        } else {
            ("phoneReady", "手机接收已就绪", "手机接收端已就绪；电脑有声音发送后将开始播放。")
        };
        if view.state != state || view.message != message || view.status_detail != detail {
            view.state = state.into();
            view.message = message.into();
            view.status_detail = detail.into();
            true
        } else {
            false
        }
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
            .unwrap_or_default();
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

    pub async fn monitor_once(&self, app: &AppHandle) {
        let Ok(_gate) = self.gate.try_lock() else { return; };
        if self.operation.lock().expect("operation lock").as_ref().is_some_and(|operation| !operation.token.is_cancelled()) { return; }
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
            (session.serial.clone(), session.transport.clone(), session.output.clone(), session.output_tail.clone(), session.reverse.clone(), session.installed_receiver, exited)
        });
        if let Some((serial, transport, output, tail, session_key, installed_receiver, exited)) = session {
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
                            if self.update_receiver_playback_status(output == "phone", status.audio_active) { self.emit(app); }
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
        if !settings.auto_connect_last_device { return; }
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
        if snapshot.transport == "wifi" && settings.auto_reconnect_wifi {
            if let Some(endpoint) = settings.last_wireless_endpoint.filter(|endpoint| endpoint.contains(':')) {
                if self.wireless_probe_attempted.swap(true, Ordering::AcqRel) { return; }
                if checked_adb(&self.adb_path, None, &["connect", &endpoint], Duration::from_secs(8), None).await.is_ok() {
                    let _ = self.refresh_devices().await;
                }
            }
        }
    }

    async fn recover_session_locked(&self, app: &AppHandle, serial: String, transport: String, was_playing: bool, session_key: String, reason: String) {
        if self.recovering.swap(true, Ordering::AcqRel) { return; }
        let suppressed = self.suppressed_auto_connect.lock().expect("auto-connect suppression lock").as_deref() == Some(serial.as_str());
        let same_session = self.session.lock().await.as_ref()
            .is_some_and(|session| session.serial == serial && session.reverse == session_key);
        if suppressed || !same_session {
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
        let settings = self.snapshot().settings;
        if !settings.auto_connect_last_device || (transport == "wifi" && !settings.auto_reconnect_wifi) {
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
        let installed_receiver = device.transport == "usb"
            && env::var("ANPOD_AUDIO_BACKEND").is_ok_and(|value| value.eq_ignore_ascii_case("installed-receiver"));
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
        *self.session.lock().await = Some(Session { serial: serial.into(), transport: device.transport.clone(), reverse: reverse.clone(), child: None, installed_receiver, output: "computer".into(), output_tail: Arc::new(Mutex::new(VecDeque::new())) });
        {
            let mut view = self.view.lock().expect("view lock");
            view.session_active = true;
            view.session_serial = Some(device.serial.clone());
            view.session_transport = Some(device.transport.clone());
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
        let (generation, token) = self.start_operation();
        let result = self.reconfigure_locked(app, kind, value, generation, &token).await;
        self.finish_operation(generation);
        result
    }

    async fn reconfigure_locked(&self, app: &AppHandle, kind: &str, value: &str, generation: u64, token: &CancellationToken) -> Result<AppSnapshot, String> {
        let _guard = self.gate.lock().await;
        token_check(token)?;
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

        if old_transport == "wifi" && self.snapshot().settings.auto_reconnect_wifi {
            if let Some(endpoint) = serial.as_deref().filter(|endpoint| endpoint.contains(':')) {
                let _ = checked_adb(&self.adb_path, None, &["connect", endpoint], Duration::from_secs(10), Some(token)).await;
            }
        }
        token_check(token)?;
        self.refresh_devices().await?;

        let transport = self.snapshot().transport;
        let devices = self.snapshot().devices;
        let target = if kind == "playback" {
            serial.as_ref().and_then(|serial| devices.iter().find(|device| &device.serial == serial && device.connectable)).map(|device| device.serial.clone())
        } else if let Some(previous) = old_device.as_ref() {
            let matching: Vec<_> = devices.iter().filter(|device|
                device.connectable && device.transport == transport && same_physical_device(previous, device)
            ).collect();
            (matching.len() == 1).then(|| matching[0].serial.clone())
        } else { None };

        if let Some(serial) = target {
            let result = self.connect_locked(app, &serial, generation, token, old_output == "phone").await?;
            if !result.success { return Err(result.message); }
            if old_output == "computer" { self.set_output_locked("computer", token).await?; }
        } else {
            let guidance = if transport == "wifi" { "连接方式已切换到 Wi-Fi；请添加手机或搜索无线调试设备。" } else { "连接方式已切换到 USB；请连接并授权手机。" };
            self.view.lock().expect("view lock").output = "computer".into();
            self.set_state(app, "ready", guidance, None);
        }
        Ok(self.snapshot())
    }

    fn update_mode_setting(&self, kind: &str, value: &str) -> Result<(), String> {
        let (key, normalized) = match (kind, value) {
            ("transport", "usb") => ("preferredTransport", "Usb"),
            ("transport", "wifi") => ("preferredTransport", "Wifi"),
            ("playback", "balanced") => ("playbackMode", "Balanced"),
            ("playback", "lowLatency") => ("playbackMode", "LowLatency"),
            _ => return Err("未知的连接或播放模式。".into()),
        };
        let mut view = self.view.lock().expect("view lock");
        let mut settings = view.settings.clone();
        match key {
            "preferredTransport" => settings.preferred_transport = normalized.into(),
            "playbackMode" => settings.playback_mode = normalized.into(),
            _ => unreachable!(),
        }
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }

    pub fn open_logs(&self) -> Result<(), String> {
        let directory = self.data_dir.join("logs");
        std::process::Command::new("explorer.exe").arg(directory).spawn().map(|_| ()).map_err(|_| "无法打开日志文件夹。".into())
    }

    async fn teardown(&self, app: Option<&AppHandle>) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Some(mut session) = self.session.lock().await.take() {
            { let mut view = self.view.lock().expect("view lock"); view.session_active = false; view.session_serial = None; view.session_transport = None; view.session_android_version = None; view.audio_bitrate_bps = None; }
            if let Err(error) = self.audio.stop_and_cleanup() {
                errors.push(format!("本机音频采集停止失败：{error}"));
            }
            if let Err(error) = self.audio.restore_muted() {
                errors.push(format!("电脑声音恢复失败：{error}"));
            }
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
            if let Err(error) = adb_run(&self.adb_path, Some(&session.serial), &["reverse", "--remove", &format!("localabstract:{}", session.reverse)], Duration::from_secs(5), None).await.and_then(|result| if result.success { Ok(()) } else { Err("无法移除本次音频连接映射。".into()) }) {
                errors.push(format!("ADB 映射清理失败：{error}"));
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
        if matches!(key, "preferredTransport" | "playbackMode") && self.session.try_lock().map_or(true, |session| session.is_some()) {
            return Err("请使用连接方式或播放模式控件切换当前会话。".into());
        }
        let mut settings = view.settings.clone();
        match key {
            "autoConnectLastDevice" => settings.auto_connect_last_device = value.as_bool().ok_or("无效设置值")?,
            "minimizeToTray" => settings.minimize_to_tray = value.as_bool().ok_or("无效设置值")?,
            "launchAtLogin" => settings.launch_at_login = value.as_bool().ok_or("无效设置值")?,
            "autoReconnectWifi" => settings.auto_reconnect_wifi = value.as_bool().ok_or("无效设置值")?,
            "preferredTransport" => settings.preferred_transport = normalize_transport(value.as_str().ok_or("无效连接方式")?).into(),
            "playbackMode" => settings.playback_mode = normalize_playback(value.as_str().ok_or("无效播放模式")?).into(),
            "appearanceTheme" => settings.appearance_theme = normalize_theme(value.as_str().ok_or("无效主题")?).into(),
            _ => return Err("未知设置项。".into()),
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

fn commit_pcm_bitrate_sample(current_session: Option<&str>, expected_session: &str,
    previous: &mut Option<(String, u64, Instant)>, current_rate: &mut Option<u64>, bytes: u64, now: Instant) -> bool {
    if current_session != Some(expected_session) { return false; }
    let bitrate = calculate_pcm_bitrate(previous.as_ref().map(|(id, count, at)| (id.as_str(), *count, *at)),
        expected_session, bytes, now);
    *previous = Some((expected_session.to_owned(), bytes, now));
    if *current_rate == Some(bitrate) { false }
    else { *current_rate = Some(bitrate); true }
}

fn calculate_pcm_bitrate(previous: Option<(&str, u64, Instant)>, session_id: &str, bytes: u64, now: Instant) -> u64 {
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
    settings.playback_mode = match settings.playback_mode.to_ascii_lowercase().as_str() { "lowlatency" | "low_latency" => "LowLatency".into(), _ => "Balanced".into() };
    settings.appearance_theme = match settings.appearance_theme.to_ascii_lowercase().as_str() { "light" => "Light".into(), "dark" => "Dark".into(), _ => "System".into() };
    settings
}

fn normalize_transport(value: &str) -> &'static str { if value.eq_ignore_ascii_case("wifi") { "Wifi" } else { "Usb" } }
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
        assert_eq!(calculate_pcm_bitrate(Some(("session-a", 0, started)), "session-a", 192_000, started + Duration::from_secs(1)), 1_536_000);
        assert_eq!(calculate_pcm_bitrate(Some(("session-a", 900_000, started)), "session-b", 192_000, started + Duration::from_secs(1)), 0);
        assert_eq!(calculate_pcm_bitrate(Some(("session-a", 192_000, started)), "session-a", 192_000, started + Duration::from_secs(1)), 0);
    }

    #[test]
    fn stale_pcm_sample_is_rejected_after_session_changes() {
        let started = Instant::now();
        let mut previous = Some(("session-b".into(), 10, started));
        let mut current_rate = None;
        assert!(!commit_pcm_bitrate_sample(Some("session-b"), "session-a", &mut previous,
            &mut current_rate, 192_010, started + Duration::from_secs(1)));
        assert_eq!(previous.as_ref().map(|(id, count, _)| (id.as_str(), *count)), Some(("session-b", 10)));
        assert_eq!(current_rate, None);
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
        assert_eq!(value["playbackMode"], "Balanced");
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
    fn background_operation_does_not_preempt_a_foreground_request() {
        let generation = AtomicU64::new(0);
        let slot = Mutex::new(None);
        let (foreground_generation, foreground_token) = begin_operation_slot(&generation, &slot, false).unwrap();
        assert!(begin_operation_slot(&generation, &slot, true).is_none());
        assert!(slot.lock().unwrap().as_ref().is_some_and(|active| active.generation == foreground_generation));
        assert!(!foreground_token.is_cancelled());
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
}
