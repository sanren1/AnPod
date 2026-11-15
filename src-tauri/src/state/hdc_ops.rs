//! HarmonyOS wired sessions over hdc: rport + `aa start`, ANHB bootstrap and pairing,
//! then the same authenticated ANPU record stream as ADB. Tokens, codes and binding
//! material are never logged.

use super::*;
use crate::hdc::hdc_run;
use crate::hdc_bootstrap::{accept_with_token, host_pair, host_proceed};
use crate::security::BindingMaterial;
use std::net::{Shutdown, TcpStream};

const HARMONY_BUNDLE: &str = "app.anpod.harmony";
const HDC_MISSING: &str = "未找到 hdc，无法连接鸿蒙设备。";
const HDC_LOG: &str = "platform=harmony transport=usb-hdc";

/// Wired HarmonyOS audio is Float32 only, exactly like the ADB record stream.
pub(crate) fn hdc_wired_profile(settings: &AppSettings) -> Result<u8, String> {
    audio_quality_profile(&settings.audio_quality)
        .filter(|profile| *profile >= 4)
        .ok_or_else(|| "ADB Float32 audio requires HiFi or Hi-Res quality.".to_string())
}

/// hdc exits 0 even when a command fails; the verdict is in its text.
pub(crate) fn hdc_command_ok(result: &AdbResult) -> bool {
    result.success
        && !result.stdout.contains("[Fail]")
        && !result.stdout.to_ascii_lowercase().contains("error")
}

/// Same contract as `adb_reverse_remove_result`: a rule that is already gone counts as removed.
pub(crate) fn hdc_fport_remove_result(result: AdbResult) -> Result<(), String> {
    let output = result.stdout.trim();
    if result.success && (!output.contains("[Fail]") || output.contains("is not exist")) {
        Ok(())
    } else {
        Err("无法移除 hdc 端口转发。".into())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Deletes this tablet's PC material and profile (spec 15.5); missing files are fine.
pub(crate) fn forget_harmony_binding(data_dir: &Path, reference: &str) -> Result<(), String> {
    let path = binding_store_path(data_dir, reference)?;
    for file in [path.with_extension("json"), path] {
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("无法删除这台平板的旧绑定。".into()),
        }
    }
    Ok(())
}

/// Runs pairing and stores material + profile only after both sides confirmed (PairSaved checked).
pub(crate) fn pair_and_store(
    stream: &mut TcpStream,
    data_dir: &Path,
    reference: &str,
    model: &str,
    material: &BindingMaterial,
    confirm: impl FnOnce(&str) -> bool,
) -> Result<(), String> {
    host_pair(stream, material, confirm)?;
    let path = binding_store_path(data_dir, reference)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "无法保存平板绑定。".to_string())?;
    }
    BindingStore::create_or_confirm(&path, material)
        .map_err(|_| "无法保存平板绑定。".to_string())?;
    update_binding_profile(data_dir, reference, |profile| {
        profile.platform = "harmony".into();
        profile.model = (!model.is_empty()).then(|| model.into());
    })
    .inspect_err(|_| {
        // Without Proceed the tablet drops its half too, so neither side keeps a partial pairing.
        let _ = forget_harmony_binding(data_dir, reference);
    })
}

impl AppState {
    /// Checks that a HarmonyOS connect can start, before an existing session is torn down.
    pub(super) fn check_hdc_target(&self) -> Result<(), String> {
        let settings = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .settings
            .clone();
        hdc_wired_profile(&settings)?;
        if self.hdc_path.is_none() {
            return Err(HDC_MISSING.into());
        }
        Ok(())
    }

    pub(super) fn set_harmony_pairing(&self, code: String, answer: std::sync::mpsc::Sender<bool>) {
        *self
            .harmony_pairing_answer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(answer);
        self.view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .harmony_pairing_code = Some(code);
    }

    /// Dropping the pending sender answers "no" to a waiting pairing.
    fn clear_harmony_pairing(&self) {
        self.harmony_pairing_answer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .harmony_pairing_code = None;
    }

