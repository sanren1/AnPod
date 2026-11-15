use super::*;
use crate::hdc_bootstrap::{encode_anhb, read_anhb};
use sha2::{Digest, Sha256};
use std::io::Write;

fn settings_with_quality(quality: &str) -> AppSettings {
    AppSettings {
        audio_quality: quality.into(),
        ..AppSettings::default()
    }
}

fn harmony_device(key: &str) -> Device {
    crate::hdc::harmony_device(key, "7.0.0", "MatePad")
}

fn temp_data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("anpod-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn harmony_session(key: &str) -> Session {
    Session {
        generation: 7,
        serial: key.into(),
        transport: "usb".into(),
        reverse: String::new(),
        child: None,
        installed_receiver: false,
        receiver_owner_id: None,
        receiver_generation: None,
        output: "phone".into(),
        output_tail: Arc::new(Mutex::new(VecDeque::new())),
        network_owner: None,
        aoa_owner: None,
        adb_stream_owner: None,
        aoa_target_id: None,
        network_sink_queue: None,
        network_binding_ref: None,
        windows_mix_format: None,
        hdc_forward: Some(HdcForward {
            key: key.into(),
            task: "tcp:47000 tcp:50123".into(),
        }),
    }
}

#[test]
fn hdc_connect_rejects_standard_quality_before_hdc() {
    // hdc_wired_profile is the first statement of connect_hdc_stream_inner, so a
    // rejected profile returns before any hdc command can run.
    assert_eq!(
        hdc_wired_profile(&settings_with_quality("standard")),
        Err("ADB Float32 audio requires HiFi or Hi-Res quality.".to_string())
    );
    assert!(hdc_wired_profile(&settings_with_quality("bogus")).is_err());
    assert_eq!(hdc_wired_profile(&settings_with_quality("hifi")), Ok(4));
    assert_eq!(hdc_wired_profile(&settings_with_quality("hires")), Ok(5));
}

#[test]
fn hdc_teardown_treats_missing_forward_as_removed() {
    let result = |success: bool, stdout: &str| AdbResult {
        success,
        stdout: stdout.into(),
        ..Default::default()
    };
    assert_eq!(
        hdc_fport_remove_result(result(
            true,
            "Remove forward ruler success, ruler:tcp:47000 tcp:50123\r\n"
        )),
        Ok(())
    );
    assert_eq!(
        hdc_fport_remove_result(result(
            true,
            "[Fail]Remove forward ruler failed, ruler is not exist tcp:47000 tcp:50123\r\n"
        )),
        Ok(())
    );
    assert!(
        hdc_fport_remove_result(result(true, "[Fail]ExecuteCommand need connect-key?")).is_err()
    );
    assert!(hdc_fport_remove_result(result(false, "")).is_err());
}

#[test]
fn answer_harmony_pairing_without_pending_is_error() {
    let state = AppState::new();
    assert!(state.answer_harmony_pairing(true).is_err());

    let (sender, receiver) = std::sync::mpsc::channel();
    state.set_harmony_pairing("123456".into(), sender);
    assert_eq!(
        state.snapshot().harmony_pairing_code.as_deref(),
        Some("123456")
    );
    let snapshot = state.answer_harmony_pairing(false).unwrap();
    assert_eq!(snapshot.harmony_pairing_code, None);
    assert_eq!(receiver.recv(), Ok(false));
    // The answer is single use.
    assert!(state.answer_harmony_pairing(true).is_err());
}

#[test]
fn hdc_command_output_is_judged_by_text_not_exit_code() {
    let result = |stdout: &str| AdbResult {
        success: true,
        stdout: stdout.into(),
        ..Default::default()
    };
    assert!(hdc_command_ok(&result("Forwardport result:OK")));
    assert!(hdc_command_ok(&result("start ability successfully.")));
    assert!(!hdc_command_ok(&result("[Fail]Forward parament failed")));
    assert!(!hdc_command_ok(&result("error: failed to start ability.")));
}

#[tokio::test]
async fn harmony_user_stopped_revokes_reconnect() {
    let caps = crate::p5c::CAP_TERMINAL_CAUSE;
    assert!(matches!(
        record_stream_exit(Some(TerminalCause::ReceiverUserStop), caps),
        Some(RecordStreamExit::Finish(_))
    ));
    // A HarmonyOS session that is finished leaves nothing for a later recovery to act on.
    let state = AppState::new();
    state.remember_pending_adb_cleanup(pending_adb_cleanup_for(&harmony_session("HW1")));
    assert_eq!(
        state
            .cleanup_pending_adb_for_target("HW1", "usb", &CancellationToken::new())
            .await,
        Ok(AdbCleanupOutcome::Complete)
    );
    assert!(state.pending_adb_cleanup.lock().unwrap().is_empty());
}

