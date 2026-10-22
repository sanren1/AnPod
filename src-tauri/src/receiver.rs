use crate::state::{adb_run, adb_run_with_stdin, parse_devices, AdbResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    future::Future,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const PACKAGE: &str = "app.anpod.receiver";
const HEALTH_URI: &str = "content://app.anpod.receiver.state/health";
const BINDING_URI: &str = "content://app.anpod.receiver.state/binding";
const INTERFACES_URI: &str = "content://app.anpod.receiver.state/interfaces";
const NETWORK_URI: &str = "content://app.anpod.receiver.state/network";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallProgressStage {
    Checking,
    Transferring,
    Verifying,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub operation_id: String,
    pub serial: String,
    pub stage: InstallProgressStage,
}

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
    pub operation_owner_id: Option<String>,
    pub operation_generation: Option<u64>,
    pub terminal_cause: Option<String>,
    pub terminal_owner_id: Option<String>,
    pub terminal_generation: Option<u64>,
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
    pub output_format: Option<u8>,
    pub opened_track_channels: Option<u8>,
    pub output_sample_rate: Option<u32>,
}

impl ReceiverStatus {
    pub fn was_stopped_by_user_for(&self, owner_id: Option<&str>, generation: Option<u64>) -> bool {
        self.terminal_cause.as_deref() == Some("USER_STOP")
            && owner_id.is_some()
            && owner_id == self.terminal_owner_id.as_deref()
            && generation.is_some()
            && generation == self.terminal_generation
    }

    pub fn monitoring_error(&self) -> Option<String> {
        if self.healthy && self.service_active && self.error.is_none() {
            None
        } else {
            Some(self.error.clone().unwrap_or_else(|| self.message.clone()))
        }
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

#[derive(Clone, Copy)]
struct AuditedReceiverArtifact {
    sha256: &'static str,
    version_code: u32,
    version_name: &'static str,
    source_commit: &'static str,
    signer_sha256: &'static str,
}

// Only same-signer APKs with an exact audited digest and recorded source are admitted.
// Every shipped APK must be added here after release, or the next version cannot
// recognise the installed one; see packaging/SIGNING.md.
const AUDITED_RECEIVER_ARTIFACTS: &[AuditedReceiverArtifact] = &[
    AuditedReceiverArtifact {
        sha256: "cbfdd5dad8ee3e38bc704290b1bcfc95a4b488fd2d80f8d135a4baecad804e56",
        version_code: 41,
        version_name: "0.18.0-v3-flow-candidate",
        source_commit: "c75856fe83579c1e0650271b0ff26e5bdebf03db",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "56578ab87996fb12b16d919f8c6996a86f13a4230703dd07f6f58c202bf2d01f",
        version_code: 40,
        version_name: "0.17.6-v3-flow-candidate",
        source_commit: "3dfb08cd0487fb2ea3b1b9d20a7c508727a73486",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "a77bf771a07272bba5d7437e1d7dd69906387e256537350de990eb789fc2b0d0",
        version_code: 39,
        version_name: "0.17.5-v3-flow-candidate",
        source_commit: "01f16af88b8fbc7578126daa15d0c757973cd2b1",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "01beca356daf6e5630fbce9ff5c24a3b61f5ca75b08f22ce40c1ab08a29037c7",
        version_code: 38,
        version_name: "0.17.4-v3-flow-candidate",
        source_commit: "f1040de83a41e04aa659af916a99fd52055ac5d1",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "379d66041630c65f2ecfab2dedbf649441687759bfdd5d41746079be0fb4dbac",
        version_code: 37,
        version_name: "0.17.3-v3-flow-candidate",
        source_commit: "17394730ee6edad41339ebb66869a41bc92b4ece",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "0f34c03b5618182fcd90f5a33883bd7a6de5f5a4696d9c1a64603f3f4983194e",
        version_code: 36,
        version_name: "0.17.2-v3-flow-candidate",
        source_commit: "ce81f03b54285691b3ce602461180516b60a1d4b",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "8f8e76e4b9e2dc1e3bb8d4575f5e21e77838056b2b55536c5736eedcccf4f15f",
        version_code: 35,
        version_name: "0.17.1-v3-flow-candidate",
        source_commit: "2bd998d65fe650bb225c09d7353c5c2ba39febfc",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "711c20c4348b1865d0475244271271f1fc32d70888f1f77054481bba138fed83",
        version_code: 34,
        version_name: "0.17.0-v3-flow-candidate",
        source_commit: "8ca9638db128f99e36769f42de18ed9244f457c2",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "c05f3407aefd7e324017ccfecd5fd431d8eb1e1e9beac4b66e4e59b4cbc0647d",
        version_code: 33,
        version_name: "0.16.0-v3-flow-candidate",
        source_commit: "c299c805f8917df0f961b5174dff47fb074517e6",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "a103232b24e1fcddb456d3d8b986ca69ec902f147c8d1665aecca1cda189ec5c",
        version_code: 32,
        version_name: "0.15.7-v3-flow-candidate",
        source_commit: "fc529c9f6ff41c227f5bf41000f1be3e3def2ac4",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "c80a965f525c81618a096a3a684e7cfe67cfdc201f9d50eb9006560fd3cc0b72",
        version_code: 31,
        version_name: "0.15.6-v3-flow-candidate",
        source_commit: "f7b457671da5450d676362c30976f45d8f30deeb",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "e70c53d1393de3a976becf78d8ab5c3d4b94bcdf5be890e0901aa79fd2e8314e",
        version_code: 30,
        version_name: "0.15.5-v3-flow-candidate",
        source_commit: "e43cd685e99696b0c6f9618cfc11871043781843",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "630e24796fc7c5fc07dc78db5ec39a29a84fcfc3bf658fc50037d5e3d283930e",
        version_code: 29,
        version_name: "0.15.4-v3-flow-candidate",
        source_commit: "e43cd685e99696b0c6f9618cfc11871043781843",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "e15777fdf1c448b404dec9b7216ea8d6b8ff6c37196fdccbae089cae0336e4bd",
        version_code: 28,
        version_name: "0.15.3-v3-flow-candidate",
        source_commit: "e43cd685e99696b0c6f9618cfc11871043781843",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "c9593516c9ec9e81f2f533d8075884f35ab35bd8e501d7fe7ab2175fc4775219",
        version_code: 27,
        version_name: "0.15.2-v3-flow-candidate",
        source_commit: "e43cd685e99696b0c6f9618cfc11871043781843",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "a6b7c61b6ad313ed47a9c817ae997f4a4be9f8045bb0053c7a38b288da33272d",
        version_code: 26,
        version_name: "0.15.1-v3-flow-candidate",
        source_commit: "e43cd685e99696b0c6f9618cfc11871043781843",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "469b4ef9ec4fd441c1e2cd64ab6423316df402b9f3f6aac0e43442309308662c",
        version_code: 25,
        version_name: "0.15.0-v3-flow-candidate",
        source_commit: "e43cd685e99696b0c6f9618cfc11871043781843",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "ce25dab8fc20891b3947369b4165bd2cff89388b9d7c9d99583605addecd4c09",
        version_code: 24,
        version_name: "0.14.0-v3-flow-candidate",
        source_commit: "e43cd685e99696b0c6f9618cfc11871043781843",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "ff762492a15c746de025cc84e877354a567dc31d747dbf857e08b9ef03ae189d",
        version_code: 23,
        version_name: "0.13.7-v3-flow-candidate",
        source_commit: "e43cd685e99696b0c6f9618cfc11871043781843",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "1b73f76b98ae2470969531085fc5bd7d5114055a58c5df3beeb0f6ad6602c549",
        version_code: 22,
        version_name: "0.13.6-v3-flow-candidate",
        source_commit: "f1b8ba751cfeee24b5e9f7f918bcd78374e0e840",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "1daf7889d1330d61187c938d2b3591029ff5c8308982e288c0fd5590c10fe732",
        version_code: 18,
        version_name: "0.13.2-v3-flow-candidate",
        source_commit: "f712e20407428adc69c6c25a9176d220aaec4f78",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "b2d501f8a387e1b339dea1f40d153f4beb00545a3f5d8597da6b0c686ca14d35",
        version_code: 16,
        version_name: "0.13.0-v3-flow-candidate",
        source_commit: "6f641de5748cc2adb4b41c9a4e84178a19c99549",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "c3d56258a0ffa94dae4a861affb21ff5783f53b7a1c22813eb8be47b0e14313b",
        version_code: 17,
        version_name: "0.13.1-v3-flow-candidate",
        source_commit: "3c13025ff4ae096a0e5a6001a8befa1887276edb",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
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
        sha256: "1791bf24f848e7470206b75151b081c05b4e5fd2af3e92c67f2077cda9cdbdbd",
        version_code: 12,
        version_name: "0.9.1-v3-flow-candidate",
        source_commit: "61f32f7798f29bb39cf256997de50992a85ea519",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
    AuditedReceiverArtifact {
        sha256: "de64b3473a7848481e9ff75e98e3171dd7052b0b44f40b09d4fdaffdd669de3f",
        version_code: 13,
        version_name: "0.10.0-v3-flow-candidate",
        source_commit: "61f32f7798f29bb39cf256997de50992a85ea519",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
    AuditedReceiverArtifact {
        sha256: "4d3b578e004992816cb1443fd26ecded1c8f3b087ffb14d1ab4f0727ee31c464",
        version_code: 14,
        version_name: "0.11.0-v3-flow-candidate",
        source_commit: "61f32f7798f29bb39cf256997de50992a85ea519",
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
    },
    AuditedReceiverArtifact {
        sha256: "9f5234bebd1c9728b24b557d8ed84f7fc82dcf66129d01588e294b22563d2cba",
        version_code: 14,
        version_name: "0.11.0-v3-flow-candidate",
        source_commit: "940b06e5de20310c589cf7b0cc3e1be983a5f45e",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "5a29fc35e9ac065b57744abef13d8d06996a8554a5a0c17277bc425a58fac2e9",
        version_code: 15,
        version_name: "0.12.0-v3-flow-candidate",
        source_commit: "9324d48f6acfd41aff5359b04ab0fde016200671",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
    },
    AuditedReceiverArtifact {
        sha256: "4f23da449cf0340effe76c9ec90b0931a78ac688c45f21dab314825a410de9ee",
        version_code: 16,
        version_name: "0.13.0-v3-flow-candidate",
        source_commit: "9091cf1b6644797995c31af209d691de203b794d",
        signer_sha256: "f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573",
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

fn verified_installed_artifact(
    digest: &str,
    current: &ArtifactMetadata,
) -> Option<VerifiedReceiverArtifact> {
    if digest.eq_ignore_ascii_case(&current.sha256) {
        return Some(VerifiedReceiverArtifact {
            version_code: current.version_code,
            version_name: current.version_name.clone(),
            sha256: current.sha256.to_ascii_lowercase(),
            signer_sha256: current.signer_sha256.to_ascii_lowercase(),
            source_commit: current.source_commit.clone(),
        });
    }
    AUDITED_RECEIVER_ARTIFACTS
        .iter()
        .find(|artifact| {
            digest.eq_ignore_ascii_case(artifact.sha256)
                && current.application_id == PACKAGE
                && current
                    .signer_sha256
                    .eq_ignore_ascii_case(artifact.signer_sha256)
        })
        .map(|artifact| VerifiedReceiverArtifact {
            version_code: artifact.version_code,
            version_name: artifact.version_name.into(),
            sha256: artifact.sha256.into(),
            signer_sha256: artifact.signer_sha256.into(),
            source_commit: Some(artifact.source_commit.into()),
        })
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
    operation_owner_id: Option<String>,
    operation_generation: Option<u64>,
    terminal_cause: Option<String>,
    terminal_owner_id: Option<String>,
    terminal_generation: Option<u64>,
    playback_diagnostics: PlaybackDiagnostics,
    error: String,
}

trait AdbRunner {
    fn run<'a>(
        &'a self,
        serial: Option<&'a str>,
        args: &'a [&'a str],
        limit: Duration,
        cancel: Option<&'a CancellationToken>,
    ) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a;
    fn run_with_stdin<'a>(
        &'a self,
        serial: Option<&'a str>,
        args: &'a [&'a str],
        input: &'a [u8],
        limit: Duration,
        cancel: &'a CancellationToken,
    ) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a;
}

struct SystemAdb<'a>(&'a Path);

impl AdbRunner for SystemAdb<'_> {
    fn run<'a>(
        &'a self,
        serial: Option<&'a str>,
        args: &'a [&'a str],
        limit: Duration,
        cancel: Option<&'a CancellationToken>,
    ) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a {
        adb_run(self.0, serial, args, limit, cancel)
    }

    fn run_with_stdin<'a>(
        &'a self,
        serial: Option<&'a str>,
        args: &'a [&'a str],
        input: &'a [u8],
        limit: Duration,
        cancel: &'a CancellationToken,
    ) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a {
        adb_run_with_stdin(self.0, serial, args, input, limit, cancel)
    }
}

