use crate::{
    aoa::{Record, RecordType},
    aoa_winusb::{AoaCancel, AoaReader, AoaStream, AoaWriter},
    network::{float32_to_le_bytes, float_to_pcm24},
    p5c::{
        self, Ack, ReceiverCapabilities, ReceiverState, SessionAudioQueue, TerminalCause,
        AUDIO_PROTOCOL_MAJOR, BIND, CAPABILITIES_QUERY, CAP_AUDIO_FLOAT32, CAP_AUDIO_V2,
        CAP_BUFFER_METRICS, CAP_PLAYBACK_PROGRESS, CAP_PROGRESS_EXT, HEARTBEAT,
        HOST_TO_RECEIVER_CONTROL, MANAGEMENT_CONTROL_MAJOR, PAUSE, PLAYBACK_PROGRESS_QUERY,
        RECEIVER_TO_HOST_CONTROL, RESULT_OK, RESULT_USER_STOPPED, RESUME, SELECT_MODE, START, STOP,
    },
    protocol::{
        AudioFormatV2, PacketEncoder, PacketType, PcmFormat, HOST_TO_RECEIVER, MAX_PAYLOAD_LEN,
    },
    receiver::{
        PlaybackDiagnostics, StartupDiagnosticsCapture, StartupDiagnosticsRecorder,
        STARTUP_DIAGNOSTICS_INTERVAL,
    },
    security::{BindingMaterial, HostHandshakeV2, SessionKeys},
};

use crate::p5c::{CAP_AOA_SESSION_RESTART, CAP_TERMINAL_CAUSE};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const REQUIRED_RECORD_CAPABILITIES: u32 = 0x82;
const RECORD_DEADLINE: Duration = Duration::from_secs(3);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const CONTROL_POLL: Duration = Duration::from_millis(20);
const READER_MAILBOX_CAPACITY: usize = 4;
const OWNER_COMMAND_CAPACITY: usize = 4;

trait RecordReader: Send {
    fn read_record(&mut self) -> io::Result<Record>;
}
trait RecordWriter: Send {
    fn write_record(&mut self, record: &Record) -> io::Result<()>;
}
trait CancelIo: Send + Sync {
    fn cancel_io(&self);
}

impl RecordReader for AoaReader {
    fn read_record(&mut self) -> io::Result<Record> {
        self.read_anpu_record()
    }
}
impl RecordWriter for AoaWriter {
    fn write_record(&mut self, record: &Record) -> io::Result<()> {
        self.write_anpu_record(record)
    }
}
impl CancelIo for AoaCancel {
    fn cancel_io(&self) {
        self.cancel();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecordTransport {
    Aoa,
    AdbStream,
}

struct TcpCancel(TcpStream);
impl CancelIo for TcpCancel {
    fn cancel_io(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

struct TcpRecordReader(TcpStream);
impl RecordReader for TcpRecordReader {
    fn read_record(&mut self) -> io::Result<Record> {
        let deadline = Instant::now() + RECORD_DEADLINE;
        let mut header = [0; 8];
        tcp_read_exact_until(&mut self.0, &mut header, deadline)?;
        let length = u16::from_be_bytes([header[6], header[7]]) as usize;
        if &header[..4] != b"ANPU" || header[4] != 1 || !(1..=1200).contains(&length) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid ANPU header",
            ));
        }
        let mut bytes = Vec::with_capacity(8 + length);
        bytes.extend_from_slice(&header);
        bytes.resize(8 + length, 0);
        tcp_read_exact_until(&mut self.0, &mut bytes[8..], deadline)?;
        crate::aoa::decode_record(&bytes)
    }
}

struct TcpRecordWriter(TcpStream);
impl RecordWriter for TcpRecordWriter {
    fn write_record(&mut self, record: &Record) -> io::Result<()> {
        let bytes = crate::aoa::encode_record(record)?;
        tcp_write_all_until(&mut self.0, &bytes, Instant::now() + RECORD_DEADLINE)
    }
}

fn tcp_read_exact_until(
    stream: &mut TcpStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "record deadline expired",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        match stream.read(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream closed",
                ))
            }
            Ok(count) => {
                let (_, rest) = std::mem::take(&mut bytes).split_at_mut(count);
                bytes = rest;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn tcp_write_all_until(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "record deadline expired",
            ));
        }
        stream.set_write_timeout(Some(remaining))?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "stream closed")),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

enum OwnerCommand {
    Pause(SyncSender<Result<(), String>>),
    Resume(SyncSender<Result<(), String>>),
    Stop {
        restart: bool,
        reply: SyncSender<Result<(), String>>,
    },
    Abort,
}

pub(crate) struct AoaOwner {
    commands: SyncSender<OwnerCommand>,
    cancel: Arc<dyn CancelIo>,
    worker: Option<JoinHandle<Result<(), String>>>,
    audio_queue: SessionAudioQueue,
    snapshot: Arc<Mutex<AoaSnapshot>>,
    transport: RecordTransport,
}

#[derive(Clone, Debug)]
pub(crate) struct AoaSnapshot {
    pub(crate) state: ReceiverState,
    pub(crate) session_id: u64,
    pub(crate) capabilities: ReceiverCapabilities,
    pub(crate) format: AudioFormatV2,
    pub(crate) last_error: Option<String>,
    pub(crate) playback_progress: Option<PlaybackDiagnostics>,
    pub(crate) pcm_payload_bytes_sent: u64,
    pub(crate) terminal_cause: Option<TerminalCause>,
}