    pub fn answer_harmony_pairing(&self, accept: bool) -> Result<AppSnapshot, String> {
        let answer = self
            .harmony_pairing_answer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or("没有等待确认的平板配对。")?;
        self.view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .harmony_pairing_code = None;
        let _ = answer.send(accept);
        Ok(self.snapshot())
    }

    pub(super) async fn remove_hdc_forward(&self, forward: &HdcForward) -> Result<(), String> {
        let hdc = self.hdc_path.as_deref().ok_or(HDC_MISSING)?;
        let mut args = vec!["fport", "rm"];
        args.extend(forward.task.split_whitespace());
        hdc_run(hdc, Some(&forward.key), &args, Duration::from_secs(5), None)
            .await
            .and_then(hdc_fport_remove_result)
    }

    pub(super) async fn connect_hdc_stream_inner(
        &self,
        app: &AppHandle,
        device: &Device,
        operation_generation: u64,
        token: &CancellationToken,
        play_on_phone: bool,
        settings: AppSettings,
    ) -> Result<String, String> {
        let profile = hdc_wired_profile(&settings)?;
        let result = self
            .connect_hdc_stream(
                app,
                device,
                operation_generation,
                token,
                play_on_phone,
                settings,
                profile,
            )
            .await;
        match &result {
            Ok(_) => crate::applog::info("session", "hdc.connected", HDC_LOG),
            Err(error) => crate::applog::warn(
                "session",
                "hdc.failed",
                format_args!("{HDC_LOG} error={error}"),
            ),
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn connect_hdc_stream(
        &self,
        app: &AppHandle,
        device: &Device,
        operation_generation: u64,
        token: &CancellationToken,
        play_on_phone: bool,
        settings: AppSettings,
        profile: u8,
    ) -> Result<String, String> {
        use crate::aoa_owner::AoaOwner;

        let hdc = self.hdc_path.clone().ok_or(HDC_MISSING)?;
        let key = device.serial.as_str();
        token_check(token)?;
        self.audio.initialize()?;
        self.audio.set_streaming(false)?;
        let (format, native, source) = self.prepare_float32_stream(profile, &settings)?;

        let reference = BindingMaterial::harmony_binding_reference(key);
        let stored = binding_store_path(&self.data_dir, &reference)
            .ok()
            .and_then(|path| BindingStore::load(&path).ok());
        let pc_has_material = stored.is_some();
        let material = match stored {
            Some(material) => material,
            None => BindingMaterial::generate().map_err(|_| "无法生成绑定材料。".to_string())?,
        };

        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| error.to_string())?;
        let pc_port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let mut random = [0u8; 18];
        getrandom::fill(&mut random).map_err(|_| "无法生成本次连接的启动令牌。".to_string())?;
        let tablet_port = 40000 + u16::from_be_bytes([random[16], random[17]]) % 10000;
        let mut launch_token = [0u8; 16];
        launch_token.copy_from_slice(&random[..16]);
        let tablet_spec = format!("tcp:{tablet_port}");
        let pc_spec = format!("tcp:{pc_port}");
        let rport = hdc_run(
            &hdc,
            Some(key),
            &["rport", &tablet_spec, &pc_spec],
            Duration::from_secs(5),
            Some(token),
        )
        .await?;
        if !hdc_command_ok(&rport) {
            return Err("hdc 端口转发失败。".into());
        }
        let generation = self
            .register_record_stream_session(
                device,
                operation_generation,
                token,
                String::new(),
                &source,
                &reference,
                native,
                Some(HdcForward {
                    key: key.into(),
                    task: format!("{tablet_spec} {pc_spec}"),
                }),
            )
            .await?;

        let port_text = tablet_port.to_string();
        let host_id = hex(&material.host_id());
        let launch_text = hex(&launch_token);
        let start = hdc_run(
            &hdc,
            Some(key),
            &[
                "shell",
                "aa",
                "start",
                "-b",
                HARMONY_BUNDLE,
                "-a",
                "EntryAbility",
                "--ps",
                "port",
                &port_text,
                "--ps",
                "hostId",
                &host_id,
                "--ps",
                "launchToken",
                &launch_text,
            ],
            Duration::from_secs(10),
            Some(token),
        )
        .await?;
        if !hdc_command_ok(&start) {
            return Err("无法在平板上启动 AnPod。".into());
        }
        let accept_cancel = token.clone();
        let (stream, tablet_has_binding) = tokio::task::spawn_blocking(move || {
            accept_with_token(
                &listener,
                launch_token,
                Instant::now() + Duration::from_secs(30),
                &accept_cancel,
            )
        })
        .await
        .map_err(|error| format!("hdc accept worker failed: {error}"))??;
        token_check(token)?;

        let (mut stream, material) = if pc_has_material && tablet_has_binding {
            (stream, material)
        } else {
            // Spec 15.5: one side lost its binding, so pair afresh. The tablet pins the
            // hostId it was launched with, so a re-pair keeps it and renews everything else.
            forget_harmony_binding(&self.data_dir, &reference)?;
            let material = if pc_has_material {
                material
                    .regenerate_keeping_host()
                    .map_err(|_| "无法生成绑定材料。".to_string())?
            } else {
                material
            };
            self.pair_harmony(app, stream, material, reference, &device.model, token)
                .await?
        };
        token_check(token)?;
        host_proceed(&mut stream).map_err(|_| "无法通知平板继续连接。".to_string())?;

        let owner_queue = source.clone();
        let startup_diagnostics = self.startup_diagnostics.clone();
        let owner = tokio::task::spawn_blocking(move || {
            AoaOwner::connect_adb_stream(
                stream,
                material,
                format,
                owner_queue,
                "usb-hdc",
                startup_diagnostics,
            )
        })
        .await
        .map_err(|error| format!("hdc record owner failed: {error}"))??;
        token_check(token)?;
        // No `content query` health here: HarmonyOS health comes from the stream itself.
        self.finish_record_stream_session(
            app,
            device,
            operation_generation,
            token,
            play_on_phone,
            generation,
            owner,
            &source,
            native,
            format,
            None,
        )
        .await
    }

    /// Shows the code through the snapshot and waits for `answer_harmony_pairing`; cancelling
    /// the operation closes the stream, which aborts the blocking pairing worker.
    async fn pair_harmony(
        &self,
        app: &AppHandle,
        mut stream: TcpStream,
        material: BindingMaterial,
        reference: String,
        model: &str,
        token: &CancellationToken,
    ) -> Result<(TcpStream, BindingMaterial), String> {
        let closer = stream.try_clone().map_err(|error| error.to_string())?;
        let (code_sender, mut code_receiver) = tokio::sync::oneshot::channel::<String>();
        let (answer_sender, answer_receiver) = std::sync::mpsc::channel::<bool>();
        let data_dir = self.data_dir.clone();
        let model = model.to_owned();
        let mut pairing = tokio::task::spawn_blocking(move || {
            pair_and_store(
                &mut stream,
                &data_dir,
                &reference,
                &model,
                &material,
                |code| {
                    let _ = code_sender.send(code.to_owned());
                    answer_receiver.recv().unwrap_or(false)
                },
            )
            .map(|()| (stream, material))
        });
        let mut answer_sender = Some(answer_sender);
        let mut cancelled = false;
        let result = loop {
            tokio::select! {
                joined = &mut pairing => {
                    break joined.map_err(|error| format!("hdc pairing worker failed: {error}"))?;
                }
                code = &mut code_receiver, if answer_sender.is_some() => {
                    if let (Ok(code), Some(answer)) = (code, answer_sender.take()) {
                        self.set_harmony_pairing(code, answer);
                        self.emit(app);
                    }
                }
                () = token.cancelled(), if !cancelled => {
                    cancelled = true;
                    answer_sender = None;
                    self.clear_harmony_pairing();
                    let _ = closer.shutdown(Shutdown::Both);
                }
            }
        };
        self.clear_harmony_pairing();
        self.emit(app);
        if result.is_ok() {
            crate::applog::info("session", "hdc.paired", HDC_LOG);
        }
        result
    }
}

#[cfg(test)]
#[path = "../hdc_ops_tests.rs"]
mod tests;
