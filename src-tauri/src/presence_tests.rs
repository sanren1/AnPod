
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
