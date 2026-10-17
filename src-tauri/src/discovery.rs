use crate::network_interface::Ipv4Interface;
use getrandom::fill;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::{
    collections::VecDeque,
    io,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub(crate) const DISCOVERY_PORT: u16 = 49_772;
const MAGIC: &[u8; 4] = b"ANPD";
const VERSION: u8 = 1;
const PROBE: u8 = 1;
const CANDIDATE: u8 = 2;
const FRAME_BYTES: usize = 26;
const IO_POLL: Duration = Duration::from_millis(50);
const MAX_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CANDIDATES: usize = 16;
const V2_VERSION: u8 = 2;
const HOST_PROBE: u8 = 3;
const HOST_CANDIDATE: u8 = 4;
const CONNECT_OFFER: u8 = 5;
const HOST_PROBE_BYTES: usize = 24;
const HOST_PACKET_BYTES: usize = 26;
const HOST_REQUEST_TTL: Duration = Duration::from_secs(10);
const HOST_OFFER_TTL: Duration = Duration::from_secs(30);

// Authenticated discovery (v3). The tag identifies which binding a probe is for
// without revealing the binding identifiers; the MAC proves the sender holds the
// binding secret. Labels keep these MACs separate from every other use of it.
const AUTH_VERSION: u8 = 3;
const AUTH_PROBE: u8 = 6;
const AUTH_CANDIDATE: u8 = 7;
const AUTH_FRAME_BYTES: usize = 50;
const TAG_BYTES: usize = 8;
const MAC_BYTES: usize = 16;
const TAG_LABEL: &[u8] = b"anpod-discovery-tag-v3";
const PROBE_LABEL: &[u8] = b"anpod-probe-v3";
const CANDIDATE_LABEL: &[u8] = b"anpod-candidate-v3";

/// Binding material needed to authenticate a discovery exchange.
pub(crate) struct DiscoveryBinding {
    pub(crate) reference: String,
    pub(crate) secret: [u8; 32],
}

/// Unauthenticated network hint. Callers must authenticate it with P5-A before START.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Candidate {
    pub(crate) endpoint: SocketAddr,
    /// Set only when the reply carried a MAC for this binding.
    pub(crate) binding_reference: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionOffer {
    pub(crate) id: String,
    pub(crate) endpoint: SocketAddr,
    #[serde(skip)]
    expires_at: Instant,
    #[serde(skip)]
    pub(crate) nonce: [u8; 16],
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostDiscoverySnapshot {
    pub(crate) active: bool,
    pub(crate) automatic_binding: Option<String>,
    pub(crate) interface_address: Option<Ipv4Addr>,
    pub(crate) interface_prefix_length: Option<u8>,
    pub(crate) offers: Vec<ConnectionOffer>,
}

pub(crate) struct HostOfferListener {
    pub(crate) automatic_binding: Option<String>,
    pub(crate) attempt_cancel: Option<CancellationToken>,
    cancellation: CancellationToken,
    snapshot: Arc<Mutex<HostDiscoverySnapshot>>,
    worker: Option<JoinHandle<()>>,
}

impl HostOfferListener {
    pub(crate) fn start(interface: &Ipv4Interface) -> Result<Self, String> {
        Self::start_inner(interface, false)
    }

    fn start_inner(interface: &Ipv4Interface, allow_loopback: bool) -> Result<Self, String> {
        if !((interface.address.is_private() && !interface.address.is_link_local())
            || (allow_loopback && interface.address.is_loopback()))
            || interface.prefix_length == 0
            || interface.prefix_length > 32
        {
            return Err("Select one active private IPv4 interface before allowing wireless UDP connections.".into());
        }
        let socket = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)))
            .map_err(|_| {
                "Unable to listen for connection requests on the selected interface.".to_string()
            })?;
        socket
            .set_read_timeout(Some(IO_POLL))
            .map_err(|_| "Unable to configure selected-interface discovery polling.".to_string())?;
        let response_socket = UdpSocket::bind(SocketAddr::from((interface.address, 0)))
            .map_err(|_| "Unable to prepare replies on the selected interface.".to_string())?;
        let local = interface.address;
        let prefix = interface.prefix_length;
        let cancellation = CancellationToken::new();
        let worker_cancel = cancellation.clone();
        let snapshot = Arc::new(Mutex::new(HostDiscoverySnapshot {
            active: true,
            automatic_binding: None,
            interface_address: Some(local),
            interface_prefix_length: Some(prefix),
            offers: Vec::new(),
        }));
        let worker_snapshot = snapshot.clone();
        let worker = thread::Builder::new()
            .name("AnPod selected-interface connection offers".into())
            .spawn(move || {
                run_host_offer_listener(
                    socket,
                    response_socket,
                    local,
                    prefix,
                    worker_cancel,
                    worker_snapshot,
                )
            })
            .map_err(|error| {
                format!("Unable to start selected-interface offer listener: {error}")
            })?;
        Ok(Self {
            automatic_binding: None,
            attempt_cancel: None,
            cancellation,
            snapshot,
            worker: Some(worker),
        })
    }

    #[cfg(test)]
    pub(crate) fn start_loopback_for_test(interface: &Ipv4Interface) -> Result<Self, String> {
        Self::start_inner(interface, true)
    }

    pub(crate) fn snapshot(&self) -> HostDiscoverySnapshot {
        let mut snapshot = self
            .snapshot
            .lock()
            .expect("host discovery snapshot lock poisoned");
        discard_expired_offers(&mut snapshot.offers);
        let mut result = snapshot.clone();
        result.automatic_binding = self.automatic_binding.clone();
        result
    }

    pub(crate) fn interface_address(&self) -> Option<Ipv4Addr> {
        self.snapshot
            .lock()
            .expect("host discovery snapshot lock poisoned")
            .interface_address
    }

    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub(crate) fn take_offer(&self, id: &str) -> Option<ConnectionOffer> {
        let mut snapshot = self
            .snapshot
            .lock()
            .expect("host discovery snapshot lock poisoned");
        discard_expired_offers(&mut snapshot.offers);
        let index = snapshot.offers.iter().position(|offer| offer.id == id)?;
        Some(snapshot.offers.remove(index))
    }
}

