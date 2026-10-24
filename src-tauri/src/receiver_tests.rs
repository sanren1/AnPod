use super::*;
use std::{sync::Mutex, time::SystemTime};

const SERIAL: &str = "FAKE123";
const TEST_USER: &str = "10";
const INSTALLED_PATH: &str = "/data/app/~~AbCdEf0123==/app.anpod.receiver-xYz987==/base.apk";

#[test]
fn receiver_install_progress_matches_frontend_event_contract() {
    let payload = serde_json::to_value(InstallProgress {
        operation_id: "operation-1".into(),
        serial: SERIAL.into(),
        stage: InstallProgressStage::Verifying,
    })
    .unwrap();
    assert_eq!(
        payload,
        serde_json::json!({
            "operationId": "operation-1",
            "serial": SERIAL,
            "stage": "verifying"
        })
    );
}

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
fn network_listener_is_ready_before_authentication_but_not_while_starting() {
    assert!(network_listener_ready("prepared"));
    assert!(network_listener_ready("authenticated"));
    assert!(network_listener_ready("receiverRunning"));
    assert!(network_listener_ready("streaming"));
    assert!(network_listener_ready("paused"));
    assert!(!network_listener_ready("starting"));
    assert!(!network_listener_ready("stopped"));
    assert!(!network_listener_ready("failed"));
}

#[test]
#[cfg(windows)]
fn saved_binding_reference_isolated_by_usb_identity_and_android_user() {
    let first = binding_reference_for("USB-A", "0");
    assert_ne!(first, binding_reference_for("USB-B", "0"));
    assert_ne!(first, binding_reference_for("USB-A", "10"));
}

#[test]
#[cfg(windows)]
fn saved_binding_reference_is_not_the_material_commitment() {
    // Guards the removed check that compared the DPAPI file name with the phone's commitment.
    let reference = binding_reference_for("USB-A", "0");
    let material = crate::security::BindingMaterial::generate().unwrap();
    assert_ne!(reference, hex_digest(&material.commitment()));
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

    fn calls(&self) -> Vec<Vec<String>> {
        self.0.lock().unwrap().calls.clone()
    }
    fn serial_calls(&self) -> Vec<Option<String>> {
        self.0.lock().unwrap().serial_calls.clone()
    }

    fn call_index(&self, prefix: &[&str]) -> Option<usize> {
        self.0.lock().unwrap().calls.iter().position(|call| {
            call.starts_with(
                &prefix
                    .iter()
                    .map(|item| (*item).to_string())
                    .collect::<Vec<_>>(),
            )
        })
    }
}

