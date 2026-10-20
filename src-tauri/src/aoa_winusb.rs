//! Explicit-accessory WinUSB transport foundation. Discovery is read-only;
//! callers must opt in by calling `discover_targets` and `open_target`.

use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ANPU_RECORD_LIMIT: Duration = Duration::from_secs(3);
const ZLP_RETRY_DELAY: Duration = Duration::from_millis(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AoaTarget {
    pub(crate) id: String,
    instance_id: String,
    interface_path: String,
}

impl AoaTarget {
    pub(crate) fn label(&self) -> String {
        "AnPod USB Direct".into()
    }
}

#[derive(Clone, Debug)]
struct DeviceNode {
    instance_id: String,
    service: String,
    interface_path: String,
    interface_guid: String,
}

fn accessory_target(node: DeviceNode) -> Option<AoaTarget> {
    let id = node.instance_id.to_ascii_uppercase();
    let service = node.service.to_ascii_lowercase();
    if !is_accessory_instance_id(&id) || service != "winusb" {
        return None;
    }
    let digest = Sha256::digest(
        [
            node.instance_id.as_bytes(),
            node.interface_guid.as_bytes(),
            node.interface_path.as_bytes(),
        ]
        .concat(),
    );
    let id = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(AoaTarget {
        id,
        instance_id: node.instance_id,
        interface_path: node.interface_path,
    })
}

fn is_accessory_instance_id(id: &str) -> bool {
    let mut parts = id.split('\\');
    let bus = parts.next().unwrap_or_default();
    let hardware_id = parts.next().unwrap_or_default();
    let tokens: Vec<_> = hardware_id.split('&').collect();
    let vendor_ok = tokens.contains(&"VID_18D1");
    let product = if tokens.contains(&"PID_2D00") {
        0
    } else if tokens.contains(&"PID_2D01") {
        1
    } else {
        return false;
    };
    let is_accessory = bus == "USB" && vendor_ok;
    let is_data_interface = if product == 0 {
        !tokens.iter().any(|token| token.starts_with("MI_"))
    } else {
        tokens.contains(&"MI_00")
    };
    is_accessory && is_data_interface
}

fn is_iqoo_11_pre_aoa_instance_id(id: &str) -> bool {
    let mut parts = id.split('\\');
    let bus = parts.next().unwrap_or_default();
    let hardware_id = parts.next().unwrap_or_default();
    let tokens: Vec<_> = hardware_id.split('&').collect();
    bus == "USB"
        && tokens.contains(&"VID_2D95")
        && tokens.contains(&"PID_6013")
        && tokens.contains(&"MI_02")
}

#[derive(Clone, Debug)]
pub(crate) struct AoaSwitchTarget {
    pub(crate) id: String,
    instance_id: String,
    interface_path: String,
}

impl AoaSwitchTarget {
    pub(crate) fn label(&self) -> String {
        "iQOO 11 V2243A".into()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BulkPipes {
    input: u8,
    output: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PipeDescriptor {
    id: u8,
    is_bulk: bool,
}

fn query_bulk_pipes(
    count: u8,
    mut query: impl FnMut(u8) -> io::Result<PipeDescriptor>,
) -> io::Result<BulkPipes> {
    let mut input = None;
    let mut output = None;
    for index in 0..count {
        let pipe = query(index)?;
        if !pipe.is_bulk {
            continue;
        }
        let slot = if pipe.id & 0x80 != 0 {
            &mut input
        } else {
            &mut output
        };
        if slot.replace(pipe.id).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "multiple bulk pipes in one direction",
            ));
        }
    }
    match (input, output) {
        (Some(input), Some(output)) => Ok(BulkPipes { input, output }),
        _ => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "accessory interface has no bulk IN/OUT pair",
        )),
    }
}

fn for_registered_interface_guids<T>(
    guids: &[String],
    mut enumerate: impl FnMut(&str) -> io::Result<Vec<T>>,
) -> io::Result<Vec<T>> {
    let mut interfaces = Vec::new();
    for guid in guids {
        interfaces.extend(enumerate(guid)?);
    }
    Ok(interfaces)
}

fn is_win32_hresult(hresult: i32, win32_error: u32) -> bool {
    hresult as u32 == (0x8007_0000 | (win32_error & 0xffff))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadFailure {
    NoProgress,
    PartialTimeout,
    Cancelled,
    Fatal,
}

fn classify_read_failure(hresult: i32, cancelled: bool, transferred: u32) -> ReadFailure {
    if is_win32_hresult(hresult, 121) {
        if transferred == 0 {
            ReadFailure::NoProgress
        } else {
            ReadFailure::PartialTimeout
        }
    } else if is_win32_hresult(hresult, 995) && cancelled {
        ReadFailure::Cancelled
    } else {
        ReadFailure::Fatal
    }
}

fn parse_registered_interface_guids(value_type: u32, bytes: &[u8]) -> io::Result<Vec<String>> {
    if !bytes.len().is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "odd UTF-16 registry value length",
        ));
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let strings = match value_type {
        7 => {
            let mut values = Vec::new();
            let mut start = 0;
            for (index, unit) in units.iter().enumerate() {
                if *unit == 0 {
                    if index > start {
                        values.push(String::from_utf16_lossy(&units[start..index]));
                    }
                    start = index + 1;
                }
            }
            if start < units.len() {
                values.push(String::from_utf16_lossy(&units[start..]));
            }
            values
        }
        1 => {
            let end = units
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(units.len());
            vec![String::from_utf16_lossy(&units[..end])]
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected DeviceInterfaceGUIDs registry type",
            ))
        }
    };
    strings
        .into_iter()
        .filter(|guid| !guid.trim().is_empty())
        .map(|guid| {
            let trimmed = guid.trim().trim_start_matches('{').trim_end_matches('}');
            let valid = trimmed.len() == 36
                && trimmed
                    .bytes()
                    .enumerate()
                    .all(|(index, byte)| match index {
                        8 | 13 | 18 | 23 => byte == b'-',
                        _ => byte.is_ascii_hexdigit(),
                    });
            if valid {
                Ok(guid)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid registered interface GUID",
                ))
            }
        })
        .collect()
}

