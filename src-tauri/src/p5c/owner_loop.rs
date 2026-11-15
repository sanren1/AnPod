use super::*;

pub(super) enum BackgroundTrustCommandOutcome {
    Continue,
    Stop,
    ConnectionFailed(String),
}

pub(super) fn reject_background_trust_command(command: OwnerCommand, message: &str) {
    match command {
        OwnerCommand::BackgroundTrustQuery { reply, .. } => {
            let _ = reply.send(Err(message.into()));
        }
        OwnerCommand::BackgroundTrustRevoke { reply, .. } => {
            let _ = reply.send(Err(message.into()));
        }
        _ => {}
    }
}

pub(super) fn execute_background_trust_command(
    session: &mut HostSession,
    owner_generation: u64,
    enabled: &AtomicBool,
    command: OwnerCommand,
) -> BackgroundTrustCommandOutcome {
    match command {
        OwnerCommand::BackgroundTrustQuery {
            session_generation,
            deadline,
            cancellation,
            reply,
        } => {
            let result = if cancellation.is_cancelled() || Instant::now() >= deadline {
                Err("背景授权查询已取消或超时。".into())
            } else if session_generation != owner_generation {
                Err("活动会话代际已变化；请刷新授权状态后重试。".into())
            } else {
                session.query_background_trust_until(deadline, &cancellation)
            };
            let receiver_user_stopped =
                session.terminal_cause() == Some(TerminalCause::ReceiverUserStop);
            let connection_failed =
                !receiver_user_stopped && session.state() == ReceiverState::Stopped;
            let error = result.as_ref().err().cloned();
            let _ = reply.send(result);
            if receiver_user_stopped {
                enabled.store(false, Ordering::Release);
                BackgroundTrustCommandOutcome::Stop
            } else if connection_failed {
                BackgroundTrustCommandOutcome::ConnectionFailed(error.unwrap_or_else(|| {
                    "P5-C control channel closed during background trust query.".into()
                }))
            } else {
                BackgroundTrustCommandOutcome::Continue
            }
        }
        OwnerCommand::BackgroundTrustRevoke {
            session_generation,
            deadline,
            cancellation,
            reply,
        } => {
            let result = if cancellation.is_cancelled() || Instant::now() >= deadline {
                Err("背景授权撤销已取消或超时。".into())
            } else if session_generation != owner_generation {
                Err("活动会话代际已变化；未撤销 Receiver 授权。".into())
            } else {
                session.revoke_background_trust_until(deadline, &cancellation)
            };
            let receiver_user_stopped =
                session.terminal_cause() == Some(TerminalCause::ReceiverUserStop);
            if result.is_ok() || receiver_user_stopped {
                enabled.store(false, Ordering::Release);
                let _ = reply.send(result);
                BackgroundTrustCommandOutcome::Stop
            } else {
                let error = result.as_ref().err().cloned().unwrap_or_default();
                let connection_failed = session.state() == ReceiverState::Stopped;
                let _ = reply.send(result);
                if connection_failed {
                    BackgroundTrustCommandOutcome::ConnectionFailed(error)
                } else {
                    BackgroundTrustCommandOutcome::Continue
                }
            }
        }
        _ => BackgroundTrustCommandOutcome::Continue,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_session_owner(
    endpoint: SessionEndpoint,
    binding: BindingMaterial,
    parameters: AudioParameters,
    cancellation: CancellationToken,
    enabled: Arc<AtomicBool>,
    commands: mpsc::Receiver<OwnerCommand>,
    status: Arc<Mutex<SessionSnapshot>>,
    queue: Arc<Mutex<Option<Arc<PcmQueue>>>>,
    float_queue: Arc<Mutex<Option<Arc<FloatPcmQueue>>>>,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
    timing: SessionTiming,
    mut profile: HostAudioProfile,
    mut initial_float_queue: Option<Arc<FloatPcmQueue>>,
    diagnostics_capture: Option<StartupDiagnosticsCapture>,
) {
    let mut endpoint = endpoint;
    let mut parameters = parameters;
    let mut paused = false;
    let mut desired_paused = false;
    let mut generation = 0u64;
    let mut attempts = 0u64;
    let mut retry_index = 0;
    let mut last_error = None;
    let mut last_active_at = None;
    let mut recovery_started_at = None;
    'owner: loop {
        if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
            break;
        }
        {
            let mut snapshot = status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            snapshot.negotiated_format = None;
            snapshot.playback_diagnostics = None;
        }
        generation = generation.saturating_add(1);
        attempts = attempts.saturating_add(1);
        publish_session(
            &status,
            SessionLifecycleState::Reconnecting,
            generation,
            attempts,
            None,
            paused,
            last_error.clone(),
        );
        profile = match profile_for_mode(profile, parameters.mode) {
            Ok(profile) => profile,
            Err(error) => {
                last_error = Some(error);
                publish_session(
                    &status,
                    SessionLifecycleState::Stopped,
                    generation,
                    attempts,
                    None,
                    paused,
                    last_error.clone(),
                );
                break 'owner;
            }
        };
        let result = HostSession::authenticate_for_profile(
            endpoint,
            &binding,
            parameters,
            profile,
            &mut initial_float_queue,
            cancellation.clone(),
        );
        let mut session = match result {
            Ok(session) => session,
            Err(error) => {
                if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
                    break;
                }
                last_error = Some(error);
                // An owner may recover only after it has actually established a live
                // session. An explicit discovery/receive intent does not turn a failed
                // first connection into background retry policy.
                if last_active_at.is_none() {
                    break 'owner;
                }
                publish_session(
                    &status,
                    SessionLifecycleState::Reconnecting,
                    generation,
                    attempts,
                    None,
                    paused,
                    last_error.clone(),
                );
                match wait_for_reconnect(
                    lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
                    &commands,
                    &cancellation,
                    &enabled,
                    &mut endpoint,
                    &mut profile,
                    &mut parameters,
                    &mut desired_paused,
                ) {
                    RetryWait::Stop => break,
                    RetryWait::ReconnectNow => retry_index = 0,
                    RetryWait::DelayElapsed => {
                        retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                        if recovery_window_expired(
                            recovery_started_at,
                            timing.recovery_window,
                            Instant::now(),
                        ) {
                            break 'owner;
                        }
                    }
                }
                continue 'owner;
            }
        };
        if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
            break;
        }
        if let Err(error) = session.negotiate_capabilities() {
            last_error = Some(error);
            drop(session);
            publish_session(
                &status,
                SessionLifecycleState::Stopped,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            break 'owner;
        }
        let capabilities = session.capabilities().cloned();
        let terminal_cause_supported = capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_TERMINAL_CAUSE != 0);
        let startup_recorder = if capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_BUFFER_METRICS != 0)
        {
            diagnostics_capture.as_ref().map(|capture| {
                let (audio_profile, playback_mode) = match profile {
                    HostAudioProfile::Standard => (1, parameters.mode),
                    HostAudioProfile::AudioV2(format) => (format.profile, format.mode),
                };
                capture.recorder("wifi-udp", audio_profile, playback_mode)
            })
        } else {
            None
        };
        status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .capabilities = capabilities.clone();
        status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .negotiated_format = session.format_v2;
        if session.format_v2.is_some()
            && capabilities.as_ref().is_some_and(|info| {
                info.management_compatible
                    && !info.compatible
                    && info.capability_bits & CAP_AUDIO_V2 != 0
            })
        {
            last_error = Some(
                "Receiver rejected the requested authenticated audio format; this V2 descriptor is unsupported.".into(),
            );
        }
        publish_session(
            &status,
            SessionLifecycleState::Authenticated,
            generation,
            attempts,
            Some(session.session_id()),
            paused,
            last_error.clone(),
        );
        if !capabilities
            .as_ref()
            .is_some_and(|info| info.management_compatible)
        {
            last_error = Some("Receiver authenticated status is incompatible; update it through authorized ADB or manual installation.".into());
            drop(session);
            publish_session(
                &status,
                SessionLifecycleState::Stopped,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            break 'owner;
        }
        if capabilities.as_ref().is_some_and(|info| !info.compatible) {
            let mut stop_owner = false;
            let mut restart = false;
            let mut management_error = None;
            let mut next_heartbeat = Instant::now() + timing.heartbeat_interval;
            loop {
                if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
                    stop_owner = true;
                    break;
                }
                if Instant::now() >= next_heartbeat {
                    match session.heartbeat() {
                        Ok(ReceiverState::Stopped)
                            if finish_receiver_user_stop(&mut session, &status) =>
                        {
                            stop_owner = true;
                            break;
                        }
                        Ok(ReceiverState::Stopped) => {
                            management_error = Some(
                                "Receiver stopped without an authenticated terminal cause.".into(),
                            );
                            restart = true;
                            break;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            management_error = Some(error);
                            restart = true;
                            break;
                        }
                    }
                    next_heartbeat = Instant::now() + timing.heartbeat_interval;
                    continue;
                }
                match commands.recv_timeout(
                    next_heartbeat
                        .saturating_duration_since(Instant::now())
                        .min(SESSION_IO_POLL),
                ) {
                    Ok(OwnerCommand::Stop) => {
                        enabled.store(false, Ordering::Release);
                        let _ = session.stop();
                        stop_owner = true;
                        break;
                    }
                    Ok(OwnerCommand::Reconnect) => {
                        restart = true;
                        break;
                    }
                    Ok(command @ OwnerCommand::BackgroundTrustQuery { .. })
                    | Ok(command @ OwnerCommand::BackgroundTrustRevoke { .. }) => {
                        match execute_background_trust_command(
                            &mut session,
                            generation,
                            &enabled,
                            command,
                        ) {
                            BackgroundTrustCommandOutcome::Continue => {}
                            BackgroundTrustCommandOutcome::Stop => {
                                stop_owner = true;
                                break;
                            }
                            BackgroundTrustCommandOutcome::ConnectionFailed(error) => {
                                management_error = Some(error);
                                restart = true;
                                break;
                            }
                        }
                    }
                    #[cfg(test)]
                    Ok(OwnerCommand::TestSenderBarrier(reply)) => drop(reply),
                    Ok(OwnerCommand::UpdateEndpoint(next)) => {
                        endpoint = next;
                        restart = true;
                        break;
                    }
                    Ok(OwnerCommand::SetMode(mode)) => {
                        parameters.mode = mode;
                        // The phone applies a playback mode only in a fresh handshake, so this
                        // restarts the session. Start the recovery window here: without it a failed
                        // switch could report "reconnecting" forever.
                        if recovery_started_at.is_none() {
                            recovery_started_at = Some(Instant::now());
                            status
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .recovery_started_at = recovery_started_at;
                        }
                        restart = true;
                        break;
                    }
                    Ok(OwnerCommand::ReconfigureAudio {
                        profile: next_profile,
                        parameters: next_parameters,
                        paused: next_paused,
                    }) => {
                        profile = next_profile;
                        parameters = next_parameters;
                        desired_paused = next_paused;
                        if recovery_started_at.is_none() {
                            recovery_started_at = Some(Instant::now());
                            status
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .recovery_started_at = recovery_started_at;
                        }
                        restart = true;
                        break;
                    }
                    Ok(OwnerCommand::SetPaused(_)) => {}
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        enabled.store(false, Ordering::Release);
                        stop_owner = true;
                        break;
                    }
                }
            }
            if finish_receiver_user_stop(&mut session, &status) {
                last_error = None;
                stop_owner = true;
            }
            session.close_local();
            drop(session);
            if stop_owner || !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
                break 'owner;
            }
            last_error = management_error.clone();
            if management_error.is_some() && (last_active_at.is_none() || !terminal_cause_supported)
            {
                publish_session(
                    &status,
                    SessionLifecycleState::Stopped,
                    generation,
                    attempts,
                    None,
                    paused,
                    last_error.clone(),
                );
                break 'owner;
            }
            publish_session(
                &status,
                SessionLifecycleState::Reconnecting,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            if restart {
                if management_error.is_some() {
                    match wait_for_reconnect(
                        lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
                        &commands,
                        &cancellation,
                        &enabled,
                        &mut endpoint,
                        &mut profile,
                        &mut parameters,
                        &mut desired_paused,
                    ) {
                        RetryWait::Stop => break 'owner,
                        RetryWait::ReconnectNow => retry_index = 0,
                        RetryWait::DelayElapsed => {
                            retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1)
                        }
                    }
                } else {
                    retry_index = 0;
                }
            }
            continue 'owner;
        }
        if let Err(error) = session.authorize_background_start() {
            last_error = Some(error);
            if finish_receiver_user_stop(&mut session, &status) {
                break 'owner;
            }
            drop(session);
            publish_session(
                &status,
                SessionLifecycleState::Stopped,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            break 'owner;
        }
        if let Err(error) = session.bind_audio().and_then(|_| session.select_mode()) {
            last_error = Some(error);
            if finish_receiver_user_stop(&mut session, &status) {
                break 'owner;
            }
            drop(session);
            if last_active_at.is_none() {
                break 'owner;
            }
            publish_session(
                &status,
                SessionLifecycleState::Reconnecting,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            match wait_for_reconnect(
                lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
                &commands,
                &cancellation,
                &enabled,
                &mut endpoint,
                &mut profile,
                &mut parameters,
                &mut desired_paused,
            ) {
                RetryWait::Stop => break,
                RetryWait::ReconnectNow => retry_index = 0,
                RetryWait::DelayElapsed => {
                    retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                    if recovery_window_expired(
                        recovery_started_at,
                        timing.recovery_window,
                        Instant::now(),
                    ) {
                        break 'owner;
                    }
                }
            }
            continue 'owner;
        }
        match apply_pending_commands(
            &commands,
            &mut endpoint,
            &mut profile,
            &mut parameters,
            &mut desired_paused,
            &enabled,
        ) {
            PendingCommands::Stop => break,
            PendingCommands::EndpointChanged => {
                drop(session);
                retry_index = 0;
                continue 'owner;
            }
            PendingCommands::ModeChanged => {
                drop(session);
                retry_index = 0;
                continue 'owner;
            }
            PendingCommands::AudioConfigurationChanged => {
                drop(session);
                retry_index = 0;
                continue 'owner;
            }
            PendingCommands::Reconnect => {
                drop(session);
                retry_index = 0;
                continue 'owner;
            }
            PendingCommands::Applied => {}
        }
        if cancellation.is_cancelled() || !enabled.load(Ordering::Acquire) {
            break;
        }
        publish_session(
            &status,
            SessionLifecycleState::TransportConnected,
            generation,
            attempts,
            Some(session.session_id()),
            paused,
            None,
        );
        if let Err(error) = session.start().and_then(|_| {
            if desired_paused {
                session.pause()
            } else {
                Ok(())
            }
        }) {
            let terminal = harmony_start_failed(endpoint.harmony, &error);
            last_error = Some(error);
            if finish_receiver_user_stop(&mut session, &status) {
                break 'owner;
            }
            drop(session);
            if last_active_at.is_none() || terminal {
                break 'owner;
            }
            publish_session(
                &status,
                SessionLifecycleState::Reconnecting,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            match wait_for_reconnect(
                lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
                &commands,
                &cancellation,
                &enabled,
                &mut endpoint,
                &mut profile,
                &mut parameters,
                &mut desired_paused,
            ) {
                RetryWait::Stop => break,
                RetryWait::ReconnectNow => retry_index = 0,
                RetryWait::DelayElapsed => {
                    retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                    if recovery_window_expired(
                        recovery_started_at,
                        timing.recovery_window,
                        Instant::now(),
                    ) {
                        break 'owner;
                    }
                }
            }
            continue 'owner;
        }
        paused = desired_paused;
        last_active_at = Some(Instant::now());
        recovery_started_at = None;
        status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recovery_started_at = None;
        let session_id = session.session_id();
        let mut last_session_bytes = 0;
        match session.audio_queue() {
            SessionAudioQueue::Pcm16(session_queue) => {
                *queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(session_queue);
                *float_queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            }
            SessionAudioQueue::Float32(session_queue, _) => {
                *queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                *float_queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(session_queue);
            }
        }
        publish_session(
            &status,
            if paused {
                SessionLifecycleState::Paused
            } else {
                SessionLifecycleState::ReceiverRunning
            },
            generation,
            attempts,
            Some(session_id),
            paused,
            None,
        );
        retry_index = 0;
        let mut endpoint_changed = false;
        let mut stop_owner = false;
        let mut next_heartbeat = Instant::now() + timing.heartbeat_interval;
        let mut next_progress = next_heartbeat;
        loop {
            if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
                stop_owner = true;
                break;
            }
            if let Err(error) = session.check_sender() {
                last_error = Some(format!("P5-D UDP sender stopped: {error}"));
                break;
            }
            let sent = session.pcm_payload_bytes_sent();
            pcm_payload_bytes_sent
                .fetch_add(sent.saturating_sub(last_session_bytes), Ordering::Relaxed);
            last_session_bytes = sent;
            if Instant::now() >= next_heartbeat {
                match session.heartbeat() {
                    Ok(ReceiverState::Stopped)
                        if session.terminal_cause() == Some(TerminalCause::ReceiverUserStop) =>
                    {
                        let mut current = status
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        current.state = SessionLifecycleState::Stopped;
                        current.session_id = None;
                        current.terminal_cause = Some(TerminalCause::ReceiverUserStop);
                        stop_owner = true;
                        break;
                    }
                    Ok(ReceiverState::Stopped) => {
                        last_error = Some(
                            "P5-C receiver stopped without an authenticated terminal cause.".into(),
                        );
                        break;
                    }
                    Ok(state) => {
                        publish_session(
                            &status,
                            lifecycle_from_receiver(state),
                            generation,
                            attempts,
                            Some(session_id),
                            paused,
                            None,
                        );
                    }
                    Err(error) => {
                        last_error = Some(error);
                        break;
                    }
                }
                next_heartbeat = Instant::now() + timing.heartbeat_interval;
            }
            if Instant::now() >= next_progress {
                match session.playback_progress() {
                    Ok(Some(mut playback)) => {
                        let audio_queue = session.audio_queue();
                        let previous = status
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .playback_diagnostics
                            .as_ref()
                            .and_then(|current| current.underrun_count);
                        playback.sample_rate = Some(session.parameters.sample_rate);
                        playback.channels = Some(session.parameters.channels);
                        let (pc_dropped, pc_queued) = match audio_queue {
                            SessionAudioQueue::Pcm16(queue) => {
                                (Some(queue.dropped_frames()), Some(queue.queued_frames()))
                            }
                            SessionAudioQueue::Float32(queue, _) => {
                                (Some(queue.dropped_frames()), Some(queue.queued_frames()))
                            }
                        };
                        playback.pc_dropped_frames = pc_dropped;
                        playback.pc_queued_frames = pc_queued;
                        playback.underrun_increment = match (previous, playback.underrun_count) {
                            (Some(previous), Some(current)) if current >= previous => {
                                Some((current - previous) as u64)
                            }
                            _ => None,
                        };
                        let now = Instant::now();
                        let fast = !paused
                            && startup_recorder.as_ref().is_some_and(|recorder| {
                                if playback.output_epoch.is_some() {
                                    recorder.record(playback.clone(), pc_queued, pc_dropped, now);
                                    recorder.should_poll_fast(
                                        playback.receiver_generation,
                                        playback.output_epoch,
                                        now,
                                    )
                                } else {
                                    false
                                }
                            });
                        status
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .playback_diagnostics = Some(playback);
                        next_progress = now
                            + if fast {
                                STARTUP_DIAGNOSTICS_INTERVAL
                            } else {
                                timing.heartbeat_interval
                            };
                    }
                    Ok(None) => {
                        status
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .playback_diagnostics = None;
                        next_progress = Instant::now() + timing.heartbeat_interval;
                    }
                    Err(error) => {
                        last_error = Some(error);
                        break;
                    }
                }
            }
            let until_next = next_heartbeat
                .min(next_progress)
                .saturating_duration_since(Instant::now());
            match commands.recv_timeout(until_next.min(SESSION_IO_POLL)) {
                Ok(OwnerCommand::SetPaused(value)) => {
                    desired_paused = value;
                    if value == paused {
                        continue;
                    }
                    let fast_resume = !value
                        && startup_recorder.as_ref().is_some_and(|recorder| {
                            let current = status
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            current
                                .playback_diagnostics
                                .as_ref()
                                .is_some_and(|progress| {
                                    recorder.should_poll_fast(
                                        progress.receiver_generation,
                                        progress.output_epoch,
                                        Instant::now(),
                                    )
                                })
                        });
                    let result = if value {
                        session.pause()
                    } else {
                        session.resume()
                    };
                    match result {
                        Ok(()) => {
                            paused = value;
                            next_progress = if value {
                                Instant::now() + timing.heartbeat_interval
                            } else if fast_resume {
                                Instant::now()
                            } else {
                                next_progress
                            };
                            let state = if paused {
                                SessionLifecycleState::Paused
                            } else {
                                SessionLifecycleState::ReceiverRunning
                            };
                            publish_session(
                                &status,
                                state,
                                generation,
                                attempts,
                                Some(session_id),
                                paused,
                                None,
                            );
                        }
                        Err(error) => {
                            last_error = Some(error);
                            break;
                        }
                    }
                }
                #[cfg(test)]
                Ok(OwnerCommand::TestSenderBarrier(reply)) => {
                    let sender_joined = session
                        .float_sender
                        .as_ref()
                        .is_none_or(FloatUdpSender::worker_joined);
                    let _ = reply.send(sender_joined);
                }
                Ok(OwnerCommand::UpdateEndpoint(next_endpoint)) => {
                    endpoint_changed = endpoint != next_endpoint;
                    endpoint = next_endpoint;
                    if !endpoint_changed {
                        continue;
                    }
                    if let Err(error) = session.stop() {
                        last_error = Some(error);
                    }
                    break;
                }
                Ok(OwnerCommand::SetMode(mode)) => {
                    if mode == parameters.mode {
                        continue;
                    }
                    parameters.mode = mode;
                    if let Err(error) = session.stop() {
                        last_error = Some(error);
                    }
                    endpoint_changed = true;
                    break;
                }
                Ok(OwnerCommand::ReconfigureAudio {
                    profile: next_profile,
                    parameters: next_parameters,
                    paused: next_paused,
                }) => {
                    if next_profile == profile && next_parameters == parameters {
                        desired_paused = next_paused;
                        if next_paused != paused {
                            let changed = if next_paused {
                                session.pause()
                            } else {
                                session.resume()
                            };
                            if let Err(error) = changed {
                                last_error = Some(error);
                                break;
                            }
                            paused = next_paused;
                        }
                        continue;
                    }
                    profile = next_profile;
                    parameters = next_parameters;
                    desired_paused = next_paused;
                    if let Err(error) = session.stop() {
                        last_error = Some(error);
                    }
                    endpoint_changed = true;
                    break;
                }
                Ok(OwnerCommand::Reconnect) => {
                    if finish_receiver_user_stop(&mut session, &status) {
                        stop_owner = true;
                    }
                    session.close_local();
                    endpoint_changed = true;
                    break;
                }
                Ok(command @ OwnerCommand::BackgroundTrustQuery { .. })
                | Ok(command @ OwnerCommand::BackgroundTrustRevoke { .. }) => {
                    match execute_background_trust_command(
                        &mut session,
                        generation,
                        &enabled,
                        command,
                    ) {
                        BackgroundTrustCommandOutcome::Continue => {}
                        BackgroundTrustCommandOutcome::Stop => {
                            stop_owner = true;
                            break;
                        }
                        BackgroundTrustCommandOutcome::ConnectionFailed(error) => {
                            last_error = Some(error);
                            break;
                        }
                    }
                }
                Ok(OwnerCommand::Stop) => {
                    enabled.store(false, Ordering::Release);
                    if let Err(error) = session.stop() {
                        last_error = Some(error);
                    }
                    status
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .terminal_cause = Some(TerminalCause::HostStop);
                    stop_owner = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    enabled.store(false, Ordering::Release);
                    stop_owner = true;
                    break;
                }
            }
        }
        if finish_receiver_user_stop(&mut session, &status) {
            last_error = None;
            stop_owner = true;
        }
        let receiver_user_stopped =
            session.terminal_cause() == Some(TerminalCause::ReceiverUserStop);
        session.close_local();
        let sent = session.pcm_payload_bytes_sent();
        pcm_payload_bytes_sent
            .fetch_add(sent.saturating_sub(last_session_bytes), Ordering::Relaxed);
        *queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *float_queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .negotiated_format = None;
        drop(session);
        if stop_owner || !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
            break 'owner;
        }
        if last_active_at.is_some() && recovery_started_at.is_none() {
            recovery_started_at = Some(Instant::now());
            status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .recovery_started_at = recovery_started_at;
        }
        if receiver_user_stopped || stop_owner {
            break 'owner;
        }
        if !endpoint_changed && !terminal_cause_supported {
            last_error = Some(
                "Receiver disconnected without terminal-cause capability; update it or reconnect manually.".into(),
            );
            publish_session(
                &status,
                SessionLifecycleState::Stopped,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            break 'owner;
        }
        publish_session(
            &status,
            SessionLifecycleState::Reconnecting,
            generation,
            attempts,
            None,
            paused,
            last_error.clone(),
        );
        if endpoint_changed {
            retry_index = 0;
            continue 'owner;
        }
        match wait_for_reconnect(
            lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
            &commands,
            &cancellation,
            &enabled,
            &mut endpoint,
            &mut profile,
            &mut parameters,
            &mut desired_paused,
        ) {
            RetryWait::Stop => break,
            RetryWait::ReconnectNow => retry_index = 0,
            RetryWait::DelayElapsed => {
                retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                if recovery_window_expired(
                    recovery_started_at,
                    timing.recovery_window,
                    Instant::now(),
                ) {
                    break 'owner;
                }
            }
        }
    }
    *queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    *float_queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    {
        let mut current = status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.terminal_cause.is_none() && last_error.is_some() {
            current.terminal_cause = Some(TerminalCause::TransportFailure);
        }
    }
    publish_session(
        &status,
        SessionLifecycleState::Stopped,
        generation,
        attempts,
        None,
        paused,
        last_error,
    );
}