impl AdbRunner for FakeAdb {
    fn run<'a>(
        &'a self,
        serial: Option<&'a str>,
        args: &'a [&'a str],
        _limit: Duration,
        _cancel: Option<&'a CancellationToken>,
    ) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a {
        async move {
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            let mut state = self.0.lock().unwrap();
            state.calls.push(args.clone());
            state.serial_calls.push(serial.map(str::to_owned));
            let result = match args.as_slice() {
                [command, _] if command == "devices" => {
                    let transport = "product:test";
                    AdbResult {
                        success: true,
                        stdout: format!(
                            "List of devices attached\n{} {} {transport}\n",
                            state.device_serial, state.device_state
                        ),
                        stderr: String::new(),
                    }
                }
                [selector, command] if selector == "-d" && command == "get-serialno" => AdbResult {
                    success: state.usb,
                    stdout: if state.usb {
                        format!("{}\n", state.device_serial)
                    } else {
                        String::new()
                    },
                    stderr: String::new(),
                },
                [shell, am, command]
                    if shell == "shell" && am == "am" && command == "get-current-user" =>
                {
                    AdbResult {
                        success: true,
                        stdout: format!("{}\n", state.user),
                        stderr: String::new(),
                    }
                }
                [shell, pm, list, packages, user_option, user, package]
                    if shell == "shell"
                        && pm == "pm"
                        && list == "list"
                        && packages == "packages"
                        && user_option == "--user"
                        && user == state.user
                        && package == PACKAGE =>
                {
                    let stdout = if state.installed {
                        format!("package:{PACKAGE}\n")
                    } else {
                        String::new()
                    };
                    AdbResult {
                        success: true,
                        stdout,
                        stderr: String::new(),
                    }
                }
                [shell, pm, path, user_option, user, package]
                    if shell == "shell"
                        && pm == "pm"
                        && path == "path"
                        && user_option == "--user"
                        && user == state.user
                        && package == PACKAGE =>
                {
                    let stdout = if state.installed {
                        format!("package:{INSTALLED_PATH}\n")
                    } else {
                        String::new()
                    };
                    AdbResult {
                        success: true,
                        stdout,
                        stderr: String::new(),
                    }
                }
                [shell, toybox, sha256sum, _]
                    if shell == "shell" && toybox == "toybox" && sha256sum == "sha256sum" =>
                {
                    AdbResult {
                        success: state.installed,
                        stdout: format!("{}  {INSTALLED_PATH}\n", state.installed_hash),
                        stderr: String::new(),
                    }
                }
                [shell, content, query, user_option, user, flag, uri]
                    if shell == "shell"
                        && content == "content"
                        && query == "query"
                        && user_option == "--user"
                        && user == state.user
                        && flag == "--uri"
                        && uri == HEALTH_URI =>
                {
                    let stdout = format!("Row: 0 applicationId={PACKAGE}, versionName={}, versionCode={}, serviceActive={}, foreground={}, healthy={}, audioActive={}, startedAtElapsedMs=1, error={}\n", state.health_version_name, state.health_version_code, state.active, state.foreground, state.healthy, state.audio_active, state.error);
                    AdbResult {
                        success: true,
                        stdout,
                        stderr: String::new(),
                    }
                }
                [shell, content, query, user_option, user, flag, uri, projection_flag, projection]
                    if shell == "shell"
                        && content == "content"
                        && query == "query"
                        && user_option == "--user"
                        && user == state.user
                        && flag == "--uri"
                        && uri == BINDING_URI
                        && projection_flag == "--projection"
                        && projection == "bindingState:bindingCommitment:bindingFailure" =>
                {
                    if state.binding_query_failed {
                        return Ok(AdbResult {
                            success: false,
                            stdout: String::new(),
                            stderr: "provider unavailable".into(),
                        });
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
                    if shell == "shell"
                        && content == "content"
                        && query == "query"
                        && user_option == "--user"
                        && user == state.user
                        && flag == "--uri"
                        && uri == BINDING_URI
                        && projection_flag == "--projection"
                        && projection == "bindingResetVersion" =>
                {
                    AdbResult {
                        success: state.binding_reset_supported,
                        stdout: if state.binding_reset_supported {
                            "Row: 0 bindingResetVersion=1\n".into()
                        } else {
                            String::new()
                        },
                        stderr: String::new(),
                    }
                }
                [shell, content, query, user_option, user, flag, uri, projection_flag, projection]
                    if shell == "shell"
                        && content == "content"
                        && query == "query"
                        && user_option == "--user"
                        && user == state.user
                        && flag == "--uri"
                        && uri == NETWORK_URI
                        && projection_flag == "--projection"
                        && projection == "networkAudioVersion:networkAudioState" =>
                {
                    AdbResult {
                        success: true,
                        stdout: format!(
                            "Row: 0 networkAudioVersion=1, networkAudioState={}\n",
                            state.network_audio_state
                        ),
                        stderr: String::new(),
                    }
                }
                [shell, content, delete, user_option, user, uri_option, uri, where_option, selection]
                    if shell == "shell"
                        && content == "content"
                        && delete == "delete"
                        && user_option == "--user"
                        && user == state.user
                        && uri_option == "--uri"
                        && uri == BINDING_URI
                        && where_option == "--where" =>
                {
                    let matches = match (
                        selection.strip_prefix("expectedCommitment="),
                        selection.as_str(),
                    ) {
                        (Some(expected), _) => {
                            state.binding_commitment.as_deref() == Some(expected)
                        }
                        (None, "expectedEmpty") => state.binding_commitment.is_none(),
                        _ => false,
                    };
                    if matches && !state.binding_reset_noop {
                        state.binding_commitment = None;
                    }
                    if matches && state.binding_query_fail_after_delete {
                        state.binding_query_failed = true;
                    }
                    // content may return success even when the provider did not change the row.
                    AdbResult {
                        success: true,
                        stdout: "0 rows deleted\n".into(),
                        stderr: String::new(),
                    }
                }
                [install, replace, user_option, user, _apk]
                    if install == "install"
                        && replace == "-r"
                        && user_option == "--user"
                        && user == state.user =>
                {
                    if state.install_fails {
                        AdbResult {
                            success: false,
                            stdout: String::new(),
                            stderr: "INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match"
                                .into(),
                        }
                    } else {
                        state.installed = true;
                        if let Some(digest) = state.install_digest.take() {
                            state.installed_hash = digest;
                        }
                        if let Some(version_code) = state.install_version_code.take() {
                            state.health_version_code = version_code;
                        }
                        if let Some(version_name) = state.install_version_name.take() {
                            state.health_version_name = version_name;
                        }
                        AdbResult {
                            success: true,
                            stdout: "Success".into(),
                            stderr: String::new(),
                        }
                    }
                }
                values
                    if values.windows(2).any(|pair| {
                        pair[0] == "-a" && pair[1] == "app.anpod.receiver.action.START"
                    }) =>
                {
                    state.active = true;
                    state.foreground = true;
                    state.healthy = true;
                    AdbResult {
                        success: true,
                        stdout: "Status: ok".into(),
                        stderr: String::new(),
                    }
                }
                values
                    if values.windows(2).any(|pair| {
                        pair[0] == "-a" && pair[1] == "app.anpod.receiver.action.START_AUDIO"
                    }) =>
                {
                    state.active = true;
                    state.foreground = true;
                    state.healthy = true;
                    AdbResult {
                        success: true,
                        stdout: "Status: ok".into(),
                        stderr: String::new(),
                    }
                }
                values
                    if values.windows(2).any(|pair| {
                        pair[0] == "-a" && pair[1] == "app.anpod.receiver.action.STOP"
                    }) =>
                {
                    state.active = false;
                    state.foreground = false;
                    state.healthy = false;
                    AdbResult {
                        success: true,
                        stdout: "Status: ok".into(),
                        stderr: String::new(),
                    }
                }
                [shell, getprop, key]
                    if shell == "shell"
                        && getprop == "getprop"
                        && key == "ro.build.version.sdk" =>
                {
                    AdbResult {
                        success: true,
                        stdout: "34\n".into(),
                        stderr: String::new(),
                    }
                }
                _ => AdbResult {
                    success: false,
                    stdout: String::new(),
                    stderr: format!("unexpected fake ADB args: {args:?}"),
                },
            };
            Ok(result)
        }
    }

    fn run_with_stdin<'a>(
        &'a self,
        serial: Option<&'a str>,
        args: &'a [&'a str],
        input: &'a [u8],
        _limit: Duration,
        cancel: &'a CancellationToken,
    ) -> impl Future<Output = Result<AdbResult, String>> + Send + 'a {
        async move {
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            let mut state = self.0.lock().unwrap();
            state.calls.push(args.clone());
            state.serial_calls.push(serial.map(str::to_owned));
            let target_matches = args.len() == 8
                && args[0] == "shell"
                && args[1] == "-T"
                && args[2] == "content"
                && args[3] == "write"
                && args[4] == "--user"
                && args[5] == state.user
                && args[6] == "--uri"
                && args[7] == BINDING_URI;
            if serial != Some(SERIAL) || !target_matches {
                return Ok(AdbResult {
                    success: false,
                    stdout: String::new(),
                    stderr: "invalid fake provision target".into(),
                });
            }
            let commitment = format!("{:x}", Sha256::digest(input));
            state.binding_input_commitments.push(commitment.clone());
            match state.binding_write_mode {
                FakeBindingWrite::Store => {
                    if state
                        .binding_commitment
                        .as_deref()
                        .is_none_or(|existing| existing == commitment)
                    {
                        state.binding_commitment = Some(commitment);
                    }
                    Ok(AdbResult {
                        success: true,
                        stdout: String::new(),
                        stderr: String::new(),
                    })
                }
                FakeBindingWrite::Cancel => {
                    state.binding_write_mode = FakeBindingWrite::Store;
                    cancel.cancel();
                    Err("操作已取消。".into())
                }
                FakeBindingWrite::ErrorText => Ok(AdbResult {
                    success: true,
                    stdout: String::new(),
                    stderr: format!(
                        "Error while accessing provider: {}",
                        input[37..69]
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    ),
                }),
                FakeBindingWrite::WrongUser => {
                    state.user = "11";
                    Ok(AdbResult {
                        success: true,
                        stdout: String::new(),
                        stderr: String::new(),
                    })
                }
            }
        }
    }
}

