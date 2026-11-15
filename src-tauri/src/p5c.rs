#[cfg(test)]
use crate::protocol::{PacketEncoder, PacketType, HOST_TO_RECEIVER};
use crate::{
    network::{FloatPcmQueue, FloatUdpSender, PcmQueue, UdpSender, UdpSessionMaterial},
    network_interface::Ipv4Interface,
    protocol::{AudioFormatV2, PcmFormat},
    receiver::{PlaybackDiagnostics, StartupDiagnosticsCapture, STARTUP_DIAGNOSTICS_INTERVAL},
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
mod wire;
pub(crate) use wire::*;
mod owner_loop;
use owner_loop::*;
mod host_session;
mod session_owner;

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
pub(crate) const START_NOT_CONFIRMED: &str = "P5-C receiver did not confirm playback start.";
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
pub(crate) const CAP_AUDIO_FLOAT32: u32 = 0x0000_0400;
pub(crate) const CAP_PROGRESS_EXT: u32 = 0x0000_0800;
pub(crate) const CAP_AUDIO_ADB_STREAM: u32 = 0x0000_1000;
pub(crate) const CAP_BUFFER_METRICS: u32 = 0x0000_2000;
pub(crate) const CAP_SOURCE_FRAME_LOSS: u32 = 0x0000_4000;
pub(crate) const CAP_SOURCE_LOSS_DETAILS: u32 = 0x0000_8000;
pub(crate) const CAP_SOURCE_SEGMENT_START: u32 = 0x0001_0000;
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

pub(crate) fn requested_progress_schema(capabilities: u32) -> u8 {
    if capabilities & CAP_SOURCE_LOSS_DETAILS != 0
        && capabilities & CAP_PROGRESS_EXT != 0
        && capabilities & CAP_BUFFER_METRICS != 0
    {
        6
    } else if capabilities & CAP_SOURCE_FRAME_LOSS != 0
        && capabilities & CAP_PROGRESS_EXT != 0
        && capabilities & CAP_BUFFER_METRICS != 0
    {
        5
    } else if capabilities & CAP_BUFFER_METRICS != 0 {
        3
    } else if capabilities & CAP_PROGRESS_EXT != 0 {
        2
    } else {
        0
    }
}

fn progress_schema_for_request(kind: u8, payload: &[u8]) -> u8 {
    if kind != PLAYBACK_PROGRESS_QUERY {
        return 0;
    }
    match payload {
        [2] => 2,
        [3] => 3,
        [4] => 4,
        [5] => 5,
        [6] => 6,
        _ => 0,
    }
}
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
    harmony: bool,
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
            harmony: false,
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
            harmony: false,
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

impl SessionAudioQueue {
    pub(crate) fn timing(&self) -> &crate::timing::CaptureSendTiming {
        match self {
            Self::Pcm16(queue) => &queue.timing,
            Self::Float32(queue, _) => &queue.timing,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostAudioProfile {
    Standard,
    AudioV2(AudioFormatV2),
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
    ReconfigureAudio {
        profile: HostAudioProfile,
        parameters: AudioParameters,
        paused: bool,
    },
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
    harmony: bool,
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

impl Drop for SessionOwner {
    fn drop(&mut self) {
        self.cancel();
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
