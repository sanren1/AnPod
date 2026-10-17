use super::*;

pub(crate) static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) fn begin_operation_slot(
    generation: &AtomicU64,
    slot: &Mutex<Option<ActiveOperation>>,
    background: bool,
) -> Option<(u64, CancellationToken)> {
    let mut current = slot.lock().expect("operation lock");
    if background && current.is_some() {
        return None;
    }
    let next_generation = generation.fetch_add(1, Ordering::AcqRel) + 1;
    let token = CancellationToken::new();
    if let Some(previous) = current.as_ref() {
        previous.token.cancel();
    }
    *current = Some(ActiveOperation {
        generation: next_generation,
        token: token.clone(),
    });
    Some((next_generation, token))
}

pub(crate) fn cancel_operation_slot(generation: &AtomicU64, slot: &Mutex<Option<ActiveOperation>>) {
    let mut current = slot.lock().expect("operation lock");
    generation.fetch_add(1, Ordering::AcqRel);
    if let Some(operation) = current.as_ref() {
        operation.token.cancel();
    }
    *current = None;
}

pub(crate) fn finish_operation_slot(slot: &Mutex<Option<ActiveOperation>>, generation: u64) {
    let mut current = slot.lock().expect("operation lock");
    if current
        .as_ref()
        .is_some_and(|active| active.generation == generation)
    {
        *current = None;
    }
}

pub(crate) fn session_is_lost(
    device_present: Option<bool>,
    process_exited: bool,
    audio_connected: bool,
) -> bool {
    process_exited || !audio_connected || device_present == Some(false)
}

pub(crate) fn receiver_health_needed(
    installed_receiver: bool,
    device_present: Option<bool>,
    process_exited: bool,
    audio_connected: bool,
) -> bool {
    installed_receiver && !session_is_lost(device_present, process_exited, audio_connected)
}

pub(crate) fn merge_usb_playback_diagnostics(
    mut diagnostics: PlaybackDiagnostics,
    format: Option<PcmFormat>,
    generation: u64,
    previous: Option<(u64, u32)>,
) -> (PlaybackDiagnostics, Option<(u64, u32)>) {
    if let Some(format) = format {
        diagnostics.sample_rate = Some(format.sample_rate);
        diagnostics.channels = u8::try_from(format.channels).ok();
    }
    diagnostics.underrun_increment = match (previous, diagnostics.underrun_count) {
        (Some((sample_generation, previous)), Some(current))
            if sample_generation == generation && current >= previous =>
        {
            Some(u64::from(current - previous))
        }
        _ => None,
    };
    let next = diagnostics.underrun_count.map(|count| (generation, count));
    (diagnostics, next)
}

pub(crate) fn commit_pcm_bitrate_sample(
    current_session: Option<u64>,
    expected_session: u64,
    previous: &mut Option<(u64, u64, Instant)>,
    current_rate: &mut Option<u64>,
    bytes: u64,
    now: Instant,
) -> bool {
    if current_session != Some(expected_session) {
        return false;
    }
    let bitrate = calculate_pcm_bitrate(
        previous.as_ref().map(|(id, count, at)| (*id, *count, *at)),
        expected_session,
        bytes,
        now,
    );
    *previous = Some((expected_session, bytes, now));
    if *current_rate == Some(bitrate) {
        false
    } else {
        *current_rate = Some(bitrate);
        true
    }
}

pub(crate) fn calculate_pcm_bitrate(
    previous: Option<(u64, u64, Instant)>,
    session_id: u64,
    bytes: u64,
    now: Instant,
) -> u64 {
    let Some((previous_session, previous_bytes, previous_at)) = previous else {
        return 0;
    };
    if previous_session != session_id {
        return 0;
    }
    let elapsed_nanos = now.saturating_duration_since(previous_at).as_nanos();
    if elapsed_nanos == 0 {
        return 0;
    }
    let delta = bytes.saturating_sub(previous_bytes) as u128;
    (delta.saturating_mul(8_000_000_000) / elapsed_nanos).min(u64::MAX as u128) as u64
}

pub(crate) fn same_physical_device(left: &Device, right: &Device) -> bool {
    left.physical_id
        .as_ref()
        .is_some_and(|id| right.physical_id.as_ref() == Some(id))
}

pub(crate) fn receiver_target_matches(
    active_serial: &str,
    active_network: bool,
    active_binding: Option<&str>,
    target_serial: &str,
    target_binding: Option<&str>,
) -> bool {
    if active_network {
        target_binding.is_some() && active_binding == target_binding
    } else {
        active_serial == target_serial
    }
}