/// A tablet fails START only when its output cannot open the negotiated format, and its run then ends:
/// reconnecting with the same format cannot succeed and would replace the reason with a connect error.
pub(super) fn harmony_start_failed(harmony: bool, error: &str) -> bool {
    harmony && error == START_NOT_CONFIRMED
}

pub(super) fn lifecycle_from_receiver(state: ReceiverState) -> SessionLifecycleState {
    match state {
        ReceiverState::Authenticated => SessionLifecycleState::Authenticated,
        ReceiverState::TransportConnected => SessionLifecycleState::TransportConnected,
        ReceiverState::ReceiverRunning => SessionLifecycleState::ReceiverRunning,
        ReceiverState::Streaming => SessionLifecycleState::Streaming,
        ReceiverState::Paused => SessionLifecycleState::Paused,
        ReceiverState::Stopped => SessionLifecycleState::Stopped,
    }
}

pub(super) fn finish_receiver_user_stop(
    session: &mut HostSession,
    status: &Mutex<SessionSnapshot>,
) -> bool {
    if session.terminal_cause() != Some(TerminalCause::ReceiverUserStop) {
        return false;
    }
    let mut current = status
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    current.state = SessionLifecycleState::Stopped;
    current.session_id = None;
    current.terminal_cause = Some(TerminalCause::ReceiverUserStop);
    current.last_error = None;
    true
}

