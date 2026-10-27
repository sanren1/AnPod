use crate::protocol::{
    AudioFormatV2, LossInterval, LossLedger, PacketEncoder, PacketType, PcmFormat,
    HOST_TO_RECEIVER, MAX_PAYLOAD_LEN,
};
#[cfg(test)]
use crate::protocol::{PacketDecoder, ProtocolError, MAX_DATAGRAM_LEN};
#[cfg(test)]
use std::collections::VecDeque;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Condvar, Mutex, TryLockError,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const PCM_QUEUE_CAPACITY: usize = 8;
const MAX_CAPTURE_FRAMES: usize = 4096;
const FLOAT_QUEUE_CHANNELS: u16 = 2;
const PCM_CHANNELS: u16 = 2;
const MAX_JITTER_PACKETS: usize = 4;
const BALANCED_CAPTURE_QUEUE_MS: u64 = 40;
const LOW_LATENCY_CAPTURE_QUEUE_MS: u64 = 20;
const UDP_SEND_ATTEMPTS: usize = 3;
const SOURCE_DROP_INTERVAL_CAPACITY: usize = 64;
#[cfg(test)]
static FAIL_NEXT_UDP_SEND: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) fn fail_next_udp_send_for_test() {
    FAIL_NEXT_UDP_SEND.store(true, Ordering::Release);
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PcmBlockInfo {
    pub(crate) frame_count: u32,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
    pub(crate) frame_index: u64,
    pub(crate) enqueued_at: Instant,
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
    drop_intervals: [SourceFrameInterval; SOURCE_DROP_INTERVAL_CAPACITY],
    drop_interval_count: usize,
    drop_tracking_complete: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceFrameInterval {
    pub(crate) start: u64,
    pub(crate) end: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceFrameSnapshot {
    pub(crate) period_epoch: u64,
    pub(crate) source_segment_generation: u64,
    pub(crate) source_segment_start: Option<u64>,
    pub(crate) intervals: [SourceFrameInterval; SOURCE_DROP_INTERVAL_CAPACITY],
    pub(crate) interval_count: usize,
    pub(crate) capture_tail: Option<u64>,
    pub(crate) settled_through: Option<u64>,
    pub(crate) complete: bool,
}
fn stable_source_watermarks(
    capture_tail: &AtomicU64,
    capture_tail_known: &AtomicBool,
    settled_through: &AtomicU64,
    settled_through_known: &AtomicBool,
) -> Option<(u64, u64)> {
    for _ in 0..3 {
        if !capture_tail_known.load(Ordering::Acquire)
            || !settled_through_known.load(Ordering::Acquire)
        {
            return None;
        }
        let tail_before = capture_tail.load(Ordering::Acquire);
        let settled = settled_through.load(Ordering::Acquire);
        let tail_after = capture_tail.load(Ordering::Acquire);
        if tail_before == tail_after && settled <= tail_before {
            return Some((tail_before, settled));
        }
    }
    None
}
#[derive(Default)]
pub(crate) struct LossLedgerTxState {
    snapshot: Option<SourceFrameSnapshot>,
    revision: u64,
    last_sent: Option<Instant>,
}

pub(crate) fn prepare_loss_ledger_payload(
    snapshot: SourceFrameSnapshot,
    acknowledged_revision: Option<u64>,
    tx: &mut LossLedgerTxState,
) -> Result<Option<Vec<u8>>, String> {
    if snapshot.capture_tail.is_none()
        || snapshot.settled_through.is_none()
        || snapshot.source_segment_start.is_none()
    {
        return Ok(None);
    }
    let changed = tx.snapshot != Some(snapshot);
    let segment_changed = tx.snapshot.is_none_or(|previous| {
        previous.source_segment_generation != snapshot.source_segment_generation
    });
    if changed {
        tx.revision = tx
            .revision
            .checked_add(1)
            .ok_or_else(|| "Source-frame loss ledger revision exhausted.".to_string())?;
        tx.snapshot = Some(snapshot);
    }
    if acknowledged_revision.is_some_and(|revision| revision >= tx.revision) {
        return Ok(None);
    }
    let now = Instant::now();
    let elapsed = tx
        .last_sent
        .map_or(Duration::MAX, |sent| now.saturating_duration_since(sent));
    if (!segment_changed && elapsed < Duration::from_millis(100))
        || (!changed && elapsed < Duration::from_secs(1))
    {
        return Ok(None);
    }
    let ledger = LossLedger {
        period_epoch: snapshot.period_epoch,
        revision: tx.revision,
        capture_tail: snapshot
            .capture_tail
            .expect("known capture tail checked above"),
        settled_through: snapshot
            .settled_through
            .expect("known settled cursor checked above"),
        source_segment_generation: snapshot.source_segment_generation,
        source_segment_start: snapshot.source_segment_start,
        complete: snapshot.complete,
        intervals: snapshot.intervals[..snapshot.interval_count]
            .iter()
            .map(|interval| LossInterval {
                start: interval.start,
                frame_count: interval.end - interval.start,
            })
            .collect(),
    };
    let payload = ledger.encode().map_err(|error| error.to_string())?;
    tx.last_sent = Some(now);
    Ok(Some(payload))
}

fn send_loss_ledger_if_due(
    socket: &UdpSocket,
    remote: SocketAddr,
    encoder: &mut PacketEncoder,
    snapshot: SourceFrameSnapshot,
    acknowledged_revision: Option<u64>,
    tx: &mut LossLedgerTxState,
) -> Result<(), String> {
    if let Some(payload) = prepare_loss_ledger_payload(snapshot, acknowledged_revision, tx)? {
        send_packet(socket, remote, encoder, PacketType::LossLedger, 0, &payload)?;
    }
    Ok(())
}

fn merge_source_drop_interval(
    intervals: &mut [SourceFrameInterval; SOURCE_DROP_INTERVAL_CAPACITY],
    interval_count: &mut usize,
    complete: &mut bool,
    start: u64,
    frame_count: u64,
) {
    let Some(end) = start.checked_add(frame_count) else {
        *complete = false;
        return;
    };
    if frame_count == 0 {
        return;
    }

    let mut merged = SourceFrameInterval { start, end };
    let mut first = 0;
    while first < *interval_count && intervals[first].end < merged.start {
        first += 1;
    }
    let mut last = first;
    while last < *interval_count && intervals[last].start <= merged.end {
        merged.start = merged.start.min(intervals[last].start);
        merged.end = merged.end.max(intervals[last].end);
        last += 1;
    }
    let removed = last - first;
    let next_count = *interval_count - removed + 1;
    if next_count > SOURCE_DROP_INTERVAL_CAPACITY {
        *complete = false;
        return;
    }
    if removed == 0 {
        for index in (first..*interval_count).rev() {
            intervals[index + 1] = intervals[index];
        }
    } else {
        for index in last..*interval_count {
            intervals[first + 1 + index - last] = intervals[index];
        }
    }
    intervals[first] = merged;
    *interval_count = next_count;
}

fn settled_after_drops(
    intervals: &[SourceFrameInterval; SOURCE_DROP_INTERVAL_CAPACITY],
    interval_count: usize,
    mut settled: u64,
) -> u64 {
    loop {
        let next = intervals[..interval_count]
            .iter()
            .find(|interval| interval.start <= settled && interval.end > settled)
            .map(|interval| interval.end);
        match next {
            Some(end) if end > settled => settled = end,
            _ => return settled,
        }
    }
}
struct SourceCaptureState<'a> {
    capture_tail: &'a AtomicU64,
    capture_tail_known: &'a AtomicBool,
    settled_through: &'a AtomicU64,
    settled_through_known: &'a AtomicBool,
    source_segment_active: &'a AtomicBool,
    source_segment_restart_pending: &'a AtomicBool,
    tracking_lost: &'a AtomicBool,
    source_frame_offset: &'a AtomicU64,
    local_capture_tail: &'a AtomicU64,
    local_capture_tail_known: &'a AtomicBool,
    source_segment_start: &'a AtomicU64,
    source_segment_start_known: &'a AtomicBool,
}

fn observe_source_capture(
    state: SourceCaptureState<'_>,
    frame_index: u64,
    frame_count: u32,
) -> u64 {
    let SourceCaptureState {
        capture_tail,
        capture_tail_known,
        settled_through,
        settled_through_known,
        source_segment_active,
        source_segment_restart_pending,
        tracking_lost,
        source_frame_offset,
        local_capture_tail,
        local_capture_tail_known,
        source_segment_start,
        source_segment_start_known,
    } = state;
    if frame_count == 0 {
        return frame_index.saturating_add(source_frame_offset.load(Ordering::Acquire));
    }
    let Some(local_end) = frame_index.checked_add(frame_count as u64) else {
        tracking_lost.store(true, Ordering::Release);
        return frame_index;
    };
    let active = source_segment_active.load(Ordering::Acquire);
    let restarting = active && source_segment_restart_pending.swap(false, Ordering::AcqRel);
    if restarting && local_capture_tail_known.load(Ordering::Acquire) {
        let previous_local_end = local_capture_tail.load(Ordering::Acquire);
        if frame_index < previous_local_end && capture_tail_known.load(Ordering::Acquire) {
            let previous_global_end = capture_tail.load(Ordering::Acquire);
            if let Some(offset) = previous_global_end.checked_sub(frame_index) {
                source_frame_offset.store(offset, Ordering::Release);
            } else {
                tracking_lost.store(true, Ordering::Release);
            }
        }
    }
    local_capture_tail.fetch_max(local_end, Ordering::AcqRel);
    local_capture_tail_known.store(true, Ordering::Release);
    let offset = source_frame_offset.load(Ordering::Acquire);
    let Some(global_start) = frame_index.checked_add(offset) else {
        tracking_lost.store(true, Ordering::Release);
        return frame_index;
    };
    let Some(global_end) = global_start.checked_add(frame_count as u64) else {
        tracking_lost.store(true, Ordering::Release);
        return frame_index;
    };
    if active && (restarting || !source_segment_start_known.load(Ordering::Acquire)) {
        source_segment_start.store(global_start, Ordering::Release);
        source_segment_start_known.store(true, Ordering::Release);
    }
    if capture_tail_known.load(Ordering::Acquire) {
        capture_tail.fetch_max(global_end, Ordering::AcqRel);
        if restarting {
            if settled_through_known.load(Ordering::Acquire) {
                let previous_settled = settled_through.load(Ordering::Acquire);
                if global_start < previous_settled {
                    tracking_lost.store(true, Ordering::Release);
                } else {
                    settled_through.store(global_start, Ordering::Release);
                }
            } else {
                settled_through.store(global_start, Ordering::Release);
                settled_through_known.store(true, Ordering::Release);
            }
        }
    } else {
        capture_tail.fetch_max(global_end, Ordering::AcqRel);
        if active {
            settled_through.fetch_min(global_start, Ordering::AcqRel);
            settled_through_known.store(true, Ordering::Release);
        }
        capture_tail_known.store(true, Ordering::Release);
    }
    global_start
}

pub struct PcmQueue {
    state: Mutex<QueueState>,
    wake: Condvar,
    dropped_frames: AtomicU64,
    capture_tail: AtomicU64,
    capture_tail_known: AtomicBool,
    source_tracking_lost: AtomicBool,
    source_epoch: AtomicU64,
    source_frame_offset: AtomicU64,
    local_capture_tail: AtomicU64,
    local_capture_tail_known: AtomicBool,
    settled_through: AtomicU64,
    settled_through_known: AtomicBool,
    source_segment_active: AtomicBool,
    source_segment_restart_pending: AtomicBool,
    source_segment_generation: AtomicU64,
    source_segment_start: AtomicU64,
    source_segment_start_known: AtomicBool,
    acknowledged_revision: AtomicU64,
    acknowledged_revision_known: AtomicBool,
    target_format: Option<PcmFormat>,
    max_queue_ms: u64,
    stopped: AtomicBool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FloatPcmBlockInfo {
    pub(crate) frame_count: u32,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
    pub(crate) frame_index: u64,
    pub(crate) enqueued_at: Instant,
}

struct FloatPcmSlot {
    samples: Box<[f32]>,
    info: FloatPcmBlockInfo,
}

struct FloatQueueState {
    slots: Vec<FloatPcmSlot>,
    head: usize,
    len: usize,
    queued_frames: u64,
    drop_intervals: [SourceFrameInterval; SOURCE_DROP_INTERVAL_CAPACITY],
    drop_interval_count: usize,
    drop_tracking_complete: bool,
}

/// Preallocated, bounded Float32 capture queue for the high-precision encoder.
/// It is deliberately independent of the existing PCM16 sender.
pub struct FloatPcmQueue {
    state: Mutex<FloatQueueState>,
    wake: Condvar,
    dropped_frames: AtomicU64,
    capture_tail: AtomicU64,
    capture_tail_known: AtomicBool,
    source_tracking_lost: AtomicBool,
    source_epoch: AtomicU64,
    source_frame_offset: AtomicU64,
    local_capture_tail: AtomicU64,
    local_capture_tail_known: AtomicBool,
    settled_through: AtomicU64,
    settled_through_known: AtomicBool,
    source_segment_active: AtomicBool,
    source_segment_restart_pending: AtomicBool,
    source_segment_generation: AtomicU64,
    source_segment_start: AtomicU64,
    source_segment_start_known: AtomicBool,
    acknowledged_revision: AtomicU64,
    acknowledged_revision_known: AtomicBool,
    sample_rate: AtomicU32,
    max_callback_frames: AtomicU32,
    max_queue_ms: u64,
    stopped: AtomicBool,
}

impl FloatPcmQueue {
    pub fn new(low_latency: bool) -> Self {
        let samples_per_slot = MAX_CAPTURE_FRAMES * FLOAT_QUEUE_CHANNELS as usize;
        let slots = (0..PCM_QUEUE_CAPACITY)
            .map(|_| FloatPcmSlot {
                samples: vec![0.0; samples_per_slot].into_boxed_slice(),
                info: FloatPcmBlockInfo {
                    frame_count: 0,
                    sample_rate: 0,
                    channels: FLOAT_QUEUE_CHANNELS,
                    frame_index: 0,
                    enqueued_at: Instant::now(),
                },
            })
            .collect();
        Self {
            state: Mutex::new(FloatQueueState {
                slots,
                head: 0,
                len: 0,
                queued_frames: 0,
                drop_intervals: [SourceFrameInterval::default(); SOURCE_DROP_INTERVAL_CAPACITY],
                drop_interval_count: 0,
                drop_tracking_complete: true,
            }),
            wake: Condvar::new(),
            dropped_frames: AtomicU64::new(0),
            capture_tail: AtomicU64::new(0),
            capture_tail_known: AtomicBool::new(false),
            source_tracking_lost: AtomicBool::new(false),
            source_epoch: AtomicU64::new(0),
            source_frame_offset: AtomicU64::new(0),
            local_capture_tail: AtomicU64::new(0),
            local_capture_tail_known: AtomicBool::new(false),
            settled_through: AtomicU64::new(u64::MAX),
            settled_through_known: AtomicBool::new(false),
            source_segment_active: AtomicBool::new(true),
            source_segment_restart_pending: AtomicBool::new(false),
            source_segment_generation: AtomicU64::new(0),
            source_segment_start: AtomicU64::new(0),
            source_segment_start_known: AtomicBool::new(false),
            acknowledged_revision: AtomicU64::new(0),
            acknowledged_revision_known: AtomicBool::new(false),
            sample_rate: AtomicU32::new(0),
            max_callback_frames: AtomicU32::new(0),
            max_queue_ms: if low_latency {
                LOW_LATENCY_CAPTURE_QUEUE_MS
            } else {
                BALANCED_CAPTURE_QUEUE_MS
            },
            stopped: AtomicBool::new(false),
        }
    }

    pub(crate) fn set_expected_format(
        &self,
        sample_rate: u32,
        max_callback_frames: u32,
    ) -> Result<(), String> {
        if sample_rate == 0
            || max_callback_frames == 0
            || max_callback_frames as usize > MAX_CAPTURE_FRAMES
        {
            return Err("Float32 采集格式或最大回调帧数无效。".into());
        }
        if self.stopped.load(Ordering::Acquire) {
            return Err("已停止的 Float32 PCM 队列不能重新启动，请创建新队列。".into());
        }
        let state = self
            .state
            .lock()
            .map_err(|_| "Float32 PCM 队列锁已损坏。")?;
        if state.len != 0 {
            return Err("Float32 PCM 队列开始采集后不能更改格式。".into());
        }
        let current = self.sample_rate.load(Ordering::Acquire);
        let current_max = self.max_callback_frames.load(Ordering::Acquire);
        if (current != 0 && current != sample_rate)
            || (current_max != 0 && current_max != max_callback_frames)
        {
            return Err("Float32 PCM 队列采样率与采集格式不一致。".into());
        }
        self.sample_rate.store(sample_rate, Ordering::Release);
        self.max_callback_frames
            .store(max_callback_frames, Ordering::Release);
        Ok(())
    }

    pub(crate) fn expected_format(&self) -> Option<(u32, u16)> {
        let sample_rate = self.sample_rate.load(Ordering::Acquire);
        (sample_rate != 0).then_some((sample_rate, FLOAT_QUEUE_CHANNELS))
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }
    pub(crate) fn end_source_segment(&self) {
        let Ok(_state) = self.state.lock() else {
            self.mark_source_tracking_lost();
            return;
        };
        self.source_segment_active.store(false, Ordering::Release);
        self.source_segment_restart_pending
            .store(false, Ordering::Release);
        self.source_segment_start_known
            .store(false, Ordering::Release);
        self.source_segment_generation
            .fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn begin_source_segment(&self) {
        let Ok(_state) = self.state.lock() else {
            self.mark_source_tracking_lost();
            return;
        };
        self.source_segment_restart_pending
            .store(true, Ordering::Release);
        self.source_segment_start_known
            .store(false, Ordering::Release);
        self.source_segment_active.store(true, Ordering::Release);
    }
    pub(crate) fn source_segment_generation(&self) -> u64 {
        self.source_segment_generation.load(Ordering::Acquire)
    }

    pub(crate) fn source_frame_snapshot(&self) -> SourceFrameSnapshot {
        match self.state.lock() {
            Ok(state) => {
                let watermarks = stable_source_watermarks(
                    &self.capture_tail,
                    &self.capture_tail_known,
                    &self.settled_through,
                    &self.settled_through_known,
                );
                SourceFrameSnapshot {
                    period_epoch: self.source_epoch.load(Ordering::Acquire),
                    source_segment_generation: self
                        .source_segment_generation
                        .load(Ordering::Acquire),
                    source_segment_start: self
                        .source_segment_start_known
                        .load(Ordering::Acquire)
                        .then(|| self.source_segment_start.load(Ordering::Acquire)),
                    intervals: state.drop_intervals,
                    interval_count: state.drop_interval_count,
                    capture_tail: watermarks.map(|(tail, _)| tail),
                    settled_through: watermarks.map(|(_, settled)| settled),
                    complete: watermarks.is_some()
                        && state.drop_tracking_complete
                        && !self.source_tracking_lost.load(Ordering::Acquire),
                }
            }
            Err(_) => SourceFrameSnapshot {
                period_epoch: self.source_epoch.load(Ordering::Acquire),
                source_segment_generation: self.source_segment_generation.load(Ordering::Acquire),
                source_segment_start: None,
                intervals: [SourceFrameInterval::default(); SOURCE_DROP_INTERVAL_CAPACITY],
                interval_count: 0,
                capture_tail: None,
                settled_through: None,
                complete: false,
            },
        }
    }
    pub(crate) fn queued_frames(&self) -> u64 {
        self.state
            .lock()
            .map(|state| state.queued_frames)
            .unwrap_or(0)
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    pub(crate) fn stop_and_clear(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut state) = self.state.lock() {
            state.head = 0;
            state.len = 0;
            state.queued_frames = 0;
            for slot in &mut state.slots {
                slot.samples.fill(0.0);
            }
        }
        self.wake.notify_all();
    }

    pub(crate) fn clear_queued(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.head = 0;
            state.len = 0;
            state.queued_frames = 0;
            for slot in &mut state.slots {
                slot.samples.fill(0.0);
            }
        }
        self.wake.notify_all();
    }

    pub(crate) fn queue_age_budget(&self) -> Duration {
        Duration::from_millis(self.max_queue_ms)
    }

    pub(crate) fn record_dropped_frames(&self, frame_index: Option<u64>, frames: u64) {
        if frames == 0 {
            return;
        }
        if let Some(frame_index) = frame_index {
            if let Some(end) = frame_index.checked_add(frames) {
                self.capture_tail.fetch_max(end, Ordering::AcqRel);
                self.capture_tail_known.store(true, Ordering::Release);
            } else {
                self.mark_source_tracking_lost();
            }
            self.add_dropped_at(frame_index, frames);
        } else {
            self.dropped_frames.fetch_add(frames, Ordering::Relaxed);
            if self.source_segment_active.load(Ordering::Acquire) {
                self.mark_source_tracking_lost();
            }
            self.wake.notify_one();
        }
    }

    pub(crate) fn push_raw(
        &self,
        samples: *const f32,
        frame_count: u32,
        sample_rate: u32,
        channels: u16,
        frame_index: u64,
    ) -> bool {
        let frames = frame_count as usize;
        let frame_index = if !self.stopped.load(Ordering::Acquire) {
            observe_source_capture(
                SourceCaptureState {
                    capture_tail: &self.capture_tail,
                    capture_tail_known: &self.capture_tail_known,
                    settled_through: &self.settled_through,
                    settled_through_known: &self.settled_through_known,
                    source_segment_active: &self.source_segment_active,
                    source_segment_restart_pending: &self.source_segment_restart_pending,
                    tracking_lost: &self.source_tracking_lost,
                    source_frame_offset: &self.source_frame_offset,
                    local_capture_tail: &self.local_capture_tail,
                    local_capture_tail_known: &self.local_capture_tail_known,
                    source_segment_start: &self.source_segment_start,
                    source_segment_start_known: &self.source_segment_start_known,
                },
                frame_index,
                frame_count,
            )
        } else {
            frame_index
        };
        if self.stopped.load(Ordering::Acquire)
            || !self.source_segment_active.load(Ordering::Acquire)
        {
            return false;
        }
        if samples.is_null()
            || sample_rate != self.sample_rate.load(Ordering::Acquire)
            || sample_rate == 0
            || channels != FLOAT_QUEUE_CHANNELS
            || frames == 0
            || frame_count > self.max_callback_frames.load(Ordering::Acquire)
            || frames > MAX_CAPTURE_FRAMES
        {
            self.add_dropped_at(frame_index, frame_count as u64);
            return false;
        }
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(_)) | Err(TryLockError::WouldBlock) => {
                self.add_dropped_at(frame_index, frame_count as u64);
                return false;
            }
        };
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        let max_queued_frames = (sample_rate as u64 * self.max_queue_ms / 1000).max(1);
        let kept_frames = (frames as u64).min(max_queued_frames) as usize;
        let skipped_frames = frames - kept_frames;
        let Some(kept_frame_index) = frame_index.checked_add(skipped_frames as u64) else {
            self.add_dropped_at(frame_index, frame_count as u64);
            return false;
        };
        self.add_dropped_locked(&mut state, frame_index, skipped_frames as u64);
        while state.len > 0 {
            let oldest = state.slots[state.head].info;
            if oldest.enqueued_at.elapsed() <= Duration::from_millis(self.max_queue_ms)
                && state.len < PCM_QUEUE_CAPACITY
                && state.queued_frames + kept_frames as u64 <= max_queued_frames
            {
                break;
            }
            state.queued_frames -= oldest.frame_count as u64;
            self.add_dropped_locked(&mut state, oldest.frame_index, oldest.frame_count as u64);
            state.head = (state.head + 1) % PCM_QUEUE_CAPACITY;
            state.len -= 1;
        }
        let index = (state.head + state.len) % PCM_QUEUE_CAPACITY;
        let slot = &mut state.slots[index];
        let sample_count = frames * FLOAT_QUEUE_CHANNELS as usize;
        let source = unsafe { std::slice::from_raw_parts(samples, sample_count) };
        let skipped_samples = skipped_frames * FLOAT_QUEUE_CHANNELS as usize;
        let kept_samples = kept_frames * FLOAT_QUEUE_CHANNELS as usize;
        slot.samples[..kept_samples].copy_from_slice(&source[skipped_samples..]);
        slot.info = FloatPcmBlockInfo {
            frame_count: kept_frames as u32,
            sample_rate,
            channels: FLOAT_QUEUE_CHANNELS,
            frame_index: kept_frame_index,
            enqueued_at: Instant::now(),
        };
        state.len += 1;
        state.queued_frames += kept_frames as u64;
        drop(state);
        self.wake.notify_one();
        true
    }

    fn add_dropped_at(&self, start: u64, frames: u64) {
        if frames == 0 {
            return;
        }
        if !self.source_segment_active.load(Ordering::Acquire) {
            self.dropped_frames.fetch_add(frames, Ordering::Relaxed);
            self.wake.notify_one();
            return;
        }
        match self.state.try_lock() {
            Ok(mut guard) => self.add_dropped_locked(&mut guard, start, frames),
            Err(TryLockError::Poisoned(_)) | Err(TryLockError::WouldBlock) => {
                self.dropped_frames.fetch_add(frames, Ordering::Relaxed);
                self.mark_source_tracking_lost();
            }
        }
        self.wake.notify_one();
    }

    fn add_dropped_locked(&self, state: &mut FloatQueueState, start: u64, frames: u64) {
        if frames == 0 {
            return;
        }
        self.dropped_frames.fetch_add(frames, Ordering::Relaxed);
        if !self.source_segment_active.load(Ordering::Acquire) {
            return;
        }
        merge_source_drop_interval(
            &mut state.drop_intervals,
            &mut state.drop_interval_count,
            &mut state.drop_tracking_complete,
            start,
            frames,
        );
        if self.settled_through_known.load(Ordering::Acquire) {
            let settled = self.settled_through.load(Ordering::Acquire);
            let advanced =
                settled_after_drops(&state.drop_intervals, state.drop_interval_count, settled);
            if advanced != settled {
                self.settled_through.store(advanced, Ordering::Release);
            }
        }
    }

    fn mark_source_tracking_lost(&self) {
        self.source_tracking_lost.store(true, Ordering::Release);
    }

    pub(crate) fn note_audio_sent(&self, generation: u64, start: u64, frames: u64) {
        if !self.source_segment_active.load(Ordering::Acquire)
            || self.source_segment_generation.load(Ordering::Acquire) != generation
        {
            return;
        }
        let Some(end) = start.checked_add(frames) else {
            self.mark_source_tracking_lost();
            return;
        };
        let Ok(state) = self.state.lock() else {
            self.mark_source_tracking_lost();
            return;
        };
        if !self.source_segment_active.load(Ordering::Acquire)
            || self.source_segment_generation.load(Ordering::Acquire) != generation
        {
            return;
        }
        let mut settled = self.settled_through.load(Ordering::Acquire);
        settled = settled_after_drops(&state.drop_intervals, state.drop_interval_count, settled);
        if !self.settled_through_known.load(Ordering::Acquire) || settled != start {
            self.mark_source_tracking_lost();
            return;
        }
        settled = settled_after_drops(&state.drop_intervals, state.drop_interval_count, end);
        self.settled_through.store(settled, Ordering::Release);
    }

    pub(crate) fn acknowledge_source_revision(&self, revision: u64) {
        self.acknowledged_revision
            .fetch_max(revision, Ordering::AcqRel);
        self.acknowledged_revision_known
            .store(true, Ordering::Release);
    }

    pub(crate) fn acknowledged_source_revision(&self) -> Option<u64> {
        self.acknowledged_revision_known
            .load(Ordering::Acquire)
            .then(|| self.acknowledged_revision.load(Ordering::Acquire))
    }
    /// Encoder worker consumes blocks into a reusable scratch buffer.
    pub(crate) fn pop_wait(&self, scratch: &mut Vec<f32>) -> Option<FloatPcmBlockInfo> {
        let mut state = self.state.lock().ok()?;
        loop {
            while state.len == 0 && !self.stopped.load(Ordering::Acquire) {
                state = self.wake.wait(state).ok()?;
            }
            if self.stopped.load(Ordering::Acquire) || state.len == 0 {
                return None;
            }
            let index = state.head;
            let info = state.slots[index].info;
            state.head = (state.head + 1) % PCM_QUEUE_CAPACITY;
            state.len -= 1;
            state.queued_frames -= info.frame_count as u64;
            if info.enqueued_at.elapsed() > Duration::from_millis(self.max_queue_ms) {
                self.add_dropped_locked(&mut state, info.frame_index, info.frame_count as u64);
                continue;
            }
            let sample_count = info.frame_count as usize * info.channels as usize;
            scratch.clear();
            scratch.extend_from_slice(&state.slots[index].samples[..sample_count]);
            return Some(info);
        }
    }

    pub(crate) fn pop_timeout(
        &self,
        scratch: &mut Vec<f32>,
        timeout: Duration,
    ) -> Option<FloatPcmBlockInfo> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().ok()?;
        loop {
            if self.stopped.load(Ordering::Acquire) {
                return None;
            }
            if state.len == 0 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return None;
                }
                let (next, result) = self.wake.wait_timeout(state, remaining).ok()?;
                state = next;
                if result.timed_out() && state.len == 0 {
                    return None;
                }
                continue;
            }
            let index = state.head;
            let info = state.slots[index].info;
            state.head = (state.head + 1) % PCM_QUEUE_CAPACITY;
            state.len -= 1;
            state.queued_frames -= info.frame_count as u64;
            if info.enqueued_at.elapsed() > Duration::from_millis(self.max_queue_ms) {
                self.add_dropped_locked(&mut state, info.frame_index, info.frame_count as u64);
                continue;
            }
            let sample_count = info.frame_count as usize * info.channels as usize;
            scratch.clear();
            scratch.extend_from_slice(&state.slots[index].samples[..sample_count]);
            return Some(info);
        }
    }
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
                drop_intervals: [SourceFrameInterval::default(); SOURCE_DROP_INTERVAL_CAPACITY],
                drop_interval_count: 0,
                drop_tracking_complete: true,
            }),
            wake: Condvar::new(),
            dropped_frames: AtomicU64::new(0),
            capture_tail: AtomicU64::new(0),
            capture_tail_known: AtomicBool::new(false),
            source_tracking_lost: AtomicBool::new(false),
            source_epoch: AtomicU64::new(0),
            source_frame_offset: AtomicU64::new(0),
            local_capture_tail: AtomicU64::new(0),
            local_capture_tail_known: AtomicBool::new(false),
            settled_through: AtomicU64::new(u64::MAX),
            settled_through_known: AtomicBool::new(false),
            source_segment_active: AtomicBool::new(true),
            source_segment_restart_pending: AtomicBool::new(false),
            source_segment_generation: AtomicU64::new(0),
            source_segment_start: AtomicU64::new(0),
            source_segment_start_known: AtomicBool::new(false),
            acknowledged_revision: AtomicU64::new(0),
            acknowledged_revision_known: AtomicBool::new(false),
            target_format: None,
            max_queue_ms: BALANCED_CAPTURE_QUEUE_MS,
            stopped: AtomicBool::new(false),
        }
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }
    pub(crate) fn end_source_segment(&self) {
        let Ok(_state) = self.state.lock() else {
            self.mark_source_tracking_lost();
            return;
        };
        self.source_segment_active.store(false, Ordering::Release);
        self.source_segment_restart_pending
            .store(false, Ordering::Release);
        self.source_segment_start_known
            .store(false, Ordering::Release);
        self.source_segment_generation
            .fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn begin_source_segment(&self) {
        let Ok(_state) = self.state.lock() else {
            self.mark_source_tracking_lost();
            return;
        };
        self.source_segment_restart_pending
            .store(true, Ordering::Release);
        self.source_segment_start_known
            .store(false, Ordering::Release);
        self.source_segment_active.store(true, Ordering::Release);
    }
    pub(crate) fn source_segment_generation(&self) -> u64 {
        self.source_segment_generation.load(Ordering::Acquire)
    }

    pub(crate) fn source_frame_snapshot(&self) -> SourceFrameSnapshot {
        match self.state.lock() {
            Ok(state) => {
                let watermarks = stable_source_watermarks(
                    &self.capture_tail,
                    &self.capture_tail_known,
                    &self.settled_through,
                    &self.settled_through_known,
                );
                SourceFrameSnapshot {
                    period_epoch: self.source_epoch.load(Ordering::Acquire),
                    source_segment_generation: self
                        .source_segment_generation
                        .load(Ordering::Acquire),
                    source_segment_start: self
                        .source_segment_start_known
                        .load(Ordering::Acquire)
                        .then(|| self.source_segment_start.load(Ordering::Acquire)),
                    intervals: state.drop_intervals,
                    interval_count: state.drop_interval_count,
                    capture_tail: watermarks.map(|(tail, _)| tail),
                    settled_through: watermarks.map(|(_, settled)| settled),
                    complete: watermarks.is_some()
                        && state.drop_tracking_complete
                        && !self.source_tracking_lost.load(Ordering::Acquire),
                }
            }
            Err(_) => SourceFrameSnapshot {
                period_epoch: self.source_epoch.load(Ordering::Acquire),
                source_segment_generation: self.source_segment_generation.load(Ordering::Acquire),
                source_segment_start: None,
                intervals: [SourceFrameInterval::default(); SOURCE_DROP_INTERVAL_CAPACITY],
                interval_count: 0,
                capture_tail: None,
                settled_through: None,
                complete: false,
            },
        }
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

    pub(crate) fn clear_queued(&self) {
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

    pub(crate) fn queue_age_budget(&self) -> Duration {
        Duration::from_millis(self.max_queue_ms)
    }

    pub(crate) fn record_dropped_frames(&self, frame_index: Option<u64>, frames: u64) {
        if frames == 0 {
            return;
        }
        if let Some(frame_index) = frame_index {
            if let Some(end) = frame_index.checked_add(frames) {
                self.capture_tail.fetch_max(end, Ordering::AcqRel);
                self.capture_tail_known.store(true, Ordering::Release);
            } else {
                self.mark_source_tracking_lost();
            }
            self.add_dropped_at(frame_index, frames);
        } else {
            self.dropped_frames.fetch_add(frames, Ordering::Relaxed);
            if self.source_segment_active.load(Ordering::Acquire) {
                self.mark_source_tracking_lost();
            }
            self.wake.notify_one();
        }
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
        let frame_index = if !self.stopped.load(Ordering::Acquire) {
            observe_source_capture(
                SourceCaptureState {
                    capture_tail: &self.capture_tail,
                    capture_tail_known: &self.capture_tail_known,
                    settled_through: &self.settled_through,
                    settled_through_known: &self.settled_through_known,
                    source_segment_active: &self.source_segment_active,
                    source_segment_restart_pending: &self.source_segment_restart_pending,
                    tracking_lost: &self.source_tracking_lost,
                    source_frame_offset: &self.source_frame_offset,
                    local_capture_tail: &self.local_capture_tail,
                    local_capture_tail_known: &self.local_capture_tail_known,
                    source_segment_start: &self.source_segment_start,
                    source_segment_start_known: &self.source_segment_start_known,
                },
                frame_index,
                frame_count,
            )
        } else {
            frame_index
        };
        let sample_count = frames.checked_mul(channels as usize);
        if self.stopped.load(Ordering::Acquire)
            || !self.source_segment_active.load(Ordering::Acquire)
        {
            return false;
        }
        if samples.is_null()
            || sample_rate == 0
            || !matches!(channels, 1 | 2)
            || frames == 0
            || frames > MAX_CAPTURE_FRAMES
            || sample_count.is_none()
        {
            self.add_dropped_at(frame_index, frame_count as u64);
            return false;
        }
        let sample_count = sample_count.unwrap();
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(_)) | Err(TryLockError::WouldBlock) => {
                self.add_dropped_at(frame_index, frame_count as u64);
                return false;
            }
        };
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        let target_channels = match self.target_format {
            Some(format) if format.sample_rate != sample_rate => {
                self.add_dropped_locked(&mut state, frame_index, frame_count as u64);
                return false;
            }
            Some(format) => format.channels,
            None => channels,
        };
        let max_queued_frames = (sample_rate as u64 * self.max_queue_ms / 1000).max(1);
        let kept_frames = (frames as u64).min(max_queued_frames) as usize;
        let skipped_frames = frames - kept_frames;
        let Some(kept_frame_index) = frame_index.checked_add(skipped_frames as u64) else {
            self.dropped_frames
                .fetch_add(frame_count as u64, Ordering::Relaxed);
            self.mark_source_tracking_lost();
            return false;
        };
        self.add_dropped_locked(&mut state, frame_index, skipped_frames as u64);
        while state.len > 0 {
            let oldest = state.slots[state.head].info;
            if oldest.enqueued_at.elapsed() <= Duration::from_millis(self.max_queue_ms)
                && state.len < PCM_QUEUE_CAPACITY
                && state.queued_frames + kept_frames as u64 <= max_queued_frames
            {
                break;
            }
            state.queued_frames -= oldest.frame_count as u64;
            self.add_dropped_locked(&mut state, oldest.frame_index, oldest.frame_count as u64);
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

    fn add_dropped_at(&self, start: u64, frames: u64) {
        if frames == 0 {
            return;
        }
        if !self.source_segment_active.load(Ordering::Acquire) {
            self.dropped_frames.fetch_add(frames, Ordering::Relaxed);
            self.wake.notify_one();
            return;
        }
        match self.state.try_lock() {
            Ok(mut guard) => self.add_dropped_locked(&mut guard, start, frames),
            Err(TryLockError::Poisoned(_)) | Err(TryLockError::WouldBlock) => {
                self.dropped_frames.fetch_add(frames, Ordering::Relaxed);
                self.mark_source_tracking_lost();
            }
        }
        self.wake.notify_one();
    }

    fn add_dropped_locked(&self, state: &mut QueueState, start: u64, frames: u64) {
        if frames == 0 {
            return;
        }
        self.dropped_frames.fetch_add(frames, Ordering::Relaxed);
        if !self.source_segment_active.load(Ordering::Acquire) {
            return;
        }
        merge_source_drop_interval(
            &mut state.drop_intervals,
            &mut state.drop_interval_count,
            &mut state.drop_tracking_complete,
            start,
            frames,
        );
        if self.settled_through_known.load(Ordering::Acquire) {
            let settled = self.settled_through.load(Ordering::Acquire);
            let advanced =
                settled_after_drops(&state.drop_intervals, state.drop_interval_count, settled);
            if advanced != settled {
                self.settled_through.store(advanced, Ordering::Release);
            }
        }
    }

    fn mark_source_tracking_lost(&self) {
        self.source_tracking_lost.store(true, Ordering::Release);
    }

    pub(crate) fn note_audio_sent(&self, generation: u64, start: u64, frames: u64) {
        if !self.source_segment_active.load(Ordering::Acquire)
            || self.source_segment_generation.load(Ordering::Acquire) != generation
        {
            return;
        }
        let Some(end) = start.checked_add(frames) else {
            self.mark_source_tracking_lost();
            return;
        };
        let Ok(state) = self.state.lock() else {
            self.mark_source_tracking_lost();
            return;
        };
        if !self.source_segment_active.load(Ordering::Acquire)
            || self.source_segment_generation.load(Ordering::Acquire) != generation
        {
            return;
        }
        let mut settled = self.settled_through.load(Ordering::Acquire);
        settled = settled_after_drops(&state.drop_intervals, state.drop_interval_count, settled);
        if !self.settled_through_known.load(Ordering::Acquire) || settled != start {
            self.mark_source_tracking_lost();
            return;
        }
        settled = settled_after_drops(&state.drop_intervals, state.drop_interval_count, end);
        self.settled_through.store(settled, Ordering::Release);
    }

    pub(crate) fn acknowledge_source_revision(&self, revision: u64) {
        self.acknowledged_revision
            .fetch_max(revision, Ordering::AcqRel);
        self.acknowledged_revision_known
            .store(true, Ordering::Release);
    }

    pub(crate) fn acknowledged_source_revision(&self) -> Option<u64> {
        self.acknowledged_revision_known
            .load(Ordering::Acquire)
            .then(|| self.acknowledged_revision.load(Ordering::Acquire))
    }
    pub(crate) fn pop_wait(
        &self,
        stopping: &AtomicBool,
        scratch: &mut Vec<i16>,
    ) -> Option<PcmBlockInfo> {
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
                self.add_dropped_locked(&mut state, info.frame_index, info.frame_count as u64);
                continue;
            }
            let sample_count = info.frame_count as usize * info.channels as usize;
            scratch.clear();
            scratch.extend_from_slice(&state.slots[index].samples[..sample_count]);
            return Some(info);
        }
    }

    pub(crate) fn pop_timeout(
        &self,
        scratch: &mut Vec<i16>,
        timeout: Duration,
    ) -> Option<PcmBlockInfo> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().ok()?;
        loop {
            if self.stopped.load(Ordering::Acquire) {
                return None;
            }
            if state.len == 0 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return None;
                }
                let (next, result) = self.wake.wait_timeout(state, remaining).ok()?;
                state = next;
                if result.timed_out() && state.len == 0 {
                    return None;
                }
                continue;
            }
            let index = state.head;
            let info = state.slots[index].info;
            state.head = (state.head + 1) % PCM_QUEUE_CAPACITY;
            state.len -= 1;
            state.queued_frames -= info.frame_count as u64;
            if info.enqueued_at.elapsed() > Duration::from_millis(self.max_queue_ms) {
                self.add_dropped_locked(&mut state, info.frame_index, info.frame_count as u64);
                continue;
            }
            let sample_count = info.frame_count as usize * info.channels as usize;
            scratch.clear();
            scratch.extend_from_slice(&state.slots[index].samples[..sample_count]);
            return Some(info);
        }
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
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

