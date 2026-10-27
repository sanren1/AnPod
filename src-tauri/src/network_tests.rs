use super::*;
use crate::protocol::{PacketType, PcmFormat};

#[test]
fn loss_snapshot_watermarks_reject_inconsistent_tail_and_settled_pairs() {
    let tail = AtomicU64::new(100);
    let tail_known = AtomicBool::new(true);
    let settled = AtomicU64::new(90);
    let settled_known = AtomicBool::new(true);
    assert_eq!(
        stable_source_watermarks(&tail, &tail_known, &settled, &settled_known),
        Some((100, 90))
    );
    settled.store(101, Ordering::Release);
    assert_eq!(
        stable_source_watermarks(&tail, &tail_known, &settled, &settled_known),
        None
    );
    settled.store(90, Ordering::Release);
    tail_known.store(false, Ordering::Release);
    assert_eq!(
        stable_source_watermarks(&tail, &tail_known, &settled, &settled_known),
        None
    );
}
#[test]
fn pcm_and_float_capture_queues_merge_indexed_drops_and_keep_capture_tail() {
    let float = FloatPcmQueue::new(true);
    float.set_expected_format(48_000, 4096).unwrap();
    let float_samples = vec![0.0f32; 1000 * 2];
    assert!(float.push_raw(float_samples.as_ptr(), 1000, 48_000, 2, 0));
    let float_samples = vec![0.0f32; 100 * 2];
    assert!(float.push_raw(float_samples.as_ptr(), 100, 48_000, 2, 1000));
    let float_snapshot = float.source_frame_snapshot();
    assert_eq!(float_snapshot.capture_tail, Some(1100));
    assert!(float_snapshot.complete);
    assert_eq!(float_snapshot.interval_count, 1);
    assert_eq!(
        float_snapshot.intervals[0],
        SourceFrameInterval {
            start: 0,
            end: 1000
        }
    );

    let pcm = PcmQueue::for_session(
        PcmFormat {
            sample_rate: 48_000,
            channels: 2,
        },
        false,
    );
    let pcm_samples = vec![0i16; 2000 * 2];
    assert!(pcm.push_raw(pcm_samples.as_ptr(), 2000, 48_000, 2, 0));
    let pcm_samples = vec![0i16; 100 * 2];
    assert!(pcm.push_raw(pcm_samples.as_ptr(), 100, 48_000, 2, 2000));
    let pcm_snapshot = pcm.source_frame_snapshot();
    assert_eq!(pcm_snapshot.capture_tail, Some(2100));
    assert!(pcm_snapshot.complete);
    assert_eq!(pcm_snapshot.interval_count, 1);
    assert_eq!(
        pcm_snapshot.intervals[0],
        SourceFrameInterval {
            start: 0,
            end: 2000
        }
    );
}
#[test]
fn authenticated_float_sender_sends_loss_snapshot_with_tail_pending() {
    let format = AudioFormatV2::for_profile(2, 0).unwrap();
    let queue = Arc::new(FloatPcmQueue::new(false));
    queue.set_expected_format(format.sample_rate, 4096).unwrap();
    let key = [0x72; 32];
    let session_id = 0x1112_1314_1516_1718;
    let receiver = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let sender = FloatUdpSender::spawn_authenticated(
        receiver.local_addr().unwrap(),
        queue.clone(),
        UdpSocket::bind(("127.0.0.1", 0)).unwrap(),
        UdpSessionMaterial { session_id, key },
        format,
        Arc::new(AtomicU64::new(0)),
        true,
    )
    .unwrap();
    let mut decoder = PacketDecoder::new_v2(&key, session_id, HOST_TO_RECEIVER, format).unwrap();
    let mut datagram = [0u8; MAX_DATAGRAM_LEN];
    let (length, _) = receiver.recv_from(&mut datagram).unwrap();
    let descriptor = decoder.open(&datagram[..length]).unwrap();
    decoder.commit(descriptor.header.sequence);
    assert_eq!(descriptor.header.packet_type, PacketType::Format);

    let samples = vec![0.0f32; 400 * 2];
    assert!(queue.push_raw(samples.as_ptr(), 400, format.sample_rate, 2, 7));
    let mut ledger = None;
    for _ in 0..4 {
        let (length, _) = receiver.recv_from(&mut datagram).unwrap();
        let packet = decoder.open(&datagram[..length]).unwrap();
        decoder.commit(packet.header.sequence);
        if packet.header.packet_type == PacketType::LossLedger {
            ledger = Some(LossLedger::decode(&packet.payload).unwrap());
            break;
        }
    }
    let ledger =
        ledger.expect("sender must publish the settled prefix while capture tail is pending");
    assert_eq!(ledger.capture_tail, 407);
    assert!(ledger.settled_through < ledger.capture_tail);
    assert!(sender.stop().is_ok());
}

