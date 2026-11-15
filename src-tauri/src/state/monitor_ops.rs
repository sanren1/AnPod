use super::*;

impl AppState {
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
                crate::applog::warn("devices", "adb_scan.failed", &error);
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
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
                crate::applog::warn("session", "phone_audio.lost", &detail);
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

    pub(super) fn start_session_audio_sink(
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

    pub(super) async fn monitor_network_session(&self, app: &AppHandle) -> bool {
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
        let harmony = owner.harmony();
        let needs_bootstrap = snapshot.state == SessionLifecycleState::Stopped;
        let management_only = snapshot.state == SessionLifecycleState::Authenticated
            && snapshot
                .capabilities
                .as_ref()
                .is_some_and(|info| info.management_compatible && !info.compatible);
        let queue = owner.audio_queue();
        let mut failure = self.audio.take_last_error().ok().flatten();
        if failure.is_none() && needs_bootstrap {
            failure = Some(hdc_ops::stopped_session_error(
                snapshot.last_error.clone(),
                harmony,
            ));
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
            // A tablet restarts from the tablet, not through a phone USB bootstrap.
            if needs_bootstrap && !harmony {
                detail.push_str("；请重新通过授权 USB/bootstrap 启动 Receiver");
            }
            let retired_generation = active.generation;
            *session = None;
            self.clear_session_cancel(retired_generation);
            {
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
                view.message = "LAN 音频会话已停止".into();
                view.status_detail = format!("本机音频已清理；{detail}");
                view.audio_bitrate_bps = None;
            }
            *self
                .pcm_rate_sample
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
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
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    // Recovery inputs must stay frozen to the generation observed by the caller.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn recover_session_locked(
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
        crate::applog::warn("session", "recovery.started", &reason);
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
            crate::applog::warn("session", "recovery.failed", &message);
            self.set_state(app, "error", message, device_name);
        }
        self.recovering.store(false, Ordering::Release);
    }
}
