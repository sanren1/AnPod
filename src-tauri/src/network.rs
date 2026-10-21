#[cfg(test)]
use crate::protocol::{PacketDecoder, ProtocolError, MAX_DATAGRAM_LEN};
use crate::protocol::{PacketEncoder, PacketType, PcmFormat, HOST_TO_RECEIVER, MAX_PAYLOAD_LEN};
#[cfg(test)]
use std::collections::VecDeque;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, TryLockError,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const PCM_QUEUE_CAPACITY: usize = 8;
const MAX_CAPTURE_FRAMES: usize = 4096;
const PCM_CHANNELS: u16 = 2;
const MAX_JITTER_PACKETS: usize = 4;
const BALANCED_CAPTURE_QUEUE_MS: u64 = 40;
const LOW_LATENCY_CAPTURE_QUEUE_MS: u64 = 20;
const UDP_SEND_ATTEMPTS: usize = 3;
#[cfg(test)]
static FAIL_NEXT_UDP_SEND: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) fn fail_next_udp_send_for_test() {
    FAIL_NEXT_UDP_SEND.store(true, Ordering::Release);
}

#[derive(Clone, Copy, Debug)]
struct PcmBlockInfo {
    frame_count: u32,
    sample_rate: u32,
    channels: u16,
    frame_index: u64,
    enqueued_at: Instant,
}

struct PcmSlot {
    samples: Box<[i16]>,
    info: PcmBlockInfo,
}

struct QueueState {
    slots: Vec<PcmSlot>,
    head: usize,
    len: usize,
    queued_frames: u64,
}

pub struct PcmQueue {
    state: Mutex<QueueState>,
    wake: Condvar,
    dropped_frames: AtomicU64,
    target_format: Option<PcmFormat>,
    max_queue_ms: u64,
    stopped: AtomicBool,
}

impl PcmQueue {
    pub fn new() -> Self {
        let samples_per_slot = MAX_CAPTURE_FRAMES * PCM_CHANNELS as usize;
        let slots = (0..PCM_QUEUE_CAPACITY)
            .map(|_| PcmSlot {
                samples: vec![0; samples_per_slot].into_boxed_slice(),
                info: PcmBlockInfo {
                    frame_count: 0,
                    sample_rate: 0,
                    channels: PCM_CHANNELS,
                    frame_index: 0,
                    enqueued_at: Instant::now(),
                },
            })
            .collect();
        Self {
            state: Mutex::new(QueueState {
                slots,
                head: 0,
                len: 0,
                queued_frames: 0,
            }),
            wake: Condvar::new(),
            dropped_frames: AtomicU64::new(0),
            target_format: None,
            max_queue_ms: BALANCED_CAPTURE_QUEUE_MS,
            stopped: AtomicBool::new(false),
        }
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    pub(crate) fn queued_frames(&self) -> u64 {
        self.state
            .lock()
            .expect("PCM queue lock poisoned")
            .queued_frames
    }

    pub(crate) fn for_session(format: PcmFormat, low_latency: bool) -> Self {
        let mut queue = Self::new();
        queue.target_format = Some(format);
        queue.max_queue_ms = if low_latency {
            LOW_LATENCY_CAPTURE_QUEUE_MS
        } else {
            BALANCED_CAPTURE_QUEUE_MS
        };
        queue
    }

    pub(crate) fn expected_format(&self) -> Option<PcmFormat> {
        self.target_format
    }

    pub(crate) fn stop_and_clear(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut state) = self.state.lock() {
            state.head = 0;
            state.len = 0;
            state.queued_frames = 0;
            for slot in &mut state.slots {
                slot.samples.fill(0);
            }
        }
        self.wake.notify_all();
    }

    pub(crate) fn push_raw(
        &self,
        samples: *const i16,
        frame_count: u32,
        sample_rate: u32,
        channels: u16,
        frame_index: u64,
    ) -> bool {
        self.push_raw_at(
            samples,
            frame_count,
            sample_rate,
            channels,
            frame_index,
            Instant::now(),
        )
    }

