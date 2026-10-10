using System.IO;
using System.Runtime.InteropServices;
using AnPod.Core.Logging;

namespace AnPod.Desktop.Services;

/// <summary>A diagnostic value reported by the native capture module.</summary>
public sealed record NativeAudioError(int Code, string Message);

/// <summary>
/// C# side of the native WASAPI capture DLL (plan §3.1/§5.4).
/// <para>
/// The six original exports are bound strictly as declared upstream. Four
/// additive diagnostics exports are bound opportunistically so a DLL built
/// before they existed still loads (see upstream/PATCHES.md).
/// </para>
/// <para>
/// <c>AudioCapture_SetStreamingEnabled</c> is additive too, but is bound as
/// <em>required</em> on purpose: it is what lets the audio switch mute the
/// stream without dropping the session. An old DLL would silently ignore the
/// call and keep playing, which is worse than refusing to load and saying why.
/// </para>
/// </summary>
public sealed class NativeAudioCapture : IDisposable
{
    private const string LibraryFileName = "audio_capture.dll";

    // Windows x64 has a single calling convention; declaring it explicitly keeps
    // the contract readable and portable to a future x86 build (plan §3.1).
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void ConnectCallbackDelegate(IntPtr connectCode);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int InitializeDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int ListenDelegate(int port, ConnectCallbackDelegate callback);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int StartDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void StopDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void SetStreamingEnabledDelegate(int enabled);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void CleanupDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int IsConnectedDelegate(IntPtr outBuffer, int bufferSize);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void SetTokenDelegate(IntPtr token);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int GetErrorCodeDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate IntPtr GetErrorMessageDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void ClearErrorDelegate();

    private readonly ILog _log;
    private readonly object _gate = new();

    private IntPtr _library;
    private InitializeDelegate? _initialize;
    private ListenDelegate? _listen;
    private StartDelegate? _start;
    private StopDelegate? _stop;
    private SetStreamingEnabledDelegate? _setStreamingEnabled;
    private CleanupDelegate? _cleanup;
    private IsConnectedDelegate? _isConnected;
    private SetTokenDelegate? _setToken;
    private GetErrorCodeDelegate? _getErrorCode;
    private GetErrorMessageDelegate? _getErrorMessage;
    private ClearErrorDelegate? _clearError;

    /// <summary>
    /// Must stay rooted: native code holds this function pointer for the whole
    /// listen window. Collecting it would crash on the native callback thread
    /// (plan §3.1/§5.4).
    /// </summary>
    private ConnectCallbackDelegate? _connectCallback;

    private bool _initialized;
    private bool _disposed;

    public NativeAudioCapture(ILog log)
    {
        _log = log;
        LibraryPath = Path.Combine(AppContext.BaseDirectory, LibraryFileName);
        TryLoadLibrary();
    }

    /// <summary>Absolute path the loader attempted.</summary>
    public string LibraryPath { get; }

    /// <summary>True when the DLL loaded and every mandatory export resolved.</summary>
    public bool IsAvailable { get; private set; }

    /// <summary>Reason the DLL is unusable, when <see cref="IsAvailable"/> is false.</summary>
    public string? LoadError { get; private set; }

    /// <summary>Raised on a native thread when the Android side completes its handshake.</summary>
    public event EventHandler<string>? AndroidConnected;

    private void TryLoadLibrary()
    {
        if (!File.Exists(LibraryPath))
        {
            LoadError = $"找不到音频采集组件 {LibraryFileName}，请确认它位于程序目录下。";
            return;
        }

        if (!NativeLibrary.TryLoad(LibraryPath, out _library))
        {
            LoadError = $"无法加载 {LibraryFileName}（位数不匹配或缺少依赖）。";
            return;
        }

        try
        {
            _initialize = GetRequiredExport<InitializeDelegate>("AudioCapture_Initialize");
            _listen = GetRequiredExport<ListenDelegate>("AudioCapture_Listen");
            _start = GetRequiredExport<StartDelegate>("AudioCapture_Start");
            _stop = GetRequiredExport<StopDelegate>("AudioCapture_Stop");
            _cleanup = GetRequiredExport<CleanupDelegate>("AudioCapture_Cleanup");
            _isConnected = GetRequiredExport<IsConnectedDelegate>("AudioCapture_IsConnected");
            _setStreamingEnabled =
                GetRequiredExport<SetStreamingEnabledDelegate>("AudioCapture_SetStreamingEnabled");
        }
        catch (EntryPointNotFoundException ex)
        {
            LoadError = $"{LibraryFileName} 缺少必需的导出函数：{ex.Message}";
            return;
        }

        // Additive diagnostics: absent in the unpatched upstream DLL.
        _setToken = TryGetExport<SetTokenDelegate>("AudioCapture_SetConnectToken");
        _getErrorCode = TryGetExport<GetErrorCodeDelegate>("AudioCapture_GetLastErrorCode");
        _getErrorMessage = TryGetExport<GetErrorMessageDelegate>("AudioCapture_GetLastErrorMessage");
        _clearError = TryGetExport<ClearErrorDelegate>("AudioCapture_ClearLastError");

        IsAvailable = true;
    }

    private T GetRequiredExport<T>(string name) where T : Delegate =>
        Marshal.GetDelegateForFunctionPointer<T>(NativeLibrary.GetExport(_library, name));

    private T? TryGetExport<T>(string name) where T : Delegate
    {
        if (NativeLibrary.TryGetExport(_library, name, out var address) && address != IntPtr.Zero)
        {
            return Marshal.GetDelegateForFunctionPointer<T>(address);
        }

        return null;
    }

