use super::*;
use crate::network_interface::interface_for_test;

#[test]
fn selected_interface_listener_correlates_single_use_connection_offers() {
    let host_address = Ipv4Addr::new(127, 0, 0, 2);
    let receiver_address = Ipv4Addr::new(127, 0, 0, 1);
    let interface = interface_for_test(host_address, 24);
    let mut listener = HostOfferListener::start_loopback_for_test(&interface).unwrap();
    let receiver = UdpSocket::bind(SocketAddr::from((receiver_address, DISCOVERY_PORT))).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let nonce = [0xA7; 16];
    receiver
        .send_to(
            &encode_host_probe(nonce),
            SocketAddr::from((host_address, DISCOVERY_PORT)),
        )
        .unwrap();
    let mut response = [0; 64];
    let (length, source) = receiver.recv_from(&mut response).unwrap();
    assert_eq!(source.ip(), host_address);
    assert_ne!(
        source.port(),
        0,
        "candidate reply uses an ephemeral selected-interface port"
    );
    assert_eq!(
        response[..length],
        encode_host_candidate(nonce, DISCOVERY_PORT)
    );

    receiver
        .send_to(
            &encode_connect_offer([0xA8; 16], 41_001),
            SocketAddr::from((host_address, DISCOVERY_PORT)),
        )
        .unwrap();
    thread::sleep(Duration::from_millis(80));
    assert!(
        listener.snapshot().offers.is_empty(),
        "unrelated nonce must not create a connection request"
    );
    receiver
        .send_to(
            &encode_connect_offer(nonce, 41_001),
            SocketAddr::from((host_address, DISCOVERY_PORT)),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let offer = loop {
        let snapshot = listener.snapshot();
        if let Some(offer) = snapshot.offers.into_iter().next() {
            break offer;
        }
        assert!(
            Instant::now() < deadline,
            "correlated connection request was not captured"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(offer.endpoint, SocketAddr::from((receiver_address, 41_001)));
    assert_eq!(HOST_OFFER_TTL, Duration::from_secs(30));
    assert!(offer.expires_at.duration_since(Instant::now()) > HOST_REQUEST_TTL,
            "the offer window starts when CONNECT_OFFER arrives, separately from the search nonce window");
    assert!(listener.take_offer(&offer.id).is_some());
    assert!(
        listener.take_offer(&offer.id).is_none(),
        "a connection request is accepted at most once"
    );
    {
        let mut current = listener.snapshot.lock().unwrap();
        current.offers.push(ConnectionOffer {
            nonce: [1; 16],
            id: "expired".into(),
            endpoint: SocketAddr::from((receiver_address, 41_002)),
            expires_at: Instant::now() - Duration::from_millis(1),
        });
    }
    assert!(
        listener
            .snapshot()
            .offers
            .iter()
            .all(|item| item.id != "expired"),
        "expired offers must disappear from snapshots"
    );
    assert!(
        listener.take_offer("expired").is_none(),
        "expired offers cannot be accepted"
    );
    receiver
        .send_to(
            &encode_connect_offer(nonce, 41_001),
            SocketAddr::from((host_address, DISCOVERY_PORT)),
        )
        .unwrap();
    thread::sleep(Duration::from_millis(80));
    assert!(
        listener.snapshot().offers.is_empty(),
        "a consumed nonce cannot replay a connection request"
    );
    let attempt = CancellationToken::new();
    listener.attempt_cancel = Some(attempt.clone());
    drop(listener);
    assert!(
        attempt.is_cancelled(),
        "stopping discovery cancels an in-flight authentication attempt"
    );
}

#[test]
fn authenticated_discovery_matches_the_shared_protocol_vector() {
    fn hex(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }
    let secret: [u8; 32] = std::array::from_fn(|index| index as u8);
    let nonce: [u8; 16] = std::array::from_fn(|index| index as u8);
    assert_eq!(binding_tag(&secret).to_vec(), hex("26414e36e4dc63e6"));
    let probe = encode_auth_probe(&secret, nonce);
    assert_eq!(
        probe[34..].to_vec(),
        hex("83ec229f58b0088dd60051cb736e6db8")
    );
    assert_eq!(probe[26..34].to_vec(), hex("26414e36e4dc63e6"));

    let mut candidate = [0u8; AUTH_FRAME_BYTES];
    candidate[..4].copy_from_slice(MAGIC);
    candidate[4] = AUTH_VERSION;
    candidate[5] = AUTH_CANDIDATE;
    candidate[8..24].copy_from_slice(&nonce);
    candidate[24..26].copy_from_slice(&41_234u16.to_be_bytes());
    candidate[26..34].copy_from_slice(&binding_tag(&secret));
    candidate[34..].copy_from_slice(&hex("a82f6568b5e977d37c46b22155edcba4"));
    assert_eq!(
        decode_auth_candidate(&candidate, &secret, &nonce),
        Some(41_234)
    );
    candidate[40] ^= 1;
    assert_eq!(
        decode_auth_candidate(&candidate, &secret, &nonce),
        None,
        "a tampered MAC is rejected"
    );
    assert_eq!(
        decode_auth_candidate(&candidate, &[9u8; 32], &nonce),
        None,
        "another binding cannot verify it"
    );
}

#[test]
fn probe_schedule_tries_the_cached_address_before_broadcasting() {
    let interface = interface_for_test(Ipv4Addr::new(192, 168, 1, 10), 24);
    let targets = probe_targets(&interface);
    let cached = Ipv4Addr::new(192, 168, 1, 23);
    let steps = probe_schedule(&interface, &targets, Some(cached));
    assert_eq!(
        steps[0],
        ProbeStep {
            target: cached,
            at: Duration::ZERO
        },
        "the cached address is probed first"
    );
    assert_eq!(
        steps.iter().filter(|step| step.target == cached).count(),
        2,
        "the unicast round is retried once"
    );
    let broadcast: Vec<_> = steps
        .iter()
        .filter(|step| targets.contains(&step.target))
        .collect();
    assert_eq!(broadcast.len(), 3, "broadcasts repeat three times");
    assert!(
        broadcast
            .iter()
            .all(|step| step.at >= Duration::from_millis(250)),
        "broadcast waits for the unicast round"
    );
    assert!(
        broadcast.windows(2).all(|pair| pair[0].at < pair[1].at),
        "broadcast rounds are spaced out"
    );
}

#[test]
fn probe_schedule_ignores_a_cached_address_outside_the_interface() {
    let interface = interface_for_test(Ipv4Addr::new(192, 168, 1, 10), 24);
    let targets = probe_targets(&interface);
    for unusable in [
        Ipv4Addr::new(10, 0, 0, 5),
        Ipv4Addr::new(192, 168, 1, 10),
        Ipv4Addr::BROADCAST,
    ] {
        let steps = probe_schedule(&interface, &targets, Some(unusable));
        assert!(
            steps.iter().all(|step| targets.contains(&step.target)),
            "{unusable} must not be probed"
        );
        assert_eq!(
            steps[0].at,
            Duration::ZERO,
            "broadcast starts immediately without a usable cache"
        );
    }
}
