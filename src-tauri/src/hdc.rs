//! HarmonyOS discovery over hdc. Everything here is additive: without an hdc
//! binary the Android device list is untouched.

use crate::state::{adb_run, AdbResult, Device};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const HDC_EXE: &str = "hdc.exe";
const HDC_SERVER_PORT: u16 = 8710;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HdcTarget {
    pub(crate) key: String,
}

/// PATH first, then the toolchain bundled with DevEco Studio.
pub(crate) fn locate_hdc(
    path_env: Option<&OsStr>,
    deveco_install: Option<PathBuf>,
) -> Option<PathBuf> {
    let on_path = path_env
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(HDC_EXE));
    let bundled = deveco_install.map(|root| {
        root.join("sdk/default/openharmony/toolchains")
            .join(HDC_EXE)
    });
    on_path.chain(bundled).find(|candidate| candidate.is_file())
}

/// `InstallLocation` of the uninstall entry whose DisplayName is "DevEco Studio".
pub(crate) fn deveco_install_location() -> Option<PathBuf> {
    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    ["HKLM", "HKCU"].into_iter().find_map(|hive| {
        let root = format!(r"{hive}\{UNINSTALL}");
        let found = reg_query(&[&root, "/s", "/f", "DevEco Studio", "/d"])?;
        let key = registry_key_with_value(&found, "DevEco Studio")?;
        let value = reg_query(&[&key, "/v", "InstallLocation"])?;
        registry_value(&value, "InstallLocation").map(PathBuf::from)
    })
}

fn reg_query(args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new("reg");
    command.arg("query").args(args);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x08000000);
    let output = command.output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn registry_key_with_value(text: &str, wanted: &str) -> Option<String> {
    let mut key = None;
    for line in text.lines() {
        if line.starts_with("HK") {
            key = Some(line.trim().to_owned());
        } else if line
            .split_once("REG_SZ")
            .is_some_and(|(_, value)| value.trim() == wanted)
        {
            return key;
        }
    }
    None
}

fn registry_value(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (head, value) = line.trim().split_once("REG_SZ")?;
        (head.trim() == name && !value.trim().is_empty()).then(|| value.trim().to_owned())
    })
}

/// Reuses the adb runner: `-t <key>` replaces `-s <serial>`.
pub(crate) async fn hdc_run(
    hdc: &Path,
    key: Option<&str>,
    args: &[&str],
    timeout: Duration,
    cancel: Option<&CancellationToken>,
) -> Result<AdbResult, String> {
    let mut full = Vec::with_capacity(args.len() + 2);
    if let Some(key) = key {
        full.extend(["-t", key]);
    }
    full.extend_from_slice(args);
    adb_run(hdc, None, &full, timeout, cancel).await
}

/// Columns: key, transport, state, hostname, daemon. Only connected USB targets
/// are kept; hdc prints `[Fail]...`/`[Empty]` for errors and empty lists.
pub(crate) fn parse_hdc_targets(text: &str) -> Vec<HdcTarget> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let (key, transport, state) = (fields.next()?, fields.next()?, fields.next()?);
            (transport == "USB" && state == "Connected").then(|| HdcTarget { key: key.into() })
        })
        .collect()
}

pub(crate) fn harmony_device(key: &str, version: &str, model: &str) -> Device {
    Device {
        serial: key.into(),
        physical_id: None,
        platform: "harmony".into(),
        transport: "usb".into(),
        state: "device".into(),
        manufacturer: String::new(),
        model: model.into(),
        android_version: String::new(),
        os_version: format!("HarmonyOS {version}").trim_end().into(),
        api_level: String::new(),
        display_name: if model.is_empty() { key } else { model }.into(),
        status_text: "可连接".into(),
        connectable: true,
    }
}

async fn read_param(
    hdc: &Path,
    key: &str,
    name: &str,
    cancel: Option<&CancellationToken>,
) -> String {
    let result = hdc_run(
        hdc,
        Some(key),
        &["shell", "param", "get", name],
        Duration::from_secs(4),
        cancel,
    )
    .await;
    match result {
        // A missing parameter prints `Get parameter "x" fail!` and still exits 0.
        Ok(r) if r.success && !r.stdout.contains("fail") && !r.stdout.contains("[Fail]") => {
            r.stdout.trim().to_owned()
        }
        _ => String::new(),
    }
}

pub(crate) async fn hdc_device(
    hdc: &Path,
    key: &str,
    cancel: Option<&CancellationToken>,
) -> Device {
    let model = read_param(hdc, key, "const.product.model", cancel).await;
    let version = read_param(hdc, key, "const.product.os.dist.version", cancel).await;
    harmony_device(key, &version, &model)
}

/// Port hdc will use: OHOS_HDC_SERVER_PORT when set, else the default. `None`
/// when the variable is set but unusable, so we never claim the server.
pub(crate) fn hdc_server_port(env_value: Option<&str>) -> Option<u16> {
    match env_value {
        None => Some(HDC_SERVER_PORT),
        Some(value) => value.trim().parse().ok().filter(|port| *port != 0),
    }
}

fn hdc_server_listening() -> bool {
    let env = std::env::var("OHOS_HDC_SERVER_PORT").ok();
    let Some(port) = hdc_server_port(env.as_deref()) else {
        return true;
    };
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

/// `None` means "no HarmonyOS result"; the caller keeps the Android list as is.
pub(crate) async fn scan_hdc(
    hdc: &Path,
    server_started: &AtomicBool,
    cancel: Option<&CancellationToken>,
) -> Option<Vec<Device>> {
    // hdc auto-starts its server; remember when it was us so exit can stop it.
    if !server_started.load(Ordering::Relaxed) && !hdc_server_listening() {
        server_started.store(true, Ordering::Relaxed);
    }
    let result = hdc_run(
        hdc,
        None,
        &["list", "targets", "-v"],
        Duration::from_secs(8),
        cancel,
    )
    .await
    .and_then(|r| {
        if r.success {
            Ok(r.stdout)
        } else {
            Err("hdc 列表命令失败。".to_string())
        }
    });
    let text = match result {
        Ok(text) => text,
        Err(error) => {
            crate::applog::warn("devices", "hdc_scan.failed", &error);
            return None;
        }
    };
    let mut devices = Vec::new();
    for target in parse_hdc_targets(&text) {
        devices.push(hdc_device(hdc, &target.key, cancel).await);
    }
    Some(devices)
}

pub(crate) fn kill_hdc_server(hdc: &Path) {
    let mut command = std::process::Command::new(hdc);
    command.arg("kill");
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x08000000);
    let Ok(mut child) = command.spawn() else {
        return;
    };
    for _ in 0..30 {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
}

#[cfg(test)]
#[path = "hdc_tests.rs"]
mod tests;