#[tokio::test]
async fn harmony_unexpected_drop_reconnects_via_hdc() {
    let caps = crate::p5c::CAP_TERMINAL_CAUSE;
    assert_eq!(
        record_stream_exit(Some(TerminalCause::TransportFailure), caps),
        Some(RecordStreamExit::Recover)
    );
    // Recovery cleanup of a HarmonyOS session needs no adb work at all.
    let cleanup = pending_adb_cleanup_for(&harmony_session("HW1"));
    assert!(!cleanup.installed_receiver && cleanup.receiver_stopped && cleanup.reverse_removed);

    // Recovery reconnects through connect_locked, which routes the tablet to hdc.
    let device = harmony_device("HW1");
    assert!(connects_over_hdc(&device));
    let android = Device {
        platform: "android".into(),
        ..device.clone()
    };
    assert!(!connects_over_hdc(&android));
    let ops = HdcRecoveryOps {
        device: device.clone(),
        bootstrapped: std::sync::Mutex::new(Vec::new()),
    };
    assert!(matches!(
        run_adb_recovery_attempt(&ops, "HW1", "usb", &CancellationToken::new()).await,
        AdbRecoveryAttempt::Connected
    ));
    assert_eq!(*ops.bootstrapped.lock().unwrap(), vec![true]);
}

struct HdcRecoveryOps {
    device: Device,
    bootstrapped: std::sync::Mutex<Vec<bool>>,
}

impl AdbRecoveryAttemptOps for HdcRecoveryOps {
    fn find_device<'a>(
        &'a self,
        _serial: &'a str,
        _transport: &'a str,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Vec<Device>> + Send + 'a>> {
        Box::pin(async move { vec![self.device.clone()] })
    }

    fn cleanup_pending<'a>(
        &'a self,
        _serial: &'a str,
        _transport: &'a str,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<AdbCleanupOutcome, String>> + Send + 'a>> {
        Box::pin(async { Ok(AdbCleanupOutcome::Complete) })
    }

    fn bootstrap<'a>(
        &'a self,
        device: Device,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            self.bootstrapped
                .lock()
                .unwrap()
                .push(connects_over_hdc(&device));
            true
        })
    }
}

/// Plays the tablet side of pairing up to the user's answer on the PC.
fn fake_tablet(mut tablet: TcpStream, tablet_accepts: bool) -> std::thread::JoinHandle<u8> {
    std::thread::spawn(move || {
        let (kind, offer) = read_anhb(&mut tablet).unwrap();
        assert_eq!(kind, 2);
        tablet.write_all(&encode_anhb(3, &[5; 16])).unwrap();
        let (answer, _) = read_anhb(&mut tablet).unwrap();
        if answer == 4 && tablet_accepts {
            let commitment = Sha256::digest(&offer[..offer.len() - 16]);
            tablet.write_all(&encode_anhb(5, &commitment)).unwrap();
        }
        answer
    })
}

fn socket_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tablet = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (host, _) = listener.accept().unwrap();
    (host, tablet)
}

#[test]
fn harmony_pairing_declined_keeps_no_material() {
    let data_dir = temp_data_dir("harmony-declined");
    let reference = BindingMaterial::harmony_binding_reference("HW1");
    let (mut host, tablet) = socket_pair();
    let tablet = fake_tablet(tablet, true);
    let material = BindingMaterial::generate().unwrap();
    let shown = std::cell::Cell::new(false);
    let result = pair_and_store(
        &mut host,
        &data_dir,
        &reference,
        "MatePad",
        &material,
        |code| {
            assert_eq!(code.len(), 6);
            shown.set(true);
            false
        },
    );
    assert!(result.is_err());
    assert!(shown.get());
    assert_eq!(tablet.join().unwrap(), 6, "tablet receives PairCancel");
    let path = binding_store_path(&data_dir, &reference).unwrap();
    assert!(!path.exists());
    assert!(!path.with_extension("json").exists());
    let _ = fs::remove_dir_all(&data_dir);
}

#[test]
fn harmony_pairing_accepted_stores_material_and_profile() {
    let data_dir = temp_data_dir("harmony-accepted");
    let reference = BindingMaterial::harmony_binding_reference("HW1");
    let (mut host, tablet) = socket_pair();
    let tablet = fake_tablet(tablet, true);
    let material = BindingMaterial::generate().unwrap();
    pair_and_store(
        &mut host,
        &data_dir,
        &reference,
        "MatePad",
        &material,
        |_| true,
    )
    .unwrap();
    assert_eq!(tablet.join().unwrap(), 4);
    let path = binding_store_path(&data_dir, &reference).unwrap();
    assert_eq!(
        BindingStore::load(&path).unwrap().commitment(),
        material.commitment()
    );
    let profile = load_binding_profile(&data_dir, &reference);
    assert_eq!(profile.platform, "harmony");
    assert_eq!(profile.model.as_deref(), Some("MatePad"));

    // Re-pairing after the tablet lost its binding replaces the old record.
    forget_harmony_binding(&data_dir, &reference).unwrap();
    assert!(!path.exists() && !path.with_extension("json").exists());
    forget_harmony_binding(&data_dir, &reference).unwrap();
    let _ = fs::remove_dir_all(&data_dir);
}

#[test]
fn harmony_repair_keeps_the_launched_host_id() {
    let old = BindingMaterial::generate().unwrap();
    let fresh = old.regenerate_keeping_host().unwrap();
    assert_eq!(fresh.host_id(), old.host_id());
    assert_ne!(fresh.commitment(), old.commitment());
}
