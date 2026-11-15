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

#[test]
fn hdc_server_port_honours_env() {
    assert_eq!(hdc_server_port(None), Some(8710));
    assert_eq!(hdc_server_port(Some("9000")), Some(9000));
    assert_eq!(hdc_server_port(Some("abc")), None);
    assert_eq!(hdc_server_port(Some("")), None);
}

#[test]
fn harmony_device_routes_to_hdc_connect_path() {
    use crate::state::connects_over_hdc;
    assert!(connects_over_hdc(&harmony("HW1")));
    assert!(!connects_over_hdc(&parse_devices(ADB_TEXT)[0]));
}

/// Real PC production owner against the HarmonyOS emulator's native StreamSession over `hdc rport`.
/// Needs a running emulator and the built+installed HAPs (see findings.md); opt in with ANPOD_HDC_EMULATOR=1.
#[cfg(windows)]
#[test]
#[ignore = "needs the HarmonyOS emulator at 127.0.0.1:5555"]
fn hdc_record_stream_harmony_emulator_loopback() {
    use crate::{
        aoa_owner::AoaOwner, network::FloatPcmQueue, p5c::SessionAudioQueue,
        protocol::AudioFormatV2, security::test_adb_binding_material,
    };
    use std::{
        net::{Ipv4Addr, TcpListener},
        process::{Command, Stdio},
        sync::Arc,
        thread,
        time::Instant,
    };

    if std::env::var("ANPOD_HDC_EMULATOR").as_deref() != Ok("1") {
        return;
    }
    const KEY: &str = "127.0.0.1:5555";
    const TABLET_PORT: &str = "47000";
    const FLOAT_BITS: [u32; 8] = [
        0x3f80_0000,
        0xbf00_0000,
        0x0000_0001,
        0x8000_0000,
        0x7f7f_ffff,
        0x8000_0001,
        0x3eaa_aaab,
        0x0080_0000,
    ];
    const PACKETS: usize = 14;
    const FRAMES_PER_PACKET: usize = 143;

    let hdc = locate_hdc(
        std::env::var_os("PATH").as_deref(),
        deveco_install_location(),
    )
    .expect("hdc not found");
    let run = |args: &[&str]| {
        let output = Command::new(&hdc)
            .args(["-t", KEY])
            .args(args)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let pc_port = listener.local_addr().unwrap().port().to_string();
    let tablet = format!("tcp:{TABLET_PORT}");
    let pc = format!("tcp:{pc_port}");
    let _ = run(&["fport", "rm", &tablet, &pc]);
    let forward = run(&["rport", &tablet, &pc]);
    assert!(forward.contains("OK"), "hdc rport failed: {forward}");
    // Removes the rport and reaps `aa test` on every exit path, panics included.
    struct Cleanup {
        hdc: PathBuf,
        specs: [String; 2],
        child: Option<std::process::Child>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = Command::new(&self.hdc)
                .args(["-t", KEY, "fport", "rm", &self.specs[0], &self.specs[1]])
                .output();
        }
    }
    let mut cleanup = Cleanup {
        hdc: hdc.clone(),
        specs: [tablet.clone(), pc.clone()],
        child: None,
    };

    for profile in [4u8, 5] {
        cleanup.child = Some(
            Command::new(&hdc)
                .args([
                    "-t",
                    KEY,
                    "shell",
                    "aa",
                    "test",
                    "-b",
                    "app.anpod.harmony",
                    "-m",
                    "entry_test",
                ])
                .args([
                    "-s",
                    "unittest",
                    "OpenHarmonyTestRunner",
                    "-s",
                    "class",
                    "HdcLoopback",
                ])
                .args(["-s", "port", TABLET_PORT, "-s", "timeout", "90000"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .spawn()
                .expect("spawn aa test"),
        );
        let deadline = Instant::now() + Duration::from_secs(120);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "tablet never connected (profile {profile})"
                    );
                    thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("accept: {e}"),
            }
        };
        stream.set_nodelay(true).unwrap();
        let format = AudioFormatV2::for_profile(profile, 1).unwrap();
        let queue = Arc::new(FloatPcmQueue::new(true));
        queue.set_expected_format(format.sample_rate, 4096).unwrap();
        let owner = AoaOwner::connect_adb_stream(
            stream,
            test_adb_binding_material(),
            format,
            SessionAudioQueue::Float32(queue.clone(), format),
            "hdc",
            None,
        )
        .unwrap_or_else(|e| panic!("owner profile {profile}: {e}"));
        let snapshot = owner.snapshot();
        assert!(
            snapshot.capabilities.compatible,
            "incompatible, profile {profile}"
        );
        assert_eq!(snapshot.format, format);

        let block: Vec<f32> = (0..FRAMES_PER_PACKET * 2)
            .map(|s| f32::from_bits(FLOAT_BITS[s % FLOAT_BITS.len()]))
            .collect();
        for packet in 0..PACKETS {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !queue.push_raw(
                block.as_ptr(),
                FRAMES_PER_PACKET as u32,
                format.sample_rate,
                2,
                (packet * FRAMES_PER_PACKET) as u64,
            ) {
                assert!(Instant::now() < deadline, "queue refused packet {packet}");
                thread::yield_now();
            }
            let expected = ((packet + 1) * FRAMES_PER_PACKET * 8) as u64;
            let deadline = Instant::now() + Duration::from_secs(10);
            while owner.snapshot().pcm_payload_bytes_sent < expected {
                let s = owner.snapshot();
                assert!(
                    s.state != crate::p5c::ReceiverState::Stopped && Instant::now() < deadline,
                    "stalled at packet {packet} profile {profile}: sent={} err={:?}",
                    s.pcm_payload_bytes_sent,
                    s.last_error
                );
                thread::sleep(Duration::from_millis(2));
            }
        }
        owner.pause().unwrap();
        owner.resume().unwrap();
        owner.stop().unwrap();
        queue.stop_and_clear();

        let mut out = String::new();
        let mut test = cleanup.child.take().unwrap();
        std::io::Read::read_to_string(test.stdout.as_mut().unwrap(), &mut out).unwrap();
        test.wait().unwrap();
        println!("aa test profile {profile}:\n{out}");
        assert!(
            out.contains("OHOS_REPORT_CODE: 0") && out.contains("Failure: 0, Error: 0, Pass: 1"),
            "tablet side failed for profile {profile}: {out}"
        );
    }
    let removed = run(&["fport", "rm", &tablet, &pc]);
    assert!(removed.contains("success"), "fport rm: {removed}");
}
