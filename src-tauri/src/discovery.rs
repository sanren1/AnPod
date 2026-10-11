use crate::network_interface::Ipv4Interface;
use getrandom::fill;
use std::{
    io,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
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

/// Unauthenticated network hint. Callers must authenticate it with P5-A before START.
pub(crate) struct Candidate {
    pub(crate) endpoint: SocketAddr,
}

pub(crate) fn probe_targets(interface: &Ipv4Interface) -> Vec<Ipv4Addr> {
    let mut targets = Vec::new();
    if let Some(gateway) = interface.gateways.iter().find(|gateway| interface.contains(**gateway)) {
        if !targets.contains(gateway) { targets.push(*gateway); }
    }
    if let Some(broadcast) = interface.directed_broadcast() {
        if !targets.contains(&broadcast) { targets.push(broadcast); }
    }
    targets
}

/// Sends one correlated probe to only the broadcast and/or explicit gateway targets
/// of the interface selected by the caller. This function is never called implicitly.
pub(crate) fn discover_on_interface(interface: &Ipv4Interface, targets: &[Ipv4Addr],
    timeout: Duration, cancellation: &CancellationToken) -> Result<Vec<Candidate>, String> {
    let local_valid = interface.address.is_loopback()
        || (interface.address.is_private() && !interface.address.is_link_local());
    if !local_valid || interface.prefix_length == 0 || interface.prefix_length > 32 {
        return Err("Selected IPv4 interface details are unavailable or outside the private LAN scope.".into());
    }
    if timeout.is_zero() || timeout > MAX_DISCOVERY_TIMEOUT {
        return Err("IPv4 discovery is limited to a two-second cancellable window.".into());
    }
    if targets.is_empty() || targets.len() > 2 {
        return Err("Select one broadcast target and at most one explicit gateway target.".into());
    }
    let allowed_targets = probe_targets(interface);
    if targets.iter().any(|target| !allowed_targets.contains(target)) {
        return Err("Discovery targets must be the selected interface's gateway or directed broadcast.".into());
    }
    let socket = UdpSocket::bind(SocketAddr::from((interface.address, 0)))
        .map_err(|_| "Selected network interface is unavailable for discovery.".to_string())?;
    socket.set_broadcast(true).map_err(|_| "Unable to enable bounded UDP discovery broadcast.".to_string())?;
    socket.set_read_timeout(Some(IO_POLL)).map_err(|_| "Unable to configure discovery receive polling.".to_string())?;

    let mut token = [0u8; 16];
    fill(&mut token).map_err(|_| "Unable to create a fresh discovery probe token.".to_string())?;
    let probe = encode(PROBE, token, 0);
    for target in targets {
        if cancellation.is_cancelled() { return Err("Network discovery was cancelled.".into()); }
        socket.send_to(&probe, SocketAddr::from((*target, DISCOVERY_PORT)))
            .map_err(|_| "Unable to send a discovery probe on the selected interface.".to_string())?;
    }

    let deadline = Instant::now() + timeout;
    let mut candidates = Vec::new();
    let mut bytes = [0u8; 128];
    while Instant::now() < deadline {
        if cancellation.is_cancelled() { return Err("Network discovery was cancelled.".into()); }
        match socket.recv_from(&mut bytes) {
            Ok((length, source)) => {
                let SocketAddr::V4(source) = source else { continue; };
                if source.port() == 0 || !interface.contains(*source.ip()) { continue; }
                let Some(port) = decode_candidate(&bytes[..length], &token) else { continue; };
                let endpoint = SocketAddr::new((*source.ip()).into(), port);
                if !candidates.iter().any(|item: &Candidate| item.endpoint == endpoint) {
                    candidates.push(Candidate { endpoint });
                    if candidates.len() == MAX_CANDIDATES { break; }
                }
            }
            Err(error) if is_poll_timeout(&error) => {}
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
    if frame.len() != FRAME_BYTES || &frame[..4] != MAGIC || frame[4] != VERSION
        || frame[5] != CANDIDATE || frame[6..8] != [0, 0] || &frame[8..24] != token {
        return None;
    }
    let port = u16::from_be_bytes([frame[24], frame[25]]);
    (port != 0).then_some(port)
}

fn is_poll_timeout(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

#[cfg(test)]
pub(crate) fn candidate_for_test(token: [u8; 16], port: u16) -> [u8; FRAME_BYTES] {
    encode(CANDIDATE, token, port)
}
