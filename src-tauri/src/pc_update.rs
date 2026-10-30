use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tauri::AppHandle;

const SCRIPT: &str = include_str!("pc_update.ps1");
pub const SINGLE_INSTANCE_MUTEX_NAME: &str = "Local\\AnPod.SingleInstance";
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    version: String,
    release_url: String,
    asset_name: String,
    asset_size: u64,
    sums_size: u64,
    zip_url: String,
    sums_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub current_version: String,
    pub latest_version: String,
    pub available: bool,
    pub release_url: String,
    pub asset_name: String,
    pub asset_size: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreparedUpdate {
    pub status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PcUpdateStatus {
    pub install_result: Option<InstallResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallResult {
    pub status: String,
    pub message: String,
}

pub struct PcUpdater {
    operation: tokio::sync::Mutex<()>,
    install_gate: tokio::sync::Mutex<()>,
    release: Mutex<Option<Release>>,
    prepared: Mutex<Option<PathBuf>>,
}

impl PcUpdater {
    pub fn new() -> Self {
        Self {
            operation: tokio::sync::Mutex::new(()),
            install_gate: tokio::sync::Mutex::new(()),
            release: Mutex::new(None),
            prepared: Mutex::new(None),
        }
    }

    pub async fn check(&self) -> Result<UpdateCheck, String> {
        let _operation = self.operation.lock().await;
        let work = unique_update_dir()?;
        let output = tokio::task::spawn_blocking(move || {
            let script = work.join("pc_update.ps1");
            let result = fs::write(&script, SCRIPT)
                .map_err(|e| format!("无法准备更新检查：{e}"))
                .and_then(|()| run_powershell(&script, &["-Mode", "check"]));
            let _ = fs::remove_dir_all(&work);
            result
        })
        .await
        .map_err(|e| format!("更新检查任务失败：{e}"))??;
        let release: Release =
            serde_json::from_str(&output).map_err(|e| format!("无法解析GitHub正式Release：{e}"))?;
        validate_release(&release)?;
        let current = env!("CARGO_PKG_VERSION").to_owned();
        let available =
            compare_versions(&release.version, &current)? == std::cmp::Ordering::Greater;
        *self.release.lock().expect("PC updater release lock") = Some(release.clone());
        if !available {
            *self.prepared.lock().expect("PC updater prepared lock") = None;
        }
        Ok(UpdateCheck {
            current_version: current,
            latest_version: release.version,
            available,
            release_url: release.release_url,
            asset_name: release.asset_name,
            asset_size: release.asset_size,
        })
    }

    pub async fn download(&self) -> Result<PreparedUpdate, String> {
        let _operation = self.operation.lock().await;
        let release = self
            .release
            .lock()
            .expect("PC updater release lock")
            .clone()
            .ok_or("请先检查正式更新。")?;
        let current = env!("CARGO_PKG_VERSION");
        if compare_versions(&release.version, current)? != std::cmp::Ordering::Greater {
            return Err("当前没有可下载的新版本。".into());
        }
        let stage = unique_update_dir()?;
        let script = stage.join("pc_update.ps1");
        let result_stage = stage.clone();
        let result_release = release.clone();
        let result = tokio::task::spawn_blocking(move || {
            fs::write(&script, SCRIPT).map_err(|e| format!("无法准备更新下载：{e}"))?;
            let size = result_release.asset_size.to_string();
            let sums_size = result_release.sums_size.to_string();
            let stage_path = result_stage.to_str().ok_or("更新目录路径无法编码。")?;
            let args = [
                "-Mode",
                "download",
                "-ZipUrl",
                result_release.zip_url.as_str(),
                "-SumsUrl",
                result_release.sums_url.as_str(),
                "-AssetName",
                result_release.asset_name.as_str(),
                "-AssetSize",
                size.as_str(),
                "-SumsSize",
                sums_size.as_str(),
                "-StageDirectory",
                stage_path,
            ];
            run_powershell(&script, &args)
        })
        .await
        .map_err(|e| format!("更新下载任务失败：{e}"))?
        .map(|_| ());
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&stage);
            return Err(error);
        }
        *self.prepared.lock().expect("PC updater prepared lock") = Some(stage);
        Ok(PreparedUpdate { status: "prepared" })
    }

    pub async fn reserve_install(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.install_gate.lock().await
    }

    pub async fn install_reserved(&self, app: &AppHandle) -> Result<(), String> {
        let stage = self
            .prepared
            .lock()
            .expect("PC updater prepared lock")
            .clone()
            .ok_or("请先下载并验证更新包。")?;
        let executable = std::env::current_exe().map_err(|e| format!("无法定位当前程序：{e}"))?;
        let target = executable.parent().ok_or("无法定位便携程序目录。")?;
        verify_target_directory(target)?;
        let exe_name = executable
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("当前程序文件名无效。")?;
        let main_file = stage.join("package").join(exe_name);
        if !main_file.is_file() {
            return Err("已验证的更新包不包含当前主程序，已取消安装。".into());
        }
        let probe = target.join(format!(".anpod-update-probe-{}", std::process::id()));
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .map_err(|e| format!("更新目录不可写：{e}"))?;
        fs::remove_file(&probe).map_err(|e| format!("无法清理更新权限检查文件：{e}"))?;

        let script = stage.join("pc_update.ps1");
        fs::write(&script, SCRIPT).map_err(|e| format!("无法准备本地更新程序：{e}"))?;
        let handoff_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let ready_path = stage.join(format!("install-ready-{handoff_id}.txt"));
        let continue_path = stage.join(format!("install-continue-{handoff_id}.txt"));
        let process_id = std::process::id().to_string();
        let process_start_time = current_process_start_time()?.to_string();
        let result_file = result_path()?;
        let mut command = hidden_powershell(&script);
        command.args([
            "-Mode",
            "install",
            "-StageDirectory",
            stage.to_str().ok_or("更新目录路径无法编码。")?,
            "-TargetDirectory",
            target.to_str().ok_or("程序目录路径无法编码。")?,
            "-ExeName",
            exe_name,
            "-ExePath",
            executable.to_str().ok_or("当前程序路径无法编码。")?,
            "-ResultPath",
            result_file.to_str().ok_or("更新结果路径无法编码。")?,
            "-MutexName",
            SINGLE_INSTANCE_MUTEX_NAME,
            "-ReadyPath",
            ready_path.to_str().ok_or("更新交接路径无法编码。")?,
            "-ContinuePath",
            continue_path.to_str().ok_or("更新交接路径无法编码。")?,
            "-ProcessId",
            process_id.as_str(),
            "-ProcessStartTime",
            process_start_time.as_str(),
        ]);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|e| format!("无法启动本地更新程序：{e}"))?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if ready_path.exists() {
                let ready = fs::read_to_string(&ready_path)
                    .map_err(|e| format!("无法读取更新交接结果：{e}"))?;
                if let Some(error) = ready.strip_prefix("error:") {
                    return Err(format!("本地更新预检失败：{error}"));
                }
                if ready != "ready" {
                    return Err("本地更新程序返回了无效交接状态。".into());
                }
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&continue_path)
                    .map_err(|e| format!("无法确认更新交接：{e}"))?;
                app.exit(0);
                return Ok(());
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|e| format!("无法检查本地更新程序：{e}"))?
            {
                return Err(format!("本地更新程序未能完成预检（退出码 {status}）。"));
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("本地更新程序预检超时，当前AnPod未退出。".into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    pub fn ensure_prepared(&self) -> Result<(), String> {
        let stage = self
            .prepared
            .lock()
            .expect("PC updater prepared lock")
            .clone()
            .ok_or("请先下载并验证更新包。")?;
        if !stage.join("manifest.json").is_file() || !stage.join("package").is_dir() {
            return Err("已准备的更新包不完整，请重新下载。".into());
        }
        Ok(())
    }

    pub fn open_download(&self) -> Result<(), String> {
        let path = self
            .prepared
            .lock()
            .expect("PC updater prepared lock")
            .clone()
            .ok_or("当前没有已准备的更新包。")?
            .join("package");
        open_path(&path)
    }

    pub fn open_release(&self) -> Result<(), String> {
        let url = self
            .release
            .lock()
            .expect("PC updater release lock")
            .as_ref()
            .map(|r| r.release_url.clone())
            .ok_or("请先检查正式更新。")?;
        Command::new("explorer.exe")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("无法打开GitHub Release：{e}"))
    }

    pub fn status(&self) -> Result<PcUpdateStatus, String> {
        let path = result_path()?;
        if !path.exists() {
            return Ok(PcUpdateStatus {
                install_result: None,
            });
        }
        verify_target_directory(path.parent().ok_or("更新结果路径无父目录。")?)?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| format!("无法检查更新结果：{e}"))?;
        if metadata.len() > 2048 {
            return Err("更新结果文件超出允许大小。".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("更新结果文件是重解析点，已拒绝读取。".into());
            }
        }
        let body = fs::read(&path).map_err(|e| format!("无法读取更新结果：{e}"))?;
        let result: InstallResult =
            serde_json::from_slice(&body).map_err(|e| format!("更新结果格式无效：{e}"))?;
        if !matches!(
            result.status.as_str(),
            "installed" | "failed" | "rolledBack" | "rollbackFailed"
        ) || result.message.len() > 2048
        {
            return Err("更新结果内容无效。".into());
        }
        fs::remove_file(path).map_err(|e| format!("无法清理已读取的更新结果：{e}"))?;
        Ok(PcUpdateStatus {
            install_result: Some(result),
        })
    }
}