impl AoaOwner {
    pub(crate) fn connect(
        stream: AoaStream,
        material: BindingMaterial,
        format: AudioFormatV2,
        queue: SessionAudioQueue,
        diagnostics: Option<StartupDiagnosticsCapture>,
    ) -> Result<Self, String> {
        Self::connect_io_with_transport(
            stream.reader,
            stream.writer,
            stream.cancel,
            material,
            format,
            queue,
            RecordTransport::Aoa,
            diagnostics,
            "usb-direct",
        )
    }

    pub(crate) fn connect_adb_stream(
        stream: TcpStream,
        material: BindingMaterial,
        format: AudioFormatV2,
        queue: SessionAudioQueue,
        diagnostic_transport: &str,
        diagnostics: Option<StartupDiagnosticsCapture>,
    ) -> Result<Self, String> {
        stream
            .set_nonblocking(false)
            .map_err(|error| format!("Unable to configure the ADB record socket: {error}"))?;
        let cancel = TcpCancel(stream.try_clone().map_err(|e| e.to_string())?);
        let reader = TcpRecordReader(stream.try_clone().map_err(|e| e.to_string())?);
        let writer = TcpRecordWriter(stream);
        Self::connect_io_with_transport(
            reader,
            writer,
            cancel,
            material,
            format,
            queue,
            RecordTransport::AdbStream,
            diagnostics,
            diagnostic_transport,
        )
    }

