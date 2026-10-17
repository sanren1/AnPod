use serde::Serialize;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

/// How often a binding that answered last time is probed again.
pub(crate) const ONLINE_INTERVAL: Duration = Duration::from_secs(30);
/// Backoff after consecutive misses; the last entry repeats.
const BACKOFF_SECONDS: [u64; 5] = [1, 2, 5, 15, 60];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PresenceView {
    pub(crate) binding_reference: String,
    pub(crate) online: bool,
    pub(crate) last_seen_unix: Option<u64>,
    pub(crate) last_address: Option<String>,
}

struct Entry {
    online: bool,
    last_seen_unix: Option<u64>,
    last_address: Option<Ipv4Addr>,
    failures: u32,
    next_probe: Instant,
}

/// Per-binding presence built only from authenticated discovery replies, so a
/// device cannot appear online by answering without the binding secret.
#[derive(Default)]
pub(crate) struct PresenceTracker {
    entries: HashMap<String, Entry>,
}

impl PresenceTracker {
    /// Adds a binding that has no history yet, without making it look online.
    pub(crate) fn register(&mut self, binding: &str, address: Option<Ipv4Addr>, now: Instant) {
        self.entries
            .entry(binding.to_owned())
            .or_insert_with(|| Entry {
                online: false,
                last_seen_unix: None,
                last_address: address,
                failures: 0,
                next_probe: now,
            });
    }

    pub(crate) fn observe(&mut self, binding: &str, address: Ipv4Addr, now: Instant, unix: u64) {
        let entry = self
            .entries
            .entry(binding.to_owned())
            .or_insert_with(|| Entry {
                online: false,
                last_seen_unix: None,
                last_address: None,
                failures: 0,
                next_probe: now,
            });
        entry.online = true;
        entry.last_seen_unix = Some(unix);
        entry.last_address = Some(address);
        entry.failures = 0;
        entry.next_probe = now + ONLINE_INTERVAL;
    }

    pub(crate) fn miss(&mut self, binding: &str, now: Instant) {
        let Some(entry) = self.entries.get_mut(binding) else {
            return;
        };
        entry.online = false;
        entry.failures = entry.failures.saturating_add(1);
        let index = usize::try_from(entry.failures.saturating_sub(1))
            .unwrap_or(usize::MAX)
            .min(BACKOFF_SECONDS.len() - 1);
        entry.next_probe = now + Duration::from_secs(BACKOFF_SECONDS[index]);
    }

    /// Bindings whose next probe is due, with the address worth trying first.
    pub(crate) fn due(&self, now: Instant) -> Vec<(String, Option<Ipv4Addr>)> {
        let mut due: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.next_probe <= now)
            .map(|(binding, entry)| (binding.clone(), entry.last_address))
            .collect();
        due.sort();
        due
    }

    /// Drops a binding that no longer exists, so a reset cannot leave a stale
    /// "last seen" line behind in the UI.
    pub(crate) fn forget(&mut self, binding: &str) {
        self.entries.remove(binding);
    }

    pub(crate) fn view(&self) -> Vec<PresenceView> {
        let mut view: Vec<_> = self
            .entries
            .iter()
            .map(|(binding, entry)| PresenceView {
                binding_reference: binding.clone(),
                online: entry.online,
                last_seen_unix: entry.last_seen_unix,
                last_address: entry.last_address.map(|address| address.to_string()),
            })
            .collect();
        view.sort_by(|left, right| left.binding_reference.cmp(&right.binding_reference));
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 1, last)
    }

    #[test]
    fn backoff_grows_after_consecutive_misses_and_resets_on_success() {
        let start = Instant::now();
        let mut tracker = PresenceTracker::default();
        tracker.observe("a".repeat(32).as_str(), address(23), start, 1_000);
        assert!(tracker.view()[0].online);
        assert_eq!(
            tracker.due(start + Duration::from_secs(29)),
            Vec::new(),
            "online interval is respected"
        );

        let mut now = start + ONLINE_INTERVAL;
        assert_eq!(
            tracker.due(now).len(),
            1,
            "the online interval expires after 30s"
        );
        for (step, expected) in BACKOFF_SECONDS.iter().enumerate() {
            tracker.miss("a".repeat(32).as_str(), now);
            assert!(!tracker.view()[0].online);
            assert!(
                tracker
                    .due(now + Duration::from_secs(expected.saturating_sub(1)))
                    .is_empty(),
                "step {step} must wait {expected}s"
            );
            now += Duration::from_secs(*expected);
            assert_eq!(
                tracker.due(now).len(),
                1,
                "step {step} should be due after {expected}s"
            );
        }

        tracker.observe("a".repeat(32).as_str(), address(23), now, 2_000);
        assert!(tracker.view()[0].online, "a verified reply restores online");
        assert!(
            tracker.due(now + Duration::from_secs(29)).is_empty(),
            "backoff resets to the online interval"
        );
    }

    #[test]
    fn presence_view_exposes_the_last_address_and_hides_unseen_bindings() {
        let now = Instant::now();
        let mut tracker = PresenceTracker::default();
        tracker.register("b".repeat(32).as_str(), Some(address(30)), now);
        tracker.observe("a".repeat(32).as_str(), address(23), now, 42);
        let view = tracker.view();
        assert_eq!(view.len(), 2);
        assert_eq!(view[0].binding_reference, "a".repeat(32));
        assert_eq!(view[0].last_address.as_deref(), Some("192.168.1.23"));
        assert_eq!(view[0].last_seen_unix, Some(42));
        assert!(!view[1].online);
        assert_eq!(
            view[1].last_seen_unix, None,
            "a registered binding that never answered is not online"
        );
        assert_eq!(
            tracker.due(now).len(),
            1,
            "only the registered binding is due; the observed one just answered"
        );
    }

    #[test]
    fn a_forgotten_binding_disappears_from_the_view_and_stops_being_probed() {
        let now = Instant::now();
        let mut tracker = PresenceTracker::default();
        tracker.observe("a".repeat(32).as_str(), address(23), now, 42);
        tracker.forget("a".repeat(32).as_str());
        assert!(
            tracker.view().is_empty(),
            "a reset binding must not keep a presence line"
        );
        assert!(
            tracker.due(now + ONLINE_INTERVAL).is_empty(),
            "a reset binding must not be probed again"
        );
    }
}
