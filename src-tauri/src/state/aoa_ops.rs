use super::*;

impl AppState {
    pub(super) fn prepare_usb_direct_configuration_operation(&self) -> Result<(), String> {
        let mut current = self
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                let mut view = self
                    .view
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            let view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if view.is_busy() || view.session_active {
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
                let mut view = self.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if view.is_busy() || view.session_active {
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
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        if load_binding_profile(&self.data_dir, binding_reference).platform != "android" {
            return Err("USB Direct 只能连接安卓手机绑定。".into());
        }
        let (generation, token) = self.start_operation();
        crate::applog::info(
            "connect",
            "usb_direct.phase",
            format_args!("generation={generation} phase=waiting_for_gate"),
        );
        let _gate = self.gate.lock().await;
        crate::applog::info(
            "connect",
            "usb_direct.phase",
            format_args!("generation={generation} phase=gate_acquired"),
        );
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
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            let settings = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .settings
                .clone();
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
                hdc_forward: None,
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
                let mut view = self
                    .view
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            *self
                .pcm_rate_sample
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
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
        crate::applog::info(
            "connect",
            "usb_direct.finished",
            format_args!("generation={generation} outcome={outcome}"),
        );
        if result.is_err() && audio_initialized {
            let _ = self.audio.stop_pcm_sink();
            let _ = self.audio.stop_and_cleanup();
            let _ = self.audio.restore_muted();
        }
        self.finish_operation(generation);
        if result.is_err() && operation_generation_matches(&self.operation_generation, generation) {
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    #[cfg(not(windows))]
    pub async fn start_aoa_audio(
        &self,
        _app: &AppHandle,
        _target_id: &str,
        _binding_reference: &str,
    ) -> Result<AppSnapshot, String> {
        Err("USB Direct WinUSB 仅在 Windows 上可用。".into())
    }

    #[cfg(windows)]
    pub(super) async fn create_aoa_owner(
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
            *worker_cancel_slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cancel.clone());
            if worker_token.is_cancelled() {
                cancel.cancel();
            }
            AoaOwner::connect(stream, material, format, worker_queue, startup_diagnostics)
        });
        let mut worker = worker;
        let owner = tokio::select! {
            result = &mut worker => result.map_err(|error| format!("USB Direct worker failed: {error}"))??,
            _ = token.cancelled() => {
                if let Some(cancel) = cancel_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
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
    pub(super) async fn restart_aoa_session_locked(
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
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub(super) fn clear_aoa_session_after_restart_failure(
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
        crate::applog::warn("aoa", "session.restart_failed", &detail);
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
        view.output = "computer".into();
        view.state = "error".into();
        view.message = "USB Direct 会话重启失败".into();
        view.status_detail = format!("本机音频和 USB Direct 会话已清理：{detail}");
        detail
    }

    pub(super) async fn monitor_aoa_session(&self, app: &AppHandle) -> bool {
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
        match record_stream_exit(
            snapshot.terminal_cause,
            snapshot.capabilities.capability_bits,
        ) {
            Some(RecordStreamExit::Finish(message)) => {
                let generation = active.generation;
                drop(session);
                self.finish_aoa_terminal(generation, app, message).await;
                return true;
            }
            Some(RecordStreamExit::Recover) => {
                let generation = active.generation;
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
            None => {}
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
                *self
                    .pcm_rate_sample
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                return true;
            }
            crate::applog::warn("aoa", "session.failed", &error);
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
            *self
                .pcm_rate_sample
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            return true;
        }
        let next_state = match snapshot.state {
            crate::p5c::ReceiverState::Streaming => "streaming",
            crate::p5c::ReceiverState::Paused => "paused",
            crate::p5c::ReceiverState::ReceiverRunning => "phoneReady",
            _ => "connected",
        };
        let changed = {
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub(super) async fn finish_aoa_terminal(
        &self,
        generation: u64,
        app: &AppHandle,
        message: &str,
    ) {
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
                self.view
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .status_detail = error;
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
        crate::applog::warn("aoa", "session.ended", &detail);
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
        view.state = if cleanup.is_empty() { "ready" } else { "error" }.into();
        view.message = detail.clone();
        view.status_detail = detail;
        view.audio_bitrate_bps = None;
        drop(view);
        drop(session);
        *self
            .pcm_rate_sample
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.emit(app);
    }

    #[cfg(windows)]
    // Recovery inputs must stay frozen to the generation observed by the caller.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn recover_aoa_session_locked(
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
        crate::applog::warn("aoa", "recovery.started", &reason);
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
            let (owner, queue, mix_format) = match self
                .create_aoa_owner(&target_id, &binding, format, &token)
                .await
            {
                Ok(created) => created,
                Err(error) => {
                    crate::applog::warn(
                        "aoa",
                        "recovery.attempt_failed",
                        format_args!("attempt={} {error}", attempt + 1),
                    );
                    continue;
                }
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
                crate::applog::warn("aoa", "recovery.output_setup_failed", &error);
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
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            crate::applog::info(
                "aoa",
                "recovery.succeeded",
                format_args!("attempt={}", attempt + 1),
            );
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
}
