use super::*;
use crate::security::test_adb_binding_material;
use std::thread;

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tablet = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (host, _) = listener.accept().unwrap();
    (host, tablet)
}

fn hello(token: [u8; 16], has_binding: u8) -> Vec<u8> {
    let mut body = token.to_vec();
    body.push(has_binding);
    encode_anhb(TABLET_HELLO, &body)
}

#[test]
fn pairing_code_matches_tablet_vector() {
    let material = test_adb_binding_material();
    assert_eq!(material.pairing_code(&[4; 16], &[5; 16]), "017647");
}

#[test]
fn harmony_binding_reference_vector() {
    let key = "FMR0223C13000649";
    let reference = BindingMaterial::harmony_binding_reference(key);
    assert_eq!(
        reference,
        "a31506dc80153256e12fa70292c4750108dc7f619075f9e7adcc13ad00dfa73e"
    );
    #[cfg(windows)]
    assert_ne!(reference, crate::receiver::binding_reference_for(key, "0"));
}

#[test]
fn host_bootstrap_ignores_wrong_token_then_accepts_right_one() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let tablet = thread::spawn(move || {
        let mut wrong = TcpStream::connect(address).unwrap();
        wrong.write_all(&hello([9; 16], 0)).unwrap();
        // The host must close the impostor.
        let mut byte = [0u8; 1];
        assert!(matches!(wrong.read(&mut byte), Ok(0) | Err(_)));
        let mut right = TcpStream::connect(address).unwrap();
        right.write_all(&hello([7; 16], 1)).unwrap();
        right
    });
    let (_stream, has_binding) = accept_with_token(
        &listener,
        [7; 16],
        Instant::now() + Duration::from_secs(10),
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(has_binding);
    tablet.join().unwrap();
}

#[test]
fn host_bootstrap_times_out_without_right_token() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let result = accept_with_token(
        &listener,
        [7; 16],
        Instant::now() + Duration::from_millis(150),
        &CancellationToken::new(),
    );
    assert!(result.is_err());
}

#[test]
fn pairing_declined_on_pc_stores_nothing() {
    let (mut host, mut tablet) = pair();
    let tablet = thread::spawn(move || {
        let (kind, body) = read_anhb(&mut tablet).unwrap();
        assert_eq!((kind, body.len()), (PAIR_OFFER, 69 + 16));
        tablet
            .write_all(&encode_anhb(PAIR_NONCE, &[5; 16]))
            .unwrap();
        read_anhb(&mut tablet).unwrap()
    });
    let material = test_adb_binding_material();
    let mut shown = String::new();
    let result = host_pair(&mut host, &material, |code| {
        shown = code.to_string();
        false
    });
    assert!(result.is_err());
    assert_eq!(shown.len(), 6);
    assert_eq!(tablet.join().unwrap(), (PAIR_CANCEL, vec![1]));
}

#[test]
fn pairing_accepted_when_commitment_matches() {
    let (mut host, mut tablet) = pair();
    let material = test_adb_binding_material();
    let commitment = material.commitment();
    let tablet = thread::spawn(move || {
        read_anhb(&mut tablet).unwrap();
        tablet
            .write_all(&encode_anhb(PAIR_NONCE, &[5; 16]))
            .unwrap();
        assert_eq!(read_anhb(&mut tablet).unwrap().0, PAIR_CONFIRMED);
        tablet
            .write_all(&encode_anhb(PAIR_SAVED, &commitment))
            .unwrap();
    });
    host_pair(&mut host, &material, |_| true).unwrap();
    tablet.join().unwrap();
}

#[test]
fn pairing_commitment_mismatch_rejected() {
    let (mut host, mut tablet) = pair();
    let tablet = thread::spawn(move || {
        read_anhb(&mut tablet).unwrap();
        tablet
            .write_all(&encode_anhb(PAIR_NONCE, &[5; 16]))
            .unwrap();
        read_anhb(&mut tablet).unwrap();
        tablet
            .write_all(&encode_anhb(PAIR_SAVED, &[0xAA; 32]))
            .unwrap();
    });
    let material = test_adb_binding_material();
    assert!(host_pair(&mut host, &material, |_| true).is_err());
    tablet.join().unwrap();
}

#[test]
fn proceed_frame_is_empty_kind_7() {
    let (mut host, mut tablet) = pair();
    host_proceed(&mut host).unwrap();
    assert_eq!(read_anhb(&mut tablet).unwrap(), (PROCEED, vec![]));
    assert_eq!(encode_anhb(PROCEED, &[]), b"ANHB\x01\x07\x00\x00");
}

#[test]
fn binding_profile_without_platform_defaults_to_android() {
    let profile: crate::state::BindingProfile =
        serde_json::from_str(r#"{"model":"Pixel 8","versionCode":50}"#).unwrap();
    assert_eq!(profile.platform, "android");
    assert_eq!(crate::state::BindingProfile::default().platform, "android");
}
