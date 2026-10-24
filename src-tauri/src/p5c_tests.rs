use super::*;

fn aoa_vectors() -> std::collections::HashMap<String, Vec<u8>> {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/aoa-v1-vectors.txt"
    ))
    .unwrap()
    .lines()
    .filter_map(|line| {
        let (name, value) = line.split_once('=')?;
        if value.is_empty()
            || !value.len().is_multiple_of(2)
            || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        Some((
            name.to_owned(),
            (0..value.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
                .collect(),
        ))
    })
    .collect()
}

#[test]
fn aoa_p5c_control_vectors_match_existing_encoder_for_all_profiles() {
    let vectors = aoa_vectors();
    for profile in 1..=3 {
        let prefix = format!("profile.{profile}");
        let session = 0x0102_0304_0506_0708 + (profile as u64 - 1);
        let key: [u8; 32] = vectors[&format!("{prefix}.key_h2r_control")]
            .as_slice()
            .try_into()
            .unwrap();
        let query = encode_control(
            &key,
            HOST_TO_RECEIVER_CONTROL,
            session,
            0,
            CAPABILITIES_QUERY,
            &vectors[&format!("{prefix}.capabilities_query_payload")],
        )
        .unwrap();
        assert_eq!(query, vectors[&format!("{prefix}.anpc_capabilities_query")]);
        let bind = encode_control(
            &key,
            HOST_TO_RECEIVER_CONTROL,
            session,
            1,
            BIND,
            &vectors[&format!("{prefix}.aoa_bind_payload")],
        )
        .unwrap();
        assert_eq!(bind, vectors[&format!("{prefix}.anpc_aoa_bind")]);
    }
}

#[test]
fn background_authorization_ack_enforces_exact_length_and_status_range() {
    let request_id = [0x35; 16];
    let make_ack = |payload_len: u16, status: u8| {
        let mut frame = vec![0; CONTROL_HEADER_BYTES + payload_len as usize + CONTROL_TAG_BYTES];
        frame[5] = ACK;
        frame[20..28].copy_from_slice(&0u64.to_be_bytes());
        frame[28..30].copy_from_slice(&payload_len.to_be_bytes());
        frame[30] = BACKGROUND_START_REQUEST;
        frame[31..39].copy_from_slice(&0u64.to_be_bytes());
        frame[39] = RESULT_OK;
        frame[40] = 0;
        frame[43..59].copy_from_slice(&request_id);
        if payload_len > 29 {
            frame[59] = status;
        }
        frame
    };
    let decoded = decode_ack(&make_ack(30, AUTH_ONCE), 0, BACKGROUND_START_REQUEST, 0).unwrap();
    assert_eq!(
        decoded.background_authorization,
        Some((request_id, AUTH_ONCE))
    );
    assert!(decode_ack(&make_ack(29, AUTH_ONCE), 0, BACKGROUND_START_REQUEST, 0).is_err());
    assert!(decode_ack(&make_ack(30, 7), 0, BACKGROUND_START_REQUEST, 0).is_err());
}

#[test]
fn extension_commands_accept_only_canonical_user_stopped_ack() {
    let make_ack = |command: u8, result: u8, state: u8, port: u16, sequence: u64| {
        let mut frame = vec![0; CONTROL_HEADER_BYTES + 13 + CONTROL_TAG_BYTES];
        frame[5] = ACK;
        frame[20..28].copy_from_slice(&0u64.to_be_bytes());
        frame[28..30].copy_from_slice(&13u16.to_be_bytes());
        frame[30] = command;
        frame[31..39].copy_from_slice(&sequence.to_be_bytes());
        frame[39] = result;
        frame[40] = state;
        frame[41..43].copy_from_slice(&port.to_be_bytes());
        frame
    };

    for command in [
        PLAYBACK_PROGRESS_QUERY,
        BACKGROUND_START_REQUEST,
        BACKGROUND_TRUST_QUERY,
    ] {
        let ack = decode_ack_with_format(
            &make_ack(command, RESULT_USER_STOPPED, 5, 0, 7),
            0,
            command,
            7,
            true,
        )
        .unwrap();
        assert_eq!(ack.result, RESULT_USER_STOPPED);
        assert_eq!(ack.state, ReceiverState::Stopped);
        assert_eq!(ack.udp_port, 0);
        assert!(ack.playback_progress.is_none());
        assert!(ack.background_authorization.is_none());
        assert!(ack.background_trust.is_none());
    }

    let progress = make_ack(PLAYBACK_PROGRESS_QUERY, RESULT_USER_STOPPED, 5, 0, 7);
    assert!(decode_ack_with_format(&progress, 1, PLAYBACK_PROGRESS_QUERY, 7, true).is_err());
    assert!(decode_ack_with_format(
        &make_ack(PLAYBACK_PROGRESS_QUERY, RESULT_USER_STOPPED, 5, 0, 8),
        0,
        PLAYBACK_PROGRESS_QUERY,
        7,
        true,
    )
    .is_err());
    assert!(decode_ack_with_format(
        &make_ack(PLAYBACK_PROGRESS_QUERY, RESULT_USER_STOPPED, 2, 0, 7),
        0,
        PLAYBACK_PROGRESS_QUERY,
        7,
        true,
    )
    .is_err());
    assert!(decode_ack_with_format(
        &make_ack(BACKGROUND_START_REQUEST, RESULT_USER_STOPPED, 5, 1, 7),
        0,
        BACKGROUND_START_REQUEST,
        7,
        true,
    )
    .is_err());
    assert!(decode_ack_with_format(
        &make_ack(BACKGROUND_TRUST_QUERY, RESULT_OK, 5, 0, 7),
        0,
        BACKGROUND_TRUST_QUERY,
        7,
        true,
    )
    .is_err());
}

#[test]
fn session_rejects_user_stopped_ack_without_terminal_cause_capability() {
    use std::net::TcpListener;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let peer = listener.local_addr().unwrap();
    let session_id = 0x0102_0304_0506_0708;
    let control_tx = [0x31; 32];
    let control_rx = [0x42; 32];
    let (stream, server) = {
        let server_keys = (control_tx, control_rx);
        let session_id = session_id;
        let server = thread::spawn(move || {
            let (mut tcp, _) = listener.accept().unwrap();
            let request = read_frame(
                &mut tcp,
                &server_keys.0,
                HOST_TO_RECEIVER_CONTROL,
                session_id,
                &CancellationToken::new(),
            )
            .unwrap();
            assert_eq!(request[5], PLAYBACK_PROGRESS_QUERY);
            let mut payload = [0u8; 13];
            payload[0] = PLAYBACK_PROGRESS_QUERY;
            payload[9] = RESULT_USER_STOPPED;
            payload[10] = 5;
            let ack = encode_control(
                &server_keys.1,
                RECEIVER_TO_HOST_CONTROL,
                session_id,
                0,
                ACK,
                &payload,
            )
            .unwrap();
            write_frame(&mut tcp, &ack, &CancellationToken::new()).unwrap();
        });
        let stream = TcpStream::connect(peer).unwrap();
        (stream, server)
    };
    let mut host = HostSession {
        tcp: Some(stream),
        udp_socket: None,
        sender: None,
        float_sender: None,
        pcm_payload_bytes_sent: Arc::new(AtomicU64::new(0)),
        queue: Arc::new(PcmQueue::for_session(
            PcmFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            false,
        )),
        float_queue: None,
        keys: SessionKeys {
            audio: [0; 32],
            control_tx,
            control_rx,
        },
        parameters: AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        format_v2: None,
        session_id,
        binding_commitment: [0; 32],
        remote_udp: None,
        command_sequence: 0,
        ack_sequence: 0,
        last_command: None,
        last_ack: None,
        last_heartbeat: Instant::now(),
        state: ReceiverState::ReceiverRunning,
        capabilities: Some(ReceiverCapabilities {
            control_major: 3,
            audio_major: 1,
            management_compatible: true,
            audio_compatible: true,
            compatible: true,
            capability_bits: CAP_AUTHENTICATED_STATUS,
            version_code: 1,
        }),
        terminal_cause: None,
        connection_intent: None,
        background_request_id: None,
        background_authorization_required: false,
        background_authorized: false,
        cancellation: CancellationToken::new(),
    };
    let error = host.command(PLAYBACK_PROGRESS_QUERY, &[]).err().unwrap();
    assert!(error.contains("terminal cause"));
    assert_eq!(host.terminal_cause(), None);
    server.join().unwrap();
}

