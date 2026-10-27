use super::*;

#[test]
fn audio_settings_batch_validates_without_silent_legacy_downgrade() {
    let current = AppSettings::default();
    let next = prepare_audio_settings(&current, "hifi", "LowLatency").unwrap();
    assert_eq!(next.audio_quality, "hifi");
    assert_eq!(next.playback_mode, "LowLatency");

    let mut legacy_adb = current;
    legacy_adb.preferred_transport = "Usb".into();
    legacy_adb.wired_mode = "adb".into();
    legacy_adb.audio_backend = "legacy".into();
    assert!(prepare_audio_settings(&legacy_adb, "hifi", "Balanced").is_err());
    assert!(prepare_audio_settings(&legacy_adb, "standard", "unknown").is_err());
}

#[test]
fn audio_preferences_commit_preserves_other_settings_and_is_write_atomic() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        ".tmp-audio-settings-commit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let old = AppSettings {
        preferred_transport: "Wifi".into(),
        wireless_protocol: "udp".into(),
        last_device_id: Some("remembered-device".into()),
        appearance_theme: "Dark".into(),
        ..AppSettings::default()
    };
    let candidate = prepare_audio_settings(&old, "hifi", "LowLatency").unwrap();
    let mut state = AppState::new();
    state.data_dir = root.clone();
    state.view.lock().unwrap().settings = old.clone();
    state.save_audio_preferences(&candidate).unwrap();
    let current = state.snapshot().settings;
    assert_eq!(current.audio_quality, "hifi");
    assert_eq!(current.playback_mode, "LowLatency");
    assert_eq!(current.preferred_transport, "Wifi");
    assert_eq!(current.wireless_protocol, "udp");
    assert_eq!(current.last_device_id.as_deref(), Some("remembered-device"));
    assert_eq!(current.appearance_theme, "Dark");
    let persisted: AppSettings =
        serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(persisted).unwrap(),
        serde_json::to_value(&current).unwrap()
    );
    fs::remove_dir_all(&root).unwrap();

    let failed_root = root.with_extension("failed");
    fs::create_dir_all(failed_root.join("settings.json")).unwrap();
    let disk_sentinel = serde_json::to_vec(&old).unwrap();
    fs::write(failed_root.join("settings.json/old.json"), &disk_sentinel).unwrap();
    let mut failing_state = AppState::new();
    failing_state.data_dir = failed_root.clone();
    failing_state.view.lock().unwrap().settings = old.clone();
    assert!(failing_state.save_audio_preferences(&candidate).is_err());
    assert_eq!(
        serde_json::to_value(failing_state.snapshot().settings).unwrap(),
        serde_json::to_value(&old).unwrap()
    );
    assert_eq!(
        fs::read(failed_root.join("settings.json/old.json")).unwrap(),
        disk_sentinel
    );
    fs::remove_dir_all(failed_root).unwrap();
}

#[test]
fn authenticated_quality_profiles_and_snapshot_format_precedence() {
    assert_eq!(audio_quality_profile("standard"), Some(1));
    assert_eq!(audio_quality_profile("hifi"), Some(4));
    assert_eq!(audio_quality_profile("hires"), Some(5));

    let diagnostics = PlaybackDiagnostics {
        sample_rate: Some(48_000),
        channels: Some(2),
        ..Default::default()
    };
    let float = NegotiatedAudioFormat {
        codec_version: 2,
        sample_format: "float32le".into(),
        sample_rate_hz: 48_000,
        channels: 2,
        valid_bits_per_sample: 32,
        bytes_per_sample: 4,
        frame_bytes: 8,
    };
    let standard_v2 = NegotiatedAudioFormat {
        codec_version: 2,
        sample_format: "pcm16le".into(),
        sample_rate_hz: 48_000,
        channels: 2,
        valid_bits_per_sample: 16,
        bytes_per_sample: 2,
        frame_bytes: 4,
    };
    let selected =
        snapshot_negotiated_audio_format(true, "phone", Some(&diagnostics), Some(&standard_v2))
            .unwrap();
    assert_eq!(selected.codec_version, 2);
    assert_eq!(selected.sample_format, "pcm16le");
    let selected =
        snapshot_negotiated_audio_format(true, "phone", Some(&diagnostics), Some(&float)).unwrap();
    assert_eq!(selected.sample_format, "float32le");
    assert_eq!(selected.codec_version, 2);
    let pcm24 = NegotiatedAudioFormat {
        codec_version: 2,
        sample_format: "pcm24le".into(),
        valid_bits_per_sample: 24,
        bytes_per_sample: 3,
        frame_bytes: 6,
        ..float.clone()
    };
    let selected =
        snapshot_negotiated_audio_format(true, "phone", Some(&diagnostics), Some(&pcm24)).unwrap();
    assert_eq!(selected.sample_format, "pcm24le");
    assert_eq!(selected.valid_bits_per_sample, 24);
    let fallback = snapshot_negotiated_audio_format(true, "phone", Some(&diagnostics), None)
        .expect("legacy 48 kHz stereo should infer PCM16");
    assert_eq!(fallback.sample_format, "pcm16le");
    assert!(
        snapshot_negotiated_audio_format(true, "computer", Some(&diagnostics), Some(&float))
            .is_none()
    );

    let state = AppState::new();
    {
        let mut view = state.view.lock().unwrap();
        view.session_active = true;
        view.output = "phone".into();
        view.playback_diagnostics = Some(diagnostics);
        view.negotiated_audio_format = Some(standard_v2);
    }
    let actual = state.snapshot().negotiated_audio_format.unwrap();
    assert_eq!(actual.codec_version, 2);
    assert_eq!(actual.sample_format, "pcm16le");
}

#[test]
fn diagnostics_sample_requires_matching_session_generation() {
    assert!(diagnostics_generation_is_consistent(Some(4), Some(4)));
    assert!(!diagnostics_generation_is_consistent(Some(4), Some(5)));
    assert!(!diagnostics_generation_is_consistent(None, Some(4)));
}