#[test]
fn settlement_waits_for_successful_audio_and_known_pc_drop_ranges() {
    let queue = FloatPcmQueue::new(false);
    queue.set_expected_format(48_000, 4096).unwrap();
    let first = vec![0.0f32; 100 * 2];
    assert!(queue.push_raw(first.as_ptr(), 100, 48_000, 2, 5));
    let captured = queue.source_frame_snapshot();
    assert_eq!(captured.capture_tail, Some(105));
    assert_eq!(captured.settled_through, Some(5));

    queue.note_audio_sent(queue.source_segment_generation(), 5, 100);
    assert_eq!(queue.source_frame_snapshot().settled_through, Some(105));
    queue.record_dropped_frames(Some(105), 2);
    assert_eq!(queue.source_frame_snapshot().settled_through, Some(107));

    let last = [0.0f32; 2];
    assert!(queue.push_raw(last.as_ptr(), 1, 48_000, 2, 107));
    queue.note_audio_sent(queue.source_segment_generation(), 107, 1);
    let settled = queue.source_frame_snapshot();
    assert_eq!(settled.capture_tail, Some(108));
    assert_eq!(settled.settled_through, Some(108));
    assert_eq!(
        settled.intervals[0],
        SourceFrameInterval {
            start: 105,
            end: 107
        }
    );
    assert!(settled.complete);
}

#[test]
fn pause_segment_excludes_capture_hole_and_preserves_prior_pc_loss() {
    let queue = FloatPcmQueue::new(false);
    queue.set_expected_format(48_000, 4096).unwrap();
    let before_pause = vec![0.0f32; 10 * 2];
    assert!(queue.push_raw(before_pause.as_ptr(), 10, 48_000, 2, 100));
    queue.record_dropped_frames(Some(110), 2);
    queue.note_audio_sent(queue.source_segment_generation(), 100, 10);
    let before = queue.source_frame_snapshot();
    assert_eq!(before.settled_through, Some(112));
    assert_eq!(
        before.intervals[0],
        SourceFrameInterval {
            start: 110,
            end: 112
        }
    );

    let ended_generation = queue.source_segment_generation();
    queue.end_source_segment();
    let paused = vec![0.0f32; 10 * 2];
    assert!(!queue.push_raw(paused.as_ptr(), 10, 48_000, 2, 112));
    queue.record_dropped_frames(Some(112), 10);
    queue.note_audio_sent(queue.source_segment_generation(), 112, 10);
    assert_eq!(queue.source_frame_snapshot().settled_through, Some(112));

    queue.clear_queued();
    queue.begin_source_segment();
    let after_resume = vec![0.0f32; 5 * 2];
    assert!(queue.push_raw(after_resume.as_ptr(), 5, 48_000, 2, 130));
    let resumed = queue.source_frame_snapshot();
    assert_eq!(resumed.settled_through, Some(130));
    assert_eq!(resumed.capture_tail, Some(135));
    assert_eq!(resumed.interval_count, 1);
    assert_eq!(
        resumed.intervals[0],
        SourceFrameInterval {
            start: 110,
            end: 112
        }
    );
    queue.note_audio_sent(ended_generation, 130, 5);
    assert_eq!(queue.source_frame_snapshot().settled_through, Some(130));
    queue.note_audio_sent(queue.source_segment_generation(), 130, 5);
    assert_eq!(queue.source_frame_snapshot().settled_through, Some(135));
    assert!(queue.source_frame_snapshot().complete);
}

