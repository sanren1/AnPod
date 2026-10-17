
use super::*;
use crate::protocol::{PacketType, PcmFormat};

#[test]
fn session_queue_discards_audio_that_does_not_match_negotiated_format() {
    let queue = PcmQueue::for_session(
        PcmFormat {
            sample_rate: 48_000,
            channels: 2,
        },
        false,
    );
    let samples = vec![0i16; 480 * 2];
    assert!(!queue.push_raw(samples.as_ptr(), 480, 44_100, 2, 0));
    assert_eq!(queue.len(), 0);
}

#[test]
fn transient_udp_send_retries_are_bounded_and_never_replay_ciphertext() {
    let mut encoder = PacketEncoder::new(&[7; 32], 9, HOST_TO_RECEIVER);
    let mut attempts = 0;
    let mut packets = Vec::new();
    send_packet_with(&mut encoder, PacketType::Audio, 0, &[1, 2], |packet| {
        attempts += 1;
        packets.push(packet.to_vec());
        if attempts < 3 {
            Err(std::io::ErrorKind::WouldBlock.into())
        } else {
            Ok(packet.len())
        }
    })
    .unwrap();
    assert_eq!(attempts, 3);
    assert_ne!(packets[0], packets[1]);
    assert_ne!(packets[1], packets[2]);

    attempts = 0;
    let error = send_packet_with(&mut encoder, PacketType::Audio, 240, &[3, 4], |_| {
        attempts += 1;
        Err(std::io::ErrorKind::WouldBlock.into())
    })
    .unwrap_err();
    assert_eq!(attempts, UDP_SEND_ATTEMPTS);
    assert!(!error.is_empty());
}

#[test]
fn loopback_pcm_queue_protocol_auth_replay_pause_and_freshness() {
    let format = PcmFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    let queue = Arc::new(PcmQueue::new());
    let callback_state = Arc::as_ptr(&queue) as *mut std::ffi::c_void;
    let samples = vec![0x1234i16; 240 * 2];
    for index in 0..PCM_QUEUE_CAPACITY + 2 {
        unsafe {
            crate::audio::on_native_pcm(
                samples.as_ptr(),
                240,
                format.sample_rate,
                format.channels,
                (index * 240) as u64,
                callback_state,
            );
        }
    }
    assert_eq!(queue.dropped_frames(), 480);
    assert_eq!(queue.len(), PCM_QUEUE_CAPACITY);

    let material = UdpSessionMaterial::generate().unwrap();
    let key = material.key;
    let session_id = material.session_id;
    let receiver_socket = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    receiver_socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let sender = UdpSender::spawn(
        receiver_socket.local_addr().unwrap(),
        queue.clone(),
        material,
        format,
    )
    .unwrap();
    let mut receiver = MockReceiver::new(&key, session_id);
    assert_eq!(
        receiver.receive(&receiver_socket).unwrap(),
        IngestResult::Format
    );
    let mut received_frame_indices = Vec::new();
    for _ in 0..PCM_QUEUE_CAPACITY {
        let mut datagram = [0; MAX_DATAGRAM_LEN + 1];
        let (length, _) = receiver_socket.recv_from(&mut datagram).unwrap();
        assert_eq!(length, 1010);
        received_frame_indices.push(u64::from_be_bytes(datagram[24..32].try_into().unwrap()));
        assert_eq!(
            receiver.ingest(&datagram[..length]).unwrap(),
            IngestResult::Queued
        );
    }
    assert_eq!(
        received_frame_indices,
        (2..PCM_QUEUE_CAPACITY + 2)
            .map(|index| (index * 240) as u64)
            .collect::<Vec<_>>()
    );
    assert!(receiver.queue.len() <= MAX_JITTER_PACKETS);
    assert_eq!(receiver.queue.front().unwrap().frame_index, 1440);
    let stats = sender.stop().unwrap();
    assert_eq!(stats.packets_sent, (PCM_QUEUE_CAPACITY + 1) as u64);
    assert_eq!(
        stats.pcm_payload_bytes_sent,
        (PCM_QUEUE_CAPACITY * 960) as u64
    );

    while let Some(packet) = receiver.pop_audio() {
        assert!((5..=8).contains(&packet.sequence));
        assert_eq!(packet.payload.len(), 960);
    }
    let out_of_order = PacketEncoder::with_sequence(&key, session_id, HOST_TO_RECEIVER, 0);
    let pcm_payload = vec![0u8; 960];
    let late_packet = out_of_order
        .encode_at(9, PacketType::Audio, 2400, &pcm_payload)
        .unwrap();
    let ahead_packet = out_of_order
        .encode_at(10, PacketType::Audio, 2640, &pcm_payload)
        .unwrap();
    let valid_retry = out_of_order
        .encode_at(11, PacketType::Audio, 3120, &pcm_payload)
        .unwrap();
    let mut bad_tag = valid_retry.clone();
    *bad_tag.last_mut().unwrap() ^= 0x80;
    receiver.set_paused(true, 0);
    assert_eq!(
        receiver.ingest(&ahead_packet).unwrap(),
        IngestResult::Paused
    );
    assert_eq!(
        receiver.ingest(&bad_tag),
        Err(ProtocolError::Authentication)
    );
    assert_eq!(receiver.ingest(&ahead_packet), Err(ProtocolError::Replay));
    receiver.set_paused(false, 3120);
    assert_eq!(receiver.ingest(&late_packet).unwrap(), IngestResult::Stale);
    assert_eq!(receiver.ingest(&valid_retry).unwrap(), IngestResult::Queued);
    assert_eq!(receiver.queue.len(), 1);
    assert_eq!(receiver.pop_audio().unwrap().frame_index, 3120);
    assert!(receiver.stale_drops >= 5);

    let wrong_session =
        PacketEncoder::with_sequence(&[0x6b; 32], session_id + 1, HOST_TO_RECEIVER, 12)
            .encode_at(12, PacketType::Audio, 3360, &pcm_payload)
            .unwrap();
    assert_eq!(
        receiver.ingest(&wrong_session),
        Err(ProtocolError::WrongSession)
    );
}