#[test]
fn adb_cleanup_keeps_receiver_user_stop_across_reverse_retry() {
    let mut cleanup = PendingAdbCleanup {
        serial: "192.0.2.10:5555".into(),
        transport: "wifi".into(),
        socket_name: "anpod_adb_test".into(),
        installed_receiver: true,
        receiver_owner_id: Some("owner".into()),
        receiver_generation: Some(7),
        receiver_stopped: false,
        receiver_user_stopped: false,
        reverse_removed: false,
    };

    assert!(record_adb_cleanup_attempt(
        &mut cleanup,
        Ok(()),
        true,
        Err("temporary reverse removal failure".into()),
    )
    .is_err());
    assert!(cleanup.receiver_stopped);
    assert!(cleanup.receiver_user_stopped);
    assert!(!cleanup.reverse_removed);

    assert_eq!(
        record_adb_cleanup_attempt(&mut cleanup, Ok(()), false, Ok(())),
        Ok(AdbCleanupOutcome::ReceiverUserStopped)
    );
    assert!(cleanup.receiver_user_stopped);
    assert!(cleanup.reverse_removed);
}

#[test]
fn adb_reverse_cleanup_treats_only_missing_listener_as_already_removed() {
    assert_eq!(
        adb_reverse_remove_result(
            AdbResult {
                success: false,
                stderr: "adb: error: listener 'localabstract:anpod_adb_test' not found\n".into(),
                ..Default::default()
            },
            "anpod_adb_test"
        ),
        Ok(())
    );
    assert_eq!(
        adb_reverse_remove_result(
            AdbResult {
                success: true,
                ..Default::default()
            },
            "anpod_adb_test"
        ),
        Ok(())
    );
    assert!(adb_reverse_remove_result(
        AdbResult {
            success: false,
            stderr: "adb: error: device offline".into(),
            ..Default::default()
        },
        "anpod_adb_test"
    )
    .is_err());
    assert!(adb_reverse_remove_result(
        AdbResult {
            success: false,
            stderr: "adb: error: listener 'localabstract:other_socket' not found".into(),
            ..Default::default()
        },
        "anpod_adb_test"
    )
    .is_err());
}

#[derive(Default)]
struct FakeAdbRecoveryOps {
    devices: std::sync::Mutex<VecDeque<Vec<Device>>>,
    cleanup: std::sync::Mutex<Option<Result<AdbCleanupOutcome, String>>>,
    cancel_during_cleanup: std::sync::atomic::AtomicBool,
    bootstrap_succeeds: bool,
    calls: std::sync::Mutex<Vec<String>>,
}

impl AdbRecoveryAttemptOps for FakeAdbRecoveryOps {
    fn find_device<'a>(
        &'a self,
        _serial: &'a str,
        _transport: &'a str,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Vec<Device>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("refresh".into());
            if cancel.is_cancelled() {
                Vec::new()
            } else {
                self.devices.lock().unwrap().pop_front().unwrap_or_default()
            }
        })
    }

    fn cleanup_pending<'a>(
        &'a self,
        _serial: &'a str,
        _transport: &'a str,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<AdbCleanupOutcome, String>> + Send + 'a>> {
        Box::pin(async move {
            if self.cancel_during_cleanup.load(Ordering::Acquire) {
                cancel.cancel();
            }
            let outcome = self
                .cleanup
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(AdbCleanupOutcome::Complete));
            let mut calls = self.calls.lock().unwrap();
            if matches!(&outcome, Err(error) if error == "owner changed") {
                calls.push("owner-mismatch".into());
            } else {
                calls.extend(["guarded-stop".into(), "remove-old-reverse".into()]);
            }
            outcome
        })
    }

    fn bootstrap<'a>(
        &'a self,
        _device: Device,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("bootstrap".into());
            self.bootstrap_succeeds
        })
    }
}

fn recovery_test_device(serial: &str, transport: &str, connectable: bool) -> Device {
    Device {
        serial: serial.into(),
        physical_id: None,
        transport: transport.into(),
        state: "device".into(),
        manufacturer: String::new(),
        model: String::new(),
        android_version: String::new(),
        api_level: String::new(),
        display_name: serial.into(),
        status_text: String::new(),
        connectable,
    }
}

#[tokio::test]
async fn adb_recovery_attempt_waits_for_same_transport_then_cleans_before_bootstrap() {
    let ops = FakeAdbRecoveryOps {
        devices: std::sync::Mutex::new(VecDeque::from([
            vec![],
            vec![recovery_test_device("192.0.2.10:5555", "wifi", true)],
        ])),
        bootstrap_succeeds: true,
        ..Default::default()
    };
    let token = CancellationToken::new();
    assert!(matches!(
        run_adb_recovery_attempt(&ops, "192.0.2.10:5555", "wifi", &token).await,
        AdbRecoveryAttempt::Offline
    ));
    assert_eq!(*ops.calls.lock().unwrap(), ["refresh"]);

    assert!(matches!(
        run_adb_recovery_attempt(&ops, "192.0.2.10:5555", "wifi", &token).await,
        AdbRecoveryAttempt::Connected
    ));
    assert_eq!(
        *ops.calls.lock().unwrap(),
        [
            "refresh",
            "refresh",
            "guarded-stop",
            "remove-old-reverse",
            "bootstrap"
        ]
    );
}

#[tokio::test]
async fn adb_recovery_attempt_never_bootstraps_after_cancel_user_stop_or_owner_mismatch() {
    let cancel_during_cleanup = FakeAdbRecoveryOps {
        devices: std::sync::Mutex::new(VecDeque::from([vec![recovery_test_device(
            "SERIAL", "usb", true,
        )]])),
        cancel_during_cleanup: std::sync::atomic::AtomicBool::new(true),
        ..Default::default()
    };
    let token = CancellationToken::new();
    assert!(matches!(
        run_adb_recovery_attempt(&cancel_during_cleanup, "SERIAL", "usb", &token).await,
        AdbRecoveryAttempt::Cancelled
    ));
    assert!(!cancel_during_cleanup
        .calls
        .lock()
        .unwrap()
        .contains(&"bootstrap".into()));

    let user_stopped = FakeAdbRecoveryOps {
        devices: std::sync::Mutex::new(VecDeque::from([vec![recovery_test_device(
            "SERIAL", "usb", true,
        )]])),
        cleanup: std::sync::Mutex::new(Some(Ok(AdbCleanupOutcome::ReceiverUserStopped))),
        ..Default::default()
    };
    assert!(matches!(
        run_adb_recovery_attempt(&user_stopped, "SERIAL", "usb", &CancellationToken::new()).await,
        AdbRecoveryAttempt::ReceiverUserStopped
    ));
    assert!(!user_stopped
        .calls
        .lock()
        .unwrap()
        .contains(&"bootstrap".into()));

    let wrong_owner = FakeAdbRecoveryOps {
        devices: std::sync::Mutex::new(VecDeque::from([vec![recovery_test_device(
            "SERIAL", "usb", true,
        )]])),
        cleanup: std::sync::Mutex::new(Some(Err("owner changed".into()))),
        ..Default::default()
    };
    assert!(matches!(
        run_adb_recovery_attempt(&wrong_owner, "SERIAL", "usb", &CancellationToken::new()).await,
        AdbRecoveryAttempt::CleanupFailed(error) if error == "owner changed"
    ));
    let calls = wrong_owner.calls.lock().unwrap();
    assert_eq!(*calls, ["refresh", "owner-mismatch"]);
    assert!(!calls.contains(&"bootstrap".into()));
}