fn host_with_user_stopped_ack(
    capability_bits: u32,
    expected_command: u8,
) -> (HostSession, thread::JoinHandle<()>) {
    use std::net::TcpListener;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let peer = listener.local_addr().unwrap();
    let session_id = 0x0102_0304_0506_0708;
    let control_tx = [0x31; 32];
    let control_rx = [0x42; 32];
    let server = thread::spawn(move || {
        let (mut tcp, _) = listener.accept().unwrap();
        let request = read_frame(
            &mut tcp,
            &control_tx,
            HOST_TO_RECEIVER_CONTROL,
            session_id,
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(request[5], expected_command);
        let mut payload = [0u8; 13];
        payload[0] = expected_command;
        payload[1..9].copy_from_slice(&request[20..28]);
        payload[9] = RESULT_USER_STOPPED;
        payload[10] = 5;
        let ack = encode_control(
            &control_rx,
            RECEIVER_TO_HOST_CONTROL,
            session_id,
            0,
            ACK,
            &payload,
        )
        .unwrap();
        write_frame(&mut tcp, &ack, &CancellationToken::new()).unwrap();
    });
    let stream = TcpStream::connect(peer).unwrap();
    let host = HostSession {
        tcp: Some(stream),
        udp_socket: None,
        sender: None,
        float_sender: None,
        pcm_payload_bytes_sent: Arc::new(AtomicU64::new(0)),
        queue: Arc::new(PcmQueue::for_session(
            PcmFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            false,
        )),
        float_queue: None,
        keys: SessionKeys {
            audio: [0; 32],
            control_tx,
            control_rx,
        },
        parameters: AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        format_v2: None,
        session_id,
        binding_commitment: [0; 32],
        remote_udp: None,
        command_sequence: 0,
        ack_sequence: 0,
        last_command: None,
        last_ack: None,
        last_heartbeat: Instant::now(),
        state: ReceiverState::ReceiverRunning,
        capabilities: Some(ReceiverCapabilities {
            control_major: 3,
            audio_major: 1,
            management_compatible: true,
            audio_compatible: true,
            compatible: true,
            capability_bits,
            version_code: 1,
        }),
        terminal_cause: None,
        connection_intent: None,
        background_request_id: None,
        background_authorization_required: false,
        background_authorized: false,
        cancellation: CancellationToken::new(),
    };
    (host, server)
}

#[test]
fn user_stopped_ack_is_recorded_in_all_command_deadline_paths() {
    let (mut host, server) =
        host_with_user_stopped_ack(CAP_TERMINAL_CAUSE, PLAYBACK_PROGRESS_QUERY);
    let ack = host
        .command_until(
            PLAYBACK_PROGRESS_QUERY,
            &[],
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(ack.result, RESULT_USER_STOPPED);
    assert_eq!(host.state(), ReceiverState::Stopped);
    assert_eq!(host.terminal_cause(), Some(TerminalCause::ReceiverUserStop));
    server.join().unwrap();

    let trust_caps = CAP_BACKGROUND_TRUST_MANAGEMENT | CAP_TERMINAL_CAUSE;
    let (mut host, server) = host_with_user_stopped_ack(trust_caps, BACKGROUND_TRUST_QUERY);
    assert!(host
        .query_background_trust_until(
            Instant::now() + Duration::from_secs(1),
            &CancellationToken::new(),
        )
        .is_err());
    assert_eq!(host.state(), ReceiverState::Stopped);
    assert_eq!(host.terminal_cause(), Some(TerminalCause::ReceiverUserStop));
    server.join().unwrap();

    let (mut host, server) =
        host_with_user_stopped_ack(CAP_BACKGROUND_TRUST_MANAGEMENT, BACKGROUND_TRUST_QUERY);
    let error = host
        .query_background_trust_until(
            Instant::now() + Duration::from_secs(1),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert!(
        error.contains("terminal cause"),
        "unexpected error: {error}"
    );
    assert_eq!(host.terminal_cause(), None);
    server.join().unwrap();
}

#[test]
fn background_trust_ack_checks_shape_echo_fields_and_unsigned_duration() {
    let make_ack =
        |length: u16, commitment: [u8; 32], kind: u8, persistence: u8, remaining: u64| {
            let mut frame = vec![0; CONTROL_HEADER_BYTES + length as usize + CONTROL_TAG_BYTES];
            frame[5] = ACK;
            frame[20..28].copy_from_slice(&0u64.to_be_bytes());
            frame[28..30].copy_from_slice(&length.to_be_bytes());
            frame[30] = BACKGROUND_TRUST_QUERY;
            frame[31..39].copy_from_slice(&0u64.to_be_bytes());
            frame[39] = RESULT_OK;
            frame[40] = 0;
            if length == 55 {
                frame[43..75].copy_from_slice(&commitment);
                frame[75] = kind;
                frame[76] = persistence;
                frame[77..85].copy_from_slice(&remaining.to_be_bytes());
            }
            frame
        };
    let binding = [0x71; 32];
    let decoded = decode_ack(
        &make_ack(55, binding, 2, 1, 28_800_000),
        0,
        BACKGROUND_TRUST_QUERY,
        0,
    )
    .unwrap();
    assert_eq!(
        decoded.background_trust.unwrap(),
        BackgroundTrustSnapshot {
            commitment: binding,
            kind: 2,
            persistence: 1,
            remaining_ms: 28_800_000,
        }
    );
    assert!(decode_ack(
        &make_ack(54, binding, 0, 0, 0),
        0,
        BACKGROUND_TRUST_QUERY,
        0
    )
    .is_err());
    assert!(decode_ack(
        &make_ack(55, binding, 2, 2, 1),
        0,
        BACKGROUND_TRUST_QUERY,
        0
    )
    .is_err());
    assert!(decode_ack(
        &make_ack(55, binding, 3, 2, u64::MAX - 1),
        0,
        BACKGROUND_TRUST_QUERY,
        0
    )
    .is_err());
    assert!(decode_ack(
        &make_ack(55, binding, 1, 1, 30_001),
        0,
        BACKGROUND_TRUST_QUERY,
        0
    )
    .is_err());
    assert!(decode_ack(
        &make_ack(55, binding, 1, 1, 30_000),
        0,
        BACKGROUND_TRUST_QUERY,
        0
    )
    .is_ok());
    assert!(decode_ack(
        &make_ack(55, binding, 1, 1, u64::MAX),
        0,
        BACKGROUND_TRUST_QUERY,
        0
    )
    .is_ok());
}

#[test]
fn playback_progress_wire_metrics_keep_unknown_values_null() {
    let mut payload = vec![0xff; 70];
    payload[..4].copy_from_slice(&48_000u32.to_be_bytes());
    payload[4] = 2;
    payload[45] = 1;
    let progress = decode_playback_progress(&payload).unwrap();
    assert_eq!(progress.receiver_sample_rate, Some(48_000));
    assert_eq!(progress.receiver_channels, Some(2));
    assert_eq!(progress.playback_advancing, Some(true));
    assert_eq!(progress.udp_expired_frames, None);
    assert_eq!(progress.track_pending_frames, None);
    assert!(decode_playback_progress(&payload[..69]).is_err());
    payload[45] = 2;
    assert!(decode_playback_progress(&payload).is_err());
}
use crate::security::{
    test_binding_material, test_binding_material_with_wrong_identity,
    test_binding_material_with_wrong_secret,
};
use std::{
    fs,
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
};

const JAVA_SOURCES: &[&str] = &[
    "PairingProtocol.java",
    "P5cControlProtocol.java",
    "UdpSessionContext.java",
    "UdpPacketDecoder.java",
    "UdpPcmQueue.java",
    "AnpdAudioIngress.java",
    "UdpReceiver.java",
    "P5cDiscovery.java",
    "P5cReceiverCoordinator.java",
    "DiscoveryAuth.java",
];

fn java_paths() -> (PathBuf, PathBuf) {
    let java_home =
        std::env::var_os("JAVA_HOME").expect("JAVA_HOME must point to the approved JDK 17");
    let bin = PathBuf::from(java_home).join("bin");
    #[cfg(windows)]
    let (javac, java) = (bin.join("javac.exe"), bin.join("java.exe"));
    #[cfg(not(windows))]
    let (javac, java) = (bin.join("javac"), bin.join("java"));
    (javac, java)
}

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }
    fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("guarded child")
    }
    fn finish(mut self, stdout: BufReader<std::process::ChildStdout>) -> String {
        finish_endpoint(self.0.take().expect("guarded child"), stdout)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct DripGuard {
    socket: TcpStream,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for DripGuard {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn compile_java_endpoint() -> PathBuf {
    let (javac, _) = java_paths();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let package = repo.join("src/AnPod.Receiver/app/src/main/java/app/anpod/receiver");
    let test_name = std::thread::current()
        .name()
        .unwrap_or("p5c-test")
        .replace(':', "-");
    let output =
        std::env::temp_dir().join(format!("anpod-p5c-java-{}-{test_name}", std::process::id()));
    let _ = fs::remove_dir_all(&output);
    fs::create_dir_all(&output).unwrap();
    let mut command = Command::new(javac);
    command
        .arg("-encoding")
        .arg("UTF-8")
        .arg("-source")
        .arg("17")
        .arg("-target")
        .arg("17")
        .arg("-d")
        .arg(&output);
    for source in JAVA_SOURCES {
        command.arg(package.join(source));
    }
    command.arg(
        repo.join("src/AnPod.AndroidProtocol/src/main/java/com/anpod/server/Pcm16FrameStream.java"),
    );
    command.arg(
        repo.join(
            "src/AnPod.AndroidProtocol/src/main/java/com/anpod/server/FloatPcmFrameQueue.java",
        ),
    );
    command.arg(
        repo.join("src/AnPod.AndroidProtocol/src/main/java/com/anpod/server/Pcm24PackedCodec.java"),
    );
    command.arg(repo.join("tests/P5cLoopbackEndpoint.java"));
    command.arg(repo.join("tests/BuildConfig.java"));
    let result = command
        .output()
        .expect("run JDK 17 javac for the P5-C Java receiver");
    assert!(
        result.status.success(),
        "javac failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    output
}

fn spawn_endpoint(classes: &Path) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    spawn_endpoint_mode(classes, false, false, false, false, false, false)
}

fn spawn_endpoint_without_queue_self_checks(
    classes: &Path,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--skip-queue-self-checks")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    (child, stdout, peer)
}

fn spawn_endpoint_with_auto_recovery_check(
    classes: &Path,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--verify-auto-renew-after-auth")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    (child, stdout, peer)
}

fn spawn_intent_renewal_endpoint(
    classes: &Path,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--verify-intent-renewal")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    (child, stdout, peer)
}

fn spawn_endpoint_mode(
    classes: &Path,
    disconnect_on_stdin: bool,
    fail_resume: bool,
    discovery: bool,
    drop_stop_ack: bool,
    verify_mode_switch: bool,
    verify_audio_window: bool,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    let (_, java) = java_paths();
    let mut command = Command::new(java);
    command
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint");
    if disconnect_on_stdin {
        command.arg("--drop-on-stdin");
    }
    if fail_resume {
        command.arg("--fail-resume");
    }
    if discovery {
        command.arg("--discovery");
    }
    if drop_stop_ack {
        command.arg("--stop-ack-drop");
    }
    if verify_mode_switch {
        command.arg("--verify-mode-switch");
    }
    if verify_mode_switch {
        command.arg("--close-on-stdin");
    }
    if verify_audio_window {
        command.arg("--verify-audio-window");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let port: u16 = line
        .trim()
        .strip_prefix("PORT=")
        .expect("Java P5-C endpoint port")
        .parse()
        .unwrap();
    (
        child,
        stdout,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
    )
}

fn spawn_endpoint_until_disconnect(
    classes: &Path,
    fail_resume: bool,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    spawn_endpoint_mode(classes, true, fail_resume, false, false, false, false)
}

fn spawn_discoverable_endpoint(
    classes: &Path,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    spawn_endpoint_mode(classes, false, false, true, false, false, true)
}

fn spawn_mode_switch_endpoint(
    classes: &Path,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    spawn_endpoint_mode(classes, false, false, true, false, true, false)
}

fn spawn_persistent_endpoint(
    classes: &Path,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    spawn_endpoint_mode(classes, true, false, true, false, false, false)
}

fn spawn_fail_start_endpoint(
    classes: &Path,
    throw: bool,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    let (_, java) = java_paths();
    let mut command = Command::new(java);
    command
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint");
    if throw {
        command.arg("--throw-start-once").arg("--throw-stop-once");
    } else {
        command.arg("--fail-start-once");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let port: u16 = line
        .trim()
        .strip_prefix("PORT=")
        .expect("Java P5-C endpoint port")
        .parse()
        .unwrap();
    (
        child,
        stdout,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
    )
}

fn spawn_recovery_window_endpoint(
    classes: &Path,
    legacy_terminal_cause: bool,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    let (_, java) = java_paths();
    let mut command = Command::new(java);
    command
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--drop-after-ms=450")
        .arg("--close-on-stdin");
    if legacy_terminal_cause {
        command.arg("--capabilities-legacy-terminal-cause");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let port: u16 = line
        .trim()
        .strip_prefix("PORT=")
        .expect("Java P5-C endpoint port")
        .parse()
        .unwrap();
    (
        child,
        stdout,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
    )
}

fn spawn_stop_ack_drop_endpoint(
    classes: &Path,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    spawn_endpoint_mode(classes, false, false, false, true, false, false)
}

fn spawn_capability_endpoint(
    classes: &Path,
    mode: &str,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg(format!("--capabilities-{mode}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut stdout = stdout;
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let port: u16 = line
        .trim()
        .strip_prefix("PORT=")
        .expect("Java capability endpoint port")
        .parse()
        .unwrap();
    (
        child,
        stdout,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
    )
}

fn spawn_audio_v2_upgrade_fixture(
    classes: &Path,
    background_auth_mode: Option<&str>,
    profile: Option<u8>,
    sessions: Option<u8>,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    spawn_audio_v2_upgrade_fixture_with_profiles(
        classes,
        background_auth_mode,
        profile,
        sessions,
        None,
    )
}

fn spawn_audio_v2_upgrade_fixture_with_profiles(
    classes: &Path,
    background_auth_mode: Option<&str>,
    profile: Option<u8>,
    sessions: Option<u8>,
    supported_profiles: Option<&str>,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    let (_, java) = java_paths();
    let mut command = Command::new(java);
    command
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--audio-v2-upgrade");
    if let Some(mode) = background_auth_mode {
        command.arg(format!("--background-auth-mode={mode}"));
    }
    if let Some(profile) = profile {
        command.arg(format!("--audio-v2-profile={profile}"));
    }
    if let Some(sessions) = sessions {
        command.arg(format!("--audio-v2-upgrade-sessions={sessions}"));
    }
    if let Some(profiles) = supported_profiles {
        command.arg(format!("--audio-v2-supported-profiles={profiles}"));
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let port = line
        .trim()
        .strip_prefix("PORT=")
        .expect("Java audio-v2 fixture port")
        .parse::<u16>()
        .unwrap();
    (
        child,
        stdout,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
    )
}

fn finish_endpoint(mut child: Child, mut stdout: BufReader<std::process::ChildStdout>) -> String {
    let mut result = String::new();
    stdout.read_line(&mut result).unwrap();
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    if let Some(mut stderr_pipe) = child.stderr.take() {
        let _ = stderr_pipe.read_to_string(&mut stderr);
    }
    assert!(
        status.success(),
        "Java P5-C test endpoint did not finish successfully: {stderr}"
    );
    result
        .trim()
        .strip_prefix("RESULT=")
        .expect("Java endpoint result")
        .to_string()
}

fn finish_background_trust_endpoint(
    mut child: Child,
    mut stdout: BufReader<std::process::ChildStdout>,
) -> String {
    let mut output = String::new();
    stdout.read_to_string(&mut output).unwrap();
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    if let Some(mut stderr_pipe) = child.stderr.take() {
        let _ = stderr_pipe.read_to_string(&mut stderr);
    }
    assert!(
        status.success(),
        "Java P5-C background trust endpoint did not finish successfully: {stderr}"
    );
    output
}

fn verify_java_v2_discovery(classes: &Path) {
    let host_address = Ipv4Addr::new(127, 0, 0, 2);
    let receiver_address = Ipv4Addr::new(127, 0, 0, 1);
    let interface = crate::network_interface::interface_for_test(host_address, 8);
    let listener =
        crate::discovery::HostOfferListener::start_loopback_for_test(&interface).unwrap();
    let (_, java) = java_paths();
    let output = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--verify-v2-discovery")
        .arg(host_address.to_string())
        .output()
        .expect("run Java V2 discovery loopback check");
    assert!(
        output.status.success(),
        "Java discovery check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("JAVA_V2_DISCOVERY=PASS"),
        "Java discovery check did not complete: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    let offer = loop {
        if let Some(offer) = listener.snapshot().offers.into_iter().next() {
            break offer;
        }
        assert!(
            Instant::now() < deadline,
            "Java CONNECT_OFFER did not reach the Rust host listener"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(offer.endpoint, SocketAddr::from((receiver_address, 45_678)));
    assert!(listener.take_offer(&offer.id).is_some());
    assert!(
        listener.take_offer(&offer.id).is_none(),
        "Java discovery offer remains single-use"
    );
    drop(listener);
    let output = Command::new(java_paths().1)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--verify-coordinator-discovery-recovery")
        .output()
        .expect("run Java discovery recovery check after closing the host listener");
    assert!(
        output.status.success(),
        "Java discovery recovery check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("JAVA_P5C_DISCOVERY_RECOVERY=PASS"));
}

fn rejected_handshake(classes: &Path, wrong_secret: bool) {
    let (child, stdout, peer) = spawn_endpoint(classes);
    let material = if wrong_secret {
        test_binding_material_with_wrong_secret()
    } else {
        test_binding_material_with_wrong_identity()
    };
    assert!(HostSession::authenticate_loopback(
        peer,
        material,
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        }
    )
    .is_err());
    let result = ChildGuard::new(child).finish(stdout);
    assert!(
        result.starts_with("session_failed,0,0,0,0,"),
        "unauthenticated handshake must not stop playback: {result}"
    );
}

#[test]
fn java_receiver_and_rust_sender_share_authenticated_session_and_cleanup() {
    crate::network::verify_capture_queue_bounds();
    let classes = compile_java_endpoint();
    verify_java_v2_discovery(&classes);
    rejected_handshake(&classes, false);
    rejected_handshake(&classes, true);

    let mut synthetic_interface =
        crate::network_interface::interface_for_test(Ipv4Addr::new(192, 168, 41, 12), 24);
    synthetic_interface
        .gateways
        .push(Ipv4Addr::new(192, 168, 41, 1));
    assert!(synthetic_interface.contains(Ipv4Addr::new(192, 168, 41, 200)));
    assert_eq!(
        crate::discovery::probe_targets(&synthetic_interface),
        [
            Ipv4Addr::new(192, 168, 41, 1),
            Ipv4Addr::new(192, 168, 41, 255)
        ]
    );
    assert!(!synthetic_interface.contains(Ipv4Addr::new(192, 168, 42, 1)));

    for send_bad_mac in [true, false] {
        let (child, stdout, peer) = spawn_endpoint(&classes);
        let mut host = HostSession::authenticate_loopback(
            peer,
            test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
        )
        .unwrap();
        if send_bad_mac {
            host.send_bad_mac_for_test().unwrap();
        } else {
            host.send_old_session_for_test().unwrap();
        }
        drop(host);
        let result = finish_endpoint(child, stdout);
        assert!(
            result.starts_with("session_failed,0,0,0,0,"),
            "authenticated but unstarted session must not stop playback: {result}"
        );
    }

    let mut interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    interface.gateways.push(Ipv4Addr::LOCALHOST);
    let unrelated_probe =
        UdpSocket::bind((Ipv4Addr::LOCALHOST, crate::discovery::DISCOVERY_PORT)).unwrap();
    let unrelated = thread::spawn(move || {
        let mut probe = [0; 128];
        let (length, source) = unrelated_probe.recv_from(&mut probe).unwrap();
        assert_eq!(length, 26);
        unrelated_probe
            .send_to(
                &crate::discovery::candidate_for_test([0xA5; 16], 43210),
                source,
            )
            .unwrap();
    });
    let no_candidates = crate::discovery::discover_on_interface(
        &interface,
        &[Ipv4Addr::LOCALHOST],
        None,
        &[],
        Duration::from_millis(120),
        &CancellationToken::new(),
    );
    unrelated.join().unwrap();
    assert!(
        no_candidates.is_err(),
        "a response with an unrelated probe token is not a candidate"
    );

    let flood_responder =
        UdpSocket::bind((Ipv4Addr::LOCALHOST, crate::discovery::DISCOVERY_PORT)).unwrap();
    let flood = thread::spawn(move || {
        let mut probe = [0; 128];
        let (length, target) = flood_responder.recv_from(&mut probe).unwrap();
        assert_eq!(length, 26);
        let token: [u8; 16] = probe[8..24].try_into().unwrap();
        for port in 10_000..10_032 {
            flood_responder
                .send_to(&crate::discovery::candidate_for_test(token, port), target)
                .unwrap();
        }
    });
    let bounded_candidates = crate::discovery::discover_on_interface(
        &interface,
        &[Ipv4Addr::LOCALHOST],
        None,
        &[],
        Duration::from_millis(250),
        &CancellationToken::new(),
    )
    .unwrap();
    flood.join().unwrap();
    assert_eq!(
        bounded_candidates.len(),
        16,
        "discovery caps candidates and stops accepting a response flood"
    );

    let discovery_cancel = CancellationToken::new();
    let cancel_token = discovery_cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(15));
        cancel_token.cancel();
    });
    let start = Instant::now();
    let cancelled_discovery = crate::discovery::discover_on_interface(
        &interface,
        &[Ipv4Addr::LOCALHOST],
        None,
        &[],
        Duration::from_secs(1),
        &discovery_cancel,
    );
    canceller.join().unwrap();
    assert!(
        cancelled_discovery
            .as_ref()
            .is_err_and(|error| error == "Network discovery was cancelled."),
        "cancelled discovery returned {:?}",
        cancelled_discovery.as_ref().err()
    );
    assert!(
        start.elapsed() < Duration::from_millis(250),
        "discovery cancellation must end at the bounded receive poll"
    );

    let (child, stdout, advertised_peer) = spawn_discoverable_endpoint(&classes);
    let malformed_sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    malformed_sender
        .send_to(
            &[0; 26],
            (Ipv4Addr::LOCALHOST, crate::discovery::DISCOVERY_PORT),
        )
        .unwrap();
    let candidates = crate::discovery::discover_on_interface(
        &interface,
        &[Ipv4Addr::LOCALHOST],
        None,
        &[],
        Duration::from_millis(250),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "only a response correlated to the fresh probe is returned"
    );
    let peer = candidates[0].endpoint;
    assert_eq!(
        peer, advertised_peer,
        "discovery provides only the endpoint candidate"
    );
    let mut host = HostSession::connect_on_interface(
        &interface,
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    let capabilities = host
        .capabilities()
        .expect("authenticated capability preflight before BIND");
    assert_eq!(capabilities.control_major, 3);
    assert_eq!(capabilities.audio_major, 1);
    assert_eq!(
        capabilities.capability_bits,
        3 | CAP_PLAYBACK_PROGRESS | CAP_TERMINAL_CAUSE
    );
    assert_eq!(
        capabilities.version_code, 0,
        "localhost fixture versionCode is informational only"
    );
    assert_eq!(host.state(), ReceiverState::TransportConnected);
    host.start_without_format_for_test().unwrap();
    assert_eq!(host.state(), ReceiverState::ReceiverRunning);
    let progress = host
        .playback_progress()
        .unwrap()
        .expect("negotiated playback progress query");
    assert_eq!(
        progress.receiver_sample_rate, None,
        "fixture reports unknown instead of inventing metrics"
    );
    assert_eq!(progress.playback_advancing, None);
    host.resend_last_for_test().unwrap();
    host.send_old_audio_for_test().unwrap();
    thread::sleep(Duration::from_millis(100));
    let samples = vec![0x1234i16; 960 * 2];
    assert!(host
        .pcm_queue()
        .push_raw(samples.as_ptr(), 960, 48_000, 2, 0));
    let mut state = ReceiverState::ReceiverRunning;
    for _ in 0..10 {
        state = host.heartbeat().unwrap();
        if state == ReceiverState::Streaming {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        state,
        ReceiverState::Streaming,
        "streaming follows synthetic PCM sink output"
    );
    host.send_format_packet_for_test(44_100).unwrap();
    host.send_format_packet_for_test(48_000).unwrap();
    host.send_format_packet_for_test(48_000).unwrap();
    assert_eq!(
        host.heartbeat().unwrap(),
        ReceiverState::ReceiverRunning,
        "stale playback activity must return to waiting while authenticated control remains alive"
    );
    host.pause().unwrap();
    assert_eq!(host.state(), ReceiverState::Paused);
    host.resume().unwrap();
    assert_eq!(host.state(), ReceiverState::ReceiverRunning);
    host.stop().unwrap();
    assert!(
        host.pcm_payload_bytes_sent() >= 3_840,
        "sender byte count must survive sender teardown"
    );
    assert_eq!(host.state(), ReceiverState::Stopped);
    drop(host);
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(fields.len(), 8, "unexpected merged P5-C result: {result}");
    assert_eq!(fields[0], "");
    assert_eq!(fields[1..5], ["1", "1", "1", "1"]);
    assert!(
        fields[5].parse::<usize>().unwrap() >= 3_840,
        "synthetic PCM was not drained: {result}"
    );
    assert_eq!(
        fields[6], "true",
        "STOP must release the receiver UDP socket"
    );
    assert_eq!(
        fields[7], "1",
        "authenticated STOP must shut down the receiver listener"
    );

    verify_capability_negotiation(&classes);
    verify_connection_intent(&classes);
    same_coordinator_accepts_a_fresh_authenticated_session(&classes);
    stop_ack_failure_still_shuts_down_listener(&classes);
    stop_disconnect_clears_active_local_session(&classes);
    start_handoff_failure_closes_session(&classes);
    lifecycle_owner_recovers_fresh_session_and_stops(&classes);
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn receive_intent_survives_automatic_search_rotation() {
    let classes = compile_java_endpoint();
    let (child, mut stdout, peer) = spawn_intent_renewal_endpoint(&classes);
    let child = ChildGuard::new(child);
    let mut first_endpoint = SessionEndpoint::loopback(peer);
    first_endpoint.connection_intent = Some(([0x5a; 16], true));
    let mut first = HostSession::connect_endpoint(
        first_endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 1,
        },
        CancellationToken::new(),
    )
    .unwrap();
    first.start().unwrap();
    first.stop().unwrap();
    drop(first);

    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(
        line.trim(),
        "INTENT_RENEWED",
        "fixture must rotate the automatic discovery nonce before PC reauthentication"
    );

    let mut second_endpoint = SessionEndpoint::loopback(peer);
    second_endpoint.connection_intent = Some(([0x5a; 16], true));
    let mut second = HostSession::connect_endpoint(
        second_endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
    )
    .expect("established receive intent must authorize its owner after discovery renewal");
    second.start().unwrap();
    second.stop().unwrap();
    drop(second);
    let result = child.finish(stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(
        fields.len(),
        8,
        "unexpected renewed-intent result: {result}"
    );
    assert_eq!(
        fields[1], "2",
        "mode reauthentication must start a second fresh playback session: {result}"
    );
    assert_eq!(
        fields[4], "2",
        "both synthetic sessions must receive their STOP: {result}"
    );
    assert_eq!(
        fields[7], "0",
        "PC STOP must keep the explicit receive listener alive: {result}"
    );
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn authenticated_control_stays_alive_without_pcm() {
    let classes = compile_java_endpoint();
    let (child, stdout, peer) = spawn_endpoint_with_auto_recovery_check(&classes);
    let mut endpoint = SessionEndpoint::loopback(peer);
    endpoint.connection_intent = Some(([0x5a; 16], true));
    let mut host = HostSession::connect_endpoint(
        endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
    )
    .unwrap();
    host.start().unwrap();
    for _ in 0..7 {
        thread::sleep(Duration::from_secs(1));
        assert_eq!(
            host.heartbeat().unwrap(),
            ReceiverState::ReceiverRunning,
            "idle PCM must not end the authenticated control session"
        );
    }
    assert_eq!(
        host.pcm_payload_bytes_sent(),
        0,
        "idle-session check must not synthesize PCM"
    );
    let samples = vec![0x1234i16; 960 * 2];
    assert!(
        host.pcm_queue()
            .push_raw(samples.as_ptr(), 960, 48_000, 2, 0),
        "fresh synthetic PCM must enter the production sender queue after idle"
    );
    let mut state = ReceiverState::ReceiverRunning;
    for _ in 0..10 {
        state = host.heartbeat().unwrap();
        if state == ReceiverState::Streaming {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        state,
        ReceiverState::Streaming,
        "receiver fixture must consume fresh PCM and report streaming after idle"
    );
    assert!(
        host.pcm_payload_bytes_sent() >= 3_840,
        "idle-to-stream recovery must send the synthetic PCM payload"
    );
    host.stop().unwrap();
    drop(host);
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(
        fields.len(),
        8,
        "unexpected idle-to-stream fixture result: {result}"
    );
    assert!(
        fields[0].contains("AUTO_RENEW_AFTER_AUTH=searching;AUTO_RENEW_AFTER_STOP=stopped|"),
        "post-auth receive intent did not renew from idle or preserve explicit STOP: {result}"
    );
    assert_eq!(
        fields[1..5],
        ["1", "0", "0", "1"],
        "idle-to-stream must start once, then STOP its receiver session: {result}"
    );
    assert!(
        fields[5].parse::<usize>().unwrap() >= 3_840,
        "receiver fixture did not drain synthetic PCM after idle: {result}"
    );
    assert_eq!(
        fields[6], "true",
        "STOP must release the receiver UDP socket: {result}"
    );
    assert_eq!(
        fields[7], "0",
        "PC STOP must preserve the explicit receive listener: {result}"
    );
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn slow_unauthenticated_handshake_has_a_total_deadline() {
    let classes = compile_java_endpoint();
    let (child, stdout, endpoint) = spawn_persistent_endpoint(&classes);
    let mut child = ChildGuard::new(child);
    let mut silent = TcpStream::connect(endpoint).unwrap();
    silent
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    let mut byte = [0; 1];
    assert_eq!(
        silent.read(&mut byte).unwrap(),
        0,
        "no-data peer was not closed at the handshake deadline"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "no-data peer exceeded the total handshake deadline"
    );
    let mut host = HostSession::connect_loopback(
        endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .expect("production listener must accept a legal client after an unauthenticated timeout");
    host.start().unwrap();
    host.stop().unwrap();
    drop(host);
    child
        .child_mut()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"drop")
        .unwrap();
    let _ = child.finish(stdout);

    let (child, stdout, endpoint) = spawn_persistent_endpoint(&classes);
    let mut child = ChildGuard::new(child);
    let first = TcpStream::connect(endpoint).unwrap();
    let slow = TcpStream::connect(endpoint).unwrap();
    let mut drip_socket = slow.try_clone().unwrap();
    let drip = DripGuard {
        socket: slow,
        worker: Some(thread::spawn(move || {
            for _ in 0..85 {
                if drip_socket.write_all(&[0]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
        })),
    };
    thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    let mut host = HostSession::connect_loopback(
        endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .expect("a legal host must displace queued unauthenticated peers");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "slow peers blocked the authenticated host"
    );
    host.start().unwrap();
    host.stop().unwrap();
    drop(host);
    drop(drip);
    let _ = first.shutdown(std::net::Shutdown::Both);
    child
        .child_mut()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"drop")
        .unwrap();
    let _ = child.finish(stdout);
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn simultaneous_authenticated_clients_claim_one_playback_owner() {
    let classes = compile_java_endpoint();
    let (child, stdout, peer) = spawn_persistent_endpoint(&classes);
    let mut child = ChildGuard::new(child);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let successful_starts = Arc::new(AtomicU64::new(0));
    let mut clients = Vec::new();
    for _ in 0..2 {
        let barrier = barrier.clone();
        let successful_starts = successful_starts.clone();
        clients.push(thread::spawn(move || {
            barrier.wait();
            let Ok(mut host) = HostSession::connect_loopback(
                peer,
                test_binding_material(),
                AudioParameters {
                    sample_rate: 48_000,
                    channels: 2,
                    mode: 0,
                },
            ) else {
                return;
            };
            if host.start().is_ok() {
                successful_starts.fetch_add(1, Ordering::AcqRel);
                thread::sleep(Duration::from_millis(100));
                let _ = host.stop();
            }
        }));
    }
    barrier.wait();
    for client in clients {
        client.join().unwrap();
    }
    assert_eq!(
        successful_starts.load(Ordering::Acquire),
        1,
        "parallel authenticated peers must never both claim playback"
    );
    child
        .child_mut()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"drop")
        .unwrap();
    let result = child.finish(stdout);
    assert_eq!(
        result.split(',').nth(1),
        Some("1"),
        "exactly one playback START must reach the endpoint"
    );
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn failed_start_releases_context_for_a_fresh_session() {
    let classes = compile_java_endpoint();
    let (child, stdout, endpoint) = spawn_fail_start_endpoint(&classes, false);
    let child = ChildGuard::new(child);
    let parameters = AudioParameters {
        sample_rate: 48_000,
        channels: 2,
        mode: 0,
    };
    let mut first =
        HostSession::connect_loopback(endpoint, test_binding_material(), parameters).unwrap();
    assert!(
        first.start().is_err(),
        "injected START failure must be reported to the host"
    );
    drop(first);
    let mut retry = HostSession::connect_loopback(endpoint, test_binding_material(), parameters)
        .expect("newly authenticated session must bind and start after failed context cleanup");
    retry.start().unwrap();
    retry.stop().unwrap();
    drop(retry);
    let result = child.finish(stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(
        fields[1], "1",
        "only the retry may start playback: {result}"
    );
    assert_eq!(
        fields[4], "2",
        "failed and retried START paths must both clean up: {result}"
    );
    assert_eq!(
        fields[6], "true",
        "retry STOP must close the UDP context: {result}"
    );
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn thrown_start_and_stop_still_destroy_context_before_retry() {
    let classes = compile_java_endpoint();
    let (child, stdout, endpoint) = spawn_fail_start_endpoint(&classes, true);
    let child = ChildGuard::new(child);
    let parameters = AudioParameters {
        sample_rate: 48_000,
        channels: 2,
        mode: 0,
    };
    let mut first =
        HostSession::connect_loopback(endpoint, test_binding_material(), parameters).unwrap();
    assert!(
        first.start().is_err(),
        "injected START and playback cleanup throws must fail the first session"
    );
    drop(first);
    let mut retry = HostSession::connect_loopback(endpoint, test_binding_material(), parameters)
        .expect("finally must destroy the failed context and preserve listener ownership");
    retry.start().unwrap();
    retry.stop().unwrap();
    drop(retry);
    let result = child.finish(stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(fields[1], "1");
    assert_eq!(
        fields[6], "true",
        "retry STOP must close its UDP socket: {result}"
    );
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn owner_recovery_window_starts_when_a_long_lived_session_disconnects() {
    let classes = compile_java_endpoint();
    let (child, stdout, peer) = spawn_recovery_window_endpoint(&classes, false);
    let mut child = ChildGuard::new(child);
    let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    let timing = SessionTiming {
        heartbeat_interval: Duration::from_millis(20),
        reconnect_delays: [Duration::from_millis(150); 4],
        recovery_window: Duration::from_millis(200),
    };
    let mut owner = SessionOwner::start_with_endpoint(
        SessionEndpoint::on_interface(&interface, peer),
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
        timing,
    )
    .unwrap();
    let initial = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning
    });
    let original_session = initial.session_id.unwrap();
    let recovering = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Reconnecting
            && status.session_id.is_none()
            && status.recovery_seconds_remaining.is_some()
    });
    assert_ne!(recovering.session_id, Some(original_session));
    assert!(recovering.recovery_seconds_remaining.unwrap() > 0,
            "a session healthy longer than the recovery window must receive a fresh window after disconnect");
    let recovered = wait_for_owner(&owner, |status| {
        status.generation > initial.generation
            && status.session_id.is_some()
            && status.state == SessionLifecycleState::ReceiverRunning
    });
    assert_ne!(
        recovered.session_id,
        Some(original_session),
        "recovery must authenticate a fresh session"
    );
    assert_eq!(recovered.recovery_seconds_remaining, None);

    child
        .child_mut()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"close")
        .unwrap();
    let stopped = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Stopped
    });
    let attempts = stopped.attempts;
    thread::sleep(Duration::from_millis(250));
    assert_eq!(
        owner.snapshot().attempts,
        attempts,
        "expired recovery must not restart after stop"
    );
    owner.stop();
    let _ = child.finish(stdout);
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn mode_change_with_unavailable_endpoint_stops_after_recovery_window() {
    let classes = compile_java_endpoint();
    let (child, _stdout, peer) = spawn_mode_switch_endpoint(&classes);
    let mut child = ChildGuard::new(child);
    let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    let timing = SessionTiming {
        heartbeat_interval: Duration::from_millis(20),
        reconnect_delays: [Duration::from_millis(15); 4],
        recovery_window: Duration::from_millis(100),
    };
    let mut owner = SessionOwner::start_with_endpoint(
        SessionEndpoint::on_interface(&interface, peer),
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 1,
        },
        CancellationToken::new(),
        timing,
    )
    .unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning
    });
    child
        .child_mut()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"close")
        .unwrap();
    owner.set_mode(0).unwrap();

    let stopped = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Stopped
    });
    let attempts = stopped.attempts;
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        owner.snapshot().attempts,
        attempts,
        "stopped recovery must not keep authenticating"
    );
    assert!(
        owner.pcm_queue().is_none(),
        "failed recovery must clear the old PCM queue"
    );
    owner.stop();
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn mode_changes_during_reconnect_and_pending_handshake_sync_pcm24_profile() {
    let format = AudioFormatV2::for_profile(3, 0).unwrap();
    let mut profile = HostAudioProfile::Pcm24(format);
    let mut parameters = AudioParameters {
        sample_rate: 96_000,
        channels: 2,
        mode: 0,
    };
    let (commands, receiver) = mpsc::channel();
    commands.send(OwnerCommand::SetMode(1)).unwrap();
    let mut endpoint = SessionEndpoint::loopback(SocketAddr::from((Ipv4Addr::LOCALHOST, 1)));
    let cancellation = CancellationToken::new();
    let enabled = AtomicBool::new(true);
    let mut paused = false;
    assert!(matches!(
        wait_for_reconnect(
            Duration::from_secs(1),
            &receiver,
            &cancellation,
            &enabled,
            &mut endpoint,
            &mut parameters,
            &mut paused,
        ),
        RetryWait::ReconnectNow
    ));
    profile = profile_for_mode(profile, parameters.mode).unwrap();
    let HostAudioProfile::Pcm24(reconnect_format) = profile else {
        panic!("PCM24 profile must be preserved during recovery mode changes");
    };
    assert_eq!(reconnect_format, AudioFormatV2::for_profile(3, 1).unwrap());

    parameters.mode = 1;
    let (commands, receiver) = mpsc::channel();
    commands.send(OwnerCommand::SetMode(0)).unwrap();
    assert!(matches!(
        apply_pending_commands(
            &receiver,
            &mut endpoint,
            &mut parameters,
            &mut paused,
            &enabled,
        ),
        PendingCommands::ModeChanged
    ));
    profile = profile_for_mode(profile, parameters.mode).unwrap();
    let HostAudioProfile::Pcm24(pending_format) = profile else {
        panic!("PCM24 profile must be preserved during pending mode changes");
    };
    assert_eq!(pending_format, AudioFormatV2::for_profile(3, 0).unwrap());
}

#[test]
fn legacy_receiver_disconnect_stops_without_guessing_recovery() {
    let classes = compile_java_endpoint();
    let (child, stdout, peer) = spawn_recovery_window_endpoint(&classes, true);
    let mut child = ChildGuard::new(child);
    let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    let timing = SessionTiming {
        heartbeat_interval: Duration::from_millis(20),
        reconnect_delays: [Duration::from_millis(25); 4],
        recovery_window: Duration::from_secs(2),
    };
    let mut owner = SessionOwner::start_with_endpoint(
        SessionEndpoint::on_interface(&interface, peer),
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
        timing,
    )
    .unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning
    });
    let stopped = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Stopped
    });
    assert_eq!(stopped.attempts, 1, "legacy endpoint must not be retried");
    assert_eq!(
        stopped.terminal_cause,
        Some(TerminalCause::TransportFailure)
    );
    assert!(stopped
        .last_error
        .as_deref()
        .is_some_and(|message| message.contains("terminal-cause capability")));
    owner.stop();
    child
        .child_mut()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"close")
        .unwrap();
    let result = child.finish(stdout);
    assert!(
        result.contains(",1,"),
        "unexpected legacy fixture result: {result}"
    );
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn initial_connection_failure_does_not_retry_in_background() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let peer = listener.local_addr().unwrap();
    drop(listener);
    let timing = SessionTiming {
        heartbeat_interval: Duration::from_millis(20),
        reconnect_delays: [Duration::from_millis(10); 4],
        recovery_window: Duration::from_secs(2),
    };
    let mut owner = SessionOwner::start_with_timing(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
        timing,
    )
    .unwrap();
    let stopped = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Stopped
    });
    thread::sleep(Duration::from_millis(80));
    assert_eq!(stopped.attempts, 1);
    assert_eq!(owner.snapshot().attempts, 1);
    owner.stop();
}

#[test]
fn session_owner_recovers_after_udp_sender_failure_while_playing_or_paused() {
    let classes = compile_java_endpoint();
    let (mut child, stdout, peer) = spawn_persistent_endpoint(&classes);
    let timing = SessionTiming {
        heartbeat_interval: Duration::from_millis(20),
        reconnect_delays: [Duration::from_millis(20); 4],
        recovery_window: Duration::from_secs(3),
    };
    let mut owner = SessionOwner::start_with_timing(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
        timing,
    )
    .unwrap();
    let initial = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning
    });
    let audio = vec![0x3344i16; 960 * 2];
    let first_queue = owner.pcm_queue().unwrap();
    assert!(first_queue.push_raw(audio.as_ptr(), 960, 48_000, 2, 0));
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Streaming
    });
    crate::network::fail_next_udp_send_for_test();
    assert!(first_queue.push_raw(audio.as_ptr(), 960, 48_000, 2, 960));
    let recovered = wait_for_owner(&owner, |status| {
        status.generation > initial.generation
            && status.session_id.is_some()
            && status.state == SessionLifecycleState::ReceiverRunning
    });
    assert_ne!(
        recovered.session_id, initial.session_id,
        "sender failure must create a fresh session"
    );
    let second_queue = owner.pcm_queue().unwrap();
    assert!(!Arc::ptr_eq(&first_queue, &second_queue));
    assert!(second_queue.push_raw(audio.as_ptr(), 960, 48_000, 2, 0));
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Streaming
    });

    owner.set_paused(true).unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Paused
    });
    crate::network::fail_next_udp_send_for_test();
    assert!(second_queue.push_raw(audio.as_ptr(), 960, 48_000, 2, 960));
    let recovered_paused = wait_for_owner(&owner, |status| {
        status.generation > recovered.generation
            && status.session_id.is_some()
            && status.state == SessionLifecycleState::Paused
    });
    assert_ne!(
        recovered_paused.session_id, recovered.session_id,
        "paused sender failure must also reauthenticate and preserve pause"
    );

    owner.stop();
    let stopped = owner.snapshot();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        owner.snapshot().attempts,
        stopped.attempts,
        "sender stop must not revive a closed owner"
    );
    child.stdin.as_mut().unwrap().write_all(b"drop").unwrap();
    let _ = finish_endpoint(child, stdout);
    let _ = fs::remove_dir_all(classes);
}

