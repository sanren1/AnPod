use crate::{
    network::{PcmQueue, UdpSender, UdpSessionMaterial},
    network_interface::Ipv4Interface,
    protocol::PcmFormat,
    receiver::PlaybackDiagnostics,
    security::{AudioParameters, BindingMaterial, HostHandshake},
};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    sync::{atomic::{AtomicBool, AtomicU64, Ordering}, mpsc, Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
#[cfg(test)]
use crate::protocol::{PacketEncoder, PacketType, HOST_TO_RECEIVER};
#[cfg(test)]
use std::net::IpAddr;

const CONTROL_MAGIC: &[u8; 4] = b"ANPC";
const CONTROL_HEADER_BYTES: usize = 30;
const CONTROL_TAG_BYTES: usize = 32;
const MAX_CONTROL_FRAME: usize = 1200;
const HOST_TO_RECEIVER_CONTROL: u32 = 0x414e_5048;
const RECEIVER_TO_HOST_CONTROL: u32 = 0x414e_5052;
const BIND: u8 = 1;
const SELECT_MODE: u8 = 2;
const START: u8 = 3;
const PAUSE: u8 = 4;
const RESUME: u8 = 5;
const STOP: u8 = 6;
const HEARTBEAT: u8 = 7;
const CAPABILITIES_QUERY: u8 = 8;
const CAPABILITIES_INFO: u8 = 9;
const PLAYBACK_PROGRESS_QUERY: u8 = 16;
const CONNECTION_INTENT: u8 = 17;
const ACK: u8 = 127;
const RESULT_OK: u8 = 0;
const RESULT_REJECTED: u8 = 1;
const RESULT_FAILED: u8 = 2;
const MANAGEMENT_CONTROL_MAJOR: u16 = 3;
const AUDIO_PROTOCOL_MAJOR: u8 = 1;
const CAP_RECEIVER_UI: u32 = 0x0000_0001;
const CAP_AUTHENTICATED_STATUS: u32 = 0x0000_0002;
const CAP_PLAYBACK_PROGRESS: u32 = 0x0000_0008;
const CAP_CONNECTION_INTENT: u32 = 0x0000_0010;
const IMPLEMENTED_CAPABILITIES: u32 = CAP_RECEIVER_UI | CAP_AUTHENTICATED_STATUS | CAP_PLAYBACK_PROGRESS;
const REQUIRED_CAPABILITIES: u32 = CAP_AUTHENTICATED_STATUS;
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_IO_POLL: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const SESSION_IO_DEADLINE: Duration = Duration::from_secs(3);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const RECONNECT_DELAYS: [Duration; 4] = [
    Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(10),
];

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct SessionEndpoint {
    interface_index: u32,
    local_address: Ipv4Addr,
    prefix_length: u8,
    peer: SocketAddr,
    connection_intent: Option<([u8; 16], bool)>,
}

impl SessionEndpoint {
    fn loopback(peer: SocketAddr) -> Self {
        Self { interface_index: 0, local_address: Ipv4Addr::LOCALHOST, prefix_length: 32, peer, connection_intent: None }
    }

    fn on_interface(interface: &Ipv4Interface, peer: SocketAddr) -> Self {
        Self { interface_index: interface.index, local_address: interface.address,
            prefix_length: interface.prefix_length, peer, connection_intent: None }
    }

    fn validate(self) -> Result<(), String> {
        let local_valid = self.local_address.is_loopback()
            || (self.local_address.is_private() && !self.local_address.is_link_local());
        let SocketAddr::V4(peer) = self.peer else { return Err("P5-C requires a literal IPv4 receiver endpoint.".into()); };
        if !local_valid || self.prefix_length == 0 || self.prefix_length > 32 || peer.port() == 0
            || peer.ip().is_unspecified() || peer.ip().is_multicast()
            || (self.local_address.is_loopback() != peer.ip().is_loopback())
            || (self.local_address.is_private() && !peer.ip().is_private()) {
            return Err("P5-C endpoint must use the selected private IPv4 interface and a literal private IPv4 peer.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReceiverState {
    Authenticated,
    TransportConnected,
    ReceiverRunning,
    Streaming,
    Paused,
    Stopped,
}

impl ReceiverState {
    fn from_wire(value: u8) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Authenticated),
            1 => Ok(Self::TransportConnected),
            2 => Ok(Self::ReceiverRunning),
            3 => Ok(Self::Streaming),
            4 => Ok(Self::Paused),
            5 => Ok(Self::Stopped),
            _ => Err("Receiver returned an unknown P5-C state.".into()),
        }
    }
    fn wire(self) -> u8 {
        match self {
            Self::Authenticated => 0,
            Self::TransportConnected => 1,
            Self::ReceiverRunning => 2,
            Self::Streaming => 3,
            Self::Paused => 4,
            Self::Stopped => 5,
        }
    }
}

struct SessionKeys {
    audio: [u8; 32],
    control_tx: [u8; 32],
    control_rx: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReceiverCapabilities {
    pub(crate) control_major: u16,
    pub(crate) audio_major: u8,
    pub(crate) management_compatible: bool,
    pub(crate) audio_compatible: bool,
    pub(crate) compatible: bool,
    pub(crate) capability_bits: u32,
    pub(crate) version_code: u32,
}

impl Drop for SessionKeys {
    fn drop(&mut self) {
        self.audio.fill(0);
        self.control_tx.fill(0);
        self.control_rx.fill(0);
    }
}

/// Explicit host side of one authenticated P5-C session. It has no listener or product caller.
pub(crate) struct HostSession {
    tcp: Option<TcpStream>,
    udp_socket: Option<UdpSocket>,
    sender: Option<UdpSender>,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
    queue: Arc<PcmQueue>,
    keys: SessionKeys,
    parameters: AudioParameters,
    session_id: u64,
    remote_udp: Option<SocketAddr>,
    command_sequence: u64,
    ack_sequence: u64,
    last_command: Option<Vec<u8>>,
    last_ack: Option<Vec<u8>>,
    last_heartbeat: Instant,
    state: ReceiverState,
    capabilities: Option<ReceiverCapabilities>,
    connection_intent: Option<([u8; 16], bool)>,
    cancellation: CancellationToken,
}

impl HostSession {
    pub(crate) fn authenticate_loopback(peer: SocketAddr, material: BindingMaterial,
        parameters: AudioParameters) -> Result<Self, String> {
        Self::authenticate_loopback_cancellable(peer, material, parameters, CancellationToken::new())
    }

    fn authenticate_loopback_cancellable(peer: SocketAddr, material: BindingMaterial,
        parameters: AudioParameters, parent_cancel: CancellationToken) -> Result<Self, String> {
        Self::authenticate_on_network(SessionEndpoint::loopback(peer), material, parameters, parent_cancel)
    }

    fn authenticate_on_network(endpoint: SessionEndpoint, material: BindingMaterial,
        parameters: AudioParameters, parent_cancel: CancellationToken) -> Result<Self, String> {
        parameters.validate().map_err(|_| "Unsupported P5-C PCM parameters.".to_string())?;
        endpoint.validate()?;
        let cancellation = parent_cancel.child_token();
        check_cancelled(&cancellation).map_err(|_| "P5-C connection was cancelled.".to_string())?;
        let udp_socket = UdpSocket::bind((endpoint.local_address, 0))
            .map_err(|_| "Unable to bind the P5-C audio socket to the selected interface.".to_string())?;
        let mut tcp = connect_cancellable(endpoint.local_address, endpoint.peer, &cancellation)
            .map_err(|error| if error.kind() == std::io::ErrorKind::AddrNotAvailable {
                "The selected network interface is no longer available.".to_string()
            } else {
                "Receiver endpoint is unavailable on the selected interface; it may be unreachable or require authorized USB/bootstrap if stopped.".to_string()
            })?;
        tcp.set_read_timeout(Some(SESSION_IO_POLL)).map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;
        tcp.set_write_timeout(Some(SESSION_IO_POLL)).map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;

        let (handshake, hello) = HostHandshake::start(material, parameters)
            .map_err(|_| "P5-A handshake could not start.".to_string())?;
        write_all_cancellable(&mut tcp, &hello, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A HELLO.".to_string())?;
        let mut challenge = [0u8; 86];
        read_exact_cancellable(&mut tcp, &mut challenge, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A CHALLENGE.".to_string())?;
        let (await_ready, finish) = handshake.accept_challenge(&challenge)
            .map_err(|_| "P5-A receiver identity or proof was rejected.".to_string())?;
        write_all_cancellable(&mut tcp, &finish, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A FINISH.".to_string())?;
        let mut ready = [0u8; 38];
        read_exact_cancellable(&mut tcp, &mut ready, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A READY.".to_string())?;
        let session = await_ready.accept_ready(&ready)
            .map_err(|_| "P5-A READY proof was rejected.".to_string())?;
        let session_id = session.session_id();
        let authenticated_parameters = session.parameters();
        let keys = session.into_keys();
        let session_keys = SessionKeys {
            audio: keys.host_to_receiver_audio,
            control_tx: keys.host_to_receiver_control,
            control_rx: keys.receiver_to_host_control,
        };
        drop(keys);
        let format = PcmFormat { sample_rate: authenticated_parameters.sample_rate,
            channels: authenticated_parameters.channels as u16 };
        let pcm_payload_bytes_sent = Arc::new(AtomicU64::new(0));
        Ok(Self {
            tcp: Some(tcp),
            udp_socket: Some(udp_socket),
            sender: None,
            pcm_payload_bytes_sent,
            queue: Arc::new(PcmQueue::for_session(format, authenticated_parameters.mode == 1)),
            keys: session_keys,
            parameters: authenticated_parameters,
            session_id,
            remote_udp: None,
            command_sequence: 0,
            ack_sequence: 0,
            last_command: None,
            last_ack: None,
            last_heartbeat: Instant::now(),
            state: ReceiverState::Authenticated,
            capabilities: None,
            connection_intent: endpoint.connection_intent,
            cancellation,
        })
    }

    pub(crate) fn connect_loopback(peer: SocketAddr, material: BindingMaterial,
        parameters: AudioParameters) -> Result<Self, String> {
        Self::connect_loopback_cancellable(peer, material, parameters, CancellationToken::new())
    }

    pub(crate) fn connect_on_interface(interface: &Ipv4Interface, peer: SocketAddr,
        material: BindingMaterial, parameters: AudioParameters) -> Result<Self, String> {
        Self::connect_endpoint(SessionEndpoint::on_interface(interface, peer), material, parameters,
            CancellationToken::new())
    }

    fn connect_loopback_cancellable(peer: SocketAddr, material: BindingMaterial,
        parameters: AudioParameters, cancellation: CancellationToken) -> Result<Self, String> {
        Self::connect_endpoint(SessionEndpoint::loopback(peer), material, parameters, cancellation)
    }

    fn connect_endpoint(endpoint: SessionEndpoint, material: BindingMaterial,
        parameters: AudioParameters, cancellation: CancellationToken) -> Result<Self, String> {
        let mut session = Self::authenticate_on_network(endpoint, material, parameters, cancellation)?;
        if let Err(error) = session.negotiate_capabilities()
            .and_then(|_| if session.capabilities().is_some_and(|info| info.compatible) {
                session.bind_audio().and_then(|_| session.select_mode())
            } else { Err("Receiver audio or management capabilities are incompatible; update it through authorized ADB or manual installation.".into()) }) {
            session.close_local();
            return Err(error);
        }
        Ok(session)
    }

    pub(crate) fn capabilities(&self) -> Option<&ReceiverCapabilities> { self.capabilities.as_ref() }

    fn negotiate_capabilities(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::Authenticated || self.capabilities.is_some() {
            return Err("P5-C capability query is out of order.".into());
        }
        let result = (|| {
            let command_sequence = self.command_sequence;
            let payload = encode_capabilities_query(MANAGEMENT_CONTROL_MAJOR, AUDIO_PROTOCOL_MAJOR, REQUIRED_CAPABILITIES);
            let frame = encode_control(&self.keys.control_tx, HOST_TO_RECEIVER_CONTROL,
                self.session_id, command_sequence, CAPABILITIES_QUERY, &payload)?;
            let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
            write_frame(tcp, &frame, &self.cancellation)?;
            let response = read_frame(tcp, &self.keys.control_rx, RECEIVER_TO_HOST_CONTROL,
                self.session_id, &self.cancellation)?;
            let info = decode_capabilities_info(&response, self.ack_sequence, command_sequence)?;
            self.command_sequence = self.command_sequence.checked_add(1)
                .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
            self.ack_sequence = self.ack_sequence.checked_add(1)
                .ok_or_else(|| "P5-C response sequence exhausted.".to_string())?;
            self.capabilities = Some(info.clone());
            if let Some((nonce, required)) = self.connection_intent {
                if info.capability_bits & CAP_CONNECTION_INTENT != 0 {
                    let ack = self.command(CONNECTION_INTENT, &nonce)?;
                    if ack.result != RESULT_OK || ack.state != ReceiverState::Authenticated {
                        return Err("Receiver connection intent expired or was cancelled; start receiving again on the phone.".into());
                    }
                } else if required {
                    return Err("Receiver does not support one-click reception; use explicit connection or update through USB.".into());
                }
            }
            Ok(())
        })();
        if result.is_err() { self.close_local(); }
        result
    }

    pub(crate) fn state(&self) -> ReceiverState { self.state }
    pub(crate) fn session_id(&self) -> u64 { self.session_id }
    pub(crate) fn pcm_queue(&self) -> Arc<PcmQueue> { self.queue.clone() }
    pub(crate) fn pcm_payload_bytes_sent(&self) -> u64 {
        self.pcm_payload_bytes_sent.load(Ordering::Relaxed)
    }

    pub(crate) fn bind_audio(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::Authenticated { return Err("P5-C BIND is out of order.".into()); }
        let port = self.udp_socket.as_ref().ok_or("P5-C UDP socket is closed.")?
            .local_addr().map_err(|_| "Unable to read P5-C UDP source port.".to_string())?.port();
        let ack = self.command(BIND, &port.to_be_bytes())?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::TransportConnected || ack.udp_port == 0 {
            return Err("P5-C receiver did not confirm the authenticated UDP peer.".into());
        }
        let tcp_peer = self.tcp.as_ref().ok_or("P5-C control channel is closed.")?
            .peer_addr().map_err(|_| "Unable to read P5-C peer address.".to_string())?;
        self.remote_udp = Some(SocketAddr::new(tcp_peer.ip(), ack.udp_port));
        self.state = ack.state;
        Ok(())
    }

    pub(crate) fn select_mode(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::TransportConnected { return Err("P5-C mode selection is out of order.".into()); }
        let ack = self.command(SELECT_MODE, &[self.parameters.mode])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::TransportConnected {
            return Err("P5-C receiver rejected the authenticated mode.".into());
        }
        Ok(())
    }

    pub(crate) fn start(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::TransportConnected || self.sender.is_some() {
            return Err("P5-C START is out of order.".into());
        }
        let ack = self.command(START, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::ReceiverRunning {
            self.close_local();
            return Err("P5-C receiver did not confirm playback start.".into());
        }
        let result = (|| {
            let socket = self.udp_socket.as_ref().ok_or("P5-C UDP socket is closed.")?
                .try_clone().map_err(|_| "Unable to transfer the authenticated UDP socket.".to_string())?;
            let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
            let material = UdpSessionMaterial::from_authenticated(self.session_id, &self.keys.audio);
            self.sender = Some(UdpSender::spawn_authenticated(remote, self.queue.clone(), socket,
                material, self.parameters, self.pcm_payload_bytes_sent.clone())?);
            Ok(())
        })();
        if let Err(error) = result {
            self.close_local();
            return Err(error);
        }
        self.state = ack.state;
        Ok(())
    }

    #[cfg(test)]
    fn start_without_format_for_test(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::TransportConnected || self.sender.is_some() {
            return Err("P5-C START is out of order.".into());
        }
        let ack = self.command(START, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::ReceiverRunning {
            self.close_local();
            return Err("P5-C receiver did not confirm playback start.".into());
        }
        let result = (|| {
            let socket = self.udp_socket.as_ref().ok_or("P5-C UDP socket is closed.")?
                .try_clone().map_err(|_| "Unable to transfer the authenticated UDP socket.".to_string())?;
            let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
            let material = UdpSessionMaterial::from_authenticated(self.session_id, &self.keys.audio);
            self.sender = Some(UdpSender::spawn_authenticated_without_format_for_test(remote,
                self.queue.clone(), socket, material, self.parameters, self.pcm_payload_bytes_sent.clone())?);
            Ok(())
        })();
        if let Err(error) = result {
            self.close_local();
            return Err(error);
        }
        self.state = ack.state;
        Ok(())
    }

    pub(crate) fn pause(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::ReceiverRunning && self.state != ReceiverState::Streaming {
            return Err("P5-C PAUSE is out of order.".into());
        }
        let ack = self.command(PAUSE, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::Paused { return Err("P5-C PAUSE was rejected.".into()); }
        self.state = ack.state;
        Ok(())
    }

    pub(crate) fn resume(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::Paused { return Err("P5-C RESUME is out of order.".into()); }
        let ack = self.command(RESUME, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::ReceiverRunning { return Err("P5-C RESUME was rejected.".into()); }
        self.state = ack.state;
        Ok(())
    }

    pub(crate) fn heartbeat(&mut self) -> Result<ReceiverState, String> {
        let ack = self.command(HEARTBEAT, &[])?;
        if ack.result != RESULT_OK { return Err("P5-C heartbeat was rejected.".into()); }
        self.last_heartbeat = Instant::now();
        self.state = ack.state;
        Ok(self.state)
    }

    fn playback_progress(&mut self) -> Result<Option<PlaybackDiagnostics>, String> {
        if !self.capabilities.as_ref().is_some_and(|info| info.capability_bits & CAP_PLAYBACK_PROGRESS != 0) {
            return Ok(None);
        }
        let ack = self.command(PLAYBACK_PROGRESS_QUERY, &[])?;
        if ack.result != RESULT_OK { return Err("P5-C playback progress query was rejected.".into()); }
        ack.playback_progress.ok_or_else(|| "Receiver playback progress response was missing.".to_string()).map(Some)
    }

    pub(crate) fn check_liveness_at(&mut self, now: Instant) -> Result<(), String> {
        if now.saturating_duration_since(self.last_heartbeat) > HEARTBEAT_TIMEOUT {
            self.close_local();
            return Err("P5-C authenticated heartbeat expired.".into());
        }
        Ok(())
    }

    pub(crate) fn stop(&mut self) -> Result<(), String> {
        let result = if self.state == ReceiverState::Stopped { Ok(()) } else {
            match self.command(STOP, &[]) {
                Ok(ack) if ack.result == RESULT_OK && ack.state == ReceiverState::Stopped => {
                    self.state = ReceiverState::Stopped;
                    Ok(())
                }
                Ok(_) => Err("P5-C STOP was rejected.".into()),
                Err(error) => Err(error),
            }
        };
        self.close_local();
        result
    }

    fn command(&mut self, kind: u8, payload: &[u8]) -> Result<Ack, String> {
        let result = self.command_inner(kind, payload);
        if result.is_err() { self.close_local(); }
        result
    }

    fn command_inner(&mut self, kind: u8, payload: &[u8]) -> Result<Ack, String> {
        let sequence = self.command_sequence;
        let frame = encode_control(&self.keys.control_tx, HOST_TO_RECEIVER_CONTROL,
            self.session_id, sequence, kind, payload)?;
        let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
        write_frame(tcp, &frame, &self.cancellation)?;
        let ack_frame = read_frame(tcp, &self.keys.control_rx, RECEIVER_TO_HOST_CONTROL,
            self.session_id, &self.cancellation)?;
        let ack = decode_ack(&ack_frame, self.ack_sequence, kind, sequence)?;
        self.command_sequence = self.command_sequence.checked_add(1)
            .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
        self.ack_sequence = self.ack_sequence.checked_add(1)
            .ok_or_else(|| "P5-C ACK sequence exhausted.".to_string())?;
        self.last_command = Some(frame);
        self.last_ack = Some(ack_frame);
        Ok(ack)
    }

    fn close_local(&mut self) {
        self.cancellation.cancel();
        self.queue.stop_and_clear();
        if let Some(sender) = self.sender.take() { let _ = sender.stop(); }
        drop(self.udp_socket.take());
        if let Some(tcp) = self.tcp.take() { let _ = tcp.shutdown(std::net::Shutdown::Both); }
        self.keys.audio.fill(0);
        self.keys.control_tx.fill(0);
        self.keys.control_rx.fill(0);
        self.state = ReceiverState::Stopped;
    }

    #[cfg(test)]
    fn resend_last_for_test(&mut self) -> Result<(), String> {
        let frame = self.last_command.as_ref().ok_or("No P5-C command to repeat.")?.clone();
        let expected_ack = self.last_ack.as_ref().ok_or("No P5-C ACK to compare.")?.clone();
        write_frame(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?, &frame, &self.cancellation)?;
        let actual_ack = read_raw_frame(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?, &self.cancellation)?;
        if actual_ack != expected_ack { return Err("P5-C duplicate command changed its ACK.".into()); }
        Ok(())
    }

    #[cfg(test)]
    fn send_bad_mac_for_test(&mut self) -> Result<(), String> {
        let port = self.udp_socket.as_ref().ok_or("P5-C UDP socket is closed.")?
            .local_addr().map_err(|_| "UDP socket unavailable".to_string())?.port();
        let mut frame = encode_control(&self.keys.control_tx, HOST_TO_RECEIVER_CONTROL,
            self.session_id, self.command_sequence, BIND, &port.to_be_bytes())?;
        *frame.last_mut().unwrap() ^= 0x80;
        write_frame(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?, &frame, &self.cancellation)?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    fn send_bad_capabilities_query_for_test(&mut self, bad_mac: bool) -> Result<(), String> {
        let mut payload = encode_capabilities_query(MANAGEMENT_CONTROL_MAJOR, AUDIO_PROTOCOL_MAJOR, REQUIRED_CAPABILITIES);
        payload[3] = 1;
        let mut frame = encode_control(&self.keys.control_tx, HOST_TO_RECEIVER_CONTROL,
            self.session_id, self.command_sequence, CAPABILITIES_QUERY, &payload)?;
        if bad_mac { *frame.last_mut().unwrap() ^= 1; }
        write_frame(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?, &frame, &self.cancellation)?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    fn send_old_session_for_test(&mut self) -> Result<(), String> {
        let port = self.udp_socket.as_ref().ok_or("P5-C UDP socket is closed.")?
            .local_addr().map_err(|_| "UDP socket unavailable".to_string())?.port();
        let frame = encode_control(&self.keys.control_tx, HOST_TO_RECEIVER_CONTROL,
            self.session_id.wrapping_sub(1), self.command_sequence, BIND, &port.to_be_bytes())?;
        write_frame(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?, &frame, &self.cancellation)?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    fn send_old_audio_for_test(&self) -> Result<(), String> {
        let format = self.parameters;
        let mut encoder = PacketEncoder::with_sequence(&self.keys.audio, self.session_id.wrapping_sub(1),
            HOST_TO_RECEIVER, 0);
        let frame = encoder.encode_next(PacketType::Format, 0, &PcmFormat {
            sample_rate: format.sample_rate, channels: format.channels as u16,
        }.encode().map_err(|_| "invalid session PCM format")?)
            .map_err(|_| "unable to encode synthetic old-session packet")?;
        let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
        let sent = self.udp_socket.as_ref().ok_or("P5-C UDP socket is closed.")?
            .send_to(&frame, remote).map_err(|_| "unable to send old-session packet")?;
        if sent != frame.len() { return Err("old-session datagram send was incomplete".into()); }
        Ok(())
    }

    #[cfg(test)]
    fn send_format_packet_for_test(&self, sample_rate: u32) -> Result<(), String> {
        let payload = PcmFormat { sample_rate, channels: self.parameters.channels as u16 }
            .encode().map_err(|_| "invalid synthetic test format")?;
        let mut encoder = PacketEncoder::with_sequence(&self.keys.audio, self.session_id,
            HOST_TO_RECEIVER, 0);
        let packet = encoder.encode_next(PacketType::Format, 0, &payload)
            .map_err(|_| "unable to encode synthetic FORMAT")?;
        let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
        let sent = self.udp_socket.as_ref().ok_or("P5-C UDP socket is closed.")?
            .send_to(&packet, remote).map_err(|_| "unable to send synthetic FORMAT")?;
        if sent != packet.len() { return Err("synthetic FORMAT datagram send was incomplete".into()); }
        Ok(())
    }

    #[cfg(test)]
    fn remove_udp_socket_for_test(&mut self) { drop(self.udp_socket.take()); }

    #[cfg(test)]
    fn remove_control_for_test(&mut self) {
        if let Some(tcp) = self.tcp.take() {
            let _ = tcp.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Drop for HostSession {
    fn drop(&mut self) { self.close_local(); }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionLifecycleState {
    Authenticated,
    TransportConnected,
    ReceiverRunning,
    Streaming,
    Paused,
    Reconnecting,
    Stopped,
}

#[derive(Clone, Debug)]
pub(crate) struct SessionSnapshot {
    pub(crate) state: SessionLifecycleState,
    pub(crate) generation: u64,
    pub(crate) attempts: u64,
    pub(crate) session_id: Option<u64>,
    pub(crate) capabilities: Option<ReceiverCapabilities>,
    pub(crate) paused: bool,
    pub(crate) last_error: Option<String>,
    pub(crate) playback_diagnostics: Option<PlaybackDiagnostics>,
}

#[derive(Clone, Copy)]
struct SessionTiming {
    heartbeat_interval: Duration,
    reconnect_delays: [Duration; 4],
    recovery_window: Duration,
}

const DEFAULT_SESSION_TIMING: SessionTiming = SessionTiming {
    heartbeat_interval: HEARTBEAT_INTERVAL,
    reconnect_delays: RECONNECT_DELAYS,
    recovery_window: Duration::from_secs(30),
};

enum OwnerCommand {
    SetPaused(bool),
    UpdateEndpoint(SessionEndpoint),
    SetMode(u8),
    Reconnect,
    Stop,
}

/// Single thread owns every HostSession. It is created only by an explicit caller.
pub(crate) struct SessionOwner {
    connection_intent: Option<([u8; 16], bool)>,
    cancellation: CancellationToken,
    enabled: Arc<AtomicBool>,
    commands: mpsc::Sender<OwnerCommand>,
    status: Arc<Mutex<SessionSnapshot>>,
    queue: Arc<Mutex<Option<Arc<PcmQueue>>>>,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
}

impl SessionOwner {
    pub(crate) fn start(peer: SocketAddr, binding: BindingMaterial, parameters: AudioParameters,
        caller_cancel: CancellationToken) -> Result<Self, String> {
        Self::start_with_timing(peer, binding, parameters, caller_cancel, DEFAULT_SESSION_TIMING)
    }

    pub(crate) fn start_on_interface(interface: &Ipv4Interface, peer: SocketAddr,
        binding: BindingMaterial, parameters: AudioParameters, caller_cancel: CancellationToken) -> Result<Self, String> {
        Self::start_with_endpoint(SessionEndpoint::on_interface(interface, peer), binding, parameters,
            caller_cancel, DEFAULT_SESSION_TIMING)
    }

    pub(crate) fn start_on_interface_with_intent(interface: &Ipv4Interface, peer: SocketAddr,
        binding: BindingMaterial, parameters: AudioParameters, caller_cancel: CancellationToken,
        nonce: [u8; 16], required: bool) -> Result<Self, String> {
        let mut endpoint = SessionEndpoint::on_interface(interface, peer);
        endpoint.connection_intent = Some((nonce, required));
        Self::start_with_endpoint(endpoint, binding, parameters, caller_cancel, DEFAULT_SESSION_TIMING)
    }

    fn start_with_timing(peer: SocketAddr, binding: BindingMaterial, parameters: AudioParameters,
        caller_cancel: CancellationToken, timing: SessionTiming) -> Result<Self, String> {
        Self::start_with_endpoint(SessionEndpoint::loopback(peer), binding, parameters, caller_cancel, timing)
    }

    fn start_with_endpoint(endpoint: SessionEndpoint, binding: BindingMaterial, parameters: AudioParameters,
        caller_cancel: CancellationToken, timing: SessionTiming) -> Result<Self, String> {
        endpoint.validate()?;
        let cancellation = caller_cancel.child_token();
        let enabled = Arc::new(AtomicBool::new(true));
        let status = Arc::new(Mutex::new(SessionSnapshot {
            state: SessionLifecycleState::Reconnecting,
            generation: 0,
            attempts: 0,
            session_id: None,
            capabilities: None,
            paused: false,
            last_error: None,
            playback_diagnostics: None,
        }));
        let queue = Arc::new(Mutex::new(None));
        let pcm_payload_bytes_sent = Arc::new(AtomicU64::new(0));
        let (commands, receiver) = mpsc::channel();
        let owner_cancel = cancellation.clone();
        let owner_enabled = enabled.clone();
        let owner_status = status.clone();
        let owner_queue = queue.clone();
        let owner_bytes = pcm_payload_bytes_sent.clone();
        let worker = thread::Builder::new().name("AnPod P5-D session owner".into())
            .spawn(move || run_session_owner(endpoint, binding, parameters, owner_cancel, owner_enabled,
                receiver, owner_status, owner_queue, owner_bytes, timing))
            .map_err(|error| format!("Unable to start P5-D session owner: {error}"))?;
        Ok(Self { connection_intent: endpoint.connection_intent, cancellation, enabled, commands, status, queue, pcm_payload_bytes_sent, worker: Some(worker) })
    }

    pub(crate) fn snapshot(&self) -> SessionSnapshot {
        self.status.lock().expect("P5-D session status lock poisoned").clone()
    }

    pub(crate) fn pcm_queue(&self) -> Option<Arc<PcmQueue>> {
        self.queue.lock().expect("P5-D PCM queue lock poisoned").clone()
    }

    pub(crate) fn pcm_payload_bytes_sent(&self) -> u64 { self.pcm_payload_bytes_sent.load(Ordering::Relaxed) }

    pub(crate) fn set_paused(&self, paused: bool) -> Result<(), String> {
        self.commands.send(OwnerCommand::SetPaused(paused))
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    pub(crate) fn set_mode(&self, mode: u8) -> Result<(), String> {
        if mode > 1 { return Err("P5-C playback mode is invalid.".into()); }
        self.commands.send(OwnerCommand::SetMode(mode))
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    pub(crate) fn reconnect(&self) -> Result<(), String> {
        self.commands.send(OwnerCommand::Reconnect)
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    /// The caller must supply each changed endpoint; the owner never discovers or guesses one.
    pub(crate) fn update_endpoint(&self, peer: SocketAddr) -> Result<(), String> {
        self.enqueue_endpoint(SessionEndpoint::loopback(peer))
    }

    pub(crate) fn update_network_endpoint(&self, interface: &Ipv4Interface, peer: SocketAddr) -> Result<(), String> {
        self.enqueue_endpoint(SessionEndpoint::on_interface(interface, peer))
    }

    fn enqueue_endpoint(&self, mut endpoint: SessionEndpoint) -> Result<(), String> {
        endpoint.connection_intent = self.connection_intent;
        endpoint.validate()?;
        self.commands.send(OwnerCommand::UpdateEndpoint(endpoint))
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    pub(crate) fn cancel(&mut self) { self.enabled.store(false, Ordering::Release); self.cancellation.cancel(); self.join(); }

    pub(crate) fn stop(&mut self) {
        if self.worker.is_none() { return; }
        if self.commands.send(OwnerCommand::Stop).is_err() {
            self.enabled.store(false, Ordering::Release);
            self.cancellation.cancel();
        } else {
            let deadline = Instant::now() + Duration::from_millis(100);
            while self.worker.as_ref().is_some_and(|worker| !worker.is_finished()) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if self.worker.as_ref().is_some_and(|worker| !worker.is_finished()) {
                self.enabled.store(false, Ordering::Release);
                self.cancellation.cancel();
            }
        }
        self.join();
        self.enabled.store(false, Ordering::Release);
        self.cancellation.cancel();
    }

    fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            if worker.thread().id() != thread::current().id() { let _ = worker.join(); }
        }
    }
}

impl Drop for SessionOwner {
    fn drop(&mut self) { self.cancel(); }
}

fn run_session_owner(endpoint: SessionEndpoint, binding: BindingMaterial, parameters: AudioParameters,
    cancellation: CancellationToken, enabled: Arc<AtomicBool>, commands: mpsc::Receiver<OwnerCommand>,
    status: Arc<Mutex<SessionSnapshot>>, queue: Arc<Mutex<Option<Arc<PcmQueue>>>>,
    pcm_payload_bytes_sent: Arc<AtomicU64>, timing: SessionTiming) {
    let mut endpoint = endpoint;
    let mut parameters = parameters;
    let mut paused = false;
    let mut desired_paused = false;
    let mut generation = 0u64;
    let mut attempts = 0u64;
    let mut retry_index = 0;
    let mut last_error = None;
    let mut last_active_at = None;
    'owner: loop {
        if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() { break; }
        generation = generation.saturating_add(1);
        attempts = attempts.saturating_add(1);
        publish_session(&status, SessionLifecycleState::Reconnecting, generation, attempts,
            None, paused, last_error.clone());
        let result = HostSession::authenticate_on_network(
            endpoint, binding.duplicate_for_session(), parameters, cancellation.clone());
        let mut session = match result {
            Ok(session) => session,
            Err(error) => {
                if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() { break; }
                last_error = Some(error);
                if endpoint.connection_intent.is_some() && last_active_at.is_none() { break 'owner; }
                publish_session(&status, SessionLifecycleState::Reconnecting, generation, attempts,
                    None, paused, last_error.clone());
                match wait_for_reconnect(lifecycle_retry_delay(&timing.reconnect_delays, retry_index), &commands, &cancellation,
                    &enabled, &mut endpoint, &mut parameters, &mut desired_paused) {
                    RetryWait::Stop => break,
                    RetryWait::ReconnectNow => retry_index = 0,
                    RetryWait::DelayElapsed => {
                        retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                        if recovery_window_expired(last_active_at, timing.recovery_window, Instant::now()) { break 'owner; }
                    }
                }
                continue 'owner;
            }
        };
        if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() { break; }
        if let Err(error) = session.negotiate_capabilities() {
            last_error = Some(error);
            drop(session);
            publish_session(&status, SessionLifecycleState::Stopped, generation, attempts,
                None, paused, last_error.clone());
            break 'owner;
        }
        let capabilities = session.capabilities().cloned();
        status.lock().expect("P5-D session status lock poisoned").capabilities = capabilities.clone();
        publish_session(&status, SessionLifecycleState::Authenticated, generation, attempts,
            Some(session.session_id()), paused, None);
        if !capabilities.as_ref().is_some_and(|info| info.management_compatible) {
            last_error = Some("Receiver authenticated status is incompatible; update it through authorized ADB or manual installation.".into());
            drop(session);
            publish_session(&status, SessionLifecycleState::Stopped, generation, attempts,
                None, paused, last_error.clone());
            break 'owner;
        }
        if capabilities.as_ref().is_some_and(|info| !info.compatible) {
            let mut stop_owner = false;
            let mut restart = false;
            let mut management_error = None;
            let mut next_heartbeat = Instant::now() + timing.heartbeat_interval;
            loop {
                if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
                    stop_owner = true;
                    break;
                }
                if Instant::now() >= next_heartbeat {
                    if let Err(error) = session.heartbeat() {
                        management_error = Some(error);
                        restart = true;
                        break;
                    }
                    next_heartbeat = Instant::now() + timing.heartbeat_interval;
                    continue;
                }
                match commands.recv_timeout(next_heartbeat.saturating_duration_since(Instant::now()).min(SESSION_IO_POLL)) {
                    Ok(OwnerCommand::Stop) => { enabled.store(false, Ordering::Release); let _ = session.stop(); stop_owner = true; break; }
                    Ok(OwnerCommand::Reconnect) => { restart = true; break; }
                    Ok(OwnerCommand::UpdateEndpoint(next)) => { endpoint = next; restart = true; break; }
                    Ok(OwnerCommand::SetMode(mode)) => { parameters.mode = mode; restart = true; break; }
                    Ok(OwnerCommand::SetPaused(_)) => {}
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => { enabled.store(false, Ordering::Release); stop_owner = true; break; }
                }
            }
            session.close_local();
            drop(session);
            if stop_owner || !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() { break 'owner; }
            last_error = management_error.clone();
            publish_session(&status, SessionLifecycleState::Reconnecting, generation, attempts,
                None, paused, last_error.clone());
            if restart {
                if management_error.is_some() {
                    match wait_for_reconnect(lifecycle_retry_delay(&timing.reconnect_delays, retry_index), &commands,
                        &cancellation, &enabled, &mut endpoint, &mut parameters, &mut desired_paused) {
                        RetryWait::Stop => break 'owner,
                        RetryWait::ReconnectNow => retry_index = 0,
                        RetryWait::DelayElapsed => retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1),
                    }
                } else { retry_index = 0; }
            }
            continue 'owner;
        }
        if let Err(error) = session.bind_audio().and_then(|_| session.select_mode()) {
            last_error = Some(error);
            drop(session);
            if endpoint.connection_intent.is_some() && last_active_at.is_none() { break 'owner; }
            publish_session(&status, SessionLifecycleState::Reconnecting, generation, attempts,
                None, paused, last_error.clone());
            match wait_for_reconnect(lifecycle_retry_delay(&timing.reconnect_delays, retry_index), &commands, &cancellation,
                &enabled, &mut endpoint, &mut parameters, &mut desired_paused) {
                RetryWait::Stop => break,
                RetryWait::ReconnectNow => retry_index = 0,
                RetryWait::DelayElapsed => {
                    retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                    if recovery_window_expired(last_active_at, timing.recovery_window, Instant::now()) { break 'owner; }
                }
            }
            continue 'owner;
        }
        match apply_pending_commands(&commands, &mut endpoint, &mut parameters, &mut desired_paused, &enabled) {
            PendingCommands::Stop => break,
            PendingCommands::EndpointChanged => {
                drop(session);
                retry_index = 0;
                continue 'owner;
            }
            PendingCommands::ModeChanged => {
                drop(session);
                retry_index = 0;
                continue 'owner;
            }
            PendingCommands::Reconnect => {
                drop(session);
                retry_index = 0;
                continue 'owner;
            }
            PendingCommands::Applied => {}
        }
        if cancellation.is_cancelled() || !enabled.load(Ordering::Acquire) { break; }
        publish_session(&status, SessionLifecycleState::TransportConnected, generation, attempts,
            Some(session.session_id()), paused, None);
        if let Err(error) = session.start().and_then(|_| if desired_paused { session.pause() } else { Ok(()) }) {
            last_error = Some(error);
            drop(session);
            if endpoint.connection_intent.is_some() && last_active_at.is_none() { break 'owner; }
            publish_session(&status, SessionLifecycleState::Reconnecting, generation, attempts,
                None, paused, last_error.clone());
            match wait_for_reconnect(lifecycle_retry_delay(&timing.reconnect_delays, retry_index), &commands, &cancellation,
                &enabled, &mut endpoint, &mut parameters, &mut desired_paused) {
                RetryWait::Stop => break,
                RetryWait::ReconnectNow => retry_index = 0,
                RetryWait::DelayElapsed => {
                    retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                    if recovery_window_expired(last_active_at, timing.recovery_window, Instant::now()) { break 'owner; }
                }
            }
            continue 'owner;
        }
        paused = desired_paused;
        last_active_at = Some(Instant::now());
        let session_id = session.session_id();
        let mut last_session_bytes = 0;
        let session_queue = session.pcm_queue();
        *queue.lock().expect("P5-D PCM queue lock poisoned") = Some(session_queue);
        publish_session(&status, if paused { SessionLifecycleState::Paused } else { SessionLifecycleState::ReceiverRunning },
            generation, attempts, Some(session_id), paused, None);
        retry_index = 0;
        let mut endpoint_changed = false;
        let mut stop_owner = false;
        let mut next_heartbeat = Instant::now() + timing.heartbeat_interval;
        loop {
            if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
                stop_owner = true;
                break;
            }
            let sent = session.pcm_payload_bytes_sent();
            pcm_payload_bytes_sent.fetch_add(sent.saturating_sub(last_session_bytes), Ordering::Relaxed);
            last_session_bytes = sent;
            if Instant::now() >= next_heartbeat {
                match session.heartbeat() {
                    Ok(state) => {
                        match session.playback_progress() {
                            Ok(Some(mut diagnostics)) => {
                                let queue = session.pcm_queue();
                                let previous = status.lock().expect("P5-D session status lock poisoned")
                                    .playback_diagnostics.as_ref().and_then(|current| current.underrun_count);
                                diagnostics.sample_rate = Some(session.parameters.sample_rate);
                                diagnostics.channels = Some(session.parameters.channels);
                                diagnostics.pc_dropped_frames = Some(queue.dropped_frames());
                                diagnostics.pc_queued_frames = Some(queue.queued_frames());
                                diagnostics.underrun_increment = match (previous, diagnostics.underrun_count) {
                                    (Some(previous), Some(current)) if current >= previous => Some((current - previous) as u64),
                                    _ => None,
                                };
                                status.lock().expect("P5-D session status lock poisoned").playback_diagnostics = Some(diagnostics);
                            }
                            Ok(None) => {
                                status.lock().expect("P5-D session status lock poisoned").playback_diagnostics = None;
                            }
                            Err(error) => { last_error = Some(error); break; }
                        }
                        publish_session(&status, lifecycle_from_receiver(state), generation,
                            attempts, Some(session_id), paused, None);
                    }
                    Err(error) => {
                        last_error = Some(error);
                        break;
                    }
                }
                next_heartbeat = Instant::now() + timing.heartbeat_interval;
                continue;
            }
            let until_heartbeat = next_heartbeat.saturating_duration_since(Instant::now());
            match commands.recv_timeout(until_heartbeat.min(SESSION_IO_POLL)) {
                Ok(OwnerCommand::SetPaused(value)) => {
                    desired_paused = value;
                    if value == paused { continue; }
                    let result = if value { session.pause() } else { session.resume() };
                    match result {
                        Ok(()) => {
                            paused = value;
                            let state = if paused { SessionLifecycleState::Paused } else { SessionLifecycleState::ReceiverRunning };
                            publish_session(&status, state, generation, attempts, Some(session_id), paused, None);
                        }
                        Err(error) => { last_error = Some(error); break; }
                    }
                }
                Ok(OwnerCommand::UpdateEndpoint(next_endpoint)) => {
                    endpoint_changed = endpoint != next_endpoint;
                    endpoint = next_endpoint;
                    if !endpoint_changed { continue; }
                    if let Err(error) = session.stop() { last_error = Some(error); }
                    break;
                }
                Ok(OwnerCommand::SetMode(mode)) => {
                    if mode == parameters.mode { continue; }
                    parameters.mode = mode;
                    session.close_local();
                    endpoint_changed = true;
                    break;
                }
                Ok(OwnerCommand::Reconnect) => {
                    session.close_local();
                    endpoint_changed = true;
                    break;
                }
                Ok(OwnerCommand::Stop) => {
                    enabled.store(false, Ordering::Release);
                    if let Err(error) = session.stop() { last_error = Some(error); }
                    stop_owner = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    enabled.store(false, Ordering::Release);
                    stop_owner = true;
                    break;
                }
            }
        }
        session.close_local();
        let sent = session.pcm_payload_bytes_sent();
        pcm_payload_bytes_sent.fetch_add(sent.saturating_sub(last_session_bytes), Ordering::Relaxed);
        *queue.lock().expect("P5-D PCM queue lock poisoned") = None;
        drop(session);
        if stop_owner || !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() { break 'owner; }
        publish_session(&status, SessionLifecycleState::Reconnecting, generation, attempts,
            None, paused, last_error.clone());
        if endpoint_changed { retry_index = 0; continue 'owner; }
        match wait_for_reconnect(lifecycle_retry_delay(&timing.reconnect_delays, retry_index), &commands, &cancellation,
            &enabled, &mut endpoint, &mut parameters, &mut desired_paused) {
            RetryWait::Stop => break,
            RetryWait::ReconnectNow => retry_index = 0,
            RetryWait::DelayElapsed => {
                retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                if recovery_window_expired(last_active_at, timing.recovery_window, Instant::now()) { break 'owner; }
            }
        }
    }
    *queue.lock().expect("P5-D PCM queue lock poisoned") = None;
    publish_session(&status, SessionLifecycleState::Stopped, generation, attempts, None, paused, last_error);
}

fn lifecycle_from_receiver(state: ReceiverState) -> SessionLifecycleState {
    match state {
        ReceiverState::Authenticated => SessionLifecycleState::Authenticated,
        ReceiverState::TransportConnected => SessionLifecycleState::TransportConnected,
        ReceiverState::ReceiverRunning => SessionLifecycleState::ReceiverRunning,
        ReceiverState::Streaming => SessionLifecycleState::Streaming,
        ReceiverState::Paused => SessionLifecycleState::Paused,
        ReceiverState::Stopped => SessionLifecycleState::Stopped,
    }
}

fn publish_session(status: &Mutex<SessionSnapshot>, state: SessionLifecycleState, generation: u64,
    attempts: u64, session_id: Option<u64>, paused: bool, last_error: Option<String>) {
    let mut current = status.lock().expect("P5-D session status lock poisoned");
    if state == SessionLifecycleState::Reconnecting && session_id.is_none() {
        current.capabilities = None;
        current.playback_diagnostics = None;
    }
    current.state = state;
    current.generation = generation;
    current.attempts = attempts;
    current.session_id = session_id;
    current.paused = paused;
    current.last_error = last_error;
}

enum RetryWait { Stop, ReconnectNow, DelayElapsed }

enum PendingCommands { Stop, EndpointChanged, ModeChanged, Reconnect, Applied }

fn apply_pending_commands(commands: &mpsc::Receiver<OwnerCommand>, endpoint: &mut SessionEndpoint,
    parameters: &mut AudioParameters, paused: &mut bool, enabled: &AtomicBool) -> PendingCommands {
    let mut endpoint_changed = false;
    let mut mode_changed = false;
    let mut reconnect = false;
    loop {
        match commands.try_recv() {
            Ok(OwnerCommand::SetPaused(value)) => *paused = value,
            Ok(OwnerCommand::Stop) => {
                enabled.store(false, Ordering::Release);
                return PendingCommands::Stop;
            }
            Ok(OwnerCommand::UpdateEndpoint(next_endpoint)) => {
                endpoint_changed |= *endpoint != next_endpoint;
                *endpoint = next_endpoint;
            }
            Ok(OwnerCommand::SetMode(mode)) => {
                mode_changed |= parameters.mode != mode;
                parameters.mode = mode;
            }
            Ok(OwnerCommand::Reconnect) => reconnect = true,
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                enabled.store(false, Ordering::Release);
                return PendingCommands::Stop;
            }
        }
    }
    if reconnect { PendingCommands::Reconnect }
    else if endpoint_changed { PendingCommands::EndpointChanged }
    else if mode_changed { PendingCommands::ModeChanged }
    else { PendingCommands::Applied }
}

fn wait_for_reconnect(delay: Duration, commands: &mpsc::Receiver<OwnerCommand>, cancellation: &CancellationToken,
    enabled: &AtomicBool, endpoint: &mut SessionEndpoint, parameters: &mut AudioParameters, paused: &mut bool) -> RetryWait {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline {
        if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() { return RetryWait::Stop; }
        let remaining = deadline.saturating_duration_since(Instant::now()).min(SESSION_IO_POLL);
        match commands.recv_timeout(remaining) {
            Ok(OwnerCommand::SetPaused(value)) => *paused = value,
            Ok(OwnerCommand::Stop) => {
                enabled.store(false, Ordering::Release);
                return RetryWait::Stop;
            }
            Ok(OwnerCommand::UpdateEndpoint(next_endpoint)) => {
                if *endpoint != next_endpoint {
                    *endpoint = next_endpoint;
                    return RetryWait::ReconnectNow;
                }
            }
            Ok(OwnerCommand::SetMode(mode)) => {
                if parameters.mode != mode { parameters.mode = mode; return RetryWait::ReconnectNow; }
            }
            Ok(OwnerCommand::Reconnect) => return RetryWait::ReconnectNow,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => { enabled.store(false, Ordering::Release); return RetryWait::Stop; }
        }
    }
    RetryWait::DelayElapsed
}

fn lifecycle_retry_delay(delays: &[Duration; 4], attempt: usize) -> Duration {
    delays[attempt.min(delays.len() - 1)]
}

fn recovery_window_expired(active_since: Option<Instant>, window: Duration, now: Instant) -> bool {
    active_since.is_some_and(|active_since| now.saturating_duration_since(active_since) >= window)
}

struct Ack {
    result: u8,
    state: ReceiverState,
    udp_port: u16,
    playback_progress: Option<PlaybackDiagnostics>,
}

fn encode_control(key: &[u8; 32], direction: u32, session_id: u64, sequence: u64,
    kind: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_CONTROL_FRAME - CONTROL_HEADER_BYTES - CONTROL_TAG_BYTES {
        return Err("P5-C control payload is oversized.".into());
    }
    let mut frame = vec![0; CONTROL_HEADER_BYTES + payload.len() + CONTROL_TAG_BYTES];
    frame[..4].copy_from_slice(CONTROL_MAGIC);
    frame[4] = 1;
    frame[5] = kind;
    frame[8..12].copy_from_slice(&direction.to_be_bytes());
    frame[12..20].copy_from_slice(&session_id.to_be_bytes());
    frame[20..28].copy_from_slice(&sequence.to_be_bytes());
    frame[28..30].copy_from_slice(&(payload.len() as u16).to_be_bytes());
    frame[30..30 + payload.len()].copy_from_slice(payload);
    let tag = control_mac(key, &frame[..CONTROL_HEADER_BYTES + payload.len()])?;
    frame[CONTROL_HEADER_BYTES + payload.len()..].copy_from_slice(&tag);
    Ok(frame)
}

fn encode_capabilities_query(control_major: u16, audio_major: u8, required: u32) -> [u8; 8] {
    let mut payload = [0; 8];
    payload[..2].copy_from_slice(&control_major.to_be_bytes());
    payload[2] = audio_major;
    payload[4..].copy_from_slice(&required.to_be_bytes());
    payload
}

fn decode_capabilities_info(frame: &[u8], response_sequence: u64, query_sequence: u64)
    -> Result<ReceiverCapabilities, String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES {
        return Err("Receiver capability response frame was truncated.".into());
    }
    if frame[5] != CAPABILITIES_INFO {
        return Err("Receiver does not support authenticated capability negotiation; use authorized ADB or manual installation.".into());
    }
    if u64::from_be_bytes(frame[20..28].try_into().unwrap()) != response_sequence
        || u16::from_be_bytes([frame[28], frame[29]]) != 20 {
        return Err("Receiver capability response sequence or length was invalid.".into());
    }
    let payload = &frame[CONTROL_HEADER_BYTES..CONTROL_HEADER_BYTES + 20];
    let echoed_sequence = u64::from_be_bytes(payload[..8].try_into().unwrap());
    let control_major = u16::from_be_bytes(payload[8..10].try_into().unwrap());
    let audio_major = payload[10];
    let result = payload[11];
    let capability_bits = u32::from_be_bytes(payload[12..16].try_into().unwrap());
    let version_code = u32::from_be_bytes(payload[16..20].try_into().unwrap());
    if echoed_sequence != query_sequence || result > 1 {
        return Err("Receiver capability response sequence or result was invalid.".into());
    }
    let management_compatible = control_major == MANAGEMENT_CONTROL_MAJOR
        && capability_bits & REQUIRED_CAPABILITIES == REQUIRED_CAPABILITIES;
    let audio_compatible = audio_major == AUDIO_PROTOCOL_MAJOR;
    let compatible = result == RESULT_OK && management_compatible && audio_compatible;
    Ok(ReceiverCapabilities { control_major, audio_major, management_compatible, audio_compatible, compatible,
        capability_bits, version_code })
}

fn write_frame(tcp: &mut TcpStream, frame: &[u8], cancellation: &CancellationToken) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES || frame.len() > MAX_CONTROL_FRAME {
        return Err("P5-C control frame length is invalid.".into());
    }
    write_all_cancellable(tcp, &(frame.len() as u16).to_be_bytes(), cancellation)
        .map_err(|_| "P5-C control write failed.".to_string())?;
    write_all_cancellable(tcp, frame, cancellation)
        .map_err(|_| "P5-C control write failed.".to_string())?;
    tcp.flush().map_err(|_| "P5-C control write failed.".into())
}

fn read_raw_frame(tcp: &mut TcpStream, cancellation: &CancellationToken) -> Result<Vec<u8>, String> {
    let mut prefix = [0; 2];
    read_exact_cancellable(tcp, &mut prefix, cancellation)
        .map_err(|_| "P5-C control peer closed.".to_string())?;
    let length = u16::from_be_bytes(prefix) as usize;
    if !(CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES..=MAX_CONTROL_FRAME).contains(&length) {
        return Err("P5-C control frame length is invalid.".into());
    }
    let mut frame = vec![0; length];
    read_exact_cancellable(tcp, &mut frame, cancellation)
        .map_err(|_| "P5-C control peer closed.".to_string())?;
    Ok(frame)
}

fn read_frame(tcp: &mut TcpStream, key: &[u8; 32], direction: u32, session_id: u64,
    cancellation: &CancellationToken) -> Result<Vec<u8>, String> {
    let frame = read_raw_frame(tcp, cancellation)?;
    if &frame[..4] != CONTROL_MAGIC || frame[4] != 1 || frame[6..8] != [0, 0]
        || u32::from_be_bytes(frame[8..12].try_into().unwrap()) != direction
        || u64::from_be_bytes(frame[12..20].try_into().unwrap()) != session_id {
        return Err("P5-C control identity or header was rejected.".into());
    }
    let payload_len = u16::from_be_bytes([frame[28], frame[29]]) as usize;
    if frame.len() != CONTROL_HEADER_BYTES + payload_len + CONTROL_TAG_BYTES {
        return Err("P5-C control length was rejected.".into());
    }
    let body_len = CONTROL_HEADER_BYTES + payload_len;
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "P5-C HMAC key was invalid.")?;
    mac.update(&frame[..body_len]);
    mac.verify_slice(&frame[body_len..]).map_err(|_| "P5-C control authentication failed.")?;
    Ok(frame)
}

fn decode_ack(frame: &[u8], expected_sequence: u64, command: u8, command_sequence: u64) -> Result<Ack, String> {
    let payload_len = u16::from_be_bytes([frame[28], frame[29]]) as usize;
    if frame[5] != ACK || u64::from_be_bytes(frame[20..28].try_into().unwrap()) != expected_sequence
        || payload_len < 13 || match command {
            PLAYBACK_PROGRESS_QUERY => payload_len != 83,
            _ => payload_len != 13,
        } {
        return Err("P5-C ACK sequence or type was invalid.".into());
    }
    let payload = &frame[CONTROL_HEADER_BYTES..CONTROL_HEADER_BYTES + payload_len];
    if payload[0] != command || u64::from_be_bytes(payload[1..9].try_into().unwrap()) != command_sequence {
        return Err("P5-C ACK did not match its command.".into());
    }
    let state = ReceiverState::from_wire(payload[10])?;
    let playback_progress = if command == PLAYBACK_PROGRESS_QUERY {
        Some(decode_playback_progress(&payload[13..])?)
    } else { None };
    Ok(Ack { result: payload[9], state, udp_port: u16::from_be_bytes([payload[11], payload[12]]), playback_progress })
}

fn decode_playback_progress(data: &[u8]) -> Result<PlaybackDiagnostics, String> {
    if data.len() != 70 { return Err("Receiver playback progress payload length was invalid.".into()); }
    let advancing = match data[45] {
        0 => Some(false), 1 => Some(true), 0xff => None,
        _ => return Err("Receiver playback progress flag was invalid.".into()),
    };
    let u32_at = |offset| u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
    let u64_at = |offset| u64::from_be_bytes(data[offset..offset + 8].try_into().unwrap());
    let metric_u32 = |offset| { let value = u32_at(offset); (value != u32::MAX).then_some(value) };
    let metric_u64 = |offset| { let value = u64_at(offset); (value != u64::MAX).then_some(value) };
    Ok(PlaybackDiagnostics {
        receiver_sample_rate: metric_u32(0),
        receiver_channels: (data[4] != 0xff).then_some(data[4]),
        udp_jitter_target_frames: metric_u32(5),
        udp_expired_frames: metric_u64(9),
        udp_queued_frames: metric_u64(17),
        track_pending_frames: metric_u64(25),
        underrun_count: metric_u32(33),
        tcp_dropped_backlog_frames: metric_u64(37),
        playback_advancing: advancing,
        requested_buffer_frames: metric_u32(46),
        accepted_buffer_frames: metric_u32(50),
        effective_buffer_frames: metric_u32(54),
        capacity_frames: metric_u32(58),
        start_threshold_frames: metric_u32(62),
        pre_roll_frames: metric_u32(66),
        ..PlaybackDiagnostics::default()
    })
}

pub(crate) fn hex_string(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes { value.push(HEX[(byte >> 4) as usize] as char); value.push(HEX[(byte & 0xf) as usize] as char); }
    value
}

fn control_mac(key: &[u8; 32], bytes: &[u8]) -> Result<[u8; 32], String> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "P5-C HMAC key was invalid.")?;
    mac.update(bytes);
    Ok(mac.finalize().into_bytes().into())
}

fn connect_cancellable(local_address: Ipv4Addr, peer: SocketAddr,
    cancellation: &CancellationToken) -> std::io::Result<TcpStream> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
    let cancellation = cancellation.clone();
    runtime.block_on(async move {
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.bind(SocketAddr::from((local_address, 0)))?;
        tokio::select! {
            _ = cancellation.cancelled() => Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted, "P5-C connection cancelled")),
            result = tokio::time::timeout(CONNECT_TIMEOUT, socket.connect(peer)) => {
                match result {
                    Ok(result) => {
                        let tcp = result?.into_std()?;
                        tcp.set_nonblocking(false)?;
                        Ok(tcp)
                    }
                    Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "P5-C connect deadline expired")),
                }
            },
        }
    })
}

fn check_cancelled(cancellation: &CancellationToken) -> std::io::Result<()> {
    if cancellation.is_cancelled() {
        Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "P5-C operation cancelled"))
    } else { Ok(()) }
}

fn is_poll_timeout(error: &std::io::Error) -> bool {
    matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
}

fn read_exact_cancellable(tcp: &mut TcpStream, bytes: &mut [u8],
    cancellation: &CancellationToken) -> std::io::Result<()> {
    let deadline = Instant::now() + SESSION_IO_DEADLINE;
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(cancellation)?;
        if Instant::now() >= deadline { return Err(std::io::ErrorKind::TimedOut.into()); }
        match tcp.read(&mut bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => offset += read,
            Err(error) if is_poll_timeout(&error) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    check_cancelled(cancellation)
}

fn write_all_cancellable(tcp: &mut TcpStream, bytes: &[u8],
    cancellation: &CancellationToken) -> std::io::Result<()> {
    let deadline = Instant::now() + SESSION_IO_DEADLINE;
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(cancellation)?;
        if Instant::now() >= deadline { return Err(std::io::ErrorKind::TimedOut.into()); }
        match tcp.write(&bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(written) => offset += written,
            Err(error) if is_poll_timeout(&error) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    check_cancelled(cancellation)
}

#[cfg(test)]
fn expect_peer_close(tcp: &mut TcpStream) -> Result<(), String> {
    let mut byte = [0; 1];
    match tcp.read(&mut byte) {
        Ok(0) | Err(_) => Ok(()),
        Ok(_) => Err("P5-C invalid control unexpectedly received a response.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_progress_wire_metrics_keep_unknown_values_null() {
        let mut payload = vec![0xff; 70];
        payload[..4].copy_from_slice(&48_000u32.to_be_bytes());
        payload[4] = 2;
        payload[45] = 1;
        let progress = decode_playback_progress(&payload).unwrap();
        assert_eq!(progress.receiver_sample_rate, Some(48_000));
        assert_eq!(progress.receiver_channels, Some(2));
        assert_eq!(progress.playback_advancing, Some(true));
        assert_eq!(progress.udp_expired_frames, None);
        assert_eq!(progress.track_pending_frames, None);
        assert!(decode_playback_progress(&payload[..69]).is_err());
        payload[45] = 2;
        assert!(decode_playback_progress(&payload).is_err());
    }
    use crate::security::{test_binding_material, test_binding_material_with_wrong_identity,
        test_binding_material_with_wrong_secret};
    use std::{
        fs,
        io::{BufRead, BufReader},
        net::TcpListener,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
    };

    const JAVA_SOURCES: &[&str] = &[
        "PairingProtocol.java", "P5cControlProtocol.java", "UdpSessionContext.java",
        "UdpPacketDecoder.java", "UdpPcmQueue.java", "UdpReceiver.java",
        "P5cDiscovery.java", "P5cReceiverCoordinator.java",
    ];

    fn java_paths() -> (PathBuf, PathBuf) {
        let java_home = std::env::var_os("JAVA_HOME").expect("JAVA_HOME must point to the approved JDK 17");
        let bin = PathBuf::from(java_home).join("bin");
        #[cfg(windows)]
        let (javac, java) = (bin.join("javac.exe"), bin.join("java.exe"));
        #[cfg(not(windows))]
        let (javac, java) = (bin.join("javac"), bin.join("java"));
        (javac, java)
    }

    fn compile_java_endpoint() -> PathBuf {
        let (javac, _) = java_paths();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let package = repo.join("src/AnPod.Receiver/app/src/main/java/app/anpod/receiver");
        let output = std::env::temp_dir().join(format!("anpod-p5c-java-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output);
        fs::create_dir_all(&output).unwrap();
        let mut command = Command::new(javac);
        command.arg("-encoding").arg("UTF-8").arg("-source").arg("17").arg("-target").arg("17").arg("-d").arg(&output);
        for source in JAVA_SOURCES { command.arg(package.join(source)); }
        command.arg(repo.join("src/AnPod.AndroidProtocol/src/main/java/com/anpod/server/Pcm16FrameStream.java"));
        command.arg(repo.join("tests/P5cLoopbackEndpoint.java"));
        let result = command.output().expect("run JDK 17 javac for the P5-C Java receiver");
        assert!(result.status.success(), "javac failed:\n{}\n{}",
            String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        output
    }

    fn spawn_endpoint(classes: &Path) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
        spawn_endpoint_mode(classes, false, false, false, false, false, false)
    }

    fn spawn_endpoint_mode(classes: &Path, disconnect_on_stdin: bool, fail_resume: bool,
        discovery: bool, drop_stop_ack: bool, verify_mode_switch: bool, verify_audio_window: bool)
        -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
        let (_, java) = java_paths();
        let mut command = Command::new(java);
        command.arg("-cp").arg(classes).arg("app.anpod.receiver.P5cLoopbackEndpoint");
        if disconnect_on_stdin { command.arg("--drop-on-stdin"); }
        if fail_resume { command.arg("--fail-resume"); }
        if discovery { command.arg("--discovery"); }
        if drop_stop_ack { command.arg("--stop-ack-drop"); }
        if verify_mode_switch { command.arg("--verify-mode-switch"); }
        if verify_audio_window { command.arg("--verify-audio-window"); }
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let port: u16 = line.trim().strip_prefix("PORT=").expect("Java P5-C endpoint port").parse().unwrap();
        (child, stdout, SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
    }

    fn spawn_endpoint_until_disconnect(classes: &Path, fail_resume: bool) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
        spawn_endpoint_mode(classes, true, fail_resume, false, false, false, false)
    }

    fn spawn_discoverable_endpoint(classes: &Path) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
        spawn_endpoint_mode(classes, false, false, true, false, false, true)
    }

    fn spawn_mode_switch_endpoint(classes: &Path) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
        spawn_endpoint_mode(classes, false, false, true, false, true, false)
    }

    fn spawn_persistent_endpoint(classes: &Path) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
        spawn_endpoint_mode(classes, true, false, true, false, false, false)
    }

    fn spawn_stop_ack_drop_endpoint(classes: &Path) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
        spawn_endpoint_mode(classes, false, false, false, true, false, false)
    }

    fn spawn_capability_endpoint(classes: &Path, mode: &str)
        -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
        let (_, java) = java_paths();
        let mut child = Command::new(java).arg("-cp").arg(classes)
            .arg("app.anpod.receiver.P5cLoopbackEndpoint").arg(format!("--capabilities-{mode}"))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut stdout = stdout;
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let port: u16 = line.trim().strip_prefix("PORT=").expect("Java capability endpoint port").parse().unwrap();
        (child, stdout, SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
    }

    fn finish_endpoint(mut child: Child, mut stdout: BufReader<std::process::ChildStdout>) -> String {
        let mut result = String::new();
        stdout.read_line(&mut result).unwrap();
        let status = child.wait().unwrap();
        let mut stderr = String::new();
        if let Some(mut stderr_pipe) = child.stderr.take() { let _ = stderr_pipe.read_to_string(&mut stderr); }
        assert!(status.success(), "Java P5-C test endpoint did not finish successfully: {stderr}");
        result.trim().strip_prefix("RESULT=").expect("Java endpoint result").to_string()
    }

    fn verify_java_v2_discovery(classes: &Path) {
        let host_address = Ipv4Addr::new(127, 0, 0, 2);
        let receiver_address = Ipv4Addr::new(127, 0, 0, 1);
        let interface = crate::network_interface::interface_for_test(host_address, 8);
        let listener = crate::discovery::HostOfferListener::start_loopback_for_test(&interface).unwrap();
        let (_, java) = java_paths();
        let output = Command::new(java).arg("-cp").arg(classes)
            .arg("app.anpod.receiver.P5cLoopbackEndpoint")
            .arg("--verify-v2-discovery").arg(host_address.to_string())
            .output().expect("run Java V2 discovery loopback check");
        assert!(output.status.success(), "Java discovery check failed: {}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("JAVA_V2_DISCOVERY=PASS"),
            "Java discovery check did not complete: {}", String::from_utf8_lossy(&output.stdout));
        let deadline = Instant::now() + Duration::from_secs(1);
        let offer = loop {
            if let Some(offer) = listener.snapshot().offers.into_iter().next() { break offer; }
            assert!(Instant::now() < deadline, "Java CONNECT_OFFER did not reach the Rust host listener");
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(offer.endpoint, SocketAddr::from((receiver_address, 45_678)));
        assert!(listener.take_offer(&offer.id).is_some());
        assert!(listener.take_offer(&offer.id).is_none(), "Java discovery offer remains single-use");
        drop(listener);
        let output = Command::new(java_paths().1).arg("-cp").arg(classes)
            .arg("app.anpod.receiver.P5cLoopbackEndpoint")
            .arg("--verify-coordinator-discovery-recovery")
            .output().expect("run Java discovery recovery check after closing the host listener");
        assert!(output.status.success(), "Java discovery recovery check failed: {}",
            String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("JAVA_P5C_DISCOVERY_RECOVERY=PASS"));
    }

    fn rejected_handshake(classes: &Path, wrong_secret: bool) {
        let (child, stdout, peer) = spawn_endpoint(classes);
        let material = if wrong_secret { test_binding_material_with_wrong_secret() }
            else { test_binding_material_with_wrong_identity() };
        assert!(HostSession::authenticate_loopback(peer, material, AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }).is_err());
        let result = finish_endpoint(child, stdout);
        assert!(result.starts_with("session_failed,0,0,0,0,"), "unauthenticated handshake must not stop playback: {result}");
    }

    #[test]
    fn java_receiver_and_rust_sender_share_authenticated_session_and_cleanup() {
        crate::network::verify_capture_queue_bounds();
        let classes = compile_java_endpoint();
        verify_java_v2_discovery(&classes);
        rejected_handshake(&classes, false);
        rejected_handshake(&classes, true);

        let mut synthetic_interface = crate::network_interface::interface_for_test(Ipv4Addr::new(192, 168, 41, 12), 24);
        synthetic_interface.gateways.push(Ipv4Addr::new(192, 168, 41, 1));
        assert!(synthetic_interface.contains(Ipv4Addr::new(192, 168, 41, 200)));
        assert_eq!(crate::discovery::probe_targets(&synthetic_interface),
            [Ipv4Addr::new(192, 168, 41, 1), Ipv4Addr::new(192, 168, 41, 255)]);
        assert!(!synthetic_interface.contains(Ipv4Addr::new(192, 168, 42, 1)));

        for send_bad_mac in [true, false] {
            let (child, stdout, peer) = spawn_endpoint(&classes);
            let mut host = HostSession::authenticate_loopback(peer, test_binding_material(), AudioParameters {
                sample_rate: 48_000, channels: 2, mode: 0,
            }).unwrap();
            if send_bad_mac { host.send_bad_mac_for_test().unwrap(); }
            else { host.send_old_session_for_test().unwrap(); }
            drop(host);
            let result = finish_endpoint(child, stdout);
            assert!(result.starts_with("session_failed,0,0,0,0,"), "authenticated but unstarted session must not stop playback: {result}");
        }

        let mut interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
        interface.gateways.push(Ipv4Addr::LOCALHOST);
        let unrelated_probe = UdpSocket::bind((Ipv4Addr::LOCALHOST, crate::discovery::DISCOVERY_PORT)).unwrap();
        let unrelated = thread::spawn(move || {
            let mut probe = [0; 128];
            let (length, source) = unrelated_probe.recv_from(&mut probe).unwrap();
            assert_eq!(length, 26);
            unrelated_probe.send_to(&crate::discovery::candidate_for_test([0xA5; 16], 43210), source).unwrap();
        });
        let no_candidates = crate::discovery::discover_on_interface(&interface, &[Ipv4Addr::LOCALHOST],
            Duration::from_millis(120), &CancellationToken::new());
        unrelated.join().unwrap();
        assert!(no_candidates.is_err(), "a response with an unrelated probe token is not a candidate");

        let flood_responder = UdpSocket::bind((Ipv4Addr::LOCALHOST, crate::discovery::DISCOVERY_PORT)).unwrap();
        let flood = thread::spawn(move || {
            let mut probe = [0; 128];
            let (length, target) = flood_responder.recv_from(&mut probe).unwrap();
            assert_eq!(length, 26);
            let token: [u8; 16] = probe[8..24].try_into().unwrap();
            for port in 10_000..10_032 {
                flood_responder.send_to(&crate::discovery::candidate_for_test(token, port), target).unwrap();
            }
        });
        let bounded_candidates = crate::discovery::discover_on_interface(&interface, &[Ipv4Addr::LOCALHOST],
            Duration::from_millis(250), &CancellationToken::new()).unwrap();
        flood.join().unwrap();
        assert_eq!(bounded_candidates.len(), 16, "discovery caps candidates and stops accepting a response flood");

        let discovery_cancel = CancellationToken::new();
        let cancel_token = discovery_cancel.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(15));
            cancel_token.cancel();
        });
        let start = Instant::now();
        let cancelled_discovery = crate::discovery::discover_on_interface(&interface, &[Ipv4Addr::LOCALHOST],
            Duration::from_secs(1), &discovery_cancel);
        canceller.join().unwrap();
        assert!(cancelled_discovery.as_ref().is_err_and(|error| error == "Network discovery was cancelled."),
            "cancelled discovery returned {:?}", cancelled_discovery.as_ref().err());
        assert!(start.elapsed() < Duration::from_millis(250), "discovery cancellation must end at the bounded receive poll");

        let (child, stdout, advertised_peer) = spawn_discoverable_endpoint(&classes);
        let malformed_sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        malformed_sender.send_to(&[0; 26], (Ipv4Addr::LOCALHOST, crate::discovery::DISCOVERY_PORT)).unwrap();
        let candidates = crate::discovery::discover_on_interface(&interface, &[Ipv4Addr::LOCALHOST],
            Duration::from_millis(250), &CancellationToken::new()).unwrap();
        assert_eq!(candidates.len(), 1, "only a response correlated to the fresh probe is returned");
        let peer = candidates[0].endpoint;
        assert_eq!(peer, advertised_peer, "discovery provides only the endpoint candidate");
        let mut host = HostSession::connect_on_interface(&interface, peer, test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }).unwrap();
        let capabilities = host.capabilities().expect("authenticated capability preflight before BIND");
        assert_eq!(capabilities.control_major, 3);
        assert_eq!(capabilities.audio_major, 1);
        assert_eq!(capabilities.capability_bits, 3 | CAP_PLAYBACK_PROGRESS);
        assert_eq!(capabilities.version_code, 0, "localhost fixture versionCode is informational only");
        assert_eq!(host.state(), ReceiverState::TransportConnected);
        host.start_without_format_for_test().unwrap();
        assert_eq!(host.state(), ReceiverState::ReceiverRunning);
        let progress = host.playback_progress().unwrap().expect("negotiated playback progress query");
        assert_eq!(progress.receiver_sample_rate, None, "fixture reports unknown instead of inventing metrics");
        assert_eq!(progress.playback_advancing, None);
        host.resend_last_for_test().unwrap();
        host.send_old_audio_for_test().unwrap();
        thread::sleep(Duration::from_millis(100));
        let samples = vec![0x1234i16; 960 * 2];
        assert!(host.pcm_queue().push_raw(samples.as_ptr(), 960, 48_000, 2, 0));
        let mut state = ReceiverState::ReceiverRunning;
        for _ in 0..10 {
            state = host.heartbeat().unwrap();
            if state == ReceiverState::Streaming { break; }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(state, ReceiverState::Streaming, "streaming follows synthetic PCM sink output");
        host.send_format_packet_for_test(44_100).unwrap();
        host.send_format_packet_for_test(48_000).unwrap();
        host.send_format_packet_for_test(48_000).unwrap();
        assert_eq!(host.heartbeat().unwrap(), ReceiverState::ReceiverRunning,
            "stale playback activity must return to waiting while authenticated control remains alive");
        host.pause().unwrap();
        assert_eq!(host.state(), ReceiverState::Paused);
        host.resume().unwrap();
        assert_eq!(host.state(), ReceiverState::ReceiverRunning);
        host.stop().unwrap();
        assert!(host.pcm_payload_bytes_sent() >= 3_840,
            "sender byte count must survive sender teardown");
        assert_eq!(host.state(), ReceiverState::Stopped);
        drop(host);
        let result = finish_endpoint(child, stdout);
        let fields: Vec<_> = result.split(',').collect();
        assert_eq!(fields.len(), 8, "unexpected merged P5-C result: {result}");
        assert_eq!(fields[0], "");
        assert_eq!(fields[1..5], ["1", "1", "1", "1"]);
        assert!(fields[5].parse::<usize>().unwrap() >= 3_840, "synthetic PCM was not drained: {result}");
        assert_eq!(fields[6], "true", "STOP must release the receiver UDP socket");
        assert_eq!(fields[7], "1", "authenticated STOP must shut down the receiver listener");

        verify_capability_negotiation(&classes);
        verify_connection_intent(&classes);

        same_coordinator_accepts_a_fresh_authenticated_session(&classes);
        stop_ack_failure_still_shuts_down_listener(&classes);
        mode_change_uses_a_fresh_session_and_preserves_pause(&classes);
        stop_disconnect_clears_active_local_session(&classes);
        start_handoff_failure_closes_session(&classes);
        lifecycle_owner_recovers_fresh_session_and_stops(&classes);
        let _ = fs::remove_dir_all(classes);
    }

    fn verify_connection_intent(classes: &Path) {
        for mode in ["valid", "wrong", "missing", "cancelled", "handoff-cancelled", "expired"] {
            let mut command = Command::new(java_paths().1);
            command.arg("-cp").arg(classes).arg("app.anpod.receiver.P5cLoopbackEndpoint").arg("--connection-intent");
            if mode == "cancelled" { command.arg("--cancel-intent-on-stdin"); }
            if mode == "handoff-cancelled" { command.arg("--cancel-before-handoff"); }
            let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let mut line = String::new();
            stdout.read_line(&mut line).unwrap();
            let peer = SocketAddr::from((Ipv4Addr::LOCALHOST, line.trim().strip_prefix("PORT=").unwrap().parse::<u16>().unwrap()));
            let mut endpoint = SessionEndpoint::loopback(peer);
            if mode != "missing" { endpoint.connection_intent = Some(([if mode == "wrong" { 0x59 } else { 0x5a }; 16], true)); }
            let mut host = HostSession::authenticate_on_network(endpoint, test_binding_material(), AudioParameters {
                sample_rate: 48_000, channels: 2, mode: 0,
            }, CancellationToken::new()).unwrap();
            let result = host.negotiate_capabilities();
            if mode == "wrong" { assert!(result.is_err(), "wrong intent must be rejected before BIND"); }
            else {
                result.unwrap();
                if mode == "expired" {
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while Instant::now() < deadline { host.heartbeat().unwrap(); thread::sleep(Duration::from_millis(200)); }
                }
                if mode == "missing" || mode == "expired" {
                    assert!(host.bind_audio().is_err(), "missing or expired intent must block BIND");
                } else {
                    host.bind_audio().unwrap();
                    host.select_mode().unwrap();
                    if mode == "cancelled" {
                        child.stdin.as_mut().unwrap().write_all(b"x").unwrap();
                        line.clear(); stdout.read_line(&mut line).unwrap();
                        assert_eq!(line.trim(), "INTENT_CANCELLED");
                        assert!(host.start().is_err(), "cancelled intent must block delayed START");
                    } else if mode == "handoff-cancelled" {
                        assert!(host.start().is_err(), "a queued start must recheck cancellation at the playback handoff");
                    } else { host.start().unwrap(); host.stop().unwrap(); }
                }
            }
            drop(host);
            let result = finish_endpoint(child, stdout);
            let fields: Vec<_> = result.split(',').collect();
            assert_eq!(fields[1], if mode == "valid" { "1" } else { "0" }, "unexpected playback for {mode}: {result}");
        }
        // An automatic caller must not fall back when an older Receiver lacks the contract.
        let (child, stdout, peer) = spawn_endpoint(classes);
        let mut endpoint = SessionEndpoint::loopback(peer);
        endpoint.connection_intent = Some(([0x5a; 16], true));
        assert!(HostSession::connect_endpoint(endpoint, test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }, CancellationToken::new()).is_err());
        assert!(finish_endpoint(child, stdout).starts_with("session_failed,0,0,0,0,"));
    }

    fn verify_capability_negotiation(classes: &Path) {
        assert_eq!(encode_capabilities_query(3, 1, CAP_AUTHENTICATED_STATUS),
            [0, 3, 1, 0, 0, 0, 0, 2]);
        for (mode, diagnostic) in [
            ("incompatible", "incompatible"),
            ("missing-capability", "incompatible"),
            ("unsupported", "does not support"),
            ("replay", "sequence"),
        ] {
            let (child, stdout, peer) = spawn_capability_endpoint(classes, mode);
            let error = HostSession::connect_loopback(peer, test_binding_material(), AudioParameters {
                sample_rate: 48_000, channels: 2, mode: 0,
            }).err().expect("capability mismatch, unsupported query, or replay must block BIND/START");
            assert!(error.to_ascii_lowercase().contains(diagnostic), "{mode} gave unexpected diagnostic: {error}");
            let result = finish_endpoint(child, stdout);
            assert!(result.starts_with("session_failed,0,0,0,0,"), "{mode} must not start AudioTrack: {result}");
        }

        for bad_mac in [false, true] {
            let (child, stdout, peer) = spawn_endpoint(classes);
            let mut host = HostSession::authenticate_loopback(peer, test_binding_material(), AudioParameters {
                sample_rate: 48_000, channels: 2, mode: 0,
            }).unwrap();
            host.send_bad_capabilities_query_for_test(bad_mac).unwrap();
            drop(host);
            let result = finish_endpoint(child, stdout);
            assert!(result.starts_with("session_failed,0,0,0,0,"), "malformed or unauthenticated capability query must be closed: {result}");
        }

        let (child, stdout, peer) = spawn_capability_endpoint(classes, "incompatible");
        let mut owner = SessionOwner::start(peer, test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }, CancellationToken::new()).unwrap();
        let authenticated = wait_for_owner(&owner, |snapshot| snapshot.state == SessionLifecycleState::Authenticated
            && snapshot.capabilities.is_some());
        let capabilities = authenticated.capabilities.unwrap();
        assert!(capabilities.management_compatible);
        assert!(!capabilities.audio_compatible);
        assert!(!capabilities.compatible);
        assert!(owner.pcm_queue().is_none(), "audio-incompatible management session must never publish a PCM queue");
        thread::sleep(Duration::from_millis(1_100));
        assert_eq!(owner.snapshot().state, SessionLifecycleState::Authenticated,
            "authenticated management-only session must remain alive through heartbeats");
        owner.stop();
        let result = finish_endpoint(child, stdout);
        let fields: Vec<_> = result.split(',').collect();
        assert_eq!(fields[1], "0", "management-only negotiation must not start AudioTrack: {result}");
    }

    fn same_coordinator_accepts_a_fresh_authenticated_session(classes: &Path) {
        let (mut child, stdout, peer) = spawn_persistent_endpoint(classes);
        let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
        for _ in 0..2 {
            let session = HostSession::connect_on_interface(&interface, peer, test_binding_material(), AudioParameters {
                sample_rate: 48_000, channels: 2, mode: 0,
            }).unwrap();
            assert_eq!(session.state(), ReceiverState::TransportConnected);
            drop(session);
        }
        child.stdin.as_mut().unwrap().write_all(&[1]).unwrap();
        drop(child.stdin.take());
        let result = finish_endpoint(child, stdout);
        let fields: Vec<_> = result.split(',').collect();
        assert_eq!(fields[1], "0", "pre-START reauthentication must not start playback");
    }

    fn stop_ack_failure_still_shuts_down_listener(classes: &Path) {
        let (child, stdout, peer) = spawn_stop_ack_drop_endpoint(classes);
        let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
        let mut session = HostSession::connect_on_interface(&interface, peer, test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }).unwrap();
        session.start().unwrap();
        assert!(session.stop().is_err(), "test endpoint must drop the authenticated STOP ACK");
        drop(session);
        let result = finish_endpoint(child, stdout);
        let fields: Vec<_> = result.split(',').collect();
        assert_eq!(fields.len(), 8, "unexpected STOP ACK failure result: {result}");
        assert_eq!(fields[4], "1", "STOP playback must be cleaned before ACK failure");
        assert_eq!(fields[7], "1", "STOP ACK failure must still close the listener");
    }

    fn mode_change_uses_a_fresh_session_and_preserves_pause(classes: &Path) {
        let (child, stdout, peer) = spawn_mode_switch_endpoint(classes);
        let interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
        let mut owner = SessionOwner::start_on_interface(&interface, peer, test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 1,
        }, CancellationToken::new()).unwrap();
        let before = wait_for_owner(&owner, |status| status.state == SessionLifecycleState::ReceiverRunning);

        // One 10 ms synthetic WASAPI capture block is split into two 5 ms UDP
        // packets and sent as a burst, as the production sender does.
        let old_queue = owner.pcm_queue().unwrap();
        let first_block = vec![0x2345i16; 480 * 2];
        assert!(old_queue.push_raw(first_block.as_ptr(), 480, 48_000, 2, 0));
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Streaming);
        thread::sleep(Duration::from_millis(10));
        assert!(old_queue.push_raw(first_block.as_ptr(), 480, 48_000, 2, 480));
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Streaming);

        owner.set_mode(1).unwrap();
        thread::sleep(Duration::from_millis(30));
        let same_mode = owner.snapshot();
        assert_eq!(same_mode.generation, before.generation,
            "reselecting the active playback mode must not rebuild the session");
        assert_eq!(same_mode.session_id, before.session_id,
            "reselecting the active playback mode must keep the authenticated session");

        owner.set_paused(true).unwrap();
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Paused);
        owner.set_mode(0).unwrap();
        let after = wait_for_owner(&owner, |status| status.generation > before.generation
            && status.session_id.is_some() && status.state == SessionLifecycleState::Paused);
        assert_ne!(after.session_id, before.session_id, "playback mode must use a fresh authenticated session");
        assert!(after.paused, "the user's pause intent must survive fresh mode authentication");
        assert!(!old_queue.push_raw(first_block.as_ptr(), 1, 48_000, 2, 960),
            "mode change must stop and clear the previous PCM queue");

        let mode_queue = owner.pcm_queue().unwrap();
        owner.set_paused(false).unwrap();
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::ReceiverRunning);
        let samples = vec![0x3456i16; 960 * 2];
        assert!(mode_queue.push_raw(samples.as_ptr(), 960, 48_000, 2, 0));
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Streaming);
        owner.set_paused(true).unwrap();
        let paused = wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Paused);

        owner.reconnect().unwrap();
        let resumed = wait_for_owner(&owner, |status| status.generation > paused.generation
            && status.session_id.is_some() && status.state == SessionLifecycleState::Paused);
        assert_ne!(resumed.session_id, paused.session_id, "output recovery must use a fresh authenticated session");
        let resumed_queue = owner.pcm_queue().unwrap();
        assert!(!Arc::ptr_eq(&mode_queue, &resumed_queue), "fresh session must publish a new PCM queue");
        owner.set_paused(false).unwrap();
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::ReceiverRunning);
        assert!(resumed_queue.push_raw(samples.as_ptr(), 960, 48_000, 2, 0),
            "new session audio frame zero must be accepted by its fresh queue");
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Streaming);
        owner.stop();
        assert_eq!(owner.pcm_payload_bytes_sent(), 11_520,
            "initial, mode-change, and recovered session PCM payloads must be counted exactly once");
        let result = finish_endpoint(child, stdout);
        let fields: Vec<_> = result.split(',').collect();
        assert_eq!(fields[1], "3", "mode change and output recovery must start fresh playback sessions: {result}");
        assert_eq!(fields[5].parse::<usize>().unwrap(), 11_520,
            "initial, mode-change, and recovered frame-zero PCM payloads must reach the fake sink exactly once: {result}");
    }

    fn wait_for_owner(owner: &SessionOwner, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let snapshot = owner.snapshot();
            if predicate(&snapshot) { return snapshot; }
            assert!(Instant::now() < deadline, "P5-D owner state timed out: {snapshot:?}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn lifecycle_owner_recovers_fresh_session_and_stops(classes: &Path) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let peer = listener.local_addr().unwrap();
        let silent_peer = thread::spawn(move || {
            let (_socket, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(300));
        });
        let mut tcp = connect_cancellable(Ipv4Addr::LOCALHOST, peer, &CancellationToken::new()).unwrap();
        tcp.set_read_timeout(Some(SESSION_IO_POLL)).unwrap();
        let start = Instant::now();
        let read = tcp.read(&mut [0]);
        assert!(is_poll_timeout(&read.unwrap_err()), "silent peer read did not wait for its poll timeout");
        assert!(start.elapsed() >= Duration::from_millis(25), "nonblocking socket spun through WouldBlock immediately");
        let cancellation = CancellationToken::new();
        let cancel_after = cancellation.clone();
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            cancel_after.cancel();
        });
        let start = Instant::now();
        let cancelled = read_exact_cancellable(&mut tcp, &mut [0], &cancellation);
        assert_eq!(cancelled.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert!(start.elapsed() < Duration::from_millis(250), "blocking poll did not end promptly after cancellation");
        cancel_thread.join().unwrap();
        drop(tcp);
        silent_peer.join().unwrap();

        let timing = SessionTiming {
            heartbeat_interval: Duration::from_millis(20),
            reconnect_delays: [Duration::from_millis(20), Duration::from_millis(30),
                Duration::from_millis(40), Duration::from_millis(50)],
            recovery_window: Duration::from_secs(5),
        };
        assert_eq!(lifecycle_retry_delay(&RECONNECT_DELAYS, 0), Duration::from_secs(1));
        assert_eq!(lifecycle_retry_delay(&RECONNECT_DELAYS, 1), Duration::from_secs(2));
        assert_eq!(lifecycle_retry_delay(&RECONNECT_DELAYS, 2), Duration::from_secs(5));
        assert_eq!(lifecycle_retry_delay(&RECONNECT_DELAYS, 3), Duration::from_secs(10));
        assert_eq!(lifecycle_retry_delay(&RECONNECT_DELAYS, 8), Duration::from_secs(10));

        let loopback_interface = crate::network_interface::interface_for_test(Ipv4Addr::LOCALHOST, 32);
        let (mut first, first_stdout, first_peer) = spawn_endpoint_until_disconnect(classes, true);
        let cancellation = CancellationToken::new();
        let mut owner = SessionOwner::start_on_interface(&loopback_interface, first_peer,
            test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }, cancellation.clone()).unwrap();
        let initial = wait_for_owner(&owner, |status| status.state == SessionLifecycleState::ReceiverRunning);
        let old_session = initial.session_id.unwrap();
        let old_queue = owner.pcm_queue().unwrap();
        owner.set_paused(true).unwrap();
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Paused);
        owner.set_paused(false).unwrap();
        let failed_resume = wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Reconnecting && status.session_id.is_none());
        assert!(failed_resume.paused, "failed RESUME must not claim the receiver is running");
        assert_eq!(failed_resume.last_error.as_deref(), Some("P5-C RESUME was rejected."),
            "Java fake endpoint must return the deterministic failed RESUME ACK");
        first.stdin.as_mut().unwrap().write_all(b"drop").unwrap();
        let first_result = finish_endpoint(first, first_stdout);
        assert!(first_result.starts_with("session_failed,1,"), "first endpoint did not drop the session: {first_result}");
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Reconnecting && status.session_id.is_none());
        let stale_samples = [1i16, 1];
        assert!(!old_queue.push_raw(stale_samples.as_ptr(), 1, 48_000, 2, 0),
            "disconnected generation PCM queue must already be stopped");

        let (second, second_stdout, second_peer) = spawn_endpoint(classes);
        owner.update_network_endpoint(&loopback_interface, second_peer).unwrap();
        let recovered = wait_for_owner(&owner, |status| status.session_id.is_some()
            && status.session_id != Some(old_session) && status.state == SessionLifecycleState::ReceiverRunning);
        assert!(recovered.generation > initial.generation);
        assert!(recovered.attempts > initial.attempts);
        let new_queue = owner.pcm_queue().unwrap();
        assert!(!Arc::ptr_eq(&old_queue, &new_queue), "recovery must publish a new PCM queue");
        owner.set_paused(false).unwrap();
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::ReceiverRunning);
        let samples = vec![0x2345i16; 960 * 2];
        assert!(new_queue.push_raw(samples.as_ptr(), 960, 48_000, 2, 0));
        wait_for_owner(&owner, |status| status.state == SessionLifecycleState::Streaming);

        owner.stop();
        let stopped = owner.snapshot();
        assert_eq!(stopped.state, SessionLifecycleState::Stopped);
        assert!(owner.pcm_queue().is_none());
        thread::sleep(Duration::from_millis(100));
        assert_eq!(owner.snapshot().attempts, stopped.attempts, "explicit stop must forbid recovery");
        let second_result = finish_endpoint(second, second_stdout);
        let stopped_fields: Vec<_> = second_result.split(',').collect();
        assert_eq!(stopped_fields[0], "", "owner STOP should reach the receiver: {second_result}");
        assert_eq!(stopped_fields[1], "1");
        assert_eq!(stopped_fields[4], "1");

        let cancel_token = CancellationToken::new();
        let mut cancelled_owner = SessionOwner::start_with_timing(first_peer, test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }, cancel_token.clone(), timing).unwrap();
        let attempting = wait_for_owner(&cancelled_owner, |status| status.attempts > 0 && status.last_error.is_some());
        cancel_token.cancel();
        cancelled_owner.join();
        let cancelled = cancelled_owner.snapshot();
        assert_eq!(cancelled.state, SessionLifecycleState::Stopped);
        assert_eq!(cancelled.attempts, attempting.attempts, "caller cancellation must terminate the retry task");
    }

    fn stop_disconnect_clears_active_local_session(classes: &Path) {
        let (child, stdout, peer) = spawn_endpoint(classes);
        let mut host = HostSession::connect_loopback(peer, test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }).unwrap();
        host.start().unwrap();
        host.remove_control_for_test();
        let stop_error = host.stop().expect_err("disconnected STOP must report the control failure");
        assert!(stop_error.contains("control channel is closed"), "STOP lost its channel diagnostic: {stop_error}");
        assert_eq!(host.state(), ReceiverState::Stopped);
        assert!(host.sender.is_none() && host.udp_socket.is_none() && host.tcp.is_none());
        assert_eq!(host.keys.audio, [0; 32]);
        assert_eq!(host.keys.control_tx, [0; 32]);
        assert_eq!(host.keys.control_rx, [0; 32]);
        let samples = [1i16, 1];
        assert!(!host.pcm_queue().push_raw(samples.as_ptr(), 1, 48_000, 2, 0));
        drop(host);
        let result = finish_endpoint(child, stdout);
        assert!(result.starts_with("session_failed,1,0,0,1,"), "remote session did not clean up: {result}");
    }

    fn start_handoff_failure_closes_session(classes: &Path) {
        let (child, stdout, peer) = spawn_endpoint(classes);
        let mut host = HostSession::connect_loopback(peer, test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }).unwrap();
        host.remove_udp_socket_for_test();
        assert!(host.start().is_err(), "missing UDP socket must fail local START handoff");
        assert_eq!(host.state(), ReceiverState::Stopped);
        assert!(host.sender.is_none() && host.udp_socket.is_none() && host.tcp.is_none());
        assert_eq!(host.keys.audio, [0; 32]);
        let samples = [1i16, 1];
        assert!(!host.pcm_queue().push_raw(samples.as_ptr(), 1, 48_000, 2, 0));
        drop(host);
        let result = finish_endpoint(child, stdout);
        assert!(result.starts_with("session_failed,1,0,0,1,"), "failed START handoff did not stop remote playback: {result}");
    }
}
