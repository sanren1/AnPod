#[cfg(test)]
use crate::protocol::{PacketEncoder, PacketType, HOST_TO_RECEIVER};
use crate::{
    network::{FloatPcmQueue, FloatUdpSender, PcmQueue, UdpSender, UdpSessionMaterial},
    network_interface::Ipv4Interface,
    protocol::{AudioFormatV2, PcmFormat},
    receiver::PlaybackDiagnostics,
    security::{AudioParameters, BindingMaterial, HostHandshake, HostHandshakeV2},
};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
#[cfg(test)]
use std::net::IpAddr;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub(crate) const CONTROL_MAGIC: &[u8; 4] = b"ANPC";
const CONTROL_HEADER_BYTES: usize = 30;
const CONTROL_TAG_BYTES: usize = 32;
const MAX_CONTROL_FRAME: usize = 1200;
pub(crate) const HOST_TO_RECEIVER_CONTROL: u32 = 0x414e_5048;
pub(crate) const RECEIVER_TO_HOST_CONTROL: u32 = 0x414e_5052;
pub(crate) const BIND: u8 = 1;
pub(crate) const SELECT_MODE: u8 = 2;
pub(crate) const START: u8 = 3;
pub(crate) const PAUSE: u8 = 4;
pub(crate) const RESUME: u8 = 5;
pub(crate) const STOP: u8 = 6;
pub(crate) const HEARTBEAT: u8 = 7;
pub(crate) const CAPABILITIES_QUERY: u8 = 8;
pub(crate) const CAPABILITIES_INFO: u8 = 9;
pub(crate) const PLAYBACK_PROGRESS_QUERY: u8 = 16;
const CONNECTION_INTENT: u8 = 17;
const BACKGROUND_START_REQUEST: u8 = 18;
const BACKGROUND_START_STATUS: u8 = 19;
const BACKGROUND_START_CANCEL: u8 = 20;
const BACKGROUND_TRUST_QUERY: u8 = 21;
const BACKGROUND_TRUST_REVOKE: u8 = 22;
pub(crate) const ACK: u8 = 127;
pub(crate) const RESULT_OK: u8 = 0;
const RESULT_REJECTED: u8 = 1;
const RESULT_FAILED: u8 = 2;
pub(crate) const RESULT_USER_STOPPED: u8 = 3;
pub(crate) const MANAGEMENT_CONTROL_MAJOR: u16 = 3;
pub(crate) const AUDIO_PROTOCOL_MAJOR: u8 = 1;
const CAP_RECEIVER_UI: u32 = 0x0000_0001;
pub(crate) const CAP_AUTHENTICATED_STATUS: u32 = 0x0000_0002;
pub(crate) const CAP_PLAYBACK_PROGRESS: u32 = 0x0000_0008;
const CAP_CONNECTION_INTENT: u32 = 0x0000_0010;
const CAP_BACKGROUND_START_AUTH: u32 = 0x0000_0020;
const CAP_BACKGROUND_TRUST_MANAGEMENT: u32 = 0x0000_0040;
pub(crate) const CAP_AUDIO_V2: u32 = 0x0000_0080;
pub(crate) const CAP_TERMINAL_CAUSE: u32 = 0x0000_0100;
pub(crate) const CAP_AOA_SESSION_RESTART: u32 = 0x0000_0200;
const IMPLEMENTED_CAPABILITIES: u32 =
    CAP_RECEIVER_UI | CAP_AUTHENTICATED_STATUS | CAP_PLAYBACK_PROGRESS | CAP_TERMINAL_CAUSE;
const REQUIRED_CAPABILITIES: u32 = CAP_AUTHENTICATED_STATUS;
const AUTH_PENDING: u8 = 0;
const AUTH_ONCE: u8 = 1;
const AUTH_EIGHT_HOURS: u8 = 2;
const AUTH_UNTIL_REVOKED: u8 = 3;
const AUTH_DENIED: u8 = 4;
const AUTH_EXPIRED: u8 = 5;
const AUTH_CANCELLED: u8 = 6;
const AUTHORIZATION_WAIT: Duration = Duration::from_secs(30);
const AUTHORIZATION_POLL: Duration = Duration::from_millis(500);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_IO_POLL: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const SESSION_IO_DEADLINE: Duration = Duration::from_secs(3);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const RECONNECT_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct SessionEndpoint {
    interface_index: u32,
    local_address: Ipv4Addr,
    prefix_length: u8,
    peer: SocketAddr,
    connection_intent: Option<([u8; 16], bool)>,
    background_request_id: Option<[u8; 16]>,
    background_authorization_required: bool,
}

impl SessionEndpoint {
    fn loopback(peer: SocketAddr) -> Self {
        Self {
            interface_index: 0,
            local_address: Ipv4Addr::LOCALHOST,
            prefix_length: 32,
            peer,
            connection_intent: None,
            background_request_id: None,
            background_authorization_required: false,
        }
    }

    fn on_interface(interface: &Ipv4Interface, peer: SocketAddr) -> Self {
        Self {
            interface_index: interface.index,
            local_address: interface.address,
            prefix_length: interface.prefix_length,
            peer,
            connection_intent: None,
            background_request_id: None,
            background_authorization_required: false,
        }
    }