fn verify_connection_intent(classes: &Path) {
    for mode in [
        "valid",
        "wrong",
        "missing",
        "cancelled",
        "handoff-cancelled",
        "expired",
    ] {
        let mut command = Command::new(java_paths().1);
        command
            .arg("-cp")
            .arg(classes)
            .arg("app.anpod.receiver.P5cLoopbackEndpoint")
            .arg("--connection-intent");
        if mode == "cancelled" {
            command.arg("--cancel-intent-on-stdin");
        }
        if mode == "handoff-cancelled" {
            command.arg("--cancel-before-handoff");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let peer = SocketAddr::from((
            Ipv4Addr::LOCALHOST,
            line.trim()
                .strip_prefix("PORT=")
                .unwrap()
                .parse::<u16>()
                .unwrap(),
        ));
        let mut endpoint = SessionEndpoint::loopback(peer);
        if mode != "missing" {
            endpoint.connection_intent =
                Some(([if mode == "wrong" { 0x59 } else { 0x5a }; 16], true));
        }
        let mut host = HostSession::authenticate_on_network(
            endpoint,
            test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
            CancellationToken::new(),
        )
        .unwrap();
        let result = host.negotiate_capabilities();
        if mode == "wrong" {
            assert!(result.is_err(), "wrong intent must be rejected before BIND");
        } else {
            result.unwrap();
            if mode == "expired" {
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    host.heartbeat().unwrap();
                    thread::sleep(Duration::from_millis(200));
                }
            }
            if mode == "missing" || mode == "expired" {
                assert!(
                    host.bind_audio().is_err(),
                    "missing or expired intent must block BIND"
                );
            } else {
                host.bind_audio().unwrap();
                host.select_mode().unwrap();
                if mode == "cancelled" {
                    child.stdin.as_mut().unwrap().write_all(b"x").unwrap();
                    line.clear();
                    stdout.read_line(&mut line).unwrap();
                    assert_eq!(line.trim(), "INTENT_CANCELLED");
                    assert!(
                        host.start().is_err(),
                        "cancelled intent must block delayed START"
                    );
                } else if mode == "handoff-cancelled" {
                    assert!(
                        host.start().is_err(),
                        "a queued start must recheck cancellation at the playback handoff"
                    );
                } else {
                    host.start().unwrap();
                    host.stop().unwrap();
                }
            }
        }
        drop(host);
        let result = finish_endpoint(child, stdout);
        let fields: Vec<_> = result.split(',').collect();
        assert_eq!(
            fields[1],
            if mode == "valid" { "1" } else { "0" },
            "unexpected playback for {mode}: {result}"
        );
    }
    // An automatic caller must not fall back when an older Receiver lacks the contract.
    let (child, stdout, peer) = spawn_endpoint(classes);
    let mut endpoint = SessionEndpoint::loopback(peer);
    endpoint.connection_intent = Some(([0x5a; 16], true));
    assert!(HostSession::connect_endpoint(
        endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new()
    )
    .is_err());
    assert!(finish_endpoint(child, stdout).starts_with("session_failed,0,0,0,0,"));
}