    fn connect_io<R, W, C>(
        reader: R,
        writer: W,
        cancel: C,
        material: BindingMaterial,
        format: AudioFormatV2,
        queue: SessionAudioQueue,
    ) -> Result<Self, String>
    where
        R: RecordReader + 'static,
        W: RecordWriter + 'static,
        C: CancelIo + 'static,
    {
        Self::connect_io_with_transport(
            reader,
            writer,
            cancel,
            material,
            format,
            queue,
            RecordTransport::Aoa,
            None,
            "usb-direct",
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn connect_io_with_transport<R, W, C>(
        reader: R,
        mut writer: W,
        cancel: C,
        material: BindingMaterial,
        format: AudioFormatV2,
        queue: SessionAudioQueue,
        transport: RecordTransport,
        diagnostics: Option<StartupDiagnosticsCapture>,
        diagnostic_transport: &str,
    ) -> Result<Self, String>
    where
        R: RecordReader + 'static,
        W: RecordWriter + 'static,
        C: CancelIo + 'static,
    {
        format
            .encode()
            .map_err(|_| "Unsupported AOA audio descriptor.".to_string())?;
        validate_queue(&queue, format)?;
        let cancel: Arc<dyn CancelIo> = Arc::new(cancel);
        let read_cancel = cancel.clone();
        let (records_tx, records_rx) = mpsc::sync_channel(READER_MAILBOX_CAPACITY);
        let reader_thread = thread::Builder::new()
            .name("aoa-owner-reader".into())
            .spawn(move || read_records(reader, records_tx, read_cancel))
            .map_err(|e| e.to_string())?;

        let setup = (|| {
            let transport_material = match transport {
                RecordTransport::Aoa => material.derive_aoa_transport(),
                RecordTransport::AdbStream => material.derive_adb_stream(),
            };
            let (handshake, hello) = HostHandshakeV2::start(transport_material, format)
                .map_err(|e| format!("Authenticated record P5-A handshake could not start: {e}"))?;
            write_record(&mut writer, RecordType::Handshake, hello)?;
            let challenge = receive_record(&records_rx, cancel.as_ref(), RECORD_DEADLINE)?;
            require_record_type(&challenge, RecordType::Handshake)?;
            let (await_ready, finish) = handshake
                .accept_challenge(&challenge.body)
                .map_err(|e| format!("AOA Receiver identity or proof was rejected: {e}"))?;
            write_record(&mut writer, RecordType::Handshake, finish)?;
            let ready = receive_record(&records_rx, cancel.as_ref(), RECORD_DEADLINE)?;
            require_record_type(&ready, RecordType::Handshake)?;
            let authenticated = await_ready
                .accept_ready(&ready.body)
                .map_err(|e| format!("AOA P5-A READY proof was rejected: {e}"))?;
            let session_id = authenticated.session_id;
            let keys = authenticated.into_keys();
            let capabilities = exchange_capabilities(
                &mut writer,
                &records_rx,
                cancel.as_ref(),
                &keys,
                session_id,
                REQUIRED_RECORD_CAPABILITIES
                    | if transport == RecordTransport::AdbStream {
                        p5c::CAP_AUDIO_ADB_STREAM
                            | CAP_AUDIO_V2
                            | CAP_AUDIO_FLOAT32
                            | CAP_TERMINAL_CAUSE
                    } else {
                        0
                    },
            )?;
            let required_audio_capabilities = CAP_AUDIO_V2
                | if format.profile >= 4 {
                    CAP_AUDIO_FLOAT32
                } else {
                    0
                };
            let required_audio_capabilities = required_audio_capabilities
                | if transport == RecordTransport::AdbStream && format.profile >= 4 {
                    p5c::CAP_AUDIO_ADB_STREAM
                } else {
                    0
                };
            if !capabilities.compatible
                || capabilities.capability_bits & required_audio_capabilities
                    != required_audio_capabilities
            {
                return Err(if format.profile >= 4 {
                    "RECEIVER_UPDATE_REQUIRED: Receiver does not advertise authenticated Float32 audio for this transport."
                } else {
                    "AOA Receiver does not support authenticated ANPD v2 audio."
                }.into());
            }
            let mut command_sequence = 1;
            let mut ack_sequence = 1;
            let bind = exchange_command(
                &mut writer,
                &records_rx,
                cancel.as_ref(),
                &keys,
                session_id,
                &mut command_sequence,
                &mut ack_sequence,
                BIND,
                &[0, 0],
            )?;
            require_ack(&bind, ReceiverState::TransportConnected, "BIND")?;
            if bind.udp_port != 0 {
                return Err("AOA BIND ACK returned a nonzero UDP port.".into());
            }
            let selected = exchange_command(
                &mut writer,
                &records_rx,
                cancel.as_ref(),
                &keys,
                session_id,
                &mut command_sequence,
                &mut ack_sequence,
                SELECT_MODE,
                &[format.mode],
            )?;
            require_ack(&selected, ReceiverState::TransportConnected, "SELECT_MODE")?;
            let started = exchange_command(
                &mut writer,
                &records_rx,
                cancel.as_ref(),
                &keys,
                session_id,
                &mut command_sequence,
                &mut ack_sequence,
                START,
                &[],
            )?;
            require_ack(&started, ReceiverState::ReceiverRunning, "START")?;
            let mut encoder = PacketEncoder::new_v2(
                &keys.host_to_receiver_audio,
                session_id,
                HOST_TO_RECEIVER,
                format,
            )
            .map_err(|e| e.to_string())?;
            let descriptor = format.encode().map_err(|e| e.to_string())?;
            let format_packet = encoder
                .encode_next(PacketType::Format, 0, &descriptor)
                .map_err(|e| e.to_string())?;
            write_record(&mut writer, RecordType::Audio, format_packet)?;
            Ok((
                keys,
                encoder,
                session_id,
                command_sequence,
                ack_sequence,
                capabilities,
            ))
        })();

        let (keys, encoder, session_id, command_sequence, ack_sequence, capabilities) = match setup
        {
            Ok(setup) => setup,
            Err(error) => {
                cancel.cancel_io();
                let _ = reader_thread.join();
                return Err(error);
            }
        };
        let diagnostics = diagnostics
            .map(|capture| capture.recorder(diagnostic_transport, format.profile, format.mode));
        let (commands, commands_rx) = mpsc::sync_channel(OWNER_COMMAND_CAPACITY);
        let worker_cancel = cancel.clone();
        let worker_queue = queue.clone();
        let snapshot = Arc::new(Mutex::new(AoaSnapshot {
            state: ReceiverState::ReceiverRunning,
            session_id,
            capabilities,
            format,
            last_error: None,
            playback_progress: None,
            pcm_payload_bytes_sent: 0,
            terminal_cause: None,
        }));
        let worker_snapshot = snapshot.clone();
        let worker_bytes = Arc::new(AtomicU64::new(0));
        let worker_bytes_for_thread = worker_bytes.clone();
        let reader_slot = Arc::new(Mutex::new(Some(reader_thread)));
        let reader_slot_for_worker = reader_slot.clone();
        let worker = thread::Builder::new()
            .name("aoa-owner-control-audio".into())
            .spawn(move || {
                let reader_thread = reader_slot_for_worker
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                let Some(reader_thread) = reader_thread else {
                    worker_cancel.cancel_io();
                    return Err("AOA reader worker handle was already consumed.".into());
                };
                run_owner_worker(
                    writer,
                    records_rx,
                    reader_thread,
                    worker_cancel,
                    keys,
                    encoder,
                    session_id,
                    format,
                    worker_queue,
                    command_sequence,
                    ack_sequence,
                    commands_rx,
                    worker_snapshot,
                    worker_bytes_for_thread,
                    diagnostics,
                )
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                cancel.cancel_io();
                if let Some(reader_thread) = reader_slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    let _ = reader_thread.join();
                }
                return Err(error.to_string());
            }
        };
        Ok(Self {
            commands,
            cancel,
            worker: Some(worker),
            audio_queue: queue,
            snapshot,
            transport,
        })
    }

    pub(crate) fn pause(&self) -> Result<(), String> {
        self.request(OwnerCommand::Pause)
    }
    pub(crate) fn resume(&self) -> Result<(), String> {
        self.request(OwnerCommand::Resume)
    }
    pub(crate) fn stop(mut self) -> Result<(), String> {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished)
            && self.snapshot().terminal_cause.is_some()
        {
            return self.join_after_terminal();
        }
        let result = self.request(|reply| OwnerCommand::Stop {
            restart: false,
            reply,
        });
        if result.is_err() && self.snapshot().terminal_cause.is_some() {
            return self.join_after_terminal();
        }
        if result.is_err() {
            stop_queue(&self.audio_queue);
            self.cancel.cancel_io();
        }
        self.join_worker().and(result)
    }
    pub(crate) fn cancel_after_transport_failure(mut self) -> Result<(), String> {
        let snapshot = self.snapshot();
        if snapshot.state != ReceiverState::Stopped
            || snapshot.terminal_cause != Some(TerminalCause::TransportFailure)
        {
            return Err("ADB record owner has no confirmed transport failure.".into());
        }
        stop_queue(&self.audio_queue);
        self.cancel.cancel_io();
        match self
            .worker
            .take()
            .ok_or_else(|| "ADB record owner worker was already joined.".to_string())?
            .join()
        {
            Err(_) => Err("ADB record owner worker panicked.".into()),
            Ok(Err(error)) if error == "AOA reader worker panicked." => Err(error),
            Ok(_) => Ok(()),
        }
    }
    pub(crate) fn stop_for_restart(mut self) -> Result<(), String> {
        if self.transport != RecordTransport::Aoa {
            return Err("Session restart is only available for AOA Direct.".into());
        }
        let result = self.request(|reply| OwnerCommand::Stop {
            restart: true,
            reply,
        });
        if result.is_err() {
            stop_queue(&self.audio_queue);
            self.cancel.cancel_io();
        }
        self.join_worker().and(result)
    }