    fn push_raw_at(
        &self,
        samples: *const i16,
        frame_count: u32,
        sample_rate: u32,
        channels: u16,
        frame_index: u64,
        enqueued_at: Instant,
    ) -> bool {
        let frames = frame_count as usize;
        let sample_count = frames.checked_mul(channels as usize);
        if self.stopped.load(Ordering::Acquire)
            || samples.is_null()
            || sample_rate == 0
            || !matches!(channels, 1 | 2)
            || frames == 0
            || frames > MAX_CAPTURE_FRAMES
            || sample_count.is_none()
        {
            self.add_dropped(frame_count as u64);
            return false;
        }
        let sample_count = sample_count.unwrap();
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(_)) | Err(TryLockError::WouldBlock) => {
                self.add_dropped(frame_count as u64);
                return false;
            }
        };
        if self.stopped.load(Ordering::Acquire) {
            self.add_dropped(frame_count as u64);
            return false;
        }
        let target_channels = match self.target_format {
            Some(format) if format.sample_rate != sample_rate => {
                self.add_dropped(frame_count as u64);
                return false;
            }
            Some(format) => format.channels,
            None => channels,
        };
        let max_queued_frames = (sample_rate as u64 * self.max_queue_ms / 1000).max(1);
        let kept_frames = (frames as u64).min(max_queued_frames) as usize;
        let skipped_frames = frames - kept_frames;
        let Some(kept_frame_index) = frame_index.checked_add(skipped_frames as u64) else {
            self.add_dropped(frame_count as u64);
            return false;
        };
        self.add_dropped(skipped_frames as u64);
        while state.len > 0 {
            let oldest = state.slots[state.head].info;
            if oldest.enqueued_at.elapsed() <= Duration::from_millis(self.max_queue_ms)
                && state.len < PCM_QUEUE_CAPACITY
                && state.queued_frames + kept_frames as u64 <= max_queued_frames
            {
                break;
            }
            state.queued_frames -= oldest.frame_count as u64;
            self.add_dropped(oldest.frame_count as u64);
            state.head = (state.head + 1) % PCM_QUEUE_CAPACITY;
            state.len -= 1;
        }
        let index = (state.head + state.len) % PCM_QUEUE_CAPACITY;
        let slot = &mut state.slots[index];
        // The native pointer is borrowed only for this callback. Copy directly
        // into the slot allocated when the session queue was created.
        let source = unsafe { std::slice::from_raw_parts(samples, sample_count) };
        let source = &source[skipped_frames * channels as usize..];
        let kept_sample_count = kept_frames * target_channels as usize;
        match (channels, target_channels) {
            (1, 2) => {
                for (frame, mono) in source.iter().take(kept_frames).enumerate() {
                    slot.samples[frame * 2] = *mono;
                    slot.samples[frame * 2 + 1] = *mono;
                }
            }
            (2, 1) => {
                for frame in 0..kept_frames {
                    slot.samples[frame] =
                        ((source[frame * 2] as i32 + source[frame * 2 + 1] as i32) / 2) as i16;
                }
            }
            _ => slot.samples[..kept_sample_count].copy_from_slice(&source[..kept_sample_count]),
        }
        slot.info = PcmBlockInfo {
            frame_count: kept_frames as u32,
            sample_rate,
            channels: target_channels,
            frame_index: kept_frame_index,
            enqueued_at,
        };
        state.len += 1;
        state.queued_frames += kept_frames as u64;
        drop(state);
        self.wake.notify_one();
        true
    }

    fn add_dropped(&self, frames: u64) {
        self.dropped_frames.fetch_add(frames, Ordering::Relaxed);
    }

    fn pop_wait(&self, stopping: &AtomicBool, scratch: &mut Vec<i16>) -> Option<PcmBlockInfo> {
        let mut state = self.state.lock().ok()?;
        loop {
            while state.len == 0 && !stopping.load(Ordering::Acquire) {
                state = self
                    .wake
                    .wait_timeout(state, Duration::from_millis(20))
                    .ok()?
                    .0;
            }
            if stopping.load(Ordering::Acquire) || state.len == 0 {
                return None;
            }
            let index = state.head;
            let info = state.slots[index].info;
            state.head = (state.head + 1) % PCM_QUEUE_CAPACITY;
            state.len -= 1;
            state.queued_frames -= info.frame_count as u64;
            if info.enqueued_at.elapsed() > Duration::from_millis(self.max_queue_ms) {
                self.add_dropped(info.frame_count as u64);
                continue;
            }
            let sample_count = info.frame_count as usize * info.channels as usize;
            scratch.clear();
            scratch.extend_from_slice(&state.slots[index].samples[..sample_count]);
            return Some(info);
        }
    }

    fn wake_worker(&self) {
        self.wake.notify_all();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.state.lock().unwrap().len
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NetworkStats {
    pub packets_sent: u64,
    pub pcm_payload_bytes_sent: u64,
}

pub struct UdpSessionMaterial {
    session_id: u64,
    key: [u8; 32],
}

impl UdpSessionMaterial {
    pub(crate) fn from_authenticated(session_id: u64, key: &[u8; 32]) -> Self {
        Self {
            session_id,
            key: *key,
        }
    }

    pub fn generate() -> Result<Self, String> {
        let mut bytes = [0; 40];
        getrandom::fill(&mut bytes)
            .map_err(|_| "Unable to generate a fresh UDP session.".to_string())?;
        let session_id = u64::from_be_bytes(bytes[..8].try_into().expect("fixed session id bytes"));
        let mut key = [0; 32];
        key.copy_from_slice(&bytes[8..]);
        Ok(Self { session_id, key })
    }
}

impl Drop for UdpSessionMaterial {
    fn drop(&mut self) {
        self.key.fill(0);
    }
}

pub struct UdpSender {
    stopping: Arc<AtomicBool>,
    queue: Arc<PcmQueue>,
    worker: Option<JoinHandle<Result<NetworkStats, String>>>,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
}

impl UdpSender {
    pub fn spawn(
        remote: SocketAddr,
        queue: Arc<PcmQueue>,
        material: UdpSessionMaterial,
        format: PcmFormat,
    ) -> Result<Self, String> {
        if remote.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) {
            return Err("P3 unauthenticated UDP prototype only permits 127.0.0.1.".into());
        }
        let socket = UdpSocket::bind(("127.0.0.1", 0)).map_err(|error| error.to_string())?;
        Self::spawn_inner(
            remote,
            queue,
            socket,
            material,
            format,
            Arc::new(AtomicU64::new(0)),
            true,
        )
    }

    pub(crate) fn spawn_authenticated(
        remote: SocketAddr,
        queue: Arc<PcmQueue>,
        socket: UdpSocket,
        material: UdpSessionMaterial,
        parameters: crate::security::AudioParameters,
        pcm_payload_bytes_sent: Arc<AtomicU64>,
    ) -> Result<Self, String> {
        parameters
            .validate()
            .map_err(|_| "Unsupported authenticated PCM parameters.".to_string())?;
        let format = PcmFormat {
            sample_rate: parameters.sample_rate,
            channels: parameters.channels as u16,
        };
        Self::spawn_inner(
            remote,
            queue,
            socket,
            material,
            format,
            pcm_payload_bytes_sent,
            true,
        )
    }

    #[cfg(test)]
    pub(crate) fn spawn_authenticated_without_format_for_test(
        remote: SocketAddr,
        queue: Arc<PcmQueue>,
        socket: UdpSocket,
        material: UdpSessionMaterial,
        parameters: crate::security::AudioParameters,
        pcm_payload_bytes_sent: Arc<AtomicU64>,
    ) -> Result<Self, String> {
        parameters
            .validate()
            .map_err(|_| "Unsupported authenticated PCM parameters.".to_string())?;
        let format = PcmFormat {
            sample_rate: parameters.sample_rate,
            channels: parameters.channels as u16,
        };
        Self::spawn_inner(
            remote,
            queue,
            socket,
            material,
            format,
            pcm_payload_bytes_sent,
            false,
        )
    }

    fn spawn_inner(
        remote: SocketAddr,
        queue: Arc<PcmQueue>,
        socket: UdpSocket,
        material: UdpSessionMaterial,
        format: PcmFormat,
        pcm_payload_bytes_sent: Arc<AtomicU64>,
        send_format: bool,
    ) -> Result<Self, String> {
        if !matches!(format.channels, 1 | 2)
            || format.sample_rate == 0
            || !remote.is_ipv4()
            || !socket
                .local_addr()
                .map_err(|error| error.to_string())?
                .is_ipv4()
        {
            return Err("Unsupported PCM format or UDP source for the native sink.".into());
        }
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let worker_queue = queue.clone();
        let worker_bytes = pcm_payload_bytes_sent.clone();
        let worker = thread::Builder::new()
            .name("AnPod local UDP audio".into())
            .spawn(move || {
                send_loop(
                    socket,
                    remote,
                    worker_queue,
                    worker_stopping,
                    material,
                    format,
                    worker_bytes,
                    send_format,
                )
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stopping,
            queue,
            worker: Some(worker),
            pcm_payload_bytes_sent,
        })
    }

    pub fn pcm_payload_bytes_sent(&self) -> u64 {
        self.pcm_payload_bytes_sent.load(Ordering::Relaxed)
    }

    pub fn stop(mut self) -> Result<NetworkStats, String> {
        self.stop_worker()
    }

    fn stop_worker(&mut self) -> Result<NetworkStats, String> {
        self.stopping.store(true, Ordering::Release);
        self.queue.wake_worker();
        let Some(worker) = self.worker.take() else {
            return Ok(NetworkStats::default());
        };
        worker
            .join()
            .map_err(|_| "UDP worker thread panicked.".to_string())?
    }

    pub(crate) fn take_finished_error(&mut self) -> Option<String> {
        let worker = self.worker.as_ref()?;
        if !worker.is_finished() {
            return None;
        }
        let worker = self.worker.take().expect("finished UDP worker");
        match worker.join() {
            Ok(Ok(_)) => Some("UDP sender stopped unexpectedly.".into()),
            Ok(Err(error)) => Some(error),
            Err(_) => Some("UDP worker thread panicked.".into()),
        }
    }
}