#[cfg(windows)]
#[allow(dead_code)]
pub async fn provision(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    binding_path: &Path,
    cancel: &CancellationToken,
) -> Result<String, String> {
    if let Some(parent) = binding_path.parent() {
        fs::create_dir_all(parent).map_err(|_| "无法创建本机绑定存储目录。".to_string())?;
    }
    provision_with(
        &SystemAdb(adb),
        serial,
        apk,
        metadata_path,
        binding_path,
        cancel,
        Duration::from_secs(8),
    )
    .await
}

#[cfg(windows)]
pub async fn binding_reference(
    adb: &Path,
    serial: &str,
    cancel: &CancellationToken,
) -> Result<String, String> {
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
pub async fn binding_reset_preview(
    adb: &Path,
    serial: &str,
    data_dir: &Path,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<BindingResetPreview, String> {
    binding_reset_preview_with(
        &SystemAdb(adb),
        serial,
        data_dir,
        apk,
        metadata_path,
        cancel,
    )
    .await
}

#[cfg(windows)]
async fn binding_reset_preview_with(
    adb: &impl AdbRunner,
    serial: &str,
    data_dir: &Path,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<BindingResetPreview, String> {
    use crate::security::{BindingStore, SecurityError};

    ensure_usb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let reference = binding_reference_for(serial, &user);
    let metadata = read_artifact(apk, metadata_path)?;
    verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    require_binding_reset_capability(adb, serial, &user, cancel).await?;

    let (phone_state, phone_commitment) =
        match read_binding_state(adb, serial, &user, cancel).await? {
            BindingStatus::Empty => ("empty".to_owned(), None),
            BindingStatus::Pending => ("pending".to_owned(), None),
            BindingStatus::Saved(commitment) => ("saved".to_owned(), Some(commitment)),
            BindingStatus::Failed(_) => ("failed".to_owned(), None),
        };
    let network = read_reset_network_state(adb, serial, &user, cancel).await?;
    let health = read_health(adb, serial, &user, cancel).await?;
    let binding_path = data_dir
        .join("receiver-bindings")
        .join(format!("{reference}.dat"));
    let (pc_state, pc_commitment) = match BindingStore::load(&binding_path) {
        Ok(material) => ("saved".to_owned(), Some(hex_digest(&material.commitment()))),
        Err(SecurityError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            ("missing".to_owned(), None)
        }
        Err(_) => {
            return Err("所选 PC 绑定文件无法通过 DPAPI 和格式核验；未停止会话或删除数据。".into())
        }
    };

    Ok(BindingResetPreview {
        serial: serial.to_owned(),
        android_user: user,
        binding_reference: reference,
        phone_state,
        phone_commitment,
        pc_state,
        pc_commitment,
        receiver_busy: health.audio_active || network != "stopped",
    })
}

#[cfg(windows)]
async fn require_binding_reset_capability(
    adb: &impl AdbRunner,
    serial: &str,
    user: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let result = adb
        .run(
            Some(serial),
            &[
                "shell",
                "content",
                "query",
                "--user",
                user,
                "--uri",
                BINDING_URI,
                "--projection",
                "bindingResetVersion",
            ],
            Duration::from_secs(6),
            Some(cancel),
        )
        .await?;
    let supported = result.success
        && result
            .stdout
            .lines()
            .filter_map(parse_content_row)
            .any(|row| {
                row.get("bindingResetVersion")
                    .is_some_and(|value| value == "1")
            });
    if supported {
        Ok(())
    } else {
        Err("RECEIVER_UPDATE_REQUIRED:已安装 Receiver 不支持条件绑定重置。".into())
    }
}

#[cfg(windows)]
async fn read_reset_network_state(
    adb: &impl AdbRunner,
    serial: &str,
    user: &str,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let result = adb
        .run(
            Some(serial),
            &[
                "shell",
                "content",
                "query",
                "--user",
                user,
                "--uri",
                NETWORK_URI,
                "--projection",
                "networkAudioVersion:networkAudioState",
            ],
            Duration::from_secs(6),
            Some(cancel),
        )
        .await?;
    if !result.success {
        return Err("无法确认手机是否仍有网络接收会话；未停止会话或删除绑定。".into());
    }
    let row = result
        .stdout
        .lines()
        .find_map(parse_content_row)
        .ok_or_else(|| "手机网络接收状态格式无效；未停止会话或删除绑定。".to_string())?;
    if row.get("networkAudioVersion").map(String::as_str) != Some("1") {
        return Err("手机网络接收状态版本不受支持；未停止会话或删除绑定。".into());
    }
    row.get("networkAudioState")
        .cloned()
        .ok_or_else(|| "手机网络接收状态缺少会话状态；未停止会话或删除绑定。".into())
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn reset_result(
    preview: &BindingResetPreview,
    outcome: &str,
    phone_state: &str,
    phone_commitment: Option<String>,
    pc_state: &str,
    pc_commitment: Option<String>,
    reason: &str,
    next_action: &str,
) -> BindingResetResult {
    BindingResetResult {
        outcome: outcome.into(),
        serial: preview.serial.clone(),
        android_user: preview.android_user.clone(),
        binding_reference: preview.binding_reference.clone(),
        phone_state: phone_state.into(),
        phone_commitment,
        pc_state: pc_state.into(),
        pc_commitment,
        reason: reason.into(),
        next_action: next_action.into(),
    }
}

#[cfg(windows)]
pub async fn reset_binding(
    adb: &Path,
    preview: &BindingResetPreview,
    data_dir: &Path,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
    is_current: &impl Fn() -> bool,
) -> Result<BindingResetResult, String> {
    reset_binding_with(
        &SystemAdb(adb),
        preview,
        data_dir,
        apk,
        metadata_path,
        cancel,
        is_current,
    )
    .await
}

#[cfg(windows)]
async fn reset_binding_with(
    adb: &impl AdbRunner,
    expected: &BindingResetPreview,
    data_dir: &Path,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
    is_current: &impl Fn() -> bool,
) -> Result<BindingResetResult, String> {
    use crate::security::BindingStore;

    let before =
        binding_reset_preview_with(adb, &expected.serial, data_dir, apk, metadata_path, cancel)
            .await?;
    if !same_reset_target(&before, expected) {
        return Err(
            "重置预览已过期（设备、Android user 或绑定摘要发生变化）；未删除绑定，请重新检查。"
                .into(),
        );
    }
    if before.phone_state == "pending" || before.phone_state == "failed" {
        return Err("手机绑定状态正在写入或不可读取；请完成/检查绑定操作后重新预览。".into());
    }
    if before.receiver_busy {
        return Err("手机仍有活动的接收或播放会话；请先在手机上停止接收，再重新预览。".into());
    }
    if !is_current() || cancel.is_cancelled() {
        return Err("绑定重置已取消；尚未更改任何绑定。".into());
    }

    if before.phone_state == "empty" && before.pc_state == "missing" {
        return Ok(reset_result(
            &before,
            "alreadyEmpty",
            "empty",
            None,
            "missing",
            None,
            "两端都没有此目标的绑定。",
            "如需配对，请单独启动 USB 首次绑定。",
        ));
    }

    let selection = match before.phone_commitment.as_deref() {
        Some(commitment) => format!("expectedCommitment={commitment}"),
        None => "expectedEmpty".to_owned(),
    };
    let delete_result = adb
        .run(
            Some(&before.serial),
            &[
                "shell",
                "content",
                "delete",
                "--user",
                &before.android_user,
                "--uri",
                BINDING_URI,
                "--where",
                &selection,
            ],
            Duration::from_secs(10),
            Some(cancel),
        )
        .await;
    let binding_path = data_dir
        .join("receiver-bindings")
        .join(format!("{}.dat", before.binding_reference));

    // The shell exit status is not authoritative; observe the fixed URI after the attempt.
    let after_phone =
        match binding_reset_preview_with(adb, &before.serial, data_dir, apk, metadata_path, cancel)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let (phone_state, phone_commitment) =
                    match read_reset_phone_status(adb, &before, apk, metadata_path, cancel).await {
                        Ok(BindingStatus::Empty) => ("empty".to_owned(), None),
                        Ok(BindingStatus::Pending) => ("pending".to_owned(), None),
                        Ok(BindingStatus::Saved(commitment)) => {
                            ("saved".to_owned(), Some(commitment))
                        }
                        Ok(BindingStatus::Failed(_)) => ("failed".to_owned(), None),
                        Err(_) => ("unknown".to_owned(), None),
                    };
                let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
                let outcome = if phone_state == "unknown" {
                    "phoneStateUnknown"
                } else if phone_state == "empty" && pc_state == "unknown" {
                    "phoneClearedPcUnknown"
                } else if phone_state == "empty" {
                    pc_partial_outcome(&before, &pc_state, &pc_commitment)
                } else {
                    "phoneRetained"
                };
                return Ok(reset_result(
                    &before,
                    outcome,
                    &phone_state,
                    phone_commitment,
                    &pc_state,
                    pc_commitment,
                    &format!("手机删除结果无法回读：{error}"),
                    "保持 USB 连接并重新预览手机与 PC 绑定状态。",
                ));
            }
        };
    if after_phone.android_user != before.android_user
        || after_phone.binding_reference != before.binding_reference
    {
        return Ok(reset_result(
            &before,
            "targetChanged",
            "unknown",
            None,
            &before.pc_state,
            before.pc_commitment.clone(),
            "Android user 或 USB 目标已变化；PC 绑定仍保留。",
            "重新选择并核验 USB 设备与 Android user。",
        ));
    }
    if after_phone.phone_state != "empty" {
        let exit_detail = match delete_result {
            Ok(result) if result.success => "删除命令已返回，但手机仍报告原绑定。",
            Ok(_) => "删除命令失败且手机仍报告绑定。",
            Err(_) => "删除命令结果不确定且手机仍报告绑定。",
        };
        return Ok(reset_result(
            &before,
            "phoneRetained",
            &after_phone.phone_state,
            after_phone.phone_commitment,
            &before.pc_state,
            before.pc_commitment.clone(),
            exit_detail,
            "绑定未清除；核对手机端状态后重新预览。PC 绑定未删除。",
        ));
    }
    if !is_current() || cancel.is_cancelled() {
        let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
        let outcome = pc_partial_outcome(&before, &pc_state, &pc_commitment);
        return Ok(reset_result(
            &before,
            outcome,
            "empty",
            None,
            &pc_state,
            pc_commitment,
            "操作在手机绑定清除后取消；PC 记录状态已回读。",
            "保持 USB 连接并重试重置，以清理这一个 PC 记录。",
        ));
    }

    let before_pc_remove =
        match binding_reset_preview_with(adb, &before.serial, data_dir, apk, metadata_path, cancel)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let (phone_state, phone_commitment) =
                    match read_reset_phone_status(adb, &before, apk, metadata_path, cancel).await {
                        Ok(BindingStatus::Empty) => ("empty".to_owned(), None),
                        Ok(BindingStatus::Pending) => ("pending".to_owned(), None),
                        Ok(BindingStatus::Saved(commitment)) => {
                            ("saved".to_owned(), Some(commitment))
                        }
                        Ok(BindingStatus::Failed(_)) => ("failed".to_owned(), None),
                        Err(_) => ("unknown".to_owned(), None),
                    };
                let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
                let outcome = if phone_state == "unknown" {
                    "phoneStateUnknown"
                } else if phone_state == "empty" {
                    pc_partial_outcome(&before, &pc_state, &pc_commitment)
                } else {
                    "phoneRetained"
                };
                return Ok(reset_result(
                    &before,
                    outcome,
                    &phone_state,
                    phone_commitment,
                    &pc_state,
                    pc_commitment,
                    &format!("手机已清除；删除 PC 记录前重新核验失败：{error}"),
                    "保持 USB 连接并重试；重新预览手机与 PC 绑定状态。",
                ));
            }
        };
    if before_pc_remove.android_user != before.android_user
        || before_pc_remove.binding_reference != before.binding_reference
        || before_pc_remove.phone_state != "empty"
        || before_pc_remove.pc_state != before.pc_state
        || before_pc_remove.pc_commitment != before.pc_commitment
    {
        return Ok(reset_result(
            &before,
            "phoneClearedPcChanged",
            &before_pc_remove.phone_state,
            before_pc_remove.phone_commitment,
            &before_pc_remove.pc_state,
            before_pc_remove.pc_commitment,
            "手机已清除，但删除前目标状态发生变化；未删除 PC 记录。",
            "重新预览并确认此 USB 目标对应的 PC 绑定状态。",
        ));
    }
    if before_pc_remove.receiver_busy {
        return Ok(reset_result(
            &before,
            "phoneClearedPcRetained",
            "empty",
            None,
            &before.pc_state,
            before.pc_commitment.clone(),
            "手机已清除，但 Receiver 出现活动会话；未删除 PC 记录。",
            "先停止手机接收，再重新预览并重试。",
        ));
    }
    if !is_current() || cancel.is_cancelled() {
        let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
        let outcome = pc_partial_outcome(&before, &pc_state, &pc_commitment);
        return Ok(reset_result(
            &before,
            outcome,
            "empty",
            None,
            &pc_state,
            pc_commitment,
            "操作在 PC 记录删除前取消；PC 记录状态已回读。",
            "保持 USB 连接并重试重置。",
        ));
    }

    let expected_pc = before
        .pc_commitment
        .as_deref()
        .map(parse_digest)
        .transpose()
        .map_err(|_| "PC 绑定摘要格式无效；没有删除 PC 记录。".to_string())?;
    let binding_path = data_dir
        .join("receiver-bindings")
        .join(format!("{}.dat", before.binding_reference));
    if let Err(error) = BindingStore::remove_if_matches(&binding_path, expected_pc.as_ref()) {
        let (pc_state, pc_commitment) = local_reset_binding(&binding_path);
        let outcome = pc_partial_outcome(&before, &pc_state, &pc_commitment);
        return Ok(reset_result(
            &before,
            outcome,
            "empty",
            None,
            &pc_state,
            pc_commitment,
            &format!("手机已清除；PC 条件删除未完成，回读当前 PC 状态：{error}"),
            "重新预览并确认此 USB 目标对应的 PC 绑定状态。",
        ));
    }

    if !is_current() || cancel.is_cancelled() {
        return Ok(reset_result(
            &before,
            "pcClearedPhoneEmpty",
            "empty",
            None,
            "missing",
            None,
            "两端绑定已清除；操作在最终核验前取消。",
            "重新预览确认两端都为空。",
        ));
    }
    let final_snapshot =
        match binding_reset_preview_with(adb, &before.serial, data_dir, apk, metadata_path, cancel)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Ok(reset_result(
                    &before,
                    "pcClearedPhoneUnknown",
                    "unknown",
                    None,
                    "missing",
                    None,
                    &format!("PC 目标记录已删除；手机最终状态无法确认：{error}"),
                    "保持 USB 连接并重新预览；若手机仍有绑定，再执行一次明确确认的重置。",
                ))
            }
        };
    if final_snapshot.android_user == before.android_user
        && final_snapshot.binding_reference == before.binding_reference
        && final_snapshot.phone_state == "empty"
        && final_snapshot.pc_state == "missing"
    {
        Ok(reset_result(
            &before,
            "reset",
            "empty",
            None,
            "missing",
            None,
            "所选 USB 设备与 Android user 的两端绑定均已清除。",
            "如需配对，请单独启动 USB 首次绑定。",
        ))
    } else {
        Ok(reset_result(
            &before,
            "pcClearedPhoneChanged",
            &final_snapshot.phone_state,
            final_snapshot.phone_commitment,
            "missing",
            None,
            "PC 目标记录已删除，但手机状态在最终核验时发生变化。",
            "重新预览当前手机绑定；不要继续使用旧配对信息。",
        ))
    }
}

