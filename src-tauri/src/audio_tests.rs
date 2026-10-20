use super::{forget_restored, receive_handshake, remember_before_muting};
use std::time::Duration;
use std::{fs, path::PathBuf};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

unsafe extern "C" fn fake_get_caps(_: *mut super::CapabilitiesV2) -> i32 {
    1
}

#[test]
fn legacy_native_function_table_keeps_pcm16_and_rejects_requested_float_v2_clearly() {
    let (v2, incomplete) = super::assemble_v2_functions(None, None, None, None);
    assert!(v2.is_none());
    assert!(!incomplete);
    assert!(super::missing_v2_error(incomplete).contains("PCM16 仍可使用"));

    let (v2, incomplete) = super::assemble_v2_functions(Some(fake_get_caps), None, None, None);
    assert!(v2.is_none());
    assert!(incomplete);
    assert!(super::missing_v2_error(incomplete).contains("接口不完整"));
}

#[test]
fn float_v2_abi_layout_and_response_version_are_checked() {
    assert_eq!(std::mem::size_of::<super::CapabilitiesV2>(), 16);
    assert_eq!(std::mem::size_of::<super::CaptureFormatV2>(), 48);

    let mut format = super::CaptureFormatV2 {
        size: std::mem::size_of::<super::CaptureFormatV2>() as u32,
        version: 2,
        mix_sample_rate: 48_000,
        mix_channels: 2,
        mix_sample_format: super::FLOAT_CAPTURE_FORMAT_FLOAT,
        mix_container_bits: 32,
        mix_valid_bits: 0,
        capture_sample_rate: 48_000,
        capture_channels: 2,
        capture_sample_format: super::FLOAT_CAPTURE_FORMAT_FLOAT,
        ..super::CaptureFormatV2::default()
    };
    assert!(super::NativeV2Fns::validate_capture_format(&format).is_ok());
    format.size = 47;
    assert!(super::NativeV2Fns::validate_capture_format(&format).is_err());
    format.size = 48;
    format.version = 3;
    assert!(super::NativeV2Fns::validate_capture_format(&format).is_err());
}

#[test]
fn stopped_float_queue_is_not_reused_by_audio_sink_fast_path() {
    let queue = std::sync::Arc::new(crate::network::FloatPcmQueue::new(false));
    assert!(super::can_reuse_float_sink(Some(&queue), &queue));
    queue.stop_and_clear();
    assert!(!super::can_reuse_float_sink(Some(&queue), &queue));
}

#[test]
fn float_pcm16_quantization_is_symmetric_saturating_and_bit_exact_on_pcm16_grid() {
    for value in i16::MIN..=i16::MAX {
        let normalized = value as f32 / 32768.0;
        assert_eq!(super::float_sample_to_pcm16(normalized), Ok(value));
    }
    assert_eq!(super::float_sample_to_pcm16(1.0), Ok(i16::MAX));
    assert_eq!(super::float_sample_to_pcm16(-1.0), Ok(i16::MIN));
    assert_eq!(super::float_sample_to_pcm16(0.5 / 32768.0), Ok(1));
    assert_eq!(super::float_sample_to_pcm16(-0.5 / 32768.0), Ok(-1));
    assert!(super::float_sample_to_pcm16(f32::NAN).is_err());
    assert!(super::float_sample_to_pcm16(f32::INFINITY).is_err());
}

#[test]
fn marker_write_failure_keeps_endpoint_recoverable() {
    let root = std::env::temp_dir().join(format!("anpod-audio-marker-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let not_a_directory = root.join("file");
    fs::write(&not_a_directory, b"x").unwrap();
    let marker = not_a_directory.join("muted-endpoints.txt");
    let mut ids = Vec::new();

    assert!(remember_before_muting(&marker, &mut ids, "speaker-1").is_err());
    assert!(ids.is_empty());

    ids.extend(["speaker-1".into(), "speaker-2".into()]);
    assert!(forget_restored(&marker, &mut ids, "speaker-1").is_err());
    assert_eq!(ids, ["speaker-1", "speaker-2"]);
    let _ = fs::remove_dir_all(PathBuf::from(root));
}

#[tokio::test]
async fn stale_handshake_nonce_is_ignored_until_current_session_arrives() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    sender.send("previous-session".into()).unwrap();
    sender.send("current-session".into()).unwrap();
    let result = receive_handshake(
        &mut receiver,
        "current-session",
        Duration::from_secs(1),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(result.unwrap(), "current-session");
}

#[tokio::test]
async fn cancelled_handshake_wait_returns_without_a_new_callback() {
    let (_sender, mut receiver) = mpsc::unbounded_channel();
    let token = CancellationToken::new();
    token.cancel();
    let result = receive_handshake(
        &mut receiver,
        "current-session",
        Duration::from_secs(30),
        &token,
    )
    .await;
    assert_eq!(result.unwrap_err(), "连接已取消。");
}
