use super::*;

#[test]
fn loss_ledger_payload_matches_shared_java_vector() {
    let ledger = LossLedger {
        period_epoch: 0,
        revision: 7,
        capture_tail: 16,
        settled_through: 12,
        source_segment_generation: 0,
        source_segment_start: Some(0),
        complete: true,
        intervals: vec![
            LossInterval {
                start: 2,
                frame_count: 3,
            },
            LossInterval {
                start: 10,
                frame_count: 5,
            },
        ],
    };
    let expected = "03010200000000000000000000000000000000070000000000000010000000000000000c0000000000000000000000000000000000000000000000020000000000000003000000000000000a0000000000000005";
    let encoded = ledger.encode().unwrap();
    assert_eq!(
        encoded
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        expected
    );
    assert_eq!(LossLedger::decode(&encoded).unwrap(), ledger);

    let invalid = LossLedger {
        intervals: vec![LossInterval {
            start: 12,
            frame_count: 5,
        }],
        ..ledger
    };
    assert_eq!(invalid.encode().unwrap_err(), ProtocolError::InvalidFormat);
}

#[test]
fn loss_ledger_matches_anpd_v1_and_v2_java_packet_vectors() {
    let vector_key: [u8; 32] = std::array::from_fn(|index| index as u8);
    let vector_ledger = LossLedger {
        period_epoch: 0,
        revision: 7,
        capture_tail: 16,
        settled_through: 12,
        source_segment_generation: 0,
        source_segment_start: Some(0),
        complete: true,
        intervals: vec![
            LossInterval {
                start: 2,
                frame_count: 3,
            },
            LossInterval {
                start: 10,
                frame_count: 5,
            },
        ],
    };
    let legacy_payload: Vec<u8> = "02010200000000000000000000000000000000070000000000000010000000000000000c00000000000000020000000000000003000000000000000a0000000000000005"
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    let legacy_ledger = LossLedger::decode(&legacy_payload).unwrap();
    let v1_packet = PacketEncoder::new(&vector_key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER)
        .encode_at(
            0x1112_1314_1516_1718,
            PacketType::LossLedger,
            0,
            &legacy_payload,
        )
        .unwrap();
    assert_eq!(
        v1_packet.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "414e504401030000010203040506070811121314151617180000000000000000004439d358e23c1d8fd383c7563d031edfd12b1063d3f80313eec3aea4c10383b03b6f607fdc9ab3f8c9c25c8e55a8f4af8cef3ab8935778a88b531232564293eedaa6de282354f421be4179f1a3887ce761218d3455"
    );
    assert_eq!(
        PacketEncoder::new(&vector_key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER)
            .encode_at(
                0x1112_1314_1516_1718,
                PacketType::LossLedger,
                1,
                &vector_ledger.encode().unwrap(),
            )
            .unwrap_err(),
        ProtocolError::InvalidFormat
    );
    let v1_decoder = PacketDecoder::new(&vector_key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER);
    assert_eq!(
        LossLedger::decode(&v1_decoder.open(&v1_packet).unwrap().payload).unwrap(),
        legacy_ledger
    );

    let format = AudioFormatV2::for_profile(1, 0).unwrap();
    let vector_packet =
        PacketEncoder::new_v2(&vector_key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER, format)
            .unwrap()
            .encode_at(
                0x1112_1314_1516_1718,
                PacketType::LossLedger,
                0,
                &legacy_payload,
            )
            .unwrap();
    let expected_packet = "414e504402030000010203040506070811121314151617180000000000000000004439d358e23c1d8fd383c7563d031edfd12b1063d3f80313eec3aea4c10383b03b6f607fdc9ab3f8c9c25c8e55a8f4af8cef3ab8935778a88b531232564293eedaa6de282391c8fc7f73a59562fb3445bd3982eb58";
    assert_eq!(
        vector_packet
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        expected_packet
    );
    let decoder =
        PacketDecoder::new_v2(&vector_key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER, format)
            .unwrap();
    assert_eq!(
        LossLedger::decode(&decoder.open(&vector_packet).unwrap().payload).unwrap(),
        legacy_ledger
    );
}
#[test]
fn java_jca_aes_gcm_vector() {
    let key: [u8; 32] = std::array::from_fn(|index| index as u8);
    let session_id = 0x0102_0304_0506_0708;
    let sequence = 0x1112_1314_1516_1718;
    let frame_index = 0x2122_2324_2526_2728;
    let payload = [0x00, 0x01, 0x7f, 0x80, 0xfe, 0xff, 0x42];
    let encoder = PacketEncoder::new(&key, session_id, HOST_TO_RECEIVER);
    let packet = encoder
        .encode_at(sequence, PacketType::Format, frame_index, &payload)
        .unwrap();
    let expected = "414e50440101000001020304050607081112131415161718212223242526272800073bd32562c2e2cdfe093c22223a9705d1ae457629baad04";
    let actual = packet
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(actual, expected);
    assert_eq!(&packet[..4], b"ANPD");
    assert_eq!(packet.len(), HEADER_LEN + payload.len() + TAG_LEN);
    let decoded = PacketDecoder::new(&key, session_id, HOST_TO_RECEIVER)
        .open(&packet)
        .unwrap();
    assert_eq!(decoded.payload, payload);
    let reverse = PacketEncoder::with_sequence(&key, session_id, RECEIVER_TO_HOST, sequence)
        .encode_at(sequence, PacketType::Format, frame_index, &payload)
        .unwrap();
    assert_ne!(&packet[HEADER_LEN..], &reverse[HEADER_LEN..]);
}