    pub(crate) fn supports_session_restart(&self) -> bool {
        self.snapshot().capabilities.capability_bits & CAP_AOA_SESSION_RESTART != 0
    }
    fn join_after_terminal(&mut self) -> Result<(), String> {
        self.worker
            .take()
            .ok_or_else(|| "AOA owner worker was already joined.".to_string())?
            .join()
            .map(|_| ())
            .map_err(|_| "AOA owner worker panicked.".to_string())
    }
    fn request(
        &self,
        make: impl FnOnce(SyncSender<Result<(), String>>) -> OwnerCommand,
    ) -> Result<(), String> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.commands
            .try_send(make(tx))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    "AOA session is busy processing another command.".to_string()
                }
                mpsc::TrySendError::Disconnected(_) => "AOA session owner has stopped.".to_string(),
            })?;
        match rx.recv_timeout(RECORD_DEADLINE + Duration::from_secs(1)) {
            Ok(result) => result,
            Err(_) => {
                stop_queue(&self.audio_queue);
                self.cancel.cancel_io();
                Err("AOA session command timed out; the session was cancelled.".into())
            }
        }
    }
    pub(crate) fn audio_queue(&self) -> SessionAudioQueue {
        self.audio_queue.clone()
    }
    pub(crate) fn snapshot(&self) -> AoaSnapshot {
        self.snapshot.lock().expect("AOA snapshot poisoned").clone()
    }
    fn join_worker(&mut self) -> Result<(), String> {
        self.worker
            .take()
            .ok_or_else(|| "AOA owner worker was already joined.".to_string())?
            .join()
            .map_err(|_| "AOA owner worker panicked.".to_string())?
    }
}

impl Drop for AoaOwner {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.commands.try_send(OwnerCommand::Abort);
            stop_queue(&self.audio_queue);
            self.cancel.cancel_io();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

fn validate_queue(queue: &SessionAudioQueue, format: AudioFormatV2) -> Result<(), String> {
    match (format.profile, queue) {
        (1, SessionAudioQueue::Pcm16(q))
            if AudioFormatV2::for_profile(1, format.mode).ok() == Some(format)
                && q.expected_format()
                    == Some(PcmFormat {
                        sample_rate: 48_000,
                        channels: 2,
                    })
                && !q.is_stopped() =>
        {
            Ok(())
        }
        (2..=5, SessionAudioQueue::Float32(q, locked))
            if *locked == format
                && q.expected_format() == Some((format.sample_rate, 2))
                && !q.is_stopped() =>
        {
            Ok(())
        }
        _ => Err("AOA source queue does not match the authenticated audio format.".into()),
    }
}

fn read_records<R: RecordReader>(
    mut reader: R,
    records: SyncSender<Result<Record, String>>,
    cancel: Arc<dyn CancelIo>,
) {
    loop {
        let record = reader
            .read_record()
            .map_err(|e| format!("AOA record read failed: {e}"));
        let failed = record.is_err();
        match records.try_send(record) {
            Ok(()) => (),
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                cancel.cancel_io();
                return;
            }
        }
        if failed {
            cancel.cancel_io();
            return;
        }
    }
}

fn receive_record(
    records: &Receiver<Result<Record, String>>,
    cancel: &dyn CancelIo,
    timeout: Duration,
) -> Result<Record, String> {
    match records.recv_timeout(timeout) {
        Ok(Ok(record)) => Ok(record),
        Ok(Err(error)) => Err(error),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            cancel.cancel_io();
            Err("AOA record deadline expired.".into())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("AOA reader worker exited unexpectedly.".into())
        }
    }
}

fn require_record_type(record: &Record, expected: RecordType) -> Result<(), String> {
    if record.kind == expected {
        Ok(())
    } else {
        Err(format!("Unexpected ANPU {:?} record.", record.kind))
    }
}

fn write_record<W: RecordWriter>(
    writer: &mut W,
    kind: RecordType,
    body: Vec<u8>,
) -> Result<(), String> {
    let record = Record::new(kind, body).map_err(|e| e.to_string())?;
    writer
        .write_record(&record)
        .map_err(|e| format!("AOA {:?} record write failed: {e}", kind))
}