impl Drop for HostOfferListener {
    fn drop(&mut self) {
        if let Some(token) = self.attempt_cancel.take() {
            token.cancel();
        }
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() {
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
        let mut snapshot = self
            .snapshot
            .lock()
            .expect("host discovery snapshot lock poisoned");
        snapshot.active = false;
        snapshot.interface_address = None;
        snapshot.interface_prefix_length = None;
        snapshot.offers.clear();
    }
}

fn discard_expired_offers(offers: &mut Vec<ConnectionOffer>) {
    let now = Instant::now();
    offers.retain(|offer| offer.expires_at > now);
}

fn run_host_offer_listener(
    socket: UdpSocket,
    response_socket: UdpSocket,
    local: Ipv4Addr,
    prefix: u8,
    cancellation: CancellationToken,
    snapshot: Arc<Mutex<HostDiscoverySnapshot>>,
) {
    let mut pending_nonces: VecDeque<([u8; 16], Ipv4Addr, Instant)> = VecDeque::new();
    let mut used_nonces: VecDeque<([u8; 16], Instant)> = VecDeque::new();
    let mut bytes = [0; 128];
    while !cancellation.is_cancelled() {
        match socket.recv_from(&mut bytes) {
            Ok((length, source)) => {
                let SocketAddr::V4(source) = source else {
                    continue;
                };
                if source.port() != DISCOVERY_PORT || !ipv4_in_prefix(local, prefix, *source.ip()) {
                    continue;
                }
                let packet = &bytes[..length];
                if let Some(nonce) = decode_host_probe(packet) {
                    pending_nonces.retain(|(_, _, at)| at.elapsed() <= HOST_REQUEST_TTL);
                    used_nonces.retain(|(_, at)| at.elapsed() <= HOST_OFFER_TTL);
                    if used_nonces.iter().any(|(used, _)| used == &nonce) {
                        continue;
                    }
                    if !pending_nonces
                        .iter()
                        .any(|(pending, _, _)| pending == &nonce)
                    {
                        if pending_nonces.len() == MAX_CANDIDATES
                            || used_nonces.len() == MAX_CANDIDATES
                        {
                            continue;
                        }
                        pending_nonces.push_back((nonce, *source.ip(), Instant::now()));
                    }
                    let response = encode_host_candidate(nonce, DISCOVERY_PORT);
                    let _ = response_socket.send_to(&response, source);
                    continue;
                }
                let Some((nonce, port)) = decode_connect_offer(packet) else {
                    continue;
                };
                pending_nonces.retain(|(_, _, at)| at.elapsed() <= HOST_REQUEST_TTL);
                used_nonces.retain(|(_, at)| at.elapsed() <= HOST_OFFER_TTL);
                if port == 0 || used_nonces.iter().any(|(used, _)| used == &nonce) {
                    continue;
                }
                let Some(index) = pending_nonces
                    .iter()
                    .position(|(pending, address, _)| pending == &nonce && address == source.ip())
                else {
                    continue;
                };
                pending_nonces.remove(index).expect("pending nonce index");
                if used_nonces.len() == MAX_CANDIDATES {
                    continue;
                }
                let received_at = Instant::now();
                used_nonces.push_back((nonce, received_at));
                let offer = ConnectionOffer {
                    nonce,
                    id: format!("{}:{}:{}", source.ip(), port, hex_nonce(&nonce)),
                    endpoint: SocketAddr::new((*source.ip()).into(), port),
                    expires_at: received_at + HOST_OFFER_TTL,
                };
                let mut current = snapshot
                    .lock()
                    .expect("host discovery snapshot lock poisoned");
                discard_expired_offers(&mut current.offers);
                if current.active && !current.offers.iter().any(|item| item.id == offer.id) {
                    if current.offers.len() == MAX_CANDIDATES {
                        continue;
                    }
                    current.offers.push(offer);
                }
            }
            Err(error) if is_poll_timeout(&error) => {}
            Err(_) => break,
        }
    }
    let mut current = snapshot
        .lock()
        .expect("host discovery snapshot lock poisoned");
    current.active = false;
    current.interface_address = None;
    current.interface_prefix_length = None;
}

fn ipv4_in_prefix(local: Ipv4Addr, prefix: u8, remote: Ipv4Addr) -> bool {
    if prefix == 0 || prefix > 32 {
        return false;
    }
    let mask = if prefix == 32 {
        u32::MAX
    } else {
        u32::MAX << (32 - prefix)
    };
    u32::from(local) & mask == u32::from(remote) & mask
}

fn encode_host_probe(nonce: [u8; 16]) -> [u8; HOST_PROBE_BYTES] {
    let mut frame = [0; HOST_PROBE_BYTES];
    frame[..4].copy_from_slice(MAGIC);
    frame[4] = V2_VERSION;
    frame[5] = HOST_PROBE;
    frame[8..24].copy_from_slice(&nonce);
    frame
}

fn encode_host_candidate(nonce: [u8; 16], port: u16) -> [u8; HOST_PACKET_BYTES] {
    encode_host_packet(HOST_CANDIDATE, nonce, port)
}

pub(crate) fn encode_connect_offer(
    nonce: [u8; 16],
    receiver_control_port: u16,
) -> [u8; HOST_PACKET_BYTES] {
    encode_host_packet(CONNECT_OFFER, nonce, receiver_control_port)
}

fn encode_host_packet(kind: u8, nonce: [u8; 16], port: u16) -> [u8; HOST_PACKET_BYTES] {
    let mut frame = [0; HOST_PACKET_BYTES];
    frame[..4].copy_from_slice(MAGIC);
    frame[4] = V2_VERSION;
    frame[5] = kind;
    frame[8..24].copy_from_slice(&nonce);
    frame[24..].copy_from_slice(&port.to_be_bytes());
    frame
}

fn decode_host_probe(frame: &[u8]) -> Option<[u8; 16]> {
    if frame.len() != HOST_PROBE_BYTES
        || &frame[..4] != MAGIC
        || frame[4] != V2_VERSION
        || frame[5] != HOST_PROBE
        || frame[6..8] != [0, 0]
        || frame[8..24].iter().all(|byte| *byte == 0)
    {
        return None;
    }
    frame[8..24].try_into().ok()
}

fn decode_connect_offer(frame: &[u8]) -> Option<([u8; 16], u16)> {
    if frame.len() != HOST_PACKET_BYTES
        || &frame[..4] != MAGIC
        || frame[4] != V2_VERSION
        || frame[5] != CONNECT_OFFER
        || frame[6..8] != [0, 0]
        || frame[8..24].iter().all(|byte| *byte == 0)
    {
        return None;
    }
    Some((
        frame[8..24].try_into().ok()?,
        u16::from_be_bytes([frame[24], frame[25]]),
    ))
}

fn hmac_sha256(secret: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("HMAC accepts a 32-byte key");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

pub(crate) fn binding_tag(secret: &[u8; 32]) -> [u8; TAG_BYTES] {
    hmac_sha256(secret, &[TAG_LABEL])[..TAG_BYTES]
        .try_into()
        .expect("tag length")
}

fn probe_mac(secret: &[u8; 32], nonce: &[u8; 16], tag: &[u8; TAG_BYTES]) -> [u8; MAC_BYTES] {
    hmac_sha256(secret, &[PROBE_LABEL, nonce, tag])[..MAC_BYTES]
        .try_into()
        .expect("mac length")
}

fn candidate_mac(
    secret: &[u8; 32],
    nonce: &[u8; 16],
    tag: &[u8; TAG_BYTES],
    port: u16,
) -> [u8; MAC_BYTES] {
    hmac_sha256(secret, &[CANDIDATE_LABEL, nonce, tag, &port.to_be_bytes()])[..MAC_BYTES]
        .try_into()
        .expect("mac length")
}

pub(crate) fn encode_auth_probe(secret: &[u8; 32], nonce: [u8; 16]) -> [u8; AUTH_FRAME_BYTES] {
    let tag = binding_tag(secret);
    let mut frame = [0; AUTH_FRAME_BYTES];
    frame[..4].copy_from_slice(MAGIC);
    frame[4] = AUTH_VERSION;
    frame[5] = AUTH_PROBE;
    frame[8..24].copy_from_slice(&nonce);
    frame[26..34].copy_from_slice(&tag);
    frame[34..].copy_from_slice(&probe_mac(secret, &nonce, &tag));
    frame
}

/// Verifies an authenticated reply against one binding and returns its control port.
pub(crate) fn decode_auth_candidate(
    frame: &[u8],
    secret: &[u8; 32],
    nonce: &[u8; 16],
) -> Option<u16> {
    if frame.len() != AUTH_FRAME_BYTES
        || &frame[..4] != MAGIC
        || frame[4] != AUTH_VERSION
        || frame[5] != AUTH_CANDIDATE
        || frame[6..8] != [0, 0]
        || &frame[8..24] != nonce
    {
        return None;
    }
    let tag: [u8; TAG_BYTES] = frame[26..34].try_into().ok()?;
    if !constant_time_eq(&tag, &binding_tag(secret)) {
        return None;
    }
    let port = u16::from_be_bytes([frame[24], frame[25]]);
    if port == 0 {
        return None;
    }
    if !constant_time_eq(&candidate_mac(secret, nonce, &tag, port), &frame[34..]) {
        return None;
    }
    Some(port)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

fn hex_nonce(nonce: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    nonce
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 15) as usize] as char,
            ]
        })
        .collect()
}