    /// <summary>
    /// Initialises WASAPI loopback on the current default render endpoint.
    /// Must be called before <see cref="Listen"/>.
    /// </summary>
    public bool Initialize()
    {
        lock (_gate)
        {
            if (!IsAvailable || _initialize is null)
            {
                return false;
            }

            if (_initialized)
            {
                return true;
            }

            _initialized = _initialize() != 0;
            if (!_initialized)
            {
                _log.Warn($"AudioCapture_Initialize failed: {DescribeLastError()}");
            }

            return _initialized;
        }
    }

    /// <summary>
    /// Starts the loopback TCP listener on the given port and registers the
    /// handshake callback. Returns false when the port could not be bound.
    /// </summary>
    public bool Listen(int port, string expectedConnectToken)
    {
        lock (_gate)
        {
            if (!_initialized || _listen is null)
            {
                return false;
            }

            SetConnectToken(expectedConnectToken);

            // A fresh delegate per listen; the previous one is released only after
            // the native side has been told to stop (AudioCapture_Stop/Cleanup).
            _connectCallback = OnNativeConnect;

            bool ok = _listen(port, _connectCallback) != 0;
            if (!ok)
            {
                _log.Warn($"AudioCapture_Listen({port}) failed: {DescribeLastError()}");
            }

            return ok;
        }
    }

    private void SetConnectToken(string token)
    {
        if (_setToken is null)
        {
            // Unpatched DLL: the token is still enforced on the managed side.
            return;
        }

        IntPtr ptr = Marshal.StringToCoTaskMemUTF8(token);
        try
        {
            _setToken(ptr);
        }
        finally
        {
            Marshal.FreeCoTaskMem(ptr);
        }
    }

    /// <summary>
    /// Native → managed handshake entry point. Runs on a native thread, so it
    /// copies the pointer contents immediately and never blocks (plan §5.4).
    /// </summary>
    private void OnNativeConnect(IntPtr connectCode)
    {
        string code;
        try
        {
            code = connectCode == IntPtr.Zero ? string.Empty : Marshal.PtrToStringUTF8(connectCode) ?? string.Empty;
        }
        catch (Exception ex)
        {
            _log.Warn($"Could not read connect code: {ex.Message}");
            return;
        }

        // Bound the value before it is used as a comparison key (plan §3.4.4).
        if (code.Length > 128)
        {
            code = code[..128];
        }

        AndroidConnected?.Invoke(this, code);
    }

    /// <summary>Begins streaming captured PCM to the connected Android client.</summary>
    public bool Start()
    {
        lock (_gate)
        {
            if (!_initialized || _start is null)
            {
                return false;
            }

            bool ok = _start() != 0;
            if (!ok)
            {
                _log.Warn($"AudioCapture_Start failed: {DescribeLastError()}");
            }

            return ok;
        }
    }

    /// <summary>
    /// Pauses or resumes forwarding without touching the socket. While paused
    /// the native side feeds the peer silence, so the Android AudioTrack and the
    /// handshake survive and resuming is instant. Unlike <see cref="Stop"/> this
    /// never drops the connection.
    /// </summary>
    public void SetStreamingEnabled(bool enabled)
    {
        lock (_gate)
        {
            _setStreamingEnabled?.Invoke(enabled ? 1 : 0);
        }
    }

    /// <summary>Signals the capture and listen threads to stop; does not block.</summary>
    public void Stop()
    {
        lock (_gate)
        {
            if (!IsAvailable || _stop is null)
            {
                return;
            }

            _stop();
        }
    }

    /// <summary>
    /// Releases native resources (threads, sockets, COM objects). Safe to call
    /// repeatedly; after this the instance must be re-initialised.
    /// </summary>
    public void Cleanup()
    {
        lock (_gate)
        {
            if (!IsAvailable)
            {
                return;
            }

            _cleanup?.Invoke();
            _connectCallback = null;
            _initialized = false;
        }
    }

    /// <summary>
    /// Reads the connect code the native module last observed. Kept for parity
    /// with the upstream export; the managed flow relies on the callback.
    /// </summary>
    public bool TryGetConnectCode(out string connectCode)
    {
        connectCode = string.Empty;

        if (!IsAvailable || _isConnected is null)
        {
            return false;
        }

        const int size = 256;
        IntPtr buffer = Marshal.AllocCoTaskMem(size);
        try
        {
            if (_isConnected(buffer, size) == 0)
            {
                return false;
            }

            connectCode = Marshal.PtrToStringUTF8(buffer) ?? string.Empty;
            return true;
        }
        finally
        {
            Marshal.FreeCoTaskMem(buffer);
        }
    }

    /// <summary>
    /// Pops the native module's pending error. Returns null when there is none
    /// or when the loaded DLL predates the diagnostics exports.
    /// </summary>
    public NativeAudioError? TakeLastError()
    {
        if (_getErrorCode is null)
        {
            return null;
        }

        int code = _getErrorCode();
        string message = string.Empty;

        if (_getErrorMessage is not null)
        {
            IntPtr ptr = _getErrorMessage();
            if (ptr != IntPtr.Zero)
            {
                message = Marshal.PtrToStringUTF8(ptr) ?? string.Empty;
            }
        }

        _clearError?.Invoke();

        if (code == 0 && string.IsNullOrEmpty(message))
        {
            return null;
        }

        return new NativeAudioError(code, message);
    }

    private string DescribeLastError()
    {
        var error = TakeLastError();
        return error is null ? "no diagnostics available" : $"{error.Code}: {error.Message}";
    }

    public void Dispose()
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;

            if (IsAvailable)
            {
                _cleanup?.Invoke();
            }

            _connectCallback = null;
            _initialized = false;
            IsAvailable = false;

            if (_library != IntPtr.Zero)
            {
                NativeLibrary.Free(_library);
                _library = IntPtr.Zero;
            }
        }
    }
}