#[test]
fn local_source_index_reset_is_remapped_to_session_global_frames() {
    let floats = FloatPcmQueue::new(false);
    floats.set_expected_format(48_000, 4096).unwrap();
    let first = vec![0.0f32; 20];
    assert!(floats.push_raw(first.as_ptr(), 10, 48_000, 2, 400));
    let mut float_scratch = Vec::new();
    let first_info = floats
        .pop_timeout(&mut float_scratch, Duration::ZERO)
        .unwrap();
    assert_eq!(first_info.frame_index, 400);
    floats.note_audio_sent(floats.source_segment_generation(), 400, 10);
    floats.clear_queued();
    floats.begin_source_segment();
    let resumed = vec![0.0f32; 10];
    assert!(floats.push_raw(resumed.as_ptr(), 5, 48_000, 2, 0));
    let resumed_info = floats
        .pop_timeout(&mut float_scratch, Duration::ZERO)
        .unwrap();
    assert_eq!(resumed_info.frame_index, 410);
    floats.note_audio_sent(floats.source_segment_generation(), 410, 5);
    let snapshot = floats.source_frame_snapshot();
    assert_eq!(snapshot.capture_tail, Some(415));
    assert_eq!(snapshot.settled_through, Some(415));
    assert!(snapshot.complete);

    let pcm = PcmQueue::for_session(
        PcmFormat {
            sample_rate: 48_000,
            channels: 2,
        },
        false,
    );
    let first_pcm = vec![0i16; 20];
    assert!(pcm.push_raw(first_pcm.as_ptr(), 10, 48_000, 2, 400));
    let mut pcm_scratch = Vec::new();
    let first_info = pcm.pop_timeout(&mut pcm_scratch, Duration::ZERO).unwrap();
    assert_eq!(first_info.frame_index, 400);
    pcm.note_audio_sent(pcm.source_segment_generation(), 400, 10);
    pcm.clear_queued();
    pcm.begin_source_segment();
    let resumed_pcm = vec![0i16; 10];
    assert!(pcm.push_raw(resumed_pcm.as_ptr(), 5, 48_000, 2, 0));
    let resumed_info = pcm.pop_timeout(&mut pcm_scratch, Duration::ZERO).unwrap();
    assert_eq!(resumed_info.frame_index, 410);
    pcm.note_audio_sent(pcm.source_segment_generation(), 410, 5);
    let snapshot = pcm.source_frame_snapshot();
    assert_eq!(snapshot.capture_tail, Some(415));
    assert_eq!(snapshot.settled_through, Some(415));
    assert!(snapshot.complete);
}
fn float32_wire_encoding_preserves_bits_and_rejects_non_finite_values() {
    for sample in [0.0f32, -0.0, 0.25, -0.5, f32::MIN_POSITIVE] {
        assert_eq!(float32_to_le_bytes(sample).unwrap(), sample.to_le_bytes());
    }
    assert!(float32_to_le_bytes(f32::NAN).is_err());
    assert!(float32_to_le_bytes(f32::INFINITY).is_err());
    assert!(float32_to_le_bytes(f32::NEG_INFINITY).is_err());
}

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
fn float_queue_bounds_capture_frames_and_preserves_callback_samples() {
    let queue = Arc::new(FloatPcmQueue::new(true));
    queue.set_expected_format(48_000, 4096).unwrap();
    let callback_state = Arc::as_ptr(&queue) as *mut std::ffi::c_void;
    let samples = (0..1000)
        .flat_map(|frame| [frame as f32, -(frame as f32)])
        .collect::<Vec<_>>();
    unsafe {
        crate::audio::on_native_float_pcm(samples.as_ptr(), 1000, 48_000, 2, 120, callback_state);
    }

    let mut scratch = Vec::with_capacity(1920);
    let info = queue.pop_wait(&mut scratch).unwrap();
    assert_eq!(info.frame_count, 960);
    assert_eq!(info.sample_rate, 48_000);
    assert_eq!(info.channels, 2);
    assert_eq!(info.frame_index, 160);
    assert_eq!(scratch.len(), 1920);
    assert_eq!(&scratch[..4], &[40.0, -40.0, 41.0, -41.0]);
    assert_eq!(queue.dropped_frames(), 40);
}

#[test]
fn float_queue_rejects_native_callbacks_above_the_advertised_maximum() {
    let queue = Arc::new(FloatPcmQueue::new(true));
    queue.set_expected_format(48_000, 4096).unwrap();
    let callback_state = Arc::as_ptr(&queue) as *mut std::ffi::c_void;
    let one_frame = [0.25f32, -0.25];
    unsafe {
        crate::audio::on_native_float_pcm(one_frame.as_ptr(), 4097, 48_000, 2, 0, callback_state);
    }
    assert_eq!(queue.dropped_frames(), 4097);
    assert_eq!(queue.state.lock().unwrap().len, 0);
}