fn unique_update_dir() -> Result<PathBuf, String> {
    let parent = updates_directory()?;
    for _ in 0..100 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("{}-{id}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("无法创建更新工作目录：{e}")),
        }
    }
    Err("无法分配唯一的更新工作目录。".into())
}

fn updates_directory() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA").ok_or("无法定位当前用户的本地应用数据目录。")?;
    let local = PathBuf::from(local);
    verify_target_directory(&local)?;
    let app_dir = local.join("AnPod");
    if !app_dir.exists() {
        fs::create_dir(&app_dir).map_err(|e| format!("无法创建更新缓存目录：{e}"))?;
    }
    verify_target_directory(&app_dir)?;
    let parent = app_dir.join("updates");
    if !parent.exists() {
        fs::create_dir(&parent).map_err(|e| format!("无法创建更新缓存目录：{e}"))?;
    }
    verify_target_directory(&parent)?;
    Ok(parent)
}

fn result_path() -> Result<PathBuf, String> {
    Ok(updates_directory()?.join("last-install-result.json"))
}

fn validate_release(release: &Release) -> Result<(), String> {
    let _ = parse_version(&release.version)?;
    let expected_name = format!("AnPod-{}-tauri-win-x64-portable.zip", release.version);
    if release.asset_name != expected_name
        || release.asset_size == 0
        || release.asset_size > 524_288_000
        || release.sums_size == 0
        || release.sums_size > 1_048_576
    {
        return Err("GitHub正式Release资产不符合AnPod便携包约定。".into());
    }
    let expected_release = format!(
        "https://github.com/sanren1/AnPod/releases/tag/v{}-main",
        release.version
    );
    if release.release_url != expected_release
        || !trusted_asset_url(&release.zip_url, &release.asset_name)
        || !trusted_asset_url(&release.sums_url, "SHA256SUMS.txt")
    {
        return Err("GitHub正式Release链接不符合官方仓库约定。".into());
    }
    Ok(())
}