#[test]
fn rejects_invalid_header_fields_and_length() {
    let key = [0x42; 32];
    let valid = PacketEncoder::new(&key, 7, HOST_TO_RECEIVER)
        .encode_at(0, PacketType::Format, 0, &[1, 2, 3, 4, 5, 6, 7])
        .unwrap();
    let decoder = PacketDecoder::new(&key, 7, HOST_TO_RECEIVER);

    let mut bad = valid.clone();
    bad[4] = 2;
    assert_eq!(
        decoder.open(&bad).unwrap_err(),
        ProtocolError::UnsupportedVersion
    );
    let mut bad = valid.clone();
    bad[5] = 4;
    assert_eq!(
        decoder.open(&bad).unwrap_err(),
        ProtocolError::UnsupportedType
    );
    let mut bad = valid.clone();
    bad[6] = 1;
    assert_eq!(
        decoder.open(&bad).unwrap_err(),
        ProtocolError::UnsupportedFlags
    );
    let mut bad = valid.clone();
    bad.pop();
    assert_eq!(decoder.open(&bad).unwrap_err(), ProtocolError::BadLength);
    let mut bad = valid;
    bad[8] ^= 1;
    assert_eq!(decoder.open(&bad).unwrap_err(), ProtocolError::WrongSession);
}

fn v2_vectors() -> std::collections::HashMap<String, Vec<u8>> {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/audio-v2-vectors.txt"
    ))
    .unwrap()
    .lines()
    .filter_map(|line| {
        let (key, value) = line.split_once('=')?;
        Some((
            key.to_owned(),
            (0..value.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
                .collect(),
        ))
    })
    .collect()
}

