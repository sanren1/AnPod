
use super::{forget_restored, receive_handshake, remember_before_muting};
use std::time::Duration;
use std::{fs, path::PathBuf};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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
