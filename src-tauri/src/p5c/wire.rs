use super::*;

pub(crate) fn encode_control(
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
    sequence: u64,
    kind: u8,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_CONTROL_FRAME - CONTROL_HEADER_BYTES - CONTROL_TAG_BYTES {
        return Err("P5-C control payload is oversized.".into());
    }
    let mut frame = vec![0; CONTROL_HEADER_BYTES + payload.len() + CONTROL_TAG_BYTES];
    frame[..4].copy_from_slice(CONTROL_MAGIC);
    frame[4] = 1;
    frame[5] = kind;
    frame[8..12].copy_from_slice(&direction.to_be_bytes());
    frame[12..20].copy_from_slice(&session_id.to_be_bytes());
    frame[20..28].copy_from_slice(&sequence.to_be_bytes());
    frame[28..30].copy_from_slice(&(payload.len() as u16).to_be_bytes());
    frame[30..30 + payload.len()].copy_from_slice(payload);
    let tag = control_mac(key, &frame[..CONTROL_HEADER_BYTES + payload.len()])?;
    frame[CONTROL_HEADER_BYTES + payload.len()..].copy_from_slice(&tag);
    Ok(frame)
}

pub(crate) fn encode_capabilities_query(
    control_major: u16,
    audio_major: u8,
    required: u32,
) -> [u8; 8] {
    let mut payload = [0; 8];
    payload[..2].copy_from_slice(&control_major.to_be_bytes());
    payload[2] = audio_major;
    payload[4..].copy_from_slice(&required.to_be_bytes());
    payload
}

pub(crate) fn decode_capabilities_info(
    frame: &[u8],
    response_sequence: u64,
    query_sequence: u64,
) -> Result<ReceiverCapabilities, String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES {
        return Err("Receiver capability response frame was truncated.".into());
    }
    if frame[5] != CAPABILITIES_INFO {
        return Err("Receiver does not support authenticated capability negotiation; use authorized ADB or manual installation.".into());
    }
    if u64::from_be_bytes(frame[20..28].try_into().unwrap()) != response_sequence
        || u16::from_be_bytes([frame[28], frame[29]]) != 20
    {
        return Err("Receiver capability response sequence or length was invalid.".into());
    }
    let payload = &frame[CONTROL_HEADER_BYTES..CONTROL_HEADER_BYTES + 20];
    let echoed_sequence = u64::from_be_bytes(payload[..8].try_into().unwrap());
    let control_major = u16::from_be_bytes(payload[8..10].try_into().unwrap());
    let audio_major = payload[10];
    let result = payload[11];
    let capability_bits = u32::from_be_bytes(payload[12..16].try_into().unwrap());
    let version_code = u32::from_be_bytes(payload[16..20].try_into().unwrap());
    if echoed_sequence != query_sequence || result > 1 {
        return Err("Receiver capability response sequence or result was invalid.".into());
    }
    let management_compatible = control_major == MANAGEMENT_CONTROL_MAJOR
        && capability_bits & REQUIRED_CAPABILITIES == REQUIRED_CAPABILITIES;
    let audio_compatible = audio_major == AUDIO_PROTOCOL_MAJOR;
    let compatible = result == RESULT_OK && management_compatible && audio_compatible;
    Ok(ReceiverCapabilities {
        control_major,
        audio_major,
        management_compatible,
        audio_compatible,
        compatible,
        capability_bits,
        version_code,
    })
}

pub(super) fn write_frame(
    tcp: &mut TcpStream,
    frame: &[u8],
    cancellation: &CancellationToken,
) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES || frame.len() > MAX_CONTROL_FRAME {
        return Err("P5-C control frame length is invalid.".into());
    }
    write_all_cancellable(tcp, &(frame.len() as u16).to_be_bytes(), cancellation)
        .map_err(|_| "P5-C control write failed.".to_string())?;
    write_all_cancellable(tcp, frame, cancellation)
        .map_err(|_| "P5-C control write failed.".to_string())?;
    tcp.flush().map_err(|_| "P5-C control write failed.".into())
}

