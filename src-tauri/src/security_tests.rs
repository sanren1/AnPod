use super::*;

fn material() -> BindingMaterial {
    BindingMaterial {
        host_id: [0x11; 16],
        receiver_id: [0x22; 16],
        secret: [0x33; 32],
    }
}

fn parameters() -> AudioParameters {
    AudioParameters {
        sample_rate: 48_000,
        channels: 2,
        mode: 0,
    }
}

fn complete() -> (AuthenticatedSession, AuthenticatedSession) {
    let (host, hello) = HostHandshake::start(material(), parameters()).unwrap();
    let (receiver, challenge) = ReceiverHandshake::accept_hello(material(), &hello).unwrap();
    let (host, finish) = host.accept_challenge(&challenge).unwrap();
    let (ready, receiver_session) = receiver.accept_finish(&finish).unwrap();
    let host_session = host.accept_ready(&ready).unwrap();
    (host_session, receiver_session)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn mutual_handshake_derives_identical_directional_keys_and_rebuilds_fresh() {
    let (host, receiver) = complete();
    assert_eq!(host.session_id(), receiver.session_id());
    assert_eq!(host.parameters(), receiver.parameters());
    let host_keys = host.into_keys();
    let receiver_keys = receiver.into_keys();
    assert_eq!(
        host_keys.host_to_receiver_audio,
        receiver_keys.host_to_receiver_audio
    );
    assert_eq!(
        host_keys.receiver_to_host_audio,
        receiver_keys.receiver_to_host_audio
    );
    assert_eq!(
        host_keys.host_to_receiver_control,
        receiver_keys.host_to_receiver_control
    );
    assert_eq!(
        host_keys.receiver_to_host_control,
        receiver_keys.receiver_to_host_control
    );
    assert_ne!(
        host_keys.host_to_receiver_audio,
        host_keys.host_to_receiver_control
    );
    let (next_host, next_receiver) = complete();
    assert_ne!(
        host_keys.host_to_receiver_audio,
        next_host.into_keys().host_to_receiver_audio
    );
    assert_ne!(
        receiver_keys.receiver_to_host_audio,
        next_receiver.into_keys().receiver_to_host_audio
    );
}

#[test]
fn bad_identity_role_proof_and_cross_session_response_are_rejected() {
    let (host, hello) = HostHandshake::start(material(), parameters()).unwrap();
    let (receiver, mut challenge) = ReceiverHandshake::accept_hello(material(), &hello).unwrap();
    challenge[22] ^= 1;
    assert!(matches!(
        host.accept_challenge(&challenge),
        Err(SecurityError::InvalidProof)
    ));
    drop(receiver);

    let (host_one, hello_one) = HostHandshake::start(material(), parameters()).unwrap();
    let (_, old_challenge) = ReceiverHandshake::accept_hello(material(), &hello_one).unwrap();
    let (host_two, _) = HostHandshake::start(material(), parameters()).unwrap();
    assert!(host_two.accept_challenge(&old_challenge).is_err());
    drop(host_one);

    let (host, hello) = HostHandshake::start(material(), parameters()).unwrap();
    let (receiver, challenge) = ReceiverHandshake::accept_hello(material(), &hello).unwrap();
    let mut wrong_identity = challenge.clone();
    wrong_identity[6] ^= 1;
    assert!(matches!(
        host.accept_challenge(&wrong_identity),
        Err(SecurityError::WrongIdentity)
    ));
    let mut wrong_identity_hello = hello.clone();
    wrong_identity_hello[6] ^= 1;
    assert!(matches!(
        ReceiverHandshake::accept_hello(material(), &wrong_identity_hello),
        Err(SecurityError::WrongIdentity)
    ));
    let mut wrong_role = hello.clone();
    wrong_role[5] = FINISH;
    assert!(matches!(
        ReceiverHandshake::accept_hello(material(), &wrong_role),
        Err(SecurityError::WrongIdentity)
    ));
    drop(receiver);

    let (host, hello) = HostHandshake::start(material(), parameters()).unwrap();
    let (_, challenge) = ReceiverHandshake::accept_hello(material(), &hello).unwrap();
    let (host, _) = host.accept_challenge(&challenge).unwrap();
    let wrong_ready = encode_proof_message(READY, &[0u8; 32]);
    assert!(host.accept_ready(&wrong_ready).is_err());
}

#[test]
fn fixed_transcript_and_hkdf_vector_is_stable() {
    let mut value = BindingMaterial {
        host_id: core::array::from_fn(|index| 0x10 + index as u8),
        receiver_id: core::array::from_fn(|index| 0x20 + index as u8),
        secret: core::array::from_fn(|index| index as u8),
    };
    let host_nonce = core::array::from_fn(|index| 0x30 + index as u8);
    let receiver_nonce = core::array::from_fn(|index| 0x50 + index as u8);
    let params = parameters();
    assert_eq!(
        hex(&value.commitment()),
        "dcb02a268976be74a08d28d7124724dbeef4c130715d6544ad8b7eb74f79eda4"
    );
    let transcript = transcript(
        &value,
        &host_nonce,
        &receiver_nonce,
        0x0102_0304_0506_0708,
        params,
    );
    assert_eq!(hex(&transcript), "414e5035010102101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f010203040506070801020000bb8000");
    let proof_h = proof_host(&transcript, &value.secret);
    let proof_r = proof_receiver(&transcript, &value.secret);
    assert_eq!(
        hex(&proof_h),
        "2d28ffee9d17f1930898388aa09f4291dc47aba00f201ecdf58a00163388aaed"
    );
    assert_eq!(
        hex(&proof_r),
        "1b4af0b2591a958ddb36186bdccd8fd1cdfc6c2f599a48aabeea5a865a8482a6"
    );
    assert_eq!(
        hex(&ready_proof(&transcript, &proof_h, &proof_r, &value.secret)),
        "80f32dbf0a5c1a29ec777fd06090593869254a34bc05eca236ad8b47b440fa1c"
    );
    let session =
        AuthenticatedSession::new(0x0102_0304_0506_0708, params, &transcript, &value.secret);
    let keys = session.into_keys();
    assert_eq!(
        hex(&keys.host_to_receiver_audio),
        "00021dc01623e41fc2a636df8fb67b19bfc7d83bd0a762ef8759d149132fac58"
    );
    assert_eq!(
        hex(&keys.receiver_to_host_audio),
        "fdbf5d4561402f457655b436f24557461e523eecd4a3e56b3634f4c47e1ebc7a"
    );
    assert_eq!(
        hex(&keys.host_to_receiver_control),
        "7b307e8464b5860d3cacf168026bab443e1ddf98adbe87204d4ccdcd6c324050"
    );
    assert_eq!(
        hex(&keys.receiver_to_host_control),
        "4c9c03cc5cc1fb8e04f7fc3a710886f8c9833901ab90464184d5256319943711"
    );
    value.secret.fill(0);
}

#[test]
fn generated_binding_material_is_fresh() {
    let first = BindingMaterial::generate().unwrap();
    let second = BindingMaterial::generate().unwrap();
    assert_ne!(first.host_id, second.host_id);
    assert_ne!(first.receiver_id, second.receiver_id);
    assert_ne!(first.secret, second.secret);
}