pub(crate) fn probe_targets(interface: &Ipv4Interface) -> Vec<Ipv4Addr> {
    let mut targets = Vec::new();
    if let Some(gateway) = interface
        .gateways
        .iter()
        .find(|gateway| interface.contains(**gateway))
    {
        if !targets.contains(gateway) {
            targets.push(*gateway);
        }
    }
    if let Some(broadcast) = interface.directed_broadcast() {
        if !targets.contains(&broadcast) {
            targets.push(broadcast);
        }
    }
    targets
}

/// One probe in the discovery timeline.
#[derive(Debug, PartialEq, Eq)]
struct ProbeStep {
    target: Ipv4Addr,
    at: Duration,
}

/// Unicast to the last known address first, then fall back to the gateway and
/// directed broadcast. Broadcasts repeat three times because a single datagram
/// is routinely lost on Wi-Fi; the cached address is retried once before that.
fn probe_schedule(
    interface: &Ipv4Interface,
    targets: &[Ipv4Addr],
    cached: Option<Ipv4Addr>,
) -> Vec<ProbeStep> {
    let cached = cached.filter(|address| {
        *address != interface.address
            && interface.contains(*address)
            && !address.is_broadcast()
            && !address.is_unspecified()
    });
    let broadcast_start = if cached.is_some() {
        Duration::from_millis(250)
    } else {
        Duration::ZERO
    };
    let mut steps = Vec::new();
    if let Some(address) = cached {
        steps.push(ProbeStep {
            target: address,
            at: Duration::ZERO,
        });
        steps.push(ProbeStep {
            target: address,
            at: Duration::from_millis(250),
        });
    }
    for offset in [
        broadcast_start,
        broadcast_start + Duration::from_millis(350),
        broadcast_start + Duration::from_millis(700),
    ] {
        steps.extend(targets.iter().map(|target| ProbeStep {
            target: *target,
            at: offset,
        }));
    }
    steps.sort_by_key(|step| step.at);
    steps
}

