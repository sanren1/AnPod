use super::*;

impl AppState {
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
            let old_settings = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .settings
                .clone();
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
                    let mut view = self
                        .view
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                    let mut view = self
                        .view
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                let mut view = self
                    .view
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                    let mut view = self
                        .view
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                    let mut view = self
                        .view
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                    let mut view = self
                        .view
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub(super) async fn reconfigure_locked(
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
                        let mut view = self
                            .view
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            let view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            self.view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .output = "computer".into();
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
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            self.view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .output = "computer".into();
            self.set_state(app, "ready", guidance, None);
        }
        Ok(self.snapshot())
    }

    pub(super) fn update_mode_setting(&self, kind: &str, value: &str) -> Result<(), String> {
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
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        view.settings = settings;
        Ok(())
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

    pub(super) async fn set_output_operation(
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

    pub(super) async fn set_output_locked(
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
                    let mut view = self
                        .view
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
                    let mut view = self
                        .view
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                let mut view = self
                    .view
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                .unwrap_or_else(std::sync::PoisonError::into_inner)
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
                    let mut view = self
                        .view
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                let mut view = self
                    .view
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                        let mut view = self
                            .view
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            view.state = "error".into();
            view.message = message;
            view.status_detail = "音频输出状态异常；请断开连接以恢复电脑声音。".into();
            return Err(restore.map_or(error.clone(), |restore| {
                format!("{error}；恢复电脑声音失败：{restore}")
            }));
        }
        if output == "computer" {
            if let Err(error) = self.audio.restore_muted() {
                let mut view = self
                    .view
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                view.state = "error".into();
                view.message = format!("电脑声音恢复失败：{error}");
                view.status_detail = "电脑声音恢复待处理；可断开连接或从托盘重试。".into();
                return Err(error);
            }
        }
        {
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
}