trait PipeIo: Send + Sync {
    fn read_pipe(&self, pipe: u8, buffer: &mut [u8]) -> io::Result<usize>;
    fn write_pipe(&self, pipe: u8, buffer: &[u8]) -> io::Result<usize>;
    fn cancel(&self);
    fn is_cancelled(&self) -> bool;
}

pub(crate) struct AoaReader {
    io: Arc<dyn PipeIo>,
    pipe: u8,
    record_deadline: Option<Instant>,
}

pub(crate) struct AoaWriter {
    io: Arc<dyn PipeIo>,
    pipe: u8,
    record_deadline: Option<Instant>,
}

#[derive(Clone)]
pub(crate) struct AoaCancel(Arc<dyn PipeIo>);

pub(crate) struct AoaStream {
    pub(crate) reader: AoaReader,
    pub(crate) writer: AoaWriter,
    pub(crate) cancel: AoaCancel,
}

// Cancel first, join workers using reader/writer, then drop the stream. Native
// handles remain Arc-owned until the last worker and control handle is gone.

impl AoaCancel {
    pub(crate) fn cancel(&self) {
        self.0.cancel();
    }
}

impl AoaReader {
    pub(crate) fn read_anpu_record(&mut self) -> io::Result<crate::aoa::Record> {
        self.record_deadline = None;
        let result = crate::aoa::read_record(self);
        self.record_deadline = None;
        if result.is_err() {
            self.io.cancel();
        }
        result
    }
}

impl AoaWriter {
    pub(crate) fn write_anpu_record(&mut self, record: &crate::aoa::Record) -> io::Result<()> {
        self.record_deadline = Some(Instant::now() + ANPU_RECORD_LIMIT);
        let result = crate::aoa::write_record(self, record);
        self.record_deadline = None;
        if result.is_err() {
            self.io.cancel();
        }
        result
    }
}

impl Read for AoaReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            if self
                .record_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "ANPU record read deadline expired",
                ));
            }
            if self.io.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "AOA I/O cancelled",
                ));
            }
            let read = self.io.read_pipe(self.pipe, output)?;
            if read != 0 {
                if self.record_deadline.is_none() {
                    self.record_deadline = Some(Instant::now() + ANPU_RECORD_LIMIT);
                } else if self
                    .record_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "ANPU record read deadline expired after partial data",
                    ));
                }
                return Ok(read);
            }
            if self
                .record_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "ANPU record read deadline expired",
                ));
            }
            std::thread::sleep(ZLP_RETRY_DELAY);
        }
    }
}

impl Write for AoaWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        let deadline = *self
            .record_deadline
            .get_or_insert_with(|| Instant::now() + ANPU_RECORD_LIMIT);
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "ANPU record write deadline expired",
                ));
            }
            if self.io.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "AOA I/O cancelled",
                ));
            }
            let written = self.io.write_pipe(self.pipe, input)?;
            if written != 0 {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "ANPU record write deadline expired after partial data",
                    ));
                }
                return Ok(written);
            }
            std::thread::sleep(ZLP_RETRY_DELAY);
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
mod windows_backend {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::core::{GUID, HRESULT, PCWSTR, PWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiEnumDeviceInterfaces,
        SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceInterfaceDetailW,
        SetupDiGetDeviceRegistryPropertyW, SetupDiOpenDevRegKey, DICS_FLAG_GLOBAL,
        DIGCF_ALLCLASSES, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, DIREG_DEV, HDEVINFO, SPDRP_SERVICE,
        SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
    };
    use windows::Win32::Devices::Usb::{
        UsbdPipeTypeBulk, WinUsb_AbortPipe, WinUsb_ControlTransfer, WinUsb_Free, WinUsb_Initialize,
        WinUsb_QueryInterfaceSettings, WinUsb_QueryPipe, WinUsb_ReadPipe, WinUsb_SetPipePolicy,
        WinUsb_WritePipe, PIPE_TRANSFER_TIMEOUT, USB_INTERFACE_DESCRIPTOR, WINUSB_INTERFACE_HANDLE,
        WINUSB_PIPE_INFORMATION,
    };
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_OPERATION_ABORTED,
        ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Registry::{RegCloseKey, RegQueryValueExW, HKEY, KEY_QUERY_VALUE};

    const IO_TIMEOUT_MS: u32 = 100;

    struct DeviceInfoSet(HDEVINFO);
    impl Drop for DeviceInfoSet {
        fn drop(&mut self) {
            let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
        }
    }

