using System.IO;
using System.Net;
using System.Net.Sockets;
using AnPod.Core.Adb;
using AnPod.Core.Logging;
using AnPod.Core.Models;

namespace AnPod.Desktop.Services;

/// <summary>A connect/handshake failure carrying a message meant for the user.</summary>
public sealed class ConnectionFailedException : Exception
{
    public ConnectionFailedException(string message, Exception? inner = null)
        : base(message, inner)
    {
    }
}

/// <summary>Payload layout for the Android side (plan §3.2).</summary>
public static class AndroidServerLayout
{
    /// <summary>Where the payload is pushed on the phone.</summary>
    public const string RemotePath = "/data/local/tmp/anpod_server";

    /// <summary>Directory passed as the <c>app_process</c> working directory.</summary>
    public const string RemoteWorkingDirectory = "/data/local/tmp";

    /// <summary>Java entry point inside the payload.</summary>
    public const string MainClass = "com.anpod.server.Main";
}

public sealed record ConnectionStateChangedEventArgs(ConnectionState State, string Message, string? DeviceName);

/// <summary>
/// The only component allowed to mutate connection state (plan §5.3).
/// <para>
/// Concurrency contract: every state transition and every session mutation
/// happens while holding <c>_gate</c>. Long waits (push, handshake) also hold
/// it, but they observe <c>_sessionCts</c>, and both <see cref="ConnectAsync"/>
/// and <see cref="DisconnectAsync"/> cancel that token <em>before</em> waiting
/// for the gate — so a user click is never blocked behind a slow ADB call.
/// </para>
/// </summary>
public sealed class ConnectionManager : IAsyncDisposable
{
    private static readonly TimeSpan ReverseTimeout = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan PushTimeout = TimeSpan.FromSeconds(15);
    private static readonly TimeSpan HandshakeTimeout = TimeSpan.FromSeconds(10);
    private static readonly int[] ReconnectBackoffMilliseconds = { 1000, 2000, 5000 };

    /// <summary>
    /// How often an established session is checked for capture-side failures.
    /// This is the ONLY polling the app does, and it runs only while a session
    /// exists: with no phone connected there is nothing to watch, and the
    /// device list is refreshed on demand instead (plan §5.2).
    /// </summary>
    private static readonly TimeSpan SessionWatchdogInterval = TimeSpan.FromSeconds(2.5);

    private readonly AdbRunner _adb;
    private readonly NativeAudioCapture _capture;
    private readonly DeviceDiscoveryService _discovery;
    private readonly PlaybackEndpointMuter _endpointMuter;
    private readonly WirelessAdbService _wireless;
    private readonly ILog _log;
    private readonly string _serverPayloadPath;
    private readonly SynchronizationContext? _uiContext;

    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly object _fieldLock = new();

    private long _generation;
    private CancellationTokenSource? _sessionCts;
    private CancellationTokenSource? _watchdogCts;
    private TaskCompletionSource<string>? _handshake;
    private AndroidProcessSession? _androidProcess;
    private string? _reverseName;
    private string? _activeSerial;
    private string? _activeDisplayName;
    private string? _activeToken;
    private string? _suppressedAutoConnectSerial;
    private bool _userInitiatedDisconnect;
    private bool _wantsPhoneOutput;
    private CancellationTokenSource? _reconnectCts;
    private bool _disposed;

    /// <summary>
    /// Transport of the session currently up, or of the one being built. Kept
    /// because teardown has to know whether the wireless link should be dropped
    /// as well (plan §1.4: USB and Wi-Fi are exclusive sessions).
    /// </summary>
    private TransportKind _activeTransport = TransportKind.Unknown;

    /// <summary>
    /// Guards the idle wireless probe against overlapping runs. Discovery only
    /// fires on demand, but two callers can ask at the same moment, and two
    /// concurrent <c>adb connect</c> calls to one phone is exactly the kind of
    /// race plan §5.9 asks to serialise.
    /// </summary>
    private int _wirelessProbeBusy;

    public ConnectionManager(
        AdbRunner adb,
        NativeAudioCapture capture,
        DeviceDiscoveryService discovery,
        PlaybackEndpointMuter endpointMuter,
        WirelessAdbService wireless,
        string serverPayloadPath,
        ILog log)
    {
        _adb = adb;
        _capture = capture;
        _discovery = discovery;
        _endpointMuter = endpointMuter;
        _wireless = wireless;
        _serverPayloadPath = serverPayloadPath;
        _log = log;
        _uiContext = SynchronizationContext.Current;

        _capture.AndroidConnected += OnCaptureConnected;
        _discovery.DevicesChanged += OnDevicesChanged;
    }

    public ConnectionState State { get; private set; } = ConnectionState.NoDevice;

    /// <summary>
    /// True while a session exists — the reverse tunnel, the Android payload and
    /// the handshake — whether or not audio is currently being forwarded.
    /// </summary>
    public bool IsSessionUp => State is ConnectionState.Connected or ConnectionState.Streaming;