#[cfg(windows)]
async fn read_reset_phone_status(
    adb: &impl AdbRunner,
    expected: &BindingResetPreview,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<BindingStatus, String> {
    ensure_usb_device(adb, &expected.serial, cancel).await?;
    let user = current_android_user(adb, &expected.serial, cancel).await?;
    if user != expected.android_user {
        return Err("Android user changed".into());
    }
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
        Err(SecurityError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            ("missing".into(), None)
        }
        Err(_) => ("unknown".into(), None),
    }
}

#[cfg(windows)]
fn pc_partial_outcome(
    before: &BindingResetPreview,
    state: &str,
    commitment: &Option<String>,
) -> &'static str {
    if state == "unknown" {
        "phoneClearedPcUnknown"
    } else if state == "missing" {
        "reset"
    } else if state == before.pc_state && commitment == &before.pc_commitment {
        "phoneClearedPcRetained"
    } else {
        "phoneClearedPcChanged"
    }
}

#[cfg(windows)]
fn same_reset_target(left: &BindingResetPreview, right: &BindingResetPreview) -> bool {
    left.serial == right.serial
        && left.android_user == right.android_user
        && left.binding_reference == right.binding_reference
        && left.phone_state == right.phone_state
        && left.phone_commitment == right.phone_commitment
        && left.pc_state == right.pc_state
        && left.pc_commitment == right.pc_commitment
}