pub(crate) struct FloatUdpSender {
    stopping: Arc<AtomicBool>,
    expected_queue_stop: Arc<AtomicBool>,
    queue: Arc<FloatPcmQueue>,
    worker: Option<JoinHandle<Result<NetworkStats, String>>>,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
}

struct FloatSenderSignals {
    stopping: Arc<AtomicBool>,
    expected_queue_stop: Arc<AtomicBool>,
    pcm_payload_bytes_sent: Arc<AtomicU64>,
    send_loss_ledger: bool,
}

impl FloatUdpSender {
    pub(crate) fn spawn_authenticated(
        remote: SocketAddr,
        queue: Arc<FloatPcmQueue>,
        socket: UdpSocket,
        material: UdpSessionMaterial,
        format: AudioFormatV2,
        pcm_payload_bytes_sent: Arc<AtomicU64>,
        send_loss_ledger: bool,
    ) -> Result<Self, String> {
        format.encode().map_err(|error| error.to_string())?;
        if !matches!(format.profile, 2..=5)
            || AudioFormatV2::for_profile(format.profile, format.mode).ok() != Some(format)
            || format.channels != 2
            || !remote.is_ipv4()
            || !socket
                .local_addr()
                .map_err(|error| error.to_string())?
                .is_ipv4()
        {
            return Err(
                "Unsupported authenticated audio format or UDP source for the native sink.".into(),
            );
        }
        let stopping = Arc::new(AtomicBool::new(false));
        let expected_queue_stop = Arc::new(AtomicBool::new(false));
        let worker_expected_queue_stop = expected_queue_stop.clone();
        let worker_stopping = stopping.clone();
        let worker_queue = queue.clone();
        let worker_signals = FloatSenderSignals {
            stopping: worker_stopping,
            expected_queue_stop: worker_expected_queue_stop,
            pcm_payload_bytes_sent: pcm_payload_bytes_sent.clone(),
            send_loss_ledger,
        };
        let worker = thread::Builder::new()
            .name("AnPod local Float32 UDP audio".into())
            .spawn(move || {
                send_float_loop(
                    socket,
                    remote,
                    worker_queue,
                    worker_signals,
                    material,
                    format,
                )
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stopping,
            expected_queue_stop,
            queue,
            worker: Some(worker),
            pcm_payload_bytes_sent,
        })
    }