struct ArtifactFiles {
    dir: std::path::PathBuf,
    apk: std::path::PathBuf,
    metadata: std::path::PathBuf,
    digest: String,
}

impl ArtifactFiles {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "anpod-receiver-test-{}-{unique}-{:?}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&dir).unwrap();
        let apk = dir.join("receiver.apk");
        let metadata = dir.join("receiver.json");
        let bytes = b"verified fake receiver apk";
        fs::write(&apk, bytes).unwrap();
        let digest = format!("{:x}", Sha256::digest(bytes));
        fs::write(
            &metadata,
            serde_json::json!({
                "applicationId": PACKAGE, "versionCode": 1, "versionName": "0.1.0-p1",
                "sha256": digest, "signerSha256": "a".repeat(64)
            })
            .to_string(),
        )
        .unwrap();
        Self {
            dir,
            apk,
            metadata,
            digest,
        }
    }
}

impl Drop for ArtifactFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(windows)]
fn binding_reset_fixture(
    files: &ArtifactFiles,
    phone: Option<&crate::security::BindingMaterial>,
    pc: Option<&crate::security::BindingMaterial>,
) -> (FakeAdb, std::path::PathBuf, String) {
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
    use crate::security::{
        test_binding_material, test_binding_material_with_wrong_secret, write_test_binding_file,
    };

    for (phone_present, pc_present, conflict) in [
        (false, false, false),
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        let files = ArtifactFiles::new();
        let phone = test_binding_material();
        let pc = if conflict {
            test_binding_material_with_wrong_secret()
        } else {
            test_binding_material()
        };
        let other = test_binding_material_with_wrong_secret();
        let (adb, data_dir, reference) = binding_reset_fixture(
            &files,
            phone_present.then_some(&phone),
            pc_present.then_some(&pc),
        );
        let binding_dir = data_dir.join("receiver-bindings");
        let other_path = binding_dir.join(format!("{}.dat", "b".repeat(64)));
        write_test_binding_file(&other_path, &other).unwrap();

        let preview = binding_reset_preview_with(
            &adb,
            SERIAL,
            &data_dir,
            &files.apk,
            &files.metadata,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(preview.binding_reference, reference);
        let result = reset_binding_with(
            &adb,
            &preview,
            &data_dir,
            &files.apk,
            &files.metadata,
            &CancellationToken::new(),
            &|| true,
        )
        .await
        .unwrap();
        assert_eq!(
            result.outcome,
            if phone_present || pc_present {
                "reset"
            } else {
                "alreadyEmpty"
            }
        );
        assert_eq!(adb.0.lock().unwrap().binding_commitment, None);
        assert!(
            other_path.exists(),
            "an unrelated PC binding must survive reset"
        );
        assert!(!binding_dir.join(format!("{reference}.dat")).exists());
        let deletes = adb
            .calls()
            .iter()
            .filter(|call| call.get(2).map(String::as_str) == Some("delete"))
            .count();
        assert_eq!(
            deletes,
            usize::from(phone_present || pc_present),
            "both-empty reset is read-only"
        );
    }
}

#[cfg(windows)]
#[tokio::test]
async fn binding_reset_requires_capability_identity_current_commitments_and_idle_receiver() {
    use crate::security::test_binding_material;
    let files = ArtifactFiles::new();
    let material = test_binding_material();
    let (adb, data_dir, reference) =
        binding_reset_fixture(&files, Some(&material), Some(&material));
    let binding_path = data_dir
        .join("receiver-bindings")
        .join(format!("{reference}.dat"));

    adb.0.lock().unwrap().binding_reset_supported = false;
    let unsupported = binding_reset_preview_with(
        &adb,
        SERIAL,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(unsupported.contains("不支持条件绑定重置"));
    assert!(adb.0.lock().unwrap().binding_commitment.is_some());
    assert!(binding_path.exists());

    adb.0.lock().unwrap().binding_reset_supported = true;
    let preview = binding_reset_preview_with(
        &adb,
        SERIAL,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    adb.0.lock().unwrap().device_state = "unauthorized";
    assert!(reset_binding_with(
        &adb,
        &preview,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        &|| true
    )
    .await
    .is_err());
    assert!(!adb
        .calls()
        .iter()
        .any(|call| call.get(2).map(String::as_str) == Some("delete")));
    assert!(binding_path.exists());

    adb.0.lock().unwrap().device_state = "device";
    let preview = binding_reset_preview_with(
        &adb,
        SERIAL,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    adb.0.lock().unwrap().user = "11";
    assert!(reset_binding_with(
        &adb,
        &preview,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        &|| true
    )
    .await
    .is_err());
    assert!(binding_path.exists());
    adb.0.lock().unwrap().user = TEST_USER;

    let preview = binding_reset_preview_with(
        &adb,
        SERIAL,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    adb.0.lock().unwrap().binding_commitment = Some("f".repeat(64));
    assert!(reset_binding_with(
        &adb,
        &preview,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        &|| true
    )
    .await
    .is_err());
    assert!(binding_path.exists());

    adb.0.lock().unwrap().binding_commitment = Some(hex_digest(&material.commitment()));
    adb.0.lock().unwrap().network_audio_state = "streaming".into();
    let busy = binding_reset_preview_with(
        &adb,
        SERIAL,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(busy.receiver_busy);
    assert!(reset_binding_with(
        &adb,
        &busy,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        &|| true
    )
    .await
    .is_err());
    assert_eq!(
        adb.calls()
            .iter()
            .filter(|call| call.get(2).map(String::as_str) == Some("delete"))
            .count(),
        0
    );
    assert!(binding_path.exists());
}

#[cfg(windows)]
#[tokio::test]
async fn binding_reset_reads_back_exit_zero_and_keeps_a_recoverable_partial_result() {
    use crate::security::{
        test_binding_material, test_binding_material_with_wrong_secret, write_test_binding_file,
    };
    let files = ArtifactFiles::new();
    let material = test_binding_material();
    let (adb, data_dir, reference) =
        binding_reset_fixture(&files, Some(&material), Some(&material));
    let binding_path = data_dir
        .join("receiver-bindings")
        .join(format!("{reference}.dat"));
    let preview = binding_reset_preview_with(
        &adb,
        SERIAL,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    adb.0.lock().unwrap().binding_reset_noop = true;
    let retained = reset_binding_with(
        &adb,
        &preview,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        &|| true,
    )
    .await
    .unwrap();
    assert_eq!(retained.outcome, "phoneRetained");
    assert_eq!(retained.phone_state, "saved");
    assert!(binding_path.exists());

    adb.0.lock().unwrap().binding_reset_noop = false;
    let preview = binding_reset_preview_with(
        &adb,
        SERIAL,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = reset_binding_with(
        &adb,
        &preview,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        &|| checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0,
    )
    .await
    .unwrap();
    assert_eq!(cancelled.outcome, "phoneClearedPcRetained");
    assert_eq!(cancelled.phone_state, "empty");
    assert_eq!(cancelled.pc_state, "saved");
    assert!(
        binding_path.exists(),
        "cancellation after phone removal must preserve the DPAPI retry copy"
    );
    assert_eq!(adb.0.lock().unwrap().binding_commitment, None);

    let files = ArtifactFiles::new();
    let material = test_binding_material();
    let (adb, data_dir, reference) =
        binding_reset_fixture(&files, Some(&material), Some(&material));
    let binding_path = data_dir
        .join("receiver-bindings")
        .join(format!("{reference}.dat"));
    adb.0.lock().unwrap().binding_query_fail_after_delete = true;
    let preview = binding_reset_preview_with(
        &adb,
        SERIAL,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let unknown = reset_binding_with(
        &adb,
        &preview,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        &|| true,
    )
    .await
    .unwrap();
    assert_eq!(unknown.outcome, "phoneStateUnknown");
    assert_eq!(unknown.phone_state, "unknown");
    assert_eq!(unknown.pc_state, "saved");
    assert!(
        binding_path.exists(),
        "failed phone readback must leave the local retry copy intact"
    );
    assert_eq!(
        adb.0.lock().unwrap().binding_commitment,
        None,
        "the fake phone did clear the binding although readback became unavailable"
    );

    let files = ArtifactFiles::new();
    let material = test_binding_material();
    let changed_material = test_binding_material_with_wrong_secret();
    let (adb, data_dir, reference) =
        binding_reset_fixture(&files, Some(&material), Some(&material));
    let binding_path = data_dir
        .join("receiver-bindings")
        .join(format!("{reference}.dat"));
    let preview = binding_reset_preview_with(
        &adb,
        SERIAL,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let failed = reset_binding_with(
        &adb,
        &preview,
        &data_dir,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        &|| {
            if checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 2 {
                write_test_binding_file(&binding_path, &changed_material).unwrap();
            }
            true
        },
    )
    .await
    .unwrap();
    assert_eq!(failed.outcome, "phoneClearedPcChanged");
    assert_eq!(failed.phone_state, "empty");
    assert_eq!(failed.pc_state, "saved");
    assert_eq!(
        failed.pc_commitment,
        Some(hex_digest(&changed_material.commitment())),
        "a failed PC CAS must report the current record commitment, not the stale preview"
    );
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
fn user_stop_requires_matching_receiver_owner_and_generation() {
    let health = parse_health("Row: 0 applicationId=app.anpod.receiver, versionName=0.1.0-p1, versionCode=1, serviceActive=true, foreground=true, healthy=true, audioActive=true, operationOwnerId=owner-a, operationGeneration=7, terminalCause=USER_STOP, terminalOwnerId=owner-a, terminalGeneration=7, error=").unwrap();
    let status = status_from_health(health);
    assert!(status.was_stopped_by_user_for(Some("owner-a"), Some(7)));
    assert!(!status.was_stopped_by_user_for(Some("owner-a"), Some(6)));
    assert!(!status.was_stopped_by_user_for(Some("owner-b"), Some(7)));
}

#[test]
fn installed_and_not_installed_package_responses_are_distinct() {
    assert_eq!(
        parse_installed_apk_path(
            "package:/data/app/~~AbCdEf0123==/app.anpod.receiver-xYz987==/base.apk\n"
        ),
        Some("/data/app/~~AbCdEf0123==/app.anpod.receiver-xYz987==/base.apk")
    );
    assert_eq!(
        parse_installed_apk_path(
            "package:/data/app/~~opaque/app.anpod.receiver/base.apk with space\n"
        ),
        None
    );
    assert_eq!(
        parse_installed_apk_path("package:/data/app/~~opaque/app.anpod.receiver/base.apk;id\n"),
        None
    );
    assert_eq!(
        parse_installed_apk_path("package:/data/app/a/base.apk\npackage:/data/app/a/split.apk\n"),
        None
    );
    assert_eq!(parse_installed_apk_path("\n"), None);
}

#[test]
fn reads_the_artifact_written_by_windows_powershell_when_requested() {
    let Some(directory) = std::env::var_os("ANPOD_RECEIVER_ARTIFACT_DIR") else {
        return;
    };
    let directory = Path::new(&directory);
    read_artifact(
        &directory.join("receiver.apk"),
        &directory.join("receiver.json"),
    )
    .unwrap();
}

#[test]
fn recognizes_audited_code12_and_code13_for_code14_upgrade_but_rejects_unknown_apks() {
    let current = ArtifactMetadata {
        application_id: PACKAGE.into(),
        version_code: 14,
        version_name: "0.11.0-v3-flow-candidate".into(),
        sha256: "a".repeat(64),
        signer_sha256: "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439".into(),
        source_commit: None,
    };
    for (digest, version_code, version_name) in [
        (
            "1791bf24f848e7470206b75151b081c05b4e5fd2af3e92c67f2077cda9cdbdbd",
            12,
            "0.9.1-v3-flow-candidate",
        ),
        (
            "de64b3473a7848481e9ff75e98e3171dd7052b0b44f40b09d4fdaffdd669de3f",
            13,
            "0.10.0-v3-flow-candidate",
        ),
    ] {
        let installed = verified_installed_artifact(digest, &current).unwrap();
        assert_eq!(installed.version_code, version_code);
        assert_eq!(installed.version_name, version_name);
    }
    assert!(verified_installed_artifact(&"e".repeat(64), &current).is_none());
}

#[test]
fn signature_mismatch_is_reported_without_an_uninstall_fallback() {
    let result = AdbResult {
        success: false,
        stdout: "Performing Streamed Install".into(),
        stderr: "INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match".into(),
    };
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
    assert_eq!(
        health.playback_diagnostics.receiver_sample_rate,
        Some(48_000)
    );
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
    assert!(status
        .message
        .contains("ForegroundServiceStartNotAllowedException"));
}

#[tokio::test]
async fn fake_adb_normal_install_health_start_stop_flow_checks_installed_apk_each_time() {
    let files = ArtifactFiles::new();
    let adb = FakeAdb::new(false, files.digest.clone());
    let cancel = CancellationToken::new();
    assert!(
        !install_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel)
            .await
            .unwrap()
            .healthy
    );
    assert!(
        !health_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel)
            .await
            .unwrap()
            .healthy
    );
    assert!(
        start_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel)
            .await
            .unwrap()
            .healthy
    );
    assert!(
        !stop_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel)
            .await
            .unwrap()
            .service_active
    );
    let calls = adb.calls();
    let install = adb.call_index(&["install", "-r"]).unwrap();
    let first_hash = adb.call_index(&["shell", "toybox", "sha256sum"]).unwrap();
    let first_health = adb.call_index(&["shell", "content", "query"]).unwrap();
    let start = calls
        .iter()
        .position(|call| {
            call.iter()
                .any(|arg| arg == "app.anpod.receiver.action.START")
        })
        .unwrap();
    let stop = calls
        .iter()
        .position(|call| {
            call.iter()
                .any(|arg| arg == "app.anpod.receiver.action.STOP")
        })
        .unwrap();
    assert!(
        install < first_hash && first_hash < first_health && first_health < start && start < stop
    );
    assert!(
        calls
            .iter()
            .filter(|call| call.starts_with(&["shell".into(), "toybox".into(), "sha256sum".into()]))
            .count()
            >= 4
    );
    for call in &calls {
        if call.starts_with(&["shell".into(), "pm".into(), "list".into()])
            || call.starts_with(&["shell".into(), "pm".into(), "path".into()])
            || call.starts_with(&["shell".into(), "content".into(), "query".into()])
            || call.starts_with(&["shell".into(), "am".into(), "start".into()])
            || call.starts_with(&["install".into()])
        {
            assert!(
                call.windows(2)
                    .any(|pair| pair[0] == "--user" && pair[1] == TEST_USER),
                "wrong Android user in {call:?}"
            );
        }
    }
    assert!(calls.iter().any(|call| call.starts_with(&[
        "shell".into(),
        "content".into(),
        "query".into(),
        "--user".into(),
        TEST_USER.into()
    ])));
}

#[tokio::test]
async fn fake_adb_wireless_receiver_start_accepts_audited_newer_install() {
    let files = ArtifactFiles::new();
    fs::write(
        &files.metadata,
        serde_json::json!({
            "applicationId": PACKAGE,
            "versionCode": 2,
            "versionName": "0.2.0-p2-candidate",
            "sha256": files.digest,
            "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439"
        })
        .to_string(),
    )
    .unwrap();
    let serial = "192.0.2.20:5555";
    let adb = FakeAdb::with_serial(
        true,
        "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into(),
        serial,
    );
    {
        let mut state = adb.0.lock().unwrap();
        state.health_version_code = 3;
        state.health_version_name = "0.3.0-v2-candidate".into();
    }
    let status = start_with_action(
        &adb,
        serial,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        Some(("wireless-session", "test-code", "balanced")),
    )
    .await
    .unwrap();
    assert_eq!(status.version_code, Some(3));
    assert!(adb.calls().iter().any(|call| call
        .iter()
        .any(|arg| arg == "app.anpod.receiver.action.START_AUDIO")));
}

#[tokio::test]
async fn fake_adb_install_refuses_older_local_receiver_before_install() {
    let files = ArtifactFiles::new();
    fs::write(
        &files.metadata,
        serde_json::json!({
            "applicationId": PACKAGE,
            "versionCode": 2,
            "versionName": "0.2.0-p2-candidate",
            "sha256": files.digest,
            "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439"
        })
        .to_string(),
    )
    .unwrap();
    let adb = FakeAdb::new(
        true,
        "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into(),
    );
    let error = install_with(
        &adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .err()
    .unwrap();
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
        fs::write(
            &files.metadata,
            serde_json::json!({
                "applicationId": PACKAGE,
                "versionCode": 12,
                "versionName": version_name,
                "sha256": files.digest,
                "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439"
            })
            .to_string(),
        )
        .unwrap();
        let adb = FakeAdb::new(true, old_digest.into());
        {
            let mut state = adb.0.lock().unwrap();
            state.install_digest = Some(files.digest.clone());
            state.install_version_code = Some(12);
            state.install_version_name = Some(version_name.into());
        }

        let mut stages = Vec::new();
        let status = install_usb_with(
            &adb,
            SERIAL,
            &files.apk,
            &files.metadata,
            None,
            &CancellationToken::new(),
            &mut |stage| stages.push(stage),
        )
        .await
        .unwrap();

        assert_eq!(status.version_code, Some(12));
        assert_eq!(status.version_name.as_deref(), Some(version_name));
        assert_eq!(
            stages,
            vec![
                InstallProgressStage::Transferring,
                InstallProgressStage::Verifying
            ]
        );
        assert!(adb.call_index(&["install", "-r"]).is_some());
        assert!(!adb
            .calls()
            .iter()
            .any(|call| call.first().is_some_and(|arg| arg == "uninstall")));
    }
}

#[cfg(windows)]
#[tokio::test]
async fn fake_adb_usb_upgrade_requires_the_saved_binding_and_preserves_it() {
    use crate::security::{test_binding_material, BindingStore};

    let files = ArtifactFiles::new();
    let version_name = "0.7.0-v3-flow-candidate";
    fs::write(
        &files.metadata,
        serde_json::json!({
            "applicationId": PACKAGE,
            "versionCode": 9,
            "versionName": version_name,
            "sha256": files.digest,
            "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439"
        })
        .to_string(),
    )
    .unwrap();
    let reference = binding_reference_for(SERIAL, TEST_USER);
    let binding_path = files.dir.join(format!("{reference}.dpapi"));
    let material = test_binding_material();
    let commitment = hex_digest(&material.commitment());
    BindingStore::create_if_absent(&binding_path, &material).unwrap();

    let adb = FakeAdb::new(
        true,
        "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into(),
    );
    {
        let mut state = adb.0.lock().unwrap();
        state.binding_commitment = Some(commitment.clone());
        state.install_digest = Some(files.digest.clone());
        state.install_version_code = Some(9);
        state.install_version_name = Some(version_name.into());
    }
    let status = install_usb_with(
        &adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        Some((&reference, &binding_path)),
        &CancellationToken::new(),
        &mut |_| {},
    )
    .await
    .unwrap();
    assert_eq!(status.version_code, Some(9));
    assert_eq!(
        adb.0.lock().unwrap().binding_commitment.as_deref(),
        Some(commitment.as_str())
    );
    assert!(adb.call_index(&["install", "-r"]).is_some());
    let calls = adb.calls();
    let binding_queries: Vec<_> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| {
            call.windows(2)
                .any(|pair| pair[0] == "--uri" && pair[1] == BINDING_URI)
        })
        .map(|(index, _)| index)
        .collect();
    assert!(
        binding_queries.len() >= 2,
        "binding must match both before and after package installation"
    );
    assert!(calls.iter().all(|call| !call
        .windows(2)
        .any(|pair| pair[0] == "content" && pair[1] == "write")));
    assert!(adb
        .serial_calls()
        .iter()
        .any(|serial| serial.as_deref() == Some(SERIAL)));
    assert!(adb
        .serial_calls()
        .iter()
        .all(|serial| serial.as_deref().is_none_or(|serial| serial == SERIAL)));

    for remote_commitment in [Some("f".repeat(64)), None] {
        let blocked = FakeAdb::new(
            true,
            "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into(),
        );
        blocked.0.lock().unwrap().binding_commitment = remote_commitment;
        let error = install_usb_with(
            &blocked,
            SERIAL,
            &files.apk,
            &files.metadata,
            Some((&reference, &binding_path)),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .await
        .err()
        .unwrap();
        assert!(error.contains("绑定"));
        let calls = blocked.calls();
        assert!(!calls
            .iter()
            .any(|call| call.first().is_some_and(|arg| arg == "install")));
        assert!(!calls.iter().any(|call| call
            .windows(2)
            .any(|pair| pair[0] == "content" && pair[1] == "write")));
    }

    let changed_user = FakeAdb::new(
        true,
        "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into(),
    );
    changed_user.0.lock().unwrap().user = "11";
    assert!(install_usb_with(
        &changed_user,
        SERIAL,
        &files.apk,
        &files.metadata,
        Some((&reference, &binding_path)),
        &CancellationToken::new(),
        &mut |_| {}
    )
    .await
    .is_err());
    assert!(!changed_user
        .calls()
        .iter()
        .any(|call| call.first().is_some_and(|arg| arg == "install")));
    assert!(!changed_user.calls().iter().any(|call| call
        .windows(2)
        .any(|pair| pair[0] == "content" && pair[1] == "write")));

    let wireless_serial = "192.0.2.15:5555";
    let wireless = FakeAdb::with_serial(
        true,
        "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into(),
        wireless_serial,
    );
    wireless.0.lock().unwrap().usb = false;
    assert!(install_usb_with(
        &wireless,
        wireless_serial,
        &files.apk,
        &files.metadata,
        Some((&reference, &binding_path)),
        &CancellationToken::new(),
        &mut |_| {}
    )
    .await
    .is_err());
    assert!(!wireless
        .calls()
        .iter()
        .any(|call| call.first().is_some_and(|arg| arg == "install")));
    assert!(!wireless.calls().iter().any(|call| call
        .windows(2)
        .any(|pair| pair[0] == "content" && pair[1] == "write")));
}

#[tokio::test]
async fn fake_adb_audio_start_sends_session_handshake_and_playback_mode() {
    let files = ArtifactFiles::new();
    let adb = FakeAdb::new(true, files.digest.clone());
    let status = start_with_action(
        &adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        Some(("anpod_test_session", "test_token", "lowlatency")),
    )
    .await
    .unwrap();
    assert!(status.healthy);
    let audio_start = adb
        .calls()
        .into_iter()
        .find(|call| {
            call.iter()
                .any(|arg| arg == "app.anpod.receiver.action.START_AUDIO")
        })
        .unwrap();
    for expected in [
        "socketName",
        "anpod_test_session",
        "connectCode",
        "test_token",
        "playbackMode",
        "lowlatency",
    ] {
        assert!(
            audio_start.iter().any(|arg| arg == expected),
            "missing {expected} in {audio_start:?}"
        );
    }
    assert!(audio_start
        .windows(2)
        .any(|pair| pair[0] == "--user" && pair[1] == TEST_USER));
    // ReceiverService is unexported, so the shell must go through the DUMP-protected, invisible
    // BootstrapActivity trampoline and must never try to start the service component directly.
    assert!(audio_start
        .iter()
        .any(|arg| arg == "app.anpod.receiver/.BootstrapActivity"));
    assert!(!audio_start
        .iter()
        .any(|arg| arg == "start-foreground-service" || arg == "startservice"));
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
    let result = start_with_action(
        &adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
        Some(("receiver-test", "test-code", "balanced")),
    )
    .await;
    assert!(result
        .err()
        .unwrap()
        .contains("AudioTrackInitializationFailed"));
    assert!(adb.call_index(&["shell", "am", "start"]).is_none());
}

#[tokio::test]
async fn fake_adb_receiver_connect_succeeds_before_first_pcm_reaches_audiotrack() {
    let files = ArtifactFiles::new();
    let adb = FakeAdb::new(true, files.digest.clone());
    let cancel = CancellationToken::new();
    let started = start_with_action(
        &adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &cancel,
        Some(("receiver-test", "test-code", "balanced")),
    )
    .await
    .unwrap();
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
    let ready = start_with_action(
        &adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &cancel,
        Some(("receiver-test", "test-code", "balanced")),
    )
    .await
    .unwrap();
    assert!(!ready.audio_active);
    assert!(ready.monitoring_error().is_none());

    adb.0.lock().unwrap().audio_active = true;
    let playing = health_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel)
        .await
        .unwrap();
    assert!(playing.audio_active);
    assert!(playing.monitoring_error().is_none());

    {
        let mut state = adb.0.lock().unwrap();
        state.audio_active = false;
        state.error = "AudioFocusLost".into();
        state.healthy = false;
    }
    let failed = health_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel)
        .await
        .unwrap();
    assert_eq!(failed.monitoring_error().as_deref(), Some("AudioFocusLost"));

    {
        let mut state = adb.0.lock().unwrap();
        state.active = false;
        state.foreground = false;
        state.healthy = false;
        state.error.clear();
    }
    let stopped = health_with(&adb, SERIAL, &files.apk, &files.metadata, &cancel)
        .await
        .unwrap();
    assert!(!stopped.service_active);
    assert!(stopped.monitoring_error().is_some());
}

#[tokio::test]
async fn fake_adb_rejects_same_version_forged_health_before_read_or_management() {
    let files = ArtifactFiles::new();
    let adb = FakeAdb::new(true, "f".repeat(64));
    let error = start_with(
        &adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.starts_with("RECEIVER_UPDATE_REQUIRED:"));
    let calls = adb.calls();
    assert!(!calls
        .iter()
        .any(|call| call.starts_with(&["shell".into(), "content".into()])));
    assert!(!calls.iter().any(|call| call
        .iter()
        .any(|arg| arg == "app.anpod.receiver.action.START"
            || arg == "app.anpod.receiver.action.STOP")));
}

#[tokio::test]
async fn fake_adb_install_failure_does_not_query_health_or_continue() {
    let files = ArtifactFiles::new();
    let adb = FakeAdb::new(false, files.digest.clone());
    adb.0.lock().unwrap().install_fails = true;
    let error = install_with(
        &adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("签名不匹配"));
    let calls = adb.calls();
    assert!(calls
        .iter()
        .any(|call| call.starts_with(&["install".into(), "-r".into()])));
    assert!(!calls
        .iter()
        .any(|call| call.starts_with(&["shell".into(), "content".into()])));
    assert!(!calls.iter().any(|call| call
        .iter()
        .any(|arg| arg == "app.anpod.receiver.action.START"
            || arg == "app.anpod.receiver.action.STOP")));
}

#[tokio::test]
async fn fake_adb_install_rejects_success_when_historical_same_version_apk_remains() {
    let files = ArtifactFiles::new();
    fs::write(
        &files.metadata,
        serde_json::json!({
            "applicationId": PACKAGE,
            "versionCode": 3,
            "versionName": "0.3.0-v2-candidate",
            "sha256": files.digest,
            "signerSha256": "f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439",
            "sourceCommit": "608c98c75ec04e11555e9e547caa516c96d4e0dd"
        })
        .to_string(),
    )
    .unwrap();
    let adb = FakeAdb::new(
        false,
        "4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050".into(),
    );
    {
        let mut state = adb.0.lock().unwrap();
        state.health_version_code = 3;
        state.health_version_name = "0.3.0-v2-candidate".into();
    }
    let error = install_with(
        &adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &CancellationToken::new(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("与本次目标包不一致"));
    let calls = adb.calls();
    assert!(calls
        .iter()
        .any(|call| call.starts_with(&["install".into(), "-r".into()])));
    assert!(calls.iter().any(|call| call.starts_with(&[
        "shell".into(),
        "toybox".into(),
        "sha256sum".into()
    ])));
    assert!(!calls.iter().any(|call| call.starts_with(&[
        "shell".into(),
        "content".into(),
        "query".into()
    ])));
}

#[tokio::test]
async fn fake_adb_rejects_unauthorized_and_wifi_devices_before_package_commands() {
    let files = ArtifactFiles::new();
    let unauthorized = FakeAdb::new(false, files.digest.clone());
    unauthorized.0.lock().unwrap().device_state = "unauthorized";
    assert!(start_with(
        &unauthorized,
        SERIAL,
        &files.apk,
        &files.metadata,
        &CancellationToken::new()
    )
    .await
    .err()
    .unwrap()
    .contains("已授权的在线 ADB 设备"));
    assert_eq!(unauthorized.calls().len(), 1);

    let wifi = FakeAdb::new(false, files.digest.clone());
    assert!(start_with(
        &wifi,
        "192.0.2.3:5555",
        &files.apk,
        &files.metadata,
        &CancellationToken::new()
    )
    .await
    .err()
    .unwrap()
    .contains("当前不可用"));
    assert_eq!(
        wifi.calls().len(),
        1,
        "wireless ADB commands must confirm the exact selected serial first"
    );
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
    let commitment = provision_with(
        &successful,
        SERIAL,
        &files.apk,
        &files.metadata,
        &successful_path,
        &cancel,
        timeout,
    )
    .await
    .unwrap();
    assert_eq!(
        successful.0.lock().unwrap().binding_commitment.as_deref(),
        Some(commitment.as_str())
    );
    let calls = successful.calls();
    let write_index = calls
        .iter()
        .position(|call| {
            call.starts_with(&[
                "shell".into(),
                "-T".into(),
                "content".into(),
                "write".into(),
            ])
        })
        .unwrap();
    assert!(calls[write_index]
        .windows(2)
        .any(|pair| pair[0] == "--user" && pair[1] == TEST_USER));
    assert!(calls[write_index]
        .windows(2)
        .any(|pair| pair[0] == "--uri" && pair[1] == BINDING_URI));
    assert!(calls
        .iter()
        .flat_map(|call| call.iter())
        .all(|arg| !arg.contains(&commitment)));

    let retry_commitment = provision_with(
        &successful,
        SERIAL,
        &files.apk,
        &files.metadata,
        &successful_path,
        &cancel,
        timeout,
    )
    .await
    .unwrap();
    assert_eq!(retry_commitment, commitment);
    let successful_inputs = successful
        .0
        .lock()
        .unwrap()
        .binding_input_commitments
        .clone();
    assert_eq!(successful_inputs.len(), 2);
    assert_eq!(successful_inputs[0], successful_inputs[1]);

    let cancelled_path = binding_path("cancelled");
    let cancelled_adb = FakeAdb::new(true, files.digest.clone());
    cancelled_adb.0.lock().unwrap().binding_write_mode = FakeBindingWrite::Cancel;
    let cancelled_token = CancellationToken::new();
    let cancelled = provision_with(
        &cancelled_adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &cancelled_path,
        &cancelled_token,
        timeout,
    )
    .await
    .unwrap_err();
    assert!(cancelled.contains("取消"));
    let saved_before_retry = BindingStore::load(&cancelled_path).unwrap().commitment();
    assert!(
        cancelled_path.exists(),
        "cancellation discarded the only DPAPI binding copy"
    );
    let retry = provision_with(
        &cancelled_adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &cancelled_path,
        &CancellationToken::new(),
        timeout,
    )
    .await
    .unwrap();
    assert_eq!(retry, hex_digest(&saved_before_retry));
    let seen = &cancelled_adb.0.lock().unwrap().binding_input_commitments;
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[0], seen[1],
        "retry generated different binding material"
    );

    let conflict_path = binding_path("conflict");
    let conflict_adb = FakeAdb::new(true, files.digest.clone());
    conflict_adb.0.lock().unwrap().binding_commitment = Some("f".repeat(64));
    let conflict = provision_with(
        &conflict_adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &conflict_path,
        &cancel,
        timeout,
    )
    .await
    .unwrap_err();
    assert!(conflict.contains("已有不同绑定"));
    assert!(
        conflict_path.exists(),
        "remote conflict removed the local DPAPI copy"
    );
    let expected_conflict = "f".repeat(64);
    assert_eq!(
        conflict_adb.0.lock().unwrap().binding_commitment.as_deref(),
        Some(expected_conflict.as_str())
    );

    let failed_write_path = binding_path("failed-write");
    let failed_write_adb = FakeAdb::new(true, files.digest.clone());
    failed_write_adb.0.lock().unwrap().binding_write_mode = FakeBindingWrite::ErrorText;
    let failed_write = provision_with(
        &failed_write_adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &failed_write_path,
        &cancel,
        timeout,
    )
    .await
    .unwrap_err();
    assert!(failed_write.contains("未确认"));
    assert!(!failed_write.contains("Error while accessing provider"));
    assert!(
        failed_write_path.exists(),
        "remote error discarded the local DPAPI copy"
    );
    let failed_material = BindingStore::load(&failed_write_path).unwrap();
    let mut failed_record = failed_material.encode();
    let secret_hex = failed_record.0[37..69]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert!(
        !failed_write.contains(&secret_hex),
        "remote error leaked the binding secret"
    );
    failed_record.0.fill(0);

    let wrong_user_path = binding_path("wrong-user");
    let wrong_user_adb = FakeAdb::new(true, files.digest.clone());
    wrong_user_adb.0.lock().unwrap().binding_write_mode = FakeBindingWrite::WrongUser;
    let wrong_user = provision_with(
        &wrong_user_adb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &wrong_user_path,
        &cancel,
        timeout,
    )
    .await
    .unwrap_err();
    assert!(wrong_user.contains("当前用户已变化"));
    assert!(
        !wrong_user_adb.calls().iter().any(|call| call.starts_with(&[
            "shell".into(),
            "content".into(),
            "query".into(),
        ]))
    );

    let non_usb = FakeAdb::new(false, files.digest.clone());
    non_usb.0.lock().unwrap().device_state = "unauthorized";
    let non_usb_path = binding_path("non-usb");
    assert!(provision_with(
        &non_usb,
        SERIAL,
        &files.apk,
        &files.metadata,
        &non_usb_path,
        &cancel,
        timeout
    )
    .await
    .is_err());
    assert!(!non_usb_path.exists());
    assert!(!non_usb.calls().iter().any(|call| call.starts_with(&[
        "shell".into(),
        "content".into(),
        "write".into(),
    ])));

    let wrong_apk = FakeAdb::new(true, "a".repeat(64));
    let wrong_apk_path = binding_path("wrong-apk");
    assert!(provision_with(
        &wrong_apk,
        SERIAL,
        &files.apk,
        &files.metadata,
        &wrong_apk_path,
        &cancel,
        timeout
    )
    .await
    .is_err());
    assert!(!wrong_apk_path.exists());
    assert!(!wrong_apk.calls().iter().any(|call| call.starts_with(&[
        "shell".into(),
        "content".into(),
        "write".into(),
    ])));
}

#[tokio::test]
async fn p1_real_usb_installed_receiver_health_start_stop_when_explicitly_requested() {
    let Some(serial) = std::env::var_os("ANPOD_P1_USB_SERIAL") else {
        return;
    };
    let serial = serial.to_string_lossy();
    let deps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../deps");
    let adb = deps.join("adb.exe");
    let apk = deps.join("receiver.apk");
    let metadata = deps.join("receiver.json");
    let cancel = CancellationToken::new();

    assert!(
        status(&adb, &serial, &apk, &metadata, &cancel)
            .await
            .unwrap()
            .installed,
        "P1 lifecycle test requires Receiver to be installed by a separate deployment step"
    );
    assert!(
        health(&adb, &serial, &apk, &metadata, &cancel)
            .await
            .unwrap()
            .installed
    );
    let started = start(&adb, &serial, &apk, &metadata, &cancel)
        .await
        .unwrap();
    assert!(started.healthy && started.foreground);
    let observed = health(&adb, &serial, &apk, &metadata, &cancel)
        .await
        .unwrap();
    assert!(observed.healthy && observed.foreground);
    let stopped = stop(&adb, &serial, &apk, &metadata, &cancel).await.unwrap();
    assert!(!stopped.service_active);
    assert!(
        !health(&adb, &serial, &apk, &metadata, &cancel)
            .await
            .unwrap()
            .service_active
    );
}