#[tokio::test]
async fn adb_recovery_attempt_rejects_stale_or_other_transport_devices() {
    let ops = FakeAdbRecoveryOps {
        devices: std::sync::Mutex::new(VecDeque::from([vec![
            recovery_test_device("192.0.2.10:5555", "usb", true),
            recovery_test_device("192.0.2.11:5555", "wifi", true),
            recovery_test_device("192.0.2.10:5555", "wifi", false),
        ]])),
        ..Default::default()
    };
    assert!(matches!(
        run_adb_recovery_attempt(&ops, "192.0.2.10:5555", "wifi", &CancellationToken::new()).await,
        AdbRecoveryAttempt::Offline
    ));
    assert_eq!(*ops.calls.lock().unwrap(), ["refresh"]);
}

#[test]
fn diagnostics_timeline_prunes_samples_older_than_one_hour() {
    let mut timeline = DiagnosticsTimelineBuffer::new();
    let start = timeline.started_at;
    let sample = |elapsed_ms| DiagnosticsTimelineSample {
        sampled_at_utc: "2026-10-25T00:00:00Z".into(),
        elapsed_ms,
        session_generation: None,
        state: "ready".into(),
        transport: "usb".into(),
        session_transport: None,
        session_wireless_protocol: None,
        output: "computer".into(),
        playback_mode: "Balanced".into(),
        audio_quality: "standard".into(),
        negotiated_audio_format: None,
        windows_mix_format: None,
        android_output_format: None,
        pc_queued_frames: None,
        pc_dropped_frames: None,
        drift_target_frames: None,
        drift_observed_frames: None,
        drift_error_frames: None,
        drift_correction_ppm: None,
        drift_reset_reason: None,
        playback: None,
    };
    timeline.push(sample(0), start);
    timeline.push(sample(3_601_000), start + Duration::from_secs(3_601));
    let exported = timeline.export();
    assert!(exported.truncated);
    assert_eq!(exported.samples_omitted, 1);
    assert_eq!(exported.samples.len(), 1);
    assert_eq!(exported.samples[0].elapsed_ms, 3_601_000);
}

#[tokio::test]
async fn usb_direct_configuration_suppression_is_owned_by_its_configuration_id() {
    let state = AppState::new();
    let configuration = state.begin_usb_direct_configuration(false).await.unwrap();
    let configuration_id = configuration.configuration_id;
    assert!(state.usb_direct_configuration_active());
    assert!(state
        .begin_usb_direct_configuration(false)
        .await
        .err()
        .is_some());

    state.end_usb_direct_configuration(configuration_id.wrapping_add(1));
    assert!(state.usb_direct_configuration_active());
    state.end_usb_direct_configuration(configuration_id);
    state.end_usb_direct_configuration(configuration_id);
    assert!(!state.usb_direct_configuration_active());
}

#[test]
fn playback_mode_capability_includes_supported_sessions_and_quality_lock_remains_separate() {
    let state = AppState::new();
    {
        let mut view = state.view.lock().unwrap();
        view.session_active = true;
        view.session_transport = Some("usb".into());
    }
    let standard_usb = state.snapshot();
    assert!(standard_usb.can_change_playback_mode);
    assert!(!standard_usb.can_change_modes);
    state.view.lock().unwrap().session_transport = Some("aoa".into());
    assert!(!state.snapshot().can_change_playback_mode);
    state.view.lock().unwrap().session_capabilities = Some(ReceiverCapabilities {
        control_major: 1,
        audio_major: 2,
        management_compatible: true,
        audio_compatible: true,
        compatible: true,
        capability_bits: 0x200,
        version_code: 36,
    });
    assert!(state.snapshot().can_change_playback_mode);
}