#[test]
fn legacy_pcm16_capture_callback_remains_sample_exact() {
    let queue = Arc::new(PcmQueue::new());
    let callback_state = Arc::as_ptr(&queue) as *mut std::ffi::c_void;
    let samples = [i16::MIN, -1, 0, i16::MAX];
    unsafe {
        crate::audio::on_native_pcm(samples.as_ptr(), 2, 48_000, 2, 77, callback_state);
    }
    let mut scratch = Vec::new();
    let info = queue
        .pop_wait(&AtomicBool::new(false), &mut scratch)
        .unwrap();
    assert_eq!(info.frame_index, 77);
    assert_eq!(scratch, samples);
}

#[test]
fn stopped_float_queue_wakes_consumer_and_cannot_be_restarted() {
    let queue = Arc::new(FloatPcmQueue::new(false));
    queue.set_expected_format(48_000, 4096).unwrap();
    let waiting_queue = queue.clone();
    let waiter = std::thread::spawn(move || {
        let mut scratch = Vec::with_capacity(8192);
        waiting_queue.pop_wait(&mut scratch)
    });
    std::thread::yield_now();
    queue.stop_and_clear();
    assert!(waiter.join().unwrap().is_none());
    assert!(queue.set_expected_format(48_000, 4096).is_err());
}

#[test]
fn pcm24_quantizer_uses_symmetric_q23_rounding_and_rejects_non_finite() {
    assert_eq!(float_to_pcm24(-1.0), Ok(-8_388_608));
    assert_eq!(float_to_pcm24(1.0), Ok(8_388_607));
    assert_eq!(float_to_pcm24(0.5 / 8_388_608.0), Ok(1));
    assert_eq!(float_to_pcm24(-0.5 / 8_388_608.0), Ok(-1));
    assert_eq!(float_to_pcm24(0.0), Ok(0));
    assert_eq!(
        float_to_pcm24(f32::NAN),
        Err("Float32 PCM contains a non-finite sample.")
    );
    assert_eq!(
        float_to_pcm24(f32::INFINITY),
        Err("Float32 PCM contains a non-finite sample.")
    );
}

#[test]
fn float_udp_sender_splits_24bit_packets_on_frame_boundaries() {
    let format = crate::protocol::AudioFormatV2::for_profile(2, 0).unwrap();
    let queue = Arc::new(FloatPcmQueue::new(false));
    queue.set_expected_format(48_000, 4096).unwrap();
    let mut samples = vec![0.5f32; 480 * 2];
    samples[0] = 1.0 / 32768.0;

    let key = [0x71; 32];
    let session_id = 0x0102_0304_0506_0708;
    let receiver = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let send_socket = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    let peer = receiver.local_addr().unwrap();
    let sender_queue = queue.clone();
    let expected_queue_stop = Arc::new(AtomicBool::new(false));
    let worker_expected_queue_stop = expected_queue_stop.clone();
    let worker = std::thread::spawn(move || {
        send_float_loop(
            send_socket,
            peer,
            sender_queue,
            FloatSenderSignals {
                stopping: Arc::new(AtomicBool::new(false)),
                expected_queue_stop: worker_expected_queue_stop,
                pcm_payload_bytes_sent: Arc::new(AtomicU64::new(0)),
                send_loss_ledger: false,
            },
            UdpSessionMaterial { session_id, key },
            format,
        )
    });

    let mut decoder = PacketDecoder::new_v2(&key, session_id, HOST_TO_RECEIVER, format).unwrap();
    let mut datagram = [0u8; MAX_DATAGRAM_LEN];
    let (length, _) = receiver.recv_from(&mut datagram).unwrap();
    let format_packet = decoder.open(&datagram[..length]).unwrap();
    decoder.commit(format_packet.header.sequence);
    assert_eq!(format_packet.header.packet_type, PacketType::Format);
    assert!(queue.push_raw(samples.as_ptr(), 480, 48_000, 2, 1000));
    let mut audio = Vec::new();
    for _ in 0..3 {
        let (length, _) = receiver.recv_from(&mut datagram).unwrap();
        let packet = decoder.open(&datagram[..length]).unwrap();
        decoder.commit(packet.header.sequence);
        audio.push(packet);
    }
    expected_queue_stop.store(true, Ordering::Release);
    queue.stop_and_clear();
    let stats = worker.join().unwrap().unwrap();
    assert_eq!(stats.packets_sent, 4);
    assert_eq!(
        audio
            .iter()
            .map(|packet| packet.header.sample_frame_index)
            .collect::<Vec<_>>(),
        [1000, 1191, 1382]
    );
    assert_eq!(
        audio
            .iter()
            .map(|packet| packet.payload.len())
            .collect::<Vec<_>>(),
        [1146, 1146, 588]
    );
    assert_eq!(&audio[0].payload[..3], &[0, 1, 0]);
    assert!(audio
        .iter()
        .flat_map(|packet| packet.payload.chunks_exact(3).skip(1))
        .all(|sample| sample == [0, 0, 64]));
}