fn spawn_background_authorization_endpoint(
    classes: &Path,
    mode: &str,
    test_mode: &str,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    spawn_background_authorization_endpoint_with_intent(classes, mode, test_mode, false)
}

fn spawn_background_authorization_endpoint_with_intent(
    classes: &Path,
    mode: &str,
    test_mode: &str,
    require_connection_intent: bool,
) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
    let (_, java) = java_paths();
    let mut command = Command::new(java);
    command
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--background-auth")
        .arg("--verify-background-auth")
        .arg(format!("--background-auth-mode={mode}"));
    if require_connection_intent {
        command.arg("--require-connection-intent");
    }
    if mode == "persistent-stop" {
        command
            .arg("--background-auth-test=persistent-stop")
            .arg("--close-on-stdin");
    }
    if !test_mode.is_empty() {
        command.arg(format!("--background-auth-test={test_mode}"));
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    (child, stdout, peer)
}

fn verify_background_grants_replace_only_the_current_session_intent(classes: &Path) {
    for mode in ["pending-allow", "once", "eight-hours", "permanent"] {
        let (child, stdout, peer) =
            spawn_background_authorization_endpoint_with_intent(classes, mode, "", true);
        let mut endpoint = SessionEndpoint::loopback(peer);
        endpoint.background_request_id = Some([0x6a; 16]);
        endpoint.background_authorization_required = true;
        let mut host = HostSession::connect_endpoint(
            endpoint,
            test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
            CancellationToken::new(),
        )
        .unwrap_or_else(|error| panic!("{mode} grant should authorize no-nonce BIND: {error}"));
        if let Err(error) = host.start() {
            drop(host);
            let result = finish_endpoint(child, stdout);
            panic!("{mode} grant should authorize no-nonce START and handoff: {error}; endpoint={result}");
        }
        host.stop().unwrap();
        drop(host);
        let result = finish_endpoint(child, stdout);
        assert_eq!(
            result.split(',').nth(1),
            Some("1"),
            "{mode} grant must start exactly once: {result}"
        );
    }

    for mode in ["denied", "expired"] {
        let (child, stdout, peer) =
            spawn_background_authorization_endpoint_with_intent(classes, mode, "", true);
        let mut endpoint = SessionEndpoint::loopback(peer);
        endpoint.background_request_id = Some([0x6b; 16]);
        endpoint.background_authorization_required = true;
        assert!(
            HostSession::connect_endpoint(
                endpoint,
                test_binding_material(),
                AudioParameters {
                    sample_rate: 48_000,
                    channels: 2,
                    mode: 0,
                },
                CancellationToken::new()
            )
            .is_err(),
            "{mode} must reject before BIND without a manual nonce"
        );
        let result = finish_endpoint(child, stdout);
        assert_eq!(
            result.split(',').nth(1),
            Some("0"),
            "{mode} must not start playback: {result}"
        );
    }

    let (child, stdout, peer) =
        spawn_background_authorization_endpoint_with_intent(classes, "no-grant", "", true);
    let mut host = HostSession::authenticate_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    let udp_port = host
        .udp_socket
        .as_ref()
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_be_bytes();
    assert_eq!(
        host.command(BIND, &udp_port).unwrap().result,
        RESULT_REJECTED,
        "missing current-session authorization must reject BIND despite no-nonce background mode"
    );
    assert_eq!(
        host.command(START, &[]).unwrap().result,
        RESULT_REJECTED,
        "missing current-session authorization must reject START"
    );
    drop(host);
    let result = finish_endpoint(child, stdout);
    assert_eq!(
        result.split(',').nth(1),
        Some("0"),
        "no grant must not start playback: {result}"
    );
}