pub(super) fn write_frame_until(
    tcp: &mut TcpStream,
    frame: &[u8],
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES || frame.len() > MAX_CONTROL_FRAME {
        return Err("P5-C control frame length is invalid.".into());
    }
    write_all_cancellable_until(
        tcp,
        &(frame.len() as u16).to_be_bytes(),
        cancellation,
        deadline,
    )
    .map_err(|_| "P5-C control write failed or approval deadline expired.".to_string())?;
    write_all_cancellable_until(tcp, frame, cancellation, deadline)
        .map_err(|_| "P5-C control write failed or approval deadline expired.".to_string())?;
    tcp.flush().map_err(|_| "P5-C control write failed.".into())
}

pub(super) fn write_frame_until_request(
    tcp: &mut TcpStream,
    frame: &[u8],
    owner_cancellation: &CancellationToken,
    request_cancellation: &CancellationToken,
    deadline: Instant,
    bytes_written: &mut bool,
) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES || frame.len() > MAX_CONTROL_FRAME {
        return Err("P5-C control frame length is invalid.".into());
    }
    write_all_cancellable_until_request(
        tcp,
        &(frame.len() as u16).to_be_bytes(),
        owner_cancellation,
        request_cancellation,
        deadline,
        bytes_written,
    )
    .map_err(|_| "P5-C background trust write failed, expired, or was cancelled.".to_string())?;
    write_all_cancellable_until_request(
        tcp,
        frame,
        owner_cancellation,
        request_cancellation,
        deadline,
        bytes_written,
    )
    .map_err(|_| "P5-C background trust write failed, expired, or was cancelled.".to_string())?;
    check_cancelled(owner_cancellation).map_err(|_| "P5-C session was cancelled.".to_string())?;
    check_cancelled(request_cancellation)
        .map_err(|_| "P5-C background trust request was cancelled.".to_string())?;
    tcp.flush().map_err(|_| "P5-C control write failed.".into())
}

