use super::*;

impl AppState {
    pub(super) fn remember_pending_adb_cleanup(&self, cleanup: PendingAdbCleanup) {
        let mut pending = self
            .pending_adb_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub(super) async fn cleanup_pending_adb_for_target(
        &self,
        serial: &str,
        transport: &str,
        cancel: &CancellationToken,
    ) -> Result<AdbCleanupOutcome, String> {
        let mut pending = self
            .pending_adb_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
                .unwrap_or_else(std::sync::PoisonError::into_inner)
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
                .unwrap_or_else(std::sync::PoisonError::into_inner)
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

    pub(super) async fn cleanup_adb_remote(
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

    pub(super) async fn teardown_adb_for_recovery(
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
            let cleanup = pending_adb_cleanup_for(active);
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

    pub(super) async fn teardown(&self, app: Option<&AppHandle>) -> Result<(), String> {
        self.cancel_active_session(false);
        self.host_offer_listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
            if let Some(forward) = session.hdc_forward.take() {
                if let Err(error) = self.remove_hdc_forward(&forward).await {
                    errors.push(format!("hdc 端口转发清理失败：{error}"));
                }
            }
            if adb_stream {
                let mut cleanup = pending_adb_cleanup_for(&session);
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
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                );
                if !tail.is_empty() {
                    crate::applog::info("android", "server_output.before_teardown", &tail);
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
        *self
            .pcm_rate_sample
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        {
            let mut view = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
}
