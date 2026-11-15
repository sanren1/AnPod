use super::*;

impl AppState {
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

    #[cfg(windows)]
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn start_network_audio_inner(
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
            let audio_quality = self.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner).settings.audio_quality.clone();
            validate_audio_quality(&audio_quality)?;
            if self.session.lock().await.is_some() { return Err("AnPod 已有活动会话；请先断开后再启动网络接收。".into()); }
            {
                let mut listener = self.host_offer_listener.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some((_, _, cancellation)) = &intent {
                    token_check(cancellation)?;
                    listener.as_mut().ok_or("无线接收已停止；旧连接请求无效。")?.attempt_cancel = Some(token.clone());
                } else { listener.take(); }
            }
            let interface = self.network_interfaces()?.into_iter().find(|item| item.address.to_string() == local_address)
                .ok_or_else(|| "所选 IPv4 接口已不可用；请重新读取接口列表。".to_string())?;
            audio_initialized = true;
            self.audio.initialize()?;
            let playback_mode = if self.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner).settings.playback_mode.eq_ignore_ascii_case("lowlatency") { 1 } else { 0 };
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
                    return Err(crate::receiver::receiver_update_required("Receiver 网络协议版本不兼容。"));
                }
                let path = binding_store_path(&self.data_dir, &reference)?;
                receiver::provision(&self.adb_path, serial, &self.receiver_apk_path,
                    &self.receiver_metadata_path, &path, &token).await?;
                let model = self.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                    .devices.iter().find(|device| device.serial == serial).map(|device| device.display_name.clone());
                if let Some(model) = model.filter(|model| !model.is_empty()) {
                    if let Err(error) = update_binding_profile(&self.data_dir, &reference, |profile| profile.model = Some(model)) {
                        crate::applog::warn("binding", "profile.save_failed", &error);
                    }
                }
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
                mode: if self.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner).settings.playback_mode.eq_ignore_ascii_case("lowlatency") { 1 } else { 0 } };
            let connection_intent = intent.as_ref().map(|(nonce, required, _)| (*nonce, *required));
            let mut owner = SessionOwner::start_on_interface_with_profile(
                &interface,
                peer,
                binding,
                parameters,
                profile,
                load_binding_profile(&self.data_dir, &binding_ref).platform == "harmony",
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
                let mut listener = self.host_offer_listener.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
                hdc_forward: None,
            });
            if !self.install_session_cancel(0, session_generation, generation, &token, cancel_signal) {
                failed_install_signal();
                self.teardown(Some(app)).await?;
                return Err("无线会话已取消。".into());
            }
            {
                let mut view = self.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
            *self.pcm_rate_sample.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            self.emit(app);
            Ok(self.snapshot())
        }.await;
        if result.is_err() && audio_initialized {
            let _ = self.audio.stop_pcm_sink();
            let _ = self.audio.stop_and_cleanup();
            if self.audio.restore_muted().is_err() {
                let mut view = self
                    .view
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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
}
