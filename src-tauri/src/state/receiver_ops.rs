use super::*;

impl AppState {
    pub(crate) async fn begin_receiver_operation<'a>(
        &'a self,
        reject_active_audio: bool,
    ) -> Result<(u64, CancellationToken, tokio::sync::MutexGuard<'a, ()>), String> {
        let (generation, token) =
            begin_operation_slot(&self.operation_generation, &self.operation, true)
                .ok_or_else(|| "AnPod 正在执行其他设备操作。".to_string())?;
        let guard = self.gate.lock().await;
        if !self.operation_is_current(generation, &token) {
            self.finish_operation(generation);
            return Err("Receiver 管理操作已取消。".into());
        }
        let audio_active = self
            .session
            .try_lock()
            .map_or(true, |session| session.is_some());
        if reject_active_audio && audio_active {
            self.finish_operation(generation);
            return Err("AnPod 音频会话正在使用此设备；Receiver 管理不会打断它。".into());
        }
        Ok((generation, token, guard))
    }

    pub async fn receiver_status(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = receiver::status(
            &self.adb_path,
            serial,
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
            &token,
        )
        .await;
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_install(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = async {
            receiver::validate_usb_install(
                &self.adb_path,
                serial,
                &self.receiver_apk_path,
                &self.receiver_metadata_path,
                &token,
            )
            .await?;
            self.ensure_receiver_operation_target(serial, None).await?;
            let has_session = self.session.lock().await.is_some();
            if token.is_cancelled() {
                return Err("Receiver 管理操作已取消。".into());
            }
            if has_session {
                self.teardown(None).await?;
            }
            receiver::install(
                &self.adb_path,
                serial,
                &self.receiver_apk_path,
                &self.receiver_metadata_path,
                &token,
            )
            .await
        }
        .await;
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_upgrade_usb(
        &self,
        serial: &str,
        binding_reference: &str,
    ) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = async {
            let path = binding_store_path(&self.data_dir, binding_reference)?;
            let actual_reference =
                receiver::binding_reference(&self.adb_path, serial, &token).await?;
            if actual_reference != binding_reference {
                return Err(
                    "USB 设备序列号或 Android 用户与所选 Receiver 绑定不匹配；未执行升级。".into(),
                );
            }
            receiver::validate_usb_upgrade(
                &self.adb_path,
                serial,
                binding_reference,
                &self.receiver_apk_path,
                &path,
                &self.receiver_metadata_path,
                &token,
            )
            .await?;
            self.ensure_receiver_operation_target(serial, Some(binding_reference))
                .await?;
            let has_session = self.session.lock().await.is_some();
            if token.is_cancelled() {
                return Err("Receiver 管理操作已取消。".into());
            }
            if has_session {
                self.teardown(None).await?;
            }
            receiver::upgrade_usb(
                &self.adb_path,
                serial,
                binding_reference,
                &path,
                &self.receiver_apk_path,
                &self.receiver_metadata_path,
                &token,
            )
            .await
        }
        .await;
        self.finish_operation(generation);
        result
    }

    pub(crate) async fn ensure_receiver_operation_target(
        &self,
        serial: &str,
        binding_reference: Option<&str>,
    ) -> Result<(), String> {
        let session = self.session.lock().await;
        let Some(active) = session.as_ref() else {
            return Ok(());
        };
        let matches = receiver_target_matches(
            &active.serial,
            active.network_owner.is_some(),
            active.network_binding_ref.as_deref(),
            serial,
            binding_reference,
        );
        if matches {
            Ok(())
        } else {
            Err("AnPod 正在使用另一台设备；未中断当前会话或执行 Receiver 操作。".into())
        }
    }

    pub async fn receiver_start(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(true).await?;
        let result = receiver::start(
            &self.adb_path,
            serial,
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
            &token,
        )
        .await;
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_health(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = receiver::health(
            &self.adb_path,
            serial,
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
            &token,
        )
        .await;
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_stop(&self, serial: &str) -> Result<ReceiverStatus, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(true).await?;
        let result = receiver::stop(
            &self.adb_path,
            serial,
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
            &token,
        )
        .await;
        self.finish_operation(generation);
        result
    }

    pub fn network_interfaces(&self) -> Result<Vec<Ipv4Interface>, String> {
        network_interface::enumerate_private_ipv4()
    }

    pub async fn receiver_network_interfaces(
        &self,
        serial: &str,
    ) -> Result<Vec<receiver::ReceiverInterface>, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = receiver::network_interfaces(
            &self.adb_path,
            serial,
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
            &token,
        )
        .await;
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
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "dat")
            {
                if let Some(reference) = entry.path().file_stem().and_then(|stem| stem.to_str()) {
                    if valid_binding_reference(reference) {
                        references.push(reference.to_owned());
                    }
                }
            }
        }
        references.sort();
        Ok(references)
    }

    pub async fn receiver_binding_reset_preview(
        &self,
        serial: &str,
    ) -> Result<receiver::BindingResetPreview, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        #[cfg(windows)]
        let result = receiver::binding_reset_preview(
            &self.adb_path,
            serial,
            &self.data_dir,
            &self.receiver_apk_path,
            &self.receiver_metadata_path,
            &token,
        )
        .await;
        #[cfg(not(windows))]
        let result = Err("USB 绑定重置仅在 Windows 上可用。".into());
        self.finish_operation(generation);
        result
    }

    pub async fn receiver_reset_binding(
        &self,
        expected: receiver::BindingResetPreview,
    ) -> Result<receiver::BindingResetResult, String> {
        let (generation, token, _guard) = self.begin_receiver_operation(false).await?;
        let result = async {
            let before = self
                .receiver_binding_reset_preview_under_gate(&expected.serial, &token)
                .await?;
            validate_receiver_reset_preview(&before, &expected)?;
            let has_session = self.session.lock().await.is_some();
            if has_session {
                self.ensure_receiver_operation_target(
                    &expected.serial,
                    Some(&expected.binding_reference),
                )
                .await?;
            } else if before.receiver_busy {
                return Err(
                    "手机仍有活动的接收或播放会话；请先在手机上停止接收，再重新预览。未删除绑定。"
                        .into(),
                );
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
                    receiver::stop(
                        &self.adb_path,
                        &expected.serial,
                        &self.receiver_apk_path,
                        &self.receiver_metadata_path,
                        &token,
                    )
                    .await?;
                }
            }
            if !self.operation_is_current(generation, &token) {
                return Err("绑定重置已取消；绑定仍保留。".into());
            }
            let after_stop = self
                .receiver_binding_reset_preview_under_gate(&expected.serial, &token)
                .await?;
            validate_receiver_reset_preview(&after_stop, &expected)?;
            if after_stop.receiver_busy {
                return Err(
                    "手机仍报告活动接收或播放状态；请在手机上停止接收后重试。绑定未删除。".into(),
                );
            }
            let is_current = || self.operation_is_current(generation, &token);
            receiver::reset_binding(
                &self.adb_path,
                &after_stop,
                &self.data_dir,
                &self.receiver_apk_path,
                &self.receiver_metadata_path,
                &token,
                &is_current,
            )
            .await
        }
        .await;
        if let Ok(reset) = &result {
            self.presence
                .lock()
                .expect("presence lock")
                .forget(&expected.binding_reference);
            if !reset_preserves_wireless_listener(reset, &expected) {
                let dropped = {
                    let mut listener = self
                        .host_offer_listener
                        .lock()
                        .expect("offer listener lock");
                    if listener.as_ref().is_some_and(|current| {
                        wireless_listener_matches_binding(
                            current.automatic_binding.as_deref(),
                            &expected.binding_reference,
                        )
                    }) {
                        listener.take()
                    } else {
                        None
                    }
                };
                drop(dropped);
            }
        }
        self.finish_operation(generation);
        result
    }

    pub(crate) async fn receiver_binding_reset_preview_under_gate(
        &self,
        serial: &str,
        token: &CancellationToken,
    ) -> Result<receiver::BindingResetPreview, String> {
        #[cfg(windows)]
        {
            receiver::binding_reset_preview(
                &self.adb_path,
                serial,
                &self.data_dir,
                &self.receiver_apk_path,
                &self.receiver_metadata_path,
                token,
            )
            .await
        }
        #[cfg(not(windows))]
        {
            let _ = (serial, token);
            Err("USB 绑定重置仅在 Windows 上可用。".into())
        }
    }
}