pub(super) fn publish_session(
    status: &Mutex<SessionSnapshot>,
    state: SessionLifecycleState,
    generation: u64,
    attempts: u64,
    session_id: Option<u64>,
    paused: bool,
    last_error: Option<String>,
) {
    let mut current = status
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if state == SessionLifecycleState::Reconnecting && session_id.is_none() {
        current.capabilities = None;
        current.playback_diagnostics = None;
        current.negotiated_format = None;
    }
    current.state = state;
    current.generation = generation;
    current.attempts = attempts;
    current.session_id = session_id;
    current.paused = paused;
    current.last_error = last_error;
}

pub(super) enum RetryWait {
    Stop,
    ReconnectNow,
    DelayElapsed,
}

pub(super) enum PendingCommands {
    Stop,
    EndpointChanged,
    ModeChanged,
    AudioConfigurationChanged,
    Reconnect,
    Applied,
}

pub(super) fn apply_pending_commands(
    commands: &mpsc::Receiver<OwnerCommand>,
    endpoint: &mut SessionEndpoint,
    profile: &mut HostAudioProfile,
    parameters: &mut AudioParameters,
    paused: &mut bool,
    enabled: &AtomicBool,
) -> PendingCommands {
    let mut endpoint_changed = false;
    let mut mode_changed = false;
    let mut audio_configuration_changed = false;
    let mut reconnect = false;
    loop {
        match commands.try_recv() {
            Ok(OwnerCommand::SetPaused(value)) => *paused = value,
            #[cfg(test)]
            Ok(OwnerCommand::TestSenderBarrier(reply)) => drop(reply),
            Ok(command @ OwnerCommand::BackgroundTrustQuery { .. })
            | Ok(command @ OwnerCommand::BackgroundTrustRevoke { .. }) => {
                reject_background_trust_command(
                    command,
                    "活动会话代际已变化；请刷新授权状态后重试。",
                );
            }
            Ok(OwnerCommand::Stop) => {
                enabled.store(false, Ordering::Release);
                return PendingCommands::Stop;
            }
            Ok(OwnerCommand::UpdateEndpoint(next_endpoint)) => {
                endpoint_changed |= *endpoint != next_endpoint;
                *endpoint = next_endpoint;
            }
            Ok(OwnerCommand::SetMode(mode)) => {
                mode_changed |= parameters.mode != mode;
                parameters.mode = mode;
            }
            Ok(OwnerCommand::ReconfigureAudio {
                profile: next_profile,
                parameters: next_parameters,
                paused: next_paused,
            }) => {
                audio_configuration_changed |=
                    *profile != next_profile || *parameters != next_parameters;
                *profile = next_profile;
                *parameters = next_parameters;
                *paused = next_paused;
            }
            Ok(OwnerCommand::Reconnect) => reconnect = true,
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                enabled.store(false, Ordering::Release);
                return PendingCommands::Stop;
            }
        }
    }
    if reconnect {
        PendingCommands::Reconnect
    } else if endpoint_changed {
        PendingCommands::EndpointChanged
    } else if mode_changed {
        PendingCommands::ModeChanged
    } else if audio_configuration_changed {
        PendingCommands::AudioConfigurationChanged
    } else {
        PendingCommands::Applied
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn wait_for_reconnect(
    delay: Duration,
    commands: &mpsc::Receiver<OwnerCommand>,
    cancellation: &CancellationToken,
    enabled: &AtomicBool,
    endpoint: &mut SessionEndpoint,
    profile: &mut HostAudioProfile,
    parameters: &mut AudioParameters,
    paused: &mut bool,
) -> RetryWait {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline {
        if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
            return RetryWait::Stop;
        }
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .min(SESSION_IO_POLL);
        match commands.recv_timeout(remaining) {
            Ok(OwnerCommand::SetPaused(value)) => *paused = value,
            #[cfg(test)]
            Ok(OwnerCommand::TestSenderBarrier(reply)) => drop(reply),
            Ok(command @ OwnerCommand::BackgroundTrustQuery { .. })
            | Ok(command @ OwnerCommand::BackgroundTrustRevoke { .. }) => {
                reject_background_trust_command(
                    command,
                    "Receiver 正在重连；请在认证会话恢复后重试。",
                );
            }
            Ok(OwnerCommand::Stop) => {
                enabled.store(false, Ordering::Release);
                return RetryWait::Stop;
            }
            Ok(OwnerCommand::UpdateEndpoint(next_endpoint)) => {
                if *endpoint != next_endpoint {
                    *endpoint = next_endpoint;
                    return RetryWait::ReconnectNow;
                }
            }
            Ok(OwnerCommand::SetMode(mode)) => {
                if parameters.mode != mode {
                    parameters.mode = mode;
                    return RetryWait::ReconnectNow;
                }
            }
            Ok(OwnerCommand::ReconfigureAudio {
                profile: next_profile,
                parameters: next_parameters,
                paused: next_paused,
            }) => {
                let changed = *profile != next_profile || *parameters != next_parameters;
                *profile = next_profile;
                *parameters = next_parameters;
                *paused = next_paused;
                if changed {
                    return RetryWait::ReconnectNow;
                }
            }
            Ok(OwnerCommand::Reconnect) => return RetryWait::ReconnectNow,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                enabled.store(false, Ordering::Release);
                return RetryWait::Stop;
            }
        }
    }
    RetryWait::DelayElapsed
}

pub(super) fn lifecycle_retry_delay(delays: &[Duration; 4], attempt: usize) -> Duration {
    delays[attempt.min(delays.len() - 1)]
}

pub(super) fn recovery_window_expired(
    active_since: Option<Instant>,
    window: Duration,
    now: Instant,
) -> bool {
    active_since.is_some_and(|active_since| now.saturating_duration_since(active_since) >= window)
}

pub(super) fn profile_for_mode(
    profile: HostAudioProfile,
    mode: u8,
) -> Result<HostAudioProfile, String> {
    match profile {
        HostAudioProfile::Standard => Ok(HostAudioProfile::Standard),
        HostAudioProfile::AudioV2(format) => AudioFormatV2::for_profile(format.profile, mode)
            .map(HostAudioProfile::AudioV2)
            .map_err(|error| error.to_string()),
    }
}

pub(super) fn random_request_id() -> Result<[u8; 16], String> {
    loop {
        let mut request_id = [0; 16];
        getrandom::fill(&mut request_id)
            .map_err(|_| "Unable to generate a background-start request ID.".to_string())?;
        if request_id.iter().any(|byte| *byte != 0) {
            return Ok(request_id);
        }
    }
}
