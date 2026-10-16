use crate::state::{adb_run, adb_run_with_stdin, parse_devices, AdbResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{future::Future, fs::{self, File}, io::Read, path::Path, time::{Duration, Instant}};
use tokio_util::sync::CancellationToken;

const PACKAGE: &str = "app.anpod.receiver";
const HEALTH_URI: &str = "content://app.anpod.receiver.state/health";
const BINDING_URI: &str = "content://app.anpod.receiver.state/binding";
const INTERFACES_URI: &str = "content://app.anpod.receiver.state/interfaces";
const NETWORK_URI: &str = "content://app.anpod.receiver.state/network";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingResetPreview {
    pub serial: String,
    pub android_user: String,
    pub binding_reference: String,
    pub phone_state: String,
    pub phone_commitment: Option<String>,
    pub pc_state: String,
    pub pc_commitment: Option<String>,
    pub receiver_busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingResetResult {
    pub outcome: String,
    pub serial: String,
    pub android_user: String,
    pub binding_reference: String,
    pub phone_state: String,
    pub phone_commitment: Option<String>,
    pub pc_state: String,
    pub pc_commitment: Option<String>,
    pub reason: String,
    pub next_action: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiverInterface {
    pub address: String,
    pub prefix_length: u8,
    pub interface_index: u32,
    pub interface_name: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiverNetworkStatus {
    pub network_audio_version: u32,
    pub network_audio_state: String,
    pub network_audio_failure: String,
    pub pcm_bytes_written: u64,
    pub network_audio_address: String,
    pub network_audio_port: u16,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiverStatus {
    pub installed: bool,
    pub version_name: Option<String>,
    pub version_code: Option<u32>,
    pub service_active: bool,
    pub foreground: bool,
    pub healthy: bool,
    pub audio_active: bool,
    pub playback_diagnostics: Option<PlaybackDiagnostics>,
    pub error: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackDiagnostics {
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub receiver_sample_rate: Option<u32>,
    pub receiver_channels: Option<u8>,
    pub pc_dropped_frames: Option<u64>,
    pub pc_queued_frames: Option<u64>,
    pub udp_jitter_target_frames: Option<u32>,
    pub udp_queued_frames: Option<u64>,
    pub udp_expired_frames: Option<u64>,
    pub tcp_dropped_backlog_frames: Option<u64>,
    pub track_pending_frames: Option<u64>,
    pub underrun_count: Option<u32>,
    pub underrun_increment: Option<u64>,
    pub playback_advancing: Option<bool>,
    pub requested_buffer_frames: Option<u32>,
    pub accepted_buffer_frames: Option<u32>,
    pub effective_buffer_frames: Option<u32>,
    pub capacity_frames: Option<u32>,
    pub start_threshold_frames: Option<u32>,
    pub pre_roll_frames: Option<u32>,
}

impl ReceiverStatus {
    pub fn monitoring_error(&self) -> Option<String> {
        if self.healthy && self.service_active && self.error.is_none() { None }
        else { Some(self.error.clone().unwrap_or_else(|| self.message.clone())) }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactMetadata {
    application_id: String,
    version_code: u32,
    version_name: String,
    sha256: String,
    signer_sha256: String,
    #[serde(default)]
    source_commit: Option<String>,
}

const V3_CONTROL_MAJOR: u16 = 3;
const PCM_AUDIO_MAJOR: u8 = 1;
const CAP_RECEIVER_UI: u32 = 1 << 0;
const CAP_AUTHENTICATED_STATUS: u32 = 1 << 1;
const CAP_PLAYBACK_PROGRESS: u32 = 1 << 3;

#[derive(Clone, Copy)]
struct AuditedReceiverArtifact {
    sha256: &'static str,
    version_code: u32,
    version_name: &'static str,
    source_commit: &'static str,
    signer_sha256: &'static str,
}

// Only same-signer APKs with an exact audited digest and recorded source are admitted.
const AUDITED_RECEIVER_ARTIFACTS: &[AuditedReceiverArtifact] = &[
    AuditedReceiverArtifact {
        sha256: "344c47ddbd51b991ba4562e09c11c195815f7f363f9d1196146263df929fa1b7",
        version_code: 10,
        version_name: "0.8.0-v3-flow-candidate",
        source_commit: "61f32f7798f29bb39cf256997de50992a85ea519",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
    AuditedReceiverArtifact {
        sha256: "c66a6a350907ab2581e941665b9984c47632777855717fe414964a2177baee41",
        version_code: 11,
        version_name: "0.9.0-v3-flow-candidate",
        source_commit: "61f32f7798f29bb39cf256997de50992a85ea519",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
    AuditedReceiverArtifact {
        sha256: "de3924d1e7801103b1fb9c9ebcde323c35493bb4fdd8aa63aa204c2998ebdb77",
        version_code: 3,
        version_name: "0.3.0-v2-candidate",
        source_commit: "20e42fac0274252dccb8ccc8b7d5e416ab8e958b",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
    AuditedReceiverArtifact {
        sha256: "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050",
        version_code: 3,
        version_name: "0.3.0-v2-candidate",
        source_commit: "81dda7630e2728881d4aa556698ba8f6bb36c453",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
    AuditedReceiverArtifact {
        sha256: "22af84b1989011d3cd214ff321ccd37bc4fe4de03546850f65d5db778f49d8c9",
        version_code: 3,
        version_name: "0.3.0-v2-candidate",
        source_commit: "608c98c75ec04e11555e9e547caa516c96d4e0dd",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
    AuditedReceiverArtifact {
        sha256: "b52a326f234ab84d6aba9b5ffbb840e75f90a101cb5479461da7a53d184a4abb",
        version_code: 4,
        version_name: "0.4.0-v3-candidate",
        source_commit: "8f868708a83a32973be902fabf7dc8357f3c55a6",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
    AuditedReceiverArtifact {
        sha256: "c514dfd555aa2734178131db47037875cf1086d934b45639b799471297668be3",
        version_code: 6,
        version_name: "0.5.0-v4-candidate",
        source_commit: "20f2389af0526ebdbc20924cad42570ce51110ce",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct VerifiedReceiverArtifact {
    version_code: u32,
    version_name: String,
    sha256: String,
    signer_sha256: String,
    source_commit: Option<String>,
}

fn verified_installed_artifact(digest: &str, current: &ArtifactMetadata) -> Option<VerifiedReceiverArtifact> {
    if digest.eq_ignore_ascii_case(&current.sha256) {
        return Some(VerifiedReceiverArtifact {
            version_code: current.version_code,
            version_name: current.version_name.clone(),
            sha256: current.sha256.to_ascii_lowercase(),
            signer_sha256: current.signer_sha256.to_ascii_lowercase(),
            source_commit: current.source_commit.clone(),
        });
    }
    AUDITED_RECEIVER_ARTIFACTS.iter().find(|artifact| {
        digest.eq_ignore_ascii_case(artifact.sha256)
            && current.application_id == PACKAGE
            && current.signer_sha256.eq_ignore_ascii_case(artifact.signer_sha256)
    })
        .map(|artifact| VerifiedReceiverArtifact {
            version_code: artifact.version_code,
            version_name: artifact.version_name.into(),
            sha256: artifact.sha256.into(),
            signer_sha256: artifact.signer_sha256.into(),
            source_commit: Some(artifact.source_commit.into()),
        })
}

#[derive(Clone, Copy)]
struct ReceiverCapabilities {
    control_major: u16,
    audio_major: u8,
    bits: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeatureCompatibility { NotNegotiated, Compatible, Incompatible }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateAvailability { Unavailable, CandidateAvailable, BlockedBySignature }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagementAccess { Denied, PreflightOnly, PreflightAndUpdateOnly, CompatibleSession }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReceiverCompatibilityDecision {
    identity_trusted: bool,
    features: FeatureCompatibility,
    update: UpdateAvailability,
    usb_upgrade_available: bool,
    management: ManagementAccess,
}

fn decide_receiver_compatibility(
    identity: Option<&VerifiedReceiverArtifact>,
    authenticated: bool,
    capabilities: Option<ReceiverCapabilities>,
    required_capabilities: u32,
    update: Option<&ArtifactMetadata>,
) -> ReceiverCompatibilityDecision {
    let identity_trusted = identity.is_some();
    let features = if !authenticated {
        FeatureCompatibility::NotNegotiated
    } else {
        match capabilities {
            None => FeatureCompatibility::NotNegotiated,
            Some(peer) if peer.control_major == V3_CONTROL_MAJOR
                && peer.audio_major == PCM_AUDIO_MAJOR
                && peer.bits & required_capabilities == required_capabilities => FeatureCompatibility::Compatible,
            Some(_) => FeatureCompatibility::Incompatible,
        }
    };
    let update = match (identity, update) {
        (Some(installed), Some(candidate)) if candidate.application_id == PACKAGE
            && candidate.version_code > installed.version_code
            && valid_digest(&candidate.sha256) => {
                if candidate.signer_sha256.eq_ignore_ascii_case(&installed.signer_sha256) {
                    UpdateAvailability::CandidateAvailable
                } else {
                    UpdateAvailability::BlockedBySignature
                }
            }
        _ => UpdateAvailability::Unavailable,
    };
    let usb_upgrade_available = identity_trusted && update == UpdateAvailability::CandidateAvailable;
    let management = if !authenticated || !identity_trusted {
        ManagementAccess::Denied
    } else if features == FeatureCompatibility::Compatible {
        ManagementAccess::CompatibleSession
    } else if usb_upgrade_available {
        ManagementAccess::PreflightAndUpdateOnly
    } else {
        ManagementAccess::PreflightOnly
    };
    ReceiverCompatibilityDecision { identity_trusted, features, update, usb_upgrade_available, management }
}

#[derive(Default)]
struct Health {
    application_id: String,
    version_name: String,
    version_code: u32,
    service_active: bool,
    foreground: bool,
    healthy: bool,
    audio_active: bool,
    playback_diagnostics: PlaybackDiagnostics,
    error: String,
}

trait AdbRunner {
    fn run<'a>(&'a self, serial: Option<&'a str>, args: &'a [&'a str], limit: Duration,
        cancel: Option<&'a CancellationToken>) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a;
    fn run_with_stdin<'a>(&'a self, serial: Option<&'a str>, args: &'a [&'a str], input: &'a [u8],
        limit: Duration, cancel: &'a CancellationToken) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a;
}

struct SystemAdb<'a>(&'a Path);

impl AdbRunner for SystemAdb<'_> {
    fn run<'a>(&'a self, serial: Option<&'a str>, args: &'a [&'a str], limit: Duration,
        cancel: Option<&'a CancellationToken>) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a {
        adb_run(self.0, serial, args, limit, cancel)
    }

    fn run_with_stdin<'a>(&'a self, serial: Option<&'a str>, args: &'a [&'a str], input: &'a [u8],
        limit: Duration, cancel: &'a CancellationToken) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a {
        adb_run_with_stdin(self.0, serial, args, input, limit, cancel)
    }
}

#[cfg(windows)]
#[allow(dead_code)]
pub async fn provision(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path,
    binding_path: &Path, cancel: &CancellationToken) -> Result<String, String> {
    if let Some(parent) = binding_path.parent() {
        fs::create_dir_all(parent).map_err(|_| "无法创建本机绑定存储目录。".to_string())?;
    }
    provision_with(&SystemAdb(adb), serial, apk, metadata_path, binding_path, cancel,
        Duration::from_secs(8)).await
}

#[cfg(windows)]
pub async fn binding_reference(adb: &Path, serial: &str, cancel: &CancellationToken) -> Result<String, String> {
    let runner = SystemAdb(adb);
    ensure_usb_device(&runner, serial, cancel).await?;
    let user = current_android_user(&runner, serial, cancel).await?;
    Ok(binding_reference_for(serial, &user))
}

#[cfg(windows)]
fn binding_reference_for(serial: &str, android_user: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"AnPod receiver binding reference v1\0");
    digest.update(serial.as_bytes());
    digest.update([0]);
    digest.update(android_user.as_bytes());
    let result: [u8; 32] = digest.finalize().into();
    hex_digest(&result)
}

#[cfg(windows)]
pub async fn binding_reset_preview(adb: &Path, serial: &str, data_dir: &Path, apk: &Path,
    metadata_path: &Path, cancel: &CancellationToken) -> Result<BindingResetPreview, String> {
    binding_reset_preview_with(&SystemAdb(adb), serial, data_dir, apk, metadata_path, cancel).await
}

#[cfg(windows)]
async fn binding_reset_preview_with(adb: &impl AdbRunner, serial: &str, data_dir: &Path, apk: &Path,
    metadata_path: &Path, cancel: &CancellationToken) -> Result<BindingResetPreview, String> {
    use crate::security::{BindingStore, SecurityError};

    ensure_usb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let reference = binding_reference_for(serial, &user);
    let metadata = read_artifact(apk, metadata_path)?;
    verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    require_binding_reset_capability(adb, serial, &user, cancel).await?;

    let (phone_state, phone_commitment) = match read_binding_state(adb, serial, &user, cancel).await? {
        BindingStatus::Empty => ("empty".to_owned(), None),
        BindingStatus::Pending => ("pending".to_owned(), None),
        BindingStatus::Saved(commitment) => ("saved".to_owned(), Some(commitment)),
        BindingStatus::Failed(_) => ("failed".to_owned(), None),
    };
    let network = read_reset_network_state(adb, serial, &user, cancel).await?;
    let health = read_health(adb, serial, &user, cancel).await?;
    let binding_path = data_dir.join("receiver-bindings").join(format!("{reference}.dat"));
    let (pc_state, pc_commitment) = match BindingStore::load(&binding_path) {
        Ok(material) => ("saved".to_owned(), Some(hex_digest(&material.commitment()))),
        Err(SecurityError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => ("missing".to_owned(), None),
        Err(_) => return Err("所选 PC 绑定文件无法通过 DPAPI 和格式核验；未停止会话或删除数据。".into()),
    };

    Ok(BindingResetPreview {
        serial: serial.to_owned(), android_user: user, binding_reference: reference,
        phone_state, phone_commitment, pc_state, pc_commitment,
        receiver_busy: health.audio_active || network != "stopped",
    })
}

#[cfg(windows)]
async fn require_binding_reset_capability(adb: &impl AdbRunner, serial: &str, user: &str,
    cancel: &CancellationToken) -> Result<(), String> {
    let result = adb.run(Some(serial), &["shell", "content", "query", "--user", user,
        "--uri", BINDING_URI, "--projection", "bindingResetVersion"], Duration::from_secs(6), Some(cancel)).await?;
    let supported = result.success && result.stdout.lines().filter_map(parse_content_row)
        .any(|row| row.get("bindingResetVersion").is_some_and(|value| value == "1"));
    if supported { Ok(()) }
    else { Err("RECEIVER_UPDATE_REQUIRED:已安装 Receiver 不支持条件绑定重置。".into()) }
}

#[cfg(windows)]
async fn read_reset_network_state(adb: &impl AdbRunner, serial: &str, user: &str,
    cancel: &CancellationToken) -> Result<String, String> {
    let result = adb.run(Some(serial), &["shell", "content", "query", "--user", user,
        "--uri", NETWORK_URI, "--projection", "networkAudioVersion:networkAudioState"],
        Duration::from_secs(6), Some(cancel)).await?;
    if !result.success { return Err("无法确认手机是否仍有网络接收会话；未停止会话或删除绑定。".into()); }
    let row = result.stdout.lines().find_map(parse_content_row)
        .ok_or_else(|| "手机网络接收状态格式无效；未停止会话或删除绑定。".to_string())?;
    if row.get("networkAudioVersion").map(String::as_str) != Some("1") {
        return Err("手机网络接收状态版本不受支持；未停止会话或删除绑定。".into());
    }
    row.get("networkAudioState").cloned()
        .ok_or_else(|| "手机网络接收状态缺少会话状态；未停止会话或删除绑定。".into())
}

#[cfg(windows)]
fn reset_result(preview: &BindingResetPreview, outcome: &str, phone_state: &str,
    phone_commitment: Option<String>, pc_state: &str, pc_commitment: Option<String>,
    reason: &str, next_action: &str) -> BindingResetResult {
    BindingResetResult {
        outcome: outcome.into(), serial: preview.serial.clone(), android_user: preview.android_user.clone(),
        binding_reference: preview.binding_reference.clone(), phone_state: phone_state.into(), phone_commitment,
        pc_state: pc_state.into(), pc_commitment, reason: reason.into(), next_action: next_action.into(),
    }
}

#[cfg(windows)]
pub async fn reset_binding(adb: &Path, preview: &BindingResetPreview, data_dir: &Path, apk: &Path,
    metadata_path: &Path, cancel: &CancellationToken, is_current: &impl Fn() -> bool)
    -> Result<BindingResetResult, String> {
    reset_binding_with(&SystemAdb(adb), preview, data_dir, apk, metadata_path, cancel, is_current).await
}

#[cfg(windows)]
async fn reset_binding_with(adb: &impl AdbRunner, expected: &BindingResetPreview, data_dir: &Path,
    apk: &Path, metadata_path: &Path, cancel: &CancellationToken, is_current: &impl Fn() -> bool)
    -> Result<BindingResetResult, String> {
    use crate::security::BindingStore;

    let before = binding_reset_preview_with(adb, &expected.serial, data_dir, apk, metadata_path, cancel).await?;
    if !same_reset_target(&before, expected) { return Err("重置预览已过期（设备、Android user 或绑定摘要发生变化）；未删除绑定，请重新检查。".into()); }
    if before.phone_state == "pending" || before.phone_state == "failed" {
        return Err("手机绑定状态正在写入或不可读取；请完成/检查绑定操作后重新预览。".into());
    }
    if before.receiver_busy { return Err("手机仍有活动的接收或播放会话；请先在手机上停止接收，再重新预览。".into()); }
    if !is_current() || cancel.is_cancelled() { return Err("绑定重置已取消；尚未更改任何绑定。".into()); }

    if before.phone_state == "empty" && before.pc_state == "missing" {
        return Ok(reset_result(&before, "alreadyEmpty", "empty", None, "missing", None,
            "两端都没有此目标的绑定。", "如需配对，请单独启动 USB 首次绑定。"));
    }

    let selection = match before.phone_commitment.as_deref() {
        Some(commitment) => format!("expectedCommitment={commitment}"),
        None => "expectedEmpty".to_owned(),
    };
    let delete_result = adb.run(Some(&before.serial), &["shell", "content", "delete", "--user",
        &before.android_user, "--uri", BINDING_URI, "--where", &selection],
        Duration::from_secs(10), Some(cancel)).await;
    let binding_path = data_dir.join("receiver-bindings").join(format!("{}.dat", before.binding_reference));

    // The shell exit status is not authoritative; observe the fixed URI after the attempt.
    let after_phone = match binding_reset_preview_with(adb, &before.serial, data_dir, apk, metadata_path, cancel).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let (phone_state, phone_commitment) = match read_reset_phone_status(adb, &before, apk, metadata_path, cancel).await {
                Ok(BindingStatus::Empty) => ("empty".to_owned(), None),
                Ok(BindingStatus::Pending) => ("pending".to_owned(), None),
                Ok(BindingStatus::Saved(commitment)) => ("saved".to_owned(), Some(commitment)),
                Ok(BindingStatus::Failed(_)) => ("failed".to_owned(), None),
                Err(_) => ("unknown".to_owned(), None),
            };
            let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
            let outcome = if phone_state == "unknown" { "phoneStateUnknown" }
                else if phone_state == "empty" && pc_state == "unknown" { "phoneClearedPcUnknown" }
                else if phone_state == "empty" { pc_partial_outcome(&before, &pc_state, &pc_commitment) }
                else { "phoneRetained" };
            return Ok(reset_result(&before, outcome, &phone_state, phone_commitment,
                &pc_state, pc_commitment, &format!("手机删除结果无法回读：{error}"),
                "保持 USB 连接并重新预览手机与 PC 绑定状态。"));
        }
    };
    if after_phone.android_user != before.android_user || after_phone.binding_reference != before.binding_reference {
        return Ok(reset_result(&before, "targetChanged", "unknown", None, &before.pc_state,
            before.pc_commitment.clone(), "Android user 或 USB 目标已变化；PC 绑定仍保留。",
            "重新选择并核验 USB 设备与 Android user。"));
    }
    if after_phone.phone_state != "empty" {
        let exit_detail = match delete_result {
            Ok(result) if result.success => "删除命令已返回，但手机仍报告原绑定。",
            Ok(_) => "删除命令失败且手机仍报告绑定。",
            Err(_) => "删除命令结果不确定且手机仍报告绑定。",
        };
        return Ok(reset_result(&before, "phoneRetained", &after_phone.phone_state,
            after_phone.phone_commitment, &before.pc_state, before.pc_commitment.clone(), exit_detail,
            "绑定未清除；核对手机端状态后重新预览。PC 绑定未删除。"));
    }
    if !is_current() || cancel.is_cancelled() {
        let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
        let outcome = pc_partial_outcome(&before, &pc_state, &pc_commitment);
        return Ok(reset_result(&before, outcome, "empty", None,
            &pc_state, pc_commitment, "操作在手机绑定清除后取消；PC 记录状态已回读。",
            "保持 USB 连接并重试重置，以清理这一个 PC 记录。"));
    }

    let before_pc_remove = match binding_reset_preview_with(adb, &before.serial, data_dir, apk, metadata_path, cancel).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let (phone_state, phone_commitment) = match read_reset_phone_status(adb, &before, apk, metadata_path, cancel).await {
                Ok(BindingStatus::Empty) => ("empty".to_owned(), None),
                Ok(BindingStatus::Pending) => ("pending".to_owned(), None),
                Ok(BindingStatus::Saved(commitment)) => ("saved".to_owned(), Some(commitment)),
                Ok(BindingStatus::Failed(_)) => ("failed".to_owned(), None),
                Err(_) => ("unknown".to_owned(), None),
            };
            let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
            let outcome = if phone_state == "unknown" { "phoneStateUnknown" }
                else if phone_state == "empty" { pc_partial_outcome(&before, &pc_state, &pc_commitment) }
                else { "phoneRetained" };
            return Ok(reset_result(&before, outcome, &phone_state, phone_commitment,
                &pc_state, pc_commitment, &format!("手机已清除；删除 PC 记录前重新核验失败：{error}"),
                "保持 USB 连接并重试；重新预览手机与 PC 绑定状态。"));
        }
    };
    if before_pc_remove.android_user != before.android_user || before_pc_remove.binding_reference != before.binding_reference
        || before_pc_remove.phone_state != "empty" || before_pc_remove.pc_state != before.pc_state
        || before_pc_remove.pc_commitment != before.pc_commitment {
        return Ok(reset_result(&before, "phoneClearedPcChanged", &before_pc_remove.phone_state,
            before_pc_remove.phone_commitment, &before_pc_remove.pc_state, before_pc_remove.pc_commitment,
            "手机已清除，但删除前目标状态发生变化；未删除 PC 记录。",
            "重新预览并确认此 USB 目标对应的 PC 绑定状态。"));
    }
    if before_pc_remove.receiver_busy {
        return Ok(reset_result(&before, "phoneClearedPcRetained", "empty", None,
            &before.pc_state, before.pc_commitment.clone(), "手机已清除，但 Receiver 出现活动会话；未删除 PC 记录。",
            "先停止手机接收，再重新预览并重试。"));
    }
    if !is_current() || cancel.is_cancelled() {
        let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
        let outcome = pc_partial_outcome(&before, &pc_state, &pc_commitment);
        return Ok(reset_result(&before, outcome, "empty", None,
            &pc_state, pc_commitment, "操作在 PC 记录删除前取消；PC 记录状态已回读。",
            "保持 USB 连接并重试重置。"));
    }

    let expected_pc = before.pc_commitment.as_deref().map(parse_digest)
        .transpose().map_err(|_| "PC 绑定摘要格式无效；没有删除 PC 记录。".to_string())?;
    let binding_path = data_dir.join("receiver-bindings").join(format!("{}.dat", before.binding_reference));
    if let Err(error) = BindingStore::remove_if_matches(&binding_path, expected_pc.as_ref()) {
        let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
        let outcome = pc_partial_outcome(&before, &pc_state, &pc_commitment);
        return Ok(reset_result(&before, outcome, "empty", None,
            &pc_state, pc_commitment, &format!("手机已清除；PC 条件删除未完成，回读当前 PC 状态：{error}"),
            "重新预览并确认此 USB 目标对应的 PC 绑定状态。"));
    }

    if !is_current() || cancel.is_cancelled() {
        return Ok(reset_result(&before, "pcClearedPhoneEmpty", "empty", None, "missing", None,
            "两端绑定已清除；操作在最终核验前取消。", "重新预览确认两端都为空。"));
    }
    let final_snapshot = match binding_reset_preview_with(adb, &before.serial, data_dir, apk, metadata_path, cancel).await {
        Ok(snapshot) => snapshot,
        Err(error) => return Ok(reset_result(&before, "pcClearedPhoneUnknown", "unknown", None,
            "missing", None, &format!("PC 目标记录已删除；手机最终状态无法确认：{error}"),
            "保持 USB 连接并重新预览；若手机仍有绑定，再执行一次明确确认的重置。")),
    };
    if final_snapshot.android_user == before.android_user && final_snapshot.binding_reference == before.binding_reference
        && final_snapshot.phone_state == "empty" && final_snapshot.pc_state == "missing" {
        Ok(reset_result(&before, "reset", "empty", None, "missing", None,
            "所选 USB 设备与 Android user 的两端绑定均已清除。", "如需配对，请单独启动 USB 首次绑定。"))
    } else {
        Ok(reset_result(&before, "pcClearedPhoneChanged", &final_snapshot.phone_state,
            final_snapshot.phone_commitment, "missing", None,
            "PC 目标记录已删除，但手机状态在最终核验时发生变化。",
            "重新预览当前手机绑定；不要继续使用旧配对信息。"))
    }
}

#[cfg(windows)]
async fn read_reset_phone_status(adb: &impl AdbRunner, expected: &BindingResetPreview, apk: &Path,
    metadata_path: &Path, cancel: &CancellationToken) -> Result<BindingStatus, String> {
    ensure_usb_device(adb, &expected.serial, cancel).await?;
    let user = current_android_user(adb, &expected.serial, cancel).await?;
    if user != expected.android_user { return Err("Android user changed".into()); }
    let metadata = read_artifact(apk, metadata_path)?;
    verify_installed_identity(adb, &expected.serial, &user, &metadata, cancel).await?;
    require_binding_reset_capability(adb, &expected.serial, &user, cancel).await?;
    read_binding_state(adb, &expected.serial, &user, cancel).await
}

#[cfg(windows)]
fn local_reset_binding(path: &Path) -> (String, Option<String>) {
    use crate::security::{BindingStore, SecurityError};
    match BindingStore::load(path) {
        Ok(material) => ("saved".into(), Some(hex_digest(&material.commitment()))),
        Err(SecurityError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => ("missing".into(), None),
        Err(_) => ("unknown".into(), None),
    }
}

#[cfg(windows)]
fn pc_partial_outcome(before: &BindingResetPreview, state: &str, commitment: &Option<String>) -> &'static str {
    if state == "unknown" { "phoneClearedPcUnknown" }
    else if state == "missing" { "reset" }
    else if state == before.pc_state && commitment == &before.pc_commitment { "phoneClearedPcRetained" }
    else { "phoneClearedPcChanged" }
}

#[cfg(windows)]
fn same_reset_target(left: &BindingResetPreview, right: &BindingResetPreview) -> bool {
    left.serial == right.serial && left.android_user == right.android_user
        && left.binding_reference == right.binding_reference
        && left.phone_state == right.phone_state && left.phone_commitment == right.phone_commitment
        && left.pc_state == right.pc_state && left.pc_commitment == right.pc_commitment
}

#[cfg(windows)]
fn parse_digest(value: &str) -> Result<[u8; 32], ()> {
    if !valid_digest(value) { return Err(()); }
    let mut result = [0u8; 32];
    for (index, output) in result.iter_mut().enumerate() {
        *output = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| ())?;
    }
    Ok(result)
}

fn valid_binding_reference(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(windows)]
async fn provision_with(adb: &impl AdbRunner, serial: &str, apk: &Path, metadata_path: &Path,
    binding_path: &Path, cancel: &CancellationToken, confirmation_timeout: Duration) -> Result<String, String> {
    use crate::security::{BindingMaterial, BindingStore, SecurityError};

    ensure_usb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let expected_reference = binding_reference_for(serial, &user);
    if binding_path.file_stem().and_then(|value| value.to_str()) != Some(expected_reference.as_str()) {
        return Err("USB 设备或 Android 用户已变化；没有复用其他设备的绑定。".into());
    }
    let metadata = read_artifact(apk, metadata_path)?;
    if !is_installed(adb, serial, &user, cancel).await? {
        return Err("Receiver 尚未安装；没有发送绑定材料。".into());
    }
    verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;

    let material = match BindingStore::load(binding_path) {
        Ok(material) => material,
        Err(SecurityError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            let generated = BindingMaterial::generate().map_err(|_| "无法创建安全绑定材料。".to_string())?;
            BindingStore::create_or_confirm(binding_path, &generated)
                .map_err(|_| "无法安全保存或确认本机绑定材料。".to_string())?;
            generated
        }
        Err(_) => return Err("无法读取本机 DPAPI 绑定材料；没有发送绑定材料。".into()),
    };
    let commitment = material.commitment();
    let commitment_text = hex_digest(&commitment);
    let mut record = material.encode();

    ensure_usb_device(adb, serial, cancel).await?;
    if current_android_user(adb, serial, cancel).await? != user {
        record.0.fill(0);
        return Err("Android 当前用户已变化；本机绑定材料已保留，未继续写入。".into());
    }
    verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    let write_result = adb.run_with_stdin(Some(serial), &[
        "shell", "-T", "content", "write", "--user", &user, "--uri", BINDING_URI,
    ], &record.0, Duration::from_secs(15), cancel).await;
    record.0.fill(0);
    // content's WriteCommand can print provider errors while leaving the shell exit code successful.
    // Only the fixed URI's digest-matched status query below completes provisioning.
    let _write_result = write_result?;

    tokio::time::timeout(confirmation_timeout, async {
        loop {
            if cancel.is_cancelled() { return Err("操作已取消；本机 DPAPI 绑定材料已保留，可用相同路径重试。".into()); }
            ensure_usb_device(adb, serial, cancel).await?;
            if current_android_user(adb, serial, cancel).await? != user {
                return Err("Android 当前用户已变化；本机 DPAPI 绑定材料已保留，未确认远端绑定。".into());
            }
            verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
            match read_binding_state(adb, serial, &user, cancel).await? {
                BindingStatus::Saved(received) if received == commitment_text => return Ok(commitment_text),
                BindingStatus::Saved(_) => return Err("手机已有不同绑定；未替换材料，本机 DPAPI 副本已保留。".into()),
                BindingStatus::Failed(message) => return Err(message.into()),
                BindingStatus::Empty | BindingStatus::Pending => {}
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err("操作已取消；本机 DPAPI 绑定材料已保留，可用相同路径重试。".into()),
                _ = tokio::time::sleep(Duration::from_millis(200)) => {}
            }
        }
    }).await.map_err(|_| "手机未确认绑定；本机 DPAPI 材料已保留，可用相同路径重试。".to_string())?
}

#[cfg(windows)]
fn hex_digest(value: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in value { result.push(HEX[(byte >> 4) as usize] as char); result.push(HEX[(byte & 15) as usize] as char); }
    result
}

#[cfg(windows)]
enum BindingStatus { Empty, Pending, Saved(String), Failed(&'static str) }

#[cfg(windows)]
async fn read_binding_state(adb: &impl AdbRunner, serial: &str, user: &str,
    cancel: &CancellationToken) -> Result<BindingStatus, String> {
    let result = adb.run(Some(serial), &[
        "shell", "content", "query", "--user", user, "--uri", BINDING_URI,
        "--projection", "bindingState:bindingCommitment:bindingFailure",
    ], Duration::from_secs(6), Some(cancel)).await?;
    if !result.success { return Err("无法读取手机绑定确认状态；本机 DPAPI 副本已保留。".into()); }
        let row = result.stdout.lines().find_map(parse_content_row)
        .ok_or_else(|| "手机绑定状态格式无效；本机 DPAPI 副本已保留。".to_string())?;
    let fields: std::collections::HashMap<_, _> = row.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
    let failure = match fields.get("bindingFailure").copied() {
        Some("") => None,
        Some("cancelled") => Some("操作已取消；本机 DPAPI 绑定材料已保留，可用相同路径重试。"),
        Some("timeout") => Some("手机绑定写入超时；本机 DPAPI 绑定材料已保留，可用相同路径重试。"),
        Some("invalid_record") => Some("手机拒绝了绑定记录；本机 DPAPI 绑定材料已保留。"),
        Some("pipe_error") => Some("手机绑定管道失败；本机 DPAPI 绑定材料已保留，可重试。"),
        Some("storage_error") => Some("手机未能保存绑定记录；本机 DPAPI 绑定材料已保留。"),
        Some("conflict") => Some("手机已有不同绑定；未替换材料，本机 DPAPI 副本已保留。"),
        None => return Err("手机绑定状态缺少失败码；本机 DPAPI 副本已保留。".into()),
        _ => return Err("手机绑定失败状态无效；本机 DPAPI 副本已保留。".into()),
    };
    if let Some(message) = failure { return Ok(BindingStatus::Failed(message)); }
    match fields.get("bindingState").copied() {
        Some("empty") if fields.get("bindingCommitment") == Some(&"") => Ok(BindingStatus::Empty),
        Some("failed") if fields.get("bindingCommitment") == Some(&"") =>
            Err("手机绑定失败状态缺少错误码；本机 DPAPI 副本已保留。".into()),
        Some("pending") => {
            let commitment = fields.get("bindingCommitment").copied().unwrap_or("");
            if commitment.is_empty() || valid_digest(commitment) { Ok(BindingStatus::Pending) }
            else { Err("手机绑定摘要无效；本机 DPAPI 副本已保留。".into()) }
        }
        Some("saved") => {
            let commitment = fields.get("bindingCommitment").copied().unwrap_or("");
            if valid_digest(commitment) { Ok(BindingStatus::Saved(commitment.to_ascii_lowercase())) }
            else { Err("手机绑定摘要无效；本机 DPAPI 副本已保留。".into()) }
        }
        _ => Err("手机绑定状态不受支持；本机 DPAPI 副本已保留。".into()),
    }
}

pub async fn status(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    status_with(&SystemAdb(adb), serial, apk, metadata_path, cancel).await
}

pub async fn network_interfaces(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path,
    cancel: &CancellationToken) -> Result<Vec<ReceiverInterface>, String> {
    let runner = SystemAdb(adb);
    ensure_usb_device(&runner, serial, cancel).await?;
    let user = current_android_user(&runner, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    if !is_installed(&runner, serial, &user, cancel).await? { return Err("Receiver 尚未安装。".into()); }
    verify_installed_identity(&runner, serial, &user, &metadata, cancel).await?;
    let result = runner.run(Some(serial), &["shell", "content", "query", "--user", &user,
        "--uri", INTERFACES_URI, "--projection", "address:prefixLength:interfaceIndex:interfaceName"],
        Duration::from_secs(8), Some(cancel)).await?;
    ensure_adb_success(&result)?;
    Ok(parse_receiver_interfaces(&result.stdout))
}

fn parse_receiver_interfaces(output: &str) -> Vec<ReceiverInterface> {
    let mut interfaces = Vec::new();
    for line in output.lines().filter_map(parse_content_row) {
        let Some(address) = line.get("address") else { continue; };
        let Some(prefix_length) = line.get("prefixLength").and_then(|value| value.parse::<u8>().ok()) else { continue; };
        let Some(interface_index) = line.get("interfaceIndex").and_then(|value| value.parse::<u32>().ok()) else { continue; };
        let Some(interface_name) = line.get("interfaceName") else { continue; };
        if address.parse::<std::net::Ipv4Addr>().is_err() || !(1..=32).contains(&prefix_length) { continue; }
        interfaces.push(ReceiverInterface { address: address.clone(), prefix_length, interface_index, interface_name: interface_name.clone() });
    }
    interfaces
}

pub async fn start_network(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path,
    address: &str, prefix_length: u8, cancel: &CancellationToken) -> Result<ReceiverNetworkStatus, String> {
    let interfaces = network_interfaces(adb, serial, apk, metadata_path, cancel).await?;
    let selected = interfaces.iter().find(|interface| interface.address == address && interface.prefix_length == prefix_length)
        .ok_or_else(|| "所选手机接口已不可用；请重新读取手机接口列表。".to_string())?;
    let _ = selected;
    let user = current_android_user(&SystemAdb(adb), serial, cancel).await?;
    let address = address.to_owned();
    let prefix = prefix_length.to_string();
    let result = adb_run(adb, Some(serial), &["shell", "am", "start", "--user", &user, "-W", "-n",
        "app.anpod.receiver/.BootstrapActivity", "-a", "app.anpod.receiver.action.START_NETWORK_AUDIO",
        "--es", "networkAddress", &address, "--ei", "networkPrefixLength", &prefix],
        Duration::from_secs(40), Some(cancel)).await?;
    ensure_adb_success(&result)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if cancel.is_cancelled() { return Err("操作已取消。".into()); }
        let status = network_status(adb, serial, apk, metadata_path, cancel).await?;
        if status.network_audio_version != 1 {
            return Err("RECEIVER_UPDATE_REQUIRED:Receiver 网络协议版本不兼容。".into());
        }
        if matches!(status.network_audio_state.as_str(), "authenticated" | "receiverRunning" | "streaming" | "paused") {
            if status.network_audio_address != address || status.network_audio_port == 0 {
                return Err("Receiver 未报告本次 bootstrap 的监听地址和端口。".into());
            }
            return Ok(status);
        }
        if status.network_audio_state == "failed" { return Err(format!("Receiver 网络启动失败：{}。", status.network_audio_failure)); }
        if Instant::now() >= deadline { return Err("Receiver 未能在临时授权窗口内启动网络监听。".into()); }
        tokio::select! {
            _ = cancel.cancelled() => return Err("操作已取消。".into()),
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    }
}

pub async fn network_status(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path,
    cancel: &CancellationToken) -> Result<ReceiverNetworkStatus, String> {
    let runner = SystemAdb(adb);
    ensure_usb_device(&runner, serial, cancel).await?;
    let user = current_android_user(&runner, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    verify_installed_identity(&runner, serial, &user, &metadata, cancel).await?;
    let result = runner.run(Some(serial), &["shell", "content", "query", "--user", &user,
        "--uri", NETWORK_URI, "--projection", "networkAudioVersion:networkAudioState:networkAudioFailure:pcmBytesWritten:networkAudioAddress:networkAudioPort"],
        Duration::from_secs(8), Some(cancel)).await?;
    if !result.success {
        return Err("RECEIVER_UPDATE_REQUIRED:Receiver 网络状态版本不可用。".into());
    }
    parse_network_status(&result.stdout)
}

fn parse_network_status(output: &str) -> Result<ReceiverNetworkStatus, String> {
    let row = output.lines().find_map(parse_content_row).ok_or_else(|| "手机网络状态格式无效。".to_string())?;
    Ok(ReceiverNetworkStatus {
        network_audio_version: row.get("networkAudioVersion").and_then(|value| value.parse().ok()).ok_or("手机不支持网络接收。")?,
        network_audio_state: row.get("networkAudioState").cloned().ok_or("手机网络状态缺少会话状态。")?,
        network_audio_failure: row.get("networkAudioFailure").cloned().unwrap_or_default(),
        pcm_bytes_written: row.get("pcmBytesWritten").and_then(|value| value.parse().ok()).unwrap_or(0),
        network_audio_address: row.get("networkAudioAddress").cloned().unwrap_or_default(),
        network_audio_port: row.get("networkAudioPort").and_then(|value| value.parse().ok()).unwrap_or(0),
    })
}

fn parse_content_row(line: &str) -> Option<std::collections::HashMap<String, String>> {
    let (index, row) = line.trim().strip_prefix("Row: ")?.split_once(' ')?;
    index.parse::<u32>().ok()?;
    Some(row.split(", ").filter_map(|field| {
        let (key, value) = field.split_once('=')?;
        Some((key.to_owned(), value.to_owned()))
    }).collect())
}

async fn status_with(adb: &impl AdbRunner, serial: &str, apk: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    ensure_adb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    if !is_installed(adb, serial, &user, cancel).await? {
        return Ok(ReceiverStatus {
            installed: false, version_name: None, version_code: None,
            service_active: false, foreground: false, healthy: false,
            audio_active: false, playback_diagnostics: None, error: None,
            message: "Receiver 尚未安装。".into(),
        });
    }
    let metadata = read_artifact(apk, metadata_path)?;
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    let health = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&health, &installed)?;
    Ok(status_from_health(health))
}

pub async fn install(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    install_usb_with(&SystemAdb(adb), serial, apk, metadata_path, None, cancel).await
}

pub async fn upgrade_usb(adb: &Path, serial: &str, binding_reference: &str, binding_path: &Path, apk: &Path,
    metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    install_usb_with(&SystemAdb(adb), serial, apk, metadata_path, Some((binding_reference, binding_path)), cancel).await
}

async fn install_usb_with(adb: &impl AdbRunner, serial: &str, apk: &Path, metadata_path: &Path,
    binding: Option<(&str, &Path)>, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    let (user, metadata, _) = prepare_usb_install(adb, serial, apk, metadata_path, binding, cancel).await?;
    let apk_text = apk.to_string_lossy().into_owned();
    let result = adb.run(Some(serial), &["install", "-r", "--user", &user, &apk_text], Duration::from_secs(90), Some(cancel)).await?;
    ensure_adb_success(&result)?;
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    if !installed.sha256.eq_ignore_ascii_case(&metadata.sha256) {
        return Err("安装命令成功，但设备上的 Receiver APK 摘要与本次目标包不一致；未报告安装完成。".into());
    }
    let health = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&health, &installed)?;
    if let Some((_, binding_path)) = binding {
        verify_saved_binding(adb, serial, &user, &metadata, binding_path, cancel).await?;
    }
    Ok(status_from_health(health))
}

async fn prepare_usb_install(adb: &impl AdbRunner, serial: &str, apk: &Path, metadata_path: &Path,
    binding: Option<(&str, &Path)>, cancel: &CancellationToken) -> Result<(String, ArtifactMetadata, Option<VerifiedReceiverArtifact>), String> {
    ensure_usb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    if let Some((expected, _)) = binding {
        if !valid_binding_reference(expected) || binding_reference_for(serial, &user) != expected {
            return Err("所选 USB 设备或 Android 用户与已保存 Receiver 绑定不匹配。".into());
        }
    }
    let metadata = read_artifact(apk, metadata_path)?;
    let installed_on_user = is_installed(adb, serial, &user, cancel).await?;
    if binding.is_some() && !installed_on_user {
        return Err("所选绑定对应的 Receiver 尚未安装；未创建新绑定或继续升级。".into());
    }
    let installed = if installed_on_user {
        let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
        if metadata.version_code <= installed.version_code {
            return Err("已阻止降级或同版本重装：本机 Receiver APK 必须高于设备受信版本；未执行安装。".into());
        }
        if !metadata.signer_sha256.eq_ignore_ascii_case(&installed.signer_sha256) {
            return Err("Receiver 签名与设备上的受信版本不一致；未执行安装。".into());
        }
        Some(installed)
    } else { None };
    if let Some((expected, binding_path)) = binding {
        if binding_path.file_stem().and_then(|value| value.to_str()) != Some(expected) {
            return Err("所选 Receiver 本机绑定材料与 USB 目标不匹配；未执行升级。".into());
        }
        verify_saved_binding(adb, serial, &user, &metadata, binding_path, cancel).await?;
    }
    Ok((user, metadata, installed))
}

pub async fn validate_usb_install(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path,
    cancel: &CancellationToken) -> Result<(), String> {
    prepare_usb_install(&SystemAdb(adb), serial, apk, metadata_path, None, cancel).await.map(|_| ())
}

#[cfg(test)]
async fn install_with(adb: &impl AdbRunner, serial: &str, apk: &Path, metadata_path: &Path,
    cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    install_usb_with(adb, serial, apk, metadata_path, None, cancel).await
}

pub async fn start(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    start_with(&SystemAdb(adb), serial, apk, metadata_path, cancel).await
}

pub async fn start_audio(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path,
    socket_name: &str, connect_code: &str, playback_mode: &str, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    start_with_action(&SystemAdb(adb), serial, apk, metadata_path, cancel,
        Some((socket_name, connect_code, playback_mode))).await
}

async fn start_with(adb: &impl AdbRunner, serial: &str, apk: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    start_with_action(adb, serial, apk, metadata_path, cancel, None).await
}

async fn start_with_action(adb: &impl AdbRunner, serial: &str, apk: &Path, metadata_path: &Path,
    cancel: &CancellationToken, audio: Option<(&str, &str, &str)>) -> Result<ReceiverStatus, String> {
    ensure_adb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    if !is_installed(adb, serial, &user, cancel).await? { return Err("Receiver 尚未安装。".into()); }
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    let before = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&before, &installed)?;
    if before.service_active && !before.error.is_empty() { return Err(format!("Receiver 启动失败：{}。", before.error)); }
    if before.healthy && audio.is_none() { return Ok(status_from_health(before)); }

    let mut args = vec![
        "shell".to_string(), "am".into(), "start".into(), "--user".into(), user.clone(), "-W".into(),
        "-n".into(), "app.anpod.receiver/.BootstrapActivity".into(), "-a".into(),
        if audio.is_some() { "app.anpod.receiver.action.START_AUDIO" } else { "app.anpod.receiver.action.START" }.into(),
    ];
    if let Some((socket_name, connect_code, playback_mode)) = audio {
        args.extend(["--es".into(), "socketName".into(), socket_name.into(),
            "--es".into(), "connectCode".into(), connect_code.into(),
            "--es".into(), "playbackMode".into(), playback_mode.into()]);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = adb.run(Some(serial), &refs, Duration::from_secs(45), Some(cancel)).await?;
    ensure_adb_success(&result)?;

    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if cancel.is_cancelled() { return Err("操作已取消。".into()); }
        let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
        let health = read_health(adb, serial, &user, cancel).await?;
        ensure_version(&health, &installed)?;
        if !health.error.is_empty() { return Err(format!("Receiver 启动失败：{}。", health.error)); }
        if health.healthy { return Ok(status_from_health(health)); }
        if Instant::now() >= deadline {
            return Err("Receiver 尚未报告健康状态；请完成手机上的系统通知授权并检查 Android 启动限制。".into());
        }
        tokio::select! {
            _ = cancel.cancelled() => return Err("操作已取消。".into()),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
    }
}

pub async fn health(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    health_with(&SystemAdb(adb), serial, apk, metadata_path, cancel).await
}

async fn health_with(adb: &impl AdbRunner, serial: &str, apk: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    ensure_adb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    if !is_installed(adb, serial, &user, cancel).await? { return Err("Receiver 尚未安装。".into()); }
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    let health = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&health, &installed)?;
    Ok(status_from_health(health))
}

pub async fn stop(adb: &Path, serial: &str, apk: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    stop_with(&SystemAdb(adb), serial, apk, metadata_path, cancel).await
}

async fn stop_with(adb: &impl AdbRunner, serial: &str, apk: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<ReceiverStatus, String> {
    ensure_adb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    if !is_installed(adb, serial, &user, cancel).await? { return Err("Receiver 尚未安装。".into()); }
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    let before = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&before, &installed)?;
    if before.application_id != PACKAGE { return Err("已安装包名冲突，未执行停止操作。".into()); }

    let result = adb.run(Some(serial), &[
        "shell", "am", "start", "--user", &user, "-W", "-n", "app.anpod.receiver/.BootstrapActivity",
        "-a", "app.anpod.receiver.action.STOP",
    ], Duration::from_secs(15), Some(cancel)).await?;
    ensure_adb_success(&result)?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if cancel.is_cancelled() { return Err("操作已取消。".into()); }
        let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
        let health = read_health(adb, serial, &user, cancel).await?;
        ensure_version(&health, &installed)?;
        if !health.service_active { return Ok(status_from_health(health)); }
        if Instant::now() >= deadline { return Err("Receiver 服务在停止请求后仍处于活动状态。".into()); }
        tokio::select! {
            _ = cancel.cancelled() => return Err("操作已取消。".into()),
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    }
}

async fn ensure_usb_device(adb: &impl AdbRunner, serial: &str, cancel: &CancellationToken) -> Result<(), String> {
    if serial.is_empty() || serial.contains('\r') || serial.contains('\n') || serial.contains(':') || serial.contains("_adb-tls-") {
        return Err("Receiver P1 只允许选择已授权的 USB ADB 设备。".into());
    }
    let transport = confirmed_adb_device(adb, serial, cancel).await?;
    if transport != "usb" {
        return Err("所选设备不是已授权的在线 USB 设备。".into());
    }
    Ok(())
}

async fn ensure_adb_device(adb: &impl AdbRunner, serial: &str, cancel: &CancellationToken) -> Result<(), String> {
    let transport = confirmed_adb_device(adb, serial, cancel).await?;
    if !matches!(transport.as_str(), "usb" | "wifi") {
        return Err("所选设备不是已授权的在线 USB 或无线 ADB 设备。".into());
    }
    Ok(())
}

async fn confirmed_adb_device(adb: &impl AdbRunner, serial: &str, cancel: &CancellationToken) -> Result<String, String> {
    if serial.is_empty() || serial.contains('\r') || serial.contains('\n') {
        return Err("Receiver 只允许选择明确的已授权 ADB 设备。".into());
    }
    let result = adb.run(None, &["devices", "-l"], Duration::from_secs(8), Some(cancel)).await?;
    ensure_adb_success(&result)?;
    let device = parse_devices(&result.stdout).into_iter().find(|device| device.serial == serial)
        .ok_or_else(|| "所选 ADB 设备当前不可用。".to_string())?;
    if device.state != "device" {
        return Err("所选设备不是已授权的在线 ADB 设备。".into());
    }
    let transport = if device.transport == "unknown" {
        let usb = adb.run(None, &["-d", "get-serialno"], Duration::from_secs(6), Some(cancel)).await?;
        ensure_adb_success(&usb)?;
        if usb.stdout.trim() == serial { "usb".to_owned() }
        else { return Err("无法确认所选设备使用已授权的 USB 或无线 ADB 传输。".into()); }
    } else { device.transport };
    Ok(transport)
}

async fn is_installed(adb: &impl AdbRunner, serial: &str, user: &str, cancel: &CancellationToken) -> Result<bool, String> {
    let result = adb.run(Some(serial), &["shell", "pm", "list", "packages", "--user", &user, PACKAGE], Duration::from_secs(6), Some(cancel)).await?;
    if !result.success { return Err(adb_error(&result)); }
    Ok(has_listed_package(&result.stdout))
}

async fn verify_installed_identity(adb: &impl AdbRunner, serial: &str, user: &str, metadata: &ArtifactMetadata, cancel: &CancellationToken) -> Result<VerifiedReceiverArtifact, String> {
    let listed = adb.run(Some(serial), &["shell", "pm", "list", "packages", "--user", &user, PACKAGE], Duration::from_secs(6), Some(cancel)).await?;
    ensure_adb_success(&listed)?;
    if !has_listed_package(&listed.stdout) { return Err("当前 Android 用户未安装 AnPod Receiver，已拒绝管理操作。".into()); }
    let path_result = adb.run(Some(serial), &["shell", "pm", "path", "--user", &user, PACKAGE], Duration::from_secs(6), Some(cancel)).await?;
    ensure_adb_success(&path_result)?;
    let path = parse_installed_apk_path(&path_result.stdout)
        .ok_or_else(|| "Receiver 安装路径不存在或不受支持，已拒绝管理操作。".to_string())?;
    let hash_result = adb.run(Some(serial), &["shell", "toybox", "sha256sum", path], Duration::from_secs(8), Some(cancel)).await?;
    ensure_adb_success(&hash_result)?;
    let installed_digest = hash_result.stdout.split_whitespace().next().unwrap_or("");
    if !valid_digest(installed_digest) {
        return Err("设备没有返回有效的 Receiver APK 摘要；未读取其状态或执行管理动作。".into());
    }
    verified_installed_artifact(installed_digest, metadata)
        .ok_or_else(|| "RECEIVER_UPDATE_REQUIRED:设备上的 Receiver APK 不在已核验版本中。".into())
}

async fn current_android_user(adb: &impl AdbRunner, serial: &str, cancel: &CancellationToken) -> Result<String, String> {
    let result = adb.run(Some(serial), &["shell", "am", "get-current-user"], Duration::from_secs(6), Some(cancel)).await?;
    ensure_adb_success(&result)?;
    let user = result.stdout.trim();
    if user.is_empty() || !user.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Android 未返回有效的当前用户编号；已停止 Receiver 管理。".into());
    }
    Ok(user.to_owned())
}

fn has_listed_package(output: &str) -> bool {
    output.lines().any(|line| line.trim() == format!("package:{PACKAGE}"))
}

async fn read_health(adb: &impl AdbRunner, serial: &str, user: &str, cancel: &CancellationToken) -> Result<Health, String> {
    let uri = HEALTH_URI;
    let result = adb.run(Some(serial), &["shell", "content", "query", "--user", user, "--uri", uri], Duration::from_secs(8), Some(cancel)).await?;
    if !result.success { return Err(adb_error(&result)); }
    parse_health(&result.stdout).ok_or_else(|| "Receiver 健康接口没有返回有效状态记录。".into())
}

fn read_artifact(apk: &Path, metadata_path: &Path) -> Result<ArtifactMetadata, String> {
    let metadata: ArtifactMetadata = serde_json::from_slice(&fs::read(metadata_path).map_err(|_| "找不到 Receiver APK 校验清单。".to_string())?)
        .map_err(|_| "Receiver APK 校验清单无效。".to_string())?;
    if metadata.application_id != PACKAGE || metadata.version_code == 0 || metadata.version_name.is_empty()
        || !valid_digest(&metadata.sha256) || !valid_digest(&metadata.signer_sha256) {
        return Err("Receiver APK 校验清单的包名、版本或摘要格式无效。".into());
    }
    let file_metadata = fs::metadata(apk).map_err(|_| "找不到 Receiver APK。".to_string())?;
    if file_metadata.len() == 0 { return Err("Receiver APK 是空文件。".into()); }
    let mut file = File::open(apk).map_err(|_| "无法读取 Receiver APK。".to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "无法读取 Receiver APK。".to_string())?;
        if count == 0 { break; }
        hasher.update(&buffer[..count]);
    }
    let digest = format!("{:x}", hasher.finalize());
    if digest != metadata.sha256.to_ascii_lowercase() { return Err("Receiver APK SHA-256 与校验清单不一致。".into()); }
    Ok(metadata)
}

pub(crate) fn local_upgrade_candidate(apk: &Path, metadata_path: &Path) -> Result<(u32, String, u64), String> {
    let metadata = read_artifact(apk, metadata_path)?;
    let size = fs::metadata(apk).map_err(|_| "找不到 Receiver APK。".to_string())?.len();
    Ok((metadata.version_code, metadata.version_name, size))
}

pub async fn validate_usb_upgrade(adb: &Path, serial: &str, binding_reference: &str, apk: &Path,
    binding_path: &Path, metadata_path: &Path, cancel: &CancellationToken) -> Result<(), String> {
    prepare_usb_install(&SystemAdb(adb), serial, apk, metadata_path, Some((binding_reference, binding_path)), cancel)
        .await.map(|_| ())
}

#[cfg(windows)]
async fn verify_saved_binding(adb: &impl AdbRunner, serial: &str, user: &str, metadata: &ArtifactMetadata,
    binding_path: &Path, cancel: &CancellationToken) -> Result<(), String> {
    use crate::security::BindingStore;
    let material = BindingStore::load(binding_path)
        .map_err(|_| "所选 Receiver 的本机绑定材料不可用；未执行升级。".to_string())?;
    let commitment = hex_digest(&material.commitment());
    verify_installed_identity(adb, serial, user, metadata, cancel).await?;
    match read_binding_state(adb, serial, user, cancel).await? {
        BindingStatus::Saved(saved) if saved == commitment => Ok(()),
        BindingStatus::Saved(_) => Err("手机保存的绑定与所选本机材料不一致；没有改写绑定或执行升级。".into()),
        BindingStatus::Empty | BindingStatus::Pending => Err("手机没有保存所选绑定；没有创建绑定或执行升级。".into()),
        BindingStatus::Failed(message) => Err(message.into()),
    }
}

#[cfg(not(windows))]
async fn verify_saved_binding(_adb: &impl AdbRunner, _serial: &str, _user: &str, _metadata: &ArtifactMetadata,
    _binding_path: &Path, _cancel: &CancellationToken) -> Result<(), String> {
    Err("USB Receiver 升级只在 Windows 上可用。".into())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_installed_apk_path(output: &str) -> Option<&str> {
    let mut paths = output.lines().filter_map(|line| line.trim().strip_prefix("package:"));
    let path = paths.next()?.trim();
    if paths.next().is_some() || !path.starts_with("/data/app/") || !path.ends_with("/base.apk")
        || !path.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"/_-.~=".contains(&byte)) {
        return None;
    }
    Some(path)
}

fn ensure_version(health: &Health, installed: &VerifiedReceiverArtifact) -> Result<(), String> {
    if health.application_id != PACKAGE { return Err("Receiver 包名不匹配。".into()); }
    if health.version_code != installed.version_code || health.version_name != installed.version_name {
        return Err("Receiver 健康记录版本与已核验 APK 不一致。".into());
    }
    Ok(())
}

fn status_from_health(health: Health) -> ReceiverStatus {
    let message = if !health.error.is_empty() {
        format!("Receiver 服务报告启动错误：{}。", health.error)
    } else if health.healthy {
        if health.audio_active { "Receiver 音频服务正在前台播放。".into() }
        else { "Receiver 管理服务正在前台运行；当前没有确认音频播放。".into() }
    } else if health.service_active {
        "Receiver 服务存在，但 Android 未确认其前台健康状态。".into()
    } else {
        "Receiver 已安装，服务当前未运行。".into()
    };
    ReceiverStatus {
        installed: true,
        version_name: Some(health.version_name),
        version_code: Some(health.version_code),
        service_active: health.service_active,
        foreground: health.foreground,
        healthy: health.healthy,
        audio_active: health.audio_active,
        playback_diagnostics: Some(health.playback_diagnostics),
        error: (!health.error.is_empty()).then_some(health.error),
        message,
    }
}

fn parse_health(output: &str) -> Option<Health> {
    let row = output.lines().find(|line| line.trim_start().starts_with("Row: 0 "))?.trim();
    let fields: std::collections::HashMap<_, _> = row.strip_prefix("Row: 0 ")?.split(", ")
        .filter_map(|field| field.split_once('='))
        .collect();
    Some(Health {
        application_id: fields.get("applicationId")?.to_string(),
        version_name: fields.get("versionName")?.to_string(),
        version_code: fields.get("versionCode")?.parse().ok()?,
        service_active: parse_bool(fields.get("serviceActive")?)?,
        foreground: parse_bool(fields.get("foreground")?)?,
        healthy: parse_bool(fields.get("healthy")?)?,
        audio_active: fields.get("audioActive").and_then(|value| parse_bool(value)).unwrap_or(false),
        playback_diagnostics: PlaybackDiagnostics {
            receiver_sample_rate: parse_optional_metric(fields.get("actualSampleRate")),
            receiver_channels: parse_optional_metric(fields.get("actualChannels")),
            udp_jitter_target_frames: parse_optional_metric(fields.get("udpJitterTargetFrames")),
            udp_queued_frames: parse_optional_metric(fields.get("udpQueuedFrames")),
            udp_expired_frames: parse_optional_metric(fields.get("udpExpiredFrames")),
            tcp_dropped_backlog_frames: parse_optional_metric(fields.get("tcpDroppedBacklogFrames")),
            track_pending_frames: parse_optional_metric(fields.get("trackPendingFrames")),
            underrun_count: parse_optional_metric(fields.get("underrunCount")),
            // USB uses the cumulative count above; the Provider's historical write-loop delta
            // is not the same interval as the desktop health-query cadence.
            underrun_increment: None,
            playback_advancing: fields.get("playbackAdvancing").and_then(|value| parse_bool(value)),
            requested_buffer_frames: parse_optional_metric(fields.get("requestedBufferFrames")),
            accepted_buffer_frames: parse_optional_metric(fields.get("acceptedBufferFrames")),
            effective_buffer_frames: parse_optional_metric(fields.get("effectiveBufferFrames")),
            capacity_frames: parse_optional_metric(fields.get("bufferCapacityFrames")),
            start_threshold_frames: parse_optional_metric(fields.get("startThresholdFrames")),
            pre_roll_frames: parse_optional_metric(fields.get("preRollFrames")),
            ..PlaybackDiagnostics::default()
        },
        error: fields.get("error").copied().unwrap_or("").to_string(),
    })
}

fn parse_bool(value: &str) -> Option<bool> {
    match value { "true" => Some(true), "false" => Some(false), _ => None }
}

fn parse_optional_metric<T: std::str::FromStr>(value: Option<&&str>) -> Option<T> {
    value.filter(|value| **value != "unknown").and_then(|value| value.parse().ok())
}

fn ensure_adb_success(result: &AdbResult) -> Result<(), String> {
    if result.success { Ok(()) } else { Err(adb_error(&result)) }
}

fn adb_error(result: &AdbResult) -> String {
    let output = format!("{} {}", result.stdout.trim(), result.stderr.trim()).trim().to_string();
    let lowered = output.to_ascii_lowercase();
    if lowered.contains("install_failed_update_incompatible") || lowered.contains("signature") {
        return "Android 拒绝更新：现有应用的签名不匹配。没有卸载或更改现有应用。".into();
    }
    if output.is_empty() { return "ADB 命令失败；没有移除现有应用。".into(); }
    let concise: String = output.chars().filter(|ch| *ch != '\r' && *ch != '\n').take(800).collect();
    format!("ADB 操作失败：{concise}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Mutex, time::SystemTime};

    const SERIAL: &str = "FAKE123";
    const TEST_USER: &str = "10";
    const INSTALLED_PATH: &str = "/data/app/~~AbCdEf0123==/app.anpod.receiver-xYz987==/base.apk";

    #[test]
    fn content_rows_preserve_multiple_interfaces_and_bootstrap_listener_endpoint() {
        let output = "Row: 0 address=192.168.1.20, prefixLength=24, interfaceIndex=3, interfaceName=wlan0\nRow: 1 address=192.168.43.1, prefixLength=24, interfaceIndex=7, interfaceName=ap0\n";
        let interfaces = parse_receiver_interfaces(output);
        assert_eq!(interfaces.len(), 2);
        assert_eq!(interfaces[1].address, "192.168.43.1");
        assert_eq!(interfaces[1].prefix_length, 24);

        let status = "Row: 1 networkAudioVersion=1, networkAudioState=authenticated, networkAudioFailure=, pcmBytesWritten=0, networkAudioAddress=192.168.43.1, networkAudioPort=41237";
        let endpoint = parse_network_status(status).unwrap();
        assert_eq!(endpoint.network_audio_version, 1);
        assert_eq!(endpoint.network_audio_address, "192.168.43.1");
        assert_eq!(endpoint.network_audio_port, 41237);
    }

    #[test]
    #[cfg(windows)]
    fn saved_binding_reference_isolated_by_usb_identity_and_android_user() {
        let first = binding_reference_for("USB-A", "0");
        assert_ne!(first, binding_reference_for("USB-B", "0"));
        assert_ne!(first, binding_reference_for("USB-A", "10"));
    }

    #[derive(Default)]
    struct FakeState {
        installed: bool,
        installed_hash: String,
        device_serial: String,
        user: &'static str,
        device_state: &'static str,
        usb: bool,
        install_fails: bool,
        install_digest: Option<String>,
        install_version_code: Option<u32>,
        install_version_name: Option<String>,
        health_version_code: u32,
        health_version_name: String,
        active: bool,
        foreground: bool,
        healthy: bool,
        audio_active: bool,
        network_audio_state: String,
        error: String,
        calls: Vec<Vec<String>>,
        serial_calls: Vec<Option<String>>,
        binding_commitment: Option<String>,
        binding_write_mode: FakeBindingWrite,
        binding_input_commitments: Vec<String>,
        binding_reset_supported: bool,
        binding_reset_noop: bool,
        binding_query_fail_after_delete: bool,
        binding_query_failed: bool,
    }

    #[derive(Clone, Copy, Default)]
    enum FakeBindingWrite {
        #[default]
        Store,
        Cancel,
        ErrorText,
        WrongUser,
    }

    #[derive(Default)]
    struct FakeAdb(Mutex<FakeState>);

    impl FakeAdb {
        fn new(installed: bool, installed_hash: String) -> Self {
            Self::with_serial(installed, installed_hash, SERIAL)
        }

        fn with_serial(installed: bool, installed_hash: String, device_serial: &str) -> Self {
            Self(Mutex::new(FakeState {
                installed,
                installed_hash,
                device_serial: device_serial.into(),
                user: TEST_USER,
                device_state: "device",
                usb: true,
                health_version_code: 1,
                health_version_name: "0.1.0-p1".into(),
                ..FakeState::default()
            }))
        }

        fn calls(&self) -> Vec<Vec<String>> { self.0.lock().unwrap().calls.clone() }
        fn serial_calls(&self) -> Vec<Option<String>> { self.0.lock().unwrap().serial_calls.clone() }

        fn call_index(&self, prefix: &[&str]) -> Option<usize> {
            self.0.lock().unwrap().calls.iter().position(|call|
                call.starts_with(&prefix.iter().map(|item| (*item).to_string()).collect::<Vec<_>>()))
        }
    }

    impl AdbRunner for FakeAdb {
        fn run<'a>(&'a self, serial: Option<&'a str>, args: &'a [&'a str], _limit: Duration,
            _cancel: Option<&'a CancellationToken>) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a {
            async move {
                let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
                let mut state = self.0.lock().unwrap();
                state.calls.push(args.clone());
                state.serial_calls.push(serial.map(str::to_owned));
                let result = match args.as_slice() {
                    [command, _] if command == "devices" => {
                        let transport = "product:test";
                        AdbResult { success: true, stdout: format!("List of devices attached\n{} {} {transport}\n", state.device_serial, state.device_state), stderr: String::new() }
                    }
                    [selector, command] if selector == "-d" && command == "get-serialno" => {
                        AdbResult { success: state.usb, stdout: if state.usb { format!("{}\n", state.device_serial) } else { String::new() }, stderr: String::new() }
                    }
                    [shell, am, command] if shell == "shell" && am == "am" && command == "get-current-user" => {
                        AdbResult { success: true, stdout: format!("{}\n", state.user), stderr: String::new() }
                    }
                    [shell, pm, list, packages, user_option, user, package] if shell == "shell" && pm == "pm" && list == "list" && packages == "packages" && user_option == "--user" && user == state.user && package == PACKAGE => {
                        let stdout = if state.installed { format!("package:{PACKAGE}\n") } else { String::new() };
                        AdbResult { success: true, stdout, stderr: String::new() }
                    }
                    [shell, pm, path, user_option, user, package] if shell == "shell" && pm == "pm" && path == "path" && user_option == "--user" && user == state.user && package == PACKAGE => {
                        let stdout = if state.installed { format!("package:{INSTALLED_PATH}\n") } else { String::new() };
                        AdbResult { success: true, stdout, stderr: String::new() }
                    }
                    [shell, toybox, sha256sum, _] if shell == "shell" && toybox == "toybox" && sha256sum == "sha256sum" => {
                        AdbResult { success: state.installed, stdout: format!("{}  {INSTALLED_PATH}\n", state.installed_hash), stderr: String::new() }
                    }
                    [shell, content, query, user_option, user, flag, uri] if shell == "shell" && content == "content" && query == "query" && user_option == "--user" && user == state.user && flag == "--uri" && uri == HEALTH_URI => {
                            let stdout = format!("Row: 0 applicationId={PACKAGE}, versionName={}, versionCode={}, serviceActive={}, foreground={}, healthy={}, audioActive={}, startedAtElapsedMs=1, error={}\n", state.health_version_name, state.health_version_code, state.active, state.foreground, state.healthy, state.audio_active, state.error);
                            AdbResult { success: true, stdout, stderr: String::new() }
                        }
                    [shell, content, query, user_option, user, flag, uri, projection_flag, projection]
                        if shell == "shell" && content == "content" && query == "query"
                            && user_option == "--user" && user == state.user && flag == "--uri"
                            && uri == BINDING_URI && projection_flag == "--projection"
                            && projection == "bindingState:bindingCommitment:bindingFailure" => {
                        if state.binding_query_failed {
                            return Ok(AdbResult { success: false, stdout: String::new(), stderr: "provider unavailable".into() });
                        }
                        let (binding_state, commitment) = match &state.binding_commitment {
                            Some(commitment) => ("saved", commitment.as_str()),
                            None => ("empty", ""),
                        };
                        AdbResult { success: true,
                            stdout: format!("Row: 0 bindingState={binding_state}, bindingCommitment={commitment}, bindingFailure=\n"),
                            stderr: String::new() }
                    }
                    [shell, content, query, user_option, user, flag, uri, projection_flag, projection]
                        if shell == "shell" && content == "content" && query == "query"
                            && user_option == "--user" && user == state.user && flag == "--uri"
                            && uri == BINDING_URI && projection_flag == "--projection"
                            && projection == "bindingResetVersion" => {
                        AdbResult { success: state.binding_reset_supported,
                            stdout: if state.binding_reset_supported { "Row: 0 bindingResetVersion=1\n".into() } else { String::new() },
                            stderr: String::new() }
                    }
                    [shell, content, query, user_option, user, flag, uri, projection_flag, projection]
                        if shell == "shell" && content == "content" && query == "query"
                            && user_option == "--user" && user == state.user && flag == "--uri"
                            && uri == NETWORK_URI && projection_flag == "--projection"
                            && projection == "networkAudioVersion:networkAudioState" => {
                        AdbResult { success: true,
                            stdout: format!("Row: 0 networkAudioVersion=1, networkAudioState={}\n", state.network_audio_state),
                            stderr: String::new() }
                    }
                    [shell, content, delete, user_option, user, uri_option, uri, where_option, selection]
                        if shell == "shell" && content == "content" && delete == "delete"
                            && user_option == "--user" && user == state.user && uri_option == "--uri"
                            && uri == BINDING_URI && where_option == "--where" => {
                        let matches = match (selection.strip_prefix("expectedCommitment="), selection.as_str()) {
                            (Some(expected), _) => state.binding_commitment.as_deref() == Some(expected),
                            (None, "expectedEmpty") => state.binding_commitment.is_none(),
                            _ => false,
                        };
                        if matches && !state.binding_reset_noop { state.binding_commitment = None; }
                        if matches && state.binding_query_fail_after_delete { state.binding_query_failed = true; }
                        // content may return success even when the provider did not change the row.
                        AdbResult { success: true, stdout: "0 rows deleted\n".into(), stderr: String::new() }
                    }
                    [install, replace, user_option, user, _apk] if install == "install" && replace == "-r" && user_option == "--user" && user == state.user => {
                        if state.install_fails {
                            AdbResult { success: false, stdout: String::new(), stderr: "INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match".into() }
                        } else {
                            state.installed = true;
                            if let Some(digest) = state.install_digest.take() { state.installed_hash = digest; }
                            if let Some(version_code) = state.install_version_code.take() { state.health_version_code = version_code; }
                            if let Some(version_name) = state.install_version_name.take() { state.health_version_name = version_name; }
                            AdbResult { success: true, stdout: "Success".into(), stderr: String::new() }
                        }
                    }
                    values if values.windows(2).any(|pair| pair[0] == "-a" && pair[1] == "app.anpod.receiver.action.START") => {
                        state.active = true;
                        state.foreground = true;
                        state.healthy = true;
                        AdbResult { success: true, stdout: "Status: ok".into(), stderr: String::new() }
                    }
                    values if values.windows(2).any(|pair| pair[0] == "-a" && pair[1] == "app.anpod.receiver.action.START_AUDIO") => {
                        state.active = true;
                        state.foreground = true;
                        state.healthy = true;
                        AdbResult { success: true, stdout: "Status: ok".into(), stderr: String::new() }
                    }
                    values if values.windows(2).any(|pair| pair[0] == "-a" && pair[1] == "app.anpod.receiver.action.STOP") => {
                        state.active = false;
                        state.foreground = false;
                        state.healthy = false;
                        AdbResult { success: true, stdout: "Status: ok".into(), stderr: String::new() }
                    }
                    _ => AdbResult { success: false, stdout: String::new(), stderr: format!("unexpected fake ADB args: {args:?}") },
                };
                Ok(result)
            }
        }

        fn run_with_stdin<'a>(&'a self, serial: Option<&'a str>, args: &'a [&'a str], input: &'a [u8],
            _limit: Duration, cancel: &'a CancellationToken) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a {
            async move {
                let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
                let mut state = self.0.lock().unwrap();
                state.calls.push(args.clone());
                state.serial_calls.push(serial.map(str::to_owned));
                let target_matches = args.len() == 8 && args[0] == "shell" && args[1] == "-T"
                    && args[2] == "content" && args[3] == "write" && args[4] == "--user"
                    && args[5] == state.user && args[6] == "--uri" && args[7] == BINDING_URI;
                if serial != Some(SERIAL) || !target_matches {
                    return Ok(AdbResult { success: false, stdout: String::new(), stderr: "invalid fake provision target".into() });
                }
                let commitment = format!("{:x}", Sha256::digest(input));
                state.binding_input_commitments.push(commitment.clone());
                match state.binding_write_mode {
                    FakeBindingWrite::Store => {
                        if state.binding_commitment.as_deref().is_none_or(|existing| existing == commitment) {
                            state.binding_commitment = Some(commitment);
                        }
                        Ok(AdbResult { success: true, stdout: String::new(), stderr: String::new() })
                    }
                    FakeBindingWrite::Cancel => {
                        state.binding_write_mode = FakeBindingWrite::Store;
                        cancel.cancel();
                        Err("操作已取消。".into())
                    }
                    FakeBindingWrite::ErrorText => Ok(AdbResult { success: true,
                        stdout: String::new(), stderr: format!("Error while accessing provider: {}",
                            input[37..69].iter().map(|byte| format!("{byte:02x}")).collect::<String>()) }),
                    FakeBindingWrite::WrongUser => {
                        state.user = "11";
                        Ok(AdbResult { success: true, stdout: String::new(), stderr: String::new() })
                    }
                }
            }
        }
    }

    struct ArtifactFiles { dir: std::path::PathBuf, apk: std::path::PathBuf, metadata: std::path::PathBuf, digest: String }

    impl ArtifactFiles {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::current_dir().unwrap().join("target").join(format!("anpod-receiver-test-{}-{unique}-{:?}", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
            fs::create_dir_all(&dir).unwrap();
            let apk = dir.join("receiver.apk");
            let metadata = dir.join("receiver.json");
            let bytes = b"verified fake receiver apk";
            fs::write(&apk, bytes).unwrap();
            let digest = format!("{:x}", Sha256::digest(bytes));
            fs::write(&metadata, serde_json::json!({
                "applicationId": PACKAGE, "versionCode": 1, "versionName": "0.1.0-p1",
                "sha256": digest, "signerSha256": "a".repeat(64)
            }).to_string()).unwrap();
            Self { dir, apk, metadata, digest }
        }
    }

    impl Drop for ArtifactFiles { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.dir); } }

    #[cfg(windows)]
    fn binding_reset_fixture(files: &ArtifactFiles, phone: Option<&crate::security::BindingMaterial>,
        pc: Option<&crate::security::BindingMaterial>) -> (FakeAdb, std::path::PathBuf, String) {
        use crate::security::write_test_binding_file;
        let data_dir = files.dir.join("profile");
        let reference = binding_reference_for(SERIAL, TEST_USER);
        let binding_dir = data_dir.join("receiver-bindings");
        fs::create_dir_all(&binding_dir).unwrap();
        if let Some(material) = pc {
            write_test_binding_file(&binding_dir.join(format!("{reference}.dat")), material).unwrap();
        }
        let adb = FakeAdb::new(true, files.digest.clone());
        {
            let mut state = adb.0.lock().unwrap();
            state.binding_reset_supported = true;
            state.network_audio_state = "stopped".into();
            state.binding_commitment = phone.map(|material| hex_digest(&material.commitment()));
        }
        (adb, data_dir, reference)
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn binding_reset_clears_only_the_confirmed_phone_and_pc_target() {
        use crate::security::{test_binding_material, test_binding_material_with_wrong_secret, write_test_binding_file};

        for (phone_present, pc_present, conflict) in [(false, false, false), (false, true, false),
                (true, false, false), (true, true, true)] {
            let files = ArtifactFiles::new();
            let phone = test_binding_material();
            let pc = if conflict { test_binding_material_with_wrong_secret() } else { test_binding_material() };
            let other = test_binding_material_with_wrong_secret();
            let (adb, data_dir, reference) = binding_reset_fixture(&files,
                phone_present.then_some(&phone), pc_present.then_some(&pc));
            let binding_dir = data_dir.join("receiver-bindings");
            let other_path = binding_dir.join(format!("{}.dat", "b".repeat(64)));
            write_test_binding_file(&other_path, &other).unwrap();

            let preview = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
                &CancellationToken::new()).await.unwrap();
            assert_eq!(preview.binding_reference, reference);
            let result = reset_binding_with(&adb, &preview, &data_dir, &files.apk, &files.metadata,
                &CancellationToken::new(), &|| true).await.unwrap();
            assert_eq!(result.outcome, if phone_present || pc_present { "reset" } else { "alreadyEmpty" });
            assert_eq!(adb.0.lock().unwrap().binding_commitment, None);
            assert!(other_path.exists(), "an unrelated PC binding must survive reset");
            assert!(!binding_dir.join(format!("{reference}.dat")).exists());
            let deletes = adb.calls().iter().filter(|call| call.get(2).map(String::as_str) == Some("delete")).count();
            assert_eq!(deletes, usize::from(phone_present || pc_present), "both-empty reset is read-only");
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn binding_reset_requires_capability_identity_current_commitments_and_idle_receiver() {
        use crate::security::test_binding_material;
        let files = ArtifactFiles::new();
        let material = test_binding_material();
        let (adb, data_dir, reference) = binding_reset_fixture(&files, Some(&material), Some(&material));
        let binding_path = data_dir.join("receiver-bindings").join(format!("{reference}.dat"));

        adb.0.lock().unwrap().binding_reset_supported = false;
        let unsupported = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new()).await.unwrap_err();
        assert!(unsupported.contains("不支持条件绑定重置"));
        assert!(adb.0.lock().unwrap().binding_commitment.is_some());
        assert!(binding_path.exists());

        adb.0.lock().unwrap().binding_reset_supported = true;
        let preview = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new()).await.unwrap();
        adb.0.lock().unwrap().device_state = "unauthorized";
        assert!(reset_binding_with(&adb, &preview, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new(), &|| true).await.is_err());
        assert!(!adb.calls().iter().any(|call| call.get(2).map(String::as_str) == Some("delete")));
        assert!(binding_path.exists());

        adb.0.lock().unwrap().device_state = "device";
        let preview = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new()).await.unwrap();
        adb.0.lock().unwrap().user = "11";
        assert!(reset_binding_with(&adb, &preview, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new(), &|| true).await.is_err());
        assert!(binding_path.exists());
        adb.0.lock().unwrap().user = TEST_USER;

        let preview = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new()).await.unwrap();
        adb.0.lock().unwrap().binding_commitment = Some("f".repeat(64));
        assert!(reset_binding_with(&adb, &preview, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new(), &|| true).await.is_err());
        assert!(binding_path.exists());

        adb.0.lock().unwrap().binding_commitment = Some(hex_digest(&material.commitment()));
        adb.0.lock().unwrap().network_audio_state = "streaming".into();
        let busy = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new()).await.unwrap();
        assert!(busy.receiver_busy);
        assert!(reset_binding_with(&adb, &busy, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new(), &|| true).await.is_err());
        assert_eq!(adb.calls().iter().filter(|call| call.get(2).map(String::as_str) == Some("delete")).count(), 0);
        assert!(binding_path.exists());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn binding_reset_reads_back_exit_zero_and_keeps_a_recoverable_partial_result() {
        use crate::security::{test_binding_material, test_binding_material_with_wrong_secret, write_test_binding_file};
        let files = ArtifactFiles::new();
        let material = test_binding_material();
        let (adb, data_dir, reference) = binding_reset_fixture(&files, Some(&material), Some(&material));
        let binding_path = data_dir.join("receiver-bindings").join(format!("{reference}.dat"));
        let preview = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new()).await.unwrap();

        adb.0.lock().unwrap().binding_reset_noop = true;
        let retained = reset_binding_with(&adb, &preview, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new(), &|| true).await.unwrap();
        assert_eq!(retained.outcome, "phoneRetained");
        assert_eq!(retained.phone_state, "saved");
        assert!(binding_path.exists());

        adb.0.lock().unwrap().binding_reset_noop = false;
        let preview = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new()).await.unwrap();
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = reset_binding_with(&adb, &preview, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new(), &|| checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0).await.unwrap();
        assert_eq!(cancelled.outcome, "phoneClearedPcRetained");
        assert_eq!(cancelled.phone_state, "empty");
        assert_eq!(cancelled.pc_state, "saved");
        assert!(binding_path.exists(), "cancellation after phone removal must preserve the DPAPI retry copy");
        assert_eq!(adb.0.lock().unwrap().binding_commitment, None);

        let files = ArtifactFiles::new();
        let material = test_binding_material();
        let (adb, data_dir, reference) = binding_reset_fixture(&files, Some(&material), Some(&material));
        let binding_path = data_dir.join("receiver-bindings").join(format!("{reference}.dat"));
        adb.0.lock().unwrap().binding_query_fail_after_delete = true;
        let preview = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new()).await.unwrap();
        let unknown = reset_binding_with(&adb, &preview, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new(), &|| true).await.unwrap();
        assert_eq!(unknown.outcome, "phoneStateUnknown");
        assert_eq!(unknown.phone_state, "unknown");
        assert_eq!(unknown.pc_state, "saved");
        assert!(binding_path.exists(), "failed phone readback must leave the local retry copy intact");
        assert_eq!(adb.0.lock().unwrap().binding_commitment, None,
            "the fake phone did clear the binding although readback became unavailable");

        let files = ArtifactFiles::new();
        let material = test_binding_material();
        let changed_material = test_binding_material_with_wrong_secret();
        let (adb, data_dir, reference) = binding_reset_fixture(&files, Some(&material), Some(&material));
        let binding_path = data_dir.join("receiver-bindings").join(format!("{reference}.dat"));
        let preview = binding_reset_preview_with(&adb, SERIAL, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new()).await.unwrap();
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let failed = reset_binding_with(&adb, &preview, &data_dir, &files.apk, &files.metadata,
            &CancellationToken::new(), &|| {
                if checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 2 {
                    write_test_binding_file(&binding_path, &changed_material).unwrap();
                }
                true
            }).await.unwrap();
        assert_eq!(failed.outcome, "phoneClearedPcChanged");
        assert_eq!(failed.phone_state, "empty");
        assert_eq!(failed.pc_state, "saved");
        assert_eq!(failed.pc_commitment, Some(hex_digest(&changed_material.commitment())),
            "a failed PC CAS must report the current record commitment, not the stale preview");
        assert!(binding_path.exists());
        assert_eq!(adb.0.lock().unwrap().binding_commitment, None);
    }

    #[test]
    fn parses_real_service_health_record() {
        let row = "Row: 0 applicationId=app.anpod.receiver, versionName=0.1.0-p1, versionCode=1, serviceActive=true, foreground=true, healthy=true, startedAtElapsedMs=123, error=";
        let health = parse_health(row).unwrap();
        assert_eq!(health.application_id, PACKAGE);
        assert_eq!(health.version_code, 1);
        assert!(health.service_active && health.foreground && health.healthy);
    }

    #[test]
    fn installed_and_not_installed_package_responses_are_distinct() {
        assert_eq!(parse_installed_apk_path("package:/data/app/~~AbCdEf0123==/app.anpod.receiver-xYz987==/base.apk\n"), Some("/data/app/~~AbCdEf0123==/app.anpod.receiver-xYz987==/base.apk"));
        assert_eq!(parse_installed_apk_path("package:/data/app/~~opaque/app.anpod.receiver/base.apk with space\n"), None);
        assert_eq!(parse_installed_apk_path("package:/data/app/~~opaque/app.anpod.receiver/base.apk;id\n"), None);
        assert_eq!(parse_installed_apk_path("package:/data/app/a/base.apk\npackage:/data/app/a/split.apk\n"), None);
        assert_eq!(parse_installed_apk_path("\n"), None);
    }

    #[test]
    fn reads_the_artifact_written_by_windows_powershell_when_requested() {
        let Some(directory) = std::env::var_os("ANPOD_RECEIVER_ARTIFACT_DIR") else { return; };
        let directory = Path::new(&directory);
        read_artifact(&directory.join("receiver.apk"), &directory.join("receiver.json")).unwrap();
    }

    #[test]
    fn receiver_identity_compatibility_and_update_decisions_are_independent() {
        let current = ArtifactMetadata {
            application_id: PACKAGE.into(),
            version_code: 3,
            version_name: "0.3.0-v2-candidate".into(),
            sha256: "22af84b1989011d3cd214ff321ccd37bc4fe4de03546850f65d5db778f49d8c9".into(),
            signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439".into(),
            source_commit: Some("608c98c75ec04e11555e9e547caa516c96d4e0dd".into()),
        };
        let old = verified_installed_artifact(
            "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050", &current,
        ).unwrap();
        assert_eq!(old.version_code, 3);
        assert_eq!(old.source_commit.as_deref(), Some("81dda7630e2728881d4aa556698ba8f6bb36c453"));
        let installed_v3 = verified_installed_artifact(
            "b52a326f234ab84d6aba9b5ffbb840e75f90a101cb5479461da7a53d184a4abb", &current,
        ).unwrap();
        assert_eq!(installed_v3.version_code, 4);
        assert_eq!(installed_v3.source_commit.as_deref(), Some("8f868708a83a32973be902fabf7dc8357f3c55a6"));
        let installed_v4_seed = verified_installed_artifact(
            "c514dfd555aa2734178131db47037875cf1086d934b45639b799471297668be3", &current,
        ).unwrap();
        assert_eq!(installed_v4_seed.version_code, 6);
        assert_eq!(installed_v4_seed.version_name, "0.5.0-v4-candidate");
        assert_eq!(installed_v4_seed.source_commit.as_deref(), Some("20f2389af0526ebdbc20924cad42570ce51110ce"));
        for (digest, code, name) in [
            ("344c47ddbd51b991ba4562e09c11c195815f7f363f9d1196146263df929fa1b7", 10, "0.8.0-v3-flow-candidate"),
            ("c66a6a350907ab2581e941665b9984c47632777855717fe414964a2177baee41", 11, "0.9.0-v3-flow-candidate"),
        ] {
            let installed = verified_installed_artifact(digest, &current).unwrap();
            assert_eq!((installed.version_code, installed.version_name.as_str()), (code, name));
            assert_eq!(installed.source_commit.as_deref(), Some("61f32f7798f29bb39cf256997de50992a85ea519"));
        }
        assert!(verified_installed_artifact(&"e".repeat(64), &current).is_none(),
            "a claimed package/version/certificate cannot replace an audited installed APK digest");
        let p1_target = ArtifactMetadata {
            application_id: PACKAGE.into(),
            version_code: 1,
            version_name: "0.1.0-p1".into(),
            sha256: "b32eeaf06d40372c4a243eadf3b1c48067875af44f9e3a7b1d23d67628004f62".into(),
            signer_sha256: "d".repeat(64),
            source_commit: None,
        };
        assert!(verified_installed_artifact(&old.sha256, &p1_target).is_none(),
            "a different signer cannot authorize a historical v2 identity");

        let older_target = ArtifactMetadata {
            application_id: PACKAGE.into(),
            version_code: 2,
            version_name: "0.2.0-p2".into(),
            sha256: "b".repeat(64),
            signer_sha256: old.signer_sha256.clone(),
            source_commit: None,
        };
        let trusted_newer_receiver = verified_installed_artifact(&old.sha256, &older_target).unwrap();
        let no_downgrade = decide_receiver_compatibility(
            Some(&trusted_newer_receiver), true, None, CAP_AUTHENTICATED_STATUS, Some(&older_target),
        );
        assert!(no_downgrade.identity_trusted);
        assert_eq!(no_downgrade.update, UpdateAvailability::Unavailable);

        let target = ArtifactMetadata {
            application_id: PACKAGE.into(),
            version_code: 5,
            version_name: "0.4.1-v3-candidate".into(),
            sha256: "a".repeat(64),
            signer_sha256: old.signer_sha256.clone(),
            source_commit: Some("b".repeat(40)),
        };
        let capabilities = ReceiverCapabilities {
            control_major: V3_CONTROL_MAJOR,
            audio_major: PCM_AUDIO_MAJOR,
            bits: CAP_RECEIVER_UI | CAP_AUTHENTICATED_STATUS | CAP_PLAYBACK_PROGRESS,
        };
        let no_same_version_update = decide_receiver_compatibility(
            Some(&installed_v3), true, Some(capabilities), CAP_AUTHENTICATED_STATUS,
            Some(&ArtifactMetadata { version_code: 4, version_name: "0.4.0-v3-candidate".into(), ..target.clone() }),
        );
        assert_eq!(no_same_version_update.update, UpdateAvailability::Unavailable);
        let compatible = decide_receiver_compatibility(
            Some(&old), true, Some(capabilities), CAP_AUTHENTICATED_STATUS, Some(&target),
        );
        assert_eq!(compatible, ReceiverCompatibilityDecision {
            identity_trusted: true,
            features: FeatureCompatibility::Compatible,
            update: UpdateAvailability::CandidateAvailable,
            usb_upgrade_available: true,
            management: ManagementAccess::CompatibleSession,
        });

        let incompatible = decide_receiver_compatibility(
            Some(&old), true, Some(ReceiverCapabilities { bits: 0, ..capabilities }),
            CAP_AUTHENTICATED_STATUS, Some(&target),
        );
        assert_eq!(incompatible.features, FeatureCompatibility::Incompatible);
        assert_eq!(incompatible.update, UpdateAvailability::CandidateAvailable);
        assert!(incompatible.usb_upgrade_available);
        assert_eq!(incompatible.management, ManagementAccess::PreflightAndUpdateOnly);

        let unnegotiated = decide_receiver_compatibility(
            Some(&old), true, None, CAP_AUTHENTICATED_STATUS, Some(&target),
        );
        assert_eq!(unnegotiated.update, UpdateAvailability::CandidateAvailable);
        assert!(unnegotiated.usb_upgrade_available);
        assert_eq!(unnegotiated.management, ManagementAccess::PreflightAndUpdateOnly);

        let old_management = decide_receiver_compatibility(
            Some(&old), true, Some(ReceiverCapabilities {
                control_major: V3_CONTROL_MAJOR - 1,
                audio_major: PCM_AUDIO_MAJOR,
                bits: 0,
            }), CAP_AUTHENTICATED_STATUS, Some(&target),
        );
        assert!(old_management.usb_upgrade_available);
        assert_eq!(old_management.management, ManagementAccess::PreflightAndUpdateOnly);

        let no_update_bit = decide_receiver_compatibility(
            Some(&old), true, Some(ReceiverCapabilities {
                bits: CAP_AUTHENTICATED_STATUS, ..capabilities
            }), CAP_AUTHENTICATED_STATUS, Some(&target),
        );
        assert!(no_update_bit.usb_upgrade_available);
        assert_eq!(no_update_bit.management, ManagementAccess::CompatibleSession);

        let audio_incompatible = decide_receiver_compatibility(
            Some(&old), true, Some(ReceiverCapabilities {
                audio_major: PCM_AUDIO_MAJOR + 1, ..capabilities
            }), CAP_AUTHENTICATED_STATUS, Some(&target),
        );
        assert_eq!(audio_incompatible.features, FeatureCompatibility::Incompatible);
        assert!(audio_incompatible.usb_upgrade_available);
        assert_eq!(audio_incompatible.update, UpdateAvailability::CandidateAvailable);
        assert_eq!(audio_incompatible.management, ManagementAccess::PreflightAndUpdateOnly);

        let unauthenticated = decide_receiver_compatibility(
            Some(&old), false, Some(capabilities), CAP_AUTHENTICATED_STATUS, Some(&target),
        );
        assert_eq!(unauthenticated.features, FeatureCompatibility::NotNegotiated);
        assert!(unauthenticated.usb_upgrade_available);
        assert_eq!(unauthenticated.management, ManagementAccess::Denied);

        let wrong_signer = ArtifactMetadata { signer_sha256: "c".repeat(64), ..target.clone() };
        let blocked_update = decide_receiver_compatibility(
            Some(&old), true, Some(capabilities), CAP_AUTHENTICATED_STATUS, Some(&wrong_signer),
        );
        assert_eq!(blocked_update.features, FeatureCompatibility::Compatible);
        assert_eq!(blocked_update.update, UpdateAvailability::BlockedBySignature);
        assert!(!blocked_update.usb_upgrade_available);

        let unknown_identity = decide_receiver_compatibility(
            None, true, Some(capabilities), CAP_AUTHENTICATED_STATUS, Some(&target),
        );
        assert!(!unknown_identity.identity_trusted);
        assert!(!unknown_identity.usb_upgrade_available);
        assert_eq!(unknown_identity.management, ManagementAccess::Denied);
        assert_eq!(unknown_identity.update, UpdateAvailability::Unavailable);
    }

    #[test]
    fn signature_mismatch_is_reported_without_an_uninstall_fallback() {
        let result = AdbResult { success: false, stdout: "Performing Streamed Install".into(), stderr: "INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match".into() };
        let error = adb_error(&result);
        assert!(error.contains("签名不匹配"));
        assert!(error.contains("没有卸载"));
    }

    #[test]
    fn health_parser_rejects_incomplete_or_non_health_rows() {
        assert!(parse_health("No result found.").is_none());
        assert!(parse_health("Row: 0 applicationId=app.anpod.receiver, healthy=true").is_none());
    }

    #[test]
    fn health_parser_keeps_unknown_playback_metrics_null() {
        let health = parse_health("Row: 0 applicationId=app.anpod.receiver, versionName=0.1.0-p1, versionCode=1, serviceActive=true, foreground=true, healthy=true, audioActive=true, actualSampleRate=48000, actualChannels=2, udpQueuedFrames=unknown, udpExpiredFrames=17, underrunCount=10, underrunIncrement=2, playbackAdvancing=true, error=").unwrap();
        assert_eq!(health.playback_diagnostics.receiver_sample_rate, Some(48_000));
        assert_eq!(health.playback_diagnostics.receiver_channels, Some(2));
        assert_eq!(health.playback_diagnostics.udp_queued_frames, None);
        assert_eq!(health.playback_diagnostics.udp_expired_frames, Some(17));
        assert_eq!(health.playback_diagnostics.underrun_count, Some(10));
        assert_eq!(health.playback_diagnostics.underrun_increment, None);
        assert_eq!(health.playback_diagnostics.playback_advancing, Some(true));
    }

    #[test]
    fn surfaces_android_startup_failure_instead_of_reporting_process_health() {
        let health = parse_health("Row: 0 applicationId=app.anpod.receiver, versionName=0.1.0-p1, versionCode=1, serviceActive=false, foreground=false, healthy=false, startedAtElapsedMs=0, error=ForegroundServiceStartNotAllowedException").unwrap();
        let status = status_from_health(health);
        assert!(!status.healthy);
        assert!(status.message.contains("ForegroundServiceStartNotAllowedException"));
    }

    #[tokio::test]
    async fn fake_adb_normal_install_health_start_stop_flow_checks_installed_apk_each_time() {
        let files = ArtifactFiles::new();
        let adb = FakeAdb::new(false, files.digest.clone());
        let cancel = CancellationToken::new();
        assert!(!install_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel).await.unwrap().healthy);
        assert!(!health_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel).await.unwrap().healthy);
        assert!(start_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel).await.unwrap().healthy);
        assert!(!stop_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel).await.unwrap().service_active);
        let calls = adb.calls();
        let install = adb.call_index(&["install", "-r"]).unwrap();
        let first_hash = adb.call_index(&["shell", "toybox", "sha256sum"]).unwrap();
        let first_health = adb.call_index(&["shell", "content", "query"]).unwrap();
        let start = calls.iter().position(|call| call.iter().any(|arg| arg == "app.anpod.receiver.action.START")).unwrap();
        let stop = calls.iter().position(|call| call.iter().any(|arg| arg == "app.anpod.receiver.action.STOP")).unwrap();
        assert!(install < first_hash && first_hash < first_health && first_health < start && start < stop);
        assert!(calls.iter().filter(|call| call.starts_with(&["shell".into(), "toybox".into(), "sha256sum".into()])).count() >= 4);
        for call in &calls {
            if call.starts_with(&["shell".into(), "pm".into(), "list".into()])
                || call.starts_with(&["shell".into(), "pm".into(), "path".into()])
                || call.starts_with(&["shell".into(), "content".into(), "query".into()])
                || call.starts_with(&["shell".into(), "am".into(), "start".into()])
                || call.starts_with(&["install".into()]) {
                assert!(call.windows(2).any(|pair| pair[0] == "--user" && pair[1] == TEST_USER), "wrong Android user in {call:?}");
            }
        }
        assert!(calls.iter().any(|call| call.starts_with(&["shell".into(), "content".into(), "query".into(), "--user".into(), TEST_USER.into()])));
    }

    #[tokio::test]
    async fn fake_adb_wireless_receiver_start_accepts_audited_newer_install() {
        let files = ArtifactFiles::new();
        fs::write(&files.metadata, serde_json::json!({
            "applicationId": PACKAGE,
            "versionCode": 2,
            "versionName": "0.2.0-p2-candidate",
            "sha256": files.digest,
            "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439"
        }).to_string()).unwrap();
        let serial = "192.0.2.20:5555";
        let adb = FakeAdb::with_serial(true,
            "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into(), serial);
        {
            let mut state = adb.0.lock().unwrap();
            state.health_version_code = 3;
            state.health_version_name = "0.3.0-v2-candidate".into();
        }
        let status = start_with_action(&adb, serial, &files.apk, &files.metadata,
            &CancellationToken::new(), Some(("wireless-session", "test-code", "balanced"))).await.unwrap();
        assert_eq!(status.version_code, Some(3));
        assert!(adb.calls().iter().any(|call|
            call.iter().any(|arg| arg == "app.anpod.receiver.action.START_AUDIO")));
    }

    #[tokio::test]
    async fn fake_adb_install_refuses_older_local_receiver_before_install() {
        let files = ArtifactFiles::new();
        fs::write(&files.metadata, serde_json::json!({
            "applicationId": PACKAGE,
            "versionCode": 2,
            "versionName": "0.2.0-p2-candidate",
            "sha256": files.digest,
            "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439"
        }).to_string()).unwrap();
        let adb = FakeAdb::new(true,
            "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into());
        let error = install_with(&adb, SERIAL, &files.apk, &files.metadata,
            &CancellationToken::new()).await.err().unwrap();
        assert!(error.contains("已阻止降级"));
        assert!(adb.call_index(&["shell", "toybox", "sha256sum"]).is_some());
        assert!(adb.call_index(&["install", "-r"]).is_none());
    }

    #[tokio::test]
    async fn fake_adb_installs_a_verified_newer_candidate_over_an_existing_receiver() {
        for old_digest in [
            "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050",
            "344c47ddbd51b991ba4562e09c11c195815f7f363f9d1196146263df929fa1b7",
            "c66a6a350907ab2581e941665b9984c47632777855717fe414964a2177baee41",
        ] {
        let files = ArtifactFiles::new();
        let version_name = "0.9.1-v3-flow-candidate";
        fs::write(&files.metadata, serde_json::json!({
            "applicationId": PACKAGE,
            "versionCode": 12,
            "versionName": version_name,
            "sha256": files.digest,
            "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439"
        }).to_string()).unwrap();
        let adb = FakeAdb::new(true, old_digest.into());
        {
            let mut state = adb.0.lock().unwrap();
            state.install_digest = Some(files.digest.clone());
            state.install_version_code = Some(12);
            state.install_version_name = Some(version_name.into());
        }

        let status = install_with(&adb, SERIAL, &files.apk, &files.metadata, &CancellationToken::new()).await.unwrap();

        assert_eq!(status.version_code, Some(12));
        assert_eq!(status.version_name.as_deref(), Some(version_name));
        assert!(adb.call_index(&["install", "-r"]).is_some());
        assert!(!adb.calls().iter().any(|call| call.first().is_some_and(|arg| arg == "uninstall")));
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn fake_adb_usb_upgrade_requires_the_saved_binding_and_preserves_it() {
        use crate::security::{test_binding_material, BindingStore};

        let files = ArtifactFiles::new();
        let version_name = "0.7.0-v3-flow-candidate";
        fs::write(&files.metadata, serde_json::json!({
            "applicationId": PACKAGE,
            "versionCode": 9,
            "versionName": version_name,
            "sha256": files.digest,
            "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439"
        }).to_string()).unwrap();
        let reference = binding_reference_for(SERIAL, TEST_USER);
        let binding_path = files.dir.join(format!("{reference}.dpapi"));
        let material = test_binding_material();
        let commitment = hex_digest(&material.commitment());
        BindingStore::create_if_absent(&binding_path, &material).unwrap();

        let adb = FakeAdb::new(true,
            "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into());
        {
            let mut state = adb.0.lock().unwrap();
            state.binding_commitment = Some(commitment.clone());
            state.install_digest = Some(files.digest.clone());
            state.install_version_code = Some(9);
            state.install_version_name = Some(version_name.into());
        }
        let status = install_usb_with(&adb, SERIAL, &files.apk, &files.metadata,
            Some((&reference, &binding_path)), &CancellationToken::new()).await.unwrap();
        assert_eq!(status.version_code, Some(9));
        assert_eq!(adb.0.lock().unwrap().binding_commitment.as_deref(), Some(commitment.as_str()));
        assert!(adb.call_index(&["install", "-r"]).is_some());
        let calls = adb.calls();
        let binding_queries: Vec<_> = calls.iter().enumerate().filter(|(_, call)|
            call.windows(2).any(|pair| pair[0] == "--uri" && pair[1] == BINDING_URI)).map(|(index, _)| index).collect();
        assert!(binding_queries.len() >= 2, "binding must match both before and after package installation");
        assert!(calls.iter().all(|call| !call.windows(2).any(|pair| pair[0] == "content" && pair[1] == "write")));
        assert!(adb.serial_calls().iter().any(|serial| serial.as_deref() == Some(SERIAL)));
        assert!(adb.serial_calls().iter().all(|serial| serial.as_deref().is_none_or(|serial| serial == SERIAL)));

        for remote_commitment in [Some("f".repeat(64)), None] {
            let blocked = FakeAdb::new(true,
                "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into());
            blocked.0.lock().unwrap().binding_commitment = remote_commitment;
            let error = install_usb_with(&blocked, SERIAL, &files.apk, &files.metadata,
                Some((&reference, &binding_path)), &CancellationToken::new()).await.err().unwrap();
            assert!(error.contains("绑定"));
            let calls = blocked.calls();
            assert!(!calls.iter().any(|call| call.first().is_some_and(|arg| arg == "install")));
            assert!(!calls.iter().any(|call| call.windows(2).any(|pair| pair[0] == "content" && pair[1] == "write")));
        }

        let changed_user = FakeAdb::new(true,
            "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into());
        changed_user.0.lock().unwrap().user = "11";
        assert!(install_usb_with(&changed_user, SERIAL, &files.apk, &files.metadata,
            Some((&reference, &binding_path)), &CancellationToken::new()).await.is_err());
        assert!(!changed_user.calls().iter().any(|call| call.first().is_some_and(|arg| arg == "install")));
        assert!(!changed_user.calls().iter().any(|call| call.windows(2).any(|pair| pair[0] == "content" && pair[1] == "write")));

        let wireless_serial = "192.0.2.15:5555";
        let wireless = FakeAdb::with_serial(true,
            "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into(), wireless_serial);
        wireless.0.lock().unwrap().usb = false;
        assert!(install_usb_with(&wireless, wireless_serial, &files.apk, &files.metadata,
            Some((&reference, &binding_path)), &CancellationToken::new()).await.is_err());
        assert!(!wireless.calls().iter().any(|call| call.first().is_some_and(|arg| arg == "install")));
        assert!(!wireless.calls().iter().any(|call| call.windows(2).any(|pair| pair[0] == "content" && pair[1] == "write")));
    }

    #[tokio::test]
    async fn fake_adb_audio_start_sends_session_handshake_and_playback_mode() {
        let files = ArtifactFiles::new();
        let adb = FakeAdb::new(true, files.digest.clone());
        let status = start_with_action(&adb, SERIAL, &files.apk, &files.metadata, &CancellationToken::new(),
            Some(("anpod_test_session", "test_token", "lowlatency"))).await.unwrap();
        assert!(status.healthy);
        let audio_start = adb.calls().into_iter().find(|call|
            call.iter().any(|arg| arg == "app.anpod.receiver.action.START_AUDIO")).unwrap();
        for expected in ["socketName", "anpod_test_session", "connectCode", "test_token", "playbackMode", "lowlatency"] {
            assert!(audio_start.iter().any(|arg| arg == expected), "missing {expected} in {audio_start:?}");
        }
        assert!(audio_start.windows(2).any(|pair| pair[0] == "--user" && pair[1] == TEST_USER));
    }

    #[tokio::test]
    async fn fake_adb_rejects_healthy_foreground_state_when_it_contains_an_error() {
        let files = ArtifactFiles::new();
        let adb = FakeAdb::new(true, files.digest.clone());
        {
            let mut state = adb.0.lock().unwrap();
            state.active = true;
            state.foreground = true;
            state.healthy = true;
            state.error = "AudioTrackInitializationFailed".into();
        }
        let result = start_with_action(&adb, SERIAL, &files.apk, &files.metadata,
            &CancellationToken::new(), Some(("receiver-test", "test-code", "balanced"))).await;
        assert!(result.err().unwrap().contains("AudioTrackInitializationFailed"));
        assert!(adb.call_index(&["shell", "am", "start"]).is_none());
    }

    #[tokio::test]
    async fn fake_adb_receiver_connect_succeeds_before_first_pcm_reaches_audiotrack() {
        let files = ArtifactFiles::new();
        let adb = FakeAdb::new(true, files.digest.clone());
        let cancel = CancellationToken::new();
        let started = start_with_action(&adb, SERIAL, &files.apk, &files.metadata, &cancel,
            Some(("receiver-test", "test-code", "balanced"))).await.unwrap();
        assert!(started.healthy);
        assert!(started.message.contains("没有确认音频播放"));
        assert!(started.message.contains("前台运行"));
        let health = parse_health("Row: 0 applicationId=app.anpod.receiver, versionName=0.1.0-p1, versionCode=1, serviceActive=true, foreground=true, healthy=true, audioActive=false, startedAtElapsedMs=1, error=").unwrap();
        assert!(!status_from_health(health).message.contains("正在前台播放"));
    }

    #[tokio::test]
    async fn fake_adb_health_distinguishes_waiting_playback_and_receiver_failure() {
        let files = ArtifactFiles::new();
        let adb = FakeAdb::new(true, files.digest.clone());
        let cancel = CancellationToken::new();
        let ready = start_with_action(&adb, SERIAL, &files.apk, &files.metadata, &cancel,
            Some(("receiver-test", "test-code", "balanced"))).await.unwrap();
        assert!(!ready.audio_active);
        assert!(ready.monitoring_error().is_none());

        adb.0.lock().unwrap().audio_active = true;
        let playing = health_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel).await.unwrap();
        assert!(playing.audio_active);
        assert!(playing.monitoring_error().is_none());

        {
            let mut state = adb.0.lock().unwrap();
            state.audio_active = false;
            state.error = "AudioFocusLost".into();
            state.healthy = false;
        }
        let failed = health_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel).await.unwrap();
        assert_eq!(failed.monitoring_error().as_deref(), Some("AudioFocusLost"));

        {
            let mut state = adb.0.lock().unwrap();
            state.active = false;
            state.foreground = false;
            state.healthy = false;
            state.error.clear();
        }
        let stopped = health_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel).await.unwrap();
        assert!(!stopped.service_active);
        assert!(stopped.monitoring_error().is_some());
    }

    #[tokio::test]
    async fn fake_adb_rejects_same_version_forged_health_before_read_or_management() {
        let files = ArtifactFiles::new();
        let adb = FakeAdb::new(true, "f".repeat(64));
        let error = start_with(&adb, SERIAL, &files.apk, &files.metadata, &CancellationToken::new()).await.err().unwrap();
        assert!(error.starts_with("RECEIVER_UPDATE_REQUIRED:"));
        let calls = adb.calls();
        assert!(!calls.iter().any(|call| call.starts_with(&["shell".into(), "content".into()])));
        assert!(!calls.iter().any(|call| call.iter().any(|arg| arg == "app.anpod.receiver.action.START" || arg == "app.anpod.receiver.action.STOP")));
    }

    #[tokio::test]
    async fn fake_adb_install_failure_does_not_query_health_or_continue() {
        let files = ArtifactFiles::new();
        let adb = FakeAdb::new(false, files.digest.clone());
        adb.0.lock().unwrap().install_fails = true;
        let error = install_with(&adb, SERIAL, &files.apk, &files.metadata, &CancellationToken::new()).await.err().unwrap();
        assert!(error.contains("签名不匹配"));
        let calls = adb.calls();
        assert!(calls.iter().any(|call| call.starts_with(&["install".into(), "-r".into()])));
        assert!(!calls.iter().any(|call| call.starts_with(&["shell".into(), "content".into()])));
        assert!(!calls.iter().any(|call| call.iter().any(|arg| arg == "app.anpod.receiver.action.START" || arg == "app.anpod.receiver.action.STOP")));
    }

    #[tokio::test]
    async fn fake_adb_install_rejects_success_when_historical_same_version_apk_remains() {
        let files = ArtifactFiles::new();
        fs::write(&files.metadata, serde_json::json!({
            "applicationId": PACKAGE,
            "versionCode": 3,
            "versionName": "0.3.0-v2-candidate",
            "sha256": files.digest,
            "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
            "sourceCommit": "608c98c75ec04e11555e9e547caa516c96d4e0dd"
        }).to_string()).unwrap();
        let adb = FakeAdb::new(false,
            "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into());
        {
            let mut state = adb.0.lock().unwrap();
            state.health_version_code = 3;
            state.health_version_name = "0.3.0-v2-candidate".into();
        }
        let error = install_with(&adb, SERIAL, &files.apk, &files.metadata,
            &CancellationToken::new()).await.err().unwrap();
        assert!(error.contains("与本次目标包不一致"));
        let calls = adb.calls();
        assert!(calls.iter().any(|call| call.starts_with(&["install".into(), "-r".into()])));
        assert!(calls.iter().any(|call| call.starts_with(&["shell".into(), "toybox".into(), "sha256sum".into()])));
        assert!(!calls.iter().any(|call| call.starts_with(&["shell".into(), "content".into(), "query".into()])));
    }

    #[tokio::test]
    async fn fake_adb_rejects_unauthorized_and_wifi_devices_before_package_commands() {
        let files = ArtifactFiles::new();
        let unauthorized = FakeAdb::new(false, files.digest.clone());
        unauthorized.0.lock().unwrap().device_state = "unauthorized";
        assert!(start_with(&unauthorized, SERIAL, &files.apk, &files.metadata, &CancellationToken::new()).await.err().unwrap().contains("已授权的在线 ADB 设备"));
        assert_eq!(unauthorized.calls().len(), 1);

        let wifi = FakeAdb::new(false, files.digest.clone());
        assert!(start_with(&wifi, "192.0.2.3:5555", &files.apk, &files.metadata, &CancellationToken::new()).await.err().unwrap().contains("当前不可用"));
        assert_eq!(wifi.calls().len(), 1, "wireless ADB commands must confirm the exact selected serial first");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn fake_adb_secure_provision_confirms_persists_and_rejects_failures_without_leaks() {
        use crate::security::BindingStore;

        let files = ArtifactFiles::new();
        let cancel = CancellationToken::new();
        let timeout = Duration::from_millis(40);
        let binding_reference = binding_reference_for(SERIAL, TEST_USER);
        let binding_path = |case: &str| {
            let directory = files.dir.join(case);
            fs::create_dir_all(&directory).unwrap();
            directory.join(format!("{binding_reference}.dpapi"))
        };

        let successful = FakeAdb::new(true, files.digest.clone());
        let successful_path = binding_path("successful");
        let commitment = provision_with(&successful, SERIAL, &files.apk, &files.metadata,
            &successful_path, &cancel, timeout).await.unwrap();
        assert_eq!(successful.0.lock().unwrap().binding_commitment.as_deref(), Some(commitment.as_str()));
        let calls = successful.calls();
        let write_index = calls.iter().position(|call| call.starts_with(&[
            "shell".into(), "-T".into(), "content".into(), "write".into(),
        ])).unwrap();
        assert!(calls[write_index].windows(2).any(|pair| pair[0] == "--user" && pair[1] == TEST_USER));
        assert!(calls[write_index].windows(2).any(|pair| pair[0] == "--uri" && pair[1] == BINDING_URI));
        assert!(calls.iter().flat_map(|call| call.iter()).all(|arg| !arg.contains(&commitment)));

        let retry_commitment = provision_with(&successful, SERIAL, &files.apk, &files.metadata,
            &successful_path, &cancel, timeout).await.unwrap();
        assert_eq!(retry_commitment, commitment);
        let successful_inputs = successful.0.lock().unwrap().binding_input_commitments.clone();
        assert_eq!(successful_inputs.len(), 2);
        assert_eq!(successful_inputs[0], successful_inputs[1]);

        let cancelled_path = binding_path("cancelled");
        let cancelled_adb = FakeAdb::new(true, files.digest.clone());
        cancelled_adb.0.lock().unwrap().binding_write_mode = FakeBindingWrite::Cancel;
        let cancelled_token = CancellationToken::new();
        let cancelled = provision_with(&cancelled_adb, SERIAL, &files.apk, &files.metadata,
            &cancelled_path, &cancelled_token, timeout).await.unwrap_err();
        assert!(cancelled.contains("取消"));
        let saved_before_retry = BindingStore::load(&cancelled_path).unwrap().commitment();
        assert!(cancelled_path.exists(), "cancellation discarded the only DPAPI binding copy");
        let retry = provision_with(&cancelled_adb, SERIAL, &files.apk, &files.metadata,
            &cancelled_path, &CancellationToken::new(), timeout).await.unwrap();
        assert_eq!(retry, hex_digest(&saved_before_retry));
        let seen = &cancelled_adb.0.lock().unwrap().binding_input_commitments;
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], seen[1], "retry generated different binding material");

        let conflict_path = binding_path("conflict");
        let conflict_adb = FakeAdb::new(true, files.digest.clone());
        conflict_adb.0.lock().unwrap().binding_commitment = Some("f".repeat(64));
        let conflict = provision_with(&conflict_adb, SERIAL, &files.apk, &files.metadata,
            &conflict_path, &cancel, timeout).await.unwrap_err();
        assert!(conflict.contains("已有不同绑定"));
        assert!(conflict_path.exists(), "remote conflict removed the local DPAPI copy");
        let expected_conflict = "f".repeat(64);
        assert_eq!(conflict_adb.0.lock().unwrap().binding_commitment.as_deref(), Some(expected_conflict.as_str()));

        let failed_write_path = binding_path("failed-write");
        let failed_write_adb = FakeAdb::new(true, files.digest.clone());
        failed_write_adb.0.lock().unwrap().binding_write_mode = FakeBindingWrite::ErrorText;
        let failed_write = provision_with(&failed_write_adb, SERIAL, &files.apk, &files.metadata,
            &failed_write_path, &cancel, timeout).await.unwrap_err();
        assert!(failed_write.contains("未确认"));
        assert!(!failed_write.contains("Error while accessing provider"));
        assert!(failed_write_path.exists(), "remote error discarded the local DPAPI copy");
        let failed_material = BindingStore::load(&failed_write_path).unwrap();
        let mut failed_record = failed_material.encode();
        let secret_hex = failed_record.0[37..69].iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert!(!failed_write.contains(&secret_hex), "remote error leaked the binding secret");
        failed_record.0.fill(0);

        let wrong_user_path = binding_path("wrong-user");
        let wrong_user_adb = FakeAdb::new(true, files.digest.clone());
        wrong_user_adb.0.lock().unwrap().binding_write_mode = FakeBindingWrite::WrongUser;
        let wrong_user = provision_with(&wrong_user_adb, SERIAL, &files.apk, &files.metadata,
            &wrong_user_path, &cancel, timeout).await.unwrap_err();
        assert!(wrong_user.contains("当前用户已变化"));
        assert!(!wrong_user_adb.calls().iter().any(|call| call.starts_with(&[
            "shell".into(), "content".into(), "query".into(),
        ])));

        let non_usb = FakeAdb::new(false, files.digest.clone());
        non_usb.0.lock().unwrap().device_state = "unauthorized";
        let non_usb_path = binding_path("non-usb");
        assert!(provision_with(&non_usb, SERIAL, &files.apk, &files.metadata,
            &non_usb_path, &cancel, timeout).await.is_err());
        assert!(!non_usb_path.exists());
        assert!(!non_usb.calls().iter().any(|call| call.starts_with(&[
            "shell".into(), "content".into(), "write".into(),
        ])));

        let wrong_apk = FakeAdb::new(true, "a".repeat(64));
        let wrong_apk_path = binding_path("wrong-apk");
        assert!(provision_with(&wrong_apk, SERIAL, &files.apk, &files.metadata,
            &wrong_apk_path, &cancel, timeout).await.is_err());
        assert!(!wrong_apk_path.exists());
        assert!(!wrong_apk.calls().iter().any(|call| call.starts_with(&[
            "shell".into(), "content".into(), "write".into(),
        ])));
    }

    #[tokio::test]
    async fn p1_real_usb_installed_receiver_health_start_stop_when_explicitly_requested() {
        let Some(serial) = std::env::var_os("ANPOD_P1_USB_SERIAL") else { return; };
        let serial = serial.to_string_lossy();
        let deps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../deps");
        let adb = deps.join("adb.exe");
        let apk = deps.join("receiver.apk");
        let metadata = deps.join("receiver.json");
        let cancel = CancellationToken::new();

        assert!(status(&adb, &serial, &apk, &metadata, &cancel).await.unwrap().installed,
            "P1 lifecycle test requires Receiver to be installed by a separate deployment step");
        assert!(health(&adb, &serial, &apk, &metadata, &cancel).await.unwrap().installed);
        let started = start(&adb, &serial, &apk, &metadata, &cancel).await.unwrap();
        assert!(started.healthy && started.foreground);
        let observed = health(&adb, &serial, &apk, &metadata, &cancel).await.unwrap();
        assert!(observed.healthy && observed.foreground);
        let stopped = stop(&adb, &serial, &apk, &metadata, &cancel).await.unwrap();
        assert!(!stopped.service_active);
        assert!(!health(&adb, &serial, &apk, &metadata, &cancel).await.unwrap().service_active);
    }
}