#[test]
fn v2_packet_matches_shared_vectors_and_rejects_cross_version_and_bad_format() {
    let expected = v2_vectors();
    let key: [u8; 32] = core::array::from_fn(|i| i as u8);
    let format = AudioFormatV2::for_profile(2, 0).unwrap();
    assert_eq!(format.encode().unwrap().as_slice(), expected["descriptor"]);
    let encoder =
        PacketEncoder::new_v2(&key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER, format).unwrap();
    let packet = encoder
        .encode_at(
            0x1112_1314_1516_1718,
            PacketType::Format,
            0,
            &format.encode().unwrap(),
        )
        .unwrap();
    assert_eq!(&packet[..HEADER_LEN], expected["aad"]);
    assert_eq!(
        nonce(HOST_TO_RECEIVER, 0x1112_1314_1516_1718).as_slice(),
        expected["nonce"]
    );
    assert_eq!(packet, expected["format_packet"]);
    let decoder =
        PacketDecoder::new_v2(&key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER, format).unwrap();
    assert_eq!(
        decoder.open(&packet).unwrap().payload,
        format.encode().unwrap()
    );

    assert_eq!(
        PacketDecoder::new(&key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER)
            .open(&packet)
            .unwrap_err(),
        ProtocolError::UnsupportedVersion
    );
    let v1 = PacketEncoder::new(&key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER)
        .encode_at(7, PacketType::Format, 0, &[1, 0, 2, 0, 0xbb, 0x80, 0])
        .unwrap();
    assert_eq!(
        PacketDecoder::new_v2(&key, 0x0102_0304_0506_0708, HOST_TO_RECEIVER, format)
            .unwrap()
            .open(&v1)
            .unwrap_err(),
        ProtocolError::UnsupportedVersion
    );

    let standard = AudioFormatV2::for_profile(1, 0).unwrap();
    let mismatched = PacketEncoder::new_v2(&key, 9, HOST_TO_RECEIVER, standard)
        .unwrap()
        .encode_at(0, PacketType::Format, 0, &standard.encode().unwrap())
        .unwrap();
    assert_eq!(
        PacketDecoder::new_v2(&key, 9, HOST_TO_RECEIVER, format)
            .unwrap()
            .open(&mismatched)
            .unwrap_err(),
        ProtocolError::InvalidFormat
    );
}

#[test]
fn aoa_v2_packets_match_all_profiles_reject_lan_keys_and_replay() {
    let expected = std::fs::read_to_string(concat!(
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
                .collect::<Vec<_>>(),
        ))
    })
    .collect::<std::collections::HashMap<_, _>>();

    for profile in 1..=3 {
        let prefix = format!("profile.{profile}");
        let format = AudioFormatV2::for_profile(profile, 0).unwrap();
        let session_id = 0x0102_0304_0506_0708 + (profile as u64 - 1);
        let key: [u8; 32] = expected[&format!("{prefix}.key_h2r_audio")]
            .as_slice()
            .try_into()
            .unwrap();
        let mut decoder =
            PacketDecoder::new_v2(&key, session_id, HOST_TO_RECEIVER, format).unwrap();
        let format_packet = &expected[&format!("{prefix}.format_datagram")];
        let decoded_format = decoder.open(format_packet).unwrap();
        assert_eq!(decoded_format.payload, format.encode().unwrap());
        assert_eq!(decoded_format.header.sample_frame_index, 0);
        decoder.commit(0);
        let audio_packet = &expected[&format!("{prefix}.audio_datagram")];
        let decoded_audio = decoder.open(audio_packet).unwrap();
        assert_eq!(decoded_audio.header.sample_frame_index, 0);
        assert_eq!(decoded_audio.payload.len() % format.frame_bytes as usize, 0);
        assert_eq!(
            decoded_audio.payload,
            expected[&format!("{prefix}.audio_payload")]
        );
        decoder.commit(1);
        assert_eq!(
            decoder.open(audio_packet).unwrap_err(),
            ProtocolError::Replay
        );

        let wrong_key: [u8; 32] = expected[&format!("{prefix}.wrong_domain_audio_key")]
            .as_slice()
            .try_into()
            .unwrap();
        assert_eq!(
            PacketDecoder::new_v2(&key, session_id, HOST_TO_RECEIVER, format)
                .unwrap()
                .open(&expected[&format!("{prefix}.wrong_domain_audio_datagram")])
                .unwrap_err(),
            ProtocolError::Authentication
        );
        assert_ne!(key, wrong_key);

        assert_eq!(
            PacketEncoder::new_v2(&key, session_id, HOST_TO_RECEIVER, format)
                .unwrap()
                .encode_at(2, PacketType::Audio, 0, &[0]),
            Err(ProtocolError::InvalidFormat)
        );
    }
}