fn verify_background_start_authorization(classes: &Path) {
    let (child, stdout, peer) =
        spawn_background_authorization_endpoint(classes, "pending-allow-after-6s", "");
    let mut endpoint = SessionEndpoint::loopback(peer);
    endpoint.background_request_id = Some([0x46; 16]);
    let mut host = HostSession::authenticate_on_network(
        endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    assert_ne!(
        host.capabilities().unwrap().capability_bits & CAP_BACKGROUND_START_AUTH,
        0
    );
    let request_id = endpoint.background_request_id.unwrap();
    assert_eq!(
        host.request_background_authorization(&request_id).unwrap(),
        AUTH_PENDING
    );
    assert_eq!(
        host.request_background_authorization(&request_id).unwrap(),
        AUTH_PENDING,
        "duplicate request ID must remain idempotent"
    );
    let approval_wait_started = Instant::now();
    host.authorize_background_start().unwrap();
    assert!(
        approval_wait_started.elapsed() >= Duration::from_secs(6),
        "Java endpoint did not keep the request pending for six seconds"
    );
    host.bind_audio().unwrap();
    host.select_mode().unwrap();
    host.start().unwrap();
    host.stop().unwrap();
    drop(host);
    let result = finish_endpoint(child, stdout);
    assert!(
        result.contains(",1,"),
        "approved background request must reach START: {result}"
    );

    let (mut child, stdout, peer) =
        spawn_background_authorization_endpoint(classes, "persistent-stop", "");
    for request_id in [[0x62; 16], [0x63; 16]] {
        let mut endpoint = SessionEndpoint::loopback(peer);
        endpoint.background_request_id = Some(request_id);
        let mut host = HostSession::authenticate_on_network(
            endpoint,
            test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
            CancellationToken::new(),
        )
        .unwrap();
        host.negotiate_capabilities().unwrap();
        host.authorize_background_start().unwrap();
        host.bind_audio().unwrap();
        host.select_mode().unwrap();
        host.start().unwrap();
        host.stop().unwrap();
    }
    child.stdin.as_mut().unwrap().write_all(&[1]).unwrap();
    drop(child.stdin.take());
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(
        fields[1], "2",
        "persistent standby must accept a new P5-A session after STOP: {result}"
    );
    assert_eq!(
        fields[4], "2",
        "both authenticated STOPs must clean up playback: {result}"
    );

    let (child, stdout, peer) = spawn_background_authorization_endpoint(classes, "pending", "");
    let cancellation = CancellationToken::new();
    let mut endpoint = SessionEndpoint::loopback(peer);
    endpoint.background_request_id = Some([0x52; 16]);
    let mut host = HostSession::authenticate_on_network(
        endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        cancellation.clone(),
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    let request_id = host.background_request_id.unwrap();
    assert_eq!(
        host.request_background_authorization(&request_id).unwrap(),
        AUTH_PENDING
    );
    let cancel_timer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(750));
        cancellation.cancel();
    });
    let started = Instant::now();
    let error = host
        .authorize_background_start()
        .expect_err("pending authorization must observe cancellation");
    cancel_timer.join().unwrap();
    assert!(
        error.contains("cancelled"),
        "unexpected pending cancellation result: {error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "pending cancellation did not exit promptly"
    );
    drop(host);
    let result = finish_endpoint(child, stdout);
    assert!(
        result.starts_with("session_failed,0,0,0,0,"),
        "cancelled wait must not start playback: {result}"
    );

    let (child, stdout, peer) =
        spawn_background_authorization_endpoint(classes, "pending-slow-status", "");
    let mut endpoint = SessionEndpoint::loopback(peer);
    endpoint.background_request_id = Some([0x53; 16]);
    let mut host = HostSession::authenticate_on_network(
        endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    let request_id = host.background_request_id.unwrap();
    assert_eq!(
        host.request_background_authorization(&request_id).unwrap(),
        AUTH_PENDING
    );
    let started = Instant::now();
    assert!(
        host.query_background_authorization_until(&request_id, started + AUTHORIZATION_WAIT)
            .is_err(),
        "a delayed authorization ACK must not be accepted"
    );
    assert!(
        started.elapsed() >= SESSION_IO_DEADLINE - Duration::from_millis(100)
            && started.elapsed() < SESSION_IO_DEADLINE + Duration::from_secs(2),
        "single authorization I/O should stop at its three-second deadline: {:?}",
        started.elapsed()
    );
    drop(host);
    let _ = finish_endpoint(child, stdout);

    for mode in ["denied", "expired"] {
        let (child, stdout, peer) = spawn_background_authorization_endpoint(classes, mode, "");
        let error = HostSession::connect_loopback(
            peer,
            test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
        )
        .err()
        .expect("denied or expired approval must stop before BIND");
        assert!(
            error.contains(if mode == "denied" {
                "denied"
            } else {
                "expired"
            }),
            "{mode}: {error}"
        );
        let result = finish_endpoint(child, stdout);
        assert!(
            result.starts_with("session_failed,0,0,0,0,"),
            "{mode} must not start playback: {result}"
        );
    }

    let (child, stdout, peer) = spawn_background_authorization_endpoint(classes, "pending", "");
    let mut host = HostSession::authenticate_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    let request_id = [0x51; 16];
    assert_eq!(
        host.request_background_authorization(&request_id).unwrap(),
        AUTH_PENDING
    );
    host.cancel_background_authorization(&request_id).unwrap();
    assert_eq!(
        host.query_background_authorization(&request_id).unwrap(),
        AUTH_CANCELLED
    );
    assert!(
        host.bind_audio().is_err(),
        "cancelled request must not bind UDP"
    );
    drop(host);
    let result = finish_endpoint(child, stdout);
    assert!(
        result.starts_with("session_failed,0,0,0,0,"),
        "cancelled request must not start playback: {result}"
    );

    let (child, stdout, peer) = spawn_background_authorization_endpoint(classes, "pending", "");
    let mut host = HostSession::authenticate_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    let request_id = [0x61; 16];
    let status = host.command(BACKGROUND_START_STATUS, &request_id).unwrap();
    assert_eq!(
        status.result, RESULT_REJECTED,
        "status query must not create an authorization request"
    );
    let malformed = host.command(BACKGROUND_START_REQUEST, &[]).unwrap();
    assert_eq!(
        malformed.result, RESULT_FAILED,
        "malformed request length must be rejected"
    );
    let udp_port = host
        .udp_socket
        .as_ref()
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_be_bytes();
    assert_eq!(
        host.command(BIND, &udp_port).unwrap().result,
        RESULT_REJECTED,
        "an older PC that skips authorization cannot bind in standby"
    );
    assert_eq!(
        host.command(START, &[]).unwrap().result,
        RESULT_REJECTED,
        "an older PC that skips authorization cannot start playback"
    );
    drop(host);
    let result = finish_endpoint(child, stdout);
    assert!(
        result.starts_with("session_failed,0,0,0,0,"),
        "unauthorized START must not start playback: {result}"
    );

    let (child, stdout, peer) =
        spawn_background_authorization_endpoint(classes, "pending", "wrong-id");
    let mut host = HostSession::authenticate_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    assert!(
        host.request_background_authorization(&[0x71; 16]).is_err(),
        "the ACK must echo the exact request ID"
    );
    drop(host);
    let _ = finish_endpoint(child, stdout);

    let (child, stdout, peer) = spawn_endpoint_without_queue_self_checks(&classes);
    let mut endpoint = SessionEndpoint::loopback(peer);
    endpoint.background_authorization_required = true;
    let error = HostSession::connect_endpoint(
        endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
    )
    .err()
    .expect("explicit background authorization must fail closed on an old endpoint");
    assert!(
        error.contains("does not support authenticated background-start authorization"),
        "unexpected fallback: {error}"
    );
    let result = finish_endpoint(child, stdout);
    assert!(
        result.starts_with("session_failed,0,0,0,0,"),
        "old manual receiver must not start for background-only mode: {result}"
    );

    let (child, stdout, peer) = spawn_endpoint_without_queue_self_checks(classes);
    let mut host = HostSession::connect_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .expect("old manually started Receiver flow must remain compatible");
    host.start().unwrap();
    host.stop().unwrap();
    drop(host);
    let result = finish_endpoint(child, stdout);
    assert!(
        result.starts_with(",1,"),
        "old manual mode should still start normally: {result}"
    );
}