    public bool WantsPhoneOutput => _wantsPhoneOutput;

    /// <summary>True when the last session ended in <see cref="ConnectionState.Error"/>.</summary>
    public bool HasError => State == ConnectionState.Error;

    public event EventHandler<ConnectionStateChangedEventArgs>? StateChanged;

    /// <summary>Set from settings; gates both auto-connect and session recovery.</summary>
    public bool AutoConnectLastDevice { get; set; } = true;

    /// <summary>Serial of the device the user last chose, for auto-connect.</summary>
    public string? LastDeviceId { get; set; }

    /// <summary>Fires when <see cref="LastDeviceId"/> should be persisted.</summary>
    public event EventHandler<string?>? LastDeviceIdChanged;

    /// <summary>
    /// Set from settings. When on (and <see cref="AutoConnectLastDevice"/> is on
    /// too), a dropped wireless link is re-established with <c>adb connect</c>
    /// before discovery is consulted (plan §5.10).
    /// </summary>
    public bool AutoReconnectWifi { get; set; } = true;

    /// <summary>
    /// Set from settings — the transport the user last picked (plan §5.10).
    /// Only used to decide whether an idle refresh should try to bring a
    /// remembered wireless phone back; it never overrides an explicit choice of
    /// device, and it never causes a wireless connection the user did not ask
    /// for (plan §1.3).
    /// </summary>
    public TransportKind PreferredTransport { get; set; } = TransportKind.Usb;

    /// <summary>AudioTrack mode snapshotted at the start of each connection attempt.</summary>
    public PlaybackMode PlaybackMode { get; set; } = PlaybackMode.Balanced;

    /// <summary>
    /// Last <c>ip:port</c> that accepted a connection. Plan §1.1a: the phone's
    /// address and port can change, so this is a hint for the next attempt, never
    /// an address to retry forever.
    /// </summary>
    public string? LastWirelessEndpoint { get; set; }

    /// <summary>Fires when <see cref="LastWirelessEndpoint"/> should be persisted.</summary>
    public event EventHandler<string?>? LastWirelessEndpointChanged;

    /// <summary>
    /// Records an endpoint that just worked, so reconnection and the next launch
    /// have somewhere to start (plan §1.1a). Passing null forgets it.
    /// </summary>
    public void RememberWirelessEndpoint(string? endpoint)
    {
        if (string.Equals(endpoint, LastWirelessEndpoint, StringComparison.Ordinal))
        {
            return;
        }

        LastWirelessEndpoint = endpoint;
        LastWirelessEndpointChanged?.Invoke(this, endpoint);
    }

    // ---------------------------------------------------------------- public API