fn exchange_capabilities<W: RecordWriter>(
    writer: &mut W,
    records: &Receiver<Result<Record, String>>,
    cancel: &dyn CancelIo,
    keys: &SessionKeys,
    session_id: u64,
    required_capabilities: u32,
) -> Result<ReceiverCapabilities, String> {
    let payload = p5c::encode_capabilities_query(
        MANAGEMENT_CONTROL_MAJOR,
        AUDIO_PROTOCOL_MAJOR,
        required_capabilities,
    );
    let query = p5c::encode_control(
        &keys.host_to_receiver_control,
        HOST_TO_RECEIVER_CONTROL,
        session_id,
        0,
        CAPABILITIES_QUERY,
        &payload,
    )?;
    write_record(writer, RecordType::Control, query)?;
    let response = receive_record(records, cancel, RECORD_DEADLINE)?;
    require_record_type(&response, RecordType::Control)?;
    p5c::verify_control_frame(
        &response.body,
        &keys.receiver_to_host_control,
        RECEIVER_TO_HOST_CONTROL,
        session_id,
    )?;
    p5c::decode_capabilities_info(&response.body, 0, 0)
}

#[allow(clippy::too_many_arguments)]
fn exchange_command<W: RecordWriter>(
    writer: &mut W,
    records: &Receiver<Result<Record, String>>,
    cancel: &dyn CancelIo,
    keys: &SessionKeys,
    session_id: u64,
    command_sequence: &mut u64,
    ack_sequence: &mut u64,
    kind: u8,
    payload: &[u8],
) -> Result<Ack, String> {
    let command = *command_sequence;
    let frame = p5c::encode_control(
        &keys.host_to_receiver_control,
        HOST_TO_RECEIVER_CONTROL,
        session_id,
        command,
        kind,
        payload,
    )?;
    write_record(writer, RecordType::Control, frame)?;
    let record = receive_record(records, cancel, RECORD_DEADLINE)?;
    require_record_type(&record, RecordType::Control)?;
    p5c::verify_control_frame(
        &record.body,
        &keys.receiver_to_host_control,
        RECEIVER_TO_HOST_CONTROL,
        session_id,
    )?;
    let progress_schema = if kind == PLAYBACK_PROGRESS_QUERY {
        payload
            .first()
            .copied()
            .filter(|schema| matches!(schema, 2 | 3))
            .unwrap_or(0)
    } else {
        0
    };
    let ack = p5c::decode_ack_with_format_schema(
        &record.body,
        *ack_sequence,
        kind,
        command,
        true,
        progress_schema,
    )?;
    *command_sequence = command
        .checked_add(1)
        .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
    *ack_sequence = ack_sequence
        .checked_add(1)
        .ok_or_else(|| "P5-C ACK sequence exhausted.".to_string())?;
    Ok(ack)
}

fn require_ack(ack: &Ack, state: ReceiverState, command: &str) -> Result<(), String> {
    if ack.result == RESULT_OK && ack.state == state {
        Ok(())
    } else {
        Err(format!("AOA P5-C {command} was rejected."))
    }
}

struct PendingBlock {
    frame_count: u32,
    sample_rate: u32,
    channels: u16,
    frame_index: u64,
    enqueued_at: Instant,
    age_budget: Duration,
    offset: usize,
}