#[tokio::test]
async fn adb_server_release_requires_confirmation_and_uses_kill_server() {
    let called = std::sync::atomic::AtomicBool::new(false);
    let declined = with_confirmed_adb_release(false, || async {
        called.store(true, Ordering::Release);
        Ok(())
    })
    .await;
    assert!(declined.unwrap_err().contains("确认停止本机 ADB server"));
    assert!(!called.load(Ordering::Acquire));

    let args = std::sync::Mutex::new(Vec::new());
    with_confirmed_adb_release(true, || async {
        *args.lock().unwrap() = ADB_SERVER_KILL_ARGS.to_vec();
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(*args.lock().unwrap(), ["kill-server"]);
}

fn sample_snapshot() -> AppSnapshot {
    AppSnapshot {
        state: "connected".into(),
        message: "正在从手机播放".into(),
        device_name: Some("Test Phone".into()),
        devices: Vec::new(),
        selected_serial: Some("TESTSERIAL".into()),
        session_serial: Some("TESTSERIAL".into()),
        session_receiver_binding: Some("phoneA-abcdef".into()),
        session_generation: Some(7),
        session_transport: Some("lan".into()),
        session_connection_transport: Some("wireless".into()),
        session_wireless_protocol: Some("udp".into()),
        session_capabilities: None,
        playback_diagnostics: Some(PlaybackDiagnostics::default()),
        negotiated_audio_format: None,
        windows_mix_format: None,
        android_output_format: None,
        network_recovery_seconds_remaining: None,
        wireless_connection_offers: HostDiscoverySnapshot::default(),
        session_android_version: Some("15".into()),
        connection_transport: "wireless".into(),
        transport: "wifi".into(),
        output: "phone".into(),
        settings: AppSettings::default(),
        busy: false,
        can_disconnect: true,
        can_change_modes: false,
        can_change_playback_mode: true,
        status_detail: "192.168.50.2 已连接".into(),
        audio_bitrate_bps: Some(1_536_000),
        presence: Vec::new(),
        version: "1.4.0".into(),
    }
}

#[test]
fn audio_quality_defaults_and_rejects_unavailable_profiles_without_changing_playback_mode() {
    let old_settings: AppSettings = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(old_settings.audio_quality, "standard");
    assert_eq!(old_settings.wired_mode, "aoa");

    assert!(validate_audio_quality("hifi").is_ok());
    assert!(validate_audio_quality("hires").is_ok());
    for quality in ["unsupported"] {
        assert!(validate_audio_quality(quality).is_err(), "{quality}");
    }
    assert!(validate_audio_quality("standard").is_ok());

    let settings = normalize_settings(AppSettings {
        audio_quality: "standard".into(),
        playback_mode: "LowLatency".into(),
        ..AppSettings::default()
    });
    assert_eq!(settings.audio_quality, "standard");
    assert_eq!(settings.playback_mode, "LowLatency");
}

#[test]
fn negotiated_audio_format_requires_active_phone_playback_and_confirmed_48khz_stereo() {
    let diagnostics = PlaybackDiagnostics {
        sample_rate: Some(48_000),
        channels: Some(2),
        ..PlaybackDiagnostics::default()
    };
    assert!(negotiated_audio_format(false, "phone", Some(&diagnostics)).is_none());
    assert!(negotiated_audio_format(true, "computer", Some(&diagnostics)).is_none());
    assert!(negotiated_audio_format(true, "phone", None).is_none());
    assert!(negotiated_audio_format(
        true,
        "phone",
        Some(&PlaybackDiagnostics {
            channels: Some(1),
            ..diagnostics.clone()
        })
    )
    .is_none());
    assert!(negotiated_audio_format(
        true,
        "phone",
        Some(&PlaybackDiagnostics {
            sample_rate: Some(44_100),
            ..diagnostics.clone()
        })
    )
    .is_none());
    assert_eq!(
        negotiated_audio_format(true, "phone", Some(&diagnostics)).map(|format| (
            format.sample_rate_hz,
            format.channels,
            format.sample_format
        )),
        Some((48_000, 2, "pcm16le".into()))
    );
}

#[test]
fn diagnostics_export_keeps_metrics_but_drops_identifiers() {
    let mut snapshot = sample_snapshot();
    snapshot.settings.last_usb_direct_target_id = Some("usb-target-private".into());
    snapshot.settings.last_usb_direct_binding_reference = Some("binding-private".into());
    snapshot.playback_diagnostics = Some(PlaybackDiagnostics {
        loss_10s_frames: Some(10),
        eligible_10s_frames: Some(100),
        loss_60s_frames: Some(20),
        eligible_60s_frames: Some(200),
        cumulative_loss_frames: Some(30),
        cumulative_eligible_frames: Some(300),
        ..PlaybackDiagnostics::default()
    });
    let json = serde_json::to_string(&diagnostics_export(
        &snapshot,
        DiagnosticsTimelineBuffer::new().export(),
    ))
    .unwrap();
    for leaked in [
        "TESTSERIAL",
        "Test Phone",
        "phoneA-abcdef",
        "192.168.50.2",
        "usb-target-private",
        "binding-private",
    ] {
        assert!(!json.contains(leaked), "{leaked} leaked into {json}");
    }
    assert!(json.contains("\"schemaVersion\":2"));
    assert!(json.contains("\"state\":\"connected\""));
    assert!(json.contains("\"playbackMode\":\"Balanced\""));
    assert!(json.contains("\"audioBitrateBps\":1536000"));
    assert!(json.contains("\"loss10sFrames\":10"));
    assert!(json.contains("\"eligible60sFrames\":200"));
    assert!(json.contains("\"cumulativeLossFrames\":30"));
    assert!(json.contains("\"cumulativeEligibleFrames\":300"));
}

#[test]
fn iso8601_formats_utc_seconds() {
    assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso8601_utc(1_700_000_000), "2023-11-14T22:13:20Z");
}

#[test]
fn background_trust_ipc_uses_camel_case_target_and_nullable_duration() {
    let active: BackgroundTrustTarget = serde_json::from_value(serde_json::json!({
        "target": "activeSession",
        "bindingReference": "ab".repeat(32),
        "sessionGeneration": 41
    }))
    .unwrap();
    assert!(matches!(
        active,
        BackgroundTrustTarget::ActiveSession {
            session_generation: 41,
            ..
        }
    ));

    let receiver: BackgroundTrustTarget = serde_json::from_value(serde_json::json!({
        "target": "receiver",
        "bindingReference": "cd".repeat(32),
        "localAddress": "192.168.1.10",
        "peer": "192.168.1.30:45678"
    }))
    .unwrap();
    assert!(
        matches!(receiver, BackgroundTrustTarget::Receiver { local_address, .. }
            if local_address == "192.168.1.10")
    );

    let status = BackgroundTrustStatus {
        target: "receiver".into(),
        binding_reference: "cd".repeat(32),
        session_generation: None,
        kind: "untilRevoked".into(),
        persistence: "binding".into(),
        remaining_ms: None,
    };
    let json = serde_json::to_value(status).unwrap();
    assert_eq!(json["target"], "receiver");
    assert_eq!(json["bindingReference"], "cd".repeat(32));
    assert!(json["sessionGeneration"].is_null());
    assert_eq!(json["kind"], "untilRevoked");
    assert_eq!(json["persistence"], "binding");
    assert!(json["remainingMs"].is_null());
    assert!(json.get("commitment").is_none());
}

#[test]
fn usb_playback_diagnostics_use_session_scoped_health_intervals() {
    let mut receiver = PlaybackDiagnostics {
        receiver_sample_rate: Some(48_000),
        receiver_channels: Some(2),
        pc_dropped_frames: None,
        pc_queued_frames: None,
        underrun_count: Some(12),
        ..PlaybackDiagnostics::default()
    };
    let (first, sample) = merge_usb_playback_diagnostics(
        receiver.clone(),
        Some(PcmFormat {
            sample_rate: 48_000,
            channels: 2,
        }),
        7,
        None,
    );
    assert_eq!(first.sample_rate, Some(48_000));
    assert_eq!(first.channels, Some(2));
    assert_eq!(first.pc_dropped_frames, None);
    assert_eq!(first.pc_queued_frames, None);
    assert_eq!(first.underrun_increment, None);
    assert_eq!(sample, Some((7, 12)));

    receiver.underrun_count = Some(15);
    let (next, sample) = merge_usb_playback_diagnostics(receiver.clone(), None, 7, sample);
    assert_eq!(next.underrun_increment, Some(3));
    assert_eq!(sample, Some((7, 15)));

    receiver.underrun_count = Some(1);
    let (new_session, sample) = merge_usb_playback_diagnostics(receiver.clone(), None, 8, sample);
    assert_eq!(new_session.underrun_increment, None);
    assert_eq!(sample, Some((8, 1)));
    receiver.underrun_count = None;
    let (unknown, sample) = merge_usb_playback_diagnostics(receiver, None, 8, sample);
    assert_eq!(unknown.underrun_increment, None);
    assert_eq!(sample, None);
}

struct SignalOnDrop(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for SignalOnDrop {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
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
    let args = [
        "--ignored",
        "--exact",
        "state::tests::adb_stdin_child_helper",
        "--nocapture",
    ];
    let cancel = CancellationToken::new();
    let child_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        adb_run_with_stdin(
            &executable,
            None,
            &args,
            b"synthetic",
            Duration::from_secs(20),
            &child_cancel,
        )
        .await
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
    assert!(
        drains[0].is_none(),
        "completed stdout handle must be consumed once"
    );
    assert!(
        drains[1].is_some(),
        "blocked stderr handle must remain available for cleanup"
    );
    stop_output_tasks(&mut drains).await;
    assert!(
        stderr_dropped_rx.await.is_ok(),
        "aborted stderr drain must be joined and dropped"
    );
}

#[test]
fn pcm_bitrate_uses_successful_byte_deltas_and_resets_between_sessions() {
    let started = Instant::now();
    assert_eq!(
        calculate_pcm_bitrate(
            Some((1, 0, started)),
            1,
            192_000,
            started + Duration::from_secs(1)
        ),
        1_536_000
    );
    assert_eq!(
        calculate_pcm_bitrate(
            Some((1, 900_000, started)),
            2,
            192_000,
            started + Duration::from_secs(1)
        ),
        0
    );
    assert_eq!(
        calculate_pcm_bitrate(
            Some((1, 192_000, started)),
            1,
            192_000,
            started + Duration::from_secs(1)
        ),
        0
    );
}

#[test]
fn stale_pcm_sample_is_rejected_after_session_changes() {
    let started = Instant::now();
    let mut previous = Some((2, 10, started));
    let mut current_rate = None;
    assert!(!commit_pcm_bitrate_sample(
        Some(2),
        1,
        &mut previous,
        &mut current_rate,
        192_010,
        started + Duration::from_secs(1)
    ));
    assert_eq!(
        previous.as_ref().map(|(id, count, _)| (*id, *count)),
        Some((2, 10))
    );
    assert_eq!(current_rate, None);
    assert!(commit_pcm_bitrate_sample(
        Some(2),
        2,
        &mut previous,
        &mut current_rate,
        192_010,
        started + Duration::from_secs(1)
    ));
    assert_eq!(
        previous.as_ref().map(|(id, count, _)| (*id, *count)),
        Some((2, 192_010))
    );
    assert_eq!(current_rate, Some(1_536_000));
}

#[test]
fn fresh_network_queue_is_handed_off_even_without_observing_none() {
    let old = Arc::new(PcmQueue::new());
    let new = Arc::new(PcmQueue::new());
    assert!(!pcm_queue_identity_changed(Some(&old), Some(&old)));
    assert!(pcm_queue_identity_changed(Some(&old), Some(&new)));
}

#[test]
fn management_only_recovery_selects_computer_output() {
    assert_eq!(monitored_network_output("phone", true), "computer");
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
    assert_eq!(value["wiredMode"], "aoa");
    assert_eq!(value["lastUsbDirectTargetId"], serde_json::Value::Null);
    assert_eq!(
        value["lastUsbDirectBindingReference"],
        serde_json::Value::Null
    );
    assert_eq!(value["wirelessProtocol"], "udp");
    assert_eq!(value["audioBackend"], "receiver");
    assert_eq!(value["playbackMode"], "Balanced");
    let settings = normalize_settings(
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Wifi",
            "wirelessProtocol": "adb", "audioBackend": "receiver"
        }))
        .unwrap(),
    );
    assert_eq!(settings.preferred_transport, "Wifi");
    assert_eq!(settings.wireless_protocol, "adb");
    assert_eq!(settings.audio_backend, "receiver");
    let empty_backend = normalize_settings(
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "audioBackend": ""
        }))
        .unwrap(),
    );
    assert_eq!(empty_backend.audio_backend, "receiver");
    let explicit_legacy = normalize_settings(
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "audioBackend": "legacy"
        }))
        .unwrap(),
    );
    assert_eq!(explicit_legacy.audio_backend, "legacy");
    let old_saved = normalize_settings(
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Wifi", "audioQuality": "hires"
        }))
        .unwrap(),
    );
    assert_eq!(old_saved.preferred_transport, "Wifi");
    assert_eq!(old_saved.wireless_protocol, "adb");
    assert_eq!(old_saved.audio_quality, "hires");
    assert_eq!(old_saved.audio_backend, "receiver");
    let legacy_adb = normalize_settings(
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Wifi",
            "wirelessProtocol": "adb", "audioBackend": "legacy", "audioQuality": "hires"
        }))
        .unwrap(),
    );
    assert_eq!(legacy_adb.audio_quality, "standard");
    let new_saved = normalize_settings(
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Usb", "wirelessProtocol": "udp"
        }))
        .unwrap(),
    );
    assert_eq!(new_saved.wireless_protocol, "udp");
    assert_eq!(new_saved.audio_backend, "receiver");
    let explicit_udp_wifi = normalize_settings(
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Wifi", "wirelessProtocol": "udp"
        }))
        .unwrap(),
    );
    assert_eq!(explicit_udp_wifi.wireless_protocol, "udp");
    let udp_hires = normalize_settings(AppSettings {
        preferred_transport: "Wifi".into(),
        wireless_protocol: "udp".into(),
        audio_quality: "hires".into(),
        ..AppSettings::default()
    });
    assert_eq!(udp_hires.audio_quality, "hires");
    let direct_hires = normalize_settings(AppSettings {
        wired_mode: "aoa".into(),
        audio_quality: "hires".into(),
        ..AppSettings::default()
    });
    assert_eq!(direct_hires.wired_mode, "aoa");
    assert_eq!(direct_hires.audio_quality, "hires");
    assert_eq!(
        reconfigure_selection("transport", "wireless", &old_saved),
        Ok(ReconfigureSelection::WirelessAdb)
    );
    assert_eq!(
        reconfigure_selection("wirelessProtocol", "udp", &old_saved),
        Ok(ReconfigureSelection::WirelessUdp)
    );
    assert!(reconfigure_selection("wirelessProtocol", "invalid", &old_saved).is_err());

    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        ".tmp-settings-default-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut settings_state = AppState::new();
    settings_state.data_dir = test_dir.clone();
    settings_state.load_settings();
    assert_eq!(
        settings_state.snapshot().settings.audio_backend,
        "receiver",
        "a missing settings file must use the Receiver default"
    );

    fs::create_dir_all(&test_dir).unwrap();
    fs::write(
        test_dir.join("settings.json"),
        r#"{"schemaVersion":1,"preferredTransport":"Wifi"}"#,
    )
    .unwrap();
    settings_state.load_settings();
    let missing_backend = settings_state.snapshot().settings;
    assert_eq!(
        missing_backend.audio_backend, "receiver",
        "a saved settings file without audioBackend must use Receiver"
    );
    assert_eq!(
        missing_backend.wireless_protocol, "adb",
        "the existing Wifi-to-ADB migration must remain unchanged"
    );
    settings_state.save_settings(&explicit_legacy).unwrap();
    settings_state.load_settings();
    assert_eq!(
        settings_state.snapshot().settings.audio_backend,
        "legacy",
        "an explicit Legacy choice must survive save and load"
    );
    let explicit_receiver = AppSettings {
        audio_backend: "receiver".into(),
        ..AppSettings::default()
    };
    settings_state.save_settings(&explicit_receiver).unwrap();
    settings_state.load_settings();
    assert_eq!(
        settings_state.snapshot().settings.audio_backend,
        "receiver",
        "an explicit Receiver choice must survive save and load"
    );
    fs::remove_dir_all(test_dir).unwrap();

    let state = AppState::new();
    state.view.lock().unwrap().settings.preferred_transport = "Wifi".into();
    assert_eq!(state.snapshot().connection_transport, "wireless");
    assert_eq!(state.snapshot().transport, "wifi");
    assert_eq!(session_connection_transport("lan"), Some("wireless"));
    assert_eq!(session_wireless_protocol("lan"), Some("udp"));
    assert_eq!(session_connection_transport("wifi"), Some("wireless"));
    assert_eq!(session_wireless_protocol("wifi"), Some("adb"));
    assert_eq!(
        normalize_endpoint("192.0.2.2:5555").unwrap(),
        "192.0.2.2:5555"
    );
    assert!(normalize_endpoint("192.0.2.2:0").is_err());
}

