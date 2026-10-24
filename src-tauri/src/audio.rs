use crate::network::{FloatPcmQueue, PcmQueue};
use crate::protocol::{AudioFormatV2, PcmFormat};
use libloading::Library;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::PathBuf,
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::{mpsc as async_mpsc, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

type ConnectCallback = unsafe extern "C" fn(*const c_char);
type PcmCallback = unsafe extern "C" fn(*const i16, u32, u32, u16, u64, *mut c_void);
type FloatPcmCallback = unsafe extern "C" fn(*const f32, u32, u32, u16, u64, *mut c_void);
const FLOAT_CAPTURE_CAPABILITY: u32 = 1;
const FLOAT_CAPTURE_FORMAT_INTEGER: u32 = 1;
const FLOAT_CAPTURE_FORMAT_FLOAT: u32 = 2;
const FLOAT_CONVERSION_FLAGS: u32 = 7;
static HANDSHAKES: OnceLock<Mutex<Option<async_mpsc::UnboundedSender<String>>>> = OnceLock::new();

#[derive(Clone)]
pub struct AudioActor {
    tx: mpsc::Sender<Request>,
    handshake_rx: std::sync::Arc<AsyncMutex<async_mpsc::UnboundedReceiver<String>>>,
}

enum Request {
    Initialize(mpsc::SyncSender<Result<(), String>>),
    Listen(u16, String, mpsc::SyncSender<Result<(), String>>),
    Start(mpsc::SyncSender<Result<(), String>>),
    StartPcmSink(Arc<PcmQueue>, mpsc::SyncSender<Result<(), String>>),
    PcmFormat(mpsc::SyncSender<Result<PcmFormat, String>>),
    ProbeFloatFormat(mpsc::SyncSender<Result<CaptureFormatV2, String>>),
    StartFloatSink(
        Arc<FloatPcmQueue>,
        AudioFormatV2,
        mpsc::SyncSender<Result<CaptureFormatV2, String>>,
    ),
    StopPcmSink(mpsc::SyncSender<Result<(), String>>),
    Stream(bool, mpsc::SyncSender<Result<(), String>>),
    StopAndCleanup(mpsc::SyncSender<Result<(), String>>),
    Mute(mpsc::SyncSender<Result<(), String>>),
    Restore(Vec<String>, mpsc::SyncSender<Result<(), String>>),
    RestoreMuted(mpsc::SyncSender<Result<(), String>>),
    IsConnected(mpsc::SyncSender<Result<bool, String>>),
    SentPcmBytes(mpsc::SyncSender<Result<u64, String>>),
    TakeLastError(mpsc::SyncSender<Result<Option<String>, String>>),
}

struct NativeFns {
    _library: Library,
    initialize: unsafe extern "C" fn() -> i32,
    listen: unsafe extern "C" fn(i32, Option<ConnectCallback>) -> i32,
    start: unsafe extern "C" fn() -> i32,
    set_pcm_callback: Option<unsafe extern "C" fn(Option<PcmCallback>, *mut c_void) -> i32>,
    get_pcm_format: Option<unsafe extern "C" fn(*mut u32, *mut u16) -> i32>,
    v2: Option<NativeV2Fns>,
    v2_incomplete: bool,
    set_stream: unsafe extern "C" fn(i32),
    stop: unsafe extern "C" fn(),
    cleanup: unsafe extern "C" fn(),
    set_token: unsafe extern "C" fn(*const c_char),
    error_code: unsafe extern "C" fn() -> i32,
    error_message: unsafe extern "C" fn() -> *const c_char,
    clear_error: unsafe extern "C" fn(),
    is_connected: unsafe extern "C" fn(*mut c_char, i32) -> i32,
    sent_pcm_bytes: unsafe extern "C" fn() -> u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CapabilitiesV2 {
    size: u32,
    version: u32,
    flags: u32,
    max_callback_frames: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CaptureFormatV2 {
    pub(crate) size: u32,
    pub(crate) version: u32,
    pub(crate) mix_sample_rate: u32,
    pub(crate) mix_channels: u32,
    pub(crate) mix_sample_format: u32,
    pub(crate) mix_container_bits: u32,
    pub(crate) mix_valid_bits: u32,
    #[allow(dead_code)] // Retained so the cached descriptor preserves the native mix layout.
    pub(crate) mix_channel_mask: u32,
    pub(crate) capture_sample_rate: u32,
    pub(crate) capture_channels: u32,
    pub(crate) capture_sample_format: u32,
    pub(crate) conversion_flags: u32,
}

struct NativeV2Fns {
    get_capabilities: unsafe extern "C" fn(*mut CapabilitiesV2) -> i32,
    get_capture_format: unsafe extern "C" fn(*mut CaptureFormatV2) -> i32,
    initialize_float: unsafe extern "C" fn() -> i32,
    set_float_callback: unsafe extern "C" fn(Option<FloatPcmCallback>, *mut c_void) -> i32,
}

fn assemble_v2_functions(
    get_capabilities: Option<unsafe extern "C" fn(*mut CapabilitiesV2) -> i32>,
    get_capture_format: Option<unsafe extern "C" fn(*mut CaptureFormatV2) -> i32>,
    initialize_float: Option<unsafe extern "C" fn() -> i32>,
    set_float_callback: Option<unsafe extern "C" fn(Option<FloatPcmCallback>, *mut c_void) -> i32>,
) -> (Option<NativeV2Fns>, bool) {
    let symbol_count = [
        get_capabilities.is_some(),
        get_capture_format.is_some(),
        initialize_float.is_some(),
        set_float_callback.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    let functions = match (
        get_capabilities,
        get_capture_format,
        initialize_float,
        set_float_callback,
    ) {
        (
            Some(get_capabilities),
            Some(get_capture_format),
            Some(initialize_float),
            Some(set_float_callback),
        ) => Some(NativeV2Fns {
            get_capabilities,
            get_capture_format,
            initialize_float,
            set_float_callback,
        }),
        _ => None,
    };
    (functions, symbol_count != 0 && symbol_count != 4)
}

impl NativeV2Fns {
    fn validate_capture_format(format: &CaptureFormatV2) -> Result<(), String> {
        if format.size < std::mem::size_of::<CaptureFormatV2>() as u32 || format.version != 2 {
            return Err("Native DLL 返回了不兼容的 Float32 格式结构（size/version）。".into());
        }
        if format.mix_sample_rate == 0
            || format.mix_channels == 0
            || !matches!(
                format.mix_sample_format,
                FLOAT_CAPTURE_FORMAT_INTEGER | FLOAT_CAPTURE_FORMAT_FLOAT
            )
            || format.mix_container_bits == 0
            || (format.mix_valid_bits != 0 && format.mix_valid_bits > format.mix_container_bits)
            || format.capture_sample_rate != format.mix_sample_rate
            || format.capture_channels != 2
            || format.capture_sample_format != FLOAT_CAPTURE_FORMAT_FLOAT
            || format.conversion_flags & !FLOAT_CONVERSION_FLAGS != 0
        {
            return Err("Native DLL 返回了不支持的 WASAPI Float32 采集格式。".into());
        }
        Ok(())
    }
}

pub(crate) fn validate_float_format_for_session(
    format: &CaptureFormatV2,
    expected: AudioFormatV2,
) -> Result<(), String> {
    if AudioFormatV2::for_profile(expected.profile, expected.mode).ok() != Some(expected)
        || !(2..=5).contains(&expected.profile)
        || expected.channels != 2
    {
        return Err(
            "Authenticated session does not describe a supported high quality audio format.".into(),
        );
    }
    if format.mix_sample_rate != expected.sample_rate
        || format.capture_sample_rate != expected.sample_rate
        || format.capture_channels != u32::from(expected.channels)
        || format.capture_sample_format != FLOAT_CAPTURE_FORMAT_FLOAT
    {
        let rate_mismatch = if matches!(expected.profile, 3 | 5)
            && expected.sample_rate == 96_000
            && format.mix_sample_rate != 96_000
        {
            format!(
                "FORMAT_RATE_MISMATCH: actualHz={}; ",
                format.mix_sample_rate
            )
        } else {
            String::new()
        };
        return Err(format!("{rate_mismatch}Windows mix/capture format does not match the authenticated session (expected {} Hz stereo Float32 capture; actual mix {} Hz/{} ch/format {}/{}, capture {} Hz/{} ch/format {}).", expected.sample_rate, format.mix_sample_rate, format.mix_channels, format.mix_sample_format, format.mix_container_bits, format.capture_sample_rate, format.capture_channels, format.capture_sample_format));
    }
    Ok(())
}

impl NativeFns {
    unsafe fn load(path: &PathBuf) -> Result<Self, String> {
        let library =
            Library::new(path).map_err(|_| format!("无法加载音频采集组件：{}", path.display()))?;
        macro_rules! sym {
            ($name:literal, $ty:ty) => {
                *library
                    .get::<$ty>(concat!($name, "\0").as_bytes())
                    .map_err(|_| format!("音频采集组件缺少函数 {}", $name))?
            };
        }
        let set_pcm_callback = library
            .get::<unsafe extern "C" fn(Option<PcmCallback>, *mut c_void) -> i32>(
                b"AudioCapture_SetPcmCallback\0",
            )
            .ok()
            .map(|symbol| *symbol);
        let get_pcm_format = library
            .get::<unsafe extern "C" fn(*mut u32, *mut u16) -> i32>(b"AudioCapture_GetPcmFormat\0")
            .ok()
            .map(|symbol| *symbol);
        let get_capabilities_v2 = library
            .get::<unsafe extern "C" fn(*mut CapabilitiesV2) -> i32>(
                b"AudioCapture_GetCapabilitiesV2\0",
            )
            .ok()
            .map(|symbol| *symbol);
        let get_capture_format_v2 = library
            .get::<unsafe extern "C" fn(*mut CaptureFormatV2) -> i32>(
                b"AudioCapture_GetCaptureFormatV2\0",
            )
            .ok()
            .map(|symbol| *symbol);
        let initialize_float_v2 = library
            .get::<unsafe extern "C" fn() -> i32>(b"AudioCapture_InitializeFloatV2\0")
            .ok()
            .map(|symbol| *symbol);
        let set_float_callback_v2 = library
            .get::<unsafe extern "C" fn(Option<FloatPcmCallback>, *mut c_void) -> i32>(
                b"AudioCapture_SetFloatPcmCallbackV2\0",
            )
            .ok()
            .map(|symbol| *symbol);
        let (v2, v2_incomplete) = assemble_v2_functions(
            get_capabilities_v2,
            get_capture_format_v2,
            initialize_float_v2,
            set_float_callback_v2,
        );
        Ok(Self {
            initialize: sym!("AudioCapture_Initialize", unsafe extern "C" fn() -> i32),
            listen: sym!(
                "AudioCapture_Listen",
                unsafe extern "C" fn(i32, Option<ConnectCallback>) -> i32
            ),
            start: sym!("AudioCapture_Start", unsafe extern "C" fn() -> i32),
            set_pcm_callback,
            get_pcm_format,
            v2,
            v2_incomplete,
            set_stream: sym!(
                "AudioCapture_SetStreamingEnabled",
                unsafe extern "C" fn(i32)
            ),
            stop: sym!("AudioCapture_Stop", unsafe extern "C" fn()),
            cleanup: sym!("AudioCapture_Cleanup", unsafe extern "C" fn()),
            set_token: sym!(
                "AudioCapture_SetConnectToken",
                unsafe extern "C" fn(*const c_char)
            ),
            error_code: sym!(
                "AudioCapture_GetLastErrorCode",
                unsafe extern "C" fn() -> i32
            ),
            error_message: sym!(
                "AudioCapture_GetLastErrorMessage",
                unsafe extern "C" fn() -> *const c_char
            ),
            clear_error: sym!("AudioCapture_ClearLastError", unsafe extern "C" fn()),
            is_connected: sym!(
                "AudioCapture_IsConnected",
                unsafe extern "C" fn(*mut c_char, i32) -> i32
            ),
            sent_pcm_bytes: sym!(
                "AudioCapture_GetSentPcmBytes",
                unsafe extern "C" fn() -> u64
            ),
            _library: library,
        })
    }

    fn describe_error(&self, fallback: &str) -> String {
        let code = unsafe { (self.error_code)() };
        let message = unsafe {
            let ptr = (self.error_message)();
            if ptr.is_null() {
                String::new()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        };
        if message.is_empty() {
            format!("{fallback} (错误 {code})")
        } else {
            message
        }
    }

    fn pcm_format(&self) -> Result<PcmFormat, String> {
        let get_format = self.get_pcm_format.ok_or_else(|| {
            "当前音频采集组件不支持读取已协商的 PCM 格式；请更新 Native DLL。".to_string()
        })?;
        let mut sample_rate = 0;
        let mut channels = 0;
        if unsafe { get_format(&mut sample_rate, &mut channels) } == 0 {
            return Err(self.describe_error("无法读取 WASAPI 已协商的 PCM 格式"));
        }
        Ok(PcmFormat {
            sample_rate,
            channels,
        })
    }

    fn require_v2(&self) -> Result<&NativeV2Fns, String> {
        self.v2
            .as_ref()
            .ok_or_else(|| missing_v2_error(self.v2_incomplete))
    }

    fn float_capabilities(&self) -> Result<CapabilitiesV2, String> {
        let v2 = self.require_v2()?;
        let mut capabilities = CapabilitiesV2 {
            size: std::mem::size_of::<CapabilitiesV2>() as u32,
            version: 2,
            ..CapabilitiesV2::default()
        };
        if unsafe { (v2.get_capabilities)(&mut capabilities) } == 0 {
            return Err(self.describe_error("无法读取 Native Float32 V2 能力"));
        }
        if capabilities.size < std::mem::size_of::<CapabilitiesV2>() as u32
            || capabilities.version != 2
        {
            return Err("Native DLL 返回了不兼容的 Float32 能力结构（size/version）。".into());
        }
        if capabilities.flags != FLOAT_CAPTURE_CAPABILITY {
            return Err("Native DLL 未报告受支持的 Float32 采集能力标志。".into());
        }
        if capabilities.max_callback_frames == 0 || capabilities.max_callback_frames > 4096 {
            return Err("Native DLL 返回了无效的 Float32 最大回调帧数。".into());
        }
        Ok(capabilities)
    }

    fn float_capture_format(&self) -> Result<CaptureFormatV2, String> {
        let v2 = self.require_v2()?;
        let mut format = CaptureFormatV2 {
            size: std::mem::size_of::<CaptureFormatV2>() as u32,
            version: 2,
            ..CaptureFormatV2::default()
        };
        if unsafe { (v2.get_capture_format)(&mut format) } == 0 {
            return Err(self.describe_error("无法读取 WASAPI Float32 采集格式"));
        }
        NativeV2Fns::validate_capture_format(&format)?;
        Ok(format)
    }
}

fn missing_v2_error(incomplete: bool) -> String {
    if incomplete {
        "音频采集组件的 V2 Float32 接口不完整，请更新 Native DLL。".to_string()
    } else {
        "当前音频采集组件不支持 Float32 V2；PCM16 仍可使用，请更新 Native DLL。".to_string()
    }
}

impl AudioActor {
    pub fn new(dll_path: PathBuf, marker_path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let (handshake_tx, handshake_rx) = async_mpsc::unbounded_channel();
        *HANDSHAKES
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("handshake callback lock") = Some(handshake_tx);
        std::thread::Builder::new().name("AnPod audio control".into()).spawn(move || {
            let mut native = None;
            let mut pcm_sink: Option<Arc<PcmQueue>> = None;
            let mut float_sink: Option<Arc<FloatPcmQueue>> = None;
            let mut float_capture_format = None;
            let mut muted_ids = Vec::<String>::new();
            while let Ok(request) = rx.recv() {
                match request {
                    Request::Initialize(reply) => {
                        let result = (|| {
                            if native.is_none() { native = Some(unsafe { NativeFns::load(&dll_path)? }); }
                            let native = native.as_ref().expect("loaded audio capture");
                            if unsafe { (native.initialize)() } == 0 { Err(native.describe_error("无法初始化系统音频采集")) } else { Ok(()) }
                        })();
                        let _ = reply.send(result);
                    }
                    Request::Listen(port, token, reply) => {
                        let result = if let Some(native) = native.as_ref() {
                            match CString::new(token) {
                                Ok(token) => {
                                    unsafe { (native.set_token)(token.as_ptr()); }
                                    if unsafe { (native.listen)(port as i32, Some(on_native_connect)) } == 0 { Err(native.describe_error("无法开启本机音频监听")) } else { Ok(()) }
                                }
                                Err(_) => Err("连接令牌格式无效。".into()),
                            }
                        } else { Err("音频采集组件尚未初始化。".into()) };
                        let _ = reply.send(result);
                    }
                    Request::Start(reply) => {
                        let result = match native.as_ref() {
                            Some(native) if unsafe { (native.start)() } != 0 => Ok(()),
                            Some(native) => Err(native.describe_error("无法开始采集系统声音")),
                            None => Err("音频采集组件尚未初始化。".into()),
                        };
                        let _ = reply.send(result);
                    }
                    Request::StartPcmSink(queue, reply) => {
                        let result = if let Some(native) = native.as_ref() {
                            let Some(set_pcm_callback) = native.set_pcm_callback else {
                                let _ = reply.send(Err("音频采集组件不支持 PCM sink。".into()));
                                continue;
                            };
                            if pcm_sink.as_ref().is_some_and(|old| Arc::ptr_eq(old, &queue))
                                && float_sink.is_none()
                            {
                                Ok(())
                            } else {
                                let expected_format = queue.expected_format();
                                cleanup_capture_contexts(native, &mut pcm_sink, &mut float_sink)
                                    .and_then(|()| {
                                        float_capture_format = None;
                                        if unsafe { (native.initialize)() } == 0 {
                                            return Err(native.describe_error("无法初始化 PCM sink 采集"));
                                        }
                                        let format = native.pcm_format()?;
                                        if expected_format != Some(format) {
                                            return Err(format!(
                                                "WASAPI PCM 格式不匹配；认证会话需要 {:?}，设备输出为 {:?}，电脑声音保持未静音。",
                                                expected_format, format));
                                        }
                                        let user_data = Arc::as_ptr(&queue) as *mut c_void;
                                        pcm_sink = Some(queue.clone());
                                        if unsafe { set_pcm_callback(Some(on_native_pcm), user_data) } == 0 {
                                            return Err(native.describe_error("无法设置 PCM sink"));
                                        }
                                        unsafe { (native.set_stream)(1); }
                                        if unsafe { (native.start)() } == 0 {
                                            return Err(native.describe_error("无法开始 PCM sink 采集"));
                                        }
                                        Ok(())
                                    })
                                    .inspect_err(|_| {
                                        let _ = cleanup_capture_contexts(native, &mut pcm_sink, &mut float_sink);
                                    })
                            }
                        } else { Err("音频采集组件尚未初始化。".into()) };
                        let _ = reply.send(result);
                    }
                    Request::PcmFormat(reply) => {
                        let result = native.as_ref().ok_or_else(|| "音频采集组件尚未初始化。".to_string())
                            .and_then(NativeFns::pcm_format);
                        let _ = reply.send(result);
                    }
                    Request::StartFloatSink(queue, expected_format, reply) => {
                        let result = if let Some(native) = native.as_ref() {
                            match native.float_capabilities() {
                                Err(error) => Err(error),
                                Ok(_capabilities) if can_reuse_float_sink(float_sink.as_ref(), &queue) => {
                                    float_capture_format
                                        .filter(|format| validate_float_format_for_session(format, expected_format).is_ok())
                                        .ok_or_else(|| format!("Cached Float32 sink does not match the authenticated {} Hz session.", expected_format.sample_rate))
                                }
                                Ok(capabilities) => {
                                    cleanup_capture_contexts(native, &mut pcm_sink, &mut float_sink)
                                        .and_then(|()| {
                                            float_capture_format = None;
                                            let v2 = native.require_v2()?;
                                            if unsafe { (v2.initialize_float)() } == 0 {
                                                return Err(native.describe_error("无法初始化 WASAPI Float32 采集"));
                                            }
                                            let format = native.float_capture_format()?;
                                            validate_float_format_for_session(&format, expected_format)?;
                                            queue.set_expected_format(
                                                format.capture_sample_rate,
                                                capabilities.max_callback_frames,
                                            )?;
                                            if queue.expected_format() != Some((format.capture_sample_rate, 2)) {
                                                return Err("Float32 PCM 队列格式与 WASAPI 回调格式不匹配。".into());
                                            }
                                            let user_data = Arc::as_ptr(&queue) as *mut c_void;
                                            float_sink = Some(queue.clone());
                                            if unsafe { (v2.set_float_callback)(Some(on_native_float_pcm), user_data) } == 0 {
                                                return Err(native.describe_error("无法设置 Float32 PCM sink"));
                                            }
                                            float_capture_format = Some(format);
                                            unsafe { (native.set_stream)(1); }
                                            if unsafe { (native.start)() } == 0 {
                                                return Err(native.describe_error("无法开始 Float32 PCM 采集"));
                                            }
                                            Ok(format)
                                        })
                                        .inspect_err(|_| {
                                            let _ = cleanup_capture_contexts(native, &mut pcm_sink, &mut float_sink);
                                            float_capture_format = None;
                                        })
                                }
                            }
                        } else { Err("音频采集组件尚未初始化。".into()) };
                        let _ = reply.send(result);
                    }
                    Request::ProbeFloatFormat(reply) => {
                        let result = (|| {
                            if native.is_none() {
                                native = Some(unsafe { NativeFns::load(&dll_path)? });
                            }
                            let native = native.as_ref().expect("loaded audio capture");
                            cleanup_capture_contexts(native, &mut pcm_sink, &mut float_sink)
                                .and_then(|()| {
                                    let probe = (|| {
                                        let v2 = native.require_v2()?;
                                        if unsafe { (v2.initialize_float)() } == 0 {
                                            return Err(native.describe_error("无法查询 WASAPI Float32 格式"));
                                        }
                                        let format = native.float_capture_format()?;
                                        if format.capture_sample_rate != format.mix_sample_rate
                                            || format.capture_channels != 2
                                        {
                                            return Err("Native DLL 的 Float32 格式与 mix 格式不一致。".into());
                                        }
                                        Ok(format)
                                    })();
                                    let cleanup = cleanup_capture_contexts(native, &mut pcm_sink, &mut float_sink);
                                    float_capture_format = None;
                                    match (probe, cleanup) {
                                        (Ok(format), Ok(())) => Ok(format),
                                        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
                                    }
                                })
                        })();
                        let _ = reply.send(result);
                    }
                    Request::StopPcmSink(reply) => {
                        let result = if let Some(native) = native.as_ref() {
                            let result = cleanup_capture_contexts(native, &mut pcm_sink, &mut float_sink);
                            float_capture_format = None;
                            result
                        } else { Ok(()) };
                        let _ = reply.send(result);
                    }
                    Request::Stream(enabled, reply) => {
                        let result = native.as_ref().map(|native| unsafe { (native.set_stream)(i32::from(enabled)); Ok(()) }).unwrap_or_else(|| Err("音频会话尚未初始化。".into()));
                        let _ = reply.send(result);
                    }
                    Request::StopAndCleanup(reply) => {
                        let result = if let Some(native) = native.as_ref() {
                            let result = cleanup_capture_contexts(native, &mut pcm_sink, &mut float_sink);
                            float_capture_format = None;
                            result
                        } else { Ok(()) };
                        let _ = reply.send(result);
                    }
                    Request::Mute(reply) => {
                        let result = mute_active_endpoints(&marker_path, &mut muted_ids);
                        let _ = reply.send(result);
                    }
                    Request::Restore(ids, reply) => {
                        muted_ids = ids;
                        let result = restore_endpoints(&marker_path, &mut muted_ids);
                        let _ = reply.send(result);
                    }
                    Request::RestoreMuted(reply) => {
                        let result = restore_endpoints(&marker_path, &mut muted_ids);
                        let _ = reply.send(result);
                    }
                    Request::IsConnected(reply) => {
                        let result = native.as_ref().map(|native| {
                            let mut buffer = [0 as c_char; 256];
                            unsafe { (native.is_connected)(buffer.as_mut_ptr(), buffer.len() as i32) != 0 }
                        }).ok_or_else(|| "音频采集组件尚未初始化。".to_string());
                        let _ = reply.send(result);
                    }
                    Request::SentPcmBytes(reply) => {
                        let result = native.as_ref().map(|native| unsafe { (native.sent_pcm_bytes)() })
                            .ok_or_else(|| "音频采集组件尚未初始化。".to_string());
                        let _ = reply.send(result);
                    }
                    Request::TakeLastError(reply) => {
                        let result = native.as_ref().map(|native| unsafe {
                            let code = (native.error_code)();
                            let pointer = (native.error_message)();
                            let message = if pointer.is_null() { String::new() } else { CStr::from_ptr(pointer).to_string_lossy().into_owned() };
                            (native.clear_error)();
                            if code == 0 && message.is_empty() { None }
                            else if message.is_empty() { Some(format!("音频采集错误 {code}")) }
                            else { Some(message) }
                        }).ok_or_else(|| "音频采集组件尚未初始化。".to_string());
                        let _ = reply.send(result);
                    }
                }
            }
            if let Some(native) = native {
                let _ = cleanup_capture_contexts(&native, &mut pcm_sink, &mut float_sink);
            }
            drop(pcm_sink.take());
            drop(float_sink.take());
            let _ = restore_endpoints(&marker_path, &mut muted_ids);
        }).expect("create audio worker thread");
        Self {
            tx,
            handshake_rx: std::sync::Arc::new(AsyncMutex::new(handshake_rx)),
        }
    }

    pub fn initialize(&self) -> Result<(), String> {
        self.call(Request::Initialize)
    }
    pub fn listen(&self, port: u16, token: &str) -> Result<(), String> {
        self.call(|reply| Request::Listen(port, token.into(), reply))
    }
    pub fn start(&self) -> Result<(), String> {
        self.call(Request::Start)
    }
    pub fn start_pcm_sink(&self, queue: Arc<PcmQueue>) -> Result<(), String> {
        self.call(|reply| Request::StartPcmSink(queue, reply))
    }
    pub fn pcm_format(&self) -> Result<PcmFormat, String> {
        self.call(Request::PcmFormat)
    }
    pub(crate) fn probe_float_format(&self) -> Result<CaptureFormatV2, String> {
        self.call(Request::ProbeFloatFormat)
    }
    pub(crate) fn start_float_sink(
        &self,
        queue: Arc<FloatPcmQueue>,
        expected_format: AudioFormatV2,
    ) -> Result<CaptureFormatV2, String> {
        self.call(|reply| Request::StartFloatSink(queue, expected_format, reply))
    }
    pub fn stop_pcm_sink(&self) -> Result<(), String> {
        self.call(Request::StopPcmSink)
    }
    pub fn set_streaming(&self, enabled: bool) -> Result<(), String> {
        self.call(|reply| Request::Stream(enabled, reply))
    }
    pub fn stop_and_cleanup(&self) -> Result<(), String> {
        self.call(Request::StopAndCleanup)
    }
    pub fn mute_active(&self) -> Result<(), String> {
        self.call(Request::Mute)
    }
    pub fn restore_ids(&self, ids: Vec<String>) -> Result<(), String> {
        self.call(|reply| Request::Restore(ids, reply))
    }
    pub fn restore_muted(&self) -> Result<(), String> {
        self.call(Request::RestoreMuted)
    }
    pub fn is_connected(&self) -> Result<bool, String> {
        self.call(Request::IsConnected)
    }
    pub fn sent_pcm_bytes(&self) -> Result<u64, String> {
        self.call(Request::SentPcmBytes)
    }
    pub fn take_last_error(&self) -> Result<Option<String>, String> {
        self.call(Request::TakeLastError)
    }

    pub async fn wait_handshake(
        &self,
        expected: &str,
        limit: Duration,
        cancel: &CancellationToken,
    ) -> Result<String, String> {
        let mut receiver = self.handshake_rx.lock().await;
        receive_handshake(&mut receiver, expected, limit, cancel).await
    }

    fn call<T>(
        &self,
        make: impl FnOnce(mpsc::SyncSender<Result<T, String>>) -> Request,
    ) -> Result<T, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx
            .send(make(tx))
            .map_err(|_| "音频控制线程已停止。".to_string())?;
        rx.recv()
            .map_err(|_| "音频控制线程未返回结果。".to_string())?
    }
}

async fn receive_handshake(
    receiver: &mut async_mpsc::UnboundedReceiver<String>,
    expected: &str,
    limit: Duration,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Err("连接已取消。".into()),
            result = tokio::time::timeout_at(deadline, receiver.recv()) => match result {
                Ok(Some(code)) if code == expected => return Ok(code),
                Ok(Some(_stale_code)) => continue,
                Ok(None) => return Err("音频连接监听已停止。".into()),
                Err(_) => return Err("等待手机完成握手超时。".into()),
            }
        }
    }
}

unsafe extern "C" fn on_native_connect(code: *const c_char) {
    if code.is_null() {
        return;
    }
    let text = CStr::from_ptr(code).to_string_lossy();
    if text.len() > 128 {
        return;
    }
    if let Some(tx) = HANDSHAKES
        .get()
        .and_then(|value| value.lock().ok()?.clone())
    {
        let _ = tx.send(text.into_owned());
    }
}

pub(crate) unsafe extern "C" fn on_native_pcm(
    samples: *const i16,
    frame_count: u32,
    sample_rate: u32,
    channels: u16,
    frame_index: u64,
    user_data: *mut c_void,
) {
    if user_data.is_null() {
        return;
    }
    let queue = &*user_data.cast::<PcmQueue>();
    queue.push_raw(samples, frame_count, sample_rate, channels, frame_index);
}

pub(crate) unsafe extern "C" fn on_native_float_pcm(
    samples: *const f32,
    frame_count: u32,
    sample_rate: u32,
    channels: u16,
    frame_index: u64,
    user_data: *mut c_void,
) {
    if user_data.is_null() {
        return;
    }
    let queue = &*user_data.cast::<FloatPcmQueue>();
    queue.push_raw(samples, frame_count, sample_rate, channels, frame_index);
}

fn can_reuse_float_sink(current: Option<&Arc<FloatPcmQueue>>, queue: &Arc<FloatPcmQueue>) -> bool {
    !queue.is_stopped() && current.is_some_and(|old| Arc::ptr_eq(old, queue))
}

fn cleanup_capture_contexts(
    native: &NativeFns,
    pcm_sink: &mut Option<Arc<PcmQueue>>,
    float_sink: &mut Option<Arc<FloatPcmQueue>>,
) -> Result<(), String> {
    unsafe {
        (native.stop)();
        // Cleanup joins the native capture thread, which closes the callback's
        // access to userData before either Arc can be released.
        (native.cleanup)();
    }

    let mut error = None;
    if pcm_sink.is_some() {
        match native.set_pcm_callback {
            Some(set_callback) if unsafe { set_callback(None, std::ptr::null_mut()) } != 0 => {
                drop(pcm_sink.take());
            }
            Some(_) => error = Some(native.describe_error("无法安全注销 PCM sink")),
            None => error = Some("音频采集组件不支持 PCM sink。".into()),
        }
    }
    if float_sink.is_some() {
        match native.require_v2() {
            Ok(v2) if unsafe { (v2.set_float_callback)(None, std::ptr::null_mut()) } != 0 => {
                if let Some(queue) = float_sink.take() {
                    queue.stop_and_clear();
                }
            }
            Ok(_) => {
                if error.is_none() {
                    error = Some(native.describe_error("无法安全注销 Float32 PCM sink"));
                }
            }
            Err(v2_error) => {
                if error.is_none() {
                    error = Some(v2_error);
                }
            }
        }
    }
    error.map_or(Ok(()), Err)
}

// Staged quantizer for the future worker-side wire encoder; callback path stays Float32.
#[allow(dead_code)]
pub(crate) fn float_sample_to_pcm16(sample: f32) -> Result<i16, &'static str> {
    if !sample.is_finite() {
        return Err("Float32 PCM contains a non-finite sample.");
    }
    let quantized = (sample.clamp(-1.0, 1.0) * 32768.0).round();
    Ok((quantized as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16)
}

#[cfg(windows)]
fn mute_active_endpoints(marker_path: &PathBuf, muted_ids: &mut Vec<String>) -> Result<(), String> {
    use windows::Win32::{
        Media::Audio::Endpoints::IAudioEndpointVolume,
        Media::Audio::{eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE},
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
        },
    };
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED);
        if initialized.is_err() {
            return Err(format!("无法初始化音频设备控制：{initialized:?}"));
        }
        let result = (|| -> Result<(), String> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| e.to_string())?;
            let devices = enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .map_err(|e| e.to_string())?;
            for index in 0..devices.GetCount().map_err(|e| e.to_string())? {
                let device = devices.Item(index).map_err(|e| e.to_string())?;
                let id = device
                    .GetId()
                    .map_err(|e| e.to_string())?
                    .to_string()
                    .map_err(|e| e.to_string())?;
                let volume: IAudioEndpointVolume = device
                    .Activate(CLSCTX_ALL, None)
                    .map_err(|e| e.to_string())?;
                if !volume.GetMute().map_err(|e| e.to_string())?.as_bool() {
                    remember_before_muting(marker_path, muted_ids, &id)?;
                    volume
                        .SetMute(true, std::ptr::null())
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        })();
        CoUninitialize();
        result
    }
}