#[test]
fn float_udp_sender_splits_96khz_profile3_and_preserves_low_q23_samples_and_indices() {
    let format = crate::protocol::AudioFormatV2::for_profile(3, 0).unwrap();
    let queue = Arc::new(FloatPcmQueue::new(false));
    queue.set_expected_format(96_000, 4096).unwrap();
    let mut samples = vec![0.0f32; 480 * 2];
    samples[..4].copy_from_slice(&[
        1.0 / 8_388_608.0,
        -1.0 / 8_388_608.0,
        32_769.0 / 8_388_608.0,
        -32_769.0 / 8_388_608.0,
    ]);
    let key = [0x72; 32];
    let session_id = 0x1112_1314_1516_1718;
    let receiver = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let sender = FloatUdpSender::spawn_authenticated(
        receiver.local_addr().unwrap(),
        queue.clone(),
        UdpSocket::bind(("127.0.0.1", 0)).unwrap(),
        UdpSessionMaterial { session_id, key },
        format,
        Arc::new(AtomicU64::new(0)),
        false,
    )
    .unwrap();

    let mut decoder = PacketDecoder::new_v2(&key, session_id, HOST_TO_RECEIVER, format).unwrap();
    let mut datagram = [0u8; MAX_DATAGRAM_LEN];
    let (length, _) = receiver.recv_from(&mut datagram).unwrap();
    let format_packet = decoder.open(&datagram[..length]).unwrap();
    decoder.commit(format_packet.header.sequence);
    assert_eq!(format_packet.header.packet_type, PacketType::Format);
    assert!(queue.push_raw(samples.as_ptr(), 480, 96_000, 2, 7));
    let mut audio = Vec::new();
    for _ in 0..3 {
        let (length, _) = receiver.recv_from(&mut datagram).unwrap();
        let packet = decoder.open(&datagram[..length]).unwrap();
        decoder.commit(packet.header.sequence);
        audio.push(packet);
    }
    sender.expect_queue_stop();
    queue.stop_and_clear();
    let stats = sender.stop().unwrap();

    assert_eq!(stats.packets_sent, 4);
    assert_eq!(
        audio
            .iter()
            .map(|packet| packet.header.sample_frame_index)
            .collect::<Vec<_>>(),
        [7, 198, 389]
    );
    assert_eq!(
        audio
            .iter()
            .map(|packet| packet.payload.len())
            .collect::<Vec<_>>(),
        [1146, 1146, 588]
    );
    assert_eq!(
        &audio[0].payload[..12],
        &[1, 0, 0, 255, 255, 255, 1, 128, 0, 255, 127, 255]
    );
}