pub(super) fn read_raw_frame(
    tcp: &mut TcpStream,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, String> {
    let mut prefix = [0; 2];
    read_exact_cancellable(tcp, &mut prefix, cancellation)
        .map_err(|_| "P5-C control peer closed.".to_string())?;
    let length = u16::from_be_bytes(prefix) as usize;
    if !(CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES..=MAX_CONTROL_FRAME).contains(&length) {
        return Err("P5-C control frame length is invalid.".into());
    }
    let mut frame = vec![0; length];
    read_exact_cancellable(tcp, &mut frame, cancellation)
        .map_err(|_| "P5-C control peer closed.".to_string())?;
    Ok(frame)
}

pub(super) fn read_frame(
    tcp: &mut TcpStream,
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, String> {
    let frame = read_raw_frame(tcp, cancellation)?;
    verify_control_frame(&frame, key, direction, session_id)?;
    Ok(frame)
}

/// Validates a length-delimited P5-C frame. AOA's ANPU envelope supplies its
/// length; LAN keeps its existing TCP prefix and calls this same validator.
pub(crate) fn verify_control_frame(
    frame: &[u8],
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES
        || frame.len() > MAX_CONTROL_FRAME
        || &frame[..4] != CONTROL_MAGIC
        || frame[4] != 1
        || frame[6..8] != [0, 0]
        || u32::from_be_bytes(frame[8..12].try_into().unwrap()) != direction
        || u64::from_be_bytes(frame[12..20].try_into().unwrap()) != session_id
    {
        return Err("P5-C control identity or header was rejected.".into());
    }
    let payload_len = u16::from_be_bytes([frame[28], frame[29]]) as usize;
    if frame.len() != CONTROL_HEADER_BYTES + payload_len + CONTROL_TAG_BYTES {
        return Err("P5-C control length was rejected.".into());
    }
    let body_len = CONTROL_HEADER_BYTES + payload_len;
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "P5-C HMAC key was invalid.")?;
    mac.update(&frame[..body_len]);
    mac.verify_slice(&frame[body_len..])
        .map_err(|_| "P5-C control authentication failed.".into())
}

pub(super) fn read_frame_until(
    tcp: &mut TcpStream,
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    let mut prefix = [0; 2];
    read_exact_cancellable_until(tcp, &mut prefix, cancellation, deadline)
        .map_err(|_| "P5-C control peer closed or approval deadline expired.".to_string())?;
    let length = u16::from_be_bytes(prefix) as usize;
    if !(CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES..=MAX_CONTROL_FRAME).contains(&length) {
        return Err("P5-C control frame length is invalid.".into());
    }
    let mut frame = vec![0; length];
    read_exact_cancellable_until(tcp, &mut frame, cancellation, deadline)
        .map_err(|_| "P5-C control peer closed or approval deadline expired.".to_string())?;
    verify_control_frame(&frame, key, direction, session_id)?;
    Ok(frame)
}

pub(super) fn read_frame_until_request(
    tcp: &mut TcpStream,
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
    owner_cancellation: &CancellationToken,
    request_cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    let mut prefix = [0; 2];
    read_exact_cancellable_until_request(
        tcp,
        &mut prefix,
        owner_cancellation,
        request_cancellation,
        deadline,
    )
    .map_err(|_| {
        "P5-C background trust peer closed, expired, or request was cancelled.".to_string()
    })?;
    let length = u16::from_be_bytes(prefix) as usize;
    if !(CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES..=MAX_CONTROL_FRAME).contains(&length) {
        return Err("P5-C control frame length is invalid.".into());
    }
    let mut frame = vec![0; length];
    read_exact_cancellable_until_request(
        tcp,
        &mut frame,
        owner_cancellation,
        request_cancellation,
        deadline,
    )
    .map_err(|_| {
        "P5-C background trust peer closed, expired, or request was cancelled.".to_string()
    })?;
    verify_control_frame(&frame, key, direction, session_id)?;
    Ok(frame)
}

pub(super) fn decode_ack(
    frame: &[u8],
    expected_sequence: u64,
    command: u8,
    command_sequence: u64,
) -> Result<Ack, String> {
    decode_ack_with_format_ext(
        frame,
        expected_sequence,
        command,
        command_sequence,
        false,
        false,
    )
}

pub(crate) fn decode_ack_with_format(
    frame: &[u8],
    expected_sequence: u64,
    command: u8,
    command_sequence: u64,
    v2_audio: bool,
) -> Result<Ack, String> {
    decode_ack_with_format_ext(
        frame,
        expected_sequence,
        command,
        command_sequence,
        v2_audio,
        false,
    )
}

pub(crate) fn decode_ack_with_format_ext(
    frame: &[u8],
    expected_sequence: u64,
    command: u8,
    command_sequence: u64,
    v2_audio: bool,
    progress_extension: bool,
) -> Result<Ack, String> {
    decode_ack_with_format_schema(
        frame,
        expected_sequence,
        command,
        command_sequence,
        v2_audio,
        if progress_extension { 2 } else { 0 },
    )
}

pub(crate) fn decode_ack_with_format_schema(
    frame: &[u8],
    expected_sequence: u64,
    command: u8,
    command_sequence: u64,
    v2_audio: bool,
    progress_schema: u8,
) -> Result<Ack, String> {
    let payload_len = u16::from_be_bytes([frame[28], frame[29]]) as usize;
    let terminal_ack = payload_len == 13 && frame[CONTROL_HEADER_BYTES + 9] == RESULT_USER_STOPPED;
    let progress_extension_bytes = match progress_schema {
        0 => 0,
        2 => 126,
        3 => 143,
        4 => 151,
        5 => 199,
        6 => 247,
        _ => return Err("P5-C playback progress schema was unsupported.".into()),
    };
    if frame[5] != ACK
        || u64::from_be_bytes(frame[20..28].try_into().unwrap()) != expected_sequence
        || (!terminal_ack
            && match command {
                PLAYBACK_PROGRESS_QUERY => {
                    payload_len != (if v2_audio { 91 } else { 83 }) + progress_extension_bytes
                }
                BACKGROUND_START_REQUEST | BACKGROUND_START_STATUS | BACKGROUND_START_CANCEL => {
                    payload_len != 30
                }
                BACKGROUND_TRUST_QUERY | BACKGROUND_TRUST_REVOKE => payload_len != 55,
                _ => payload_len != 13,
            })
    {
        return Err("P5-C ACK sequence or type was invalid.".into());
    }
    let payload = &frame[CONTROL_HEADER_BYTES..CONTROL_HEADER_BYTES + payload_len];
    if payload[0] != command
        || u64::from_be_bytes(payload[1..9].try_into().unwrap()) != command_sequence
    {
        return Err("P5-C ACK did not match its command.".into());
    }
    let state = ReceiverState::from_wire(payload[10])?;
    let result = payload[9];
    if result > RESULT_USER_STOPPED {
        return Err("P5-C ACK result code was invalid.".into());
    }
    let udp_port = u16::from_be_bytes([payload[11], payload[12]]);
    if result == RESULT_USER_STOPPED
        && (payload_len != 13 || state != ReceiverState::Stopped || udp_port != 0)
    {
        return Err("P5-C Receiver terminal cause was invalid.".into());
    }
    let playback_progress = if command == PLAYBACK_PROGRESS_QUERY && !terminal_ack {
        Some(decode_playback_progress_with_format(
            &payload[13..],
            v2_audio,
            progress_schema,
        )?)
    } else {
        None
    };
    let background_authorization = if !terminal_ack
        && matches!(
            command,
            BACKGROUND_START_REQUEST | BACKGROUND_START_STATUS | BACKGROUND_START_CANCEL
        ) {
        let request_id: [u8; 16] = payload[13..29].try_into().unwrap();
        let status = payload[29];
        if status > AUTH_CANCELLED {
            return Err("P5-C background authorization status was invalid.".into());
        }
        Some((request_id, status))
    } else {
        None
    };
    let background_trust =
        if !terminal_ack && matches!(command, BACKGROUND_TRUST_QUERY | BACKGROUND_TRUST_REVOKE) {
            let commitment: [u8; 32] = payload[13..45].try_into().unwrap();
            let kind = payload[45];
            let persistence = payload[46];
            let remaining_ms = u64::from_be_bytes(payload[47..55].try_into().unwrap());
            if !valid_background_trust(kind, persistence, remaining_ms) {
                return Err("P5-C background trust fields were invalid.".into());
            }
            Some(BackgroundTrustSnapshot {
                commitment,
                kind,
                persistence,
                remaining_ms,
            })
        } else {
            None
        };
    Ok(Ack {
        result,
        state,
        udp_port,
        playback_progress,
        background_authorization,
        background_trust,
    })
}

pub(super) fn valid_background_trust(kind: u8, persistence: u8, remaining_ms: u64) -> bool {
    match kind {
        0 => persistence == 0 && remaining_ms == 0,
        1 => persistence == 1 && (remaining_ms <= 30_000 || remaining_ms == u64::MAX),
        2 => {
            persistence == 1 && remaining_ms <= Duration::from_secs(8 * 60 * 60).as_millis() as u64
        }
        3 => persistence == 2 && remaining_ms == u64::MAX,
        _ => false,
    }
}

pub(super) fn background_trust_management_state(state: ReceiverState) -> bool {
    matches!(
        state,
        ReceiverState::Authenticated
            | ReceiverState::TransportConnected
            | ReceiverState::ReceiverRunning
            | ReceiverState::Streaming
            | ReceiverState::Paused
    )
}

pub(super) fn decode_playback_progress(data: &[u8]) -> Result<PlaybackDiagnostics, String> {
    decode_playback_progress_with_format(data, false, 0)
}

pub(super) fn decode_playback_progress_with_format(
    data: &[u8],
    v2_audio: bool,
    progress_schema: u8,
) -> Result<PlaybackDiagnostics, String> {
    let base_len = if v2_audio { 78 } else { 70 };
    let extension_len = match progress_schema {
        0 => 0,
        2 => 126,
        3 => 143,
        4 => 151,
        5 => 199,
        6 => 247,
        _ => return Err("Receiver playback progress schema was unsupported.".into()),
    };
    if data.len() != base_len + extension_len {
        return Err("Receiver playback progress payload length was invalid.".into());
    }
    let advancing = match data[45] {
        0 => Some(false),
        1 => Some(true),
        0xff => None,
        _ => return Err("Receiver playback progress flag was invalid.".into()),
    };
    let u32_at = |offset| u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
    let u64_at = |offset| u64::from_be_bytes(data[offset..offset + 8].try_into().unwrap());
    let metric_u32 = |offset| {
        let value = u32_at(offset);
        (value != u32::MAX).then_some(value)
    };
    let metric_u64 = |offset| {
        let value = u64_at(offset);
        (value != u64::MAX).then_some(value)
    };
    let mut diagnostics = PlaybackDiagnostics {
        receiver_sample_rate: metric_u32(0),
        receiver_channels: (data[4] != 0xff).then_some(data[4]),
        udp_jitter_target_frames: metric_u32(5),
        udp_expired_frames: metric_u64(9),
        udp_queued_frames: metric_u64(17),
        track_pending_frames: metric_u64(25),
        underrun_count: metric_u32(33),
        tcp_dropped_backlog_frames: metric_u64(37),
        playback_advancing: advancing,
        requested_buffer_frames: metric_u32(46),
        accepted_buffer_frames: metric_u32(50),
        effective_buffer_frames: metric_u32(54),
        capacity_frames: metric_u32(58),
        start_threshold_frames: metric_u32(62),
        pre_roll_frames: metric_u32(66),
        ..PlaybackDiagnostics::default()
    };
    if v2_audio {
        if data[70] != 1 || data[73] != 0 || data[71] > 3 || !matches!(data[72], 0..=2) {
            return Err("Receiver v2 playback output format was invalid.".into());
        }
        let output_sample_rate = u32_at(74);
        if data[71] == 0 && (data[72] != 0 || output_sample_rate != 0) {
            return Err("Receiver unknown v2 output format had populated track fields.".into());
        }
        diagnostics.output_format = (data[71] != 0).then_some(data[71]);
        diagnostics.opened_track_channels = (data[72] != 0).then_some(data[72]);
        diagnostics.output_sample_rate = (output_sample_rate != 0).then_some(output_sample_rate);
    }
    if progress_schema != 0 {
        let extension = &data[base_len..];
        if extension[0] != progress_schema || extension[21] > 2 {
            return Err("Receiver extended playback progress schema or engine was invalid.".into());
        }
        let u32_at_ext =
            |offset| u32::from_be_bytes(extension[offset..offset + 4].try_into().unwrap());
        let u64_at_ext =
            |offset| u64::from_be_bytes(extension[offset..offset + 8].try_into().unwrap());
        let metric_u32 = |offset| {
            let value = u32_at_ext(offset);
            (value != u32::MAX).then_some(value)
        };
        let metric_u64 = |offset| {
            let value = u64_at_ext(offset);
            (value != u64::MAX).then_some(value)
        };
        diagnostics.progress_schema = Some(progress_schema);
        diagnostics.progress_monotonic_nanos = metric_u64(1);
        diagnostics.receiver_generation = metric_u64(9);
        diagnostics.output_epoch = metric_u32(17);
        diagnostics.output_engine = Some(extension[21]);
        diagnostics.ingress_audio_frames = metric_u64(22);
        diagnostics.queue_dropped_frames = metric_u64(30);
        diagnostics.queue_expired_frames = metric_u64(38);
        diagnostics.queue_concealed_frames = metric_u64(46);
        diagnostics.drift_inserted_frames = metric_u64(54);
        diagnostics.drift_deleted_frames = metric_u64(62);
        diagnostics.drift_reset_count = metric_u64(70);
        diagnostics.output_accepted_frames = metric_u64(78);
        diagnostics.output_consumed_frames = metric_u64(86);
        diagnostics.oboe_ring_consumed_frames = metric_u64(94);
        diagnostics.oboe_callback_requested_frames = metric_u64(102);
        diagnostics.oboe_zero_filled_frames = metric_u64(110);
        diagnostics.ring_pending_frames = metric_u32(118);
        diagnostics.stream_pending_frames = metric_u32(122);
        if progress_schema >= 3 {
            let metric_i32 = |offset| {
                let value = i32::from_be_bytes(extension[offset..offset + 4].try_into().unwrap());
                (value != i32::MIN).then_some(value)
            };
            diagnostics.drift_target_frames = metric_u32(126);
            diagnostics.drift_observed_frames = metric_u32(130);
            diagnostics.drift_error_frames = metric_i32(134);
            diagnostics.drift_correction_ppm = metric_i32(138);
            diagnostics.drift_reset_reason = (extension[142] != 0).then_some(extension[142]);
            if progress_schema >= 4 {
                diagnostics.applied_loss_ledger_revision = metric_u64(143);
            }
            if progress_schema >= 5 {
                diagnostics.applied_loss_ledger_revision = metric_u64(143);
                diagnostics.loss_10s_frames = metric_u64(151);
                diagnostics.eligible_10s_frames = metric_u64(159);
                diagnostics.loss_60s_frames = metric_u64(167);
                diagnostics.eligible_60s_frames = metric_u64(175);
                diagnostics.cumulative_loss_frames = metric_u64(183);
                diagnostics.cumulative_eligible_frames = metric_u64(191);
                for (loss, eligible) in [
                    (diagnostics.loss_10s_frames, diagnostics.eligible_10s_frames),
                    (diagnostics.loss_60s_frames, diagnostics.eligible_60s_frames),
                    (
                        diagnostics.cumulative_loss_frames,
                        diagnostics.cumulative_eligible_frames,
                    ),
                ] {
                    if loss.is_some() != eligible.is_some()
                        || loss.zip(eligible).is_some_and(|(lost, total)| lost > total)
                    {
                        return Err("Receiver loss diagnostics fields were invalid.".into());
                    }
                }
            }
            if progress_schema == 6 {
                diagnostics.pc_drop_frames = metric_u64(199);
                diagnostics.receiver_drop_frames = metric_u64(207);
                diagnostics.transport_missing_frames = metric_u64(215);
                diagnostics.output_drop_frames = metric_u64(223);
                diagnostics.recovery_count = metric_u64(231);
                diagnostics.recovery_duration_ms = metric_u64(239);
            }
        }
    }
    Ok(diagnostics)
}

pub(crate) fn hex_string(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0xf) as usize] as char);
    }
    value
}