#[test]
fn adb_mode_quality_updates_are_atomic_and_usb_direct_memory_is_persisted() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        ".tmp-wired-settings-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut state = AppState::new();
    state.data_dir = root.clone();

    assert!(state
        .update_setting("wiredMode", serde_json::json!("invalid"))
        .is_err());
    state
        .update_setting("wiredMode", serde_json::json!("aoa"))
        .unwrap();
    state
        .update_setting("audioQuality", serde_json::json!("hires"))
        .unwrap();
    assert_eq!(state.snapshot().settings.audio_quality, "hires");

    state
        .update_setting("preferredTransport", serde_json::json!("Wifi"))
        .unwrap();
    state
        .update_setting("wirelessProtocol", serde_json::json!("adb"))
        .unwrap();
    assert_eq!(state.snapshot().settings.audio_quality, "hires");
    state
        .update_setting("audioQuality", serde_json::json!("hifi"))
        .unwrap();
    state
        .update_setting("audioBackend", serde_json::json!("legacy"))
        .unwrap();
    assert_eq!(state.snapshot().settings.audio_quality, "standard");
    assert!(state
        .update_setting("audioQuality", serde_json::json!("hifi"))
        .is_err());
    state
        .update_setting("wirelessProtocol", serde_json::json!("udp"))
        .unwrap();
    state
        .update_setting("audioQuality", serde_json::json!("hires"))
        .unwrap();
    state
        .update_setting("wiredMode", serde_json::json!("adb"))
        .unwrap();
    state
        .update_setting("preferredTransport", serde_json::json!("Usb"))
        .unwrap();
    let snapshot = state.snapshot();
    assert_eq!(snapshot.settings.audio_quality, "standard");
    assert_eq!(snapshot.settings.wired_mode, "adb");
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["wiredMode"], "adb");
    assert_eq!(saved["audioQuality"], "standard");
    assert!(state
        .update_setting("audioQuality", serde_json::json!("hires"))
        .is_err());

    state
        .update_setting("wiredMode", serde_json::json!("aoa"))
        .unwrap();
    state
        .update_setting("audioQuality", serde_json::json!("hifi"))
        .unwrap();
    state
        .remember_usb_direct_target("usb-target-private", "binding-private")
        .unwrap();
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.settings.last_usb_direct_target_id.as_deref(),
        Some("usb-target-private")
    );
    assert_eq!(
        snapshot
            .settings
            .last_usb_direct_binding_reference
            .as_deref(),
        Some("binding-private")
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["lastUsbDirectTargetId"], "usb-target-private");
    assert_eq!(saved["lastUsbDirectBindingReference"], "binding-private");

    let mut diagnostic_snapshot = sample_snapshot();
    diagnostic_snapshot.settings = snapshot.settings;
    let diagnostic = serde_json::to_string(&diagnostics_export(
        &diagnostic_snapshot,
        DiagnosticsTimelineBuffer::new().export(),
    ))
    .unwrap();
    assert!(!diagnostic.contains("usb-target-private"));
    assert!(!diagnostic.contains("binding-private"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn operation_owner_cannot_be_cleared_by_an_older_completion() {
    let generation = AtomicU64::new(0);
    let slot = Mutex::new(None);
    let (first_generation, first_token) = begin_operation_slot(&generation, &slot, false).unwrap();
    let (current_generation, current_token) =
        begin_operation_slot(&generation, &slot, false).unwrap();
    assert!(first_token.is_cancelled());
    finish_operation_slot(&slot, first_generation);
    assert!(slot
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(
            |active| active.generation == current_generation && !active.token.is_cancelled()
        ));
    assert!(!current_token.is_cancelled());
}

#[test]
fn aoa_switch_reservation_rejects_existing_operations_without_cancelling_them() {
    let generation = AtomicU64::new(0);
    let slot = Mutex::new(None);
    let (_, current) = begin_operation_slot(&generation, &slot, false).unwrap();

    assert!(try_begin_exclusive_operation(&generation, &slot).is_none());
    assert!(!current.is_cancelled());
}

#[test]
fn cancelled_aoa_start_cannot_publish_over_a_new_operation_generation() {
    let generation = AtomicU64::new(0);
    let slot = Mutex::new(None);
    let (old_generation, old_token) = begin_operation_slot(&generation, &slot, false).unwrap();
    cancel_operation_slot(&generation, &slot);
    assert!(old_token.is_cancelled());
    assert!(!operation_generation_matches(&generation, old_generation));

    let (new_generation, new_token) = begin_operation_slot(&generation, &slot, false).unwrap();
    finish_operation_slot(&slot, old_generation);
    assert!(operation_generation_matches(&generation, new_generation));
    assert!(!new_token.is_cancelled());
    assert!(slot
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|operation| operation.generation == new_generation));
}

#[test]
fn aoa_snapshot_keeps_transport_and_binding_separate_from_adb_identity() {
    let state = AppState::new();
    {
        let mut view = state.view.lock().unwrap();
        view.settings.preferred_transport = "Wifi".into();
        view.selected_serial = Some("existing-adb-serial".into());
        view.session_active = true;
        view.session_serial = None;
        view.session_receiver_binding = Some("b".repeat(64));
        view.session_transport = Some("aoa".into());
        view.output = "phone".into();
    }

    let snapshot = state.snapshot();
    assert_eq!(snapshot.session_transport.as_deref(), Some("aoa"));
    assert_eq!(
        snapshot.session_connection_transport.as_deref(),
        Some("usbDirect")
    );
    assert_eq!(snapshot.session_wireless_protocol, None);
    assert_eq!(snapshot.session_serial, None);
    assert_eq!(
        snapshot.selected_serial.as_deref(),
        Some("existing-adb-serial")
    );
    assert_eq!(snapshot.session_receiver_binding, Some("b".repeat(64)));
    assert_eq!(snapshot.transport, "wifi");
    assert_eq!(snapshot.connection_transport, "wireless");
    assert!(!snapshot.can_change_modes);
}

#[test]
fn wireless_protocol_switch_rejects_completion_from_previous_owner() {
    let generation = AtomicU64::new(0);
    let slot = Mutex::new(None);
    let (old_generation, old_token) = begin_operation_slot(&generation, &slot, false).unwrap();
    let (new_generation, new_token) = begin_operation_slot(&generation, &slot, false).unwrap();
    assert!(old_token.is_cancelled());
    finish_operation_slot(&slot, old_generation);
    assert!(slot
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|operation| operation.generation == new_generation));
    assert!(!new_token.is_cancelled());
}