pub(crate) fn wireless_listener_matches_binding(
    listener_binding: Option<&str>,
    target_binding: &str,
) -> bool {
    listener_binding == Some(target_binding)
}

pub(crate) fn reset_preserves_wireless_listener(
    result: &receiver::BindingResetResult,
    expected: &receiver::BindingResetPreview,
) -> bool {
    result.serial == expected.serial
        && result.android_user == expected.android_user
        && result.binding_reference == expected.binding_reference
        && result.phone_state == "saved"
        && result.phone_commitment == expected.phone_commitment
        && result.pc_state == "saved"
        && result.pc_commitment == expected.pc_commitment
}

pub(crate) fn validate_receiver_reset_preview(
    actual: &receiver::BindingResetPreview,
    expected: &receiver::BindingResetPreview,
) -> Result<(), String> {
    if actual.serial != expected.serial
        || actual.android_user != expected.android_user
        || actual.binding_reference != expected.binding_reference
        || actual.phone_state != expected.phone_state
        || actual.phone_commitment != expected.phone_commitment
        || actual.pc_state != expected.pc_state
        || actual.pc_commitment != expected.pc_commitment
    {
        return Err("重置预览已过期（设备、Android user 或绑定摘要发生变化）；未停止会话或删除绑定，请重新检查。".into());
    }
    if actual.phone_state == "pending" || actual.phone_state == "failed" {
        return Err(
            "手机绑定状态正在写入或不可读取；请完成/检查绑定操作后重新预览。未停止会话或删除绑定。"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn valid_binding_reference(reference: &str) -> bool {
    reference.len() == 64 && reference.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn background_trust_status(
    snapshot: BackgroundTrustSnapshot,
    target: &str,
    binding_reference: String,
    session_generation: Option<u64>,
) -> Result<BackgroundTrustStatus, String> {
    let (kind, persistence) = match (snapshot.kind, snapshot.persistence) {
        (0, 0) => ("none", "none"),
        (1, 1) => ("once", "process"),
        (2, 1) => ("eightHours", "process"),
        (3, 2) => ("untilRevoked", "binding"),
        _ => return Err("Receiver returned an invalid background trust snapshot.".into()),
    };
    Ok(BackgroundTrustStatus {
        target: target.into(),
        binding_reference,
        session_generation,
        kind: kind.into(),
        persistence: persistence.into(),
        remaining_ms: (snapshot.remaining_ms != u64::MAX).then_some(snapshot.remaining_ms),
    })
}

pub(crate) fn automatic_reception_binding(
    references: &[String],
    selected: Option<&str>,
) -> Result<Option<String>, String> {
    match selected.filter(|value| !value.is_empty()) {
        Some(value) if references.iter().any(|item| item == value) => Ok(Some(value.to_owned())),
        Some(_) => Err("所选 Receiver 绑定已不可用；请重新选择。".into()),
        None if references.len() == 1 => Ok(references.first().cloned()),
        None => Ok(None),
    }
}

pub(crate) fn pcm_queue_identity_changed(
    current: Option<&Arc<PcmQueue>>,
    next: Option<&Arc<PcmQueue>>,
) -> bool {
    match (current, next) {
        (Some(current), Some(next)) => !Arc::ptr_eq(current, next),
        (None, None) => false,
        _ => true,
    }
}

pub(crate) fn monitored_network_output(current: &str, management_only: bool) -> &str {
    if management_only {
        "computer"
    } else {
        current
    }
}

pub(crate) fn binding_store_path(data_dir: &Path, reference: &str) -> Result<PathBuf, String> {
    if !valid_binding_reference(reference) {
        return Err("Receiver 绑定引用格式无效。".into());
    }
    Ok(data_dir
        .join("receiver-bindings")
        .join(format!("{}.dat", reference.to_ascii_lowercase())))
}

pub(crate) fn choose_selected_serial(
    session_serial: Option<String>,
    previous: Option<String>,
    devices: &[Device],
    preferred: &str,
) -> Option<String> {
    if session_serial.is_some() {
        return session_serial;
    }
    let visible: Vec<_> = devices
        .iter()
        .filter(|device| device.transport == preferred || device.transport == "unknown")
        .collect();
    previous
        .filter(|serial| visible.iter().any(|device| &device.serial == serial))
        .or_else(|| {
            let candidates: Vec<_> = visible.iter().filter(|device| device.connectable).collect();
            (candidates.len() == 1).then(|| candidates[0].serial.clone())
        })
}

#[derive(Default)]
pub(crate) struct AdbResult {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) async fn adb_run(
    adb: &Path,
    serial: Option<&str>,
    args: &[&str],
    limit: Duration,
    cancel: Option<&CancellationToken>,
) -> Result<AdbResult, String> {
    if !adb.is_file() {
        return Err(format!("找不到 ADB：{}", adb.display()));
    }
    let mut command = Command::new(adb);
    if let Some(serial) = serial {
        command.args(["-s", serial]);
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
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
        timeout(limit, wait)
            .await
            .map_err(|_| "ADB 命令超时。".to_string())?
            .map_err(|_| "ADB 命令失败。".to_string())?
    };
    Ok(AdbResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

pub(crate) async fn adb_run_with_stdin(
    adb: &Path,
    serial: Option<&str>,
    args: &[&str],
    input: &[u8],
    limit: Duration,
    cancel: &CancellationToken,
) -> Result<AdbResult, String> {
    use tokio::io::AsyncWriteExt;
    use tokio::time::{timeout_at, Instant as TokioInstant};

    if !adb.is_file() {
        return Err("找不到 ADB。".into());
    }
    let mut command = Command::new(adb);
    if let Some(serial) = serial {
        command.args(["-s", serial]);
    }
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|_| "无法启动 ADB。".to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "无法打开 ADB 私密输入管道。".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取 ADB 命令状态。".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取 ADB 命令状态。".to_string())?;
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
    let (stdout_present, stderr_present) =
        match drain_output_tasks(&mut output_tasks, deadline, cancel).await {
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
    let stdout = if stdout_present {
        "[output present]"
    } else {
        ""
    };
    let stderr = if stderr_present {
        "[output present]"
    } else {
        ""
    };
    Ok(AdbResult {
        success: status.success(),
        stdout: stdout.into(),
        stderr: stderr.into(),
    })
}

pub(crate) async fn stop_adb_child(
    child: &mut Child,
    output_tasks: &mut [Option<tokio::task::JoinHandle<bool>>; 2],
) {
    let _ = child.start_kill();
    let _ = timeout(Duration::from_secs(2), child.wait()).await;
    stop_output_tasks(output_tasks).await;
}

pub(crate) enum DrainError {
    Cancelled,
    TimedOut,
    Failed,
}

pub(crate) async fn drain_output_tasks(
    tasks: &mut [Option<tokio::task::JoinHandle<bool>>; 2],
    deadline: tokio::time::Instant,
    cancel: &CancellationToken,
) -> Result<(bool, bool), DrainError> {
    let stdout = await_output_task(&mut tasks[0], deadline, cancel).await?;
    let stderr = await_output_task(&mut tasks[1], deadline, cancel).await?;
    Ok((stdout, stderr))
}

pub(crate) async fn await_output_task(
    task: &mut Option<tokio::task::JoinHandle<bool>>,
    deadline: tokio::time::Instant,
    cancel: &CancellationToken,
) -> Result<bool, DrainError> {
    tokio::select! {
        _ = cancel.cancelled() => Err(DrainError::Cancelled),
        result = tokio::time::timeout_at(deadline, join_output_task(task)) => match result {
            Ok(Ok(presence)) => Ok(presence),
            Ok(Err(())) => Err(DrainError::Failed),
            Err(_) => Err(DrainError::TimedOut),
        },
    }
}

pub(crate) async fn join_output_task(
    task: &mut Option<tokio::task::JoinHandle<bool>>,
) -> Result<bool, ()> {
    let Some(handle) = task.as_mut() else {
        return Err(());
    };
    let result = handle.await.map_err(|_| ());
    task.take();
    result
}

pub(crate) async fn stop_output_tasks(tasks: &mut [Option<tokio::task::JoinHandle<bool>>; 2]) {
    for task in tasks.iter().flatten() {
        task.abort();
    }
    for task in tasks.iter_mut() {
        if let Some(task) = task.take() {
            let _ = task.await;
        }
    }
}

pub(crate) async fn drain_output_presence<R: tokio::io::AsyncRead + Unpin>(mut reader: R) -> bool {
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

pub(crate) async fn capture_child_output<R>(
    reader: R,
    stream: &'static str,
    tail: Arc<Mutex<VecDeque<String>>>,
    log_path: PathBuf,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let entry = format!("{stream}: {line}");
        if safe_android_diagnostic(&entry) {
            append_app_log(&log_path, &format!("Android server: {line}"));
        }
        let mut tail = tail.lock().expect("output log lock");
        tail.push_back(entry);
        while tail.len() > 100 {
            tail.pop_front();
        }
    }
}

pub(crate) fn safe_android_diagnostic(line: &str) -> bool {
    let line = line
        .strip_prefix("stdout: ")
        .or_else(|| line.strip_prefix("stderr: "))
        .unwrap_or(line);
    [
        "ANPOD AUDIO ",
        "ANPOD BUFFER_",
        "ANPOD START_THRESHOLD",
        "ANPOD LOW_LATENCY",
        "ANPOD PLAYING ",
        "ANPOD UNDERRUNS ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

pub(crate) fn has_restore_failure(error: &str) -> bool {
    error.contains("电脑声音恢复失败：")
}

pub(crate) fn safe_android_tail(lines: &[String]) -> String {
    lines
        .iter()
        .filter(|line| safe_android_diagnostic(line))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn append_app_log(path: &Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} {message}", chrono_stub_timestamp());
    }
}

pub(crate) async fn checked_adb(
    adb: &Path,
    serial: Option<&str>,
    args: &[&str],
    limit: Duration,
    cancel: Option<&CancellationToken>,
) -> Result<(), String> {
    let result = adb_run(adb, serial, args, limit, cancel).await?;
    let text = format!("{}\n{}", result.stdout, result.stderr).to_ascii_lowercase();
    if !result.success
        || [
            "failed",
            "failure",
            "unable to",
            "cannot",
            "connection refused",
            "timed out",
            "error:",
        ]
        .iter()
        .any(|m| text.contains(m))
    {
        return Err("ADB 未能完成此操作；请检查手机状态和连接地址。".into());
    }
    Ok(())
}

pub(crate) fn token_check(token: &CancellationToken) -> Result<(), String> {
    if token.is_cancelled() {
        Err("操作已取消。".into())
    } else {
        Ok(())
    }
}

pub(crate) fn parse_devices(text: &str) -> Vec<Device> {
    text.lines()
        .skip_while(|line| !line.starts_with("List of devices"))
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let serial = (*fields.first()?).to_owned();
            let state_index =
                if fields.get(1) == Some(&"no") && fields.get(2) == Some(&"permissions") {
                    3
                } else {
                    2
                };
            let state_raw = *fields.get(1)?;
            if serial.starts_with('*') {
                return None;
            }
            let tags: HashMap<_, _> = fields
                .iter()
                .skip(state_index)
                .filter_map(|field| field.split_once(':'))
                .collect();
            let state = match state_raw {
                "device" => "device",
                "unauthorized" => "unauthorized",
                "offline" => "offline",
                "no" if state_index == 3 => "noPermissions",
                _ => "unknown",
            };
            let transport = if serial.contains(':') || serial.contains("_adb-tls-") {
                "wifi"
            } else if tags.contains_key("usb") {
                "usb"
            } else {
                "unknown"
            };
            let manufacturer = tags
                .get("manufacturer")
                .copied()
                .unwrap_or("")
                .replace('_', " ");
            let model = tags.get("model").copied().unwrap_or("").replace('_', " ");
            let display_name = if model.is_empty() {
                serial.clone()
            } else if manufacturer.is_empty() {
                model.clone()
            } else {
                format!("{manufacturer} {model}")
            };
            let status_text = if transport == "unknown" && state == "device" {
                "连接方式待确认"
            } else {
                match state {
                    "device" => "可连接",
                    "unauthorized" => "等待手机授权",
                    "offline" => "设备暂时离线",
                    "noPermissions" => "缺少 USB 驱动权限",
                    _ => "状态未知",
                }
            };
            Some(Device {
                serial,
                physical_id: None,
                transport: transport.into(),
                state: state.into(),
                manufacturer,
                model,
                android_version: String::new(),
                api_level: String::new(),
                display_name,
                status_text: status_text.into(),
                connectable: state == "device" && transport != "unknown",
            })
        })
        .collect()
}

pub(crate) fn parse_getprop(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let close = line.find("]:")?;
            Some((
                line.get(1..close)?.into(),
                line.get(close + 2..)?
                    .trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .into(),
            ))
        })
        .collect()
}

pub(crate) fn normalize_endpoint(value: &str) -> Result<String, String> {
    let value = value.trim().trim_start_matches("tcp:");
    let (host, port) = value.rsplit_once(':').ok_or("地址格式应为 IPv4:端口。")?;
    host.parse::<std::net::Ipv4Addr>()
        .map_err(|_| "地址格式应为 IPv4:端口。")?;
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("端口需为 1–65535。".into());
    }
    let port: u16 = port.parse().map_err(|_| "端口需为 1–65535。")?;
    if port == 0 {
        return Err("端口需为 1–65535。".into());
    }
    Ok(format!("{host}:{port}"))
}

pub(crate) fn normalize_mdns_instance(value: &str) -> String {
    value
        .trim()
        .trim_end_matches('.')
        .replace("._adb-tls-pairing._tcp", "")
        .replace("._adb-tls-connect._tcp", "")
        .replace("._adb._tcp", "")
}

pub(crate) fn normalize_settings(mut settings: AppSettings) -> AppSettings {
    settings.schema_version = 1;
    settings.preferred_transport = match settings.preferred_transport.to_ascii_lowercase().as_str()
    {
        "wifi" => "Wifi".into(),
        _ => "Usb".into(),
    };
    settings.wireless_protocol = if settings.wireless_protocol.is_empty() {
        if settings.preferred_transport == "Wifi" {
            "adb"
        } else {
            "udp"
        }
        .into()
    } else {
        normalize_wireless_protocol(&settings.wireless_protocol).into()
    };
    settings.audio_backend = normalize_audio_backend(&settings.audio_backend).into();
    settings.playback_mode = match settings.playback_mode.to_ascii_lowercase().as_str() {
        "lowlatency" | "low_latency" => "LowLatency".into(),
        _ => "Balanced".into(),
    };
    settings.appearance_theme = match settings.appearance_theme.to_ascii_lowercase().as_str() {
        "light" => "Light".into(),
        "dark" => "Dark".into(),
        _ => "System".into(),
    };
    settings
}

pub(crate) fn normalize_transport(value: &str) -> &'static str {
    if value.eq_ignore_ascii_case("wifi") {
        "Wifi"
    } else {
        "Usb"
    }
}
pub(crate) fn normalize_wireless_protocol(value: &str) -> &'static str {
    if value.eq_ignore_ascii_case("adb") {
        "adb"
    } else {
        "udp"
    }
}
pub(crate) fn normalize_audio_backend(value: &str) -> &'static str {
    if value.eq_ignore_ascii_case("legacy") {
        "legacy"
    } else {
        "receiver"
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReconfigureSelection {
    Playback,
    AudioBackend,
    Usb,
    WirelessAdb,
    WirelessUdp,
}
pub(crate) fn wireless_adb_enabled(settings: &AppSettings) -> bool {
    settings.wireless_protocol == "adb"
}
pub(crate) fn selected_transport_allows_auto_adb(settings: &AppSettings) -> bool {
    !settings.preferred_transport.eq_ignore_ascii_case("wifi") || wireless_adb_enabled(settings)
}
pub(crate) fn reconfigure_selection(
    kind: &str,
    value: &str,
    settings: &AppSettings,
) -> Result<ReconfigureSelection, String> {
    match (kind, value) {
        ("playback", "balanced" | "lowLatency") => Ok(ReconfigureSelection::Playback),
        ("audioBackend", "receiver" | "legacy") => Ok(ReconfigureSelection::AudioBackend),
        ("transport", "usb") => Ok(ReconfigureSelection::Usb),
        ("transport", "wireless" | "wifi") => Ok(if settings.wireless_protocol == "adb" {
            ReconfigureSelection::WirelessAdb
        } else {
            ReconfigureSelection::WirelessUdp
        }),
        ("wirelessProtocol", "udp") => Ok(ReconfigureSelection::WirelessUdp),
        ("wirelessProtocol", "adb") => Ok(ReconfigureSelection::WirelessAdb),
        _ => Err("未知的连接或播放模式。".into()),
    }
}
pub(crate) fn selected_connection_transport(preferred: &str) -> &'static str {
    if preferred.eq_ignore_ascii_case("wifi") {
        "wireless"
    } else {
        "usb"
    }
}
pub(crate) fn session_connection_transport(transport: &str) -> Option<&'static str> {
    match transport {
        "usb" => Some("usb"),
        "wifi" | "lan" => Some("wireless"),
        _ => None,
    }
}
pub(crate) fn session_wireless_protocol(transport: &str) -> Option<&'static str> {
    match transport {
        "wifi" => Some("adb"),
        "lan" => Some("udp"),
        _ => None,
    }
}
pub(crate) fn normalize_playback(value: &str) -> &'static str {
    if value.eq_ignore_ascii_case("lowLatency") {
        "LowLatency"
    } else {
        "Balanced"
    }
}
pub(crate) fn normalize_theme(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "light" => "Light",
        "dark" => "Dark",
        _ => "System",
    }
}
pub(crate) fn uuid_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成本次连接的安全握手令牌。".to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
pub(crate) fn chrono_stub_timestamp() -> String {
    format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

pub fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let ok = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(source, destination)
    }
}
