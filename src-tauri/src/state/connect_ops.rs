use super::*;

impl AppState {
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
            // Reject before teardown so the current session stays untouched.
            let target = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .devices
                .iter()
                .find(|device| device.serial == serial)
                .cloned();
            if let Some(target) = &target {
                ensure_android_device(target)?;
            }
            let (device_available, cleanup_pending) = {
                let view = self
                    .view
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                    let mut view = self
                        .view
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub(super) async fn connect_device_mode(
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

    pub(super) async fn connect_locked(
        &self,
        app: &AppHandle,
        serial: &str,
        generation: u64,
        token: &CancellationToken,
        play_on_phone: bool,
    ) -> Result<OperationResult, String> {
        if play_on_phone {
            let settings = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .settings
                .clone();
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
            let view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            view.devices
                .iter()
                .find(|d| d.serial == serial && d.connectable)
                .cloned()
                .ok_or_else(|| "此手机当前不可连接，请刷新设备列表。".to_string())?
        };
        ensure_android_device(&device)?;
        self.view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .selected_serial = Some(device.serial.clone());
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
            } else if self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .devices
                .is_empty()
            {
                "noDevice"
            } else {
                "ready"
            };
            self.set_state(app, idle_state, &message, None);
            self.view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .output = "computer".into();
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

    pub(super) async fn connect_adb_stream_inner(
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
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        *self
            .pcm_rate_sample
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;

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
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub(super) async fn connect_inner(
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
        let settings = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .settings
            .clone();
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
        let installed_receiver = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .settings
            .audio_backend
            == "receiver";
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
        self.view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .session_generation = Some(session_generation);
        {
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        *self
            .pcm_rate_sample
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let mode = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
            if let Some(stdout) = child.stdout.take() {
                tokio::spawn(capture_child_output(stdout, "stdout", output_tail.clone()));
            }
            if let Some(stderr) = child.stderr.take() {
                tokio::spawn(capture_child_output(stderr, "stderr", output_tail.clone()));
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
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        crate::applog::info("session", "connected", format_args!("state={state}"));
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
        crate::applog::info("session", "disconnected.user", "");
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
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if view.session_transport.as_deref() == Some("aoa") {
            return Err("USB Direct 会话没有 ADB serial；请先断开后再选择 ADB 设备。".into());
        }
        if view.is_busy() {
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
}