    /// <summary>Opens a session to <paramref name="device"/>.</summary>
    public async Task ConnectAsync(DeviceInfo device, bool startPlayback = true, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(device);

        CancelCurrentSession();
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            if (IsSessionUp || State == ConnectionState.Connecting)
            {
                return;
            }

            _suppressedAutoConnectSerial = null;
            _wantsPhoneOutput = startPlayback;
            var token = BeginSession(cancellationToken);

            try
            {
                await ConnectCoreAsync(device, token, startPlayback).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                await TeardownSessionAsync().ConfigureAwait(false);
                SetState(ConnectionState.Ready, "已取消", device.DisplayName);
            }
            catch (ConnectionFailedException ex)
            {
                await TeardownSessionAsync().ConfigureAwait(false);
                _log.Warn($"Connect failed: {ex.Message}");
                SetState(ConnectionState.Error, ex.Message, device.DisplayName);
            }
            catch (Exception ex)
            {
                await TeardownSessionAsync().ConfigureAwait(false);
                _log.Error("Connect failed unexpectedly", ex);
                SetState(ConnectionState.Error, "连接失败，可查看日志了解详情。", device.DisplayName);
            }
        }
        finally
        {
            _gate.Release();
        }
    }

    /// <summary>
    /// Closes the session. <paramref name="userInitiated"/> suppresses any
    /// automatic reconnect for this device until it is unplugged (plan §1.1.6).
    /// </summary>
    public async Task DisconnectAsync(bool userInitiated = true)
    {
        if (userInitiated)
        {
            _userInitiatedDisconnect = true;
            _suppressedAutoConnectSerial = _activeSerial;
        }

        CancelCurrentSession();
        CancelReconnect();
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            var deviceName = _activeDisplayName;
            await TeardownSessionAsync().ConfigureAwait(false);

            if (State != ConnectionState.Error)
            {
                SetState(ConnectionState.Ready, "已断开", deviceName);
            }
        }
        finally
        {
            _gate.Release();
        }
    }

    private void CancelReconnect()
    {
        try { _reconnectCts?.Cancel(); }
        catch (ObjectDisposedException) { }
    }

    /// <summary>
    /// Releases the remembered wireless endpoint from the shared ADB server and
    /// forgets it.
    /// </summary>
    /// <remarks>
    /// Always with an explicit endpoint. A bare <c>adb disconnect</c> removes
    /// every TCP device the ADB server knows about — including ones another tool
    /// is using — which is the wireless equivalent of <c>reverse --remove-all</c>
    /// (plan §3.5). Refused while a session is up: the phone in use must not be
    /// pulled out from under the running audio path.
    /// </remarks>
    public async Task DisconnectWirelessAsync()
    {
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            if (IsSessionUp || State == ConnectionState.Connecting)
            {
                return;
            }

            var endpoint = LastWirelessEndpoint;
            if (string.IsNullOrWhiteSpace(endpoint))
            {
                return;
            }

            await _wireless.DisconnectAsync(endpoint).ConfigureAwait(false);
            RememberWirelessEndpoint(null);
            _log.Info("Wireless ADB link released");
        }
        finally
        {
            _gate.Release();
        }
    }

    /// <summary>
    /// Turns audio forwarding on or off <em>without</em> touching the session.
    /// <para>
    /// This is what makes the "从手机播放声音" switch independent of the
    /// connection: disabling pauses the native stream — the peer keeps its
    /// socket and receives silence — and hands the machine's speakers back;
    /// enabling reverses both. Neither direction re-handshakes, so toggling is
    /// instant and the phone never leaves <see cref="ConnectionState.Connected"/>.
    /// </para>
    /// <para>No-op unless a session is up.</para>
    /// </summary>
    public async Task SetAudioEnabledAsync(bool enabled)
    {
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            SetAudioEnabledCore(enabled);
        }
        finally
        {
            _gate.Release();
        }
    }

    /// <summary>
    /// The body of <see cref="SetAudioEnabledAsync"/> with no gate of its own,
    /// so session recovery can restore the user's chosen audio state while it
    /// already holds the gate.
    /// </summary>
    private void SetAudioEnabledCore(bool enabled)
    {
        _wantsPhoneOutput = enabled;
        if (enabled)
        {
            if (State != ConnectionState.Connected)
            {
                return;
            }

            // The capture thread has been running the whole time; this just
            // stops it substituting silence.
            _capture.SetStreamingEnabled(true);
            int mutedCount = _endpointMuter.MuteAll();
            SetState(
                ConnectionState.Streaming,
                mutedCount > 0 ? "正在播放（已静音本机扬声器）" : "正在播放",
                _activeDisplayName);
            _log.Info("Audio resumed on the existing session");
        }
        else
        {
            if (State != ConnectionState.Streaming)
            {
                return;
            }

            _capture.SetStreamingEnabled(false);
            RestoreSpeakers();
            SetState(ConnectionState.Connected, "已连接，声音从电脑播放", _activeDisplayName);
            _log.Info("Audio paused; the device session is kept alive");
        }
    }

    private void CancelCurrentSession()
    {
        CancellationTokenSource? cts;
        lock (_fieldLock)
        {
            cts = _sessionCts;
        }

        try
        {
            cts?.Cancel();
        }
        catch (ObjectDisposedException)
        {
        }
    }

    // ------------------------------------------------------------- connect steps

    private CancellationToken BeginSession(CancellationToken externalToken)
    {
        var cts = CancellationTokenSource.CreateLinkedTokenSource(externalToken);
        CancellationTokenSource? previous;

        lock (_fieldLock)
        {
            previous = _sessionCts;
            _sessionCts = cts;
            _generation++;
            _handshake = null;
            _userInitiatedDisconnect = false;
        }

        previous?.Dispose();
        return cts.Token;
    }

    /// <summary>Implements plan §5.3 steps 1-10. Gate is held by the caller.</summary>
    private async Task ConnectCoreAsync(DeviceInfo device, CancellationToken token, bool startPlayback)
    {
        var playbackMode = PlaybackMode;
        _log.Info($"Playback mode requested: {playbackMode}");
        _activeSerial = device.Serial;
        _activeDisplayName = device.DisplayName;
        _activeTransport = device.Transport;
        _log.AddSecret(device.Serial);

        SetState(ConnectionState.Connecting, $"正在连接 {device.DisplayName}…", device.DisplayName);

        // Step 1 — only ever act on the exact device the user selected.
        if (device.State != AdbDeviceState.Device)
        {
            throw new ConnectionFailedException(device.IsWifi
                ? "手机尚未授权或已离线，请在手机上重新开启无线调试。"
                : "设备尚未授权或已离线，请检查手机上的 USB 调试授权。");
        }

        // Plan §5.2: a transport we cannot classify must not be driven. USB and
        // Wi-Fi are both fine — they are exclusive sessions, not a
        // supported/unsupported split (plan §1.4).
        if (device.TransportUnknown)
        {
            throw new ConnectionFailedException("无法确认该设备的连接方式，请重新刷新设备列表。");
        }

        // Remember the endpoint so a dropped wireless link can be rebuilt without
        // making the user retype an address that may well have changed (plan §5.10).
        if (device.IsWifi)
        {
            RememberWirelessEndpoint(device.Serial);
        }

        // Step 2 — remove only mappings AnPod itself created (plan §3.4.2).
        await RemoveOwnReverseAsync(device.Serial, token).ConfigureAwait(false);

        // Step 3 — native capture must be usable before we touch the phone.
        if (!_capture.IsAvailable)
        {
            throw new ConnectionFailedException(_capture.LoadError ?? "音频采集组件不可用。");
        }

        if (!_capture.Initialize())
        {
            throw new ConnectionFailedException("未发现可采集的 Windows 输出设备。");
        }

        // Steps 4-5 — reserve a loopback port and bind the listener.
        var sessionToken = BuildSessionToken();
        _activeToken = sessionToken;
        _log.AddSecret(sessionToken);

        int port = 0;
        bool listening = false;
        for (int attempt = 0; attempt < 5 && !listening; attempt++)
        {
            token.ThrowIfCancellationRequested();
            port = FindFreeLoopbackPort();
            listening = _capture.Listen(port, sessionToken);
        }

        if (!listening)
        {
            throw new ConnectionFailedException("无法在本机开启音频传输端口。");
        }

        // Step 6 — reverse tunnel from the phone's abstract socket to our port.
        _reverseName = $"anpod_{_generation}";
        var reverse = await _adb
            .RunAsync(
                device.Serial,
                new[] { "reverse", $"localabstract:{_reverseName}", $"tcp:{port}" },
                ReverseTimeout,
                token)
            .ConfigureAwait(false);

        if (!reverse.Success)
        {
            throw new ConnectionFailedException($"无法建立传输通道（{reverse.DescribeFailure()}）。");
        }

        // Step 7 — push the Android payload.
        if (!File.Exists(_serverPayloadPath))
        {
            throw new ConnectionFailedException("缺少手机端服务载荷 server，请先构建 AnPod.AndroidServer。");
        }

        var push = await _adb
            .RunAsync(
                device.Serial,
                new[] { "push", _serverPayloadPath, AndroidServerLayout.RemotePath },
                PushTimeout,
                token)
            .ConfigureAwait(false);

        if (!push.Success)
        {
            throw new ConnectionFailedException($"无法把音频服务推送到手机（{push.DescribeFailure()}）。");
        }

        // Step 8 — start the headless Android process and keep its pipes drained.
        var session = AndroidProcessSession.Start(
            _adb.AdbPath,
            device.Serial,
            new[]
            {
                $"-Djava.class.path={AndroidServerLayout.RemotePath}",
                AndroidServerLayout.RemoteWorkingDirectory,
                AndroidServerLayout.MainClass,
                $"socketName={_reverseName}",
                $"connectCode={sessionToken}",
                $"playbackMode={(playbackMode == PlaybackMode.LowLatency ? "low-latency" : "balanced")}",
            },
            _log);

        _androidProcess = session;
        session.Exited += OnAndroidProcessExited;
        session.OutputLine += (_, line) => _log.Debug($"[android] {line}");

        // Step 9 — wait for the handshake and validate it against this session.
        var code = await WaitForHandshakeAsync(session, token).ConfigureAwait(false);

        // Checked before the token comparison: a payload that died during the
        // wait cannot have produced a usable handshake, and "it exited" is a
        // far more actionable message than "the code did not match" (plan §6).
        if (session.HasExited)
        {
            var tail = session.StderrTail.Trim();
            _log.Warn($"Android process exited during handshake. {tail}");
            throw new ConnectionFailedException("手机端音频服务启动后立即退出，可能不受当前系统支持。");
        }

        if (!string.Equals(code, sessionToken, StringComparison.Ordinal))
        {
            // A different local process connected; refuse to start capture.
            throw new ConnectionFailedException("握手校验失败，已拒绝该连接。");
        }

        // Step 10 — start capture.
        if (!_capture.Start())
        {
            var error = _capture.TakeLastError();
            throw new ConnectionFailedException(error is null
                ? "无法开始采集系统音频。"
                : $"无法开始采集系统音频：{error.Message}");
        }

        LastDeviceId = device.Serial;
        LastDeviceIdChanged?.Invoke(this, device.Serial);

        // Step 11 — silence the machine's own speakers. Done after capture has
        // started so a driver that ties endpoint mute to the loopback tap cannot
        // stop us from having a working stream in the first place. Best-effort:
        // MuteAll never throws and reports 0 when it changed nothing.
        if (startPlayback)
        {
            SetState(ConnectionState.Connected, "已连接，准备切换声音输出…", device.DisplayName);
            SetAudioEnabledCore(true);
        }
        else
        {
            _capture.SetStreamingEnabled(false);
            RestoreSpeakers();
            SetState(ConnectionState.Connected, "已连接，声音从电脑播放", device.DisplayName);
        }
        _log.Info($"Streaming session started on port {port}");

        // Only now is there something worth watching. Nothing polls while idle.
        StartWatchdog();
    }

    // ------------------------------------------------------- session watchdog

    /// <summary>
    /// Watches an established session for a capture-side failure. Without it a
    /// dead WASAPI stream is invisible: the socket stays open, no PCM flows, and
    /// the UI keeps claiming it is playing. Runs only between session start and
    /// teardown (plan §5.2).
    /// </summary>
    private void StartWatchdog()
    {
        StopWatchdog();

        var cts = new CancellationTokenSource();
        _watchdogCts = cts;
        _ = WatchdogAsync(cts.Token);
    }

    /// <summary>
    /// Cancels the watchdog without waiting for it. Deliberately not awaited:
    /// teardown runs under the gate, and the watchdog's own work can end up
    /// wanting that same gate, so blocking here would deadlock. The loop checks
    /// its token between steps and exits on its own.
    /// </summary>
    private void StopWatchdog()
    {
        var cts = _watchdogCts;
        _watchdogCts = null;

        try
        {
            cts?.Cancel();
        }
        catch (ObjectDisposedException)
        {
        }

        cts?.Dispose();
    }

    private async Task WatchdogAsync(CancellationToken token)
    {
        while (!token.IsCancellationRequested)
        {
            try
            {
                await Task.Delay(SessionWatchdogInterval, token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return;
            }

            // Only meaningful while audio is actually flowing; while paused the
            // capture thread is idle from the listener's point of view.
            if (State != ConnectionState.Streaming)
            {
                continue;
            }

            var error = _capture.TakeLastError();
            if (error is null)
            {
                continue;
            }

            _log.Warn($"Audio capture reported error {error.Code}: {error.Message}");
            _ = HandleSessionLossAsync("Windows 音频输出发生变化");
            return;
        }
    }

    private async Task<string> WaitForHandshakeAsync(AndroidProcessSession session, CancellationToken token)
    {
        var tcs = new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        lock (_fieldLock)
        {
            _handshake = tcs;
        }

        // A payload that dies immediately (corrupt container, unsupported ROM)
        // can never deliver a handshake. Complete the wait as soon as the
        // process goes away so the caller can report the real cause instead of
        // waiting out HandshakeTimeout and blaming a timeout.
        void OnSessionExited(object? sender, EventArgs e) => tcs.TrySetResult(string.Empty);

        session.Exited += OnSessionExited;

        using var timeoutCts = CancellationTokenSource.CreateLinkedTokenSource(token);
        timeoutCts.CancelAfter(HandshakeTimeout);

        try
        {
            return await tcs.Task.WaitAsync(timeoutCts.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (!token.IsCancellationRequested)
        {
            throw new ConnectionFailedException("连接超时，请重新连接。");
        }
        finally
        {
            session.Exited -= OnSessionExited;

            lock (_fieldLock)
            {
                if (ReferenceEquals(_handshake, tcs))
                {
                    _handshake = null;
                }
            }
        }
    }

    private void OnCaptureConnected(object? sender, string connectCode)
    {
        TaskCompletionSource<string>? tcs;
        lock (_fieldLock)
        {
            tcs = _handshake;
        }

        tcs?.TrySetResult(connectCode);
    }

    // ------------------------------------------------------------- teardown

    /// <summary>
    /// Idempotent, best-effort teardown (plan §5.3). Every step is guarded so a
    /// single failure cannot leave the rest of the cleanup undone.
    /// </summary>
    private async Task TeardownSessionAsync()
    {
        StopWatchdog();

        // First, before anything can throw: give the user their speakers back.
        // Every path out of a session (disconnect, connect failure, session loss,
        // app exit) funnels through here, so this is the one place that has to be
        // right — a leaked mute is indistinguishable from a Windows fault.
        RestoreSpeakers();
        _wantsPhoneOutput = false;

        try
        {
            _capture.Stop();
        }
        catch (Exception ex)
        {
            _log.Debug($"capture.Stop threw: {ex.Message}");
        }

        var session = _androidProcess;
        _androidProcess = null;
        if (session is not null)
        {
            session.Exited -= OnAndroidProcessExited;
            try
            {
                await session.StopAsync(TimeSpan.FromSeconds(2)).ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                _log.Debug($"Stopping Android session threw: {ex.Message}");
            }

            await session.DisposeAsync().ConfigureAwait(false);
        }

        var serial = _activeSerial;
        var reverseName = _reverseName;
        _reverseName = null;

        if (!string.IsNullOrEmpty(serial) && !string.IsNullOrEmpty(reverseName))
        {
            // Remove ONLY our own mapping (plan §3.4.2/§7.1).
            try
            {
                var result = await _adb
                    .RunAsync(
                        serial,
                        new[] { "reverse", "--remove", $"localabstract:{reverseName}" },
                        ReverseTimeout,
                        CancellationToken.None)
                    .ConfigureAwait(false);

                if (!result.Success)
                {
                    _log.Debug($"Reverse mapping cleanup: {result.DescribeFailure()}");
                }
            }
            catch (Exception ex)
            {
                _log.Debug($"Reverse mapping cleanup threw: {ex.Message}");
            }
        }

        try
        {
            _capture.Cleanup();
        }
        catch (Exception ex)
        {
            _log.Debug($"capture.Cleanup threw: {ex.Message}");
        }

        lock (_fieldLock)
        {
            _handshake = null;
            _activeToken = null;
        }

        _activeDisplayName = null;
        _activeTransport = TransportKind.Unknown;
    }

    /// <summary>
    /// Hands the machine's playback endpoints back. Never throws: a failure to
    /// unmute must not abort the rest of a teardown.
    /// </summary>
    private void RestoreSpeakers()
    {
        try
        {
            int restored = _endpointMuter.Restore();
            if (restored > 0)
            {
                _log.Info($"Restored {restored} local playback device(s).");
            }
        }
        catch (Exception ex)
        {
            _log.Warn($"Restoring playback devices threw: {ex.Message}");
        }
    }

    // ------------------------------------------------------- session loss

    private void OnAndroidProcessExited(object? sender, EventArgs e)
    {
        if (IsSessionUp)
        {
            // The stderr tail is the only place the real cause appears — the Java
            // side prints "ANPOD ..." lines there. Logging just "exited" left the
            // user with a reconnect loop and nothing to act on.
            var tail = (sender as AndroidProcessSession)?.StderrTail.Trim();
            _log.Warn(string.IsNullOrEmpty(tail)
                ? "Android audio process exited while streaming (no stderr captured)"
                : $"Android audio process exited while streaming. {tail}");
            _ = HandleSessionLossAsync("手机端音频服务已停止");
        }
    }

    private void OnDevicesChanged(object? sender, IReadOnlyList<DeviceInfo> devices)
    {
        // Clear the auto-connect suppression once the device is physically gone.
        var suppressed = _suppressedAutoConnectSerial;
        if (!string.IsNullOrEmpty(suppressed) && devices.All(d => d.Serial != suppressed))
        {
            _suppressedAutoConnectSerial = null;
        }

        // Capture-side failures are owned by the session watchdog, which is the
        // only thing that runs while a session is up.
        if (IsSessionUp)
        {
            var serial = _activeSerial;
            if (!string.IsNullOrEmpty(serial) &&
                devices.All(d => d.Serial != serial || d.State != AdbDeviceState.Device))
            {
                _ = HandleSessionLossAsync(_activeTransport == TransportKind.Wifi
                    ? "无线连接已断开"
                    : "USB 连接已断开");
            }

            return;
        }

        SyncIdleState(devices);

        // A wireless phone that slept drops out of `adb devices` completely, so
        // waiting for it to reappear would never work — `adb connect` is what
        // puts it back (plan §1.1a). Discovery runs only on demand, so this
        // cannot turn into a poll loop.
        if (ShouldProbeWireless(devices))
        {
            _ = ProbeWirelessAsync();
        }

        if (State == ConnectionState.Ready && AutoConnectLastDevice)
        {
            TryAutoConnect(devices);
        }
    }

    /// <summary>
    /// True when a remembered wireless phone is worth one <c>adb connect</c>:
    /// the user chose Wi-Fi, auto-reconnect is on, and the endpoint is not in the
    /// list we were just handed.
    /// </summary>
    private bool ShouldProbeWireless(IReadOnlyList<DeviceInfo> devices)
    {
        if (!AutoConnectLastDevice || !AutoReconnectWifi)
        {
            return false;
        }

        if (PreferredTransport != TransportKind.Wifi)
        {
            return false;
        }

        var endpoint = LastWirelessEndpoint;
        if (string.IsNullOrWhiteSpace(endpoint))
        {
            return false;
        }

        return devices.All(d => !string.Equals(d.Serial, endpoint, StringComparison.Ordinal));
    }

    /// <summary>
    /// Brings a remembered wireless phone back into the ADB device list, then
    /// re-scans so the ordinary auto-connect path can pick it up. Fire-and-forget
    /// on purpose: it is called from inside a discovery refresh, which still owns
    /// its gate until the event handler returns.
    /// </summary>
    private async Task ProbeWirelessAsync()
    {
        if (Interlocked.Exchange(ref _wirelessProbeBusy, 1) == 1)
        {
            return;
        }

        try
        {
            // Yield before touching discovery, so the refresh that called us has
            // definitely released its gate.
            await Task.Yield();

            if (!await TryWirelessConnectAsync().ConfigureAwait(false))
            {
                return;
            }

            await _discovery.RefreshAsync(CancellationToken.None).ConfigureAwait(false);
        }
        catch (Exception ex)
        {
            _log.Debug($"Wireless discovery probe failed: {ex.Message}");
        }
        finally
        {
            Interlocked.Exchange(ref _wirelessProbeBusy, 0);
        }
    }

    /// <summary>
    /// Reflects the transport list in the state machine while no session is
    /// running — plan §5.3: <c>NoDevice --USB出现--> Unauthorized / Ready</c>.
    /// Without this the manager would stay in <see cref="ConnectionState.NoDevice"/>
    /// forever, which also silently disables auto-connect because that is gated
    /// on <see cref="ConnectionState.Ready"/>.
    /// </summary>
    private void SyncIdleState(IReadOnlyList<DeviceInfo> devices)
    {
        // Only idle states are derived. Connecting/Streaming/Reconnecting are
        // owned by the running session, and Error stays visible until the user
        // retries or the device goes away (plan §5.3).
        if (State is not (ConnectionState.NoDevice
            or ConnectionState.Ready
            or ConnectionState.Unauthorized))
        {
            return;
        }

        ConnectionState derived;
        string message;
        string? deviceName = null;

        var connectable = devices.FirstOrDefault(d => d.IsConnectable);
        if (connectable is not null)
        {
            derived = ConnectionState.Ready;
            message = "已就绪";
            deviceName = connectable.DisplayName;
        }
        else if (devices.Any(d => d.State == AdbDeviceState.Unauthorized))
        {
            derived = ConnectionState.Unauthorized;
            message = "请在手机上允许 USB 调试授权";
        }
        else if (devices.Count == 0)
        {
            derived = ConnectionState.NoDevice;
            message = "未检测到设备";
        }
        else
        {
            // Devices are present but none is usable: offline, "no permissions",
            // or a transport we will not drive. Saying "no device" would be
            // misleading, so name the real condition.
            derived = ConnectionState.NoDevice;
            message = "未发现可用的设备";
        }

        // Discovery re-runs every couple of seconds; only report real changes so
        // the status line does not churn.
        if (derived == State)
        {
            return;
        }

        SetState(derived, message, deviceName);
    }

    private void TryAutoConnect(IReadOnlyList<DeviceInfo> devices)
    {
        var last = LastDeviceId;
        if (string.IsNullOrEmpty(last) || last == _suppressedAutoConnectSerial)
        {
            return;
        }

        // The remembered serial is not necessarily the one this mode uses — right
        // after switching USB/Wi-Fi it usually is not. Without this check a stored
        // USB serial would open a USB session while the window sat on the Wi-Fi
        // panel, which is exactly what makes the two modes look like one screen.
        // No match is not an error: the user picks a device by hand instead
        // (plan §5.10).
        var device = devices.FirstOrDefault(
            d => d.Serial == last && d.IsConnectable && d.MatchesTransport(PreferredTransport));

        if (device is null)
        {
            return;
        }

        _log.Info("Auto-connecting last device");
        _ = ConnectAsync(device, startPlayback: false);
    }

    /// <summary>
    /// Reacts to a lost session: teardown, then a bounded reconnect when the
    /// user has not explicitly disconnected (plan §5.3).
    /// </summary>
    private async Task HandleSessionLossAsync(string reason)
    {
        CancelCurrentSession();
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            if (_userInitiatedDisconnect || !IsSessionUp)
            {
                return;
            }

            // Remembered so a session that was deliberately paused comes back
            // paused. Reconnecting must not silently turn the audio back on.
            bool wasPlaying = WantsPhoneOutput;
            bool wasWireless = _activeTransport == TransportKind.Wifi;
            var deviceName = _activeDisplayName;
            await TeardownSessionAsync().ConfigureAwait(false);

            if (!AutoConnectLastDevice)
            {
                SetState(ConnectionState.Error, reason, deviceName);
                return;
            }

            var serial = LastDeviceId;
            var reconnectCts = new CancellationTokenSource();
            _reconnectCts = reconnectCts;
            var reconnectToken = reconnectCts.Token;
            try
            {
                for (int attempt = 0; attempt < ReconnectBackoffMilliseconds.Length; attempt++)
                {
                    if (reconnectToken.IsCancellationRequested || _userInitiatedDisconnect) return;
                    SetState(
                        ConnectionState.Reconnecting,
                        $"{reason}，正在重连（{attempt + 1}/{ReconnectBackoffMilliseconds.Length}）…",
                        deviceName);

                    try
                    {
                        await Task.Delay(ReconnectBackoffMilliseconds[attempt], reconnectToken).ConfigureAwait(false);
                    }
                    catch (OperationCanceledException)
                    {
                        return;
                    }

                    if (_userInitiatedDisconnect) return;

                    // A wireless phone that slept leaves `adb devices` entirely, so
                    // refreshing alone would never find it again; `adb connect` is
                    // what puts it back (plan §1.1a). Bounded by the attempt counter.
                    if (wasWireless && !await TryWirelessConnectAsync(reconnectToken).ConfigureAwait(false))
                    {
                        reconnectToken.ThrowIfCancellationRequested();
                    }

                    // Refresh on demand: nothing polls while idle, so the cached
                    // list cannot be trusted to tell us whether the device is back.
                    try
                    {
                        await _discovery.RefreshAsync(reconnectToken).ConfigureAwait(false);
                    }
                    catch (OperationCanceledException) when (reconnectToken.IsCancellationRequested)
                    {
                        return;
                    }
                    catch (Exception ex)
                    {
                        _log.Debug($"Discovery refresh during reconnect failed: {ex.Message}");
                    }

                    var device = _discovery.Devices.FirstOrDefault(
                        d => d.Serial == serial && d.IsConnectable);

                    if (device is null)
                    {
                        continue;
                    }

                    var token = BeginSession(reconnectToken);
                    try
                    {
                        await ConnectCoreAsync(device, token, wasPlaying).ConfigureAwait(false);

                        _log.Info("Reconnect succeeded");
                        return;
                    }
                    catch (ConnectionFailedException ex)
                    {
                        _log.Warn($"Reconnect attempt {attempt + 1} failed: {ex.Message}");
                        await TeardownSessionAsync().ConfigureAwait(false);
                    }
                    catch (OperationCanceledException)
                    {
                        await TeardownSessionAsync().ConfigureAwait(false);
                        return;
                    }
                }
            }
            finally
            {
                if (ReferenceEquals(_reconnectCts, reconnectCts)) _reconnectCts = null;
                reconnectCts.Dispose();
            }

            SetState(ConnectionState.Error, "重连失败，请手动重试。", deviceName);
        }
        finally
        {
            _gate.Release();
        }
    }

    // ------------------------------------------------------------- helpers

    /// <summary>
    /// One bounded <c>adb connect</c> back to the remembered wireless endpoint.
    /// </summary>
    /// <remarks>
    /// Best-effort by design. Both callers — the bounded reconnect loop and the
    /// idle discovery probe — already have their own way of reporting failure, so
    /// throwing here would replace a useful status line with the generic "连接失败"
    /// path. A failure is logged and reported as <c>false</c> instead.
    /// <para>
    /// The endpoint is never retried on a timer: plan §1.1a is explicit that a
    /// phone's address and port can change, so a stale address must not be
    /// hammered. Each call is driven by something the user did or by an attempt
    /// the reconnect loop has already counted.
    /// </para>
    /// </remarks>
    private async Task<bool> TryWirelessConnectAsync(CancellationToken cancellationToken = default)
    {
        if (!AutoReconnectWifi)
        {
            return false;
        }

        var endpoint = LastWirelessEndpoint;
        if (string.IsNullOrWhiteSpace(endpoint))
        {
            return false;
        }

        try
        {
            var result = await _wireless.ConnectAsync(endpoint, cancellationToken).ConfigureAwait(false);
            if (result.Success)
            {
                _log.Info("Wireless ADB link restored");
                return true;
            }

            _log.Debug($"Wireless ADB link could not be restored: {result.Message}");
            return false;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch (Exception ex)
        {
            _log.Debug($"Wireless ADB reconnect threw: {ex.Message}");
            return false;
        }
    }

    private static string BuildSessionToken() => Guid.NewGuid().ToString("N");

    private static int FindFreeLoopbackPort()
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        try
        {
            return ((IPEndPoint)listener.LocalEndpoint).Port;
        }
        finally
        {
            listener.Stop();
        }
    }

    /// <summary>
    /// Best-effort removal of a mapping left behind by a previous AnPod session.
    /// Only names carrying <see cref="AdbOutputParser.OwnReversePrefix"/> are
    /// touched, so an earlier crash is recoverable without ever issuing
    /// <c>reverse --remove-all</c> (plan §3.4.2).
    /// </summary>
    private async Task RemoveOwnReverseAsync(string serial, CancellationToken token)
    {
        var result = await _adb
            .RunAsync(serial, new[] { "reverse", "--list" }, ReverseTimeout, token)
            .ConfigureAwait(false);

        if (!result.Success)
        {
            return;
        }

        foreach (var name in AdbOutputParser.ParseOwnReverseMappings(result.StdOut))
        {
            await _adb
                .RunAsync(
                    serial,
                    new[] { "reverse", "--remove", name },
                    ReverseTimeout,
                    token)
                .ConfigureAwait(false);
        }
    }

    private void SetState(ConnectionState state, string message, string? deviceName)
    {
        State = state;
        var args = new ConnectionStateChangedEventArgs(state, message, deviceName);

        if (_uiContext is not null)
        {
            _uiContext.Post(_ => StateChanged?.Invoke(this, args), null);
        }
        else
        {
            StateChanged?.Invoke(this, args);
        }
    }

    /// <summary>
    /// Full shutdown used on application exit. Unlike <see cref="DisconnectAsync"/>
    /// it always runs the teardown, even from the error state.
    /// </summary>
    public async Task ShutdownAsync()
    {
        _userInitiatedDisconnect = true;
        CancelCurrentSession();
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            await TeardownSessionAsync().ConfigureAwait(false);
            SetState(ConnectionState.NoDevice, "已退出", null);
        }
        finally
        {
            _gate.Release();
        }
    }

    public async ValueTask DisposeAsync()
    {
        if (_disposed)
        {
            return;
        }

        _disposed = true;
        _capture.AndroidConnected -= OnCaptureConnected;
        _discovery.DevicesChanged -= OnDevicesChanged;

        await ShutdownAsync().ConfigureAwait(false);
        _gate.Dispose();
    }
}