fn trusted_asset_url(value: &str, file: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://github.com/sanren1/AnPod/releases/download/")
    else {
        return false;
    };
    let mut parts = rest.split('/');
    parts.next().is_some_and(|tag| {
        !tag.is_empty()
            && tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    }) && parts.next() == Some(file)
        && parts.next().is_none()
}

fn parse_version(version: &str) -> Result<[u64; 3], String> {
    let mut parts = version.split('.');
    let mut values = [0; 3];
    for value in &mut values {
        *value = parts
            .next()
            .ok_or("Release版本号无效。")?
            .parse()
            .map_err(|_| "Release版本号无效。")?;
    }
    if parts.next().is_some() {
        return Err("Release版本号无效。".into());
    }
    Ok(values)
}

fn compare_versions(left: &str, right: &str) -> Result<std::cmp::Ordering, String> {
    Ok(parse_version(left)?.cmp(&parse_version(right)?))
}

fn open_path(path: &Path) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法打开更新目录：{e}"))
}

fn verify_target_directory(path: &Path) -> Result<(), String> {
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("无法解析更新目录：{e}"))?
            .join(path)
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let mut current = Some(full.as_path());
        while let Some(path) = current {
            let metadata =
                fs::symlink_metadata(path).map_err(|e| format!("无法检查更新路径：{e}"))?;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("更新路径包含Windows重解析点，已拒绝安装。".into());
            }
            current = path.parent();
        }
    }
    #[cfg(not(windows))]
    let _ = full;
    Ok(())
}

