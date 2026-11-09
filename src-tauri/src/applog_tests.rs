use super::*;

fn temp_log_dir(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "anpod-applog-{name}-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let _ = fs::remove_dir_all(&directory);
    directory
}

#[test]
fn utc_timestamp_formats_milliseconds_and_calendar_dates() {
    assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(utc_timestamp(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    assert_eq!(utc_timestamp(951_782_400_000), "2000-02-29T00:00:00.000Z");
    assert_eq!(utc_timestamp(1_798_761_599_999), "2026-12-31T23:59:59.999Z");
}

#[test]
fn line_carries_level_run_module_event_and_single_line_detail() {
    let line = format_line(
        Level::Warn,
        1_700_000_000_123,
        "abcd0123",
        "state",
        "connect.failed",
        &"reason=timeout\nsecond",
    );
    assert_eq!(
        line,
        "2023-11-14T22:13:20.123Z WARN  run=abcd0123 state connect.failed reason=timeout | second\r\n"
    );
    let bare = format_line(Level::Info, 0, "r", "state", "startup", &"");
    assert!(bare.ends_with("state startup\r\n"));
}

#[test]
fn levels_order_from_error_to_debug() {
    assert!(Level::Error < Level::Warn && Level::Warn < Level::Info && Level::Info < Level::Debug);
    assert!(Level::Info <= MAX_LEVEL);
}

#[test]
fn ipv4_addresses_are_redacted_but_versions_and_counts_are_kept() {
    assert_eq!(
        redact_ipv4("connect 192.168.1.20:5555 failed"),
        "connect <ipv4>:5555 failed"
    );
    assert_eq!(redact_ipv4("peer=10.0.0.1,"), "peer=<ipv4>,");
    assert_eq!(
        redact_ipv4("version 1.19.0 frames=48000"),
        "version 1.19.0 frames=48000"
    );
    assert_eq!(redact_ipv4("1.2.3.4.5 999.1.1.1"), "1.2.3.4.5 999.1.1.1");
    assert_eq!(redact_ipv4("连接 172.16.0.9 失败"), "连接 <ipv4> 失败");
}

#[test]
fn sink_keeps_one_handle_and_rotates_within_bounded_files() {
    let directory = temp_log_dir("rotate");
    let mut sink = Sink::new(directory.clone(), "anpod", "log", 100, KEEP_ROTATED);
    let line = format!("{}\r\n", "x".repeat(38));
    for _ in 0..40 {
        sink.write_line(&line);
    }
    let mut names = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "anpod.1.log",
            "anpod.2.log",
            "anpod.3.log",
            "anpod.4.log",
            "anpod.log"
        ]
    );
    for name in &names {
        assert!(fs::metadata(directory.join(name)).unwrap().len() <= 100);
    }
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn sink_appends_to_existing_file_size() {
    let directory = temp_log_dir("append");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(FILE_NAME), "y".repeat(90)).unwrap();
    let mut sink = Sink::new(directory.clone(), "anpod", "log", 100, KEEP_ROTATED);
    sink.write_line("0123456789abcdef\r\n");
    assert_eq!(
        fs::read_to_string(directory.join("anpod.1.log"))
            .unwrap()
            .len(),
        90
    );
    assert_eq!(
        fs::read_to_string(directory.join(FILE_NAME)).unwrap(),
        "0123456789abcdef\r\n"
    );
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn timeline_sink_uses_its_own_bounded_files() {
    let directory = temp_log_dir("timeline");
    let mut sink = Sink::new(directory.clone(), "timeline", "jsonl", 50, 2);
    for _ in 0..10 {
        sink.write_line("{\"state\":\"streaming\",\"n\":1234567}\r\n");
    }
    let mut names = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["timeline.1.jsonl", "timeline.2.jsonl", "timeline.jsonl"]
    );
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn export_keeps_structured_detail_and_drops_free_text() {
    let structured = "2026-11-07T05:38:52.123Z INFO  run=ab12cd34 state state.changed from=ready to=connecting generation=3";
    assert_eq!(
        export_line(structured).as_deref(),
        Some("2026-11-07T05:38:52.123Z INFO run=ab12cd34 state state.changed from=ready to=connecting generation=3")
    );
    let free = "2026-11-07T05:38:52.123Z WARN  run=ab12cd34 command command.failed command=connect_device error=找不到 Pixel 8";
    assert_eq!(
        export_line(free).as_deref(),
        Some("2026-11-07T05:38:52.123Z WARN run=ab12cd34 command command.failed")
    );
    assert_eq!(
        export_line("1730000000 legacy unstructured line here"),
        None
    );
    assert_eq!(export_line(""), None);
}