impl Drop for UdpSender {
    fn drop(&mut self) {
        let _ = self.stop_worker();
    }
}

#[allow(clippy::too_many_arguments)]
fn send_loop(
    socket: UdpSocket,
    remote: SocketAddr,
    queue: Arc<PcmQueue>,
    stopping: Arc<AtomicBool>,
    material: UdpSessionMaterial,
    format: PcmFormat,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
    send_format: bool,
) -> Result<NetworkStats, String> {
    let mut encoder = PacketEncoder::with_sequence(
        &material.key,
        material.session_id,
        HOST_TO_RECEIVER,
        if send_format { 0 } else { 1 },
    );
    let format_payload = format.encode().map_err(|error| error.to_string())?;
    if send_format {
        send_packet(
            &socket,
            remote,
            &mut encoder,
            PacketType::Format,
            0,
            &format_payload,
        )?;
    }

    let mut stats = NetworkStats {
        packets_sent: if send_format { 1 } else { 0 },
        pcm_payload_bytes_sent: 0,
    };
    let mut samples = Vec::with_capacity(MAX_CAPTURE_FRAMES * PCM_CHANNELS as usize);
    let mut payload = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let sample_bytes_per_frame = format.channels as usize * 2;
    let byte_limited_frames = MAX_PAYLOAD_LEN / sample_bytes_per_frame;
    let five_ms_frames = (format.sample_rate as usize / 200).max(1);
    let frames_per_packet = five_ms_frames.min(byte_limited_frames);

    while let Some(info) = queue.pop_wait(&stopping, &mut samples) {
        if info.sample_rate != format.sample_rate || info.channels != format.channels {
            continue;
        }
        let frames = info.frame_count as usize;
        if samples.len() != frames * info.channels as usize {
            continue;
        }
        let mut offset_frames = 0;
        while offset_frames < frames {
            if info.enqueued_at.elapsed() > Duration::from_millis(queue.max_queue_ms) {
                queue.add_dropped((frames - offset_frames) as u64);
                break;
            }
            let packet_frames = frames_per_packet.min(frames - offset_frames);
            let sample_offset = offset_frames * info.channels as usize;
            let sample_end = sample_offset + packet_frames * info.channels as usize;
            payload.clear();
            for sample in &samples[sample_offset..sample_end] {
                payload.extend_from_slice(&sample.to_le_bytes());
            }
            if payload.len() > MAX_PAYLOAD_LEN {
                return Err("PCM packet exceeds the datagram limit.".into());
            }
            let frame_index = info
                .frame_index
                .checked_add(offset_frames as u64)
                .ok_or_else(|| "PCM sample frame index exhausted.".to_string())?;
            send_packet_with(
                &mut encoder,
                PacketType::Audio,
                frame_index,
                &payload,
                |packet| send_datagram(&socket, remote, packet),
            )?;
            stats.packets_sent = stats.packets_sent.saturating_add(1);
            stats.pcm_payload_bytes_sent = stats
                .pcm_payload_bytes_sent
                .saturating_add(payload.len() as u64);
            pcm_payload_bytes_sent.fetch_add(payload.len() as u64, Ordering::Relaxed);
            offset_frames += packet_frames;
        }
    }
    Ok(stats)
}