    pub(crate) fn pcm_payload_bytes_sent(&self) -> u64 {
        self.pcm_payload_bytes_sent.load(Ordering::Relaxed)
    }

    pub(crate) fn expect_queue_stop(&self) {
        self.expected_queue_stop.store(true, Ordering::Release);
    }

    pub(crate) fn resume_after_ack(&self) -> Result<(), String> {
        if self.queue.is_stopped() || self.worker.as_ref().is_none_or(JoinHandle::is_finished) {
            return Err(
                "Float32 sender cannot resume after its queue stopped; reconnect with a fresh queue."
                    .into(),
            );
        }
        self.expected_queue_stop.store(false, Ordering::Release);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn worker_finished(&self) -> bool {
        self.worker.as_ref().is_some_and(JoinHandle::is_finished)
    }

    #[cfg(test)]
    pub(crate) fn worker_joined(&self) -> bool {
        self.worker.is_none()
    }

    pub(crate) fn stop(mut self) -> Result<NetworkStats, String> {
        self.stop_worker()
    }

    pub(crate) fn take_finished_error(&mut self) -> Option<String> {
        let worker = self.worker.as_ref()?;
        if !worker.is_finished() {
            return None;
        }
        let worker = self.worker.take().expect("finished Float32 UDP worker");
        match worker.join() {
            Ok(Ok(_))
                if self.expected_queue_stop.load(Ordering::Acquire) && self.queue.is_stopped() =>
            {
                None
            }
            Ok(Ok(_)) => Some("Float32 UDP sender stopped unexpectedly.".into()),
            Ok(Err(error)) => Some(error),
            Err(_) => Some("Float32 UDP worker thread panicked.".into()),
        }
    }

    fn stop_worker(&mut self) -> Result<NetworkStats, String> {
        self.stopping.store(true, Ordering::Release);
        self.expected_queue_stop.store(true, Ordering::Release);
        self.queue.stop_and_clear();
        let Some(worker) = self.worker.take() else {
            return Ok(NetworkStats::default());
        };
        worker
            .join()
            .map_err(|_| "Float32 UDP worker thread panicked.".to_string())?
    }
}

impl Drop for FloatUdpSender {
    fn drop(&mut self) {
        let _ = self.stop_worker();
    }
}

pub(crate) fn float_to_pcm24(sample: f32) -> Result<i32, &'static str> {
    if !sample.is_finite() {
        return Err("Float32 PCM contains a non-finite sample.");
    }
    let scaled = (sample.clamp(-1.0, 1.0) * 8_388_608.0).round();
    Ok((scaled as i32).clamp(-8_388_608, 8_388_607))
}

