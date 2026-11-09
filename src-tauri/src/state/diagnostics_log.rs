//! Key-event log lines derived from consecutive diagnostics timeline samples,
//! so state, mode, format and loss changes reach the log without a second
//! state machine or poll.

use super::DiagnosticsTimelineSample;
use crate::applog::Level;
use crate::receiver::PlaybackDiagnostics;
use serde::Serialize;

pub(crate) struct TimelineEvent {
    pub level: Level,
    pub module: &'static str,
    pub event: &'static str,
    pub detail: String,
}

fn event(level: Level, module: &'static str, event: &'static str, detail: String) -> TimelineEvent {
    TimelineEvent {
        level,
        module,
        event,
        detail,
    }
}

fn json<T: Serialize>(value: &Option<T>) -> String {
    value
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok())
        .unwrap_or_else(|| "null".into())
}

fn text(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("none")
}

fn generation(value: Option<u64>) -> String {
    value.map_or_else(|| "none".into(), |value| value.to_string())
}

/// Events describing what changed from `previous` to `next`. Loss counters are
/// only compared inside one session, receiver generation and output epoch,
/// and only when both sides are known.
pub(crate) fn timeline_events(
    previous: Option<&DiagnosticsTimelineSample>,
    next: &DiagnosticsTimelineSample,
) -> Vec<TimelineEvent> {
    let mut events = Vec::new();
    let generation_text = generation(next.session_generation);
    let Some(previous) = previous else {
        events.push(event(
            Level::Info,
            "diagnostics",
            "timeline.started",
            format!("state={} generation={generation_text}", next.state),
        ));
        return events;
    };
    if previous.state != next.state {
        let level = if next.state == "error" {
            Level::Warn
        } else {
            Level::Info
        };
        events.push(event(
            level,
            "state",
            "state.changed",
            format!(
                "from={} to={} generation={generation_text}",
                previous.state, next.state
            ),
        ));
    }
    if previous.session_generation != next.session_generation
        || previous.session_transport != next.session_transport
        || previous.session_wireless_protocol != next.session_wireless_protocol
    {
        events.push(event(
            Level::Info,
            "session",
            "session.changed",
            format!(
                "generation={}->{generation_text} transport={}->{} protocol={}->{}",
                generation(previous.session_generation),
                text(&previous.session_transport),
                text(&next.session_transport),
                text(&previous.session_wireless_protocol),
                text(&next.session_wireless_protocol),
            ),
        ));
    }
    for (field, before, after) in [
        ("output", &previous.output, &next.output),
        ("playbackMode", &previous.playback_mode, &next.playback_mode),
        ("audioQuality", &previous.audio_quality, &next.audio_quality),
        ("transport", &previous.transport, &next.transport),
    ] {
        if before != after {
            events.push(event(
                Level::Info,
                "settings",
                "mode.changed",
                format!("field={field} from={before} to={after} generation={generation_text}"),
            ));
        }
    }
    for (name, before, after) in [
        (
            "negotiated",
            json(&previous.negotiated_audio_format),
            json(&next.negotiated_audio_format),
        ),
        (
            "windowsMix",
            json(&previous.windows_mix_format),
            json(&next.windows_mix_format),
        ),
        (
            "androidOutput",
            json(&previous.android_output_format),
            json(&next.android_output_format),
        ),
    ] {
        if before != after {
            events.push(event(
                Level::Info,
                "audio",
                "format.changed",
                format!("kind={name} generation={generation_text} format={after}"),
            ));
        }
    }
    if previous.session_generation.is_some()
        && previous.session_generation == next.session_generation
    {
        // The PC queue belongs to the session; receiver counters also need
        // the same receiver generation and output epoch.
        push_increase(
            "pcDropped",
            previous.pc_dropped_frames,
            next.pc_dropped_frames,
            &generation_text,
            &mut events,
        );
        if let (Some(before), Some(after)) = (previous.playback.as_ref(), next.playback.as_ref()) {
            loss_events(before, after, &generation_text, &mut events);
        }
    }
    events
}

fn same_epoch(before: &PlaybackDiagnostics, after: &PlaybackDiagnostics) -> bool {
    before.receiver_generation.is_some()
        && before.receiver_generation == after.receiver_generation
        && before.output_epoch == after.output_epoch
}

