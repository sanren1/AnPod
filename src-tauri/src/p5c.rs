use crate::{
    network::{PcmQueue, UdpSender, UdpSessionMaterial},
    protocol::{PacketEncoder, PacketType, PcmFormat, HOST_TO_RECEIVER},
    security::{AudioParameters, BindingMaterial, HostHandshake},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    sync::Arc,
    time::{Duration, Instant},
};

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
const ACK: u8 = 127;
const RESULT_OK: u8 = 0;
const RESULT_REJECTED: u8 = 1;
const RESULT_FAILED: u8 = 2;
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
type HmacSha256 = Hmac<Sha256>;

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

impl Drop for SessionKeys {
    fn drop(&mut self) {
        self.audio.fill(0);
        self.control_tx.fill(0);
        self.control_rx.fill(0);
    }
}

/// Explicit host side of one authenticated loopback session. It has no listener or product caller.
pub(crate) struct HostSession {
    tcp: Option<TcpStream>,
    udp_socket: Option<UdpSocket>,
    sender: Option<UdpSender>,
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
}

impl HostSession {
    pub(crate) fn authenticate_loopback(peer: SocketAddr, material: BindingMaterial,
        parameters: AudioParameters) -> Result<Self, String> {
        parameters.validate().map_err(|_| "Unsupported P5-C PCM parameters.".to_string())?;
        if peer.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) || peer.port() == 0 {
            return Err("P5-C accepts only an explicit 127.0.0.1 peer.".into());
        }
        let udp_socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "Unable to bind the P5-C loopback audio socket.".to_string())?;
        let mut tcp = TcpStream::connect_timeout(&peer, Duration::from_secs(3))
            .map_err(|_| "Unable to connect to the explicit P5-C loopback peer.".to_string())?;
        tcp.set_read_timeout(Some(Duration::from_secs(3))).map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;
        tcp.set_write_timeout(Some(Duration::from_secs(3))).map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;

        let (handshake, hello) = HostHandshake::start(material, parameters)
            .map_err(|_| "P5-A handshake could not start.".to_string())?;
        tcp.write_all(&hello).map_err(|_| "P5-C peer closed during P5-A HELLO.".to_string())?;
        let mut challenge = [0u8; 86];
        tcp.read_exact(&mut challenge).map_err(|_| "P5-C peer closed during P5-A CHALLENGE.".to_string())?;
        let (await_ready, finish) = handshake.accept_challenge(&challenge)
            .map_err(|_| "P5-A receiver identity or proof was rejected.".to_string())?;
        tcp.write_all(&finish).map_err(|_| "P5-C peer closed during P5-A FINISH.".to_string())?;
        let mut ready = [0u8; 38];
        tcp.read_exact(&mut ready).map_err(|_| "P5-C peer closed during P5-A READY.".to_string())?;
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
        Ok(Self {
            tcp: Some(tcp),
            udp_socket: Some(udp_socket),
            sender: None,
            queue: Arc::new(PcmQueue::for_session(format)),
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
        })
    }

    pub(crate) fn connect_loopback(peer: SocketAddr, material: BindingMaterial,
        parameters: AudioParameters) -> Result<Self, String> {
        let mut session = Self::authenticate_loopback(peer, material, parameters)?;
        if let Err(error) = session.bind_audio().and_then(|_| session.select_mode()) {
            session.close_local();
            return Err(error);
        }
        Ok(session)
    }

    pub(crate) fn state(&self) -> ReceiverState { self.state }
    pub(crate) fn pcm_queue(&self) -> Arc<PcmQueue> { self.queue.clone() }

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
                material, self.parameters)?);
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
        write_frame(tcp, &frame)?;
        let ack_frame = read_frame(tcp, &self.keys.control_rx, RECEIVER_TO_HOST_CONTROL, self.session_id)?;
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
        write_frame(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?, &frame)?;
        let actual_ack = read_raw_frame(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)?;
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
        write_frame(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?, &frame)?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    fn send_old_session_for_test(&mut self) -> Result<(), String> {
        let port = self.udp_socket.as_ref().ok_or("P5-C UDP socket is closed.")?
            .local_addr().map_err(|_| "UDP socket unavailable".to_string())?.port();
        let frame = encode_control(&self.keys.control_tx, HOST_TO_RECEIVER_CONTROL,
            self.session_id.wrapping_sub(1), self.command_sequence, BIND, &port.to_be_bytes())?;
        write_frame(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?, &frame)?;
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

struct Ack { result: u8, state: ReceiverState, udp_port: u16 }

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

fn write_frame(tcp: &mut TcpStream, frame: &[u8]) -> Result<(), String> {
    if frame.len() < CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES || frame.len() > MAX_CONTROL_FRAME {
        return Err("P5-C control frame length is invalid.".into());
    }
    tcp.write_all(&(frame.len() as u16).to_be_bytes()).and_then(|_| tcp.write_all(frame))
        .and_then(|_| tcp.flush()).map_err(|_| "P5-C control write failed.".into())
}

fn read_raw_frame(tcp: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut prefix = [0; 2];
    tcp.read_exact(&mut prefix).map_err(|_| "P5-C control peer closed.".to_string())?;
    let length = u16::from_be_bytes(prefix) as usize;
    if !(CONTROL_HEADER_BYTES + CONTROL_TAG_BYTES..=MAX_CONTROL_FRAME).contains(&length) {
        return Err("P5-C control frame length is invalid.".into());
    }
    let mut frame = vec![0; length];
    tcp.read_exact(&mut frame).map_err(|_| "P5-C control peer closed.".to_string())?;
    Ok(frame)
}

fn read_frame(tcp: &mut TcpStream, key: &[u8; 32], direction: u32, session_id: u64) -> Result<Vec<u8>, String> {
    let frame = read_raw_frame(tcp)?;
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
    if frame[5] != ACK || u64::from_be_bytes(frame[20..28].try_into().unwrap()) != expected_sequence
        || u16::from_be_bytes([frame[28], frame[29]]) != 13 {
        return Err("P5-C ACK sequence or type was invalid.".into());
    }
    let payload = &frame[CONTROL_HEADER_BYTES..CONTROL_HEADER_BYTES + 13];
    if payload[0] != command || u64::from_be_bytes(payload[1..9].try_into().unwrap()) != command_sequence {
        return Err("P5-C ACK did not match its command.".into());
    }
    let state = ReceiverState::from_wire(payload[10])?;
    Ok(Ack { result: payload[9], state, udp_port: u16::from_be_bytes([payload[11], payload[12]]) })
}

fn control_mac(key: &[u8; 32], bytes: &[u8]) -> Result<[u8; 32], String> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "P5-C HMAC key was invalid.")?;
    mac.update(bytes);
    Ok(mac.finalize().into_bytes().into())
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
    use crate::security::{test_binding_material, test_binding_material_with_wrong_identity,
        test_binding_material_with_wrong_secret};
    use std::{
        fs,
        io::{BufRead, BufReader},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
    };

    const JAVA_SOURCES: &[&str] = &[
        "PairingProtocol.java", "P5cControlProtocol.java", "UdpSessionContext.java",
        "UdpPacketDecoder.java", "UdpPcmQueue.java", "UdpReceiver.java",
        "P5cReceiverCoordinator.java",
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
        command.arg("-source").arg("17").arg("-target").arg("17").arg("-d").arg(&output);
        for source in JAVA_SOURCES { command.arg(package.join(source)); }
        command.arg(repo.join("tests/P5cLoopbackEndpoint.java"));
        let result = command.output().expect("run JDK 17 javac for the P5-C Java receiver");
        assert!(result.status.success(), "javac failed:\n{}\n{}",
            String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        output
    }

    fn spawn_endpoint(classes: &Path) -> (Child, BufReader<std::process::ChildStdout>, SocketAddr) {
        let (_, java) = java_paths();
        let mut child = Command::new(java).arg("-cp").arg(classes)
            .arg("app.anpod.receiver.P5cLoopbackEndpoint")
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut stdout = stdout;
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let port: u16 = line.trim().strip_prefix("PORT=").expect("Java endpoint port").parse().unwrap();
        (child, stdout, SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
    }

    fn finish_endpoint(mut child: Child, mut stdout: BufReader<std::process::ChildStdout>) -> String {
        let mut result = String::new();
        stdout.read_line(&mut result).unwrap();
        let status = child.wait().unwrap();
        assert!(status.success(), "Java P5-C test endpoint did not finish successfully");
        result.trim().strip_prefix("RESULT=").expect("Java endpoint result").to_string()
    }

    fn rejected_handshake(classes: &Path, wrong_secret: bool) {
        let (child, stdout, peer) = spawn_endpoint(classes);
        let material = if wrong_secret { test_binding_material_with_wrong_secret() }
            else { test_binding_material_with_wrong_identity() };
        assert!(HostSession::authenticate_loopback(peer, material, AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }).is_err());
        let result = finish_endpoint(child, stdout);
        assert!(result.starts_with("session_failed,0,0,0,1,"), "unexpected rejected-handshake result: {result}");
    }

    #[test]
    fn java_receiver_and_rust_sender_share_authenticated_session_and_cleanup() {
        let classes = compile_java_endpoint();
        rejected_handshake(&classes, false);
        rejected_handshake(&classes, true);

        for send_bad_mac in [true, false] {
            let (child, stdout, peer) = spawn_endpoint(&classes);
            let mut host = HostSession::authenticate_loopback(peer, test_binding_material(), AudioParameters {
                sample_rate: 48_000, channels: 2, mode: 0,
            }).unwrap();
            if send_bad_mac { host.send_bad_mac_for_test().unwrap(); }
            else { host.send_old_session_for_test().unwrap(); }
            drop(host);
            let result = finish_endpoint(child, stdout);
            assert!(result.starts_with("session_failed,0,0,0,1,"), "unexpected invalid-control result: {result}");
        }

        let (child, stdout, peer) = spawn_endpoint(&classes);
        let mut host = HostSession::connect_loopback(peer, test_binding_material(), AudioParameters {
            sample_rate: 48_000, channels: 2, mode: 0,
        }).unwrap();
        assert_eq!(host.state(), ReceiverState::TransportConnected);
        host.start().unwrap();
        assert_eq!(host.state(), ReceiverState::ReceiverRunning);
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
        host.pause().unwrap();
        assert_eq!(host.state(), ReceiverState::Paused);
        host.resume().unwrap();
        assert_eq!(host.state(), ReceiverState::ReceiverRunning);
        host.stop().unwrap();
        assert_eq!(host.state(), ReceiverState::Stopped);
        drop(host);
        let result = finish_endpoint(child, stdout);
        let fields: Vec<_> = result.split(',').collect();
        assert_eq!(fields.len(), 7, "unexpected merged P5-C result: {result}");
        assert_eq!(fields[0], "");
        assert_eq!(fields[1..5], ["1", "1", "1", "1"]);
        assert!(fields[5].parse::<usize>().unwrap() >= 3_840, "synthetic PCM was not drained: {result}");
        assert_eq!(fields[6], "true", "STOP must release the receiver UDP socket");

        stop_disconnect_clears_active_local_session(&classes);
        start_handoff_failure_closes_session(&classes);
        let _ = fs::remove_dir_all(classes);
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