#[test]
fn background_operation_does_not_preempt_a_foreground_request() {
    let generation = AtomicU64::new(0);
    let slot = Mutex::new(None);
    let (foreground_generation, foreground_token) =
        begin_operation_slot(&generation, &slot, false).unwrap();
    assert!(begin_operation_slot(&generation, &slot, true).is_none());
    assert!(slot
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|active| active.generation == foreground_generation));
    assert!(!foreground_token.is_cancelled());
    assert_eq!(automatic_reception_binding(&[], None).unwrap(), None);
    let references = vec!["saved-a".to_string(), "saved-b".to_string()];
    assert_eq!(
        automatic_reception_binding(&references[..1], None).unwrap(),
        Some("saved-a".into())
    );
    assert_eq!(
        automatic_reception_binding(&references, None).unwrap(),
        None,
        "several saved identities need selection"
    );
    assert_eq!(
        automatic_reception_binding(&references, Some("saved-b")).unwrap(),
        Some("saved-b".into())
    );
    assert!(automatic_reception_binding(&references, Some("untrusted-hint")).is_err());
}

#[test]
fn foreground_receiver_operation_preempts_only_background_monitoring() {
    let generation = AtomicU64::new(0);
    let slot = Mutex::new(None);
    let (background_generation, background_token) =
        begin_preemptible_background_operation(&generation, &slot).unwrap();
    let (foreground_generation, foreground_token) =
        begin_foreground_receiver_operation(&generation, &slot).unwrap();
    assert!(background_token.is_cancelled());
    finish_operation_slot(&slot, background_generation);
    assert!(slot.lock().unwrap().as_ref().is_some_and(|active| {
        active.generation == foreground_generation && !active.token.is_cancelled()
    }));
    assert!(begin_foreground_receiver_operation(&generation, &slot).is_none());
    assert!(!foreground_token.is_cancelled());
}

