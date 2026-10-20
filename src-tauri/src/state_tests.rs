use super::*;

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
    let json = serde_json::to_string(&diagnostics_export(&sample_snapshot())).unwrap();
    for leaked in ["TESTSERIAL", "Test Phone", "phoneA-abcdef", "192.168.50.2"] {
        assert!(!json.contains(leaked), "{leaked} leaked into {json}");
    }
    assert!(json.contains("\"schemaVersion\":1"));
    assert!(json.contains("\"state\":\"connected\""));
    assert!(json.contains("\"playbackMode\":\"Balanced\""));
    assert!(json.contains("\"audioBitrateBps\":1536000"));
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
            "schemaVersion": 1, "preferredTransport": "Wifi"
        }))
        .unwrap(),
    );
    assert_eq!(old_saved.preferred_transport, "Wifi");
    assert_eq!(old_saved.wireless_protocol, "adb");
    assert_eq!(old_saved.audio_backend, "receiver");
    let new_saved = normalize_settings(
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Usb", "wirelessProtocol": "udp"
        }))
        .unwrap(),
    );
    assert_eq!(new_saved.wireless_protocol, "udp");
    assert_eq!(new_saved.audio_backend, "receiver");
    assert_eq!(selected_transport_allows_auto_adb(&old_saved), true);
    assert_eq!(selected_transport_allows_auto_adb(&new_saved), true);
    let explicit_udp_wifi = normalize_settings(
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "preferredTransport": "Wifi", "wirelessProtocol": "udp"
        }))
        .unwrap(),
    );
    assert_eq!(explicit_udp_wifi.wireless_protocol, "udp");
    assert!(!selected_transport_allows_auto_adb(&explicit_udp_wifi));
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
fn receiver_health_monitor_skips_known_local_or_transport_failures() {
    assert!(receiver_health_needed(true, Some(true), false, true));
    assert!(!receiver_health_needed(false, Some(true), false, true));
    assert!(!receiver_health_needed(true, Some(false), false, true));
    assert!(!receiver_health_needed(true, Some(true), true, true));
    assert!(!receiver_health_needed(true, Some(true), false, false));
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