#[cfg(windows)]
fn current_process_start_time() -> Result<u64, String> {
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessTimes(
            process: *mut std::ffi::c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let mut creation = FileTime { low: 0, high: 0 };
    let mut exit = FileTime { low: 0, high: 0 };
    let mut kernel = FileTime { low: 0, high: 0 };
    let mut user = FileTime { low: 0, high: 0 };
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return Err("无法读取当前进程的启动时间。".into());
    }
    Ok((u64::from(creation.high) << 32) | u64::from(creation.low))
}

#[cfg(not(windows))]
fn current_process_start_time() -> Result<u64, String> {
    Err("PC自动更新仅支持Windows便携版。".into())
}

fn hidden_powershell(script: &Path) -> Command {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn run_powershell(script: &Path, args: &[&str]) -> Result<String, String> {
    let output = hidden_powershell(script)
        .args(args)
        .output()
        .map_err(|e| format!("无法启动Windows PowerShell 5.1：{e}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if error.is_empty() {
            "更新检查或下载失败。".into()
        } else {
            error
        });
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|e| format!("PowerShell返回了无效文本：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numeric_release_versions() {
        assert_eq!(
            compare_versions("1.10.0", "1.9.0").unwrap(),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.9.0", "1.10.0").unwrap(),
            std::cmp::Ordering::Less
        );
        assert!(compare_versions("1.9", "1.10.0").is_err());
        assert_eq!(
            compare_versions("1.18.0", env!("CARGO_PKG_VERSION")).unwrap(),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn download_command_refuses_the_older_official_release() {
        let updater = PcUpdater::new();
        *updater.release.lock().unwrap() = Some(Release {
            version: "1.18.0".into(),
            release_url: "https://github.com/sanren1/AnPod/releases/tag/v1.18.0-main".into(),
            asset_name: "AnPod-1.18.0-tauri-win-x64-portable.zip".into(),
            asset_size: 19_280_382,
            sums_size: 205,
            zip_url: "https://github.com/sanren1/AnPod/releases/download/v1.18.0-main/AnPod-1.18.0-tauri-win-x64-portable.zip".into(),
            sums_url: "https://github.com/sanren1/AnPod/releases/download/v1.18.0-main/SHA256SUMS.txt".into(),
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let error = runtime.block_on(updater.download()).unwrap_err();
        assert_eq!(error, "当前没有可下载的新版本。");
        assert!(updater.prepared.lock().unwrap().is_none());
    }

    #[test]
    fn restricts_release_asset_urls_to_official_repo() {
        assert!(trusted_asset_url("https://github.com/sanren1/AnPod/releases/download/v1.19.0-main/AnPod-1.19.0-tauri-win-x64-portable.zip", "AnPod-1.19.0-tauri-win-x64-portable.zip"));
        assert!(!trusted_asset_url("https://example.com/AnPod/releases/download/v1.19.0-main/AnPod-1.19.0-tauri-win-x64-portable.zip", "AnPod-1.19.0-tauri-win-x64-portable.zip"));
    }

    #[cfg(windows)]
    #[test]
    fn embedded_helper_replaces_and_rolls_back_fake_portable_targets() {
        use sha2::{Digest, Sha256};
        use std::{
            process::Stdio,
            thread,
            time::{Duration, Instant},
        };

        fn hash(path: &Path) -> String {
            let bytes = fs::read(path).unwrap();
            format!("{:x}", Sha256::digest(bytes))
        }

        fn write_manifest(stage: &Path, names: &[&str]) {
            let files: Vec<_> = names
                .iter()
                .map(|name| {
                    let path = stage.join("package").join(name.replace('/', "\\"));
                    serde_json::json!({
                        "name": name,
                        "length": fs::metadata(&path).unwrap().len(),
                        "sha256": hash(&path),
                    })
                })
                .collect();
            fs::write(
                stage.join("manifest.json"),
                serde_json::to_vec(&serde_json::json!({ "files": files })).unwrap(),
            )
            .unwrap();
        }

        fn run_case(root: &Path, case: &str, fail_after_first_file: bool, retry_preflight: bool) {
            let stage = root.join(format!("{case}-stage"));
            let target = root.join(format!("{case}-fake-app"));
            fs::create_dir_all(stage.join("package/docs")).unwrap();
            fs::create_dir_all(&target).unwrap();
            let windows = PathBuf::from(std::env::var_os("WINDIR").unwrap());
            let powershell = windows.join("System32/WindowsPowerShell/v1.0/powershell.exe");
            let where_exe = windows.join("System32/where.exe");
            fs::copy(&where_exe, stage.join("package/AnPod.exe")).unwrap();
            fs::copy(&where_exe, target.join("AnPod.exe")).unwrap();
            fs::write(stage.join("package/README.md"), b"new README").unwrap();
            fs::write(target.join("README.md"), b"old README").unwrap();
            let mut names = vec!["README.md"];
            if fail_after_first_file {
                fs::write(stage.join("package/docs/new.txt"), b"new docs").unwrap();
                fs::write(target.join("docs"), b"user file blocks directory").unwrap();
                names.push("docs/new.txt");
            }
            names.push("AnPod.exe");
            write_manifest(&stage, &names);

            let helper = stage.join("pc_update.ps1");
            fs::write(&helper, SCRIPT).unwrap();
            let mutex_name = format!(
                "Local\\AnPod.UpdateTest.{}",
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            );
            let parent_script = stage.join("fake-parent.ps1");
            fs::write(
                &parent_script,
                "param([string]$MutexName,[string]$StartPath,[string]$ExitPath,[string]$LockPath,[string]$ReleasePath,[string]$ReleasedPath); $m=[Threading.Mutex]::new($false,$MutexName); $lock=$null; if($LockPath){$lock=[IO.File]::Open($LockPath,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)}; [IO.File]::WriteAllText($StartPath,([Diagnostics.Process]::GetCurrentProcess().StartTime.ToFileTimeUtc().ToString())); while($true){if($lock -and [IO.File]::Exists($ReleasePath)){$lock.Dispose();$lock=$null;[IO.File]::WriteAllText($ReleasedPath,'released')};if([IO.File]::Exists($ExitPath)){break};Start-Sleep -Milliseconds 20};if($lock){$lock.Dispose()};$m.Dispose()",
            )
            .unwrap();
            let start_path = stage.join("parent-start.txt");
            let exit_path = stage.join("parent-exit.txt");
            let lock_path = if retry_preflight {
                target.join("README.md")
            } else {
                PathBuf::new()
            };
            let release_path = stage.join("parent-release-lock.txt");
            let released_path = stage.join("parent-lock-released.txt");
            let result_path = root.join(format!("{case}-result.json"));
            let mut parent = Command::new(&powershell)
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(&parent_script)
                .args(["-MutexName", mutex_name.as_str(), "-StartPath"])
                .arg(&start_path)
                .arg("-ExitPath")
                .arg(&exit_path)
                .arg("-LockPath")
                .arg(&lock_path)
                .arg("-ReleasePath")
                .arg(&release_path)
                .arg("-ReleasedPath")
                .arg(&released_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let started = Instant::now();
            while !start_path.exists() && started.elapsed() < Duration::from_secs(5) {
                assert!(
                    parent.try_wait().unwrap().is_none(),
                    "fake parent exited before creating its mutex"
                );
                thread::sleep(Duration::from_millis(20));
            }
            assert!(start_path.exists(), "fake parent did not start");
            let start_time = fs::read_to_string(&start_path).unwrap();
            let process_id = parent.id().to_string();
            let launch = |attempt: u64| {
                let ready_path = stage.join(format!("install-ready-{attempt}.txt"));
                let continue_path = stage.join(format!("install-continue-{attempt}.txt"));
                let mut command = hidden_powershell(&helper);
                command.args([
                    "-Mode",
                    "install",
                    "-StageDirectory",
                    stage.to_str().unwrap(),
                    "-TargetDirectory",
                    target.to_str().unwrap(),
                    "-ExeName",
                    "AnPod.exe",
                    "-ExePath",
                    powershell.to_str().unwrap(),
                    "-ResultPath",
                    result_path.to_str().unwrap(),
                    "-MutexName",
                    mutex_name.as_str(),
                    "-ReadyPath",
                    ready_path.to_str().unwrap(),
                    "-ContinuePath",
                    continue_path.to_str().unwrap(),
                    "-ProcessId",
                    process_id.as_str(),
                    "-ProcessStartTime",
                    start_time.trim(),
                    "-SilentTest",
                ]);
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                (command.spawn().unwrap(), ready_path, continue_path)
            };
            let await_ready = |child: &mut std::process::Child, path: &Path| {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !path.exists() && Instant::now() < deadline {
                    if let Some(status) = child.try_wait().unwrap() {
                        panic!("embedded updater exited before handoff: {status}");
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                fs::read_to_string(path).unwrap()
            };
            let first_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let (mut updater, ready_path, continue_path) = launch(first_id);
            let first_status = await_ready(&mut updater, &ready_path);
            if retry_preflight {
                assert!(
                    first_status.starts_with("error:"),
                    "locked file did not fail preflight: {first_status}"
                );
                let status = updater.wait().unwrap();
                assert!(
                    !status.success(),
                    "locked-file preflight unexpectedly passed"
                );
                let result: InstallResult =
                    serde_json::from_slice(&fs::read(&result_path).unwrap()).unwrap();
                assert_eq!(result.status, "failed");
                fs::write(&release_path, b"release lock").unwrap();
                let released_deadline = Instant::now() + Duration::from_secs(5);
                while !released_path.exists() && Instant::now() < released_deadline {
                    thread::sleep(Duration::from_millis(20));
                }
                assert!(
                    released_path.exists(),
                    "fake parent did not release the occupied file"
                );
                assert_eq!(fs::read(target.join("README.md")).unwrap(), b"old README");
                let retry_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                let (mut retry, retry_ready, retry_continue) = launch(retry_id);
                assert_eq!(await_ready(&mut retry, &retry_ready), "ready");
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&retry_continue)
                    .unwrap();
                thread::sleep(Duration::from_millis(150));
                assert!(
                    retry.try_wait().unwrap().is_none(),
                    "retry helper did not wait for the same parent"
                );
                fs::write(&exit_path, b"exit").unwrap();
                let parent_status = parent.wait().unwrap();
                assert!(
                    parent_status.success(),
                    "fake parent could not exit cleanly"
                );
                updater = retry;
            } else {
                assert_eq!(first_status, "ready");
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&continue_path)
                    .unwrap();
                thread::sleep(Duration::from_millis(150));
                assert!(
                    updater.try_wait().unwrap().is_none(),
                    "helper did not wait for the exact parent process"
                );
                fs::write(&exit_path, b"exit").unwrap();
                let parent_status = parent.wait().unwrap();
                assert!(
                    parent_status.success(),
                    "fake parent could not exit cleanly"
                );
            }
            fs::write(&exit_path, b"exit").unwrap();
            let parent_status = parent.wait().unwrap();
            assert!(
                parent_status.success(),
                "fake parent could not exit cleanly"
            );
            let finish_deadline = Instant::now() + Duration::from_secs(15);
            let update_status = loop {
                if let Some(status) = updater.try_wait().unwrap() {
                    break status;
                }
                assert!(
                    Instant::now() < finish_deadline,
                    "embedded updater did not finish"
                );
                thread::sleep(Duration::from_millis(20));
            };

            let result: InstallResult =
                serde_json::from_slice(&fs::read(result_path).unwrap()).unwrap();
            if fail_after_first_file {
                assert!(!update_status.success());
                assert_eq!(result.status, "rolledBack");
                assert_eq!(fs::read(target.join("README.md")).unwrap(), b"old README");
                assert_eq!(
                    fs::read(target.join("docs")).unwrap(),
                    b"user file blocks directory"
                );
            } else {
                assert!(update_status.success());
                assert_eq!(result.status, "installed");
                assert_eq!(fs::read(target.join("README.md")).unwrap(), b"new README");
            }
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn CreateMutexW(
                    attributes: *const std::ffi::c_void,
                    owner: i32,
                    name: *const u16,
                ) -> isize;
                fn GetLastError() -> u32;
                fn CloseHandle(handle: isize) -> i32;
            }
            let name: Vec<u16> = format!("{mutex_name}\0").encode_utf16().collect();
            let (handle, last_error) = unsafe {
                let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
                let error = GetLastError();
                let _ = CloseHandle(handle);
                (handle, error)
            };
            assert_ne!(handle, 0);
            assert_eq!(
                last_error, 0,
                "helper left its named mutex handle open after restart"
            );
        }

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("artifacts/pc-update-20261030")
            .join(format!("embedded-helper-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        run_case(&root, "success", false, false);
        run_case(&root, "rollback", true, false);
        run_case(&root, "retry", false, true);
    }
}