fn verify_background_trust_management(classes: &Path) {
    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--background-trust-management")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    let mut host = HostSession::authenticate_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    assert_eq!(host.state(), ReceiverState::Authenticated);
    assert_ne!(
        host.capabilities().unwrap().capability_bits & CAP_BACKGROUND_TRUST_MANAGEMENT,
        0
    );
    let trust = host.query_background_trust().unwrap();
    assert_eq!(
        (trust.kind, trust.persistence, trust.remaining_ms),
        (2, 1, 10_800_000)
    );
    host.revoke_background_trust()
        .expect("effective revoke ACK must arrive before endpoint shutdown");
    drop(host);
    let output = finish_background_trust_endpoint(child, stdout);
    assert!(
        output.contains("TRUST_REVOKED=true,TRUST_CLEARED=true,TRUST_QUERIES=2,TRUST_CANCELS=0"),
        "revoke must clear trust without invoking type 20 request cancellation: {output}"
    );
    let fields: Vec<_> = output
        .lines()
        .find(|line| line.starts_with("RESULT="))
        .unwrap()
        .trim_start_matches("RESULT=")
        .split(',')
        .collect();
    assert_eq!(fields[1], "0", "query/revoke must not start playback");

    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--background-trust-management")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    let mut host = HostSession::authenticate_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    host.bind_audio().unwrap();
    host.select_mode().unwrap();
    host.start().unwrap();
    assert_eq!(host.state(), ReceiverState::ReceiverRunning);
    host.query_background_trust().unwrap();
    assert_eq!(
        host.state(),
        ReceiverState::ReceiverRunning,
        "trust query must preserve active playback state"
    );
    host.pause().unwrap();
    host.query_background_trust().unwrap();
    assert_eq!(
        host.state(),
        ReceiverState::Paused,
        "trust query must preserve pause state"
    );
    host.resume().unwrap();
    host.revoke_background_trust()
        .expect("active revoke ACK must follow synchronous playback cleanup");
    assert_eq!(host.state(), ReceiverState::Stopped);
    drop(host);
    let output = finish_background_trust_endpoint(child, stdout);
    assert!(
        output.contains("TRUST_REVOKED=true,TRUST_CLEARED=true,TRUST_QUERIES=3,TRUST_CANCELS=0"),
        "active revoke must clear trust and avoid type 20 cancellation: {output}"
    );
    let fields: Vec<_> = output
        .lines()
        .find(|line| line.starts_with("RESULT="))
        .unwrap()
        .trim_start_matches("RESULT=")
        .split(',')
        .collect();
    assert_eq!(
        fields[1], "1",
        "active trust query/revoke must not issue a new START"
    );
    assert_eq!(
        fields[4], "1",
        "active revoke must synchronously stop its playback"
    );

    // An endpoint without the explicitly advertised capability receives no management frame.
    let (child, stdout, peer) = spawn_endpoint_without_queue_self_checks(classes);
    let mut host = HostSession::authenticate_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    assert_eq!(
        host.capabilities().unwrap().capability_bits & CAP_BACKGROUND_TRUST_MANAGEMENT,
        0
    );
    assert!(
        host.query_background_trust().is_err(),
        "unsupported management must fail before sending"
    );
    drop(host);
    let output = finish_background_trust_endpoint(child, stdout);
    assert!(
        output.contains("session_failed,0,0,0,0,"),
        "unsupported query should close cleanly: {output}"
    );

    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--background-trust-wrong-binding")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    let mut host = HostSession::authenticate_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.negotiate_capabilities().unwrap();
    assert!(
        host.query_background_trust()
            .unwrap_err()
            .contains("binding did not match"),
        "management ACK must echo the authenticated commitment"
    );
    drop(host);
    let output = finish_background_trust_endpoint(child, stdout);
    assert!(
        output.contains("TRUST_REVOKED=false"),
        "bad binding ACK must fail closed: {output}"
    );
}

fn verify_background_trust_session_owner(classes: &Path) {
    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--background-trust-management")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    let mut owner = SessionOwner::start_on_interface(
        &interface,
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
    )
    .unwrap();
    let active = wait_for_owner(&owner, |snapshot| {
        snapshot.state == SessionLifecycleState::ReceiverRunning && snapshot.session_id.is_some()
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(
        runtime
            .block_on(owner.query_background_trust(active.generation, deadline, cancelled.clone()))
            .is_err(),
        "cancelled queued query must not reach P5-C"
    );
    assert!(
        runtime
            .block_on(owner.revoke_background_trust(active.generation, deadline, cancelled))
            .is_err(),
        "cancelled queued revoke must not reach P5-C"
    );

    let wrong_generation = active.generation + 1;
    assert!(
        runtime
            .block_on(owner.query_background_trust(
                wrong_generation,
                deadline,
                CancellationToken::new()
            ))
            .unwrap_err()
            .contains("代际已变化"),
        "stale target identity must be rejected by the Owner"
    );

    let trust = runtime
        .block_on(owner.query_background_trust(
            active.generation,
            deadline,
            CancellationToken::new(),
        ))
        .unwrap();
    assert_eq!(
        (trust.kind, trust.persistence, trust.remaining_ms),
        (2, 1, 10_800_000)
    );
    let revoked = runtime
        .block_on(owner.revoke_background_trust(
            active.generation,
            deadline,
            CancellationToken::new(),
        ))
        .unwrap();
    assert_eq!(
        (revoked.kind, revoked.persistence, revoked.remaining_ms),
        (0, 0, 0),
        "Owner revoke result must come from the receiver's post-clear ACK"
    );
    owner.stop();

    let output = finish_background_trust_endpoint(child, stdout);
    assert!(output.contains("TRUST_REVOKED=true,TRUST_CLEARED=true,TRUST_QUERIES=2,TRUST_CANCELS=0"),
            "only the current Owner query should reach Receiver and revoke must stop active playback: {output}");
    let fields: Vec<_> = output
        .lines()
        .find(|line| line.starts_with("RESULT="))
        .unwrap()
        .trim_start_matches("RESULT=")
        .split(',')
        .collect();
    assert_eq!(
        fields[1], "1",
        "Owner management must not issue a second START"
    );
    assert_eq!(
        fields[4], "1",
        "Owner revoke must stop its existing playback"
    );
}

fn verify_management_owner_stops_on_receiver_user_stop(classes: &Path) {
    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--background-trust-management")
        .arg("--capabilities-incompatible")
        .arg("--request-user-stop-on-stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    let mut owner = SessionOwner::start_on_interface(
        &interface,
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
    )
    .unwrap();
    let active = wait_for_owner(&owner, |snapshot| {
        snapshot.state == SessionLifecycleState::Authenticated
            && snapshot.session_id.is_some()
            && snapshot
                .capabilities
                .as_ref()
                .is_some_and(|caps| !caps.compatible)
    });
    child.stdin.as_mut().unwrap().write_all(b"stop").unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "USER_STOP_REQUESTED=true");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let error = runtime
        .block_on(owner.query_background_trust(
            active.generation,
            Instant::now() + Duration::from_secs(3),
            CancellationToken::new(),
        ))
        .unwrap_err();
    assert!(
        error.contains("Receiver rejected"),
        "unexpected query result: {error}"
    );

    let stopped = wait_for_owner(&owner, |snapshot| {
        snapshot.state == SessionLifecycleState::Stopped
            && snapshot.terminal_cause == Some(TerminalCause::ReceiverUserStop)
    });
    assert_eq!(stopped.attempts, active.attempts);
    assert_eq!(stopped.last_error, None);
    thread::sleep(Duration::from_millis(100));
    let final_status = owner.snapshot();
    assert_eq!(
        final_status.attempts, active.attempts,
        "user stop must not reconnect"
    );
    assert_eq!(final_status.state, SessionLifecycleState::Stopped);
    assert_eq!(
        final_status.terminal_cause,
        Some(TerminalCause::ReceiverUserStop)
    );
    assert_eq!(
        final_status.last_error, None,
        "user stop must not publish a query error"
    );
    owner.stop();

    let output = finish_background_trust_endpoint(child, stdout);
    assert!(
        output.contains("TRUST_QUERIES=1"),
        "the user-stop ACK must follow the management query: {output}"
    );
}

fn verify_cancelled_after_delayed_management_capability(classes: &Path) {
    let (_, java) = java_paths();
    let mut child = Command::new(java)
        .arg("-cp")
        .arg(classes)
        .arg("app.anpod.receiver.P5cLoopbackEndpoint")
        .arg("--background-trust-management")
        .arg("--delay-background-trust-capability")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        line.trim()
            .strip_prefix("PORT=")
            .unwrap()
            .parse::<u16>()
            .unwrap(),
    ));
    let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    let cancellation = CancellationToken::new();
    let thread_cancellation = cancellation.clone();
    let host_thread = thread::spawn(move || {
        HostSession::authenticate_management_on_interface(
            &interface,
            peer,
            test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
            thread_cancellation,
        )
    });

    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(
        line.trim(),
        "CAPABILITIES_RECEIVED",
        "fixture must be delaying its capability response"
    );
    cancellation.cancel();
    assert!(
        host_thread.join().unwrap().is_err(),
        "cancel during delayed capability negotiation must abort management authentication"
    );

    let output = finish_background_trust_endpoint(child, stdout);
    assert!(
        output.contains("TRUST_REVOKED=false"),
        "cancelled management must not revoke: {output}"
    );
    // The fixture performs one local query after shutdown to report its final state;
    // any higher count would mean the cancelled host sent a management command.
    assert!(
        output.contains("TRUST_QUERIES=1"),
        "no management command may follow cancelled capability negotiation: {output}"
    );
}