#[cfg(not(windows))]
fn mute_active_endpoints(_: &PathBuf, _: &mut Vec<String>) -> Result<(), String> {
    Err("AnPod 音频端点控制仅支持 Windows。".into())
}

#[cfg(windows)]
fn restore_endpoints(marker_path: &PathBuf, muted_ids: &mut Vec<String>) -> Result<(), String> {
    use windows::Win32::{
        Media::Audio::Endpoints::IAudioEndpointVolume,
        Media::Audio::{eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE},
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
        },
    };
    if muted_ids.is_empty() {
        return Ok(());
    }
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED);
        if initialized.is_err() {
            return Err(format!("无法初始化音频设备控制：{initialized:?}"));
        }
        let result = (|| -> Result<(), String> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| e.to_string())?;
            let devices = enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .map_err(|e| e.to_string())?;
            let wanted = muted_ids.clone();
            let mut seen = std::collections::HashSet::new();
            let mut first_error = None;
            for index in 0..devices.GetCount().map_err(|e| e.to_string())? {
                let device = devices.Item(index).map_err(|e| e.to_string())?;
                let id = device
                    .GetId()
                    .map_err(|e| e.to_string())?
                    .to_string()
                    .map_err(|e| e.to_string())?;
                if !wanted.contains(&id) {
                    continue;
                }
                seen.insert(id.clone());
                let restored = device
                    .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                    .map_err(|e| e.to_string())
                    .and_then(|volume| {
                        volume
                            .SetMute(false, std::ptr::null())
                            .map_err(|e| e.to_string())
                    });
                match restored {
                    Ok(()) => {
                        if let Err(error) = forget_restored(marker_path, muted_ids, &id) {
                            first_error.get_or_insert(error);
                        }
                    }
                    Err(error) => {
                        first_error
                            .get_or_insert_with(|| format!("无法恢复音频设备 {id}：{error}"));
                    }
                }
            }
            if let Some(id) = wanted.iter().find(|id| !seen.contains(*id)) {
                first_error
                    .get_or_insert_with(|| format!("音频设备 {id} 当前不可用；恢复记录已保留。"));
            }
            first_error.map_or(Ok(()), Err)
        })();
        CoUninitialize();
        result
    }
}