pub(crate) fn float32_to_le_bytes(sample: f32) -> Result<[u8; 4], &'static str> {
    sample
        .is_finite()
        .then(|| sample.to_le_bytes())
        .ok_or("Float32 PCM contains a non-finite sample.")
}

fn send_float_loop(
    socket: UdpSocket,
    remote: SocketAddr,
    queue: Arc<FloatPcmQueue>,
    signals: FloatSenderSignals,
    material: UdpSessionMaterial,
    format: AudioFormatV2,
) -> Result<NetworkStats, String> {
    let FloatSenderSignals {
        stopping,
        expected_queue_stop,
        pcm_payload_bytes_sent,
        send_loss_ledger,
    } = signals;
    let mut encoder =
        PacketEncoder::new_v2(&material.key, material.session_id, HOST_TO_RECEIVER, format)
            .map_err(|error| error.to_string())?;
    let mut stats = NetworkStats::default();
    send_packet(
        &socket,
        remote,
        &mut encoder,
        PacketType::Format,
        0,
        &format.encode().map_err(|error| error.to_string())?,
    )?;
    stats.packets_sent = 1;
    let mut samples = Vec::with_capacity(MAX_CAPTURE_FRAMES * FLOAT_QUEUE_CHANNELS as usize);
    let mut payload = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let frames_per_packet = MAX_PAYLOAD_LEN / format.frame_bytes as usize;
    let mut ledger_tx = LossLedgerTxState::default();

    loop {
        let Some(info) = queue.pop_timeout(&mut samples, Duration::from_millis(20)) else {
            if stopping.load(Ordering::Acquire) || queue.is_stopped() {
                break;
            }
            if send_loss_ledger {
                send_loss_ledger_if_due(
                    &socket,
                    remote,
                    &mut encoder,
                    queue.source_frame_snapshot(),
                    queue.acknowledged_source_revision(),
                    &mut ledger_tx,
                )?;
            }
            continue;
        };
        if stopping.load(Ordering::Acquire) {
            break;
        }
        if info.sample_rate != format.sample_rate || info.channels != format.channels as u16 {
            return Err(
                "Float32 queue block does not match the authenticated audio format.".into(),
            );
        }
        let frames = info.frame_count as usize;
        if samples.len() != frames * info.channels as usize {
            return Err("Float32 queue block length does not match its frame count.".into());
        }
        let mut offset_frames = 0;
        while offset_frames < frames {
            if stopping.load(Ordering::Acquire) {
                break;
            }
            if info.enqueued_at.elapsed() > Duration::from_millis(queue.max_queue_ms) {
                let dropped_frames = (frames - offset_frames) as u64;
                if let Some(start) = info.frame_index.checked_add(offset_frames as u64) {
                    queue.record_dropped_frames(Some(start), dropped_frames);
                } else {
                    queue
                        .dropped_frames
                        .fetch_add(dropped_frames, Ordering::Relaxed);
                    queue.mark_source_tracking_lost();
                }
                break;
            }
            let packet_frames = frames_per_packet.min(frames - offset_frames);
            let sample_offset = offset_frames * info.channels as usize;
            let sample_end = sample_offset + packet_frames * info.channels as usize;
            payload.clear();
            for sample in &samples[sample_offset..sample_end] {
                if format.sample_format == 3 {
                    payload
                        .extend_from_slice(&float32_to_le_bytes(*sample).map_err(str::to_string)?);
                } else {
                    let quantized = float_to_pcm24(*sample).map_err(str::to_string)? as u32;
                    payload.extend_from_slice(&quantized.to_le_bytes()[..3]);
                }
            }
            let frame_index = info
                .frame_index
                .checked_add(offset_frames as u64)
                .ok_or_else(|| "PCM sample frame index exhausted.".to_string())?;
            let segment_generation = queue.source_segment_generation();
            if send_loss_ledger {
                send_loss_ledger_if_due(
                    &socket,
                    remote,
                    &mut encoder,
                    queue.source_frame_snapshot(),
                    queue.acknowledged_source_revision(),
                    &mut ledger_tx,
                )?;
            }
            send_packet_with(
                &mut encoder,
                PacketType::Audio,
                frame_index,
                &payload,
                |packet| send_datagram(&socket, remote, packet),
            )?;
            queue.note_audio_sent(segment_generation, frame_index, packet_frames as u64);
            if send_loss_ledger {
                send_loss_ledger_if_due(
                    &socket,
                    remote,
                    &mut encoder,
                    queue.source_frame_snapshot(),
                    queue.acknowledged_source_revision(),
                    &mut ledger_tx,
                )?;
            }
            stats.packets_sent = stats.packets_sent.saturating_add(1);
            stats.pcm_payload_bytes_sent = stats
                .pcm_payload_bytes_sent
                .saturating_add(payload.len() as u64);
            pcm_payload_bytes_sent.fetch_add(payload.len() as u64, Ordering::Relaxed);
            offset_frames += packet_frames;
        }
    }
    if expected_queue_stop.load(Ordering::Acquire) {
        Ok(stats)
    } else {
        Err("Float32 PCM queue stopped while the authenticated sender was active.".into())
    }
}