#[test]
fn background_start_authorization_java_rust_loopback_contract() {
    let classes = compile_java_endpoint();
    verify_background_start_authorization(&classes);
    verify_background_grants_replace_only_the_current_session_intent(&classes);
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn background_trust_management_java_rust_loopback_contract() {
    let classes = compile_java_endpoint();
    verify_background_trust_management(&classes);
    verify_background_trust_session_owner(&classes);
    verify_management_owner_stops_on_receiver_user_stop(&classes);
    verify_cancelled_after_delayed_management_capability(&classes);
    let _ = fs::remove_dir_all(classes);
}

fn verify_capability_negotiation(classes: &Path) {
    assert_eq!(
        encode_capabilities_query(3, 1, CAP_AUTHENTICATED_STATUS),
        [0, 3, 1, 0, 0, 0, 0, 2]
    );
    for (mode, diagnostic) in [
        ("incompatible", "incompatible"),
        ("missing-capability", "incompatible"),
        ("unsupported", "does not support"),
        ("replay", "sequence"),
    ] {
        let (child, stdout, peer) = spawn_capability_endpoint(classes, mode);
        let error = HostSession::connect_loopback(
            peer,
            test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
        )
        .err()
        .expect("capability mismatch, unsupported query, or replay must block BIND/START");
        assert!(
            error.to_ascii_lowercase().contains(diagnostic),
            "{mode} gave unexpected diagnostic: {error}"
        );
        let result = finish_endpoint(child, stdout);
        assert!(
            result.starts_with("session_failed,0,0,0,0,"),
            "{mode} must not start AudioTrack: {result}"
        );
    }

    for bad_mac in [false, true] {
        let (child, stdout, peer) = spawn_endpoint(classes);
        let mut host = HostSession::authenticate_loopback(
            peer,
            test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
        )
        .unwrap();
        host.send_bad_capabilities_query_for_test(bad_mac).unwrap();
        drop(host);
        let result = finish_endpoint(child, stdout);
        assert!(
            result.starts_with("session_failed,0,0,0,0,"),
            "malformed or unauthenticated capability query must be closed: {result}"
        );
    }

    let (child, stdout, peer) = spawn_capability_endpoint(classes, "incompatible");
    let mut owner = SessionOwner::start(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        CancellationToken::new(),
    )
    .unwrap();
    let authenticated = wait_for_owner(&owner, |snapshot| {
        snapshot.state == SessionLifecycleState::Authenticated && snapshot.capabilities.is_some()
    });
    let capabilities = authenticated.capabilities.unwrap();
    assert!(capabilities.management_compatible);
    assert!(!capabilities.audio_compatible);
    assert!(!capabilities.compatible);
    assert!(
        owner.pcm_queue().is_none(),
        "audio-incompatible management session must never publish a PCM queue"
    );
    thread::sleep(Duration::from_millis(1_100));
    assert_eq!(
        owner.snapshot().state,
        SessionLifecycleState::Authenticated,
        "authenticated management-only session must remain alive through heartbeats"
    );
    owner.stop();
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(
        fields[1], "0",
        "management-only negotiation must not start AudioTrack: {result}"
    );
}

#[test]
fn pcm24_upgrade_probe_fresh_v2_auth_and_control_use_one_receiver_intent() {
    let classes = compile_java_endpoint();
    let (mut child, stdout, peer) =
        spawn_audio_v2_upgrade_fixture(&classes, Some("once"), Some(2), None);
    let mut endpoint = SessionEndpoint::loopback(peer);
    endpoint.connection_intent = Some(([0x5a; 16], true));
    endpoint.background_request_id = Some([0x37; 16]);
    endpoint.background_authorization_required = true;
    let format = AudioFormatV2::for_profile(2, 0).unwrap();
    let queue = Arc::new(FloatPcmQueue::new(false));
    queue.set_expected_format(48_000, 4096).unwrap();
    let mut initial_queue = Some(queue.clone());
    let mut session = HostSession::authenticate_for_profile(
        endpoint,
        &test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        HostAudioProfile::Pcm24(format),
        &mut initial_queue,
        CancellationToken::new(),
    )
    .unwrap();
    assert!(
        initial_queue.is_none(),
        "the authenticated V2 session must own its Float queue"
    );
    assert!(session.is_v2_audio());
    session.negotiate_capabilities().unwrap();
    let capabilities = session.capabilities().unwrap();
    assert!(capabilities.compatible);
    assert_ne!(capabilities.capability_bits & CAP_AUDIO_V2, 0);
    session.authorize_background_start().unwrap();
    session.bind_audio().unwrap();
    session.select_mode().unwrap();
    session.start().unwrap();

    let samples = [
        1.0 / 8_388_608.0,
        -1.0 / 8_388_608.0,
        32_769.0 / 8_388_608.0,
        -32_769.0 / 8_388_608.0,
    ];
    assert!(queue.push_raw(samples.as_ptr(), 2, 48_000, 2, 0));
    let deadline = Instant::now() + Duration::from_secs(2);
    while session.pcm_payload_bytes_sent() < 12 && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(
        session.pcm_payload_bytes_sent() >= 12,
        "Float32 samples did not reach the PCM24 UDP sender"
    );
    let progress = session.playback_progress().unwrap().unwrap();
    assert_eq!(progress.output_format, None);
    assert_eq!(progress.opened_track_channels, None);
    assert_eq!(progress.output_sample_rate, None);
    session.pause().unwrap();
    queue.stop_and_clear();
    let sender_deadline = Instant::now() + Duration::from_secs(2);
    while !session
        .float_sender
        .as_ref()
        .is_some_and(FloatUdpSender::worker_finished)
        && Instant::now() < sender_deadline
    {
        thread::yield_now();
    }
    assert!(session
        .float_sender
        .as_ref()
        .is_some_and(FloatUdpSender::worker_finished));
    assert!(
        session.check_sender().is_ok(),
        "a queue stop after authenticated PAUSE is expected"
    );
    assert!(session
        .float_sender
        .as_ref()
        .is_some_and(FloatUdpSender::worker_joined));
    assert_eq!(session.state(), ReceiverState::Paused);
    assert!(session
        .resume()
        .unwrap_err()
        .contains("reconnect with a fresh queue"));
    assert!(queue.is_stopped());
    session.stop().unwrap();
    drop(session);

    child.stdin.as_mut().unwrap().write_all(&[1]).unwrap();
    drop(child.stdin.take());
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(fields.len(), 7, "unexpected PCM24 upgrade result: {result}");
    assert_eq!(
        fields[0], "",
        "probe/fresh V2 session unexpectedly failed: {result}"
    );
    assert_eq!(fields[1..5], ["1", "1", "1", "1"]);
    assert_eq!(fields[5], "true", "UDP receiver was not closed: {result}");
    assert_eq!(
        fields[6], "true",
        "Java PCM24 decoder did not preserve low-bit samples: {result}"
    );
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn hires96_upgrade_authenticates_profile3_and_sends_q23_without_resampling() {
    let classes = compile_java_endpoint();
    let (mut child, stdout, peer) =
        spawn_audio_v2_upgrade_fixture_with_profiles(&classes, None, Some(3), None, Some("2,3"));
    let mut endpoint = SessionEndpoint::loopback(peer);
    endpoint.connection_intent = Some(([0x5a; 16], true));
    let format = AudioFormatV2::for_profile(3, 0).unwrap();
    let queue = Arc::new(FloatPcmQueue::new(false));
    queue.set_expected_format(96_000, 4096).unwrap();
    let mut initial_queue = Some(queue.clone());
    let mut session = HostSession::authenticate_for_profile(
        endpoint,
        &test_binding_material(),
        AudioParameters {
            sample_rate: 96_000,
            channels: 2,
            mode: 0,
        },
        HostAudioProfile::Pcm24(format),
        &mut initial_queue,
        CancellationToken::new(),
    )
    .unwrap();
    assert!(session.is_v2_audio());
    session.negotiate_capabilities().unwrap();
    assert!(session.capabilities().unwrap().compatible);
    session.bind_audio().unwrap();
    session.select_mode().unwrap();
    session.start().unwrap();

    let samples = [
        1.0 / 8_388_608.0,
        -1.0 / 8_388_608.0,
        32_769.0 / 8_388_608.0,
        -32_769.0 / 8_388_608.0,
    ];
    assert!(queue.push_raw(samples.as_ptr(), 2, 96_000, 2, 0));
    let deadline = Instant::now() + Duration::from_secs(2);
    while session.pcm_payload_bytes_sent() < 12 && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(session.pcm_payload_bytes_sent() >= 12);
    session.pause().unwrap();
    queue.stop_and_clear();
    session.stop().unwrap();
    drop(session);

    child.stdin.as_mut().unwrap().write_all(&[1]).unwrap();
    drop(child.stdin.take());
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(fields.len(), 7, "unexpected PCM24/96 result: {result}");
    assert_eq!(fields[0], "", "96 kHz v2 session failed: {result}");
    assert_eq!(fields[1..5], ["1", "1", "0", "1"]);
    assert_eq!(fields[5], "true");
    assert_eq!(
        fields[6], "true",
        "96 kHz PCM24 Q23 decode failed: {result}"
    );
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn pcm24_owner_pause_and_mode_restart_use_a_fresh_float_queue() {
    let classes = compile_java_endpoint();
    let (mut child, stdout, peer) =
        spawn_audio_v2_upgrade_fixture_with_profiles(&classes, None, Some(3), Some(2), Some("2,3"));
    let mut endpoint = SessionEndpoint::loopback(peer);
    endpoint.connection_intent = Some(([0x5a; 16], true));
    let format = AudioFormatV2::for_profile(3, 0).unwrap();
    let initial_queue = Arc::new(FloatPcmQueue::new(false));
    initial_queue.set_expected_format(96_000, 4096).unwrap();
    let timing = SessionTiming {
        heartbeat_interval: Duration::from_millis(20),
        reconnect_delays: [Duration::from_millis(20); 4],
        recovery_window: Duration::from_secs(5),
    };
    let mut owner = SessionOwner::start_with_endpoint_and_profile(
        endpoint,
        test_binding_material(),
        AudioParameters {
            sample_rate: 96_000,
            channels: 2,
            mode: 0,
        },
        HostAudioProfile::Pcm24(format),
        Some(initial_queue.clone()),
        CancellationToken::new(),
        timing,
    )
    .unwrap();

    let first = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning && status.negotiated_format.is_some()
    });
    assert_eq!(first.negotiated_format.unwrap().profile, 3);
    assert!(owner.matches_audio_quality("hires"));
    assert!(!owner.matches_audio_quality("hifi"));
    let first_queue = match owner.audio_queue().unwrap() {
        SessionAudioQueue::Float32(queue, _) => queue,
        SessionAudioQueue::Pcm16(_) => panic!("PCM24 owner must publish a Float32 queue"),
    };
    assert!(Arc::ptr_eq(&first_queue, &initial_queue));
    let samples = [
        1.0 / 8_388_608.0,
        -1.0 / 8_388_608.0,
        32_769.0 / 8_388_608.0,
        -32_769.0 / 8_388_608.0,
    ];
    assert!(first_queue.push_raw(samples.as_ptr(), 2, 96_000, 2, 0));
    let first_send_deadline = Instant::now() + Duration::from_secs(2);
    while owner.pcm_payload_bytes_sent() < 12 && Instant::now() < first_send_deadline {
        thread::yield_now();
    }
    assert!(
        owner.pcm_payload_bytes_sent() >= 12,
        "first Float session did not send the low-bit PCM24 test vector"
    );

    owner.set_paused(true).unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Paused
    });
    first_queue.stop_and_clear();
    let barrier_deadline = Instant::now() + Duration::from_secs(4);
    let mut sender_joined = false;
    while !sender_joined && Instant::now() < barrier_deadline {
        sender_joined = owner
            .sender_barrier_for_test(Duration::from_millis(250))
            .expect("owner must process the sender barrier while paused");
        thread::yield_now();
    }
    assert!(
        sender_joined,
        "stopped Float sender was not joined by the owner"
    );
    let paused = owner.snapshot();
    assert_eq!(paused.state, SessionLifecycleState::Paused);
    assert_eq!(paused.generation, first.generation);
    assert!(paused.last_error.is_none());
    assert!(first_queue.is_stopped());

    owner.set_mode(1).unwrap();
    let second = wait_for_owner(&owner, |status| {
        status.generation > first.generation
            && status.session_id.is_some()
            && status.state == SessionLifecycleState::Paused
            && status
                .negotiated_format
                .is_some_and(|format| format.mode == 1)
    });
    assert_eq!(second.negotiated_format.unwrap().profile, 3);
    assert_ne!(second.session_id, first.session_id);
    let second_queue = match owner.audio_queue().unwrap() {
        SessionAudioQueue::Float32(queue, _) => queue,
        SessionAudioQueue::Pcm16(_) => panic!("reconnected PCM24 owner must keep Float32"),
    };
    second_queue.set_expected_format(96_000, 4096).unwrap();
    assert!(!Arc::ptr_eq(&first_queue, &second_queue));
    assert!(!second_queue.is_stopped());

    owner.set_paused(false).unwrap();
    let resumed = wait_for_owner(&owner, |status| {
        status.generation == second.generation
            && status.session_id == second.session_id
            && status.state == SessionLifecycleState::ReceiverRunning
    });
    let resumed_format = resumed.negotiated_format.unwrap();
    assert_eq!(resumed_format.mode, 1);
    assert_eq!(resumed_format.profile, 3);
    assert!(second_queue.push_raw(samples.as_ptr(), 2, 96_000, 2, 0));
    let second_send_deadline = Instant::now() + Duration::from_secs(2);
    while owner.pcm_payload_bytes_sent() < 24 && Instant::now() < second_send_deadline {
        thread::yield_now();
    }
    assert!(
        owner.pcm_payload_bytes_sent() >= 24,
        "restarted Float session did not send the low-bit PCM24 test vector"
    );
    owner.set_paused(true).unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Paused
    });
    owner.stop();

    child.stdin.as_mut().unwrap().write_all(&[1]).unwrap();
    drop(child.stdin.take());
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(fields.len(), 7, "unexpected PCM24 owner result: {result}");
    assert_eq!(fields[0], "", "V2 owner restart failed: {result}");
    assert_eq!(fields[1..5], ["2", "3", "1", "2"]);
    assert_eq!(fields[5], "true", "UDP receiver was not closed: {result}");
    assert_eq!(
        fields[6], "true",
        "Java PCM24 decoder lost Q23 samples: {result}"
    );
    let _ = fs::remove_dir_all(classes);
}

#[test]
fn pcm24_upgrade_rejects_receiver_unsupported_v2_descriptor_before_bind() {
    let classes = compile_java_endpoint();
    for profile in [1, 3] {
        let (mut child, stdout, peer) =
            spawn_audio_v2_upgrade_fixture(&classes, None, Some(profile), None);
        let endpoint = SessionEndpoint::loopback(peer);
        let format = AudioFormatV2::for_profile(2, 0).unwrap();
        let mut initial_queue = Some(Arc::new(FloatPcmQueue::new(false)));
        let mut session = HostSession::authenticate_for_profile(
            endpoint,
            &test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
            HostAudioProfile::Pcm24(format),
            &mut initial_queue,
            CancellationToken::new(),
        )
        .unwrap();
        session.negotiate_capabilities().unwrap();
        let capabilities = session.capabilities().unwrap();
        assert!(capabilities.management_compatible);
        assert!(
            !capabilities.compatible,
            "profile {profile} must be rejected"
        );
        assert_eq!(session.state(), ReceiverState::Authenticated);
        assert!(
            session.bind_audio().is_err(),
            "unsupported format must not BIND"
        );
        drop(session);
        child.stdin.as_mut().unwrap().write_all(&[1]).unwrap();
        drop(child.stdin.take());
        let result = finish_endpoint(child, stdout);
        assert!(
            result.contains(",0,0,0,0,"),
            "unsupported format started playback: {result}"
        );
    }
    let _ = fs::remove_dir_all(classes);
}

fn same_coordinator_accepts_a_fresh_authenticated_session(classes: &Path) {
    let (mut child, stdout, peer) = spawn_persistent_endpoint(classes);
    let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    for _ in 0..2 {
        let session = HostSession::connect_on_interface(
            &interface,
            peer,
            test_binding_material(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: 0,
            },
        )
        .unwrap();
        assert_eq!(session.state(), ReceiverState::TransportConnected);
        drop(session);
    }
    child.stdin.as_mut().unwrap().write_all(&[1]).unwrap();
    drop(child.stdin.take());
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(
        fields[1], "0",
        "pre-START reauthentication must not start playback"
    );
}