pub(super) fn control_mac(key: &[u8; 32], bytes: &[u8]) -> Result<[u8; 32], String> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "P5-C HMAC key was invalid.")?;
    mac.update(bytes);
    Ok(mac.finalize().into_bytes().into())
}

pub(super) fn connect_cancellable(
    local_address: Ipv4Addr,
    peer: SocketAddr,
    cancellation: &CancellationToken,
) -> std::io::Result<TcpStream> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let cancellation = cancellation.clone();
    runtime.block_on(async move {
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.bind(SocketAddr::from((local_address, 0)))?;
        tokio::select! {
            _ = cancellation.cancelled() => Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted, "P5-C connection cancelled")),
            result = tokio::time::timeout(CONNECT_TIMEOUT, socket.connect(peer)) => {
                match result {
                    Ok(result) => {
                        let tcp = result?.into_std()?;
                        tcp.set_nonblocking(false)?;
                        Ok(tcp)
                    }
                    Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "P5-C connect deadline expired")),
                }
            },
        }
    })
}

pub(super) fn check_cancelled(cancellation: &CancellationToken) -> std::io::Result<()> {
    if cancellation.is_cancelled() {
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "P5-C operation cancelled",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn is_poll_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

pub(super) fn read_exact_cancellable(
    tcp: &mut TcpStream,
    bytes: &mut [u8],
    cancellation: &CancellationToken,
) -> std::io::Result<()> {
    read_exact_cancellable_until(
        tcp,
        bytes,
        cancellation,
        Instant::now() + SESSION_IO_DEADLINE,
    )
}

pub(super) fn read_exact_cancellable_until(
    tcp: &mut TcpStream,
    bytes: &mut [u8],
    cancellation: &CancellationToken,
    deadline: Instant,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(cancellation)?;
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        match tcp.read(&mut bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => offset += read,
            Err(error) if is_poll_timeout(&error) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    check_cancelled(cancellation)
}

pub(super) fn read_exact_cancellable_until_request(
    tcp: &mut TcpStream,
    bytes: &mut [u8],
    owner_cancellation: &CancellationToken,
    request_cancellation: &CancellationToken,
    deadline: Instant,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(owner_cancellation)?;
        check_cancelled(request_cancellation)?;
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        match tcp.read(&mut bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => offset += read,
            Err(error) if is_poll_timeout(&error) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(super) fn write_all_cancellable(
    tcp: &mut TcpStream,
    bytes: &[u8],
    cancellation: &CancellationToken,
) -> std::io::Result<()> {
    write_all_cancellable_until(
        tcp,
        bytes,
        cancellation,
        Instant::now() + SESSION_IO_DEADLINE,
    )
}

pub(super) fn write_all_cancellable_until(
    tcp: &mut TcpStream,
    bytes: &[u8],
    cancellation: &CancellationToken,
    deadline: Instant,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(cancellation)?;
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        match tcp.write(&bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(written) => offset += written,
            Err(error) if is_poll_timeout(&error) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    check_cancelled(cancellation)
}

pub(super) fn write_all_cancellable_until_request(
    tcp: &mut TcpStream,
    bytes: &[u8],
    owner_cancellation: &CancellationToken,
    request_cancellation: &CancellationToken,
    deadline: Instant,
    bytes_written: &mut bool,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(owner_cancellation)?;
        check_cancelled(request_cancellation)?;
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        match tcp.write(&bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(written) => {
                offset += written;
                *bytes_written = true;
            }
            Err(error) if is_poll_timeout(&error) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
