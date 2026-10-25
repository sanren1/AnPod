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

fn vector(name: &str) -> Vec<u8> {
    let value = include_str!("../../tests/aoa-v1-vectors.txt")
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("missing vector {name}"));
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
        .collect()
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

fn vectors() -> std::collections::HashMap<String, Vec<u8>> {
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
fn v2_security_matches_shared_vectors_and_completes_mutual_handshake() {
    let mut fixture = BindingMaterial {
        host_id: core::array::from_fn(|i| 0x10 + i as u8),
        receiver_id: core::array::from_fn(|i| 0x20 + i as u8),
        secret: core::array::from_fn(|i| i as u8),
    };
    let host_nonce = core::array::from_fn(|i| 0x30 + i as u8);
    let receiver_nonce = core::array::from_fn(|i| 0x50 + i as u8);
    let format = crate::protocol::AudioFormatV2::for_profile(2, 0).unwrap();
    let expected = vectors();
    let descriptor = format.encode().unwrap();
    let transcript = transcript_v2(
        &fixture,
        &host_nonce,
        &receiver_nonce,
        0x0102_0304_0506_0708,
        format,
    )
    .unwrap();
    let host_proof = hmac(
        &fixture.secret,
        b"AnPod P5-A proof host v2",
        &transcript,
        &[],
    );
    let receiver_proof = hmac(
        &fixture.secret,
        b"AnPod P5-A proof receiver v2",
        &transcript,
        &[],
    );
    let ready_proof = hmac(
        &fixture.secret,
        b"AnPod P5-A ready v2",
        &transcript,
        &[host_proof.as_slice(), receiver_proof.as_slice()].concat(),
    );
    let keys = derive_v2_keys(&fixture.secret, &transcript);
    assert_eq!(descriptor.as_slice(), expected["descriptor"]);
    assert_eq!(transcript, expected["transcript"]);
    assert_eq!(host_proof.as_slice(), expected["host_proof"]);
    assert_eq!(receiver_proof.as_slice(), expected["receiver_proof"]);
    assert_eq!(ready_proof.as_slice(), expected["ready_proof"]);
    assert_eq!(
        keys.host_to_receiver_audio.as_slice(),
        expected["key_h2r_audio"]
    );
    assert_eq!(
        keys.receiver_to_host_audio.as_slice(),
        expected["key_r2h_audio"]
    );
    assert_eq!(
        keys.host_to_receiver_control.as_slice(),
        expected["key_h2r_control"]
    );
    assert_eq!(
        keys.receiver_to_host_control.as_slice(),
        expected["key_r2h_control"]
    );
    let session = 0x0102_0304_0506_0708u64.to_be_bytes();
    assert_eq!(
        v2_message(
            HELLO,
            96,
            &[
                &fixture.host_id,
                &fixture.receiver_id,
                &host_nonce,
                &session,
                &descriptor
            ]
        ),
        expected["hello"]
    );
    assert_eq!(
        v2_message(
            CHALLENGE,
            104,
            &[
                &fixture.receiver_id,
                &receiver_nonce,
                &descriptor,
                &receiver_proof
            ]
        ),
        expected["challenge"]
    );
    assert_eq!(v2_message(FINISH, 40, &[&host_proof]), expected["finish"]);
    assert_eq!(v2_message(READY, 40, &[&ready_proof]), expected["ready"]);
    fixture.secret.fill(0);

    let format = crate::protocol::AudioFormatV2::for_profile(2, 0).unwrap();
    let (host, hello) = HostHandshakeV2::start(material(), format).unwrap();
    let (receiver, challenge) = ReceiverHandshakeV2::accept_hello(material(), &hello).unwrap();
    let (host, finish) = host.accept_challenge(&challenge).unwrap();
    let (ready, receiver_session) = receiver.accept_finish(&finish).unwrap();
    let host_session = host.accept_ready(&ready).unwrap();
    assert_eq!(host_session.session_id, receiver_session.session_id);
    assert_eq!(host_session.format, receiver_session.format);
    let host_keys = host_session.into_keys();
    let receiver_keys = receiver_session.into_keys();
    assert_eq!(
        host_keys.host_to_receiver_audio,
        receiver_keys.host_to_receiver_audio
    );
    assert_eq!(
        host_keys.receiver_to_host_control,
        receiver_keys.receiver_to_host_control
    );
    assert_ne!(
        host_keys.host_to_receiver_audio,
        host_keys.host_to_receiver_control
    );
}

#[test]
fn v2_handshake_rejects_version_length_identity_selection_and_stale_proofs() {
    let format = crate::protocol::AudioFormatV2::for_profile(2, 0).unwrap();
    let (host, hello) = HostHandshakeV2::start(material(), format).unwrap();
    let mut wrong_version = hello.clone();
    wrong_version[4] = 1;
    assert!(matches!(
        ReceiverHandshakeV2::accept_hello(material(), &wrong_version),
        Err(SecurityError::Malformed)
    ));
    let mut wrong_length = hello.clone();
    wrong_length[7] = 95;
    assert!(matches!(
        ReceiverHandshakeV2::accept_hello(material(), &wrong_length),
        Err(SecurityError::Malformed)
    ));
    let mut wrong_id = hello.clone();
    wrong_id[8] ^= 1;
    assert!(matches!(
        ReceiverHandshakeV2::accept_hello(material(), &wrong_id),
        Err(SecurityError::WrongIdentity)
    ));
    let (receiver, challenge) = ReceiverHandshakeV2::accept_hello(material(), &hello).unwrap();
    let mut changed_selection = challenge.clone();
    changed_selection[56..72].copy_from_slice(
        &crate::protocol::AudioFormatV2::for_profile(1, 0)
            .unwrap()
            .encode()
            .unwrap(),
    );
    assert!(matches!(
        host.accept_challenge(&changed_selection),
        Err(SecurityError::InvalidParameters)
    ));
    drop(receiver);

    let (host, hello) = HostHandshakeV2::start(material(), format).unwrap();
    let (receiver, challenge) = ReceiverHandshakeV2::accept_hello(material(), &hello).unwrap();
    let mut bad_proof = challenge;
    bad_proof[72] ^= 1;
    assert!(matches!(
        host.accept_challenge(&bad_proof),
        Err(SecurityError::InvalidProof)
    ));
    drop(receiver);

    let (host, hello) = HostHandshakeV2::start(material(), format).unwrap();
    let mut wrong_secret = material();
    wrong_secret.secret[0] ^= 1;
    let (_, challenge) = ReceiverHandshakeV2::accept_hello(wrong_secret, &hello).unwrap();
    assert!(matches!(
        host.accept_challenge(&challenge),
        Err(SecurityError::InvalidProof)
    ));

    let (host, hello) = HostHandshakeV2::start(material(), format).unwrap();
    let (_, old_challenge) = ReceiverHandshakeV2::accept_hello(material(), &hello).unwrap();
    let (new_host, _) = HostHandshakeV2::start(material(), format).unwrap();
    assert!(new_host.accept_challenge(&old_challenge).is_err());
    drop(host);

    let (host, hello) = HostHandshakeV2::start(material(), format).unwrap();
    let (receiver, challenge) = ReceiverHandshakeV2::accept_hello(material(), &hello).unwrap();
    let (host, finish) = host.accept_challenge(&challenge).unwrap();
    let (ready, _) = receiver.accept_finish(&finish).unwrap();
    let mut tampered = ready.to_vec();
    tampered[8] ^= 1;
    assert!(host.accept_ready(&tampered).is_err());
}

#[test]
fn generated_binding_material_is_fresh() {
    let first = BindingMaterial::generate().unwrap();
    let second = BindingMaterial::generate().unwrap();
    assert_ne!(first.host_id, second.host_id);
    assert_ne!(first.receiver_id, second.receiver_id);
    assert_ne!(first.secret, second.secret);
}

#[test]
fn aoa_binding_derivation_matches_fixed_vector_and_rejects_lan_proof() {
    let mut material = BindingMaterial {
        host_id: [0; 16],
        receiver_id: [0; 16],
        secret: [0; 32],
    };
    material.host_id.copy_from_slice(&vector("binding.host_id"));
    material
        .receiver_id
        .copy_from_slice(&vector("binding.receiver_id"));
    material
        .secret
        .copy_from_slice(&vector("binding.synthetic_secret"));

    let derived = material.derive_aoa_transport();
    assert_eq!(derived.host_id, material.host_id);
    assert_eq!(derived.receiver_id, material.receiver_id);
    assert_eq!(derived.secret.as_slice(), vector("aoa.binding_secret"));
    assert_ne!(derived.secret, material.secret);

    for profile in 1..=3 {
        let prefix = format!("profile.{profile}");
        let transcript = vector(&format!("{prefix}.transcript"));
        let host_proof = hmac(
            &derived.secret,
            b"AnPod P5-A proof host v2",
            &transcript,
            &[],
        );
        let receiver_proof = hmac(
            &derived.secret,
            b"AnPod P5-A proof receiver v2",
            &transcript,
            &[],
        );
        let mut proof_pair = [0; 64];
        proof_pair[..32].copy_from_slice(&host_proof);
        proof_pair[32..].copy_from_slice(&receiver_proof);
        let ready = hmac(
            &derived.secret,
            b"AnPod P5-A ready v2",
            &transcript,
            &proof_pair,
        );
        assert_eq!(
            hex(&host_proof),
            hex(&vector(&format!("{prefix}.host_proof")))
        );
        assert_eq!(
            hex(&receiver_proof),
            hex(&vector(&format!("{prefix}.receiver_proof")))
        );
        assert_eq!(hex(&ready), hex(&vector(&format!("{prefix}.ready_proof"))));
        assert!(verify_hmac(
            &derived.secret,
            b"AnPod P5-A proof receiver v2",
            &transcript,
            &[],
            &vector(&format!("{prefix}.wrong_domain_receiver_proof")),
        )
        .is_err());

        let keys = derive_v2_keys(&derived.secret, &transcript);
        for (actual, name) in [
            (keys.host_to_receiver_audio, "key_h2r_audio"),
            (keys.receiver_to_host_audio, "key_r2h_audio"),
            (keys.host_to_receiver_control, "key_h2r_control"),
            (keys.receiver_to_host_control, "key_r2h_control"),
        ] {
            assert_eq!(hex(&actual), hex(&vector(&format!("{prefix}.{name}"))));
        }
    }

    let format = AudioFormatV2::for_profile(1, 0).unwrap();
    let (host, hello) = HostHandshakeV2::start(derived.duplicate_for_session(), format).unwrap();
    let (receiver, challenge) =
        ReceiverHandshakeV2::accept_hello(derived.duplicate_for_session(), &hello).unwrap();
    let (host, finish) = host.accept_challenge(&challenge).unwrap();
    let (ready, receiver_session) = receiver.accept_finish(&finish).unwrap();
    let host_session = host.accept_ready(&ready).unwrap();
    assert_eq!(host_session.format, receiver_session.format);
    assert_eq!(host_session.session_id, receiver_session.session_id);
}

#[test]
fn adb_binding_derivation_matches_independent_fixed_vector_and_is_domain_separated() {
    let material = BindingMaterial {
        host_id: [0x01; 16],
        receiver_id: [0x02; 16],
        secret: [0x03; 32],
    };
    let adb = material.derive_adb_stream();
    assert_eq!(adb.host_id, material.host_id);
    assert_eq!(adb.receiver_id, material.receiver_id);
    assert_eq!(
        hex(&adb.secret),
        "cb76cb157e4c5446d03232534f61486a750cb61ff29ccf8882b09c3ffe1d0c37"
    );
    assert_ne!(adb.secret, material.derive_aoa_transport().secret);
    assert_ne!(adb.secret, material.secret);
}