/// Sends a correlated probe to the cached address first, then to the broadcast and/or
/// explicit gateway targets of the interface selected by the caller. This function is
/// never called implicitly.
pub(crate) fn discover_on_interface(
    interface: &Ipv4Interface,
    targets: &[Ipv4Addr],
    cached: Option<Ipv4Addr>,
    bindings: &[DiscoveryBinding],
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<Vec<Candidate>, String> {
    let local_valid = interface.address.is_loopback()
        || (interface.address.is_private() && !interface.address.is_link_local());
    if !local_valid || interface.prefix_length == 0 || interface.prefix_length > 32 {
        return Err(
            "Selected IPv4 interface details are unavailable or outside the private LAN scope."
                .into(),
        );
    }
    if timeout.is_zero() || timeout > MAX_DISCOVERY_TIMEOUT {
        return Err("IPv4 discovery is limited to a two-second cancellable window.".into());
    }
    if targets.is_empty() || targets.len() > 2 {
        return Err("Select one broadcast target and at most one explicit gateway target.".into());
    }
    let allowed_targets = probe_targets(interface);
    if targets
        .iter()
        .any(|target| !allowed_targets.contains(target))
    {
        return Err(
            "Discovery targets must be the selected interface's gateway or directed broadcast."
                .into(),
        );
    }
    let socket = UdpSocket::bind(SocketAddr::from((interface.address, 0)))
        .map_err(|_| "Selected network interface is unavailable for discovery.".to_string())?;
    socket
        .set_broadcast(true)
        .map_err(|_| "Unable to enable bounded UDP discovery broadcast.".to_string())?;
    socket
        .set_read_timeout(Some(IO_POLL))
        .map_err(|_| "Unable to configure discovery receive polling.".to_string())?;

    let mut token = [0u8; 16];
    fill(&mut token).map_err(|_| "Unable to create a fresh discovery probe token.".to_string())?;
    let probe = encode(PROBE, token, 0);
    let auth_probes: Vec<[u8; AUTH_FRAME_BYTES]> = bindings
        .iter()
        .map(|binding| encode_auth_probe(&binding.secret, token))
        .collect();
    let schedule = probe_schedule(interface, targets, cached);
    let started = Instant::now();
    let mut next_step = 0;

    let deadline = Instant::now() + timeout;
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut bytes = [0u8; 128];
    while Instant::now() < deadline {
        if cancellation.is_cancelled() {
            return Err("Network discovery was cancelled.".into());
        }
        while next_step < schedule.len() && started.elapsed() >= schedule[next_step].at {
            let target = SocketAddr::from((schedule[next_step].target, DISCOVERY_PORT));
            socket.send_to(&probe, target).map_err(|_| {
                "Unable to send a discovery probe on the selected interface.".to_string()
            })?;
            for frame in &auth_probes {
                socket.send_to(frame, target).map_err(|_| {
                    "Unable to send a discovery probe on the selected interface.".to_string()
                })?;
            }
            next_step += 1;
        }
        match socket.recv_from(&mut bytes) {
            Ok((length, source)) => {
                let SocketAddr::V4(source) = source else {
                    continue;
                };
                if source.port() == 0 || !interface.contains(*source.ip()) {
                    continue;
                }
                let frame = &bytes[..length];
                let (port, binding_reference) = match decode_candidate(frame, &token) {
                    Some(port) => (port, None),
                    None => match bindings.iter().find_map(|binding| {
                        decode_auth_candidate(frame, &binding.secret, &token)
                            .map(|port| (port, Some(binding.reference.clone())))
                    }) {
                        Some(found) => found,
                        None => continue,
                    },
                };
                let endpoint = SocketAddr::new((*source.ip()).into(), port);
                match candidates.iter_mut().find(|item| item.endpoint == endpoint) {
                    // An authenticated reply upgrades a candidate first seen unverified.
                    Some(existing) if existing.binding_reference.is_none() => {
                        existing.binding_reference = binding_reference
                    }
                    Some(_) => {}
                    None => {
                        candidates.push(Candidate {
                            endpoint,
                            binding_reference,
                        });
                        if candidates.len() == MAX_CANDIDATES {
                            break;
                        }
                    }
                }
            }
            Err(error) if is_poll_timeout(&error) => {}
            Err(_) if cancellation.is_cancelled() => {
                return Err("Network discovery was cancelled.".into())
            }
            // Windows reports ICMP port-unreachable for an earlier UDP response as a
            // connection reset on the next receive; it is not a listener failure.
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            Err(_) => return Err("Discovery receive failed on the selected interface.".into()),
        }
    }
    if candidates.is_empty() {
        return Err("No AnPod Receiver answered on the selected interface; it may be unreachable or stopped and require authorized USB/bootstrap. Discovery cannot determine the cause.".into());
    }
    Ok(candidates)
}

fn encode(kind: u8, token: [u8; 16], control_port: u16) -> [u8; FRAME_BYTES] {
    let mut frame = [0; FRAME_BYTES];
    frame[..4].copy_from_slice(MAGIC);
    frame[4] = VERSION;
    frame[5] = kind;
    frame[8..24].copy_from_slice(&token);
    frame[24..].copy_from_slice(&control_port.to_be_bytes());
    frame
}

fn decode_candidate(frame: &[u8], token: &[u8; 16]) -> Option<u16> {
    if frame.len() != FRAME_BYTES
        || &frame[..4] != MAGIC
        || frame[4] != VERSION
        || frame[5] != CANDIDATE
        || frame[6..8] != [0, 0]
        || &frame[8..24] != token
    {
        return None;
    }
    let port = u16::from_be_bytes([frame[24], frame[25]]);
    (port != 0).then_some(port)
}

fn is_poll_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
pub(crate) fn candidate_for_test(token: [u8; 16], port: u16) -> [u8; FRAME_BYTES] {
    encode(CANDIDATE, token, port)
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
