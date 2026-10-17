
use super::*;

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
    bad[5] = 3;
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
