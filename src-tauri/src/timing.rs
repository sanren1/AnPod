//! Lock-free latency histograms for the PC capture → queue → send path.
//! Each record is one relaxed atomic add; quantiles are read as deltas by
//! the diagnostics sampler.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Four buckets per doubling of microseconds (≤ ~19% bucket width), up to
/// about 18 minutes; longer values land in the last bucket.
const BUCKETS: usize = 120;

fn bucket_index(micros: u64) -> usize {
    if micros < 4 {
        return micros as usize;
    }
    let msb = 63 - micros.leading_zeros() as usize;
    let sub = ((micros >> (msb - 2)) & 3) as usize;
    ((msb - 1) * 4 + sub).min(BUCKETS - 1)
}

/// Largest microsecond value that maps to `index`.
fn bucket_upper(index: usize) -> u64 {
    if index < 4 {
        return index as u64;
    }
    let msb = index / 4 + 1;
    let sub = (index % 4) as u64;
    ((4 + sub) << (msb - 2)) + (1u64 << (msb - 2)) - 1
}

pub(crate) struct LatencyHistogram {
    buckets: [AtomicU64; BUCKETS],
}

impl LatencyHistogram {
    fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    pub(crate) fn record(&self, duration: Duration) {
        let micros = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        self.buckets[bucket_index(micros)].fetch_add(1, Ordering::Relaxed);
    }

    /// Counts recorded since the previous take.
    fn take(&self) -> [u64; BUCKETS] {
        std::array::from_fn(|index| self.buckets[index].swap(0, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingSummary {
    pub count: u64,
    /// Bucket upper bounds in microseconds.
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

fn summarize(counts: &[u64; BUCKETS]) -> Option<TimingSummary> {
    let count: u64 = counts.iter().sum();
    if count == 0 {
        return None;
    }
    let quantile = |numerator: u64| {
        let rank = (count * numerator).div_ceil(100).max(1);
        let mut seen = 0;
        for (index, bucket) in counts.iter().enumerate() {
            seen += bucket;
            if seen >= rank {
                return bucket_upper(index);
            }
        }
        bucket_upper(BUCKETS - 1)
    };
    let max_index = counts.iter().rposition(|bucket| *bucket > 0).unwrap_or(0);
    Some(TimingSummary {
        count,
        p50_us: quantile(50),
        p95_us: quantile(95),
        p99_us: quantile(99),
        max_us: bucket_upper(max_index),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSendTimingSummary {
    pub capture_interval: Option<TimingSummary>,
    pub queue_wait: Option<TimingSummary>,
    pub send: Option<TimingSummary>,
}

pub(crate) struct CaptureSendTiming {
    base: Instant,
    /// Nanoseconds after `base` of the last capture push, plus one; 0 = none.
    last_capture: AtomicU64,
    capture_interval: LatencyHistogram,
    queue_wait: LatencyHistogram,
    send: LatencyHistogram,
}

impl CaptureSendTiming {
    pub(crate) fn new() -> Self {
        Self {
            base: Instant::now(),
            last_capture: AtomicU64::new(0),
            capture_interval: LatencyHistogram::new(),
            queue_wait: LatencyHistogram::new(),
            send: LatencyHistogram::new(),
        }
    }

    pub(crate) fn record_capture(&self, at: Instant) {
        let nanos = u64::try_from(at.saturating_duration_since(self.base).as_nanos())
            .unwrap_or(u64::MAX - 1)
            + 1;
        let previous = self.last_capture.swap(nanos, Ordering::Relaxed);
        if previous != 0 && nanos > previous {
            self.capture_interval
                .record(Duration::from_nanos(nanos - previous));
        }
    }

    pub(crate) fn record_queue_wait(&self, waited: Duration) {
        self.queue_wait.record(waited);
    }

    pub(crate) fn record_send(&self, elapsed: Duration) {
        self.send.record(elapsed);
    }

    /// Summaries of everything recorded since the previous call; `None` when
    /// nothing at all was recorded.
    pub(crate) fn take_summary(&self) -> Option<CaptureSendTimingSummary> {
        let summary = CaptureSendTimingSummary {
            capture_interval: summarize(&self.capture_interval.take()),
            queue_wait: summarize(&self.queue_wait.take()),
            send: summarize(&self.send.take()),
        };
        (summary.capture_interval.is_some()
            || summary.queue_wait.is_some()
            || summary.send.is_some())
        .then_some(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_contiguous_and_cover_their_values() {
        for micros in [0, 1, 3, 4, 7, 8, 9, 15, 16, 10_000, 10_667, 1 << 29] {
            let index = bucket_index(micros);
            assert!(micros <= bucket_upper(index), "{micros}");
            assert!(index == 0 || micros > bucket_upper(index - 1), "{micros}");
        }
        for index in 1..BUCKETS {
            assert_eq!(bucket_index(bucket_upper(index - 1) + 1), index);
        }
        assert_eq!(bucket_index(u64::MAX), BUCKETS - 1);
    }

    #[test]
    fn summary_reports_quantiles_within_bucket_width_and_resets() {
        let timing = CaptureSendTiming::new();
        for _ in 0..98 {
            timing.record_send(Duration::from_micros(1_000));
        }
        timing.record_send(Duration::from_micros(20_000));
        timing.record_send(Duration::from_micros(50_000));
        let send = timing.take_summary().unwrap().send.unwrap();
        assert_eq!(send.count, 100);
        assert!((1_000..1_200).contains(&send.p50_us));
        assert!((1_000..1_200).contains(&send.p95_us));
        assert!((20_000..24_000).contains(&send.p99_us));
        assert!((50_000..60_000).contains(&send.max_us));
        assert!(timing.take_summary().is_none());
    }

    #[test]
    fn capture_interval_needs_two_pushes() {
        let timing = CaptureSendTiming::new();
        let start = Instant::now();
        timing.record_capture(start);
        assert!(timing.take_summary().is_none());
        timing.record_capture(start + Duration::from_millis(10));
        timing.record_capture(start + Duration::from_millis(20));
        let capture = timing.take_summary().unwrap().capture_interval.unwrap();
        assert_eq!(capture.count, 2);
        assert!((10_000..12_000).contains(&capture.p50_us));
    }
}