fn send_packet(
    socket: &UdpSocket,
    remote: SocketAddr,
    encoder: &mut PacketEncoder,
    packet_type: PacketType,
    frame_index: u64,
    payload: &[u8],
) -> Result<(), String> {
    send_packet_with(encoder, packet_type, frame_index, payload, |packet| {
        send_datagram(socket, remote, packet)
    })
}

fn send_datagram(socket: &UdpSocket, remote: SocketAddr, packet: &[u8]) -> std::io::Result<usize> {
    #[cfg(test)]
    if FAIL_NEXT_UDP_SEND.swap(false, Ordering::AcqRel) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "injected UDP sender failure",
        ));
    }
    socket.send_to(packet, remote)
}

fn send_packet_with(
    encoder: &mut PacketEncoder,
    packet_type: PacketType,
    frame_index: u64,
    payload: &[u8],
    mut send: impl FnMut(&[u8]) -> std::io::Result<usize>,
) -> Result<(), String> {
    for attempt in 0..UDP_SEND_ATTEMPTS {
        // Re-encode each attempt so an ambiguous send error never replays a GCM nonce/ciphertext.
        let packet = encoder
            .encode_next(packet_type, frame_index, payload)
            .map_err(|error| error.to_string())?;
        match send(&packet) {
            Ok(sent) if sent == packet.len() => return Ok(()),
            Ok(_) => return Err("UDP datagram was not sent atomically.".into()),
            Err(error)
                if attempt + 1 < UDP_SEND_ATTEMPTS
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("UDP send retry limit reached.".into())
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IngestResult {
    Format,
    Queued,
    Paused,
    Stale,
}

#[cfg(test)]
struct QueuedAudio {
    sequence: u64,
    frame_index: u64,
    frame_count: u64,
    payload: Vec<u8>,
}

#[cfg(test)]
struct MockReceiver {
    decoder: PacketDecoder,
    format: Option<PcmFormat>,
    queue: VecDeque<QueuedAudio>,
    playout_frame: u64,
    paused: bool,
    stale_drops: u64,
    paused_drops: u64,
}

#[cfg(test)]
impl MockReceiver {
    fn new(key: &[u8; 32], session_id: u64) -> Self {
        Self {
            decoder: PacketDecoder::new(key, session_id, HOST_TO_RECEIVER),
            format: None,
            queue: VecDeque::with_capacity(MAX_JITTER_PACKETS),
            playout_frame: 0,
            paused: false,
            stale_drops: 0,
            paused_drops: 0,
        }
    }

    fn receive(&mut self, socket: &UdpSocket) -> Result<IngestResult, String> {
        let mut datagram = [0; MAX_DATAGRAM_LEN + 1];
        let (length, _) = socket
            .recv_from(&mut datagram)
            .map_err(|error| error.to_string())?;
        self.ingest(&datagram[..length])
            .map_err(|error| error.to_string())
    }

    fn ingest(&mut self, datagram: &[u8]) -> Result<IngestResult, ProtocolError> {
        let decoded = self.decoder.open(datagram)?;
        match decoded.header.packet_type {
            PacketType::Format => {
                if self.format.is_some() || decoded.header.sample_frame_index != 0 {
                    return Err(ProtocolError::InvalidFormat);
                }
                let format = PcmFormat::decode(&decoded.payload)?;
                self.decoder.commit(decoded.header.sequence);
                self.format = Some(format);
                Ok(IngestResult::Format)
            }
            PacketType::Audio => {
                let format = self.format.ok_or(ProtocolError::InvalidFormat)?;
                let frame_bytes = format.channels as usize * 2;
                if frame_bytes == 0
                    || decoded.payload.is_empty()
                    || decoded.payload.len() % frame_bytes != 0
                {
                    return Err(ProtocolError::InvalidFormat);
                }
                let frame_count = (decoded.payload.len() / frame_bytes) as u64;
                let _end_frame = decoded
                    .header
                    .sample_frame_index
                    .checked_add(frame_count)
                    .ok_or(ProtocolError::InvalidFormat)?;
                self.decoder.commit(decoded.header.sequence);
                if self.paused {
                    self.paused_drops = self.paused_drops.saturating_add(1);
                    return Ok(IngestResult::Paused);
                }
                if decoded.header.sample_frame_index < self.playout_frame {
                    self.stale_drops = self.stale_drops.saturating_add(1);
                    return Ok(IngestResult::Stale);
                }
                if self.queue.len() == MAX_JITTER_PACKETS {
                    self.queue.pop_front();
                    self.stale_drops = self.stale_drops.saturating_add(1);
                }
                let packet = QueuedAudio {
                    sequence: decoded.header.sequence,
                    frame_index: decoded.header.sample_frame_index,
                    frame_count,
                    payload: decoded.payload,
                };
                let position = self
                    .queue
                    .iter()
                    .position(|queued| queued.frame_index > packet.frame_index)
                    .unwrap_or(self.queue.len());
                self.queue.insert(position, packet);
                Ok(IngestResult::Queued)
            }
        }
    }

    fn set_paused(&mut self, paused: bool, resume_frame: u64) {
        self.paused = paused;
        if !paused {
            self.playout_frame = resume_frame;
            let before = self.queue.len();
            self.queue
                .retain(|packet| packet.frame_index >= resume_frame);
            self.stale_drops = self
                .stale_drops
                .saturating_add((before - self.queue.len()) as u64);
        }
    }

    fn pop_audio(&mut self) -> Option<QueuedAudio> {
        let packet = self.queue.pop_front()?;
        self.playout_frame = packet.frame_index.saturating_add(packet.frame_count);
        Some(packet)
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) fn verify_capture_queue_bounds() {
    let format = PcmFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    let samples: Vec<i16> = (0..MAX_CAPTURE_FRAMES)
        .flat_map(|frame| [(frame + 10) as i16; 2])
        .collect();
    let queue = Arc::new(PcmQueue::for_session(format, false));
    assert!(queue.push_raw(samples.as_ptr(), MAX_CAPTURE_FRAMES as u32, 48_000, 2, 10));
    {
        let state = queue.state.lock().unwrap();
        let info = state.slots[state.head].info;
        let kept_frames = 48_000 * BALANCED_CAPTURE_QUEUE_MS as usize / 1000;
        assert_eq!(state.queued_frames, kept_frames as u64);
        assert_eq!(info.frame_count as usize, kept_frames);
        assert_eq!(
            info.frame_index,
            10 + MAX_CAPTURE_FRAMES as u64 - kept_frames as u64
        );
    }
    let material = UdpSessionMaterial::generate().unwrap();
    let key = material.key;
    let session_id = material.session_id;
    let receiver_socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    receiver_socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let sender = UdpSender::spawn(
        receiver_socket.local_addr().unwrap(),
        queue.clone(),
        material,
        format,
    )
    .unwrap();
    let mut decoder = PacketDecoder::new(&key, session_id, HOST_TO_RECEIVER);
    let mut datagram = [0; MAX_DATAGRAM_LEN + 1];
    let (length, _) = receiver_socket.recv_from(&mut datagram).unwrap();
    let decoded = decoder.open(&datagram[..length]).unwrap();
    assert_eq!(decoded.header.packet_type, PacketType::Format);
    decoder.commit(decoded.header.sequence);
    let kept_frames = 48_000 * BALANCED_CAPTURE_QUEUE_MS as usize / 1000;
    let first_frame = 10 + MAX_CAPTURE_FRAMES as u64 - kept_frames as u64;
    for index in 0..kept_frames / 240 {
        let (length, _) = receiver_socket.recv_from(&mut datagram).unwrap();
        let packet = decoder.open(&datagram[..length]).unwrap();
        assert_eq!(packet.header.packet_type, PacketType::Audio);
        assert_eq!(
            packet.header.sample_frame_index,
            first_frame + index as u64 * 240
        );
        assert_eq!(packet.payload.len(), 960);
        assert_eq!(
            i16::from_le_bytes([packet.payload[0], packet.payload[1]]),
            (first_frame + index as u64 * 240) as i16
        );
        decoder.commit(packet.header.sequence);
    }
    let stats = sender.stop().unwrap();
    assert_eq!(stats.packets_sent, 1 + (kept_frames / 240) as u64);
    assert_eq!(stats.pcm_payload_bytes_sent, (kept_frames * 4) as u64);
    assert_eq!(
        queue.dropped_frames(),
        (MAX_CAPTURE_FRAMES - kept_frames) as u64
    );

    let stale_queue = Arc::new(PcmQueue::for_session(format, false));
    assert!(stale_queue.push_raw_at(
        samples.as_ptr(),
        240,
        48_000,
        2,
        900,
        Instant::now() - Duration::from_millis(BALANCED_CAPTURE_QUEUE_MS + 10)
    ));
    let stale_material = UdpSessionMaterial::generate().unwrap();
    let stale_socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    stale_socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let stale_sender = UdpSender::spawn(
        stale_socket.local_addr().unwrap(),
        stale_queue.clone(),
        stale_material,
        format,
    )
    .unwrap();
    let mut received_format = [0u8; MAX_DATAGRAM_LEN + 1];
    let (length, _) = stale_socket.recv_from(&mut received_format).unwrap();
    assert_eq!(length, 57);
    let deadline = Instant::now() + Duration::from_secs(1);
    while stale_queue.dropped_frames() != 240 && Instant::now() < deadline {
        thread::yield_now();
    }
    assert_eq!(
        stale_queue.dropped_frames(),
        240,
        "expired capture audio was not dropped before UDP send"
    );
    let stale_stats = stale_sender.stop().unwrap();
    assert_eq!(stale_stats.packets_sent, 1);
    assert_eq!(stale_stats.pcm_payload_bytes_sent, 0);
    assert_eq!(stale_queue.dropped_frames(), 240);
}