    fn last_error(error: impl std::fmt::Display, what: &str) -> io::Error {
        io::Error::other(format!("{what}: {error}"))
    }

    fn device_instance_id(set: HDEVINFO, devinfo: &SP_DEVINFO_DATA) -> io::Result<String> {
        let mut buffer = vec![0u16; 1024];
        unsafe { SetupDiGetDeviceInstanceIdW(set, devinfo, Some(&mut buffer), None) }
            .map_err(|error| last_error(error, "SetupDiGetDeviceInstanceIdW"))?;
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        Ok(String::from_utf16_lossy(&buffer[..end]))
    }

    fn device_service(set: HDEVINFO, devinfo: &SP_DEVINFO_DATA) -> io::Result<String> {
        let mut bytes = [0u8; 512];
        unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                set,
                devinfo,
                SPDRP_SERVICE,
                None,
                Some(&mut bytes),
                None,
            )
        }
        .map_err(|error| last_error(error, "read device service"))?;
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        Ok(String::from_utf16_lossy(&units[..end]))
    }

    fn registered_interface_guids(
        set: HDEVINFO,
        devinfo: &SP_DEVINFO_DATA,
    ) -> io::Result<Vec<String>> {
        let hardware_key = unsafe {
            SetupDiOpenDevRegKey(
                set,
                devinfo,
                DICS_FLAG_GLOBAL.0,
                0,
                DIREG_DEV,
                KEY_QUERY_VALUE.0,
            )
        }
        .map_err(|error| last_error(error, "open device registry key"))?;
        struct RegKey(windows::Win32::System::Registry::HKEY);
        impl Drop for RegKey {
            fn drop(&mut self) {
                let _ = unsafe { RegCloseKey(self.0) };
            }
        }
        let hardware_key = RegKey(hardware_key);

        let mut guids = Vec::new();
        for (name, expected_type) in [("DeviceInterfaceGUIDs", 7), ("DeviceInterfaceGUID", 1)] {
            if let Some((value_type, bytes)) = query_registry_value(hardware_key.0, name)? {
                if value_type != expected_type {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected {name} registry type {value_type}"),
                    ));
                }
                guids.extend(parse_registered_interface_guids(value_type, &bytes)?);
            }
        }
        guids.sort_unstable_by_key(|guid| guid.to_ascii_lowercase());
        guids.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        Ok(guids)
    }

    fn query_registry_value(key: HKEY, name: &str) -> io::Result<Option<(u32, Vec<u8>)>> {
        let value_name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut value_type = windows::Win32::System::Registry::REG_VALUE_TYPE(0);
        let mut byte_count = 0u32;
        let result = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(value_name.as_ptr()),
                None,
                Some(&mut value_type),
                None,
                Some(&mut byte_count),
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if result != ERROR_SUCCESS {
            return Err(io::Error::other(format!(
                "query {name} registry value size: Win32 error {}",
                result.0
            )));
        }
        if byte_count == 0 || byte_count > 64 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name} registry value size is invalid"),
            ));
        }
        let mut bytes = vec![0u8; byte_count as usize];
        let result = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(value_name.as_ptr()),
                None,
                Some(&mut value_type),
                Some(bytes.as_mut_ptr()),
                Some(&mut byte_count),
            )
        };
        if result != ERROR_SUCCESS {
            return Err(io::Error::other(format!(
                "read {name} registry value: Win32 error {}",
                result.0
            )));
        }
        bytes.truncate(byte_count as usize);
        Ok(Some((value_type.0, bytes)))
    }

    fn enumerate_nodes(is_target_instance_id: fn(&str) -> bool) -> io::Result<Vec<DeviceNode>> {
        let set = unsafe {
            SetupDiGetClassDevsW(
                None,
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_ALLCLASSES | DIGCF_DEVICEINTERFACE,
            )
        }
        .map(DeviceInfoSet)
        .map_err(|error| last_error(error, "SetupDiGetClassDevsW"))?;
        let mut found = Vec::new();
        let mut device_index = 0;
        loop {
            let mut devinfo = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            let result = unsafe { SetupDiEnumDeviceInfo(set.0, device_index, &mut devinfo) };
            if let Err(error) = result {
                if error.code() == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) {
                    break;
                }
                return Err(last_error(error, "SetupDiEnumDeviceInfo"));
            }
            device_index += 1;
            let Ok(instance_id) = device_instance_id(set.0, &devinfo) else {
                continue;
            };
            if !is_target_instance_id(&instance_id.to_ascii_uppercase()) {
                continue;
            }
            let Ok(service) = device_service(set.0, &devinfo) else {
                continue;
            };
            if !service.eq_ignore_ascii_case("WinUSB") {
                continue;
            }
            let Ok(registered_guids) = registered_interface_guids(set.0, &devinfo) else {
                continue;
            };
            let interfaces = for_registered_interface_guids(&registered_guids, |registered| {
                let guid = GUID::try_from(
                    registered
                        .trim()
                        .trim_start_matches('{')
                        .trim_end_matches('}'),
                )
                .map_err(|error| last_error(error, "parse registered interface GUID"))?;
                let mut interfaces = Vec::new();
                let mut interface_index = 0;
                loop {
                    let mut interface = SP_DEVICE_INTERFACE_DATA {
                        cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                        ..Default::default()
                    };
                    if let Err(error) = unsafe {
                        SetupDiEnumDeviceInterfaces(
                            set.0,
                            Some(&devinfo),
                            &guid,
                            interface_index,
                            &mut interface,
                        )
                    } {
                        if error.code() == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) {
                            break;
                        }
                        return Err(last_error(error, "SetupDiEnumDeviceInterfaces"));
                    }
                    interface_index += 1;
                    let mut required = 0;
                    let _ = unsafe {
                        SetupDiGetDeviceInterfaceDetailW(
                            set.0,
                            &interface,
                            None,
                            0,
                            Some(&mut required),
                            None,
                        )
                    };
                    if required < std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32
                        || required > 16 * 1024
                    {
                        continue;
                    }
                    let mut detail = vec![0u64; (required as usize).div_ceil(8)];
                    let detail_ptr = detail
                        .as_mut_ptr()
                        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
                    unsafe {
                        (*detail_ptr).cbSize = if cfg!(target_pointer_width = "64") {
                            8
                        } else {
                            6
                        }
                    };
                    let mut interface_devinfo = SP_DEVINFO_DATA {
                        cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                        ..Default::default()
                    };
                    if unsafe {
                        SetupDiGetDeviceInterfaceDetailW(
                            set.0,
                            &interface,
                            Some(detail_ptr),
                            required,
                            None,
                            Some(&mut interface_devinfo),
                        )
                    }
                    .is_err()
                    {
                        continue;
                    }
                    let Ok(interface_instance) = device_instance_id(set.0, &interface_devinfo)
                    else {
                        continue;
                    };
                    if !interface_instance.eq_ignore_ascii_case(&instance_id) {
                        continue;
                    }
                    let Ok(path) =
                        (unsafe { PWSTR((*detail_ptr).DevicePath.as_mut_ptr()).to_string() })
                    else {
                        continue;
                    };
                    interfaces.push(DeviceNode {
                        instance_id: instance_id.clone(),
                        service: service.clone(),
                        interface_path: path,
                        interface_guid: registered.to_owned(),
                    });
                }
                Ok(interfaces)
            })?;
            found.extend(interfaces);
        }
        Ok(found)
    }

    struct WinUsbIo {
        file: HANDLE,
        interface: WINUSB_INTERFACE_HANDLE,
        pipes: BulkPipes,
        cancelled: AtomicBool,
    }

    // WinUSB permits one reader and one writer on distinct pipes. The Arc keeps
    // both native handles alive until each blocking transfer has returned.
    unsafe impl Send for WinUsbIo {}
    unsafe impl Sync for WinUsbIo {}

    impl Drop for WinUsbIo {
        fn drop(&mut self) {
            let _ = unsafe { WinUsb_Free(self.interface) };
            let _ = unsafe { CloseHandle(self.file) };
        }
    }

    impl PipeIo for WinUsbIo {
        fn read_pipe(&self, pipe: u8, output: &mut [u8]) -> io::Result<usize> {
            let mut transferred = 0;
            let result = unsafe {
                WinUsb_ReadPipe(
                    self.interface,
                    pipe,
                    Some(output),
                    Some(&mut transferred),
                    None,
                )
            };
            if let Err(error) = result {
                match classify_read_failure(error.code().0, self.is_cancelled(), transferred) {
                    ReadFailure::NoProgress => return Ok(0),
                    ReadFailure::PartialTimeout => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "WinUsb_ReadPipe timed out after transferring bytes",
                        ));
                    }
                    ReadFailure::Cancelled => {
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionAborted,
                            "AOA I/O cancelled",
                        ));
                    }
                    ReadFailure::Fatal => {}
                }
                return Err(last_error(error, "WinUsb_ReadPipe"));
            }
            Ok(transferred as usize)
        }

        fn write_pipe(&self, pipe: u8, input: &[u8]) -> io::Result<usize> {
            let mut transferred = 0;
            let result = unsafe {
                WinUsb_WritePipe(self.interface, pipe, input, Some(&mut transferred), None)
            };
            if let Err(error) = result {
                if is_win32_hresult(error.code().0, ERROR_OPERATION_ABORTED.0)
                    && self.is_cancelled()
                {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "AOA I/O cancelled",
                    ));
                }
                return Err(last_error(error, "WinUsb_WritePipe"));
            }
            Ok(transferred as usize)
        }

        fn cancel(&self) {
            if !self.cancelled.swap(true, Ordering::AcqRel) {
                let _ = unsafe { WinUsb_AbortPipe(self.interface, self.pipes.input) };
                let _ = unsafe { WinUsb_AbortPipe(self.interface, self.pipes.output) };
            }
        }

        fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Acquire)
        }
    }

    #[allow(dead_code)]
    pub(crate) fn discover_targets() -> io::Result<Vec<AoaTarget>> {
        Ok(enumerate_nodes(is_accessory_instance_id)?
            .into_iter()
            .filter_map(accessory_target)
            .collect())
    }

    fn switch_target(node: DeviceNode) -> Option<AoaSwitchTarget> {
        let id = node.instance_id.to_ascii_uppercase();
        if !is_iqoo_11_pre_aoa_instance_id(&id) || !node.service.eq_ignore_ascii_case("WinUSB") {
            return None;
        }
        let digest = Sha256::digest(
            [
                node.instance_id.as_bytes(),
                node.interface_guid.as_bytes(),
                node.interface_path.as_bytes(),
            ]
            .concat(),
        );
        Some(AoaSwitchTarget {
            id: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
            instance_id: node.instance_id,
            interface_path: node.interface_path,
        })
    }

    pub(crate) fn discover_switch_targets() -> io::Result<Vec<AoaSwitchTarget>> {
        Ok(enumerate_nodes(is_iqoo_11_pre_aoa_instance_id)?
            .into_iter()
            .filter_map(switch_target)
            .collect())
    }

    struct SwitchDevice {
        file: HANDLE,
        interface: WINUSB_INTERFACE_HANDLE,
    }

    impl Drop for SwitchDevice {
        fn drop(&mut self) {
            let _ = unsafe { WinUsb_Free(self.interface) };
            let _ = unsafe { CloseHandle(self.file) };
        }
    }

    fn open_switch_target(target: &AoaSwitchTarget) -> io::Result<SwitchDevice> {
        let path: Vec<u16> = target
            .interface_path
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let file = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .map_err(|error| last_error(error, "open selected iQOO 11 WinUSB interface (MI_02)"))?;
        let mut interface = WINUSB_INTERFACE_HANDLE::default();
        if let Err(error) = unsafe { WinUsb_Initialize(file, &mut interface) } {
            let _ = unsafe { CloseHandle(file) };
            return Err(last_error(
                error,
                "initialize selected iQOO 11 WinUSB interface",
            ));
        }
        Ok(SwitchDevice { file, interface })
    }

    fn control_transfer(
        device: &SwitchDevice,
        request_type: u8,
        request: u8,
        index: u16,
        buffer: &mut [u8],
        cancelled: &impl Fn() -> bool,
    ) -> io::Result<usize> {
        use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
        use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

        if cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "AOA switch cancelled",
            ));
        }
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map_err(|error| last_error(error, "create USB control-transfer event"))?;
        struct Event(HANDLE);
        impl Drop for Event {
            fn drop(&mut self) {
                let _ = unsafe { CloseHandle(self.0) };
            }
        }
        let event = Event(event);
        #[allow(unused_mut)]
        let mut overlapped = OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        };
        let setup = windows::Win32::Devices::Usb::WINUSB_SETUP_PACKET {
            RequestType: request_type,
            Request: request,
            Value: 0,
            Index: index,
            Length: buffer.len() as u16,
        };
        let mut transferred = 0u32;
        let result = unsafe {
            WinUsb_ControlTransfer(
                device.interface,
                setup,
                Some(buffer),
                Some(&mut transferred),
                Some(&overlapped),
            )
        };
        if result.is_ok() {
            return Ok(transferred as usize);
        }
        let error = result.unwrap_err();
        if !is_win32_hresult(
            error.code().0,
            windows::Win32::Foundation::ERROR_IO_PENDING.0,
        ) {
            return Err(last_error(error, "WinUsb_ControlTransfer"));
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if cancelled() || Instant::now() >= deadline {
                let _ = unsafe { CancelIoEx(device.file, Some(&overlapped)) };
                let mut ignored = 0u32;
                let _ =
                    unsafe { GetOverlappedResult(device.file, &overlapped, &mut ignored, true) };
                return Err(if cancelled() {
                    io::Error::new(io::ErrorKind::Interrupted, "AOA switch cancelled")
                } else {
                    io::Error::new(io::ErrorKind::TimedOut, "AOA control transfer timed out")
                });
            }
            let wait = unsafe { WaitForSingleObject(event.0, 50) };
            if wait == WAIT_TIMEOUT {
                continue;
            }
            if wait != WAIT_OBJECT_0 {
                let error = unsafe { windows::Win32::Foundation::GetLastError().0 };
                let _ = unsafe { CancelIoEx(device.file, Some(&overlapped)) };
                let mut ignored = 0u32;
                let _ =
                    unsafe { GetOverlappedResult(device.file, &overlapped, &mut ignored, true) };
                return Err(io::Error::other(format!(
                    "wait for USB control transfer failed: {}",
                    error
                )));
            }
            unsafe { GetOverlappedResult(device.file, &overlapped, &mut transferred, false) }
                .map_err(|error| last_error(error, "complete WinUsb_ControlTransfer"))?;
            return Ok(transferred as usize);
        }
    }

    pub(crate) fn switch_to_accessory(
        target_id: &str,
        cancelled: impl Fn() -> bool,
    ) -> io::Result<()> {
        let target = discover_switch_targets()?
            .into_iter()
            .find(|target| target.id == target_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "selected iQOO 11 target is no longer available",
                )
            })?;
        let _identity = &target.instance_id;
        let device = open_switch_target(&target)?;
        let mut protocol = [0u8; 2];
        let transferred = control_transfer(&device, 0xC0, 51, 0, &mut protocol, &cancelled)?;
        if transferred != protocol.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "AOA GET_PROTOCOL returned an incomplete response",
            ));
        }
        if u16::from_le_bytes(protocol) < 1 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "phone does not support Android Open Accessory mode",
            ));
        }
        for (index, value) in [
            "AnPod",
            "AnPod USB Direct",
            "AnPod USB Direct audio receiver",
            "1.0",
            "https://github.com/sanren1/AnPod",
            "ANPOD-AOA-USB-DIRECT",
        ]
        .into_iter()
        .enumerate()
        {
            let mut bytes = value.as_bytes().to_vec();
            bytes.push(0);
            if bytes.len() > 256 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "AOA identity string exceeds 256 bytes",
                ));
            }
            let transferred =
                control_transfer(&device, 0x40, 52, index as u16, &mut bytes, &cancelled)?;
            if transferred != bytes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!(
                        "AOA SEND_STRING {index} transferred {transferred}/{} bytes",
                        bytes.len()
                    ),
                ));
            }
        }
        let mut empty = [];
        let transferred = control_transfer(&device, 0x40, 53, 0, &mut empty, &cancelled)?;
        if transferred != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "AOA START_ACCESSORY returned unexpected data",
            ));
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn open_target(target: &AoaTarget) -> io::Result<AoaStream> {
        let _identity = &target.instance_id;
        let wide_path: Vec<u16> = target
            .interface_path
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let file = unsafe {
            CreateFileW(
                PCWSTR(wide_path.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .map_err(|error| last_error(error, "CreateFileW"))?;
        let mut interface = WINUSB_INTERFACE_HANDLE::default();
        if let Err(error) = unsafe { WinUsb_Initialize(file, &mut interface) } {
            let _ = unsafe { CloseHandle(file) };
            return Err(last_error(error, "WinUsb_Initialize"));
        }
        let mut descriptor = USB_INTERFACE_DESCRIPTOR::default();
        if let Err(error) = unsafe { WinUsb_QueryInterfaceSettings(interface, 0, &mut descriptor) }
        {
            let _ = unsafe { WinUsb_Free(interface) };
            let _ = unsafe { CloseHandle(file) };
            return Err(last_error(error, "WinUsb_QueryInterfaceSettings"));
        }
        let pipes = match query_bulk_pipes(descriptor.bNumEndpoints, |index| {
            let mut pipe = WINUSB_PIPE_INFORMATION::default();
            unsafe { WinUsb_QueryPipe(interface, 0, index, &mut pipe) }
                .map_err(|error| last_error(error, "WinUsb_QueryPipe"))?;
            Ok(PipeDescriptor {
                id: pipe.PipeId,
                is_bulk: pipe.PipeType == UsbdPipeTypeBulk,
            })
        }) {
            Ok(pipes) => pipes,
            Err(error) => {
                let _ = unsafe { WinUsb_Free(interface) };
                let _ = unsafe { CloseHandle(file) };
                return Err(error);
            }
        };
        let io = Arc::new(WinUsbIo {
            file,
            interface,
            pipes,
            cancelled: AtomicBool::new(false),
        });
        for pipe in [pipes.input, pipes.output] {
            let timeout = IO_TIMEOUT_MS;
            unsafe {
                WinUsb_SetPipePolicy(
                    io.interface,
                    pipe,
                    PIPE_TRANSFER_TIMEOUT,
                    std::mem::size_of_val(&timeout) as u32,
                    (&timeout as *const u32).cast(),
                )
            }
            .map_err(|error| last_error(error, "set WinUSB transfer timeout"))?;
        }
        let io: Arc<dyn PipeIo> = io;
        Ok(AoaStream {
            reader: AoaReader {
                io: io.clone(),
                pipe: pipes.input,
                record_deadline: None,
            },
            writer: AoaWriter {
                io: io.clone(),
                pipe: pipes.output,
                record_deadline: None,
            },
            cancel: AoaCancel(io),
        })
    }
}

#[cfg(windows)]
#[allow(dead_code)]
pub(crate) fn discover_targets() -> io::Result<Vec<AoaTarget>> {
    windows_backend::discover_targets()
}

#[cfg(windows)]
#[allow(dead_code)]
pub(crate) fn open_target(target: &AoaTarget) -> io::Result<AoaStream> {
    windows_backend::open_target(target)
}

#[cfg(windows)]
pub(crate) fn discover_switch_targets() -> io::Result<Vec<AoaSwitchTarget>> {
    windows_backend::discover_switch_targets()
}

#[cfg(windows)]
pub(crate) fn switch_to_accessory(target_id: &str, cancelled: impl Fn() -> bool) -> io::Result<()> {
    windows_backend::switch_to_accessory(target_id, cancelled)
}

#[cfg(not(windows))]
pub(crate) fn discover_targets() -> io::Result<Vec<AoaTarget>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "AOA WinUSB is Windows-only",
    ))
}