#[test]
fn native_disconnect_is_detected_when_adb_scan_fails() {
    assert!(session_is_lost(None, false, false));
    assert!(!session_is_lost(None, false, true));
    assert!(session_is_lost(Some(false), false, true));
}

#[test]
fn active_device_selection_is_idempotent_and_cannot_change_session() {
    let state = AppState::new();
    let make = |serial: &str| Device {
        serial: serial.into(),
        physical_id: None,
        transport: "usb".into(),
        state: "device".into(),
        manufacturer: "Google".into(),
        model: "Pixel".into(),
        android_version: "15".into(),
        api_level: "35".into(),
        display_name: serial.into(),
        status_text: "可连接".into(),
        connectable: true,
    };
    {
        let mut view = state.view.lock().unwrap();
        view.devices = vec![make("TEST-A"), make("TEST-B")];
        view.selected_serial = Some("TEST-A".into());
        view.session_serial = Some("TEST-A".into());
        view.session_active = true;
    }

    assert_eq!(
        state
            .select_device("TEST-A")
            .unwrap()
            .selected_serial
            .as_deref(),
        Some("TEST-A")
    );
    assert!(state.select_device("TEST-B").is_err());
    let snapshot = state.snapshot();
    assert_eq!(snapshot.session_serial.as_deref(), Some("TEST-A"));
    assert_eq!(snapshot.selected_serial.as_deref(), Some("TEST-A"));
    let scanned_devices = [make("TEST-B")];
    assert_eq!(
        choose_selected_serial(
            snapshot.session_serial,
            snapshot.selected_serial,
            &scanned_devices,
            "usb"
        )
        .as_deref(),
        Some("TEST-A")
    );
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
    assert_eq!(
        snapshot.settings.last_wireless_endpoint.as_deref(),
        Some("192.0.2.1:5555")
    );
}