fn loss_events(
    before: &PlaybackDiagnostics,
    after: &PlaybackDiagnostics,
    generation: &str,
    events: &mut Vec<TimelineEvent>,
) {
    if !same_epoch(before, after) {
        return;
    }
    let counters = [
        (
            "underruns",
            before.underrun_count.map(u64::from),
            after.underrun_count.map(u64::from),
        ),
        (
            "udpExpired",
            before.udp_expired_frames,
            after.udp_expired_frames,
        ),
        (
            "tcpDroppedBacklog",
            before.tcp_dropped_backlog_frames,
            after.tcp_dropped_backlog_frames,
        ),
        (
            "queueDropped",
            before.queue_dropped_frames,
            after.queue_dropped_frames,
        ),
        (
            "queueExpired",
            before.queue_expired_frames,
            after.queue_expired_frames,
        ),
    ];
    for (name, before, after) in counters {
        push_increase(name, before, after, generation, events);
    }
}

fn push_increase(
    name: &str,
    before: Option<u64>,
    after: Option<u64>,
    generation: &str,
    events: &mut Vec<TimelineEvent>,
) {
    if let (Some(before), Some(after)) = (before, after) {
        if after > before {
            events.push(event(
                Level::Warn,
                "audio",
                "loss.increased",
                format!(
                    "counter={name} delta={} total={after} generation={generation}",
                    after - before
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(state: &str, generation: Option<u64>) -> DiagnosticsTimelineSample {
        DiagnosticsTimelineSample {
            sampled_at_utc: String::new(),
            elapsed_ms: 0,
            session_generation: generation,
            state: state.into(),
            transport: "usb".into(),
            session_transport: generation.map(|_| "lan".into()),
            session_wireless_protocol: generation.map(|_| "udp".into()),
            output: "phone".into(),
            playback_mode: "Balanced".into(),
            audio_quality: "standard".into(),
            negotiated_audio_format: None,
            windows_mix_format: None,
            android_output_format: None,
            pc_queued_frames: None,
            pc_dropped_frames: None,
            drift_target_frames: None,
            drift_observed_frames: None,
            drift_error_frames: None,
            drift_correction_ppm: None,
            drift_reset_reason: None,
            playback: None,
            capture_send_timing: None,
        }
    }

    fn playback(
        receiver_generation: Option<u64>,
        epoch: Option<u32>,
        underruns: u32,
    ) -> PlaybackDiagnostics {
        PlaybackDiagnostics {
            receiver_generation,
            output_epoch: epoch,
            underrun_count: Some(underruns),
            ..PlaybackDiagnostics::default()
        }
    }

    fn names(events: &[TimelineEvent]) -> Vec<(&'static str, String)> {
        events
            .iter()
            .map(|event| (event.event, event.detail.clone()))
            .collect()
    }

    #[test]
    fn first_sample_starts_and_unchanged_sample_is_silent() {
        let first = sample("ready", None);
        assert_eq!(
            names(&timeline_events(None, &first))[0].0,
            "timeline.started"
        );
        assert!(timeline_events(Some(&first), &sample("ready", None)).is_empty());
    }

    #[test]
    fn state_session_and_mode_changes_are_logged() {
        let before = sample("connecting", None);
        let mut after = sample("streaming", Some(7));
        after.audio_quality = "hires".into();
        let events = timeline_events(Some(&before), &after);
        assert_eq!(
            names(&events),
            [
                (
                    "state.changed",
                    "from=connecting to=streaming generation=7".to_string()
                ),
                (
                    "session.changed",
                    "generation=none->7 transport=none->lan protocol=none->udp".to_string()
                ),
                (
                    "mode.changed",
                    "field=audioQuality from=standard to=hires generation=7".to_string()
                ),
            ]
        );
        let error = timeline_events(Some(&after), &sample("error", Some(7)));
        assert_eq!(error[0].level, Level::Warn);
    }

    #[test]
    fn loss_counters_compare_only_within_one_session_and_output_epoch() {
        let mut before = sample("streaming", Some(3));
        before.pc_dropped_frames = Some(10);
        before.playback = Some(playback(Some(1), Some(2), 4));
        let mut after = sample("streaming", Some(3));
        after.pc_dropped_frames = Some(25);
        after.playback = Some(playback(Some(1), Some(2), 6));
        assert_eq!(
            names(&timeline_events(Some(&before), &after)),
            [
                (
                    "loss.increased",
                    "counter=pcDropped delta=15 total=25 generation=3".to_string()
                ),
                (
                    "loss.increased",
                    "counter=underruns delta=2 total=6 generation=3".to_string()
                ),
            ]
        );

        after.playback = Some(playback(Some(1), Some(3), 9));
        assert_eq!(timeline_events(Some(&before), &after).len(), 1);
        after.playback = Some(playback(None, None, 9));
        before.playback = Some(playback(None, None, 4));
        assert_eq!(timeline_events(Some(&before), &after).len(), 1);

        let next_session = {
            let mut next = sample("streaming", Some(4));
            next.pc_dropped_frames = Some(99);
            next
        };
        assert!(names(&timeline_events(Some(&before), &next_session))
            .iter()
            .all(|(name, _)| *name != "loss.increased"));
    }
}