#[test]
fn float_udp_sender_preserves_float32_le_for_profiles_4_and_5() {
    for (profile, rate) in [(4, 48_000), (5, 96_000)] {
        let format = crate::protocol::AudioFormatV2::for_profile(profile, 0).unwrap();
        let queue = Arc::new(FloatPcmQueue::new(false));
        queue.set_expected_format(rate, 4096).unwrap();
        let receiver = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let sender = FloatUdpSender::spawn_authenticated(
            receiver.local_addr().unwrap(),
            queue.clone(),
            UdpSocket::bind(("127.0.0.1", 0)).unwrap(),
            UdpSessionMaterial {
                session_id: 0x2122_2324_2526_2728,
                key: [0x73; 32],
            },
            format,
            Arc::new(AtomicU64::new(0)),
            false,
        )
        .unwrap();

        let mut decoder =
            PacketDecoder::new_v2(&[0x73; 32], 0x2122_2324_2526_2728, HOST_TO_RECEIVER, format)
                .unwrap();
        let mut datagram = [0u8; MAX_DATAGRAM_LEN];
        let (length, _) = receiver.recv_from(&mut datagram).unwrap();
        let format_packet = decoder.open(&datagram[..length]).unwrap();
        decoder.commit(format_packet.header.sequence);
        assert_eq!(format_packet.payload, format.encode().unwrap());

        let mut samples = vec![0.0f32; 150 * 2];
        samples[..6].copy_from_slice(&[0.25, -0.5, -0.0, f32::MIN_POSITIVE, 1.0, -1.0]);
        assert!(queue.push_raw(samples.as_ptr(), 150, rate, 2, 900));
        let (length, _) = receiver.recv_from(&mut datagram).unwrap();
        let first = decoder.open(&datagram[..length]).unwrap();
        decoder.commit(first.header.sequence);
        let (length, _) = receiver.recv_from(&mut datagram).unwrap();
        let second = decoder.open(&datagram[..length]).unwrap();
        decoder.commit(second.header.sequence);
        assert_eq!(first.header.sample_frame_index, 900);
        assert_eq!(second.header.sample_frame_index, 900 + 1150 / 8);
        assert_eq!(first.payload.len(), (1150 / 8) * 8);
        assert_eq!(second.payload.len(), (150 - 1150 / 8) * 8);
        let expected = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(
            [first.payload.as_slice(), second.payload.as_slice()].concat(),
            expected
        );
        sender.expect_queue_stop();
        queue.stop_and_clear();
        assert_eq!(sender.stop().unwrap().packets_sent, 3);
    }
}

#[test]
fn float_udp_sender_reports_an_unplanned_active_queue_stop() {
    let format = crate::protocol::AudioFormatV2::for_profile(2, 0).unwrap();
    let queue = Arc::new(FloatPcmQueue::new(false));
    queue.set_expected_format(48_000, 4096).unwrap();
    let receiver = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let send_socket = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    let peer = receiver.local_addr().unwrap();
    let sender_queue = queue.clone();
    let worker = std::thread::spawn(move || {
        send_float_loop(
            send_socket,
            peer,
            sender_queue,
            FloatSenderSignals {
                stopping: Arc::new(AtomicBool::new(false)),
                expected_queue_stop: Arc::new(AtomicBool::new(false)),
                pcm_payload_bytes_sent: Arc::new(AtomicU64::new(0)),
                send_loss_ledger: false,
            },
            UdpSessionMaterial {
                session_id: 99,
                key: [4; 32],
            },
            format,
        )
    });
    let mut datagram = [0u8; MAX_DATAGRAM_LEN];
    let _ = receiver.recv_from(&mut datagram).unwrap();
    queue.stop_and_clear();
    assert!(worker.join().unwrap().is_err());
}

#[test]
fn float_udp_sender_reports_a_queue_stop_after_resume_ack() {
    let format = crate::protocol::AudioFormatV2::for_profile(2, 0).unwrap();
    let queue = Arc::new(FloatPcmQueue::new(false));
    queue.set_expected_format(48_000, 4096).unwrap();
    let receiver = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let sender = FloatUdpSender::spawn_authenticated(
        receiver.local_addr().unwrap(),
        queue.clone(),
        UdpSocket::bind(("127.0.0.1", 0)).unwrap(),
        UdpSessionMaterial {
            session_id: 100,
            key: [5; 32],
        },
        format,
        Arc::new(AtomicU64::new(0)),
        false,
    )
    .unwrap();
    let mut datagram = [0u8; MAX_DATAGRAM_LEN];
    let _ = receiver.recv_from(&mut datagram).unwrap();

    sender.expect_queue_stop();
    sender.resume_after_ack().unwrap();
    queue.stop_and_clear();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut sender = sender;
    let mut error = None;
    while error.is_none() && Instant::now() < deadline {
        error = sender.take_finished_error();
        std::thread::yield_now();
    }
    assert_eq!(
        error.as_deref(),
        Some("Float32 PCM queue stopped while the authenticated sender was active.")
    );
    assert!(sender.resume_after_ack().is_err());
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