#[test]
fn v2_descriptors_roundtrip_and_reject_unknown_or_reserved_fields() {
    for profile in 1..=5 {
        for mode in 0..=1 {
            let format = AudioFormatV2::for_profile(profile, mode).unwrap();
            assert_eq!(
                AudioFormatV2::decode(&format.encode().unwrap()).unwrap(),
                format
            );
        }
    }
    let valid = AudioFormatV2::for_profile(2, 0).unwrap().encode().unwrap();
    for (index, value) in [(0, 1), (1, 9), (2, 9), (7, 1), (14, 1)] {
        let mut bad = valid;
        bad[index] = value;
        assert_eq!(
            AudioFormatV2::decode(&bad).unwrap_err(),
            ProtocolError::InvalidFormat
        );
    }
    for (profile, expected) in [
        (4, [2, 4, 3, 2, 32, 4, 0, 0, 0, 0, 0xbb, 0x80, 0, 8, 0, 0]),
        (5, [2, 5, 3, 2, 32, 4, 0, 0, 0, 1, 0x77, 0, 0, 8, 0, 0]),
    ] {
        let format = AudioFormatV2::for_profile(profile, 0).unwrap();
        assert_eq!(format.encode().unwrap(), expected);
    }
    assert!(AudioFormatV2::for_profile(6, 0).is_err());
}

#[test]
fn v2_packet_validation_does_not_commit_failed_sequences_and_enforces_frame_limits() {
    let key = [0x42; 32];
    let format = AudioFormatV2::for_profile(2, 0).unwrap();
    let mut decoder = PacketDecoder::new_v2(&key, 7, HOST_TO_RECEIVER, format).unwrap();
    let encoder = PacketEncoder::new_v2(&key, 7, HOST_TO_RECEIVER, format).unwrap();
    let payload = [1, 2, 3, 4, 5, 6];
    let good = encoder
        .encode_at(4, PacketType::Audio, 10, &payload)
        .unwrap();
    let mut bad_tag = good.clone();
    *bad_tag.last_mut().unwrap() ^= 1;
    assert_eq!(
        decoder.open(&bad_tag).unwrap_err(),
        ProtocolError::Authentication
    );
    assert_eq!(decoder.open(&good).unwrap().payload, payload);
    decoder.commit(4);
    assert_eq!(decoder.open(&good).unwrap_err(), ProtocolError::Replay);

    let invalid_header = Header {
        packet_type: PacketType::Audio,
        session_id: 7,
        sequence: 5,
        sample_frame_index: 11,
        payload_len: 5,
    }
    .encode_version(2);
    let nonce_bytes = nonce(HOST_TO_RECEIVER, 5);
    let nonce = Nonce::try_from(nonce_bytes.as_slice()).unwrap();
    let invalid_payload = [1, 2, 3, 4, 5];
    let invalid_ciphertext = Aes256Gcm::new_from_slice(&key)
        .unwrap()
        .encrypt(
            &nonce,
            Payload {
                msg: &invalid_payload,
                aad: &invalid_header,
            },
        )
        .unwrap();
    let mut invalid_packet = invalid_header.to_vec();
    invalid_packet.extend_from_slice(&invalid_ciphertext);
    assert_eq!(
        decoder.open(&invalid_packet).unwrap_err(),
        ProtocolError::InvalidFormat
    );
    let aligned_after_failure = encoder
        .encode_at(5, PacketType::Audio, 11, &[1, 2, 3, 4, 5, 6])
        .unwrap();
    assert_eq!(
        decoder.open(&aligned_after_failure).unwrap().payload,
        [1, 2, 3, 4, 5, 6]
    );
    let largest_aligned = vec![0x55; MAX_PAYLOAD_LEN / 6 * 6];
    assert_eq!(largest_aligned.len(), 1146);
    let max_packet = encoder
        .encode_at(6, PacketType::Audio, 12, &largest_aligned)
        .unwrap();
    assert_eq!(
        max_packet.len(),
        HEADER_LEN + largest_aligned.len() + TAG_LEN
    );
    assert_eq!(
        encoder
            .encode_at(7, PacketType::Audio, 12, &vec![0; 1152])
            .unwrap_err(),
        ProtocolError::Oversize
    );
}