struct SenderSetup {
    send_format: bool,
    send_loss_ledger: bool,
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
            SenderSetup {
                send_format: true,
                send_loss_ledger: false,
            },
        )
    }

    pub(crate) fn spawn_authenticated(
        remote: SocketAddr,
        queue: Arc<PcmQueue>,
        socket: UdpSocket,
        material: UdpSessionMaterial,
        parameters: crate::security::AudioParameters,
        pcm_payload_bytes_sent: Arc<AtomicU64>,
        send_loss_ledger: bool,
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
            SenderSetup {
                send_format: true,
                send_loss_ledger,
            },
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
            SenderSetup {
                send_format: false,
                send_loss_ledger: false,
            },
        )
    }

    fn spawn_inner(
        remote: SocketAddr,
        queue: Arc<PcmQueue>,
        socket: UdpSocket,
        material: UdpSessionMaterial,
        format: PcmFormat,
        pcm_payload_bytes_sent: Arc<AtomicU64>,
        setup: SenderSetup,
    ) -> Result<Self, String> {
        let SenderSetup {
            send_format,
            send_loss_ledger,
        } = setup;
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
                    send_loss_ledger,
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
    send_loss_ledger: bool,
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
    let mut ledger_tx = LossLedgerTxState::default();

    loop {
        let Some(info) = queue.pop_timeout(&mut samples, Duration::from_millis(20)) else {
            if stopping.load(Ordering::Acquire) || queue.is_stopped() {
                break;
            }
            if send_loss_ledger {
                send_loss_ledger_if_due(
                    &socket,
                    remote,
                    &mut encoder,
                    queue.source_frame_snapshot(),
                    queue.acknowledged_source_revision(),
                    &mut ledger_tx,
                )?;
            }
            continue;
        };
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
                let dropped_frames = (frames - offset_frames) as u64;
                if let Some(start) = info.frame_index.checked_add(offset_frames as u64) {
                    queue.record_dropped_frames(Some(start), dropped_frames);
                } else {
                    queue
                        .dropped_frames
                        .fetch_add(dropped_frames, Ordering::Relaxed);
                    queue.mark_source_tracking_lost();
                }
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
            let segment_generation = queue.source_segment_generation();
            if send_loss_ledger {
                send_loss_ledger_if_due(
                    &socket,
                    remote,
                    &mut encoder,
                    queue.source_frame_snapshot(),
                    queue.acknowledged_source_revision(),
                    &mut ledger_tx,
                )?;
            }
            send_packet_with(
                &mut encoder,
                PacketType::Audio,
                frame_index,
                &payload,
                |packet| send_datagram(&socket, remote, packet),
            )?;
            queue.note_audio_sent(segment_generation, frame_index, packet_frames as u64);
            if send_loss_ledger {
                send_loss_ledger_if_due(
                    &socket,
                    remote,
                    &mut encoder,
                    queue.source_frame_snapshot(),
                    queue.acknowledged_source_revision(),
                    &mut ledger_tx,
                )?;
            }
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
            PacketType::LossLedger => Err(ProtocolError::InvalidFormat),
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