fn stop_ack_failure_still_shuts_down_listener(classes: &Path) {
    let (child, stdout, peer) = spawn_stop_ack_drop_endpoint(classes);
    let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    let mut session = HostSession::connect_on_interface(
        &interface,
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    session.start().unwrap();
    assert!(
        session.stop().is_err(),
        "test endpoint must drop the authenticated STOP ACK"
    );
    drop(session);
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(
        fields.len(),
        8,
        "unexpected STOP ACK failure result: {result}"
    );
    assert_eq!(
        fields[4], "1",
        "STOP playback must be cleaned before ACK failure"
    );
    assert_eq!(
        fields[7], "1",
        "STOP ACK failure must still close the listener"
    );
}

#[test]
fn mode_change_uses_a_fresh_session_and_preserves_pause() {
    let classes = compile_java_endpoint();
    let (mut child, stdout, peer) = spawn_mode_switch_endpoint(&classes);
    let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    let mut owner = SessionOwner::start_on_interface(
        &interface,
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 1,
        },
        CancellationToken::new(),
    )
    .unwrap();
    let before = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning
    });

    // Two 10 ms synthetic capture blocks follow each other at the production
    // callback cadence; each block is split into two 5 ms UDP packets.
    let old_queue = owner.pcm_queue().unwrap();
    let first_block = vec![0x2345i16; 480 * 2];
    assert!(old_queue.push_raw(first_block.as_ptr(), 480, 48_000, 2, 0));
    thread::sleep(Duration::from_millis(10));
    assert!(old_queue.push_raw(first_block.as_ptr(), 480, 48_000, 2, 480));
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Streaming
    });
    let initial_audio_deadline = Instant::now() + Duration::from_secs(2);
    while owner.pcm_payload_bytes_sent() < 960 * 2 * 2 {
        assert!(
            Instant::now() < initial_audio_deadline,
            "both initial mode-switch capture blocks were not sent"
        );
        thread::sleep(Duration::from_millis(1));
    }
    // Consume the 20 ms of pre-pause audio before SetPaused clears the receiver queue.
    thread::sleep(Duration::from_millis(30));

    owner.set_mode(1).unwrap();
    thread::sleep(Duration::from_millis(30));
    let same_mode = owner.snapshot();
    assert_eq!(
        same_mode.generation, before.generation,
        "reselecting the active playback mode must not rebuild the session"
    );
    assert_eq!(
        same_mode.session_id, before.session_id,
        "reselecting the active playback mode must keep the authenticated session"
    );

    owner.set_paused(true).unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Paused
    });
    owner.set_mode(0).unwrap();
    let after = wait_for_owner(&owner, |status| {
        status.generation > before.generation
            && status.session_id.is_some()
            && status.state == SessionLifecycleState::Paused
    });
    assert_ne!(
        after.session_id, before.session_id,
        "playback mode must use a fresh authenticated session"
    );
    assert!(
        after.paused,
        "the user's pause intent must survive fresh mode authentication"
    );
    assert!(
        !old_queue.push_raw(first_block.as_ptr(), 1, 48_000, 2, 960),
        "mode change must stop and clear the previous PCM queue"
    );

    let mode_queue = owner.pcm_queue().unwrap();
    owner.set_paused(false).unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning
    });
    let samples = vec![0x3456i16; 960 * 2];
    let mut accepted = false;
    for _ in 0..20 {
        if mode_queue.push_raw(samples.as_ptr(), 960, 48_000, 2, 0) {
            accepted = true;
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        accepted,
        "fresh balanced PCM frame zero must enter the new mode queue"
    );
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Streaming
    });
    owner.set_paused(true).unwrap();
    let paused = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Paused
    });

    owner.reconnect().unwrap();
    let resumed = wait_for_owner(&owner, |status| {
        status.generation > paused.generation
            && status.session_id.is_some()
            && status.state == SessionLifecycleState::Paused
    });
    assert_ne!(
        resumed.session_id, paused.session_id,
        "output recovery must use a fresh authenticated session"
    );
    let resumed_queue = owner.pcm_queue().unwrap();
    assert!(
        !Arc::ptr_eq(&mode_queue, &resumed_queue),
        "fresh session must publish a new PCM queue"
    );
    owner.set_paused(false).unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning
    });
    assert!(
        resumed_queue.push_raw(samples.as_ptr(), 960, 48_000, 2, 0),
        "new session audio frame zero must be accepted by its fresh queue"
    );
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Streaming
    });
    owner.stop();
    assert_eq!(
        owner.pcm_payload_bytes_sent(),
        11_520,
        "initial, mode-change, and recovered session PCM payloads must be counted exactly once"
    );
    child.stdin.as_mut().unwrap().write_all(b"close").unwrap();
    let result = finish_endpoint(child, stdout);
    let fields: Vec<_> = result.split(',').collect();
    assert_eq!(
        fields[1], "3",
        "mode change and output recovery must start fresh playback sessions: {result}"
    );
    assert_eq!(fields[5].parse::<usize>().unwrap(), 11_520,
            "initial, mode-change, and recovered frame-zero PCM payloads must reach the fake sink exactly once: {result}");
    assert_eq!(fields[4], "3", "each active session, including mode and recovery rebuilds, must receive one STOP: {result}");
    let _ = fs::remove_dir_all(classes);
}

fn wait_for_owner(
    owner: &SessionOwner,
    predicate: impl Fn(&SessionSnapshot) -> bool,
) -> SessionSnapshot {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let snapshot = owner.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "P5-D owner state timed out: {snapshot:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn lifecycle_owner_recovers_fresh_session_and_stops(classes: &Path) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let peer = listener.local_addr().unwrap();
    let silent_peer = thread::spawn(move || {
        let (_socket, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(300));
    });
    let mut tcp =
        connect_cancellable(Ipv4Addr::LOCALHOST, peer, &CancellationToken::new()).unwrap();
    tcp.set_read_timeout(Some(SESSION_IO_POLL)).unwrap();
    let start = Instant::now();
    let read = tcp.read(&mut [0]);
    assert!(
        is_poll_timeout(&read.unwrap_err()),
        "silent peer read did not wait for its poll timeout"
    );
    assert!(
        start.elapsed() >= Duration::from_millis(25),
        "nonblocking socket spun through WouldBlock immediately"
    );
    let cancellation = CancellationToken::new();
    let cancel_after = cancellation.clone();
    let cancel_thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        cancel_after.cancel();
    });
    let start = Instant::now();
    let cancelled = read_exact_cancellable(&mut tcp, &mut [0], &cancellation);
    assert_eq!(
        cancelled.unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
    assert!(
        start.elapsed() < Duration::from_millis(250),
        "blocking poll did not end promptly after cancellation"
    );
    cancel_thread.join().unwrap();
    drop(tcp);
    silent_peer.join().unwrap();

    let timing = SessionTiming {
        heartbeat_interval: Duration::from_millis(20),
        reconnect_delays: [
            Duration::from_millis(20),
            Duration::from_millis(30),
            Duration::from_millis(40),
            Duration::from_millis(50),
        ],
        recovery_window: Duration::from_secs(5),
    };
    assert_eq!(
        lifecycle_retry_delay(&RECONNECT_DELAYS, 0),
        Duration::from_secs(1)
    );
    assert_eq!(
        lifecycle_retry_delay(&RECONNECT_DELAYS, 1),
        Duration::from_secs(2)
    );
    assert_eq!(
        lifecycle_retry_delay(&RECONNECT_DELAYS, 2),
        Duration::from_secs(5)
    );
    assert_eq!(
        lifecycle_retry_delay(&RECONNECT_DELAYS, 3),
        Duration::from_secs(10)
    );
    assert_eq!(
        lifecycle_retry_delay(&RECONNECT_DELAYS, 8),
        Duration::from_secs(10)
    );

    let loopback_interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
    let (mut first, first_stdout, first_peer) = spawn_endpoint_until_disconnect(classes, true);
    let cancellation = CancellationToken::new();
    let mut owner = SessionOwner::start_on_interface(
        &loopback_interface,
        first_peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        cancellation.clone(),
    )
    .unwrap();
    let initial = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning
    });
    let old_session = initial.session_id.unwrap();
    let old_queue = owner.pcm_queue().unwrap();
    owner.set_paused(true).unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Paused
    });
    owner.set_paused(false).unwrap();
    let failed_resume = wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Reconnecting && status.session_id.is_none()
    });
    assert!(
        failed_resume.paused,
        "failed RESUME must not claim the receiver is running"
    );
    assert_eq!(
        failed_resume.last_error.as_deref(),
        Some("P5-C RESUME was rejected."),
        "Java fake endpoint must return the deterministic failed RESUME ACK"
    );
    first.stdin.as_mut().unwrap().write_all(b"drop").unwrap();
    let first_result = finish_endpoint(first, first_stdout);
    assert!(
        first_result.starts_with("session_failed,1,"),
        "first endpoint did not drop the session: {first_result}"
    );
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Reconnecting && status.session_id.is_none()
    });
    let stale_samples = [1i16, 1];
    assert!(
        !old_queue.push_raw(stale_samples.as_ptr(), 1, 48_000, 2, 0),
        "disconnected generation PCM queue must already be stopped"
    );

    let (second, second_stdout, second_peer) = spawn_endpoint(classes);
    owner
        .update_network_endpoint(&loopback_interface, second_peer)
        .unwrap();
    let recovered = wait_for_owner(&owner, |status| {
        status.session_id.is_some()
            && status.session_id != Some(old_session)
            && status.state == SessionLifecycleState::ReceiverRunning
    });
    assert!(recovered.generation > initial.generation);
    assert!(recovered.attempts > initial.attempts);
    let new_queue = owner.pcm_queue().unwrap();
    assert!(
        !Arc::ptr_eq(&old_queue, &new_queue),
        "recovery must publish a new PCM queue"
    );
    owner.set_paused(false).unwrap();
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::ReceiverRunning
    });
    let samples = vec![0x2345i16; 960 * 2];
    assert!(new_queue.push_raw(samples.as_ptr(), 960, 48_000, 2, 0));
    wait_for_owner(&owner, |status| {
        status.state == SessionLifecycleState::Streaming
    });

    owner.stop();
    let stopped = owner.snapshot();
    assert_eq!(stopped.state, SessionLifecycleState::Stopped);
    assert!(owner.pcm_queue().is_none());
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        owner.snapshot().attempts,
        stopped.attempts,
        "explicit stop must forbid recovery"
    );
    let second_result = finish_endpoint(second, second_stdout);
    let stopped_fields: Vec<_> = second_result.split(',').collect();
    assert_eq!(
        stopped_fields[0], "",
        "owner STOP should reach the receiver: {second_result}"
    );
    assert_eq!(stopped_fields[1], "1");
    assert_eq!(stopped_fields[4], "1");

    let cancel_token = CancellationToken::new();
    let mut cancelled_owner = SessionOwner::start_with_timing(
        first_peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
        cancel_token.clone(),
        timing,
    )
    .unwrap();
    let attempting = wait_for_owner(&cancelled_owner, |status| {
        status.attempts > 0 && status.last_error.is_some()
    });
    cancel_token.cancel();
    cancelled_owner.join();
    let cancelled = cancelled_owner.snapshot();
    assert_eq!(cancelled.state, SessionLifecycleState::Stopped);
    assert_eq!(
        cancelled.attempts, attempting.attempts,
        "caller cancellation must terminate the retry task"
    );
}

fn stop_disconnect_clears_active_local_session(classes: &Path) {
    let (child, stdout, peer) = spawn_endpoint(classes);
    let mut host = HostSession::connect_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.start().unwrap();
    host.remove_control_for_test();
    let stop_error = host
        .stop()
        .expect_err("disconnected STOP must report the control failure");
    assert!(
        stop_error.contains("control channel is closed"),
        "STOP lost its channel diagnostic: {stop_error}"
    );
    assert_eq!(host.state(), ReceiverState::Stopped);
    assert!(host.sender.is_none() && host.udp_socket.is_none() && host.tcp.is_none());
    assert_eq!(host.keys.audio, [0; 32]);
    assert_eq!(host.keys.control_tx, [0; 32]);
    assert_eq!(host.keys.control_rx, [0; 32]);
    let samples = [1i16, 1];
    assert!(!host.pcm_queue().push_raw(samples.as_ptr(), 1, 48_000, 2, 0));
    drop(host);
    let result = finish_endpoint(child, stdout);
    assert!(
        result.starts_with("session_failed,1,0,0,1,"),
        "remote session did not clean up: {result}"
    );
}

fn start_handoff_failure_closes_session(classes: &Path) {
    let (child, stdout, peer) = spawn_endpoint(classes);
    let mut host = HostSession::connect_loopback(
        peer,
        test_binding_material(),
        AudioParameters {
            sample_rate: 48_000,
            channels: 2,
            mode: 0,
        },
    )
    .unwrap();
    host.remove_udp_socket_for_test();
    assert!(
        host.start().is_err(),
        "missing UDP socket must fail local START handoff"
    );
    assert_eq!(host.state(), ReceiverState::Stopped);
    assert!(host.sender.is_none() && host.udp_socket.is_none() && host.tcp.is_none());
    assert_eq!(host.keys.audio, [0; 32]);
    let samples = [1i16, 1];
    assert!(!host.pcm_queue().push_raw(samples.as_ptr(), 1, 48_000, 2, 0));
    drop(host);
    let result = finish_endpoint(child, stdout);
    assert!(
        result.starts_with("session_failed,1,0,0,1,"),
        "failed START handoff did not stop remote playback: {result}"
    );
}
