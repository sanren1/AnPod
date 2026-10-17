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
#[path = "presence_tests.rs"]
mod tests;