#[test]
fn transport_switch_requires_a_proven_physical_identity() {
    let make = |serial: &str, physical_id: Option<&str>| Device {
        serial: serial.into(),
        physical_id: physical_id.map(str::to_owned),
        transport: "usb".into(),
        state: "device".into(),
        manufacturer: "Google".into(),
        model: "Pixel".into(),
        android_version: "15".into(),
        api_level: "35".into(),
        display_name: "Google Pixel".into(),
        status_text: "可连接".into(),
        connectable: true,
    };
    assert!(same_physical_device(
        &make("USB-1", Some("SERIAL")),
        &make("192.0.2.1:5555", Some("SERIAL"))
    ));
    assert!(!same_physical_device(
        &make("USB-1", None),
        &make("192.0.2.1:5555", None)
    ));
    assert!(!same_physical_device(
        &make("USB-1", Some("SERIAL-A")),
        &make("192.0.2.1:5555", Some("SERIAL-B"))
    ));
}

#[test]
fn receiver_reset_cannot_stop_a_session_for_another_target() {
    assert!(receiver_target_matches(
        "USB-A",
        false,
        None,
        "USB-A",
        Some("reference-a")
    ));
    assert!(!receiver_target_matches(
        "USB-B",
        false,
        None,
        "USB-A",
        Some("reference-a")
    ));
    assert!(receiver_target_matches(
        "USB-A",
        true,
        Some("reference-a"),
        "USB-Z",
        Some("reference-a")
    ));
    assert!(!receiver_target_matches(
        "USB-A",
        true,
        Some("reference-b"),
        "USB-A",
        Some("reference-a")
    ));
    assert!(!receiver_target_matches(
        "USB-A",
        true,
        Some("reference-a"),
        "USB-A",
        None
    ));
}

#[test]
fn operation_slot_keeps_reset_and_offer_reservations_exclusive() {
    let generation = AtomicU64::new(0);
    let slot = Mutex::new(None);
    let (reset_generation, reset_token) = begin_operation_slot(&generation, &slot, true).unwrap();
    assert!(begin_operation_slot(&generation, &slot, true).is_none());
    assert!(begin_foreground_receiver_operation(&generation, &slot).is_none());
    assert!(!reset_token.is_cancelled());
    finish_operation_slot(&slot, reset_generation);

    let (offer_generation, offer_token) = begin_operation_slot(&generation, &slot, true).unwrap();
    assert!(begin_operation_slot(&generation, &slot, true).is_none());
    assert!(begin_foreground_receiver_operation(&generation, &slot).is_none());
    assert!(!offer_token.is_cancelled());
    finish_operation_slot(&slot, offer_generation);
}

#[test]
fn binding_reset_preserves_only_the_matching_listener_and_unchanged_saved_binding() {
    let expected = receiver::BindingResetPreview {
        serial: "USB-A".into(),
        android_user: "10".into(),
        binding_reference: "reference-a".into(),
        phone_state: "saved".into(),
        phone_commitment: Some("a".repeat(64)),
        pc_state: "saved".into(),
        pc_commitment: Some("b".repeat(64)),
        receiver_busy: false,
    };
    assert!(wireless_listener_matches_binding(
        Some("reference-a"),
        "reference-a"
    ));
    assert!(!wireless_listener_matches_binding(
        Some("reference-b"),
        "reference-a"
    ));
    assert!(!wireless_listener_matches_binding(None, "reference-a"));
    let mut result = receiver::BindingResetResult {
        outcome: "phoneRetained".into(),
        serial: expected.serial.clone(),
        android_user: expected.android_user.clone(),
        binding_reference: expected.binding_reference.clone(),
        phone_state: "saved".into(),
        phone_commitment: expected.phone_commitment.clone(),
        pc_state: "saved".into(),
        pc_commitment: expected.pc_commitment.clone(),
        reason: String::new(),
        next_action: String::new(),
    };
    assert!(reset_preserves_wireless_listener(&result, &expected));
    for (phone_state, pc_state) in [
        ("empty", "missing"),
        ("unknown", "saved"),
        ("saved", "missing"),
    ] {
        result.phone_state = phone_state.into();
        result.pc_state = pc_state.into();
        assert!(!reset_preserves_wireless_listener(&result, &expected));
    }
    result.phone_state = "saved".into();
    result.pc_state = "saved".into();
    result.pc_commitment = Some("c".repeat(64));
    assert!(!reset_preserves_wireless_listener(&result, &expected));
}

#[test]
fn receiver_reset_preflight_rejects_pending_or_failed_phone_state_before_stop() {
    let preview = receiver::BindingResetPreview {
        serial: "USB-A".into(),
        android_user: "10".into(),
        binding_reference: "reference-a".into(),
        phone_state: "saved".into(),
        phone_commitment: Some("a".repeat(64)),
        pc_state: "saved".into(),
        pc_commitment: Some("b".repeat(64)),
        receiver_busy: false,
    };
    assert!(validate_receiver_reset_preview(&preview, &preview).is_ok());
    for state in ["pending", "failed"] {
        let invalid = receiver::BindingResetPreview {
            phone_state: state.into(),
            phone_commitment: None,
            ..preview.clone()
        };
        assert!(
            validate_receiver_reset_preview(&invalid, &invalid).is_err(),
            "{state} must be rejected before any session teardown"
        );
    }
    let changed = receiver::BindingResetPreview {
        android_user: "11".into(),
        ..preview.clone()
    };
    assert!(validate_receiver_reset_preview(&changed, &preview).is_err());
}