#[cfg(not(windows))]
fn restore_endpoints(marker_path: &PathBuf, muted_ids: &mut Vec<String>) -> Result<(), String> {
    muted_ids.clear();
    let _ = std::fs::remove_file(marker_path);
    Ok(())
}

fn remember_before_muting(path: &PathBuf, ids: &mut Vec<String>, id: &str) -> Result<(), String> {
    if ids.iter().any(|existing| existing == id) {
        return Ok(());
    }
    ids.push(id.to_owned());
    if let Err(error) = write_marker(path, ids) {
        ids.pop();
        return Err(error);
    }
    Ok(())
}

fn forget_restored(path: &PathBuf, ids: &mut Vec<String>, id: &str) -> Result<(), String> {
    let Some(index) = ids.iter().position(|existing| existing == id) else {
        return Ok(());
    };
    let restored = ids.remove(index);
    if let Err(error) = write_marker(path, ids) {
        ids.insert(index, restored);
        return Err(error);
    }
    Ok(())
}

fn write_marker(path: &PathBuf, ids: &[String]) -> Result<(), String> {
    if ids.is_empty() {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("txt.tmp");
    std::fs::write(&temp, ids.join("\r\n")).map_err(|e| e.to_string())?;
    crate::state::atomic_replace(&temp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