#[allow(clippy::too_many_arguments)]
fn run_owner_worker<W: RecordWriter>(
    mut writer: W,
    records: Receiver<Result<Record, String>>,
    reader_thread: JoinHandle<()>,
    cancel: Arc<dyn CancelIo>,
    keys: SessionKeys,
    mut encoder: PacketEncoder,
    session_id: u64,
    format: AudioFormatV2,
    queue: SessionAudioQueue,
    command_sequence: u64,
    ack_sequence: u64,
    commands: Receiver<OwnerCommand>,
    snapshot: Arc<Mutex<AoaSnapshot>>,
    bytes_sent: Arc<AtomicU64>,
    diagnostics: Option<StartupDiagnosticsRecorder>,
) -> Result<(), String> {
    let result = run_owner_loop(
        &mut writer,
        &records,
        cancel.as_ref(),
        &keys,
        &mut encoder,
        session_id,
        format,
        &queue,
        command_sequence,
        ack_sequence,
        &commands,
        &snapshot,
        &bytes_sent,
        diagnostics,
    );
    if let Err(error) = &result {
        stop_queue(&queue);
        if let Ok(mut state) = snapshot.lock() {
            state.last_error = Some(error.clone());
            state.state = ReceiverState::Stopped;
            if state.capabilities.capability_bits & CAP_TERMINAL_CAUSE != 0 {
                state.terminal_cause = Some(TerminalCause::TransportFailure);
            }
        }
    }
    cancel.cancel_io();
    let joined = reader_thread
        .join()
        .map_err(|_| "AOA reader worker panicked.".to_string());
    drop(keys);
    match (result, joined) {
        (_, Err(error)) => Err(error),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_owner_loop<W: RecordWriter>(
    writer: &mut W,
    records: &Receiver<Result<Record, String>>,
    cancel: &dyn CancelIo,
    keys: &SessionKeys,
    encoder: &mut PacketEncoder,
    session_id: u64,
    format: AudioFormatV2,
    queue: &SessionAudioQueue,
    mut command_sequence: u64,
    mut ack_sequence: u64,
    commands: &Receiver<OwnerCommand>,
    snapshot: &Arc<Mutex<AoaSnapshot>>,
    bytes_sent: &AtomicU64,
    diagnostics: Option<StartupDiagnosticsRecorder>,
) -> Result<(), String> {
    let mut state = ReceiverState::ReceiverRunning;
    let mut next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
    let mut next_progress = Instant::now();
    let mut pcm16 = Vec::new();
    let mut float32 = Vec::new();
    let mut pending: Option<PendingBlock> = None;
    loop {
        match commands.try_recv() {
            Ok(OwnerCommand::Abort) | Err(TryRecvError::Disconnected) => {
                return Err("AOA session was cancelled.".into())
            }
            Ok(command) => {
                let resumed = matches!(&command, OwnerCommand::Resume(_));
                if process_command(
                    command,
                    writer,
                    records,
                    cancel,
                    keys,
                    session_id,
                    &mut command_sequence,
                    &mut ack_sequence,
                    &mut state,
                    queue,
                    &mut pending,
                    snapshot,
                )? {
                    return Ok(());
                }
                if resumed && startup_window_is_active(diagnostics.as_ref(), snapshot) {
                    next_progress = Instant::now();
                }
                continue;
            }
            Err(TryRecvError::Empty) => {}
        }

        if Instant::now() >= next_heartbeat {
            let ack = exchange_command(
                writer,
                records,
                cancel,
                keys,
                session_id,
                &mut command_sequence,
                &mut ack_sequence,
                HEARTBEAT,
                &[],
            )?;
            if ack.result == RESULT_USER_STOPPED
                && ack.state == ReceiverState::Stopped
                && snapshot
                    .lock()
                    .map_err(|_| "AOA snapshot poisoned.")?
                    .capabilities
                    .capability_bits
                    & CAP_TERMINAL_CAUSE
                    != 0
            {
                let mut view = snapshot.lock().map_err(|_| "AOA snapshot poisoned.")?;
                view.state = ReceiverState::Stopped;
                view.terminal_cause = Some(TerminalCause::ReceiverUserStop);
                stop_queue(queue);
                return Ok(());
            }
            if ack.result != RESULT_OK || ack.state == ReceiverState::Stopped {
                return Err("AOA Receiver rejected heartbeat or stopped the session.".into());
            }
            state = ack.state;
            snapshot.lock().map_err(|_| "AOA snapshot poisoned.")?.state = state;
            next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
        }

        let supports_progress = snapshot
            .lock()
            .map_err(|_| "AOA snapshot poisoned.")?
            .capabilities
            .capability_bits
            & CAP_PLAYBACK_PROGRESS
            != 0;
        if supports_progress && Instant::now() >= next_progress {
            let capability_bits = snapshot
                .lock()
                .map_err(|_| "AOA snapshot poisoned.")?
                .capabilities
                .capability_bits;
            let progress_schema = if capability_bits & CAP_BUFFER_METRICS != 0 {
                3
            } else if capability_bits & CAP_PROGRESS_EXT != 0 {
                2
            } else {
                0
            };
            let ack = exchange_command(
                writer,
                records,
                cancel,
                keys,
                session_id,
                &mut command_sequence,
                &mut ack_sequence,
                PLAYBACK_PROGRESS_QUERY,
                if progress_schema == 0 {
                    &[]
                } else if progress_schema == 2 {
                    &[2]
                } else {
                    &[3]
                },
            )?;
            if ack.result == RESULT_USER_STOPPED {
                if ack.state != ReceiverState::Stopped
                    || snapshot
                        .lock()
                        .map_err(|_| "AOA snapshot poisoned.")?
                        .capabilities
                        .capability_bits
                        & CAP_TERMINAL_CAUSE
                        == 0
                {
                    return Err(
                        "AOA Receiver returned an invalid authenticated terminal cause.".into(),
                    );
                }
                let mut view = snapshot.lock().map_err(|_| "AOA snapshot poisoned.")?;
                view.state = ReceiverState::Stopped;
                view.terminal_cause = Some(TerminalCause::ReceiverUserStop);
                stop_queue(queue);
                return Ok(());
            }
            if ack.result != RESULT_OK {
                return Err("AOA playback progress query was rejected.".into());
            }
            let playback = ack.playback_progress;
            let next_interval = if let (Some(recorder), Some(progress)) =
                (diagnostics.as_ref(), playback.as_ref())
            {
                let active = ack.state != ReceiverState::Paused && progress.output_epoch.is_some();
                if active {
                    let (pc_queued, pc_dropped) = match queue {
                        SessionAudioQueue::Pcm16(queue) => {
                            (Some(queue.queued_frames()), Some(queue.dropped_frames()))
                        }
                        SessionAudioQueue::Float32(queue, _) => {
                            (Some(queue.queued_frames()), Some(queue.dropped_frames()))
                        }
                    };
                    let now = Instant::now();
                    recorder.record(progress.clone(), pc_queued, pc_dropped, now);
                    if recorder.should_poll_fast(
                        progress.receiver_generation,
                        progress.output_epoch,
                        now,
                    ) {
                        STARTUP_DIAGNOSTICS_INTERVAL
                    } else {
                        Duration::from_secs(2)
                    }
                } else {
                    Duration::from_secs(2)
                }
            } else {
                Duration::from_secs(2)
            };
            let mut view = snapshot.lock().map_err(|_| "AOA snapshot poisoned.")?;
            view.state = ack.state;
            view.playback_progress = playback;
            next_progress = Instant::now() + next_interval;
        }

        match records.try_recv() {
            Ok(Ok(record)) => {
                return Err(format!(
                    "Unexpected unsolicited ANPU {:?} record.",
                    record.kind
                ))
            }
            Ok(Err(error)) => return Err(error),
            Err(TryRecvError::Disconnected) => {
                return Err("AOA reader worker exited unexpectedly.".into())
            }
            Err(TryRecvError::Empty) => (),
        }

        if state == ReceiverState::Paused {
            match commands.recv_timeout(CONTROL_POLL) {
                Ok(command) => {
                    let resumed = matches!(&command, OwnerCommand::Resume(_));
                    if process_command(
                        command,
                        writer,
                        records,
                        cancel,
                        keys,
                        session_id,
                        &mut command_sequence,
                        &mut ack_sequence,
                        &mut state,
                        queue,
                        &mut pending,
                        snapshot,
                    )? {
                        return Ok(());
                    }
                    if resumed && startup_window_is_active(diagnostics.as_ref(), snapshot) {
                        next_progress = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("AOA session was cancelled.".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
            continue;
        }

        if pending.is_none() {
            pending = match queue {
                SessionAudioQueue::Pcm16(q) => {
                    q.pop_timeout(&mut pcm16, CONTROL_POLL)
                        .map(|i| PendingBlock {
                            frame_count: i.frame_count,
                            sample_rate: i.sample_rate,
                            channels: i.channels,
                            frame_index: i.frame_index,
                            enqueued_at: i.enqueued_at,
                            age_budget: q.queue_age_budget(),
                            offset: 0,
                        })
                }
                SessionAudioQueue::Float32(q, _) => {
                    q.pop_timeout(&mut float32, CONTROL_POLL)
                        .map(|i| PendingBlock {
                            frame_count: i.frame_count,
                            sample_rate: i.sample_rate,
                            channels: i.channels,
                            frame_index: i.frame_index,
                            enqueued_at: i.enqueued_at,
                            age_budget: q.queue_age_budget(),
                            offset: 0,
                        })
                }
            };
        }
        if queue_is_stopped(queue) {
            return Err("AOA audio queue stopped while the session was active.".into());
        }
        let Some(block) = pending.as_mut() else {
            continue;
        };
        if block.enqueued_at.elapsed() > block.age_budget {
            record_dropped_frames(queue, (block.frame_count as usize - block.offset) as u64);
            pending = None;
            continue;
        }
        if block.sample_rate != format.sample_rate || block.channels != format.channels as u16 {
            return Err("AOA queue block does not match the authenticated format.".into());
        }
        let frame_bytes = format.frame_bytes as usize;
        let max_frames = MAX_PAYLOAD_LEN / frame_bytes;
        let frames = (block.frame_count as usize - block.offset).min(max_frames);
        if frames == 0 {
            pending = None;
            continue;
        }
        let frame_index = block
            .frame_index
            .checked_add(block.offset as u64)
            .ok_or_else(|| "AOA PCM frame index exhausted.".to_string())?;
        let sample_start = block.offset * format.channels as usize;
        let sample_end = sample_start + frames * format.channels as usize;
        let mut payload = Vec::with_capacity(frames * frame_bytes);
        match (&queue, format.profile) {
            (SessionAudioQueue::Pcm16(_), 1) => {
                for sample in &pcm16[sample_start..sample_end] {
                    payload.extend_from_slice(&sample.to_le_bytes());
                }
            }
            (SessionAudioQueue::Float32(_, _), 2 | 3) => {
                for sample in &float32[sample_start..sample_end] {
                    let q23 = float_to_pcm24(*sample).map_err(str::to_string)? as u32;
                    payload.extend_from_slice(&q23.to_le_bytes()[..3]);
                }
            }
            (SessionAudioQueue::Float32(_, _), 4 | 5) => {
                for sample in &float32[sample_start..sample_end] {
                    payload
                        .extend_from_slice(&float32_to_le_bytes(*sample).map_err(str::to_string)?);
                }
            }
            _ => return Err("AOA queue type does not match the authenticated profile.".into()),
        }
        let packet = encoder
            .encode_next(PacketType::Audio, frame_index, &payload)
            .map_err(|e| e.to_string())?;
        write_record(writer, RecordType::Audio, packet)?;
        bytes_sent.fetch_add(payload.len() as u64, Ordering::Relaxed);
        snapshot
            .lock()
            .map_err(|_| "AOA snapshot poisoned.")?
            .pcm_payload_bytes_sent = bytes_sent.load(Ordering::Relaxed);
        block.offset += frames;
        if block.offset == block.frame_count as usize {
            pending = None;
        }
    }
}

fn startup_window_is_active(
    diagnostics: Option<&StartupDiagnosticsRecorder>,
    snapshot: &Mutex<AoaSnapshot>,
) -> bool {
    diagnostics.is_some_and(|recorder| {
        let snapshot = snapshot.lock().expect("AOA snapshot lock");
        let progress = snapshot.playback_progress.as_ref();
        recorder.should_poll_fast(
            progress.and_then(|progress| progress.receiver_generation),
            progress.and_then(|progress| progress.output_epoch),
            Instant::now(),
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn process_command<W: RecordWriter>(
    command: OwnerCommand,
    writer: &mut W,
    records: &Receiver<Result<Record, String>>,
    cancel: &dyn CancelIo,
    keys: &SessionKeys,
    session_id: u64,
    command_sequence: &mut u64,
    ack_sequence: &mut u64,
    state: &mut ReceiverState,
    queue: &SessionAudioQueue,
    pending: &mut Option<PendingBlock>,
    snapshot: &Arc<Mutex<AoaSnapshot>>,
) -> Result<bool, String> {
    let (kind, expected, response, restart) = match command {
        OwnerCommand::Pause(tx) => (PAUSE, ReceiverState::Paused, Some(tx), false),
        OwnerCommand::Resume(tx) => {
            if queue_is_stopped(queue) {
                let _ = tx.send(Err("A stopped AOA queue requires a fresh session.".into()));
                return Ok(false);
            }
            clear_queue(queue);
            (RESUME, ReceiverState::ReceiverRunning, Some(tx), false)
        }
        OwnerCommand::Stop { restart, reply } => {
            if restart
                && snapshot
                    .lock()
                    .map_err(|_| "AOA snapshot poisoned.")?
                    .capabilities
                    .capability_bits
                    & CAP_AOA_SESSION_RESTART
                    == 0
            {
                let _ = reply.send(Err("AOA Receiver does not support session restart.".into()));
                return Ok(false);
            }
            stop_queue(queue);
            *pending = None;
            (STOP, ReceiverState::Stopped, Some(reply), restart)
        }
        OwnerCommand::Abort => return Ok(true),
    };
    let valid_order = match kind {
        PAUSE => *state == ReceiverState::ReceiverRunning || *state == ReceiverState::Streaming,
        RESUME => *state == ReceiverState::Paused,
        STOP => *state != ReceiverState::Stopped,
        _ => false,
    };
    if !valid_order {
        if let Some(tx) = response.as_ref() {
            let _ = tx.send(Err("AOA P5-C command is out of order.".into()));
        }
        return Ok(false);
    }
    if kind == PAUSE {
        *pending = None;
        clear_queue(queue);
    }
    let mut terminal_stop = false;
    let result = exchange_command(
        writer,
        records,
        cancel,
        keys,
        session_id,
        command_sequence,
        ack_sequence,
        kind,
        if restart { &[1] } else { &[] },
    )
    .and_then(|ack| {
        if ack.result == RESULT_USER_STOPPED {
            if ack.state != ReceiverState::Stopped
                || snapshot
                    .lock()
                    .map_err(|_| "AOA snapshot poisoned.")?
                    .capabilities
                    .capability_bits
                    & CAP_TERMINAL_CAUSE
                    == 0
            {
                return Err(
                    "AOA Receiver returned an invalid authenticated terminal cause.".into(),
                );
            }
            terminal_stop = true;
            *state = ReceiverState::Stopped;
            let mut view = snapshot.lock().map_err(|_| "AOA snapshot poisoned.")?;
            view.state = ReceiverState::Stopped;
            view.terminal_cause = Some(TerminalCause::ReceiverUserStop);
            return Ok(());
        }
        require_ack(
            &ack,
            expected,
            match kind {
                PAUSE => "PAUSE",
                RESUME => "RESUME",
                _ => "STOP",
            },
        )?;
        *state = ack.state;
        let mut view = snapshot.lock().map_err(|_| "AOA snapshot poisoned.")?;
        view.state = ack.state;
        if kind == STOP {
            view.terminal_cause = Some(TerminalCause::HostStop);
        }
        Ok(())
    });
    if terminal_stop {
        stop_queue(queue);
        if let Some(tx) = response {
            let _ = tx.send(Err("AOA Receiver stopped the session.".into()));
        }
        return Ok(true);
    }
    let stopping = kind == STOP;
    if let Some(tx) = response {
        let _ = tx.send(result.clone());
    }
    result?;
    Ok(stopping)
}

fn queue_is_stopped(queue: &SessionAudioQueue) -> bool {
    match queue {
        SessionAudioQueue::Pcm16(q) => q.is_stopped(),
        SessionAudioQueue::Float32(q, _) => q.is_stopped(),
    }
}
fn stop_queue(queue: &SessionAudioQueue) {
    match queue {
        SessionAudioQueue::Pcm16(q) => q.stop_and_clear(),
        SessionAudioQueue::Float32(q, _) => q.stop_and_clear(),
    }
}
fn clear_queue(queue: &SessionAudioQueue) {
    match queue {
        SessionAudioQueue::Pcm16(q) => q.clear_queued(),
        SessionAudioQueue::Float32(q, _) => q.clear_queued(),
    }
}

fn record_dropped_frames(queue: &SessionAudioQueue, frames: u64) {
    match queue {
        SessionAudioQueue::Pcm16(q) => q.record_dropped_frames(frames),
        SessionAudioQueue::Float32(q, _) => q.record_dropped_frames(frames),
    }
}

#[cfg(test)]
#[path = "aoa_owner_tests.rs"]
mod tests;