#[cfg(windows)]
fn parse_digest(value: &str) -> Result<[u8; 32], ()> {
    if !valid_digest(value) {
        return Err(());
    }
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
async fn provision_with(
    adb: &impl AdbRunner,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    binding_path: &Path,
    cancel: &CancellationToken,
    confirmation_timeout: Duration,
) -> Result<String, String> {
    use crate::security::{BindingMaterial, BindingStore, SecurityError};

    ensure_usb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let expected_reference = binding_reference_for(serial, &user);
    if binding_path.file_stem().and_then(|value| value.to_str())
        != Some(expected_reference.as_str())
    {
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
            let generated =
                BindingMaterial::generate().map_err(|_| "无法创建安全绑定材料。".to_string())?;
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
    let write_result = adb
        .run_with_stdin(
            Some(serial),
            &[
                "shell",
                "-T",
                "content",
                "write",
                "--user",
                &user,
                "--uri",
                BINDING_URI,
            ],
            &record.0,
            Duration::from_secs(15),
            cancel,
        )
        .await;
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
    for byte in value {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    result
}

#[cfg(windows)]
enum BindingStatus {
    Empty,
    Pending,
    Saved(String),
    Failed(&'static str),
}

#[cfg(windows)]
async fn read_binding_state(
    adb: &impl AdbRunner,
    serial: &str,
    user: &str,
    cancel: &CancellationToken,
) -> Result<BindingStatus, String> {
    let result = adb
        .run(
            Some(serial),
            &[
                "shell",
                "content",
                "query",
                "--user",
                user,
                "--uri",
                BINDING_URI,
                "--projection",
                "bindingState:bindingCommitment:bindingFailure",
            ],
            Duration::from_secs(6),
            Some(cancel),
        )
        .await?;
    if !result.success {
        return Err("无法读取手机绑定确认状态；本机 DPAPI 副本已保留。".into());
    }
    let row = result
        .stdout
        .lines()
        .find_map(parse_content_row)
        .ok_or_else(|| "手机绑定状态格式无效；本机 DPAPI 副本已保留。".to_string())?;
    let fields: std::collections::HashMap<_, _> = row
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
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
    if let Some(message) = failure {
        return Ok(BindingStatus::Failed(message));
    }
    match fields.get("bindingState").copied() {
        Some("empty") if fields.get("bindingCommitment") == Some(&"") => Ok(BindingStatus::Empty),
        Some("failed") if fields.get("bindingCommitment") == Some(&"") => {
            Err("手机绑定失败状态缺少错误码；本机 DPAPI 副本已保留。".into())
        }
        Some("pending") => {
            let commitment = fields.get("bindingCommitment").copied().unwrap_or("");
            if commitment.is_empty() || valid_digest(commitment) {
                Ok(BindingStatus::Pending)
            } else {
                Err("手机绑定摘要无效；本机 DPAPI 副本已保留。".into())
            }
        }
        Some("saved") => {
            let commitment = fields.get("bindingCommitment").copied().unwrap_or("");
            if valid_digest(commitment) {
                Ok(BindingStatus::Saved(commitment.to_ascii_lowercase()))
            } else {
                Err("手机绑定摘要无效；本机 DPAPI 副本已保留。".into())
            }
        }
        _ => Err("手机绑定状态不受支持；本机 DPAPI 副本已保留。".into()),
    }
}

pub async fn status(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    status_with(&SystemAdb(adb), serial, apk, metadata_path, cancel).await
}

pub async fn network_interfaces(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<Vec<ReceiverInterface>, String> {
    let runner = SystemAdb(adb);
    ensure_usb_device(&runner, serial, cancel).await?;
    let user = current_android_user(&runner, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    if !is_installed(&runner, serial, &user, cancel).await? {
        return Err("Receiver 尚未安装。".into());
    }
    verify_installed_identity(&runner, serial, &user, &metadata, cancel).await?;
    let result = runner
        .run(
            Some(serial),
            &[
                "shell",
                "content",
                "query",
                "--user",
                &user,
                "--uri",
                INTERFACES_URI,
                "--projection",
                "address:prefixLength:interfaceIndex:interfaceName",
            ],
            Duration::from_secs(8),
            Some(cancel),
        )
        .await?;
    ensure_adb_success(&result)?;
    Ok(parse_receiver_interfaces(&result.stdout))
}

fn parse_receiver_interfaces(output: &str) -> Vec<ReceiverInterface> {
    let mut interfaces = Vec::new();
    for line in output.lines().filter_map(parse_content_row) {
        let Some(address) = line.get("address") else {
            continue;
        };
        let Some(prefix_length) = line
            .get("prefixLength")
            .and_then(|value| value.parse::<u8>().ok())
        else {
            continue;
        };
        let Some(interface_index) = line
            .get("interfaceIndex")
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let Some(interface_name) = line.get("interfaceName") else {
            continue;
        };
        if address.parse::<std::net::Ipv4Addr>().is_err() || !(1..=32).contains(&prefix_length) {
            continue;
        }
        interfaces.push(ReceiverInterface {
            address: address.clone(),
            prefix_length,
            interface_index,
            interface_name: interface_name.clone(),
        });
    }
    interfaces
}

/// A listener that has bound its UDP port is ready even before the PC authenticates: the P5-C
/// handshake is started by the caller only after this function returns, so `prepared` must count.
fn network_listener_ready(state: &str) -> bool {
    matches!(
        state,
        "prepared" | "authenticated" | "receiverRunning" | "streaming" | "paused"
    )
}

pub async fn start_network(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    address: &str,
    prefix_length: u8,
    cancel: &CancellationToken,
) -> Result<ReceiverNetworkStatus, String> {
    let interfaces = network_interfaces(adb, serial, apk, metadata_path, cancel).await?;
    let selected = interfaces
        .iter()
        .find(|interface| interface.address == address && interface.prefix_length == prefix_length)
        .ok_or_else(|| "所选手机接口已不可用；请重新读取手机接口列表。".to_string())?;
    let _ = selected;
    let runner = SystemAdb(adb);
    let user = current_android_user(&runner, serial, cancel).await?;
    stop_stale_network_listener(&runner, serial, &user, cancel).await;
    let address = address.to_owned();
    let prefix = prefix_length.to_string();
    let result = runner
        .run(
            Some(serial),
            &[
                "shell",
                "am",
                "start",
                "--user",
                &user,
                "-W",
                "-n",
                "app.anpod.receiver/.BootstrapActivity",
                "-a",
                "app.anpod.receiver.action.START_NETWORK_AUDIO",
                "--es",
                "networkAddress",
                &address,
                "--ei",
                "networkPrefixLength",
                &prefix,
            ],
            Duration::from_secs(40),
            Some(cancel),
        )
        .await?;
    ensure_adb_success(&result)?;
    // The installed identity is verified once here; the poll below only re-reads the state row so a
    // 200 ms cadence does not spawn four extra adb commands per tick and starve the same link the
    // listener is starting on.
    let metadata = read_artifact(apk, metadata_path)?;
    verify_installed_identity(&runner, serial, &user, &metadata, cancel).await?;
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        if cancel.is_cancelled() {
            return Err("操作已取消。".into());
        }
        let status = network_status_rows(&runner, serial, &user, cancel).await?;
        if status.network_audio_version != 1 {
            return Err("RECEIVER_UPDATE_REQUIRED:Receiver 网络协议版本不兼容。".into());
        }
        if network_listener_ready(&status.network_audio_state) {
            if status.network_audio_address != address || status.network_audio_port == 0 {
                return Err("Receiver 未报告本次 bootstrap 的监听地址和端口。".into());
            }
            return Ok(status);
        }
        if status.network_audio_state == "failed" {
            return Err(format!(
                "Receiver 网络启动失败：{}。",
                status.network_audio_failure
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Receiver 未能在启动窗口内开始网络监听（手机端状态：{}）。请保持手机解锁并允许通知后重试。",
                status.network_audio_state
            ));
        }
        tokio::select! {
            _ = cancel.cancelled() => return Err("操作已取消。".into()),
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    }
}

pub async fn network_status(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverNetworkStatus, String> {
    let runner = SystemAdb(adb);
    ensure_usb_device(&runner, serial, cancel).await?;
    let user = current_android_user(&runner, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    verify_installed_identity(&runner, serial, &user, &metadata, cancel).await?;
    network_status_rows(&runner, serial, &user, cancel).await
}

/// ReceiverService ignores a new bootstrap while its coordinator still exists, so a listener left
/// over from a failed attempt has to be stopped first or every retry waits on the stale one.
async fn stop_stale_network_listener(
    runner: &impl AdbRunner,
    serial: &str,
    user: &str,
    cancel: &CancellationToken,
) {
    let Ok(status) = network_status_rows(runner, serial, user, cancel).await else {
        return;
    };
    if !matches!(
        status.network_audio_state.as_str(),
        "prepared" | "starting" | "stopping" | "receiverRunning" | "streaming" | "paused"
    ) {
        return;
    }
    let _ = runner
        .run(
            Some(serial),
            &[
                "shell",
                "am",
                "start",
                "--user",
                user,
                "-n",
                "app.anpod.receiver/.BootstrapActivity",
                "-a",
                "app.anpod.receiver.action.STOP",
            ],
            Duration::from_secs(10),
            Some(cancel),
        )
        .await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match network_status_rows(runner, serial, user, cancel).await {
            Ok(state)
                if !matches!(
                    state.network_audio_state.as_str(),
                    "prepared"
                        | "starting"
                        | "stopping"
                        | "receiverRunning"
                        | "streaming"
                        | "paused"
                ) =>
            {
                return
            }
            Ok(_) => {}
            Err(_) => return,
        }
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    }
}

/// Reads only the network state row; callers that already verified identity use this to poll.
async fn network_status_rows(
    runner: &impl AdbRunner,
    serial: &str,
    user: &str,
    cancel: &CancellationToken,
) -> Result<ReceiverNetworkStatus, String> {
    let result = runner.run(Some(serial), &["shell", "content", "query", "--user", user,
        "--uri", NETWORK_URI, "--projection", "networkAudioVersion:networkAudioState:networkAudioFailure:pcmBytesWritten:networkAudioAddress:networkAudioPort"],
        Duration::from_secs(8), Some(cancel)).await?;
    if !result.success {
        return Err("RECEIVER_UPDATE_REQUIRED:Receiver 网络状态版本不可用。".into());
    }
    parse_network_status(&result.stdout)
}

fn parse_network_status(output: &str) -> Result<ReceiverNetworkStatus, String> {
    let row = output
        .lines()
        .find_map(parse_content_row)
        .ok_or_else(|| "手机网络状态格式无效。".to_string())?;
    Ok(ReceiverNetworkStatus {
        network_audio_version: row
            .get("networkAudioVersion")
            .and_then(|value| value.parse().ok())
            .ok_or("手机不支持网络接收。")?,
        network_audio_state: row
            .get("networkAudioState")
            .cloned()
            .ok_or("手机网络状态缺少会话状态。")?,
        network_audio_failure: row.get("networkAudioFailure").cloned().unwrap_or_default(),
        pcm_bytes_written: row
            .get("pcmBytesWritten")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        network_audio_address: row.get("networkAudioAddress").cloned().unwrap_or_default(),
        network_audio_port: row
            .get("networkAudioPort")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
    })
}

fn parse_content_row(line: &str) -> Option<std::collections::HashMap<String, String>> {
    let (index, row) = line.trim().strip_prefix("Row: ")?.split_once(' ')?;
    index.parse::<u32>().ok()?;
    Some(
        row.split(", ")
            .filter_map(|field| {
                let (key, value) = field.split_once('=')?;
                Some((key.to_owned(), value.to_owned()))
            })
            .collect(),
    )
}

async fn status_with(
    adb: &impl AdbRunner,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    ensure_adb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    if !is_installed(adb, serial, &user, cancel).await? {
        return Ok(ReceiverStatus {
            installed: false,
            version_name: None,
            version_code: None,
            service_active: false,
            foreground: false,
            healthy: false,
            audio_active: false,
            operation_owner_id: None,
            operation_generation: None,
            terminal_cause: None,
            terminal_owner_id: None,
            terminal_generation: None,
            playback_diagnostics: None,
            error: None,
            message: "Receiver 尚未安装。".into(),
        });
    }
    let metadata = read_artifact(apk, metadata_path)?;
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    let health = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&health, &installed)?;
    Ok(status_from_health(health))
}

pub async fn install(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
    progress: &mut impl FnMut(InstallProgressStage),
) -> Result<ReceiverStatus, String> {
    install_usb_with(
        &SystemAdb(adb),
        serial,
        apk,
        metadata_path,
        None,
        cancel,
        progress,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn upgrade_usb(
    adb: &Path,
    serial: &str,
    binding_reference: &str,
    binding_path: &Path,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
    progress: &mut impl FnMut(InstallProgressStage),
) -> Result<ReceiverStatus, String> {
    install_usb_with(
        &SystemAdb(adb),
        serial,
        apk,
        metadata_path,
        Some((binding_reference, binding_path)),
        cancel,
        progress,
    )
    .await
}

async fn install_usb_with(
    adb: &impl AdbRunner,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    binding: Option<(&str, &Path)>,
    cancel: &CancellationToken,
    progress: &mut impl FnMut(InstallProgressStage),
) -> Result<ReceiverStatus, String> {
    let (user, metadata, _) =
        prepare_usb_install(adb, serial, apk, metadata_path, binding, cancel).await?;
    let apk_text = apk.to_string_lossy().into_owned();
    progress(InstallProgressStage::Transferring);
    let result = adb
        .run(
            Some(serial),
            &["install", "-r", "--user", &user, &apk_text],
            Duration::from_secs(90),
            Some(cancel),
        )
        .await?;
    ensure_adb_success(&result)?;
    progress(InstallProgressStage::Verifying);
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    if !installed.sha256.eq_ignore_ascii_case(&metadata.sha256) {
        return Err(
            "安装命令成功，但设备上的 Receiver APK 摘要与本次目标包不一致；未报告安装完成。".into(),
        );
    }
    let health = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&health, &installed)?;
    if let Some((_, binding_path)) = binding {
        verify_saved_binding(adb, serial, &user, &metadata, binding_path, cancel).await?;
    }
    Ok(status_from_health(health))
}

async fn prepare_usb_install(
    adb: &impl AdbRunner,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    binding: Option<(&str, &Path)>,
    cancel: &CancellationToken,
) -> Result<(String, ArtifactMetadata, Option<VerifiedReceiverArtifact>), String> {
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
            return Err(
                "已阻止降级或同版本重装：本机 Receiver APK 必须高于设备受信版本；未执行安装。"
                    .into(),
            );
        }
        if !metadata
            .signer_sha256
            .eq_ignore_ascii_case(&installed.signer_sha256)
        {
            return Err("Receiver 签名与设备上的受信版本不一致；未执行安装。".into());
        }
        Some(installed)
    } else {
        None
    };
    if let Some((expected, binding_path)) = binding {
        if binding_path.file_stem().and_then(|value| value.to_str()) != Some(expected) {
            return Err("所选 Receiver 本机绑定材料与 USB 目标不匹配；未执行升级。".into());
        }
        verify_saved_binding(adb, serial, &user, &metadata, binding_path, cancel).await?;
    }
    Ok((user, metadata, installed))
}

pub async fn validate_usb_install(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<(), String> {
    prepare_usb_install(&SystemAdb(adb), serial, apk, metadata_path, None, cancel)
        .await
        .map(|_| ())
}

#[cfg(test)]
async fn install_with(
    adb: &impl AdbRunner,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    install_usb_with(adb, serial, apk, metadata_path, None, cancel, &mut |_| {}).await
}

pub async fn start(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    start_with(&SystemAdb(adb), serial, apk, metadata_path, cancel).await
}

#[allow(clippy::too_many_arguments)]
pub async fn start_audio(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    socket_name: &str,
    connect_code: &str,
    playback_mode: &str,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    start_with_action(
        &SystemAdb(adb),
        serial,
        apk,
        metadata_path,
        cancel,
        Some((socket_name, connect_code, playback_mode)),
    )
    .await
}

async fn start_with(
    adb: &impl AdbRunner,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    start_with_action(adb, serial, apk, metadata_path, cancel, None).await
}

async fn start_with_action(
    adb: &impl AdbRunner,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
    audio: Option<(&str, &str, &str)>,
) -> Result<ReceiverStatus, String> {
    ensure_adb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    if !is_installed(adb, serial, &user, cancel).await? {
        return Err("Receiver 尚未安装。".into());
    }
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    let before = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&before, &installed)?;
    if before.service_active && !before.error.is_empty() {
        return Err(format!("Receiver 启动失败：{}。", before.error));
    }
    if before.healthy && audio.is_none() {
        return Ok(status_from_health(before));
    }

    let mut args = vec![
        "shell".to_string(),
        "am".into(),
        "start".into(),
        "--user".into(),
        user.clone(),
        "-W".into(),
        "-n".into(),
        "app.anpod.receiver/.BootstrapActivity".into(),
        "-a".into(),
        if audio.is_some() {
            "app.anpod.receiver.action.START_AUDIO"
        } else {
            "app.anpod.receiver.action.START"
        }
        .into(),
    ];
    if let Some((socket_name, connect_code, playback_mode)) = audio {
        args.extend([
            "--es".into(),
            "socketName".into(),
            socket_name.into(),
            "--es".into(),
            "connectCode".into(),
            connect_code.into(),
            "--es".into(),
            "playbackMode".into(),
            playback_mode.into(),
        ]);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = adb
        .run(Some(serial), &refs, Duration::from_secs(45), Some(cancel))
        .await?;
    ensure_adb_success(&result)?;

    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if cancel.is_cancelled() {
            return Err("操作已取消。".into());
        }
        let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
        let health = read_health(adb, serial, &user, cancel).await?;
        ensure_version(&health, &installed)?;
        if !health.error.is_empty() {
            return Err(format!("Receiver 启动失败：{}。", health.error));
        }
        if health.healthy {
            return Ok(status_from_health(health));
        }
        if Instant::now() >= deadline {
            return Err(
                "Receiver 尚未报告健康状态；请完成手机上的系统通知授权并检查 Android 启动限制。"
                    .into(),
            );
        }
        tokio::select! {
            _ = cancel.cancelled() => return Err("操作已取消。".into()),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
    }
}

pub async fn health(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    health_with(&SystemAdb(adb), serial, apk, metadata_path, cancel).await
}

async fn health_with(
    adb: &impl AdbRunner,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    ensure_adb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    if !is_installed(adb, serial, &user, cancel).await? {
        return Err("Receiver 尚未安装。".into());
    }
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    let health = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&health, &installed)?;
    Ok(status_from_health(health))
}

pub async fn stop(
    adb: &Path,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    stop_with(&SystemAdb(adb), serial, apk, metadata_path, cancel).await
}

async fn stop_with(
    adb: &impl AdbRunner,
    serial: &str,
    apk: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<ReceiverStatus, String> {
    ensure_adb_device(adb, serial, cancel).await?;
    let user = current_android_user(adb, serial, cancel).await?;
    let metadata = read_artifact(apk, metadata_path)?;
    if !is_installed(adb, serial, &user, cancel).await? {
        return Err("Receiver 尚未安装。".into());
    }
    let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
    let before = read_health(adb, serial, &user, cancel).await?;
    ensure_version(&before, &installed)?;
    if before.application_id != PACKAGE {
        return Err("已安装包名冲突，未执行停止操作。".into());
    }

    if !before.service_active {
        // Nothing is running, so there is no session to clean up and no activity needs to launch.
        return Ok(status_from_health(before));
    }
    let result = adb
        .run(
            Some(serial),
            &[
                "shell",
                "am",
                "start",
                "--user",
                &user,
                "-W",
                "-n",
                "app.anpod.receiver/.BootstrapActivity",
                "-a",
                "app.anpod.receiver.action.STOP",
            ],
            Duration::from_secs(15),
            Some(cancel),
        )
        .await?;
    ensure_adb_success(&result)?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if cancel.is_cancelled() {
            return Err("操作已取消。".into());
        }
        let installed = verify_installed_identity(adb, serial, &user, &metadata, cancel).await?;
        let health = read_health(adb, serial, &user, cancel).await?;
        ensure_version(&health, &installed)?;
        if !health.service_active {
            return Ok(status_from_health(health));
        }
        if Instant::now() >= deadline {
            return Err("Receiver 服务在停止请求后仍处于活动状态。".into());
        }
        tokio::select! {
            _ = cancel.cancelled() => return Err("操作已取消。".into()),
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    }
}

async fn ensure_usb_device(
    adb: &impl AdbRunner,
    serial: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    if serial.is_empty()
        || serial.contains('\r')
        || serial.contains('\n')
        || serial.contains(':')
        || serial.contains("_adb-tls-")
    {
        return Err("Receiver P1 只允许选择已授权的 USB ADB 设备。".into());
    }
    let transport = confirmed_adb_device(adb, serial, cancel).await?;
    if transport != "usb" {
        return Err("所选设备不是已授权的在线 USB 设备。".into());
    }
    Ok(())
}

async fn ensure_adb_device(
    adb: &impl AdbRunner,
    serial: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let transport = confirmed_adb_device(adb, serial, cancel).await?;
    if !matches!(transport.as_str(), "usb" | "wifi") {
        return Err("所选设备不是已授权的在线 USB 或无线 ADB 设备。".into());
    }
    Ok(())
}

async fn confirmed_adb_device(
    adb: &impl AdbRunner,
    serial: &str,
    cancel: &CancellationToken,
) -> Result<String, String> {
    if serial.is_empty() || serial.contains('\r') || serial.contains('\n') {
        return Err("Receiver 只允许选择明确的已授权 ADB 设备。".into());
    }
    let result = adb
        .run(
            None,
            &["devices", "-l"],
            Duration::from_secs(8),
            Some(cancel),
        )
        .await?;
    ensure_adb_success(&result)?;
    let device = parse_devices(&result.stdout)
        .into_iter()
        .find(|device| device.serial == serial)
        .ok_or_else(|| "所选 ADB 设备当前不可用。".to_string())?;
    if device.state != "device" {
        return Err("所选设备不是已授权的在线 ADB 设备。".into());
    }
    let transport = if device.transport == "unknown" {
        let usb = adb
            .run(
                None,
                &["-d", "get-serialno"],
                Duration::from_secs(6),
                Some(cancel),
            )
            .await?;
        ensure_adb_success(&usb)?;
        if usb.stdout.trim() == serial {
            "usb".to_owned()
        } else {
            return Err("无法确认所选设备使用已授权的 USB 或无线 ADB 传输。".into());
        }
    } else {
        device.transport
    };
    Ok(transport)
}

async fn is_installed(
    adb: &impl AdbRunner,
    serial: &str,
    user: &str,
    cancel: &CancellationToken,
) -> Result<bool, String> {
    let result = adb
        .run(
            Some(serial),
            &["shell", "pm", "list", "packages", "--user", user, PACKAGE],
            Duration::from_secs(6),
            Some(cancel),
        )
        .await?;
    if !result.success {
        return Err(adb_error(&result));
    }
    Ok(has_listed_package(&result.stdout))
}

async fn verify_installed_identity(
    adb: &impl AdbRunner,
    serial: &str,
    user: &str,
    metadata: &ArtifactMetadata,
    cancel: &CancellationToken,
) -> Result<VerifiedReceiverArtifact, String> {
    let listed = adb
        .run(
            Some(serial),
            &["shell", "pm", "list", "packages", "--user", user, PACKAGE],
            Duration::from_secs(6),
            Some(cancel),
        )
        .await?;
    ensure_adb_success(&listed)?;
    if !has_listed_package(&listed.stdout) {
        return Err("当前 Android 用户未安装 AnPod Receiver，已拒绝管理操作。".into());
    }
    let path_result = adb
        .run(
            Some(serial),
            &["shell", "pm", "path", "--user", user, PACKAGE],
            Duration::from_secs(6),
            Some(cancel),
        )
        .await?;
    ensure_adb_success(&path_result)?;
    let path = parse_installed_apk_path(&path_result.stdout)
        .ok_or_else(|| "Receiver 安装路径不存在或不受支持，已拒绝管理操作。".to_string())?;
    let hash_result = adb
        .run(
            Some(serial),
            &["shell", "toybox", "sha256sum", path],
            Duration::from_secs(8),
            Some(cancel),
        )
        .await?;
    ensure_adb_success(&hash_result)?;
    let installed_digest = hash_result.stdout.split_whitespace().next().unwrap_or("");
    if !valid_digest(installed_digest) {
        return Err("设备没有返回有效的 Receiver APK 摘要；未读取其状态或执行管理动作。".into());
    }
    verified_installed_artifact(installed_digest, metadata)
        .ok_or_else(|| "RECEIVER_UPDATE_REQUIRED:设备上的 Receiver APK 不在已核验版本中。".into())
}

async fn current_android_user(
    adb: &impl AdbRunner,
    serial: &str,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let result = adb
        .run(
            Some(serial),
            &["shell", "am", "get-current-user"],
            Duration::from_secs(6),
            Some(cancel),
        )
        .await?;
    ensure_adb_success(&result)?;
    let user = result.stdout.trim();
    if user.is_empty() || !user.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Android 未返回有效的当前用户编号；已停止 Receiver 管理。".into());
    }
    Ok(user.to_owned())
}

fn has_listed_package(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.trim() == format!("package:{PACKAGE}"))
}

async fn read_health(
    adb: &impl AdbRunner,
    serial: &str,
    user: &str,
    cancel: &CancellationToken,
) -> Result<Health, String> {
    let uri = HEALTH_URI;
    let result = adb
        .run(
            Some(serial),
            &["shell", "content", "query", "--user", user, "--uri", uri],
            Duration::from_secs(8),
            Some(cancel),
        )
        .await?;
    if !result.success {
        return Err(adb_error(&result));
    }
    parse_health(&result.stdout).ok_or_else(|| "Receiver 健康接口没有返回有效状态记录。".into())
}

fn read_artifact(apk: &Path, metadata_path: &Path) -> Result<ArtifactMetadata, String> {
    let metadata: ArtifactMetadata = serde_json::from_slice(
        &fs::read(metadata_path).map_err(|_| "找不到 Receiver APK 校验清单。".to_string())?,
    )
    .map_err(|_| "Receiver APK 校验清单无效。".to_string())?;
    if metadata.application_id != PACKAGE
        || metadata.version_code == 0
        || metadata.version_name.is_empty()
        || !valid_digest(&metadata.sha256)
        || !valid_digest(&metadata.signer_sha256)
    {
        return Err("Receiver APK 校验清单的包名、版本或摘要格式无效。".into());
    }
    let file_metadata = fs::metadata(apk).map_err(|_| "找不到 Receiver APK。".to_string())?;
    if file_metadata.len() == 0 {
        return Err("Receiver APK 是空文件。".into());
    }
    let mut file = File::open(apk).map_err(|_| "无法读取 Receiver APK。".to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "无法读取 Receiver APK。".to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = format!("{:x}", hasher.finalize());
    if digest != metadata.sha256.to_ascii_lowercase() {
        return Err("Receiver APK SHA-256 与校验清单不一致。".into());
    }
    Ok(metadata)
}

pub(crate) fn local_upgrade_candidate(
    apk: &Path,
    metadata_path: &Path,
) -> Result<(u32, String, u64), String> {
    let metadata = read_artifact(apk, metadata_path)?;
    let size = fs::metadata(apk)
        .map_err(|_| "找不到 Receiver APK。".to_string())?
        .len();
    Ok((metadata.version_code, metadata.version_name, size))
}

pub async fn validate_usb_upgrade(
    adb: &Path,
    serial: &str,
    binding_reference: &str,
    apk: &Path,
    binding_path: &Path,
    metadata_path: &Path,
    cancel: &CancellationToken,
) -> Result<(), String> {
    prepare_usb_install(
        &SystemAdb(adb),
        serial,
        apk,
        metadata_path,
        Some((binding_reference, binding_path)),
        cancel,
    )
    .await
    .map(|_| ())
}

#[cfg(windows)]
async fn verify_saved_binding(
    adb: &impl AdbRunner,
    serial: &str,
    user: &str,
    metadata: &ArtifactMetadata,
    binding_path: &Path,
    cancel: &CancellationToken,
) -> Result<(), String> {
    use crate::security::BindingStore;
    let material = BindingStore::load(binding_path)
        .map_err(|_| "所选 Receiver 的本机绑定材料不可用；未执行升级。".to_string())?;
    let commitment = hex_digest(&material.commitment());
    verify_installed_identity(adb, serial, user, metadata, cancel).await?;
    match read_binding_state(adb, serial, user, cancel).await? {
        BindingStatus::Saved(saved) if saved == commitment => Ok(()),
        BindingStatus::Saved(_) => {
            Err("手机保存的绑定与所选本机材料不一致；没有改写绑定或执行升级。".into())
        }
        BindingStatus::Empty | BindingStatus::Pending => {
            Err("手机没有保存所选绑定；没有创建绑定或执行升级。".into())
        }
        BindingStatus::Failed(message) => Err(message.into()),
    }
}

#[cfg(not(windows))]
async fn verify_saved_binding(
    _adb: &impl AdbRunner,
    _serial: &str,
    _user: &str,
    _metadata: &ArtifactMetadata,
    _binding_path: &Path,
    _cancel: &CancellationToken,
) -> Result<(), String> {
    Err("USB Receiver 升级只在 Windows 上可用。".into())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_installed_apk_path(output: &str) -> Option<&str> {
    let mut paths = output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("package:"));
    let path = paths.next()?.trim();
    if paths.next().is_some()
        || !path.starts_with("/data/app/")
        || !path.ends_with("/base.apk")
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.~=".contains(&byte))
    {
        return None;
    }
    Some(path)
}

fn ensure_version(health: &Health, installed: &VerifiedReceiverArtifact) -> Result<(), String> {
    if health.application_id != PACKAGE {
        return Err("Receiver 包名不匹配。".into());
    }
    if health.version_code != installed.version_code
        || health.version_name != installed.version_name
    {
        return Err("Receiver 健康记录版本与已核验 APK 不一致。".into());
    }
    Ok(())
}

fn status_from_health(health: Health) -> ReceiverStatus {
    let message = if !health.error.is_empty() {
        format!("Receiver 服务报告启动错误：{}。", health.error)
    } else if health.healthy {
        if health.audio_active {
            "Receiver 音频服务正在前台播放。".into()
        } else {
            "Receiver 管理服务正在前台运行；当前没有确认音频播放。".into()
        }
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
        operation_owner_id: health.operation_owner_id,
        operation_generation: health.operation_generation,
        terminal_cause: health.terminal_cause,
        terminal_owner_id: health.terminal_owner_id,
        terminal_generation: health.terminal_generation,
        playback_diagnostics: Some(health.playback_diagnostics),
        error: (!health.error.is_empty()).then_some(health.error),
        message,
    }
}

fn parse_health(output: &str) -> Option<Health> {
    let row = output
        .lines()
        .find(|line| line.trim_start().starts_with("Row: 0 "))?
        .trim();
    let fields: std::collections::HashMap<_, _> = row
        .strip_prefix("Row: 0 ")?
        .split(", ")
        .filter_map(|field| field.split_once('='))
        .collect();
    Some(Health {
        application_id: fields.get("applicationId")?.to_string(),
        version_name: fields.get("versionName")?.to_string(),
        version_code: fields.get("versionCode")?.parse().ok()?,
        service_active: parse_bool(fields.get("serviceActive")?)?,
        foreground: parse_bool(fields.get("foreground")?)?,
        healthy: parse_bool(fields.get("healthy")?)?,
        audio_active: fields
            .get("audioActive")
            .and_then(|value| parse_bool(value))
            .unwrap_or(false),
        operation_owner_id: fields
            .get("operationOwnerId")
            .filter(|value| !value.is_empty() && **value != "unknown")
            .map(|value| value.to_string()),
        operation_generation: parse_optional_metric(fields.get("operationGeneration")),
        terminal_cause: fields
            .get("terminalCause")
            .filter(|value| !value.is_empty() && **value != "unknown")
            .map(|value| value.to_string()),
        terminal_owner_id: fields
            .get("terminalOwnerId")
            .filter(|value| !value.is_empty() && **value != "unknown")
            .map(|value| value.to_string()),
        terminal_generation: parse_optional_metric(fields.get("terminalGeneration")),
        playback_diagnostics: PlaybackDiagnostics {
            receiver_sample_rate: parse_optional_metric(fields.get("actualSampleRate")),
            receiver_channels: parse_optional_metric(fields.get("actualChannels")),
            udp_jitter_target_frames: parse_optional_metric(fields.get("udpJitterTargetFrames")),
            udp_queued_frames: parse_optional_metric(fields.get("udpQueuedFrames")),
            udp_expired_frames: parse_optional_metric(fields.get("udpExpiredFrames")),
            tcp_dropped_backlog_frames: parse_optional_metric(
                fields.get("tcpDroppedBacklogFrames"),
            ),
            track_pending_frames: parse_optional_metric(fields.get("trackPendingFrames")),
            underrun_count: parse_optional_metric(fields.get("underrunCount")),
            // USB uses the cumulative count above; the Provider's historical write-loop delta
            // is not the same interval as the desktop health-query cadence.
            underrun_increment: None,
            playback_advancing: fields
                .get("playbackAdvancing")
                .and_then(|value| parse_bool(value)),
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
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_optional_metric<T: std::str::FromStr>(value: Option<&&str>) -> Option<T> {
    value
        .filter(|value| **value != "unknown")
        .and_then(|value| value.parse().ok())
}

fn ensure_adb_success(result: &AdbResult) -> Result<(), String> {
    if result.success {
        Ok(())
    } else {
        Err(adb_error(result))
    }
}

fn adb_error(result: &AdbResult) -> String {
    let output = format!("{} {}", result.stdout.trim(), result.stderr.trim())
        .trim()
        .to_string();
    let lowered = output.to_ascii_lowercase();
    if lowered.contains("install_failed_update_incompatible") || lowered.contains("signature") {
        return "Android 拒绝更新：现有应用的签名不匹配。没有卸载或更改现有应用。".into();
    }
    if output.is_empty() {
        return "ADB 命令失败；没有移除现有应用。".into();
    }
    let concise: String = output
        .chars()
        .filter(|ch| *ch != '\r' && *ch != '\n')
        .take(800)
        .collect();
    format!("ADB 操作失败：{concise}")
}

#[cfg(test)]
#[path = "receiver_tests.rs"]
mod tests;