    fn validate(self) -> Result<(), String> {
        let local_valid = self.local_address.is_loopback()
            || (self.local_address.is_private() && !self.local_address.is_link_local());
        let SocketAddr::V4(peer) = self.peer else {
            return Err("P5-C requires a literal IPv4 receiver endpoint.".into());
        };
        if !local_valid
            || self.prefix_length == 0
            || self.prefix_length > 32
            || peer.port() == 0
            || peer.ip().is_unspecified()
            || peer.ip().is_multicast()
            || (self.local_address.is_loopback() != peer.ip().is_loopback())
            || (self.local_address.is_private() && !peer.ip().is_private())
        {
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
#[derive(Clone)]
pub(crate) enum SessionAudioQueue {
    Pcm16(Arc<PcmQueue>),
    Float32(Arc<FloatPcmQueue>, AudioFormatV2),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostAudioProfile {
    Standard,
    Pcm24(AudioFormatV2),
}

pub(crate) struct HostSession {
    tcp: Option<TcpStream>,
    udp_socket: Option<UdpSocket>,
    sender: Option<UdpSender>,
    float_sender: Option<FloatUdpSender>,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
    queue: Arc<PcmQueue>,
    float_queue: Option<Arc<FloatPcmQueue>>,
    keys: SessionKeys,
    parameters: AudioParameters,
    format_v2: Option<AudioFormatV2>,
    session_id: u64,
    binding_commitment: [u8; 32],
    remote_udp: Option<SocketAddr>,
    command_sequence: u64,
    ack_sequence: u64,
    last_command: Option<Vec<u8>>,
    last_ack: Option<Vec<u8>>,
    last_heartbeat: Instant,
    state: ReceiverState,
    capabilities: Option<ReceiverCapabilities>,
    terminal_cause: Option<TerminalCause>,
    connection_intent: Option<([u8; 16], bool)>,
    background_request_id: Option<[u8; 16]>,
    background_authorization_required: bool,
    background_authorized: bool,
    cancellation: CancellationToken,
}

impl HostSession {
    pub(crate) fn authenticate_loopback(
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
    ) -> Result<Self, String> {
        Self::authenticate_loopback_cancellable(
            peer,
            material,
            parameters,
            CancellationToken::new(),
        )
    }

    pub(crate) fn authenticate_management_on_interface(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let mut session = Self::authenticate_on_network(
            SessionEndpoint::on_interface(interface, peer),
            material,
            parameters,
            cancellation,
        )?;
        let result = session.negotiate_capabilities().and_then(|_| {
            if session
                .capabilities()
                .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_TRUST_MANAGEMENT != 0)
            {
                Ok(())
            } else {
                Err("Receiver does not support background trust management.".into())
            }
        });
        if let Err(error) = result {
            session.close_local();
            return Err(error);
        }
        Ok(session)
    }

    fn authenticate_loopback_cancellable(
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
        parent_cancel: CancellationToken,
    ) -> Result<Self, String> {
        Self::authenticate_on_network(
            SessionEndpoint::loopback(peer),
            material,
            parameters,
            parent_cancel,
        )
    }

    fn authenticate_on_network(
        endpoint: SessionEndpoint,
        material: BindingMaterial,
        parameters: AudioParameters,
        parent_cancel: CancellationToken,
    ) -> Result<Self, String> {
        parameters
            .validate()
            .map_err(|_| "Unsupported P5-C PCM parameters.".to_string())?;
        endpoint.validate()?;
        let cancellation = parent_cancel.child_token();
        check_cancelled(&cancellation).map_err(|_| "P5-C connection was cancelled.".to_string())?;
        let udp_socket = UdpSocket::bind((endpoint.local_address, 0)).map_err(|_| {
            "Unable to bind the P5-C audio socket to the selected interface.".to_string()
        })?;
        let mut tcp = connect_cancellable(endpoint.local_address, endpoint.peer, &cancellation)
            .map_err(|error| if error.kind() == std::io::ErrorKind::AddrNotAvailable {
                "The selected network interface is no longer available.".to_string()
            } else {
                "Receiver endpoint is unavailable on the selected interface; it may be unreachable or require authorized USB/bootstrap if stopped.".to_string()
            })?;
        tcp.set_read_timeout(Some(SESSION_IO_POLL))
            .map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;
        tcp.set_write_timeout(Some(SESSION_IO_POLL))
            .map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;

        let binding_commitment = material.commitment();
        let (handshake, hello) = HostHandshake::start(material, parameters)
            .map_err(|_| "P5-A handshake could not start.".to_string())?;
        write_all_cancellable(&mut tcp, &hello, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A HELLO.".to_string())?;
        let mut challenge = [0u8; 86];
        read_exact_cancellable(&mut tcp, &mut challenge, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A CHALLENGE.".to_string())?;
        let (await_ready, finish) = handshake
            .accept_challenge(&challenge)
            .map_err(|_| "P5-A receiver identity or proof was rejected.".to_string())?;
        write_all_cancellable(&mut tcp, &finish, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A FINISH.".to_string())?;
        let mut ready = [0u8; 38];
        read_exact_cancellable(&mut tcp, &mut ready, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A READY.".to_string())?;
        let session = await_ready
            .accept_ready(&ready)
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
        let format = PcmFormat {
            sample_rate: authenticated_parameters.sample_rate,
            channels: authenticated_parameters.channels as u16,
        };
        let pcm_payload_bytes_sent = Arc::new(AtomicU64::new(0));
        Ok(Self {
            tcp: Some(tcp),
            udp_socket: Some(udp_socket),
            sender: None,
            float_sender: None,
            pcm_payload_bytes_sent,
            queue: Arc::new(PcmQueue::for_session(
                format,
                authenticated_parameters.mode == 1,
            )),
            float_queue: None,
            keys: session_keys,
            parameters: authenticated_parameters,
            format_v2: None,
            session_id,
            binding_commitment,
            remote_udp: None,
            command_sequence: 0,
            ack_sequence: 0,
            last_command: None,
            last_ack: None,
            last_heartbeat: Instant::now(),
            state: ReceiverState::Authenticated,
            capabilities: None,
            terminal_cause: None,
            connection_intent: endpoint.connection_intent,
            background_request_id: endpoint.background_request_id,
            background_authorization_required: endpoint.background_authorization_required,
            background_authorized: false,
            cancellation,
        })
    }

    fn authenticate_v2_on_network(
        endpoint: SessionEndpoint,
        material: BindingMaterial,
        format: AudioFormatV2,
        float_queue: Arc<FloatPcmQueue>,
        parent_cancel: CancellationToken,
    ) -> Result<Self, String> {
        format
            .encode()
            .map_err(|_| "Unsupported P5-C PCM24 format.".to_string())?;
        if !matches!(
            (format.profile, format.sample_rate),
            (2, 48_000) | (3, 96_000)
        ) || format.channels != 2
            || format.sample_format != 2
            || format.valid_bits != 24
            || format.bytes_per_sample != 3
            || format.frame_bytes != 6
        {
            return Err(
                "Only 48 kHz HiFi or 96 kHz Hi-Res stereo packed PCM24 is supported.".into(),
            );
        }
        endpoint.validate()?;
        let cancellation = parent_cancel.child_token();
        check_cancelled(&cancellation).map_err(|_| "P5-C connection was cancelled.".to_string())?;
        let udp_socket = UdpSocket::bind((endpoint.local_address, 0)).map_err(|_| {
            "Unable to bind the P5-C audio socket to the selected interface.".to_string()
        })?;
        let mut tcp = connect_cancellable(endpoint.local_address, endpoint.peer, &cancellation)
            .map_err(|_| {
                "Receiver endpoint is unavailable on the selected interface.".to_string()
            })?;
        tcp.set_read_timeout(Some(SESSION_IO_POLL))
            .map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;
        tcp.set_write_timeout(Some(SESSION_IO_POLL))
            .map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;

        let binding_commitment = material.commitment();
        let (handshake, hello) = HostHandshakeV2::start(material, format)
            .map_err(|_| "P5-A v2 handshake could not start.".to_string())?;
        write_all_cancellable(&mut tcp, &hello, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A v2 HELLO.".to_string())?;
        let mut challenge = [0u8; 104];
        read_exact_cancellable(&mut tcp, &mut challenge, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A v2 CHALLENGE.".to_string())?;
        let (await_ready, finish) = handshake
            .accept_challenge(&challenge)
            .map_err(|_| "P5-A v2 receiver identity or proof was rejected.".to_string())?;
        write_all_cancellable(&mut tcp, &finish, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A v2 FINISH.".to_string())?;
        let mut ready = [0u8; 40];
        read_exact_cancellable(&mut tcp, &mut ready, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A v2 READY.".to_string())?;
        let session = await_ready
            .accept_ready(&ready)
            .map_err(|_| "P5-A v2 READY proof was rejected.".to_string())?;
        if session.format != format {
            return Err("P5-A v2 returned an unexpected audio format.".into());
        }
        let session_id = session.session_id;
        let keys = session.into_keys();
        let session_keys = SessionKeys {
            audio: keys.host_to_receiver_audio,
            control_tx: keys.host_to_receiver_control,
            control_rx: keys.receiver_to_host_control,
        };
        drop(keys);
        let parameters = AudioParameters {
            sample_rate: format.sample_rate,
            channels: format.channels,
            mode: format.mode,
        };
        let queue = Arc::new(PcmQueue::for_session(
            PcmFormat {
                sample_rate: format.sample_rate,
                channels: format.channels as u16,
            },
            format.mode == 1,
        ));
        Ok(Self {
            tcp: Some(tcp),
            udp_socket: Some(udp_socket),
            sender: None,
            float_sender: None,
            pcm_payload_bytes_sent: Arc::new(AtomicU64::new(0)),
            queue,
            float_queue: Some(float_queue),
            keys: session_keys,
            parameters,
            format_v2: Some(format),
            session_id,
            binding_commitment,
            remote_udp: None,
            command_sequence: 0,
            ack_sequence: 0,
            last_command: None,
            last_ack: None,
            last_heartbeat: Instant::now(),
            state: ReceiverState::Authenticated,
            capabilities: None,
            terminal_cause: None,
            connection_intent: endpoint.connection_intent,
            background_request_id: endpoint.background_request_id,
            background_authorization_required: endpoint.background_authorization_required,
            background_authorized: false,
            cancellation,
        })
    }

    fn authenticate_for_profile(
        endpoint: SessionEndpoint,
        binding: &BindingMaterial,
        parameters: AudioParameters,
        profile: HostAudioProfile,
        initial_float_queue: &mut Option<Arc<FloatPcmQueue>>,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let HostAudioProfile::Pcm24(format) = profile else {
            return Self::authenticate_on_network(
                endpoint,
                binding.duplicate_for_session(),
                parameters,
                cancellation,
            );
        };

        if AudioFormatV2::for_profile(format.profile, format.mode).ok() != Some(format)
            || !matches!(
                (format.profile, format.sample_rate),
                (2, 48_000) | (3, 96_000)
            )
            || parameters.sample_rate != format.sample_rate
            || parameters.channels != format.channels
            || parameters.mode != format.mode
        {
            return Err("Requested PCM24 audio profile does not match the authenticated session parameters.".into());
        }

        let mut probe = Self::authenticate_on_network(
            endpoint,
            binding.duplicate_for_session(),
            AudioParameters {
                sample_rate: 48_000,
                channels: 2,
                mode: format.mode,
            },
            cancellation.clone(),
        )?;
        probe.negotiate_capabilities_probe()?;
        let supports_v2 = probe.capabilities().is_some_and(|capabilities| {
            capabilities.compatible && capabilities.capability_bits & CAP_AUDIO_V2 != 0
        });
        let close_result = probe.close_probe_after_peer_eof();
        if let Err(error) = close_result {
            probe.close_local();
            return Err(error);
        }
        if !supports_v2 {
            return Err("Receiver does not support authenticated PCM24 audio; update the Receiver or select Standard.".into());
        }

        let queue = initial_float_queue
            .take()
            .unwrap_or_else(|| Arc::new(FloatPcmQueue::new(format.mode == 1)));
        Self::authenticate_v2_on_network(
            endpoint,
            binding.duplicate_for_session(),
            format,
            queue,
            cancellation,
        )
    }

    pub(crate) fn connect_loopback(
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
    ) -> Result<Self, String> {
        Self::connect_loopback_cancellable(peer, material, parameters, CancellationToken::new())
    }

    pub(crate) fn connect_on_interface(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
    ) -> Result<Self, String> {
        Self::connect_endpoint(
            SessionEndpoint::on_interface(interface, peer),
            material,
            parameters,
            CancellationToken::new(),
        )
    }

    fn connect_loopback_cancellable(
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        Self::connect_endpoint(
            SessionEndpoint::loopback(peer),
            material,
            parameters,
            cancellation,
        )
    }

    fn connect_endpoint(
        endpoint: SessionEndpoint,
        material: BindingMaterial,
        parameters: AudioParameters,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let mut endpoint = endpoint;
        if endpoint.background_request_id.is_none() {
            endpoint.background_request_id = Some(random_request_id()?);
        }
        let mut session =
            Self::authenticate_on_network(endpoint, material, parameters, cancellation)?;
        if let Err(error) = session.negotiate_capabilities()
            .and_then(|_| if session.capabilities().is_some_and(|info| info.compatible) {
                session.authorize_background_start().and_then(|_| session.bind_audio()).and_then(|_| session.select_mode())
            } else { Err("Receiver audio or management capabilities are incompatible; update it through authorized ADB or manual installation.".into()) }) {
            session.close_local();
            return Err(error);
        }
        Ok(session)
    }

    pub(crate) fn capabilities(&self) -> Option<&ReceiverCapabilities> {
        self.capabilities.as_ref()
    }

    pub(crate) fn query_background_trust(&mut self) -> Result<BackgroundTrustSnapshot, String> {
        let request_cancellation = CancellationToken::new();
        self.query_background_trust_until(
            Instant::now() + SESSION_IO_DEADLINE,
            &request_cancellation,
        )
    }

    pub(crate) fn query_background_trust_until(
        &mut self,
        deadline: Instant,
        request_cancellation: &CancellationToken,
    ) -> Result<BackgroundTrustSnapshot, String> {
        if !background_trust_management_state(self.state) {
            return Err("P5-C background trust query is out of order.".into());
        }
        let state = self.state;
        let commitment = self.binding_commitment;
        let ack = self.command_until_request(
            BACKGROUND_TRUST_QUERY,
            &commitment,
            deadline,
            request_cancellation,
        )?;
        if ack.result != RESULT_OK || ack.state != state || ack.udp_port != 0 {
            return Err("Receiver rejected the background trust query.".into());
        }
        ack.background_trust
            .ok_or_else(|| "Receiver background trust response was missing.".into())
    }

    pub(crate) fn revoke_background_trust(&mut self) -> Result<BackgroundTrustSnapshot, String> {
        let request_cancellation = CancellationToken::new();
        self.revoke_background_trust_until(
            Instant::now() + SESSION_IO_DEADLINE,
            &request_cancellation,
        )
    }

    pub(crate) fn revoke_background_trust_until(
        &mut self,
        deadline: Instant,
        request_cancellation: &CancellationToken,
    ) -> Result<BackgroundTrustSnapshot, String> {
        if !background_trust_management_state(self.state) {
            return Err("P5-C background trust revocation is out of order.".into());
        }
        let commitment = self.binding_commitment;
        let ack = self.command_until_request(
            BACKGROUND_TRUST_REVOKE,
            &commitment,
            deadline,
            request_cancellation,
        )?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::Stopped || ack.udp_port != 0 {
            return Err("Receiver did not confirm effective background trust revocation.".into());
        }
        let snapshot = ack
            .background_trust
            .ok_or_else(|| "Receiver background trust response was missing.".to_string())?;
        if snapshot.kind != 0 || snapshot.persistence != 0 || snapshot.remaining_ms != 0 {
            return Err("Receiver ACK did not confirm that background trust was cleared.".into());
        }
        self.close_local();
        Ok(snapshot)
    }

    fn negotiate_capabilities(&mut self) -> Result<(), String> {
        self.negotiate_capabilities_with(REQUIRED_CAPABILITIES, true)
    }

    fn negotiate_capabilities_probe(&mut self) -> Result<(), String> {
        self.negotiate_capabilities_with(REQUIRED_CAPABILITIES | CAP_AUDIO_V2, false)
    }

    fn negotiate_capabilities_with(
        &mut self,
        required_capabilities: u32,
        claim_intent: bool,
    ) -> Result<(), String> {
        if self.state != ReceiverState::Authenticated || self.capabilities.is_some() {
            return Err("P5-C capability query is out of order.".into());
        }
        let result = (|| {
            let command_sequence = self.command_sequence;
            let payload = encode_capabilities_query(
                MANAGEMENT_CONTROL_MAJOR,
                AUDIO_PROTOCOL_MAJOR,
                required_capabilities,
            );
            let frame = encode_control(
                &self.keys.control_tx,
                HOST_TO_RECEIVER_CONTROL,
                self.session_id,
                command_sequence,
                CAPABILITIES_QUERY,
                &payload,
            )?;
            let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
            write_frame(tcp, &frame, &self.cancellation)?;
            let response = read_frame(
                tcp,
                &self.keys.control_rx,
                RECEIVER_TO_HOST_CONTROL,
                self.session_id,
                &self.cancellation,
            )?;
            let info = decode_capabilities_info(&response, self.ack_sequence, command_sequence)?;
            self.command_sequence = self
                .command_sequence
                .checked_add(1)
                .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
            self.ack_sequence = self
                .ack_sequence
                .checked_add(1)
                .ok_or_else(|| "P5-C response sequence exhausted.".to_string())?;
            self.capabilities = Some(info.clone());
            if claim_intent {
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
            }
            if self.background_authorization_required
                && info.capability_bits & CAP_BACKGROUND_START_AUTH == 0
            {
                return Err(
                    "Receiver does not support authenticated background-start authorization."
                        .into(),
                );
            }
            Ok(())
        })();
        if result.is_err() {
            self.close_local();
        }
        result
    }

    fn close_probe_after_peer_eof(&mut self) -> Result<(), String> {
        let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
        tcp.shutdown(std::net::Shutdown::Write)
            .map_err(|_| "Unable to close the capability probe session.".to_string())?;
        let deadline = Instant::now() + SESSION_IO_DEADLINE;
        let mut byte = [0u8; 1];
        loop {
            check_cancelled(&self.cancellation)
                .map_err(|_| "P5-C capability probe was cancelled.".to_string())?;
            if Instant::now() >= deadline {
                return Err("Receiver did not release the capability probe session.".into());
            }
            match tcp.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    return Err("Unexpected data followed the capability probe response.".into());
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    return Err("Receiver closed the capability probe unexpectedly.".into());
                }
            }
        }
        self.close_local();
        Ok(())
    }

    fn request_background_authorization(&mut self, request_id: &[u8; 16]) -> Result<u8, String> {
        let ack = self.command(BACKGROUND_START_REQUEST, request_id)?;
        self.background_authorization_ack(&ack, request_id)
    }

    fn query_background_authorization(&mut self, request_id: &[u8; 16]) -> Result<u8, String> {
        let ack = self.command(BACKGROUND_START_STATUS, request_id)?;
        self.background_authorization_ack(&ack, request_id)
    }

    fn request_background_authorization_until(
        &mut self,
        request_id: &[u8; 16],
        deadline: Instant,
    ) -> Result<u8, String> {
        let ack = self.command_until(BACKGROUND_START_REQUEST, request_id, deadline)?;
        self.background_authorization_ack(&ack, request_id)
    }

    fn query_background_authorization_until(
        &mut self,
        request_id: &[u8; 16],
        deadline: Instant,
    ) -> Result<u8, String> {
        let ack = self.command_until(BACKGROUND_START_STATUS, request_id, deadline)?;
        self.background_authorization_ack(&ack, request_id)
    }

    fn cancel_background_authorization(&mut self, request_id: &[u8; 16]) -> Result<(), String> {
        let ack = self.command(BACKGROUND_START_CANCEL, request_id)?;
        let status = self.background_authorization_ack(&ack, request_id)?;
        if status != AUTH_CANCELLED {
            return Err("Receiver did not cancel the pending background-start request.".into());
        }
        Ok(())
    }

    fn background_authorization_ack(&self, ack: &Ack, request_id: &[u8; 16]) -> Result<u8, String> {
        if ack.result != RESULT_OK || ack.state != ReceiverState::Authenticated || ack.udp_port != 0
        {
            return Err("Receiver rejected background-start authorization.".into());
        }
        let (echoed_id, status) = ack
            .background_authorization
            .as_ref()
            .ok_or("Receiver background-start ACK was missing its status.")?;
        if echoed_id != request_id {
            return Err("Receiver background-start ACK request ID did not match.".into());
        }
        Ok(*status)
    }

    fn authorize_background_start(&mut self) -> Result<(), String> {
        let supported = self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_START_AUTH != 0);
        if !supported {
            return if self.background_authorization_required {
                Err(
                    "Receiver does not support authenticated background-start authorization."
                        .into(),
                )
            } else {
                Ok(())
            };
        }
        let request_id = self
            .background_request_id
            .ok_or_else(|| "P5-C background-start request ID is unavailable.".to_string())?;
        let deadline = Instant::now() + AUTHORIZATION_WAIT;
        let mut status = self.request_background_authorization_until(&request_id, deadline)?;
        let mut next_poll = Instant::now() + AUTHORIZATION_POLL;
        let mut next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
        loop {
            if Instant::now() >= deadline {
                return Err(
                    "Receiver user did not approve background playback within 30 seconds.".into(),
                );
            }
            match status {
                AUTH_ONCE | AUTH_EIGHT_HOURS | AUTH_UNTIL_REVOKED => {
                    if Instant::now() >= deadline {
                        return Err(
                            "Receiver user did not approve background playback within 30 seconds."
                                .into(),
                        );
                    }
                    self.background_authorized = true;
                    return Ok(());
                }
                AUTH_DENIED => return Err("Receiver user denied background playback.".into()),
                AUTH_EXPIRED => return Err("Receiver background-start request expired.".into()),
                AUTH_CANCELLED => {
                    return Err("Receiver background-start request was cancelled.".into())
                }
                AUTH_PENDING => {}
                _ => return Err("Receiver returned an invalid background-start status.".into()),
            }
            while Instant::now() < next_poll.min(next_heartbeat).min(deadline) {
                check_cancelled(&self.cancellation)
                    .map_err(|_| "P5-C connection was cancelled.".to_string())?;
                thread::sleep(
                    SESSION_IO_POLL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            if Instant::now() >= deadline {
                return Err(
                    "Receiver user did not approve background playback within 30 seconds.".into(),
                );
            }
            if Instant::now() >= next_heartbeat {
                let heartbeat = self.command_until(HEARTBEAT, &[], deadline)?;
                if Instant::now() >= deadline
                    || heartbeat.result != RESULT_OK
                    || heartbeat.state != ReceiverState::Authenticated
                {
                    return Err(
                        "P5-C authenticated heartbeat failed during background-start approval."
                            .into(),
                    );
                }
                self.last_heartbeat = Instant::now();
                self.state = heartbeat.state;
                next_heartbeat += HEARTBEAT_INTERVAL;
            }
            if Instant::now() >= next_poll {
                status = self.query_background_authorization_until(&request_id, deadline)?;
                if Instant::now() >= deadline {
                    return Err(
                        "Receiver user did not approve background playback within 30 seconds."
                            .into(),
                    );
                }
                next_poll += AUTHORIZATION_POLL;
            }
        }
    }

    pub(crate) fn state(&self) -> ReceiverState {
        self.state
    }
    pub(crate) fn session_id(&self) -> u64 {
        self.session_id
    }
    pub(crate) fn pcm_queue(&self) -> Arc<PcmQueue> {
        self.queue.clone()
    }

    pub(crate) fn audio_queue(&self) -> SessionAudioQueue {
        self.float_queue
            .as_ref()
            .zip(self.format_v2)
            .map(|(queue, format)| SessionAudioQueue::Float32(queue.clone(), format))
            .unwrap_or_else(|| SessionAudioQueue::Pcm16(self.queue.clone()))
    }

    pub(crate) fn is_v2_audio(&self) -> bool {
        self.format_v2.is_some()
    }
    pub(crate) fn pcm_payload_bytes_sent(&self) -> u64 {
        self.pcm_payload_bytes_sent.load(Ordering::Relaxed)
    }

    fn check_sender(&mut self) -> Result<(), String> {
        if let Some(sender) = self.sender.as_mut() {
            if let Some(error) = sender.take_finished_error() {
                return Err(error);
            }
        }
        if let Some(sender) = self.float_sender.as_mut() {
            if let Some(error) = sender.take_finished_error() {
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn bind_audio(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::Authenticated {
            return Err("P5-C BIND is out of order.".into());
        }
        if self
            .capabilities
            .as_ref()
            .is_some_and(|info| !info.compatible)
        {
            return Err(if self.format_v2.is_some() {
                "Receiver rejected the authenticated PCM24 audio format; only supported V2 descriptors can start audio.".into()
            } else {
                "Receiver audio capabilities are incompatible; update it or select Standard.".into()
            });
        }
        if self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_START_AUTH != 0)
            && !self.background_authorized
        {
            return Err("P5-C BIND requires an approved background-start request.".into());
        }
        let port = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .local_addr()
            .map_err(|_| "Unable to read P5-C UDP source port.".to_string())?
            .port();
        let ack = self.command(BIND, &port.to_be_bytes())?;
        if ack.result != RESULT_OK
            || ack.state != ReceiverState::TransportConnected
            || ack.udp_port == 0
        {
            return Err(format!(
                "P5-C receiver did not confirm the authenticated UDP peer (result={}, state={:?}, udp_port={}).",
                ack.result, ack.state, ack.udp_port
            ));
        }
        let tcp_peer = self
            .tcp
            .as_ref()
            .ok_or("P5-C control channel is closed.")?
            .peer_addr()
            .map_err(|_| "Unable to read P5-C peer address.".to_string())?;
        self.remote_udp = Some(SocketAddr::new(tcp_peer.ip(), ack.udp_port));
        self.state = ack.state;
        Ok(())
    }

    pub(crate) fn select_mode(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::TransportConnected {
            return Err("P5-C mode selection is out of order.".into());
        }
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
        if self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_START_AUTH != 0)
            && !self.background_authorized
        {
            return Err("P5-C START requires an approved background-start request.".into());
        }
        let ack = self.command(START, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::ReceiverRunning {
            self.close_local();
            return Err("P5-C receiver did not confirm playback start.".into());
        }
        let result = (|| {
            let socket = self
                .udp_socket
                .as_ref()
                .ok_or("P5-C UDP socket is closed.")?
                .try_clone()
                .map_err(|_| "Unable to transfer the authenticated UDP socket.".to_string())?;
            let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
            let material =
                UdpSessionMaterial::from_authenticated(self.session_id, &self.keys.audio);
            if let (Some(queue), Some(format)) = (&self.float_queue, self.format_v2) {
                self.float_sender = Some(FloatUdpSender::spawn_authenticated(
                    remote,
                    queue.clone(),
                    socket,
                    material,
                    format,
                    self.pcm_payload_bytes_sent.clone(),
                )?);
            } else {
                self.sender = Some(UdpSender::spawn_authenticated(
                    remote,
                    self.queue.clone(),
                    socket,
                    material,
                    self.parameters,
                    self.pcm_payload_bytes_sent.clone(),
                )?);
            }
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
            let socket = self
                .udp_socket
                .as_ref()
                .ok_or("P5-C UDP socket is closed.")?
                .try_clone()
                .map_err(|_| "Unable to transfer the authenticated UDP socket.".to_string())?;
            let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
            let material =
                UdpSessionMaterial::from_authenticated(self.session_id, &self.keys.audio);
            self.sender = Some(UdpSender::spawn_authenticated_without_format_for_test(
                remote,
                self.queue.clone(),
                socket,
                material,
                self.parameters,
                self.pcm_payload_bytes_sent.clone(),
            )?);
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
        if ack.result != RESULT_OK || ack.state != ReceiverState::Paused {
            return Err("P5-C PAUSE was rejected.".into());
        }
        self.state = ack.state;
        if let Some(sender) = &self.float_sender {
            sender.expect_queue_stop();
        }
        Ok(())
    }

    pub(crate) fn resume(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::Paused {
            return Err("P5-C RESUME is out of order.".into());
        }
        let ack = self.command(RESUME, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::ReceiverRunning {
            return Err("P5-C RESUME was rejected.".into());
        }
        if let Some(sender) = &self.float_sender {
            sender.resume_after_ack()?;
        }
        self.state = ack.state;
        Ok(())
    }

    pub(crate) fn heartbeat(&mut self) -> Result<ReceiverState, String> {
        let ack = self.command(HEARTBEAT, &[])?;
        if ack.result == RESULT_USER_STOPPED {
            if ack.state != ReceiverState::Stopped
                || !self
                    .capabilities
                    .as_ref()
                    .is_some_and(|caps| caps.capability_bits & CAP_TERMINAL_CAUSE != 0)
            {
                return Err("Receiver returned an invalid authenticated terminal cause.".into());
            }
            self.terminal_cause = Some(TerminalCause::ReceiverUserStop);
        } else if ack.result != RESULT_OK {
            return Err("P5-C heartbeat was rejected.".into());
        }
        self.last_heartbeat = Instant::now();
        self.state = ack.state;
        Ok(self.state)
    }

    pub(crate) fn terminal_cause(&self) -> Option<TerminalCause> {
        self.terminal_cause
    }

    fn playback_progress(&mut self) -> Result<Option<PlaybackDiagnostics>, String> {
        if !self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_PLAYBACK_PROGRESS != 0)
        {
            return Ok(None);
        }
        let ack = self.command(PLAYBACK_PROGRESS_QUERY, &[])?;
        if ack.result != RESULT_OK {
            return Err("P5-C playback progress query was rejected.".into());
        }
        ack.playback_progress
            .ok_or_else(|| "Receiver playback progress response was missing.".to_string())
            .map(Some)
    }

    pub(crate) fn check_liveness_at(&mut self, now: Instant) -> Result<(), String> {
        if now.saturating_duration_since(self.last_heartbeat) > HEARTBEAT_TIMEOUT {
            self.close_local();
            return Err("P5-C authenticated heartbeat expired.".into());
        }
        Ok(())
    }

    pub(crate) fn stop(&mut self) -> Result<(), String> {
        let result = if self.state == ReceiverState::Stopped {
            Ok(())
        } else {
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
        if result.is_err() {
            self.close_local();
        }
        result
    }

    fn command_until(
        &mut self,
        kind: u8,
        payload: &[u8],
        deadline: Instant,
    ) -> Result<Ack, String> {
        let result = self.command_inner_until(kind, payload, deadline);
        if result.is_err() {
            self.close_local();
        }
        result
    }

    fn command_until_request(
        &mut self,
        kind: u8,
        payload: &[u8],
        deadline: Instant,
        request_cancellation: &CancellationToken,
    ) -> Result<Ack, String> {
        let mut bytes_written = false;
        let result = self.command_inner_until_request(
            kind,
            payload,
            deadline,
            request_cancellation,
            &mut bytes_written,
        );
        if result.is_err() && bytes_written {
            self.close_local();
        }
        result
    }

    fn command_inner(&mut self, kind: u8, payload: &[u8]) -> Result<Ack, String> {
        if matches!(kind, BACKGROUND_TRUST_QUERY | BACKGROUND_TRUST_REVOKE)
            && !self
                .capabilities
                .as_ref()
                .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_TRUST_MANAGEMENT != 0)
        {
            return Err("Receiver does not support background trust management.".into());
        }
        if matches!(kind, BACKGROUND_TRUST_QUERY | BACKGROUND_TRUST_REVOKE)
            && (payload.len() != 32 || payload != self.binding_commitment.as_slice())
        {
            return Err(
                "P5-C background trust request must target the authenticated binding.".into(),
            );
        }
        let sequence = self.command_sequence;
        let frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            sequence,
            kind,
            payload,
        )?;
        let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
        write_frame(tcp, &frame, &self.cancellation)?;
        let ack_frame = read_frame(
            tcp,
            &self.keys.control_rx,
            RECEIVER_TO_HOST_CONTROL,
            self.session_id,
            &self.cancellation,
        )?;
        let ack = decode_ack_with_format(
            &ack_frame,
            self.ack_sequence,
            kind,
            sequence,
            self.format_v2.is_some(),
        )?;
        self.record_terminal_cause(&ack)?;
        if ack
            .background_trust
            .as_ref()
            .is_some_and(|trust| trust.commitment != self.binding_commitment)
        {
            return Err(
                "Receiver background trust ACK binding did not match the authenticated session."
                    .into(),
            );
        }
        self.command_sequence = self
            .command_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
        self.ack_sequence = self
            .ack_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C ACK sequence exhausted.".to_string())?;
        self.last_command = Some(frame);
        self.last_ack = Some(ack_frame);
        Ok(ack)
    }

    fn command_inner_until(
        &mut self,
        kind: u8,
        payload: &[u8],
        deadline: Instant,
    ) -> Result<Ack, String> {
        if Instant::now() >= deadline {
            return Err("P5-C approval deadline expired.".into());
        }
        let io_deadline = deadline.min(Instant::now() + SESSION_IO_DEADLINE);
        let sequence = self.command_sequence;
        let frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            sequence,
            kind,
            payload,
        )?;
        let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
        write_frame_until(tcp, &frame, &self.cancellation, io_deadline)?;
        let ack_frame = read_frame_until(
            tcp,
            &self.keys.control_rx,
            RECEIVER_TO_HOST_CONTROL,
            self.session_id,
            &self.cancellation,
            io_deadline,
        )?;
        if Instant::now() >= deadline {
            return Err("P5-C approval deadline expired.".into());
        }
        let ack = decode_ack_with_format(
            &ack_frame,
            self.ack_sequence,
            kind,
            sequence,
            self.format_v2.is_some(),
        )?;
        self.record_terminal_cause(&ack)?;
        if ack
            .background_trust
            .as_ref()
            .is_some_and(|trust| trust.commitment != self.binding_commitment)
        {
            return Err(
                "Receiver background trust ACK binding did not match the authenticated session."
                    .into(),
            );
        }
        self.command_sequence = self
            .command_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
        self.ack_sequence = self
            .ack_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C ACK sequence exhausted.".to_string())?;
        self.last_command = Some(frame);
        self.last_ack = Some(ack_frame);
        Ok(ack)
    }

    fn command_inner_until_request(
        &mut self,
        kind: u8,
        payload: &[u8],
        deadline: Instant,
        request_cancellation: &CancellationToken,
        bytes_written: &mut bool,
    ) -> Result<Ack, String> {
        if Instant::now() >= deadline || request_cancellation.is_cancelled() {
            return Err(
                "P5-C background trust request expired or was cancelled before sending.".into(),
            );
        }
        if !self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_TRUST_MANAGEMENT != 0)
        {
            return Err("Receiver does not support background trust management.".into());
        }
        if payload.len() != 32 || payload != self.binding_commitment.as_slice() {
            return Err(
                "P5-C background trust request must target the authenticated binding.".into(),
            );
        }
        let sequence = self.command_sequence;
        let frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            sequence,
            kind,
            payload,
        )?;
        let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
        write_frame_until_request(
            tcp,
            &frame,
            &self.cancellation,
            request_cancellation,
            deadline,
            bytes_written,
        )?;
        let ack_frame = read_frame_until_request(
            tcp,
            &self.keys.control_rx,
            RECEIVER_TO_HOST_CONTROL,
            self.session_id,
            &self.cancellation,
            request_cancellation,
            deadline,
        )?;
        let ack = decode_ack_with_format(
            &ack_frame,
            self.ack_sequence,
            kind,
            sequence,
            self.format_v2.is_some(),
        )?;
        self.record_terminal_cause(&ack)?;
        if ack
            .background_trust
            .as_ref()
            .is_some_and(|trust| trust.commitment != self.binding_commitment)
        {
            return Err(
                "Receiver background trust ACK binding did not match the authenticated session."
                    .into(),
            );
        }
        self.command_sequence = self
            .command_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
        self.ack_sequence = self
            .ack_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C ACK sequence exhausted.".to_string())?;
        self.last_command = Some(frame);
        self.last_ack = Some(ack_frame);
        Ok(ack)
    }

    fn record_terminal_cause(&mut self, ack: &Ack) -> Result<(), String> {
        if ack.result != RESULT_USER_STOPPED {
            return Ok(());
        }
        if ack.state != ReceiverState::Stopped
            || !self
                .capabilities
                .as_ref()
                .is_some_and(|caps| caps.capability_bits & CAP_TERMINAL_CAUSE != 0)
        {
            return Err("P5-C Receiver terminal cause was invalid.".into());
        }
        self.terminal_cause = Some(TerminalCause::ReceiverUserStop);
        self.state = ReceiverState::Stopped;
        Ok(())
    }

    fn close_local(&mut self) {
        self.cancellation.cancel();
        self.queue.stop_and_clear();
        if let Some(queue) = &self.float_queue {
            queue.stop_and_clear();
        }
        if let Some(sender) = self.sender.take() {
            let _ = sender.stop();
        }
        if let Some(sender) = self.float_sender.take() {
            let _ = sender.stop();
        }
        drop(self.udp_socket.take());
        if let Some(tcp) = self.tcp.take() {
            let _ = tcp.shutdown(std::net::Shutdown::Both);
        }
        self.keys.audio.fill(0);
        self.keys.control_tx.fill(0);
        self.keys.control_rx.fill(0);
        self.state = ReceiverState::Stopped;
    }

    #[cfg(test)]
    fn resend_last_for_test(&mut self) -> Result<(), String> {
        let frame = self
            .last_command
            .as_ref()
            .ok_or("No P5-C command to repeat.")?
            .clone();
        let expected_ack = self
            .last_ack
            .as_ref()
            .ok_or("No P5-C ACK to compare.")?
            .clone();
        write_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &frame,
            &self.cancellation,
        )?;
        let actual_ack = read_raw_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &self.cancellation,
        )?;
        if actual_ack != expected_ack {
            return Err("P5-C duplicate command changed its ACK.".into());
        }
        Ok(())
    }

    #[cfg(test)]
    fn send_bad_mac_for_test(&mut self) -> Result<(), String> {
        let port = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .local_addr()
            .map_err(|_| "UDP socket unavailable".to_string())?
            .port();
        let mut frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            self.command_sequence,
            BIND,
            &port.to_be_bytes(),
        )?;
        *frame.last_mut().unwrap() ^= 0x80;
        write_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &frame,
            &self.cancellation,
        )?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    fn send_bad_capabilities_query_for_test(&mut self, bad_mac: bool) -> Result<(), String> {
        let mut payload = encode_capabilities_query(
            MANAGEMENT_CONTROL_MAJOR,
            AUDIO_PROTOCOL_MAJOR,
            REQUIRED_CAPABILITIES,
        );
        payload[3] = 1;
        let mut frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            self.command_sequence,
            CAPABILITIES_QUERY,
            &payload,
        )?;
        if bad_mac {
            *frame.last_mut().unwrap() ^= 1;
        }
        write_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &frame,
            &self.cancellation,
        )?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    fn send_old_session_for_test(&mut self) -> Result<(), String> {
        let port = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .local_addr()
            .map_err(|_| "UDP socket unavailable".to_string())?
            .port();
        let frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id.wrapping_sub(1),
            self.command_sequence,
            BIND,
            &port.to_be_bytes(),
        )?;
        write_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &frame,
            &self.cancellation,
        )?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    fn send_old_audio_for_test(&self) -> Result<(), String> {
        let format = self.parameters;
        let mut encoder = PacketEncoder::with_sequence(
            &self.keys.audio,
            self.session_id.wrapping_sub(1),
            HOST_TO_RECEIVER,
            0,
        );
        let frame = encoder
            .encode_next(
                PacketType::Format,
                0,
                &PcmFormat {
                    sample_rate: format.sample_rate,
                    channels: format.channels as u16,
                }
                .encode()
                .map_err(|_| "invalid session PCM format")?,
            )
            .map_err(|_| "unable to encode synthetic old-session packet")?;
        let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
        let sent = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .send_to(&frame, remote)
            .map_err(|_| "unable to send old-session packet")?;
        if sent != frame.len() {
            return Err("old-session datagram send was incomplete".into());
        }
        Ok(())
    }

    #[cfg(test)]
    fn send_format_packet_for_test(&self, sample_rate: u32) -> Result<(), String> {
        let payload = PcmFormat {
            sample_rate,
            channels: self.parameters.channels as u16,
        }
        .encode()
        .map_err(|_| "invalid synthetic test format")?;
        let mut encoder =
            PacketEncoder::with_sequence(&self.keys.audio, self.session_id, HOST_TO_RECEIVER, 0);
        let packet = encoder
            .encode_next(PacketType::Format, 0, &payload)
            .map_err(|_| "unable to encode synthetic FORMAT")?;
        let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
        let sent = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .send_to(&packet, remote)
            .map_err(|_| "unable to send synthetic FORMAT")?;
        if sent != packet.len() {
            return Err("synthetic FORMAT datagram send was incomplete".into());
        }
        Ok(())
    }

    #[cfg(test)]
    fn remove_udp_socket_for_test(&mut self) {
        drop(self.udp_socket.take());
    }

    #[cfg(test)]
    fn remove_control_for_test(&mut self) {
        if let Some(tcp) = self.tcp.take() {
            let _ = tcp.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Drop for HostSession {
    fn drop(&mut self) {
        self.close_local();
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalCause {
    ReceiverUserStop,
    HostStop,
    TransportFailure,
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
    pub(crate) terminal_cause: Option<TerminalCause>,
    pub(crate) playback_diagnostics: Option<PlaybackDiagnostics>,
    pub(crate) negotiated_format: Option<AudioFormatV2>,
    pub(crate) recovery_seconds_remaining: Option<u64>,
    recovery_started_at: Option<Instant>,
    recovery_window: Duration,
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
    BackgroundTrustQuery {
        session_generation: u64,
        deadline: Instant,
        cancellation: CancellationToken,
        reply: oneshot::Sender<Result<BackgroundTrustSnapshot, String>>,
    },
    BackgroundTrustRevoke {
        session_generation: u64,
        deadline: Instant,
        cancellation: CancellationToken,
        reply: oneshot::Sender<Result<BackgroundTrustSnapshot, String>>,
    },
    #[cfg(test)]
    TestSenderBarrier(std::sync::mpsc::SyncSender<bool>),
    Stop,
}

/// Single thread owns every HostSession. It is created only by an explicit caller.
pub(crate) struct SessionOwner {
    connection_intent: Option<([u8; 16], bool)>,
    background_authorization_required: bool,
    cancellation: CancellationToken,
    enabled: Arc<AtomicBool>,
    commands: mpsc::Sender<OwnerCommand>,
    status: Arc<Mutex<SessionSnapshot>>,
    queue: Arc<Mutex<Option<Arc<PcmQueue>>>>,
    float_queue: Arc<Mutex<Option<Arc<FloatPcmQueue>>>>,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
}

impl SessionOwner {
    pub(crate) fn start(
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
    ) -> Result<Self, String> {
        Self::start_with_timing(
            peer,
            binding,
            parameters,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
        )
    }

    pub(crate) fn start_on_interface(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
    ) -> Result<Self, String> {
        Self::start_with_endpoint(
            SessionEndpoint::on_interface(interface, peer),
            binding,
            parameters,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_on_interface_with_profile(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        profile: HostAudioProfile,
        initial_float_queue: Option<Arc<FloatPcmQueue>>,
        caller_cancel: CancellationToken,
        connection_intent: Option<([u8; 16], bool)>,
        background_authorization_required: bool,
    ) -> Result<Self, String> {
        let mut endpoint = SessionEndpoint::on_interface(interface, peer);
        endpoint.connection_intent = connection_intent;
        endpoint.background_authorization_required = background_authorization_required;
        Self::start_with_endpoint_and_profile(
            endpoint,
            binding,
            parameters,
            profile,
            initial_float_queue,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
        )
    }

    pub(crate) fn start_on_interface_with_intent(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
        nonce: [u8; 16],
        required: bool,
    ) -> Result<Self, String> {
        let mut endpoint = SessionEndpoint::on_interface(interface, peer);
        endpoint.connection_intent = Some((nonce, required));
        Self::start_with_endpoint(
            endpoint,
            binding,
            parameters,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
        )
    }

    pub(crate) fn start_on_interface_with_background_authorization(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
    ) -> Result<Self, String> {
        let mut endpoint = SessionEndpoint::on_interface(interface, peer);
        endpoint.background_authorization_required = true;
        Self::start_with_endpoint(
            endpoint,
            binding,
            parameters,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
        )
    }

    fn start_with_timing(
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
        timing: SessionTiming,
    ) -> Result<Self, String> {
        Self::start_with_endpoint(
            SessionEndpoint::loopback(peer),
            binding,
            parameters,
            caller_cancel,
            timing,
        )
    }

    fn start_with_endpoint(
        endpoint: SessionEndpoint,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
        timing: SessionTiming,
    ) -> Result<Self, String> {
        Self::start_with_endpoint_and_profile(
            endpoint,
            binding,
            parameters,
            HostAudioProfile::Standard,
            None,
            caller_cancel,
            timing,
        )
    }

    fn start_with_endpoint_and_profile(
        mut endpoint: SessionEndpoint,
        binding: BindingMaterial,
        parameters: AudioParameters,
        profile: HostAudioProfile,
        initial_float_queue: Option<Arc<FloatPcmQueue>>,
        caller_cancel: CancellationToken,
        timing: SessionTiming,
    ) -> Result<Self, String> {
        endpoint.validate()?;
        if endpoint.background_request_id.is_none() {
            endpoint.background_request_id = Some(random_request_id()?);
        }
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
            terminal_cause: None,
            playback_diagnostics: None,
            negotiated_format: None,
            recovery_seconds_remaining: None,
            recovery_started_at: None,
            recovery_window: timing.recovery_window,
        }));
        let queue = Arc::new(Mutex::new(None));
        let float_queue = Arc::new(Mutex::new(None));
        let pcm_payload_bytes_sent = Arc::new(AtomicU64::new(0));
        let (commands, receiver) = mpsc::channel();
        let owner_cancel = cancellation.clone();
        let owner_enabled = enabled.clone();
        let owner_status = status.clone();
        let owner_queue = queue.clone();
        let owner_float_queue = float_queue.clone();
        let owner_bytes = pcm_payload_bytes_sent.clone();
        let worker = thread::Builder::new()
            .name("AnPod P5-D session owner".into())
            .spawn(move || {
                run_session_owner(
                    endpoint,
                    binding,
                    parameters,
                    owner_cancel,
                    owner_enabled,
                    receiver,
                    owner_status,
                    owner_queue,
                    owner_float_queue,
                    owner_bytes,
                    timing,
                    profile,
                    initial_float_queue,
                )
            })
            .map_err(|error| format!("Unable to start P5-D session owner: {error}"))?;
        Ok(Self {
            connection_intent: endpoint.connection_intent,
            background_authorization_required: endpoint.background_authorization_required,
            cancellation,
            enabled,
            commands,
            status,
            queue,
            float_queue,
            pcm_payload_bytes_sent,
            worker: Some(worker),
        })
    }

    pub(crate) fn snapshot(&self) -> SessionSnapshot {
        let mut snapshot = self
            .status
            .lock()
            .expect("P5-D session status lock poisoned")
            .clone();
        snapshot.recovery_seconds_remaining =
            if snapshot.state == SessionLifecycleState::Reconnecting {
                snapshot.recovery_started_at.map(|started| {
                    snapshot
                        .recovery_window
                        .saturating_sub(started.elapsed())
                        .as_secs_f64()
                        .ceil() as u64
                })
            } else {
                None
            };
        snapshot
    }

    pub(crate) async fn query_background_trust(
        &self,
        session_generation: u64,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<BackgroundTrustSnapshot, String> {
        if cancellation.is_cancelled() || Instant::now() >= deadline {
            return Err("背景授权查询已取消或超时。".into());
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .send(OwnerCommand::BackgroundTrustQuery {
                session_generation,
                deadline,
                cancellation: cancellation.clone(),
                reply,
            })
            .map_err(|_| "P5-D session owner has stopped.".to_string())?;
        tokio::select! {
            biased;
            result = response => result.map_err(|_| "P5-D session owner stopped before the background trust query completed.".to_string())?,
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err("背景授权查询已超时。".into()),
        }
    }

    pub(crate) async fn revoke_background_trust(
        &self,
        session_generation: u64,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<BackgroundTrustSnapshot, String> {
        if cancellation.is_cancelled() || Instant::now() >= deadline {
            return Err("背景授权撤销已取消或超时。".into());
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .send(OwnerCommand::BackgroundTrustRevoke {
                session_generation,
                deadline,
                cancellation: cancellation.clone(),
                reply,
            })
            .map_err(|_| "P5-D session owner has stopped.".to_string())?;
        tokio::select! {
            biased;
            result = response => result.map_err(|_| "P5-D session owner stopped before the background trust revocation completed.".to_string())?,
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err("背景授权撤销已超时。".into()),
        }
    }

    pub(crate) fn pcm_queue(&self) -> Option<Arc<PcmQueue>> {
        self.queue
            .lock()
            .expect("P5-D PCM queue lock poisoned")
            .clone()
    }

    pub(crate) fn audio_queue(&self) -> Option<SessionAudioQueue> {
        if let Some(queue) = self
            .float_queue
            .lock()
            .expect("P5-D Float32 queue lock poisoned")
            .clone()
        {
            let format = self
                .status
                .lock()
                .expect("P5-D session status lock poisoned")
                .negotiated_format?;
            Some(SessionAudioQueue::Float32(queue, format))
        } else {
            self.pcm_queue().map(SessionAudioQueue::Pcm16)
        }
    }

    pub(crate) fn matches_audio_quality(&self, quality: &str) -> bool {
        let profile = self
            .status
            .lock()
            .expect("P5-D session status lock poisoned")
            .negotiated_format
            .map(|format| format.profile);
        matches!(
            (quality, profile),
            ("standard", None) | ("hifi", Some(2)) | ("hires", Some(3))
        )
    }

    pub(crate) fn pcm_payload_bytes_sent(&self) -> u64 {
        self.pcm_payload_bytes_sent.load(Ordering::Relaxed)
    }

    pub(crate) fn set_paused(&self, paused: bool) -> Result<(), String> {
        self.commands
            .send(OwnerCommand::SetPaused(paused))
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    #[cfg(test)]
    fn sender_barrier_for_test(&self, timeout: Duration) -> Result<bool, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.commands
            .send(OwnerCommand::TestSenderBarrier(reply))
            .map_err(|_| "P5-D session owner has stopped.".to_string())?;
        response
            .recv_timeout(timeout)
            .map_err(|_| "P5-D sender barrier was not reached.".to_string())
    }

    pub(crate) fn set_mode(&self, mode: u8) -> Result<(), String> {
        if mode > 1 {
            return Err("P5-C playback mode is invalid.".into());
        }
        self.commands
            .send(OwnerCommand::SetMode(mode))
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    pub(crate) fn reconnect(&self) -> Result<(), String> {
        self.commands
            .send(OwnerCommand::Reconnect)
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    /// The caller must supply each changed endpoint; the owner never discovers or guesses one.
    pub(crate) fn update_endpoint(&self, peer: SocketAddr) -> Result<(), String> {
        self.enqueue_endpoint(SessionEndpoint::loopback(peer))
    }

    pub(crate) fn update_network_endpoint(
        &self,
        interface: &Ipv4Interface,
        peer: SocketAddr,
    ) -> Result<(), String> {
        self.enqueue_endpoint(SessionEndpoint::on_interface(interface, peer))
    }

    fn enqueue_endpoint(&self, mut endpoint: SessionEndpoint) -> Result<(), String> {
        endpoint.connection_intent = self.connection_intent;
        endpoint.background_authorization_required = self.background_authorization_required;
        endpoint.background_request_id = Some(random_request_id()?);
        endpoint.validate()?;
        self.commands
            .send(OwnerCommand::UpdateEndpoint(endpoint))
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    pub(crate) fn cancel(&mut self) {
        self.enabled.store(false, Ordering::Release);
        self.cancellation.cancel();
        self.join();
    }

    pub(crate) fn stop(&mut self) {
        if self.worker.is_none() {
            return;
        }
        if self.commands.send(OwnerCommand::Stop).is_err() {
            self.enabled.store(false, Ordering::Release);
            self.cancellation.cancel();
        } else {
            let deadline = Instant::now() + Duration::from_millis(100);
            while self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(5));
            }
            if self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
            {
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
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

impl Drop for SessionOwner {
    fn drop(&mut self) {
        self.cancel();
    }
}

enum BackgroundTrustCommandOutcome {
    Continue,
    Stop,
    ConnectionFailed(String),
}

fn reject_background_trust_command(command: OwnerCommand, message: &str) {
    match command {
        OwnerCommand::BackgroundTrustQuery { reply, .. } => {
            let _ = reply.send(Err(message.into()));
        }
        OwnerCommand::BackgroundTrustRevoke { reply, .. } => {
            let _ = reply.send(Err(message.into()));
        }
        _ => {}
    }
}

fn execute_background_trust_command(
    session: &mut HostSession,
    owner_generation: u64,
    enabled: &AtomicBool,
    command: OwnerCommand,
) -> BackgroundTrustCommandOutcome {
    match command {
        OwnerCommand::BackgroundTrustQuery {
            session_generation,
            deadline,
            cancellation,
            reply,
        } => {
            let result = if cancellation.is_cancelled() || Instant::now() >= deadline {
                Err("背景授权查询已取消或超时。".into())
            } else if session_generation != owner_generation {
                Err("活动会话代际已变化；请刷新授权状态后重试。".into())
            } else {
                session.query_background_trust_until(deadline, &cancellation)
            };
            let receiver_user_stopped =
                session.terminal_cause() == Some(TerminalCause::ReceiverUserStop);
            let connection_failed =
                !receiver_user_stopped && session.state() == ReceiverState::Stopped;
            let error = result.as_ref().err().cloned();
            let _ = reply.send(result);
            if receiver_user_stopped {
                enabled.store(false, Ordering::Release);
                BackgroundTrustCommandOutcome::Stop
            } else if connection_failed {
                BackgroundTrustCommandOutcome::ConnectionFailed(error.unwrap_or_else(|| {
                    "P5-C control channel closed during background trust query.".into()
                }))
            } else {
                BackgroundTrustCommandOutcome::Continue
            }
        }
        OwnerCommand::BackgroundTrustRevoke {
            session_generation,
            deadline,
            cancellation,
            reply,
        } => {
            let result = if cancellation.is_cancelled() || Instant::now() >= deadline {
                Err("背景授权撤销已取消或超时。".into())
            } else if session_generation != owner_generation {
                Err("活动会话代际已变化；未撤销 Receiver 授权。".into())
            } else {
                session.revoke_background_trust_until(deadline, &cancellation)
            };
            let receiver_user_stopped =
                session.terminal_cause() == Some(TerminalCause::ReceiverUserStop);
            if result.is_ok() || receiver_user_stopped {
                enabled.store(false, Ordering::Release);
                let _ = reply.send(result);
                BackgroundTrustCommandOutcome::Stop
            } else {
                let error = result.as_ref().err().cloned().unwrap_or_default();
                let connection_failed = session.state() == ReceiverState::Stopped;
                let _ = reply.send(result);
                if connection_failed {
                    BackgroundTrustCommandOutcome::ConnectionFailed(error)
                } else {
                    BackgroundTrustCommandOutcome::Continue
                }
            }
        }
        _ => BackgroundTrustCommandOutcome::Continue,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_session_owner(
    endpoint: SessionEndpoint,
    binding: BindingMaterial,
    parameters: AudioParameters,
    cancellation: CancellationToken,
    enabled: Arc<AtomicBool>,
    commands: mpsc::Receiver<OwnerCommand>,
    status: Arc<Mutex<SessionSnapshot>>,
    queue: Arc<Mutex<Option<Arc<PcmQueue>>>>,
    float_queue: Arc<Mutex<Option<Arc<FloatPcmQueue>>>>,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
    timing: SessionTiming,
    mut profile: HostAudioProfile,
    mut initial_float_queue: Option<Arc<FloatPcmQueue>>,
) {
    let mut endpoint = endpoint;
    let mut parameters = parameters;
    let mut paused = false;
    let mut desired_paused = false;
    let mut generation = 0u64;
    let mut attempts = 0u64;
    let mut retry_index = 0;
    let mut last_error = None;
    let mut last_active_at = None;
    let mut recovery_started_at = None;
    'owner: loop {
        if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
            break;
        }
        {
            let mut snapshot = status.lock().expect("P5-D session status lock poisoned");
            snapshot.negotiated_format = None;
            snapshot.playback_diagnostics = None;
        }
        generation = generation.saturating_add(1);
        attempts = attempts.saturating_add(1);
        publish_session(
            &status,
            SessionLifecycleState::Reconnecting,
            generation,
            attempts,
            None,
            paused,
            last_error.clone(),
        );
        let result = HostSession::authenticate_for_profile(
            endpoint,
            &binding,
            parameters,
            profile,
            &mut initial_float_queue,
            cancellation.clone(),
        );
        let mut session = match result {
            Ok(session) => session,
            Err(error) => {
                if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
                    break;
                }
                last_error = Some(error);
                // An owner may recover only after it has actually established a live
                // session. An explicit discovery/receive intent does not turn a failed
                // first connection into background retry policy.
                if last_active_at.is_none() {
                    break 'owner;
                }
                publish_session(
                    &status,
                    SessionLifecycleState::Reconnecting,
                    generation,
                    attempts,
                    None,
                    paused,
                    last_error.clone(),
                );
                match wait_for_reconnect(
                    lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
                    &commands,
                    &cancellation,
                    &enabled,
                    &mut endpoint,
                    &mut parameters,
                    &mut desired_paused,
                ) {
                    RetryWait::Stop => break,
                    RetryWait::ReconnectNow => retry_index = 0,
                    RetryWait::DelayElapsed => {
                        retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                        if recovery_window_expired(
                            recovery_started_at,
                            timing.recovery_window,
                            Instant::now(),
                        ) {
                            break 'owner;
                        }
                    }
                }
                continue 'owner;
            }
        };
        if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
            break;
        }
        if let Err(error) = session.negotiate_capabilities() {
            last_error = Some(error);
            drop(session);
            publish_session(
                &status,
                SessionLifecycleState::Stopped,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            break 'owner;
        }
        let capabilities = session.capabilities().cloned();
        let terminal_cause_supported = capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_TERMINAL_CAUSE != 0);
        status
            .lock()
            .expect("P5-D session status lock poisoned")
            .capabilities = capabilities.clone();
        status
            .lock()
            .expect("P5-D session status lock poisoned")
            .negotiated_format = session.format_v2;
        if session.format_v2.is_some()
            && capabilities.as_ref().is_some_and(|info| {
                info.management_compatible
                    && !info.compatible
                    && info.capability_bits & CAP_AUDIO_V2 != 0
            })
        {
            last_error = Some(
                "Receiver rejected the requested authenticated PCM24 format; this V2 descriptor is unsupported.".into(),
            );
        }
        publish_session(
            &status,
            SessionLifecycleState::Authenticated,
            generation,
            attempts,
            Some(session.session_id()),
            paused,
            last_error.clone(),
        );
        if !capabilities
            .as_ref()
            .is_some_and(|info| info.management_compatible)
        {
            last_error = Some("Receiver authenticated status is incompatible; update it through authorized ADB or manual installation.".into());
            drop(session);
            publish_session(
                &status,
                SessionLifecycleState::Stopped,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
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
                    match session.heartbeat() {
                        Ok(ReceiverState::Stopped)
                            if finish_receiver_user_stop(&mut session, &status) =>
                        {
                            stop_owner = true;
                            break;
                        }
                        Ok(ReceiverState::Stopped) => {
                            management_error = Some(
                                "Receiver stopped without an authenticated terminal cause.".into(),
                            );
                            restart = true;
                            break;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            management_error = Some(error);
                            restart = true;
                            break;
                        }
                    }
                    next_heartbeat = Instant::now() + timing.heartbeat_interval;
                    continue;
                }
                match commands.recv_timeout(
                    next_heartbeat
                        .saturating_duration_since(Instant::now())
                        .min(SESSION_IO_POLL),
                ) {
                    Ok(OwnerCommand::Stop) => {
                        enabled.store(false, Ordering::Release);
                        let _ = session.stop();
                        stop_owner = true;
                        break;
                    }
                    Ok(OwnerCommand::Reconnect) => {
                        restart = true;
                        break;
                    }
                    Ok(command @ OwnerCommand::BackgroundTrustQuery { .. })
                    | Ok(command @ OwnerCommand::BackgroundTrustRevoke { .. }) => {
                        match execute_background_trust_command(
                            &mut session,
                            generation,
                            &enabled,
                            command,
                        ) {
                            BackgroundTrustCommandOutcome::Continue => {}
                            BackgroundTrustCommandOutcome::Stop => {
                                stop_owner = true;
                                break;
                            }
                            BackgroundTrustCommandOutcome::ConnectionFailed(error) => {
                                management_error = Some(error);
                                restart = true;
                                break;
                            }
                        }
                    }
                    #[cfg(test)]
                    Ok(OwnerCommand::TestSenderBarrier(reply)) => drop(reply),
                    Ok(OwnerCommand::UpdateEndpoint(next)) => {
                        endpoint = next;
                        restart = true;
                        break;
                    }
                    Ok(OwnerCommand::SetMode(mode)) => {
                        parameters.mode = mode;
                        if let HostAudioProfile::Pcm24(current) = profile {
                            profile = match AudioFormatV2::for_profile(current.profile, mode) {
                                Ok(format) => HostAudioProfile::Pcm24(format),
                                Err(error) => {
                                    management_error = Some(error.to_string());
                                    restart = true;
                                    break;
                                }
                            };
                        }
                        // The phone applies a playback mode only in a fresh handshake, so this
                        // restarts the session. Start the recovery window here: without it a failed
                        // switch could report "reconnecting" forever.
                        if recovery_started_at.is_none() {
                            recovery_started_at = Some(Instant::now());
                            status
                                .lock()
                                .expect("P5-D session status lock poisoned")
                                .recovery_started_at = recovery_started_at;
                        }
                        restart = true;
                        break;
                    }
                    Ok(OwnerCommand::SetPaused(_)) => {}
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        enabled.store(false, Ordering::Release);
                        stop_owner = true;
                        break;
                    }
                }
            }
            if finish_receiver_user_stop(&mut session, &status) {
                last_error = None;
                stop_owner = true;
            }
            session.close_local();
            drop(session);
            if stop_owner || !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
                break 'owner;
            }
            last_error = management_error.clone();
            if management_error.is_some() && (last_active_at.is_none() || !terminal_cause_supported)
            {
                publish_session(
                    &status,
                    SessionLifecycleState::Stopped,
                    generation,
                    attempts,
                    None,
                    paused,
                    last_error.clone(),
                );
                break 'owner;
            }
            publish_session(
                &status,
                SessionLifecycleState::Reconnecting,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            if restart {
                if management_error.is_some() {
                    match wait_for_reconnect(
                        lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
                        &commands,
                        &cancellation,
                        &enabled,
                        &mut endpoint,
                        &mut parameters,
                        &mut desired_paused,
                    ) {
                        RetryWait::Stop => break 'owner,
                        RetryWait::ReconnectNow => retry_index = 0,
                        RetryWait::DelayElapsed => {
                            retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1)
                        }
                    }
                } else {
                    retry_index = 0;
                }
            }
            continue 'owner;
        }
        if let Err(error) = session.authorize_background_start() {
            last_error = Some(error);
            if finish_receiver_user_stop(&mut session, &status) {
                break 'owner;
            }
            drop(session);
            publish_session(
                &status,
                SessionLifecycleState::Stopped,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            break 'owner;
        }
        if let Err(error) = session.bind_audio().and_then(|_| session.select_mode()) {
            last_error = Some(error);
            if finish_receiver_user_stop(&mut session, &status) {
                break 'owner;
            }
            drop(session);
            if last_active_at.is_none() {
                break 'owner;
            }
            publish_session(
                &status,
                SessionLifecycleState::Reconnecting,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            match wait_for_reconnect(
                lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
                &commands,
                &cancellation,
                &enabled,
                &mut endpoint,
                &mut parameters,
                &mut desired_paused,
            ) {
                RetryWait::Stop => break,
                RetryWait::ReconnectNow => retry_index = 0,
                RetryWait::DelayElapsed => {
                    retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                    if recovery_window_expired(
                        recovery_started_at,
                        timing.recovery_window,
                        Instant::now(),
                    ) {
                        break 'owner;
                    }
                }
            }
            continue 'owner;
        }
        match apply_pending_commands(
            &commands,
            &mut endpoint,
            &mut parameters,
            &mut desired_paused,
            &enabled,
        ) {
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
        if cancellation.is_cancelled() || !enabled.load(Ordering::Acquire) {
            break;
        }
        publish_session(
            &status,
            SessionLifecycleState::TransportConnected,
            generation,
            attempts,
            Some(session.session_id()),
            paused,
            None,
        );
        if let Err(error) = session.start().and_then(|_| {
            if desired_paused {
                session.pause()
            } else {
                Ok(())
            }
        }) {
            last_error = Some(error);
            if finish_receiver_user_stop(&mut session, &status) {
                break 'owner;
            }
            drop(session);
            if last_active_at.is_none() {
                break 'owner;
            }
            publish_session(
                &status,
                SessionLifecycleState::Reconnecting,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            match wait_for_reconnect(
                lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
                &commands,
                &cancellation,
                &enabled,
                &mut endpoint,
                &mut parameters,
                &mut desired_paused,
            ) {
                RetryWait::Stop => break,
                RetryWait::ReconnectNow => retry_index = 0,
                RetryWait::DelayElapsed => {
                    retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                    if recovery_window_expired(
                        recovery_started_at,
                        timing.recovery_window,
                        Instant::now(),
                    ) {
                        break 'owner;
                    }
                }
            }
            continue 'owner;
        }
        paused = desired_paused;
        last_active_at = Some(Instant::now());
        recovery_started_at = None;
        status
            .lock()
            .expect("P5-D session status lock poisoned")
            .recovery_started_at = None;
        let session_id = session.session_id();
        let mut last_session_bytes = 0;
        match session.audio_queue() {
            SessionAudioQueue::Pcm16(session_queue) => {
                *queue.lock().expect("P5-D PCM queue lock poisoned") = Some(session_queue);
                *float_queue
                    .lock()
                    .expect("P5-D Float32 queue lock poisoned") = None;
            }
            SessionAudioQueue::Float32(session_queue, _) => {
                *queue.lock().expect("P5-D PCM queue lock poisoned") = None;
                *float_queue
                    .lock()
                    .expect("P5-D Float32 queue lock poisoned") = Some(session_queue);
            }
        }
        publish_session(
            &status,
            if paused {
                SessionLifecycleState::Paused
            } else {
                SessionLifecycleState::ReceiverRunning
            },
            generation,
            attempts,
            Some(session_id),
            paused,
            None,
        );
        retry_index = 0;
        let mut endpoint_changed = false;
        let mut stop_owner = false;
        let mut next_heartbeat = Instant::now() + timing.heartbeat_interval;
        loop {
            if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
                stop_owner = true;
                break;
            }
            if let Err(error) = session.check_sender() {
                last_error = Some(format!("P5-D UDP sender stopped: {error}"));
                break;
            }
            let sent = session.pcm_payload_bytes_sent();
            pcm_payload_bytes_sent
                .fetch_add(sent.saturating_sub(last_session_bytes), Ordering::Relaxed);
            last_session_bytes = sent;
            if Instant::now() >= next_heartbeat {
                match session.heartbeat() {
                    Ok(ReceiverState::Stopped)
                        if session.terminal_cause() == Some(TerminalCause::ReceiverUserStop) =>
                    {
                        let mut current = status.lock().expect("P5-D session status lock poisoned");
                        current.state = SessionLifecycleState::Stopped;
                        current.session_id = None;
                        current.terminal_cause = Some(TerminalCause::ReceiverUserStop);
                        stop_owner = true;
                        break;
                    }
                    Ok(ReceiverState::Stopped) => {
                        last_error = Some(
                            "P5-C receiver stopped without an authenticated terminal cause.".into(),
                        );
                        break;
                    }
                    Ok(state) => {
                        match session.playback_progress() {
                            Ok(Some(mut diagnostics)) => {
                                let audio_queue = session.audio_queue();
                                let previous = status
                                    .lock()
                                    .expect("P5-D session status lock poisoned")
                                    .playback_diagnostics
                                    .as_ref()
                                    .and_then(|current| current.underrun_count);
                                diagnostics.sample_rate = Some(session.parameters.sample_rate);
                                diagnostics.channels = Some(session.parameters.channels);
                                let (dropped_frames, queued_frames) = match audio_queue {
                                    SessionAudioQueue::Pcm16(queue) => {
                                        (queue.dropped_frames(), queue.queued_frames())
                                    }
                                    SessionAudioQueue::Float32(queue, _) => {
                                        (queue.dropped_frames(), queue.queued_frames())
                                    }
                                };
                                diagnostics.pc_dropped_frames = Some(dropped_frames);
                                diagnostics.pc_queued_frames = Some(queued_frames);
                                diagnostics.underrun_increment =
                                    match (previous, diagnostics.underrun_count) {
                                        (Some(previous), Some(current)) if current >= previous => {
                                            Some((current - previous) as u64)
                                        }
                                        _ => None,
                                    };
                                status
                                    .lock()
                                    .expect("P5-D session status lock poisoned")
                                    .playback_diagnostics = Some(diagnostics);
                            }
                            Ok(None) => {
                                status
                                    .lock()
                                    .expect("P5-D session status lock poisoned")
                                    .playback_diagnostics = None;
                            }
                            Err(error) => {
                                last_error = Some(error);
                                break;
                            }
                        }
                        publish_session(
                            &status,
                            lifecycle_from_receiver(state),
                            generation,
                            attempts,
                            Some(session_id),
                            paused,
                            None,
                        );
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
                    if value == paused {
                        continue;
                    }
                    let result = if value {
                        session.pause()
                    } else {
                        session.resume()
                    };
                    match result {
                        Ok(()) => {
                            paused = value;
                            let state = if paused {
                                SessionLifecycleState::Paused
                            } else {
                                SessionLifecycleState::ReceiverRunning
                            };
                            publish_session(
                                &status,
                                state,
                                generation,
                                attempts,
                                Some(session_id),
                                paused,
                                None,
                            );
                        }
                        Err(error) => {
                            last_error = Some(error);
                            break;
                        }
                    }
                }
                #[cfg(test)]
                Ok(OwnerCommand::TestSenderBarrier(reply)) => {
                    let sender_joined = session
                        .float_sender
                        .as_ref()
                        .is_none_or(FloatUdpSender::worker_joined);
                    let _ = reply.send(sender_joined);
                }
                Ok(OwnerCommand::UpdateEndpoint(next_endpoint)) => {
                    endpoint_changed = endpoint != next_endpoint;
                    endpoint = next_endpoint;
                    if !endpoint_changed {
                        continue;
                    }
                    if let Err(error) = session.stop() {
                        last_error = Some(error);
                    }
                    break;
                }
                Ok(OwnerCommand::SetMode(mode)) => {
                    if mode == parameters.mode {
                        continue;
                    }
                    parameters.mode = mode;
                    if let HostAudioProfile::Pcm24(current) = profile {
                        match AudioFormatV2::for_profile(current.profile, mode) {
                            Ok(format) => profile = HostAudioProfile::Pcm24(format),
                            Err(error) => {
                                last_error = Some(error.to_string());
                                break;
                            }
                        }
                    }
                    if let Err(error) = session.stop() {
                        last_error = Some(error);
                    }
                    endpoint_changed = true;
                    break;
                }
                Ok(OwnerCommand::Reconnect) => {
                    if finish_receiver_user_stop(&mut session, &status) {
                        stop_owner = true;
                    }
                    session.close_local();
                    endpoint_changed = true;
                    break;
                }
                Ok(command @ OwnerCommand::BackgroundTrustQuery { .. })
                | Ok(command @ OwnerCommand::BackgroundTrustRevoke { .. }) => {
                    match execute_background_trust_command(
                        &mut session,
                        generation,
                        &enabled,
                        command,
                    ) {
                        BackgroundTrustCommandOutcome::Continue => {}
                        BackgroundTrustCommandOutcome::Stop => {
                            stop_owner = true;
                            break;
                        }
                        BackgroundTrustCommandOutcome::ConnectionFailed(error) => {
                            last_error = Some(error);
                            break;
                        }
                    }
                }
                Ok(OwnerCommand::Stop) => {
                    enabled.store(false, Ordering::Release);
                    if let Err(error) = session.stop() {
                        last_error = Some(error);
                    }
                    status
                        .lock()
                        .expect("P5-D session status lock poisoned")
                        .terminal_cause = Some(TerminalCause::HostStop);
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
        if finish_receiver_user_stop(&mut session, &status) {
            last_error = None;
            stop_owner = true;
        }
        let receiver_user_stopped =
            session.terminal_cause() == Some(TerminalCause::ReceiverUserStop);
        session.close_local();
        let sent = session.pcm_payload_bytes_sent();
        pcm_payload_bytes_sent
            .fetch_add(sent.saturating_sub(last_session_bytes), Ordering::Relaxed);
        *queue.lock().expect("P5-D PCM queue lock poisoned") = None;
        *float_queue
            .lock()
            .expect("P5-D Float32 queue lock poisoned") = None;
        status
            .lock()
            .expect("P5-D session status lock poisoned")
            .negotiated_format = None;
        drop(session);
        if stop_owner || !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
            break 'owner;
        }
        if endpoint_changed {
            recovery_started_at = None;
            status
                .lock()
                .expect("P5-D session status lock poisoned")
                .recovery_started_at = None;
        } else if last_active_at.is_some() && recovery_started_at.is_none() {
            recovery_started_at = Some(Instant::now());
            status
                .lock()
                .expect("P5-D session status lock poisoned")
                .recovery_started_at = recovery_started_at;
        }
        if receiver_user_stopped || stop_owner {
            break 'owner;
        }
        if !endpoint_changed && !terminal_cause_supported {
            last_error = Some(
                "Receiver disconnected without terminal-cause capability; update it or reconnect manually.".into(),
            );
            publish_session(
                &status,
                SessionLifecycleState::Stopped,
                generation,
                attempts,
                None,
                paused,
                last_error.clone(),
            );
            break 'owner;
        }
        publish_session(
            &status,
            SessionLifecycleState::Reconnecting,
            generation,
            attempts,
            None,
            paused,
            last_error.clone(),
        );
        if endpoint_changed {
            retry_index = 0;
            continue 'owner;
        }
        match wait_for_reconnect(
            lifecycle_retry_delay(&timing.reconnect_delays, retry_index),
            &commands,
            &cancellation,
            &enabled,
            &mut endpoint,
            &mut parameters,
            &mut desired_paused,
        ) {
            RetryWait::Stop => break,
            RetryWait::ReconnectNow => retry_index = 0,
            RetryWait::DelayElapsed => {
                retry_index = (retry_index + 1).min(timing.reconnect_delays.len() - 1);
                if recovery_window_expired(
                    recovery_started_at,
                    timing.recovery_window,
                    Instant::now(),
                ) {
                    break 'owner;
                }
            }
        }
    }
    *queue.lock().expect("P5-D PCM queue lock poisoned") = None;
    *float_queue
        .lock()
        .expect("P5-D Float32 queue lock poisoned") = None;
    {
        let mut current = status.lock().expect("P5-D session status lock poisoned");
        if current.terminal_cause.is_none() {
            if last_error.is_some() {
                current.terminal_cause = Some(TerminalCause::TransportFailure);
            }
        }
    }
    publish_session(
        &status,
        SessionLifecycleState::Stopped,
        generation,
        attempts,
        None,
        paused,
        last_error,
    );
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

fn finish_receiver_user_stop(session: &mut HostSession, status: &Mutex<SessionSnapshot>) -> bool {
    if session.terminal_cause() != Some(TerminalCause::ReceiverUserStop) {
        return false;
    }
    let mut current = status.lock().expect("P5-D session status lock poisoned");
    current.state = SessionLifecycleState::Stopped;
    current.session_id = None;
    current.terminal_cause = Some(TerminalCause::ReceiverUserStop);
    current.last_error = None;
    true
}

fn publish_session(
    status: &Mutex<SessionSnapshot>,
    state: SessionLifecycleState,
    generation: u64,
    attempts: u64,
    session_id: Option<u64>,
    paused: bool,
    last_error: Option<String>,
) {
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

enum RetryWait {
    Stop,
    ReconnectNow,
    DelayElapsed,
}

enum PendingCommands {
    Stop,
    EndpointChanged,
    ModeChanged,
    Reconnect,
    Applied,
}

fn apply_pending_commands(
    commands: &mpsc::Receiver<OwnerCommand>,
    endpoint: &mut SessionEndpoint,
    parameters: &mut AudioParameters,
    paused: &mut bool,
    enabled: &AtomicBool,
) -> PendingCommands {
    let mut endpoint_changed = false;
    let mut mode_changed = false;
    let mut reconnect = false;
    loop {
        match commands.try_recv() {
            Ok(OwnerCommand::SetPaused(value)) => *paused = value,
            #[cfg(test)]
            Ok(OwnerCommand::TestSenderBarrier(reply)) => drop(reply),
            Ok(command @ OwnerCommand::BackgroundTrustQuery { .. })
            | Ok(command @ OwnerCommand::BackgroundTrustRevoke { .. }) => {
                reject_background_trust_command(
                    command,
                    "活动会话代际已变化；请刷新授权状态后重试。",
                );
            }
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
    if reconnect {
        PendingCommands::Reconnect
    } else if endpoint_changed {
        PendingCommands::EndpointChanged
    } else if mode_changed {
        PendingCommands::ModeChanged
    } else {
        PendingCommands::Applied
    }
}

fn wait_for_reconnect(
    delay: Duration,
    commands: &mpsc::Receiver<OwnerCommand>,
    cancellation: &CancellationToken,
    enabled: &AtomicBool,
    endpoint: &mut SessionEndpoint,
    parameters: &mut AudioParameters,
    paused: &mut bool,
) -> RetryWait {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline {
        if !enabled.load(Ordering::Acquire) || cancellation.is_cancelled() {
            return RetryWait::Stop;
        }
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .min(SESSION_IO_POLL);
        match commands.recv_timeout(remaining) {
            Ok(OwnerCommand::SetPaused(value)) => *paused = value,
            #[cfg(test)]
            Ok(OwnerCommand::TestSenderBarrier(reply)) => drop(reply),
            Ok(command @ OwnerCommand::BackgroundTrustQuery { .. })
            | Ok(command @ OwnerCommand::BackgroundTrustRevoke { .. }) => {
                reject_background_trust_command(
                    command,
                    "Receiver 正在重连；请在认证会话恢复后重试。",
                );
            }
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
                if parameters.mode != mode {
                    parameters.mode = mode;
                    return RetryWait::ReconnectNow;
                }
            }
            Ok(OwnerCommand::Reconnect) => return RetryWait::ReconnectNow,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                enabled.store(false, Ordering::Release);
                return RetryWait::Stop;
            }
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

fn random_request_id() -> Result<[u8; 16], String> {
    loop {
        let mut request_id = [0; 16];
        getrandom::fill(&mut request_id)
            .map_err(|_| "Unable to generate a background-start request ID.".to_string())?;
        if request_id.iter().any(|byte| *byte != 0) {
            return Ok(request_id);
        }
    }
}

pub(crate) struct Ack {
    pub(crate) result: u8,
    pub(crate) state: ReceiverState,
    pub(crate) udp_port: u16,
    pub(crate) playback_progress: Option<PlaybackDiagnostics>,
    background_authorization: Option<([u8; 16], u8)>,
    background_trust: Option<BackgroundTrustSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BackgroundTrustSnapshot {
    commitment: [u8; 32],
    pub(crate) kind: u8,
    pub(crate) persistence: u8,
    pub(crate) remaining_ms: u64,
}

pub(crate) fn encode_control(
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
    sequence: u64,
    kind: u8,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
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

pub(crate) fn encode_capabilities_query(
    control_major: u16,
    audio_major: u8,
    required: u32,
) -> [u8; 8] {
    let mut payload = [0; 8];
    payload[..2].copy_from_slice(&control_major.to_be_bytes());
    payload[2] = audio_major;
    payload[4..].copy_from_slice(&required.to_be_bytes());
    payload
}

pub(crate) fn decode_capabilities_info(
    frame: &[u8],
    response_sequence: u64,
    query_sequence: u64,
) -> Result<ReceiverCapabilities, String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES {
        return Err("Receiver capability response frame was truncated.".into());
    }
    if frame[5] != CAPABILITIES_INFO {
        return Err("Receiver does not support authenticated capability negotiation; use authorized ADB or manual installation.".into());
    }
    if u64::from_be_bytes(frame[20..28].try_into().unwrap()) != response_sequence
        || u16::from_be_bytes([frame[28], frame[29]]) != 20
    {
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
    Ok(ReceiverCapabilities {
        control_major,
        audio_major,
        management_compatible,
        audio_compatible,
        compatible,
        capability_bits,
        version_code,
    })
}

fn write_frame(
    tcp: &mut TcpStream,
    frame: &[u8],
    cancellation: &CancellationToken,
) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES || frame.len() > MAX_CONTROL_FRAME {
        return Err("P5-C control frame length is invalid.".into());
    }
    write_all_cancellable(tcp, &(frame.len() as u16).to_be_bytes(), cancellation)
        .map_err(|_| "P5-C control write failed.".to_string())?;
    write_all_cancellable(tcp, frame, cancellation)
        .map_err(|_| "P5-C control write failed.".to_string())?;
    tcp.flush().map_err(|_| "P5-C control write failed.".into())
}

fn write_frame_until(
    tcp: &mut TcpStream,
    frame: &[u8],
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES || frame.len() > MAX_CONTROL_FRAME {
        return Err("P5-C control frame length is invalid.".into());
    }
    write_all_cancellable_until(
        tcp,
        &(frame.len() as u16).to_be_bytes(),
        cancellation,
        deadline,
    )
    .map_err(|_| "P5-C control write failed or approval deadline expired.".to_string())?;
    write_all_cancellable_until(tcp, frame, cancellation, deadline)
        .map_err(|_| "P5-C control write failed or approval deadline expired.".to_string())?;
    tcp.flush().map_err(|_| "P5-C control write failed.".into())
}

fn write_frame_until_request(
    tcp: &mut TcpStream,
    frame: &[u8],
    owner_cancellation: &CancellationToken,
    request_cancellation: &CancellationToken,
    deadline: Instant,
    bytes_written: &mut bool,
) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES || frame.len() > MAX_CONTROL_FRAME {
        return Err("P5-C control frame length is invalid.".into());
    }
    write_all_cancellable_until_request(
        tcp,
        &(frame.len() as u16).to_be_bytes(),
        owner_cancellation,
        request_cancellation,
        deadline,
        bytes_written,
    )
    .map_err(|_| "P5-C background trust write failed, expired, or was cancelled.".to_string())?;
    write_all_cancellable_until_request(
        tcp,
        frame,
        owner_cancellation,
        request_cancellation,
        deadline,
        bytes_written,
    )
    .map_err(|_| "P5-C background trust write failed, expired, or was cancelled.".to_string())?;
    check_cancelled(owner_cancellation).map_err(|_| "P5-C session was cancelled.".to_string())?;
    check_cancelled(request_cancellation)
        .map_err(|_| "P5-C background trust request was cancelled.".to_string())?;
    tcp.flush().map_err(|_| "P5-C control write failed.".into())
}

fn read_raw_frame(
    tcp: &mut TcpStream,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, String> {
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

fn read_frame(
    tcp: &mut TcpStream,
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, String> {
    let frame = read_raw_frame(tcp, cancellation)?;
    verify_control_frame(&frame, key, direction, session_id)?;
    Ok(frame)
}

/// Validates a length-delimited P5-C frame. AOA's ANPU envelope supplies its
/// length; LAN keeps its existing TCP prefix and calls this same validator.
pub(crate) fn verify_control_frame(
    frame: &[u8],
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES
        || frame.len() > MAX_CONTROL_FRAME
        || &frame[..4] != CONTROL_MAGIC
        || frame[4] != 1
        || frame[6..8] != [0, 0]
        || u32::from_be_bytes(frame[8..12].try_into().unwrap()) != direction
        || u64::from_be_bytes(frame[12..20].try_into().unwrap()) != session_id
    {
        return Err("P5-C control identity or header was rejected.".into());
    }
    let payload_len = u16::from_be_bytes([frame[28], frame[29]]) as usize;
    if frame.len() != CONTROL_HEADER_BYTES + payload_len + CONTROL_TAG_BYTES {
        return Err("P5-C control length was rejected.".into());
    }
    let body_len = CONTROL_HEADER_BYTES + payload_len;
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "P5-C HMAC key was invalid.")?;
    mac.update(&frame[..body_len]);
    mac.verify_slice(&frame[body_len..])
        .map_err(|_| "P5-C control authentication failed.".into())
}

fn read_frame_until(
    tcp: &mut TcpStream,
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    let mut prefix = [0; 2];
    read_exact_cancellable_until(tcp, &mut prefix, cancellation, deadline)
        .map_err(|_| "P5-C control peer closed or approval deadline expired.".to_string())?;
    let length = u16::from_be_bytes(prefix) as usize;
    if !(CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES..=MAX_CONTROL_FRAME).contains(&length) {
        return Err("P5-C control frame length is invalid.".into());
    }
    let mut frame = vec![0; length];
    read_exact_cancellable_until(tcp, &mut frame, cancellation, deadline)
        .map_err(|_| "P5-C control peer closed or approval deadline expired.".to_string())?;
    verify_control_frame(&frame, key, direction, session_id)?;
    Ok(frame)
}

fn read_frame_until_request(
    tcp: &mut TcpStream,
    key: &[u8; 32],
    direction: u32,
    session_id: u64,
    owner_cancellation: &CancellationToken,
    request_cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    let mut prefix = [0; 2];
    read_exact_cancellable_until_request(
        tcp,
        &mut prefix,
        owner_cancellation,
        request_cancellation,
        deadline,
    )
    .map_err(|_| {
        "P5-C background trust peer closed, expired, or request was cancelled.".to_string()
    })?;
    let length = u16::from_be_bytes(prefix) as usize;
    if !(CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES..=MAX_CONTROL_FRAME).contains(&length) {
        return Err("P5-C control frame length is invalid.".into());
    }
    let mut frame = vec![0; length];
    read_exact_cancellable_until_request(
        tcp,
        &mut frame,
        owner_cancellation,
        request_cancellation,
        deadline,
    )
    .map_err(|_| {
        "P5-C background trust peer closed, expired, or request was cancelled.".to_string()
    })?;
    verify_control_frame(&frame, key, direction, session_id)?;
    Ok(frame)
}

fn decode_ack(
    frame: &[u8],
    expected_sequence: u64,
    command: u8,
    command_sequence: u64,
) -> Result<Ack, String> {
    decode_ack_with_format(frame, expected_sequence, command, command_sequence, false)
}

pub(crate) fn decode_ack_with_format(
    frame: &[u8],
    expected_sequence: u64,
    command: u8,
    command_sequence: u64,
    v2_audio: bool,
) -> Result<Ack, String> {
    let payload_len = u16::from_be_bytes([frame[28], frame[29]]) as usize;
    let terminal_ack = payload_len == 13 && frame[CONTROL_HEADER_BYTES + 9] == RESULT_USER_STOPPED;
    if frame[5] != ACK
        || u64::from_be_bytes(frame[20..28].try_into().unwrap()) != expected_sequence
        || (!terminal_ack
            && match command {
                PLAYBACK_PROGRESS_QUERY => payload_len != if v2_audio { 91 } else { 83 },
                BACKGROUND_START_REQUEST | BACKGROUND_START_STATUS | BACKGROUND_START_CANCEL => {
                    payload_len != 30
                }
                BACKGROUND_TRUST_QUERY | BACKGROUND_TRUST_REVOKE => payload_len != 55,
                _ => payload_len != 13,
            })
    {
        return Err("P5-C ACK sequence or type was invalid.".into());
    }
    let payload = &frame[CONTROL_HEADER_BYTES..CONTROL_HEADER_BYTES + payload_len];
    if payload[0] != command
        || u64::from_be_bytes(payload[1..9].try_into().unwrap()) != command_sequence
    {
        return Err("P5-C ACK did not match its command.".into());
    }
    let state = ReceiverState::from_wire(payload[10])?;
    let result = payload[9];
    if result > RESULT_USER_STOPPED {
        return Err("P5-C ACK result code was invalid.".into());
    }
    let udp_port = u16::from_be_bytes([payload[11], payload[12]]);
    if result == RESULT_USER_STOPPED
        && (payload_len != 13 || state != ReceiverState::Stopped || udp_port != 0)
    {
        return Err("P5-C Receiver terminal cause was invalid.".into());
    }
    let playback_progress = if command == PLAYBACK_PROGRESS_QUERY && !terminal_ack {
        Some(decode_playback_progress_with_format(
            &payload[13..],
            v2_audio,
        )?)
    } else {
        None
    };
    let background_authorization = if !terminal_ack
        && matches!(
            command,
            BACKGROUND_START_REQUEST | BACKGROUND_START_STATUS | BACKGROUND_START_CANCEL
        ) {
        let request_id: [u8; 16] = payload[13..29].try_into().unwrap();
        let status = payload[29];
        if status > AUTH_CANCELLED {
            return Err("P5-C background authorization status was invalid.".into());
        }
        Some((request_id, status))
    } else {
        None
    };
    let background_trust =
        if !terminal_ack && matches!(command, BACKGROUND_TRUST_QUERY | BACKGROUND_TRUST_REVOKE) {
            let commitment: [u8; 32] = payload[13..45].try_into().unwrap();
            let kind = payload[45];
            let persistence = payload[46];
            let remaining_ms = u64::from_be_bytes(payload[47..55].try_into().unwrap());
            if !valid_background_trust(kind, persistence, remaining_ms) {
                return Err("P5-C background trust fields were invalid.".into());
            }
            Some(BackgroundTrustSnapshot {
                commitment,
                kind,
                persistence,
                remaining_ms,
            })
        } else {
            None
        };
    Ok(Ack {
        result,
        state,
        udp_port,
        playback_progress,
        background_authorization,
        background_trust,
    })
}

fn valid_background_trust(kind: u8, persistence: u8, remaining_ms: u64) -> bool {
    match kind {
        0 => persistence == 0 && remaining_ms == 0,
        1 => persistence == 1 && (remaining_ms <= 30_000 || remaining_ms == u64::MAX),
        2 => {
            persistence == 1 && remaining_ms <= Duration::from_secs(8 * 60 * 60).as_millis() as u64
        }
        3 => persistence == 2 && remaining_ms == u64::MAX,
        _ => false,
    }
}

fn background_trust_management_state(state: ReceiverState) -> bool {
    matches!(
        state,
        ReceiverState::Authenticated
            | ReceiverState::TransportConnected
            | ReceiverState::ReceiverRunning
            | ReceiverState::Streaming
            | ReceiverState::Paused
    )
}

fn decode_playback_progress(data: &[u8]) -> Result<PlaybackDiagnostics, String> {
    decode_playback_progress_with_format(data, false)
}

fn decode_playback_progress_with_format(
    data: &[u8],
    v2_audio: bool,
) -> Result<PlaybackDiagnostics, String> {
    if data.len() != if v2_audio { 78 } else { 70 } {
        return Err("Receiver playback progress payload length was invalid.".into());
    }
    let advancing = match data[45] {
        0 => Some(false),
        1 => Some(true),
        0xff => None,
        _ => return Err("Receiver playback progress flag was invalid.".into()),
    };
    let u32_at = |offset| u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
    let u64_at = |offset| u64::from_be_bytes(data[offset..offset + 8].try_into().unwrap());
    let metric_u32 = |offset| {
        let value = u32_at(offset);
        (value != u32::MAX).then_some(value)
    };
    let metric_u64 = |offset| {
        let value = u64_at(offset);
        (value != u64::MAX).then_some(value)
    };
    let mut diagnostics = PlaybackDiagnostics {
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
    };
    if v2_audio {
        if data[70] != 1 || data[73] != 0 || data[71] > 3 || !matches!(data[72], 0..=2) {
            return Err("Receiver v2 playback output format was invalid.".into());
        }
        let output_sample_rate = u32_at(74);
        if data[71] == 0 && (data[72] != 0 || output_sample_rate != 0) {
            return Err("Receiver unknown v2 output format had populated track fields.".into());
        }
        diagnostics.output_format = (data[71] != 0).then_some(data[71]);
        diagnostics.opened_track_channels = (data[72] != 0).then_some(data[72]);
        diagnostics.output_sample_rate = (output_sample_rate != 0).then_some(output_sample_rate);
    }
    Ok(diagnostics)
}

pub(crate) fn hex_string(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0xf) as usize] as char);
    }
    value
}

fn control_mac(key: &[u8; 32], bytes: &[u8]) -> Result<[u8; 32], String> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "P5-C HMAC key was invalid.")?;
    mac.update(bytes);
    Ok(mac.finalize().into_bytes().into())
}

fn connect_cancellable(
    local_address: Ipv4Addr,
    peer: SocketAddr,
    cancellation: &CancellationToken,
) -> std::io::Result<TcpStream> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
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
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "P5-C operation cancelled",
        ))
    } else {
        Ok(())
    }
}

fn is_poll_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn read_exact_cancellable(
    tcp: &mut TcpStream,
    bytes: &mut [u8],
    cancellation: &CancellationToken,
) -> std::io::Result<()> {
    read_exact_cancellable_until(
        tcp,
        bytes,
        cancellation,
        Instant::now() + SESSION_IO_DEADLINE,
    )
}

fn read_exact_cancellable_until(
    tcp: &mut TcpStream,
    bytes: &mut [u8],
    cancellation: &CancellationToken,
    deadline: Instant,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(cancellation)?;
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
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

fn read_exact_cancellable_until_request(
    tcp: &mut TcpStream,
    bytes: &mut [u8],
    owner_cancellation: &CancellationToken,
    request_cancellation: &CancellationToken,
    deadline: Instant,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(owner_cancellation)?;
        check_cancelled(request_cancellation)?;
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        match tcp.read(&mut bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => offset += read,
            Err(error) if is_poll_timeout(&error) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn write_all_cancellable(
    tcp: &mut TcpStream,
    bytes: &[u8],
    cancellation: &CancellationToken,
) -> std::io::Result<()> {
    write_all_cancellable_until(
        tcp,
        bytes,
        cancellation,
        Instant::now() + SESSION_IO_DEADLINE,
    )
}

fn write_all_cancellable_until(
    tcp: &mut TcpStream,
    bytes: &[u8],
    cancellation: &CancellationToken,
    deadline: Instant,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(cancellation)?;
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
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

fn write_all_cancellable_until_request(
    tcp: &mut TcpStream,
    bytes: &[u8],
    owner_cancellation: &CancellationToken,
    request_cancellation: &CancellationToken,
    deadline: Instant,
    bytes_written: &mut bool,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancelled(owner_cancellation)?;
        check_cancelled(request_cancellation)?;
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        match tcp.write(&bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(written) => {
                offset += written;
                *bytes_written = true;
            }
            Err(error) if is_poll_timeout(&error) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
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
#[path = "p5c_tests.rs"]
mod tests;