#[cfg(not(windows))]
pub(crate) fn open_target(_: &AoaTarget) -> io::Result<AoaStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "AOA WinUSB is Windows-only",
    ))
}

#[cfg(not(windows))]
pub(crate) fn discover_switch_targets() -> io::Result<Vec<AoaSwitchTarget>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "AOA WinUSB is Windows-only",
    ))
}

#[cfg(not(windows))]
pub(crate) fn switch_to_accessory(_: &str, _: impl Fn() -> bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "AOA WinUSB is Windows-only",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn production_accessory_filter_excludes_adb_and_wrong_driver_nodes() {
        let cases = [
            ("USB\\VID_18D1&PID_2D00\\one", "WinUSB", true),
            ("USB\\VID_18D1&PID_2D00&MI_01\\adb", "WinUSB", false),
            ("USB\\VID_18D1&PID_2D01&MI_00\\two", "winusb", true),
            ("USB\\VID_18D1&PID_2D01&MI_01\\adb", "WinUSB", false),
            ("USB\\VID_18D1&PID_2D01\\parent", "WinUSB", false),
            ("USB\\VID_18D1&PID_2D01&MI_000\\bad-mi", "WinUSB", false),
            ("USB\\VID_18D1&PID_2D000\\bad-pid", "WinUSB", false),
            (
                "USB\\VID_18D1&PID_2D00\\adb",
                "AndroidUsbDeviceClass",
                false,
            ),
            ("USB\\VID_1234&PID_2D00\\other", "WinUSB", false),
        ];
        for (instance_id, service, accepted) in cases {
            assert_eq!(
                accessory_target(DeviceNode {
                    instance_id: instance_id.into(),
                    service: service.into(),
                    interface_path: "test-path".into(),
                    interface_guid: "{registered-guid}".into(),
                })
                .is_some(),
                accepted,
                "{instance_id}"
            );
        }

        let target = accessory_target(DeviceNode {
            instance_id: "USB\\VID_18D1&PID_2D00\\private-serial".into(),
            service: "WinUSB".into(),
            interface_path: "test-path".into(),
            interface_guid: "{registered-guid}".into(),
        })
        .unwrap();
        assert_eq!(target.label(), "AnPod USB Direct");
    }

    #[test]
    fn pre_aoa_switch_filter_accepts_only_the_confirmed_iqoo_interface() {
        let cases = [
            ("USB\\VID_2D95&PID_6013&MI_02\\device", true),
            ("USB\\VID_2D95&PID_6013&MI_01\\adb", false),
            ("USB\\VID_2D95&PID_6013&MI_03\\other", false),
            ("USB\\VID_2D95&PID_6013\\parent", false),
            ("USB\\VID_2D95&PID_60130&MI_02\\wrong-pid", false),
            ("USB\\VID_2D95&PID_6013&MI_020\\wrong-mi", false),
            ("USB\\VID_1234&PID_6013&MI_02\\other", false),
            ("USB\\VID_2D95&PID_6013&MI_02&REV_0001\\rev", true),
        ];
        for (instance_id, accepted) in cases {
            assert_eq!(
                is_iqoo_11_pre_aoa_instance_id(instance_id),
                accepted,
                "{instance_id}"
            );
        }
    }

    #[test]
    fn production_registered_guid_enumerator_never_uses_an_unspecified_class() {
        let guids = vec![
            "{00112233-4455-6677-8899-AABBCCDDEEFF}".to_owned(),
            "{11223344-5566-7788-99AA-BBCCDDEEFF00}".to_owned(),
        ];
        let mut queried = Vec::new();
        let found = for_registered_interface_guids(&guids, |guid| {
            queried.push(guid.to_owned());
            Ok(vec![guid.to_owned()])
        })
        .unwrap();
        assert_eq!(queried, guids);
        assert_eq!(found, queried);

        let mut called = false;
        let empty: Vec<String> = for_registered_interface_guids(&[], |_| {
            called = true;
            Ok::<Vec<String>, io::Error>(vec![])
        })
        .unwrap();
        assert!(empty.is_empty());
        assert!(!called);
    }

    #[test]
    fn registered_interface_guid_values_are_decoded_as_little_endian_utf16() {
        let value = [
            "{00112233-4455-6677-8899-AABBCCDDEEFF}",
            "{11223344-5566-7788-99AA-BBCCDDEEFF00}",
        ]
        .join("\0")
            + "\0\0";
        let bytes: Vec<u8> = value.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(
            parse_registered_interface_guids(7, &bytes).unwrap(),
            vec![
                "{00112233-4455-6677-8899-AABBCCDDEEFF}".to_owned(),
                "{11223344-5566-7788-99AA-BBCCDDEEFF00}".to_owned()
            ]
        );
        let single = "{00112233-4455-6677-8899-AABBCCDDEEFF}\0";
        let single_bytes: Vec<u8> = single.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(
            parse_registered_interface_guids(1, &single_bytes).unwrap(),
            vec!["{00112233-4455-6677-8899-AABBCCDDEEFF}".to_owned()]
        );
        assert!(parse_registered_interface_guids(7, &[0]).is_err());
    }

    #[test]
    fn win32_hresult_matching_uses_hresult_from_win32_encoding() {
        assert!(is_win32_hresult(0x8007_0079u32 as i32, 121));
        assert!(is_win32_hresult(0x8007_03e3u32 as i32, 995));
        assert!(!is_win32_hresult(121, 121));
        assert!(!is_win32_hresult(0x8007_03e3u32 as i32, 121));
        assert_eq!(
            classify_read_failure(0x8007_0079u32 as i32, false, 0),
            ReadFailure::NoProgress
        );
        assert_eq!(
            classify_read_failure(0x8007_0079u32 as i32, false, 3),
            ReadFailure::PartialTimeout
        );
        assert_eq!(
            classify_read_failure(0x8007_03e3u32 as i32, true, 0),
            ReadFailure::Cancelled
        );
        assert_eq!(
            classify_read_failure(0x8007_03e3u32 as i32, false, 0),
            ReadFailure::Fatal
        );
        assert_eq!(
            classify_read_failure(0x8007_0005u32 as i32, false, 0),
            ReadFailure::Fatal
        );
    }

    #[test]
    fn production_pipe_query_uses_reported_bulk_ids_and_rejects_ambiguous_pairs() {
        let queried = Mutex::new(Vec::new());
        let pipes = query_bulk_pipes(4, |index| {
            queried.lock().unwrap().push(index);
            Ok([
                PipeDescriptor {
                    id: 0x83,
                    is_bulk: true,
                },
                PipeDescriptor {
                    id: 0x02,
                    is_bulk: true,
                },
                PipeDescriptor {
                    id: 0x84,
                    is_bulk: false,
                },
                PipeDescriptor {
                    id: 0x05,
                    is_bulk: false,
                },
            ][index as usize])
        })
        .unwrap();
        assert_eq!(
            pipes,
            BulkPipes {
                input: 0x83,
                output: 0x02
            }
        );
        assert_eq!(*queried.lock().unwrap(), vec![0, 1, 2, 3]);
        assert!(query_bulk_pipes(2, |index| Ok(PipeDescriptor {
            id: if index == 0 { 0x81 } else { 0x82 },
            is_bulk: true,
        }))
        .is_err());
    }

    struct FakeIo {
        input: Mutex<Vec<u8>>,
        output: Mutex<Vec<u8>>,
        max_transfer: usize,
        zero_read: std::sync::atomic::AtomicBool,
        zero_write: std::sync::atomic::AtomicBool,
        cancelled: std::sync::atomic::AtomicBool,
        dropped: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Drop for FakeIo {
        fn drop(&mut self) {
            self.dropped
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    impl PipeIo for FakeIo {
        fn read_pipe(&self, _: u8, output: &mut [u8]) -> io::Result<usize> {
            if self.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "cancelled",
                ));
            }
            if self
                .zero_read
                .swap(false, std::sync::atomic::Ordering::AcqRel)
            {
                return Ok(0);
            }
            let mut input = self.input.lock().unwrap();
            if input.is_empty() {
                return Ok(0);
            }
            let count = output.len().min(input.len()).min(self.max_transfer);
            output[..count].copy_from_slice(&input[..count]);
            input.drain(..count);
            Ok(count)
        }

        fn write_pipe(&self, _: u8, input: &[u8]) -> io::Result<usize> {
            if self.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "cancelled",
                ));
            }
            if self
                .zero_write
                .swap(false, std::sync::atomic::Ordering::AcqRel)
            {
                return Ok(0);
            }
            let count = input.len().min(self.max_transfer);
            self.output
                .lock()
                .unwrap()
                .extend_from_slice(&input[..count]);
            Ok(count)
        }

        fn cancel(&self) {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::Release);
        }

        fn is_cancelled(&self) -> bool {
            self.cancelled.load(std::sync::atomic::Ordering::Acquire)
        }
    }

    #[test]
    fn production_stream_adapters_preserve_offsets_across_short_transfers() {
        let payload = b"partial USB record contents";
        let fake = Arc::new(FakeIo {
            input: Mutex::new(payload.to_vec()),
            output: Mutex::new(Vec::new()),
            max_transfer: 3,
            zero_read: std::sync::atomic::AtomicBool::new(true),
            zero_write: std::sync::atomic::AtomicBool::new(true),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            dropped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let mut reader = AoaReader {
            io: fake.clone(),
            pipe: 0x81,
            record_deadline: None,
        };
        let mut got = [0; 64];
        let mut len = 0;
        while len < payload.len() {
            len += reader.read(&mut got[len..]).unwrap();
        }
        assert_eq!(&got[..len], payload);

        let mut writer = AoaWriter {
            io: fake.clone(),
            pipe: 0x02,
            record_deadline: None,
        };
        writer.write_all(payload).unwrap();
        assert_eq!(*fake.output.lock().unwrap(), payload);

        let cancel = AoaCancel(fake.clone());
        cancel.cancel();
        assert_eq!(
            reader.read(&mut got).unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
        let dropped = fake.dropped.clone();
        drop(cancel);
        drop(fake);
        assert!(!dropped.load(std::sync::atomic::Ordering::Acquire));
        drop(reader);
        drop(writer);
        assert!(dropped.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn expired_record_deadline_rejects_data_and_invalid_record_cancels_stream() {
        let fake = Arc::new(FakeIo {
            input: Mutex::new(b"still-buffered".to_vec()),
            output: Mutex::new(Vec::new()),
            max_transfer: 64,
            zero_read: std::sync::atomic::AtomicBool::new(false),
            zero_write: std::sync::atomic::AtomicBool::new(false),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            dropped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let mut reader = AoaReader {
            io: fake.clone(),
            pipe: 0x81,
            record_deadline: Some(Instant::now() - Duration::from_millis(1)),
        };
        assert_eq!(
            reader.read(&mut [0; 4]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(*fake.input.lock().unwrap(), b"still-buffered");

        let invalid = Arc::new(FakeIo {
            input: Mutex::new(vec![0; 8]),
            output: Mutex::new(Vec::new()),
            max_transfer: 64,
            zero_read: std::sync::atomic::AtomicBool::new(false),
            zero_write: std::sync::atomic::AtomicBool::new(false),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            dropped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let mut reader = AoaReader {
            io: invalid.clone(),
            pipe: 0x81,
            record_deadline: None,
        };
        assert_eq!(
            reader.read_anpu_record().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(invalid.is_cancelled());
    }
}
