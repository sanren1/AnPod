use super::*;
use crate::{
    network::PcmQueue,
    p5c::{
        self, ReceiverState, TerminalCause, ACK, CAPABILITIES_INFO, CAP_AUDIO_FLOAT32,
        CAP_AUDIO_V2, CAP_AUTHENTICATED_STATUS, CAP_PLAYBACK_PROGRESS, CAP_PROGRESS_EXT,
        CAP_TERMINAL_CAUSE, PLAYBACK_PROGRESS_QUERY, RESULT_USER_STOPPED,
    },
    protocol::PacketDecoder,
    security::{test_binding_material, ReceiverHandshakeV2},
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver as ChannelReceiver, Sender},
    },
    time::Duration,
};

struct FakeReader {
    rx: ChannelReceiver<Record>,
    cancelled: Arc<AtomicBool>,
}
impl RecordReader for FakeReader {
    fn read_record(&mut self) -> io::Result<Record> {
        loop {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "cancelled",
                ));
            }
            match self.rx.recv_timeout(Duration::from_millis(5)) {
                Ok(record) => return Ok(record),
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed"))
                }
            }
        }
    }
}

struct FakeWriter {
    tx: Sender<Record>,
}
impl RecordWriter for FakeWriter {
    fn write_record(&mut self, record: &Record) -> io::Result<()> {
        self.tx
            .send(Record {
                kind: record.kind,
                body: record.body.clone(),
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }
}

struct DelayedFakeWriter {
    tx: Sender<Record>,
    audio_records: usize,
}
impl RecordWriter for DelayedFakeWriter {
    fn write_record(&mut self, record: &Record) -> io::Result<()> {
        self.tx
            .send(Record {
                kind: record.kind,
                body: record.body.clone(),
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "closed"))?;
        if record.kind == RecordType::Audio {
            self.audio_records += 1;
            // FORMAT is #1, the first PCM packet is #2. Stall after that
            // packet so the remainder of the popped block exceeds its queue TTL.
            if self.audio_records == 2 {
                std::thread::sleep(Duration::from_millis(60));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct FakeCancel(Arc<AtomicBool>);
impl CancelIo for FakeCancel {
    fn cancel_io(&self) {
        self.0.store(true, Ordering::Release);
    }
}

fn send_record(tx: &Sender<Record>, kind: RecordType, body: Vec<u8>) {
    tx.send(Record::new(kind, body).unwrap()).unwrap();
}

fn read_record(rx: &ChannelReceiver<Record>, kind: RecordType) -> Record {
    let record = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("timed out waiting for ANPU record");
    assert_eq!(record.kind, kind);
    record
}

fn retry_until_accepted(mut push: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !push() {
        assert!(
            std::time::Instant::now() < deadline,
            "audio queue stayed contended"
        );
        std::thread::yield_now();
    }
}

fn send_capabilities(tx: &Sender<Record>, keys: &SessionKeys, session_id: u64, request: &Record) {
    send_capabilities_with_bits(
        tx,
        keys,
        session_id,
        request,
        CAP_AUTHENTICATED_STATUS | CAP_AUDIO_V2,
    );
}

fn send_capabilities_with_bits(
    tx: &Sender<Record>,
    keys: &SessionKeys,
    session_id: u64,
    request: &Record,
    capability_bits: u32,
) {
    p5c::verify_control_frame(
        &request.body,
        &keys.host_to_receiver_control,
        HOST_TO_RECEIVER_CONTROL,
        session_id,
    )
    .unwrap();
    assert_eq!(request.body[5], CAPABILITIES_QUERY);
    let mut payload = [0u8; 20];
    payload[..8].copy_from_slice(&0u64.to_be_bytes());
    payload[8..10].copy_from_slice(&MANAGEMENT_CONTROL_MAJOR.to_be_bytes());
    payload[10] = AUDIO_PROTOCOL_MAJOR;
    payload[11] = 0;
    payload[12..16].copy_from_slice(&capability_bits.to_be_bytes());
    payload[16..20].copy_from_slice(&1u32.to_be_bytes());
    let response = p5c::encode_control(
        &keys.receiver_to_host_control,
        RECEIVER_TO_HOST_CONTROL,
        session_id,
        0,
        CAPABILITIES_INFO,
        &payload,
    )
    .unwrap();
    send_record(tx, RecordType::Control, response);
}

fn send_user_stopped_ack(
    tx: &Sender<Record>,
    keys: &SessionKeys,
    session_id: u64,
    request: &Record,
) {
    p5c::verify_control_frame(
        &request.body,
        &keys.host_to_receiver_control,
        HOST_TO_RECEIVER_CONTROL,
        session_id,
    )
    .unwrap();
    let command_sequence = u64::from_be_bytes(request.body[20..28].try_into().unwrap());
    let mut payload = [0u8; 13];
    payload[0] = request.body[5];
    payload[1..9].copy_from_slice(&command_sequence.to_be_bytes());
    payload[9] = RESULT_USER_STOPPED;
    payload[10] = 5;
    let ack = p5c::encode_control(
        &keys.receiver_to_host_control,
        RECEIVER_TO_HOST_CONTROL,
        session_id,
        4,
        ACK,
        &payload,
    )
    .unwrap();
    send_record(tx, RecordType::Control, ack);
}

fn send_ack(
    tx: &Sender<Record>,
    keys: &SessionKeys,
    session_id: u64,
    ack_sequence: u64,
    request: &Record,
    state: ReceiverState,
    udp_port: u16,
) {
    p5c::verify_control_frame(
        &request.body,
        &keys.host_to_receiver_control,
        HOST_TO_RECEIVER_CONTROL,
        session_id,
    )
    .unwrap();
    let command = request.body[5];
    let command_sequence = u64::from_be_bytes(request.body[20..28].try_into().unwrap());
    let mut payload = [0u8; 13];
    payload[0] = command;
    payload[1..9].copy_from_slice(&command_sequence.to_be_bytes());
    payload[9] = RESULT_OK;
    payload[10] = match state {
        ReceiverState::Authenticated => 0,
        ReceiverState::TransportConnected => 1,
        ReceiverState::ReceiverRunning => 2,
        ReceiverState::Streaming => 3,
        ReceiverState::Paused => 4,
        ReceiverState::Stopped => 5,
    };
    payload[11..13].copy_from_slice(&udp_port.to_be_bytes());
    let ack = p5c::encode_control(
        &keys.receiver_to_host_control,
        RECEIVER_TO_HOST_CONTROL,
        session_id,
        ack_sequence,
        ACK,
        &payload,
    )
    .unwrap();
    send_record(tx, RecordType::Control, ack);
}

fn send_progress_extension_ack(
    tx: &Sender<Record>,
    keys: &SessionKeys,
    session_id: u64,
    ack_sequence: u64,
    request: &Record,
    format: AudioFormatV2,
) {
    p5c::verify_control_frame(
        &request.body,
        &keys.host_to_receiver_control,
        HOST_TO_RECEIVER_CONTROL,
        session_id,
    )
    .unwrap();
    assert_eq!(request.body[5], PLAYBACK_PROGRESS_QUERY);
    assert_eq!(u16::from_be_bytes([request.body[28], request.body[29]]), 1);
    assert_eq!(
        request.body[30], 2,
        "host did not explicitly request progress schema 2"
    );
    let command_sequence = u64::from_be_bytes(request.body[20..28].try_into().unwrap());
    let mut payload = vec![0xff; 13 + 78 + 126];
    payload[0] = PLAYBACK_PROGRESS_QUERY;
    payload[1..9].copy_from_slice(&command_sequence.to_be_bytes());
    payload[9] = 0;
    payload[10] = 2;
    let progress = &mut payload[13..13 + 78 + 126];
    progress[45] = 1;
    progress[70] = 1;
    progress[71] = format.sample_format;
    progress[72] = 2;
    progress[73] = 0;
    progress[74..78].copy_from_slice(&format.sample_rate.to_be_bytes());
    let extension = &mut progress[78..];
    extension[0] = 2;
    extension[1..9].copy_from_slice(&1234u64.to_be_bytes());
    extension[9..17].copy_from_slice(&77u64.to_be_bytes());
    extension[17..21].copy_from_slice(&4u32.to_be_bytes());
    extension[21] = 2;
    extension[22..30].copy_from_slice(&480u64.to_be_bytes());
    extension[30..38].copy_from_slice(&3u64.to_be_bytes());
    let ack = p5c::encode_control(
        &keys.receiver_to_host_control,
        RECEIVER_TO_HOST_CONTROL,
        session_id,
        ack_sequence,
        ACK,
        &payload,
    )
    .unwrap();
    send_record(tx, RecordType::Control, ack);
}

fn run_fake_receiver(
    rx: ChannelReceiver<Record>,
    tx: Sender<Record>,
    material: crate::security::BindingMaterial,
    format: AudioFormatV2,
) -> (u8, Vec<u8>, u64) {
    let hello = read_record(&rx, RecordType::Handshake);
    let (await_finish, challenge) =
        ReceiverHandshakeV2::accept_hello(material.derive_aoa_transport(), &hello.body).unwrap();
    let session_id = u64::from_be_bytes(hello.body[72..80].try_into().unwrap());
    send_record(&tx, RecordType::Handshake, challenge);
    let finish = read_record(&rx, RecordType::Handshake);
    let (ready, authenticated) = await_finish.accept_finish(&finish.body).unwrap();
    send_record(&tx, RecordType::Handshake, ready);
    let keys = authenticated.into_keys();

    let caps = read_record(&rx, RecordType::Control);
    let audio_caps = CAP_AUTHENTICATED_STATUS
        | CAP_AUDIO_V2
        | CAP_PLAYBACK_PROGRESS
        | CAP_PROGRESS_EXT
        | if format.profile >= 4 {
            CAP_AUDIO_FLOAT32
        } else {
            0
        };
    send_capabilities_with_bits(&tx, &keys, session_id, &caps, audio_caps);
    let bind = read_record(&rx, RecordType::Control);
    assert_eq!(bind.body[5], BIND);
    assert_eq!(&bind.body[30..32], &[0, 0]);
    send_ack(
        &tx,
        &keys,
        session_id,
        1,
        &bind,
        ReceiverState::TransportConnected,
        0,
    );
    let select = read_record(&rx, RecordType::Control);
    assert_eq!(select.body[5], SELECT_MODE);
    send_ack(
        &tx,
        &keys,
        session_id,
        2,
        &select,
        ReceiverState::TransportConnected,
        0,
    );
    let start = read_record(&rx, RecordType::Control);
    assert_eq!(start.body[5], START);
    send_ack(
        &tx,
        &keys,
        session_id,
        3,
        &start,
        ReceiverState::ReceiverRunning,
        0,
    );

    let mut decoder = PacketDecoder::new_v2(
        &keys.host_to_receiver_audio,
        session_id,
        HOST_TO_RECEIVER,
        format,
    )
    .unwrap();
    let format_record = read_record(&rx, RecordType::Audio);
    let format_packet = decoder.open(&format_record.body).unwrap();
    assert_eq!(format_packet.header.packet_type, PacketType::Format);
    assert_eq!(format_packet.header.sequence, 0);
    decoder.commit(format_packet.header.sequence);

    let mut ack_sequence = 4;
    let (mut saw_progress, mut saw_pause, mut saw_resume) = (false, false, false);
    while !saw_progress || !saw_pause || !saw_resume {
        let request = read_record(&rx, RecordType::Control);
        match request.body[5] {
            PLAYBACK_PROGRESS_QUERY => {
                send_progress_extension_ack(&tx, &keys, session_id, ack_sequence, &request, format);
                saw_progress = true;
            }
            PAUSE => {
                send_ack(
                    &tx,
                    &keys,
                    session_id,
                    ack_sequence,
                    &request,
                    ReceiverState::Paused,
                    0,
                );
                saw_pause = true;
            }
            RESUME => {
                send_ack(
                    &tx,
                    &keys,
                    session_id,
                    ack_sequence,
                    &request,
                    ReceiverState::ReceiverRunning,
                    0,
                );
                saw_resume = true;
            }
            command => panic!("unexpected control command {command}"),
        }
        ack_sequence += 1;
    }

    let frame_bytes = format.frame_bytes as usize;
    let frame_count = if matches!(format.profile, 3 | 5) {
        480
    } else {
        1
    };
    let mut remaining_frames = frame_count;
    let mut expected_frames = Vec::new();
    while remaining_frames > 0 {
        let frames = remaining_frames.min(1150 / frame_bytes);
        expected_frames.push(frames);
        remaining_frames -= frames;
    }
    let mut payload = Vec::new();
    let mut next_frame = 9;
    for (packet_index, packet_frames) in expected_frames.iter().copied().enumerate() {
        let audio = read_record(&rx, RecordType::Audio);
        let packet = decoder.open(&audio.body).unwrap();
        assert_eq!(packet.header.packet_type, PacketType::Audio);
        assert_eq!(packet.header.sequence, packet_index as u64 + 1);
        assert_eq!(packet.header.sample_frame_index, next_frame);
        assert_eq!(packet.payload.len(), packet_frames * frame_bytes);
        assert!(packet.payload.len() <= 1150);
        next_frame += packet_frames as u64;
        payload.extend_from_slice(&packet.payload);
        decoder.commit(packet.header.sequence);
    }

    let stop = read_record(&rx, RecordType::Control);
    assert_eq!(stop.body[5], STOP);
    send_ack(
        &tx,
        &keys,
        session_id,
        ack_sequence,
        &stop,
        ReceiverState::Stopped,
        0,
    );
    (format.profile, payload, 9)
}

fn run_stalled_receiver(
    rx: ChannelReceiver<Record>,
    tx: Sender<Record>,
    material: crate::security::BindingMaterial,
    format: AudioFormatV2,
    cancelled: Arc<AtomicBool>,
) {
    let hello = read_record(&rx, RecordType::Handshake);
    let (await_finish, challenge) =
        ReceiverHandshakeV2::accept_hello(material.derive_aoa_transport(), &hello.body).unwrap();
    let session_id = u64::from_be_bytes(hello.body[72..80].try_into().unwrap());
    send_record(&tx, RecordType::Handshake, challenge);
    let finish = read_record(&rx, RecordType::Handshake);
    let (ready, authenticated) = await_finish.accept_finish(&finish.body).unwrap();
    send_record(&tx, RecordType::Handshake, ready);
    let keys = authenticated.into_keys();

    let caps = read_record(&rx, RecordType::Control);
    send_capabilities(&tx, &keys, session_id, &caps);
    let bind = read_record(&rx, RecordType::Control);
    send_ack(
        &tx,
        &keys,
        session_id,
        1,
        &bind,
        ReceiverState::TransportConnected,
        0,
    );
    let select = read_record(&rx, RecordType::Control);
    send_ack(
        &tx,
        &keys,
        session_id,
        2,
        &select,
        ReceiverState::TransportConnected,
        0,
    );
    let start = read_record(&rx, RecordType::Control);
    send_ack(
        &tx,
        &keys,
        session_id,
        3,
        &start,
        ReceiverState::ReceiverRunning,
        0,
    );
    let decoder = PacketDecoder::new_v2(
        &keys.host_to_receiver_audio,
        session_id,
        HOST_TO_RECEIVER,
        format,
    )
    .unwrap();
    let format_record = read_record(&rx, RecordType::Audio);
    let packet = decoder.open(&format_record.body).unwrap();
    assert_eq!(packet.header.packet_type, PacketType::Format);
    while !cancelled.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
}

fn run_user_stopped_progress_receiver(
    rx: ChannelReceiver<Record>,
    tx: Sender<Record>,
    material: crate::security::BindingMaterial,
) {
    let hello = read_record(&rx, RecordType::Handshake);
    let (await_finish, challenge) =
        ReceiverHandshakeV2::accept_hello(material.derive_aoa_transport(), &hello.body).unwrap();
    let session_id = u64::from_be_bytes(hello.body[72..80].try_into().unwrap());
    send_record(&tx, RecordType::Handshake, challenge);
    let finish = read_record(&rx, RecordType::Handshake);
    let (ready, authenticated) = await_finish.accept_finish(&finish.body).unwrap();
    send_record(&tx, RecordType::Handshake, ready);
    let keys = authenticated.into_keys();

    let caps = read_record(&rx, RecordType::Control);
    send_capabilities_with_bits(
        &tx,
        &keys,
        session_id,
        &caps,
        CAP_AUTHENTICATED_STATUS | CAP_AUDIO_V2 | CAP_PLAYBACK_PROGRESS | CAP_TERMINAL_CAUSE,
    );
    for (sequence, command, state) in [
        (1, BIND, ReceiverState::TransportConnected),
        (2, SELECT_MODE, ReceiverState::TransportConnected),
        (3, START, ReceiverState::ReceiverRunning),
    ] {
        let request = read_record(&rx, RecordType::Control);
        assert_eq!(request.body[5], command);
        send_ack(&tx, &keys, session_id, sequence, &request, state, 0);
    }
    let format_record = read_record(&rx, RecordType::Audio);
    assert_eq!(format_record.kind, RecordType::Audio);
    let progress = read_record(&rx, RecordType::Control);
    assert_eq!(progress.body[5], PLAYBACK_PROGRESS_QUERY);
    send_user_stopped_ack(&tx, &keys, session_id, &progress);
}

fn run_stale_block_receiver(
    rx: ChannelReceiver<Record>,
    tx: Sender<Record>,
    material: crate::security::BindingMaterial,
    format: AudioFormatV2,
) {
    let hello = read_record(&rx, RecordType::Handshake);
    let (await_finish, challenge) =
        ReceiverHandshakeV2::accept_hello(material.derive_aoa_transport(), &hello.body).unwrap();
    let session_id = u64::from_be_bytes(hello.body[72..80].try_into().unwrap());
    send_record(&tx, RecordType::Handshake, challenge);
    let finish = read_record(&rx, RecordType::Handshake);
    let (ready, authenticated) = await_finish.accept_finish(&finish.body).unwrap();
    send_record(&tx, RecordType::Handshake, ready);
    let keys = authenticated.into_keys();
    let caps = read_record(&rx, RecordType::Control);
    send_capabilities(&tx, &keys, session_id, &caps);
    for (sequence, command, state) in [
        (1, BIND, ReceiverState::TransportConnected),
        (2, SELECT_MODE, ReceiverState::TransportConnected),
        (3, START, ReceiverState::ReceiverRunning),
    ] {
        let request = read_record(&rx, RecordType::Control);
        assert_eq!(request.body[5], command);
        send_ack(&tx, &keys, session_id, sequence, &request, state, 0);
    }
    let mut decoder = PacketDecoder::new_v2(
        &keys.host_to_receiver_audio,
        session_id,
        HOST_TO_RECEIVER,
        format,
    )
    .unwrap();
    let format_record = read_record(&rx, RecordType::Audio);
    let format_packet = decoder.open(&format_record.body).unwrap();
    assert_eq!(format_packet.header.packet_type, PacketType::Format);
    decoder.commit(0);
    let first_audio = read_record(&rx, RecordType::Audio);
    let packet = decoder.open(&first_audio.body).unwrap();
    assert_eq!(packet.header.packet_type, PacketType::Audio);
    assert_eq!(packet.header.sequence, 1);
    assert_eq!(packet.payload.len(), 191 * 6);
    decoder.commit(1);
    let stop = read_record(&rx, RecordType::Control);
    assert_eq!(stop.body[5], STOP);
    send_ack(&tx, &keys, session_id, 4, &stop, ReceiverState::Stopped, 0);
}

#[test]
fn production_owner_handshakes_controls_and_sends_v2_media_for_all_profiles() {
    for profile in 1..=5 {
        let format = AudioFormatV2::for_profile(profile, 0).unwrap();
        let material = test_binding_material();
        let server_material = material.duplicate_for_session();
        let (to_server_tx, to_server_rx) = mpsc::channel();
        let (to_host_tx, to_host_rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let peer = std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                run_fake_receiver(to_server_rx, to_host_tx, server_material, format)
            });
            let _ =
                result_tx.send(result.map_err(|_| "fake receiver assertion failed".to_string()));
        });

        let queue = if profile == 1 {
            SessionAudioQueue::Pcm16(Arc::new(PcmQueue::for_session(
                PcmFormat {
                    sample_rate: 48_000,
                    channels: 2,
                },
                false,
            )))
        } else {
            let queue = Arc::new(crate::network::FloatPcmQueue::new(false));
            queue.set_expected_format(format.sample_rate, 4096).unwrap();
            SessionAudioQueue::Float32(queue, format)
        };
        let owner = AoaOwner::connect_io(
            FakeReader {
                rx: to_host_rx,
                cancelled: cancelled.clone(),
            },
            FakeWriter { tx: to_server_tx },
            FakeCancel(cancelled),
            material,
            format,
            queue.clone(),
        )
        .unwrap();

        owner.pause().unwrap();
        match &queue {
            SessionAudioQueue::Pcm16(q) => {
                let old = [100i16, -100];
                assert!(q.push_raw(old.as_ptr(), 1, 48_000, 2, 0));
            }
            SessionAudioQueue::Float32(q, _) => {
                let old = [0.25f32, -0.25];
                assert!(q.push_raw(old.as_ptr(), 1, format.sample_rate, 2, 0));
            }
        }
        owner.resume().unwrap();
        let frame_index = 9;
        match &queue {
            SessionAudioQueue::Pcm16(q) => {
                let samples = [16_384i16, -16_384];
                retry_until_accepted(|| q.push_raw(samples.as_ptr(), 1, 48_000, 2, frame_index));
            }
            SessionAudioQueue::Float32(q, _) => {
                let frame_count = if matches!(profile, 3 | 5) { 480 } else { 1 };
                let samples = [1.0 / 8_388_608.0, -1.0 / 8_388_608.0].repeat(frame_count);
                retry_until_accepted(|| {
                    q.push_raw(
                        samples.as_ptr(),
                        frame_count as u32,
                        format.sample_rate,
                        2,
                        frame_index,
                    )
                });
            }
        }
        let expected_payload_bytes =
            (if matches!(profile, 3 | 5) { 480 } else { 1 }) * format.frame_bytes as u64;
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while owner.snapshot().pcm_payload_bytes_sent < expected_payload_bytes
            && std::time::Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert!(
            owner.snapshot().pcm_payload_bytes_sent >= expected_payload_bytes,
            "owner did not emit media"
        );
        let owner_snapshot = owner.snapshot();
        let progress = owner_snapshot.playback_progress.unwrap();
        assert_eq!(progress.progress_schema, Some(2));
        assert_eq!(progress.progress_monotonic_nanos, Some(1234));
        assert_eq!(progress.receiver_generation, Some(77));
        assert_eq!(progress.ingress_audio_frames, Some(480));
        assert_eq!(progress.queue_dropped_frames, Some(3));
        assert_eq!(owner_snapshot.format, format);
        owner.stop().unwrap();
        let (got_profile, payload, got_frame) = result_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert_eq!(got_profile, profile);
        assert_eq!(got_frame, frame_index);
        if profile == 1 {
            assert_eq!(payload, [0, 0x40, 0, 0xc0]);
        } else if profile <= 3 {
            let one_frame = [1, 0, 0, 0xff, 0xff, 0xff];
            let expected: Vec<u8> = one_frame.repeat(if matches!(profile, 3) { 480 } else { 1 });
            assert_eq!(payload, expected);
        } else {
            let one_frame = [
                (1.0f32 / 8_388_608.0).to_le_bytes(),
                (-1.0f32 / 8_388_608.0).to_le_bytes(),
            ]
            .concat();
            let expected: Vec<u8> = one_frame.repeat(if profile == 5 { 480 } else { 1 });
            assert_eq!(payload, expected);
        }
        peer.join().unwrap();
    }
}

#[test]
fn production_owner_rejects_mismatched_queue_before_io() {
    let format = AudioFormatV2::for_profile(2, 0).unwrap();
    let queue = Arc::new(crate::network::FloatPcmQueue::new(false));
    queue.set_expected_format(48_000, 4096).unwrap();
    let queue = SessionAudioQueue::Float32(queue, AudioFormatV2::for_profile(3, 0).unwrap());
    let (to_server_tx, _to_server_rx) = mpsc::channel();
    let (_to_host_tx, to_host_rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let result = AoaOwner::connect_io(
        FakeReader {
            rx: to_host_rx,
            cancelled: cancelled.clone(),
        },
        FakeWriter { tx: to_server_tx },
        FakeCancel(cancelled),
        test_binding_material(),
        format,
        queue,
    );
    assert!(matches!(result, Err(error) if error.contains("queue does not match")));
}

#[test]
fn production_owner_rejects_lan_binding_domain_during_handshake() {
    let format = AudioFormatV2::for_profile(2, 0).unwrap();
    let material = test_binding_material();
    let server_material = material.duplicate_for_session();
    let (to_server_tx, to_server_rx) = mpsc::channel();
    let (to_host_tx, to_host_rx) = mpsc::channel();
    let peer = std::thread::spawn(move || {
        let hello = read_record(&to_server_rx, RecordType::Handshake);
        // Deliberately use the ordinary LAN binding instead of derive_aoa_transport().
        let (_await_finish, challenge) =
            ReceiverHandshakeV2::accept_hello(server_material, &hello.body).unwrap();
        send_record(&to_host_tx, RecordType::Handshake, challenge);
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let queue = Arc::new(crate::network::FloatPcmQueue::new(false));
    queue.set_expected_format(48_000, 4096).unwrap();
    let result = AoaOwner::connect_io(
        FakeReader {
            rx: to_host_rx,
            cancelled: cancelled.clone(),
        },
        FakeWriter { tx: to_server_tx },
        FakeCancel(cancelled.clone()),
        material,
        format,
        SessionAudioQueue::Float32(queue, format),
    );
    assert!(matches!(result, Err(error) if error.contains("identity or proof was rejected")));
    assert!(cancelled.load(Ordering::Acquire));
    peer.join().unwrap();
}

#[test]
fn dropping_owner_cancels_and_joins_an_idle_reader() {
    let format = AudioFormatV2::for_profile(1, 0).unwrap();
    let material = test_binding_material();
    let server_material = material.duplicate_for_session();
    let (to_server_tx, to_server_rx) = mpsc::channel();
    let (to_host_tx, to_host_rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let peer_cancelled = cancelled.clone();
    let peer = std::thread::spawn(move || {
        run_stalled_receiver(
            to_server_rx,
            to_host_tx,
            server_material,
            format,
            peer_cancelled,
        )
    });
    let queue = SessionAudioQueue::Pcm16(Arc::new(PcmQueue::for_session(
        PcmFormat {
            sample_rate: 48_000,
            channels: 2,
        },
        false,
    )));
    let owner = AoaOwner::connect_io(
        FakeReader {
            rx: to_host_rx,
            cancelled: cancelled.clone(),
        },
        FakeWriter { tx: to_server_tx },
        FakeCancel(cancelled.clone()),
        material,
        format,
        queue.clone(),
    )
    .unwrap();
    drop(owner);
    assert!(cancelled.load(Ordering::Acquire));
    assert!(matches!(&queue, SessionAudioQueue::Pcm16(q) if q.is_stopped()));
    peer.join().unwrap();
}

#[test]
fn user_stopped_progress_ack_is_terminal_and_does_not_fail_owner() {
    let format = AudioFormatV2::for_profile(1, 0).unwrap();
    let material = test_binding_material();
    let server_material = material.duplicate_for_session();
    let (to_server_tx, to_server_rx) = mpsc::channel();
    let (to_host_tx, to_host_rx) = mpsc::channel();
    let peer = std::thread::spawn(move || {
        run_user_stopped_progress_receiver(to_server_rx, to_host_tx, server_material)
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let queue = SessionAudioQueue::Pcm16(Arc::new(PcmQueue::for_session(
        PcmFormat {
            sample_rate: 48_000,
            channels: 2,
        },
        false,
    )));
    let owner = AoaOwner::connect_io(
        FakeReader {
            rx: to_host_rx,
            cancelled: cancelled.clone(),
        },
        FakeWriter { tx: to_server_tx },
        FakeCancel(cancelled),
        material,
        format,
        queue,
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let snapshot = loop {
        let snapshot = owner.snapshot();
        if snapshot.terminal_cause.is_some() || std::time::Instant::now() >= deadline {
            break snapshot;
        }
        std::thread::yield_now();
    };
    assert_eq!(snapshot.state, ReceiverState::Stopped);
    assert_eq!(
        snapshot.terminal_cause,
        Some(TerminalCause::ReceiverUserStop)
    );
    assert!(snapshot.last_error.is_none());
    drop(owner);
    peer.join().unwrap();
}

#[test]
fn unexpected_active_queue_stop_fails_owner_and_joins_reader() {
    let format = AudioFormatV2::for_profile(1, 0).unwrap();
    let material = test_binding_material();
    let server_material = material.duplicate_for_session();
    let (to_server_tx, to_server_rx) = mpsc::channel();
    let (to_host_tx, to_host_rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let peer_cancelled = cancelled.clone();
    let peer = std::thread::spawn(move || {
        run_stalled_receiver(
            to_server_rx,
            to_host_tx,
            server_material,
            format,
            peer_cancelled,
        )
    });
    let queue = SessionAudioQueue::Pcm16(Arc::new(PcmQueue::for_session(
        PcmFormat {
            sample_rate: 48_000,
            channels: 2,
        },
        false,
    )));
    let owner = AoaOwner::connect_io(
        FakeReader {
            rx: to_host_rx,
            cancelled: cancelled.clone(),
        },
        FakeWriter { tx: to_server_tx },
        FakeCancel(cancelled.clone()),
        material,
        format,
        queue.clone(),
    )
    .unwrap();
    if let SessionAudioQueue::Pcm16(queue) = &queue {
        queue.stop_and_clear();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while owner.snapshot().last_error.is_none() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let snapshot = owner.snapshot();
    assert_eq!(snapshot.state, ReceiverState::Stopped);
    assert!(snapshot
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("queue stopped")));
    drop(owner);
    assert!(cancelled.load(Ordering::Acquire));
    peer.join().unwrap();
}

#[test]
fn expired_pending_profile3_frames_are_counted_after_fake_write_stall() {
    let format = AudioFormatV2::for_profile(3, 0).unwrap();
    let material = test_binding_material();
    let server_material = material.duplicate_for_session();
    let (to_server_tx, to_server_rx) = mpsc::channel();
    let (to_host_tx, to_host_rx) = mpsc::channel();
    let peer = std::thread::spawn(move || {
        run_stale_block_receiver(to_server_rx, to_host_tx, server_material, format)
    });
    let queue = Arc::new(crate::network::FloatPcmQueue::new(false));
    queue.set_expected_format(96_000, 4096).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let owner = AoaOwner::connect_io(
        FakeReader {
            rx: to_host_rx,
            cancelled: cancelled.clone(),
        },
        DelayedFakeWriter {
            tx: to_server_tx,
            audio_records: 0,
        },
        FakeCancel(cancelled),
        material,
        format,
        SessionAudioQueue::Float32(queue.clone(), format),
    )
    .unwrap();
    let samples = [1.0f32, -1.0].repeat(480);
    retry_until_accepted(|| queue.push_raw(samples.as_ptr(), 480, 96_000, 2, 100));
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while queue.dropped_frames() < 289 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(queue.dropped_frames(), 289);
    owner.stop().unwrap();
    peer.join().unwrap();
}
