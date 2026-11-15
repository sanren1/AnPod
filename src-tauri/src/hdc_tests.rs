use super::*;
use crate::state::{merge_devices, parse_devices, Device};

const TARGETS_SAMPLE: &str =
    "127.0.0.1:5555\t\tTCP\tConnected\tlocalhost\thdc\nCOM1\t\tUART\tReady\tunknown...\thdc\n";

#[test]
fn parse_hdc_targets_keeps_only_connected_usb() {
    let text = format!(
        "{TARGETS_SAMPLE}ABC123\t\tUSB\tConnected\tlocalhost\thdc\nDEF456\t\tUSB\tOffline\tlocalhost\thdc\n192.168.1.9:8710\t\tTCP\tConnected\tlocalhost\thdc\n"
    );
    let keys: Vec<_> = parse_hdc_targets(&text)
        .into_iter()
        .map(|t| t.key)
        .collect();
    assert_eq!(keys, vec!["ABC123".to_string()]);
}

#[test]
fn parse_hdc_targets_ignores_garbage() {
    assert!(parse_hdc_targets("[Fail]hdc server error").is_empty());
    assert!(parse_hdc_targets("").is_empty());
    assert!(parse_hdc_targets("[Empty]").is_empty());
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("anpod-hdc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn locate_hdc_prefers_path_then_deveco() {
    let on_path = scratch("path");
    std::fs::write(on_path.join("hdc.exe"), b"").unwrap();
    let install = scratch("deveco");
    let tools = install.join("sdk/default/openharmony/toolchains");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(tools.join("hdc.exe"), b"").unwrap();
    let empty = scratch("empty");

    let path_env = std::env::join_paths([&empty, &on_path]).unwrap();
    assert_eq!(
        locate_hdc(Some(&path_env), Some(install.clone())),
        Some(on_path.join("hdc.exe"))
    );
    let path_env = std::env::join_paths([&empty]).unwrap();
    assert_eq!(
        locate_hdc(Some(&path_env), Some(install.clone())),
        Some(tools.join("hdc.exe"))
    );
    assert_eq!(locate_hdc(None, None), None);
    assert_eq!(locate_hdc(Some(&path_env), Some(empty.clone())), None);
}

const ADB_TEXT: &str = "List of devices attached\nR58M123 device usb:1-2 product:p model:Pixel_8 device:d transport_id:1\n";

fn harmony(key: &str) -> Device {
    let mut device = parse_devices(ADB_TEXT).remove(0);
    device.serial = key.into();
    device.platform = "harmony".into();
    device.os_version = "HarmonyOS 7.0.0".into();
    device
}

#[test]
fn merge_devices_without_hdc_is_adb_only() {
    let adb = parse_devices(ADB_TEXT);
    let merged = merge_devices(adb.clone(), None);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].serial, adb[0].serial);
    assert_eq!(merged[0].platform, "android");
}

#[test]
fn merge_devices_appends_harmony_after_android() {
    let merged = merge_devices(parse_devices(ADB_TEXT), Some(vec![harmony("HW1")]));
    let serials: Vec<_> = merged.iter().map(|d| d.serial.as_str()).collect();
    assert_eq!(serials, ["R58M123", "HW1"]);
}

#[test]
fn android_devices_get_platform_android() {
    let devices = parse_devices(ADB_TEXT);
    assert!(!devices.is_empty());
    assert!(devices.iter().all(|d| d.platform == "android"));
}

#[test]
fn harmony_params_build_device() {
    let device = harmony_device("HW1", "7.0.0", "emulator");
    assert_eq!(device.platform, "harmony");
    assert_eq!(device.transport, "usb");
    assert_eq!(device.os_version, "HarmonyOS 7.0.0");
    assert_eq!(device.display_name, "emulator");
    assert!(device.connectable && device.android_version.is_empty() && device.api_level.is_empty());
}

#[test]
fn registry_output_yields_install_location() {
    let found = r"
HKEY_LOCAL_MACHINE\X\DevEco
    DisplayName    REG_SZ    DevEco Studio
";
    assert_eq!(
        registry_key_with_value(found, "DevEco Studio").as_deref(),
        Some(r"HKEY_LOCAL_MACHINE\X\DevEco")
    );
    let value = r"
HKEY_X
    InstallLocation    REG_SZ    E:\Software\DevEco Studio
";
    assert_eq!(
        registry_value(value, "InstallLocation").as_deref(),
        Some(r"E:\Software\DevEco Studio")
    );
}
