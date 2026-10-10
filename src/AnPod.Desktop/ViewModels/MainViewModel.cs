using System.Collections.ObjectModel;
using System.Diagnostics;
using System.Reflection;
using System.Windows.Threading;
using AnPod.Core.Adb;
using AnPod.Core.Models;
using AnPod.Desktop.Services;

// UseWindowsForms + UseWPF makes Clipboard ambiguous; pin it to WPF.
using Clipboard = System.Windows.Clipboard;

namespace AnPod.Desktop.ViewModels;

/// <summary>
/// Presentation state for the single window (plan §5.7). It owns no ADB or
/// audio logic — it translates <see cref="ConnectionManager"/> state into the
/// few strings and command states the view binds to.
/// <para>
/// Two independent controls, deliberately not merged into one:
/// <list type="bullet">
///   <item><description><see cref="PrimaryCommand"/> — connect / disconnect the
///   phone (the session: reverse tunnel, payload, handshake).</description></item>
///   <item><description><see cref="TogglePhoneOutputCommand"/> — route the sound
///   through the phone or not. Turning it off keeps the session alive.</description></item>
/// </list>
/// </para>
/// </summary>
public sealed class MainViewModel : ObservableObject
{
    private readonly ConnectionManager _connection;
    private readonly DeviceDiscoveryService _discovery;
    private readonly SettingsService _settings;
    private readonly FileLogger _logger;
    private readonly AdbRunner _adb;
    private readonly NativeAudioCapture _capture;
    private readonly WirelessAdbService _wireless;
    private readonly Dispatcher _dispatcher;

    private DeviceInfo? _selectedDevice;
    private ConnectionState _state = ConnectionState.NoDevice;

    /// <summary>
    /// Every device ADB reported, before the transport filter.
    /// </summary>
    /// <remarks>
    /// Held so switching USB/Wi-Fi can re-filter immediately instead of waiting
    /// for the next scan: the mode switch is a pure view change, and making the
    /// user press 刷新 to see the other list would be a needless round trip.
    /// </remarks>
    private IReadOnlyList<DeviceInfo> _allDevices = Array.Empty<DeviceInfo>();
    // Deliberately terse. The empty-list panel already carries the "how to get a
    // device" instructions, and two long sentences saying the same thing made the
    // window look like it was shouting. This line only has to say what is true
    // right now.
    private string _statusMessage = "尚未连接";
    private bool _isSettingsOpen;
    private bool _isRefreshing;
    private bool _isWifiBusy;
    private string _wifiEndpoint = string.Empty;
    private string _pairingCode = string.Empty;
    private string _wifiStatusMessage = string.Empty;
    private string? _lastProblem;

    public MainViewModel(
        ConnectionManager connection,
        DeviceDiscoveryService discovery,
        SettingsService settings,
        FileLogger logger,
        AdbRunner adb,
        NativeAudioCapture capture,
        WirelessAdbService wireless)
    {
        _connection = connection;
        _discovery = discovery;
        _settings = settings;
        _logger = logger;
        _adb = adb;
        _capture = capture;
        _wireless = wireless;
        _dispatcher = Dispatcher.CurrentDispatcher;

        PrimaryCommand = new AsyncRelayCommand(ToggleSessionAsync, () => CanToggleSession);
        TogglePhoneOutputCommand = new AsyncRelayCommand(TogglePhoneOutputAsync, () => CanTogglePhoneOutput);
        RefreshCommand = new AsyncRelayCommand(RefreshDevicesAsync, () => CanRefresh);
        PairCommand = new AsyncRelayCommand(PairAsync, () => CanPairWifi);
        ConnectWifiCommand = new AsyncRelayCommand(ConnectWifiAsync, () => CanConnectWifi);
        DiscoverCommand = new AsyncRelayCommand(DiscoverAsync, () => CanDiscoverWifi);
        ForgetWifiCommand = new AsyncRelayCommand(ForgetWifiAsync, () => CanForgetWifi);
        OpenLogsCommand = new RelayCommand(OpenLogFolder);
        CopyDiagnosticsCommand = new RelayCommand(CopyDiagnostics);
        OpenSettingsFileLocationCommand = new RelayCommand(OpenSettingsFolder);
        ToggleSettingsCommand = new RelayCommand(() => IsSettingsOpen = !IsSettingsOpen);
        OpenHotspotSettingsCommand = new RelayCommand(OpenHotspotSettings);

        AutoConnectLastDevice = _settings.Current.AutoConnectLastDevice;
        MinimizeToTray = _settings.Current.MinimizeToTray;
        AutoReconnectWifi = _settings.Current.AutoReconnectWifi;

        // Seed the remembered address so a returning user only has to press
        // 「连接」 — plan §1.1a allows re-entry but should not require it.
        _wifiEndpoint = _settings.Current.LastWirelessEndpoint ?? string.Empty;

        // Seed the manager explicitly. The property setters above short-circuit
        // when the value already matches, and OnConnectionStateChanged only runs
        // after the first transition — so without this the manager would not
        // know the last device until something else changed state, and
        // auto-connect (gated on Ready + LastDeviceId) would silently never fire.
        _connection.AutoConnectLastDevice = _settings.Current.AutoConnectLastDevice;
        _connection.LastDeviceId = _settings.Current.LastDeviceId;
        _connection.AutoReconnectWifi = _settings.Current.AutoReconnectWifi;
        _connection.PreferredTransport = _settings.Current.PreferredTransport;
        _connection.LastWirelessEndpoint = _settings.Current.LastWirelessEndpoint;

        _connection.StateChanged += OnConnectionStateChanged;
        _discovery.DevicesChanged += OnDevicesChanged;
        _discovery.DiscoveryProblem += OnDiscoveryProblem;

        SyncDevices(_discovery.Devices);
    }

    // ------------------------------------------------------------- device list

    public ObservableCollection<DeviceInfo> Devices { get; } = new();

    public DeviceInfo? SelectedDevice
    {
        get => _selectedDevice;
        set
        {
            if (SetProperty(ref _selectedDevice, value))
            {
                RefreshCommands();
            }
        }
    }

    /// <summary>Drives the "no devices" panel; the list itself stays visible.</summary>
    public bool IsDeviceListEmpty => Devices.Count == 0;

    public bool HasDevices => Devices.Count > 0;

    /// <summary>
    /// The list is locked while a session exists: swapping the phone out from
    /// under a running session is not supported, so the control is disabled
    /// rather than silently ignoring the click.
    /// </summary>
    public bool IsDeviceListEnabled => IsIdle;

    /// <summary>Nothing is running, so the user is free to change how we connect.</summary>
    public bool IsIdle => !IsSessionUp && !IsBusy;

    /// <summary>
    /// Switching USB/Wi-Fi mid-session is refused, not silently ignored: plan §1.4
    /// requires the old channel to be stopped in order first, and the window only
    /// shows one mode at a time.
    /// </summary>
    public bool IsTransportSwitchEnabled => IsIdle;

    /// <summary>
    /// What to do about an empty device list. Depends on the mode, because "plug
    /// in a cable" is useless advice to someone who chose Wi-Fi (plan §5.10).
    /// </summary>
    public string EmptyDeviceHint => IsWifiMode
        ? "还没有设备。请在上面填写手机「无线调试」里的 IP:端口 并点「连接」；若提示需要配对，先点「配对」。"
        : "未检测到设备。请用 USB 数据线连接手机、在手机上允许 USB 调试，然后点「刷新」。";

    public bool IsRefreshing
    {
        get => _isRefreshing;
        private set
        {
            if (SetProperty(ref _isRefreshing, value))
            {
                OnPropertyChanged(nameof(RefreshButtonText));
                RefreshCommands();
            }
        }
    }

    public string RefreshButtonText => IsRefreshing ? "刷新中…" : "刷新";

    /// <summary>A refresh while connecting would race the handshake; block it.</summary>
    public bool CanRefresh => !IsRefreshing && !IsBusy;

    // ------------------------------------------------------------ status

    public ConnectionState State
    {
        get => _state;
        private set
        {
            if (SetProperty(ref _state, value))
            {
                OnPropertyChanged(nameof(StatusTitle));
                OnPropertyChanged(nameof(IsBusy));
                OnPropertyChanged(nameof(StatusBrushKey));
                OnPropertyChanged(nameof(IsPhoneOutputEnabled));
                OnPropertyChanged(nameof(CanTogglePhoneOutput));
                OnPropertyChanged(nameof(PhoneOutputHint));
                OnPropertyChanged(nameof(SessionActionText));
                OnPropertyChanged(nameof(IsDeviceListEnabled));
                OnPropertyChanged(nameof(IsIdle));
                OnPropertyChanged(nameof(IsTransportSwitchEnabled));
                OnPropertyChanged(nameof(IsWifiPanelEnabled));
                OnPropertyChanged(nameof(IsWifiInputEnabled));
                RefreshCommands();
            }
        }
    }

    public string StatusMessage
    {
        get => _statusMessage;
        private set => SetProperty(ref _statusMessage, value);
    }

    /// <summary>True while a session exists, with or without audio flowing.</summary>
    public bool IsSessionUp => State is ConnectionState.Connected or ConnectionState.Streaming;

    /// <summary>
    /// The "从手机播放声音" switch. True while audio is routed to the phone —
    /// including <see cref="ConnectionState.Connecting"/> and
    /// <see cref="ConnectionState.Reconnecting"/>, otherwise flipping the switch
    /// on would visually snap straight back off while the handshake runs and the
    /// control would look broken.
    /// </summary>
    public bool IsPhoneOutputEnabled =>
        State is ConnectionState.Streaming
            or ConnectionState.Connecting
            or ConnectionState.Reconnecting;

    /// <summary>
    /// The switch only means something once a session exists; before that it is
    /// disabled rather than hidden, so the layout does not jump when a phone is
    /// plugged in.
    /// </summary>
    public bool CanTogglePhoneOutput =>
        State is ConnectionState.Connected or ConnectionState.Streaming;

    /// <summary>
    /// One line of plain text under the switch. Required, not decoration: the
    /// switch position must never be the only thing carrying the state
    /// (ui-ux-pro-max: "don't convey information by colour alone").
    /// </summary>
    public string PhoneOutputHint => State switch
    {
        ConnectionState.Streaming => "声音从手机播放",
        ConnectionState.Connected => "已连接手机，声音仍从电脑播放",
        ConnectionState.Connecting => "正在连接手机…",
        ConnectionState.Reconnecting => "连接中断，正在重连…",
        ConnectionState.Unauthorized => "等待手机授权",
        _ => "连接手机后可切换到手机播放",
    };

    /// <summary>The connect / disconnect button's label.</summary>
    public string SessionActionText => State switch
    {
        ConnectionState.Connected or ConnectionState.Streaming => "断开连接",
        ConnectionState.Connecting => "正在连接…",
        ConnectionState.Reconnecting => "正在重连…",
        _ => "连接手机",
    };

    /// <summary>Headline shown next to the status dot.</summary>
    public string StatusTitle => State switch
    {
        ConnectionState.NoDevice => "未检测到设备",
        ConnectionState.Unauthorized => "等待授权",
        ConnectionState.Ready => "已就绪",
        ConnectionState.Connecting => "正在连接",
        ConnectionState.Connected => "已连接",
        ConnectionState.Streaming => "正在播放",
        ConnectionState.Reconnecting => "正在重连",
        ConnectionState.Error => "出错了",
        _ => "未知状态",
    };

    /// <summary>Key into the view's status brush dictionary; text always accompanies it.</summary>
    public string StatusBrushKey => State switch
    {
        ConnectionState.Connected or ConnectionState.Streaming => "StatusOkBrush",
        ConnectionState.Connecting or ConnectionState.Reconnecting => "StatusBusyBrush",
        ConnectionState.Error => "StatusErrorBrush",
        ConnectionState.Unauthorized => "StatusWarnBrush",
        _ => "StatusIdleBrush",
    };

    public bool IsBusy => State is ConnectionState.Connecting or ConnectionState.Reconnecting;

    /// <summary>
    /// Connect is offered when a usable device is picked; disconnect is offered
    /// whenever a session exists.
    /// </summary>
    public bool CanToggleSession =>
        IsSessionUp ||
        (State is ConnectionState.Ready or ConnectionState.NoDevice or ConnectionState.Error &&
         SelectedDevice?.IsConnectable == true);

    // ------------------------------------------------------------ settings

    public bool AutoConnectLastDevice
    {
        get => _settings.Current.AutoConnectLastDevice;
        set
        {
            if (_settings.Current.AutoConnectLastDevice == value)
            {
                return;
            }

            _settings.Current.AutoConnectLastDevice = value;
            _connection.AutoConnectLastDevice = value;
            _settings.Save(_settings.Current);
            OnPropertyChanged();
        }
    }

    public bool MinimizeToTray
    {
        get => _settings.Current.MinimizeToTray;
        set
        {
            if (_settings.Current.MinimizeToTray == value)
            {
                return;
            }

            _settings.Current.MinimizeToTray = value;
            _settings.Save(_settings.Current);
            OnPropertyChanged();
        }
    }

    public bool IsSettingsOpen
    {
        get => _isSettingsOpen;
        set => SetProperty(ref _isSettingsOpen, value);
    }

    /// <summary>
    /// Re-establish a dropped wireless link with <c>adb connect</c> before
    /// consulting the device list (plan §5.10). Only meaningful with Wi-Fi, and
    /// only alongside <see cref="AutoConnectLastDevice"/>.
    /// </summary>
    public bool AutoReconnectWifi
    {
        get => _settings.Current.AutoReconnectWifi;
        set
        {
            if (_settings.Current.AutoReconnectWifi == value)
            {
                return;
            }

            _settings.Current.AutoReconnectWifi = value;
            _connection.AutoReconnectWifi = value;
            _settings.Save(_settings.Current);
            OnPropertyChanged();
        }
    }

    public string VersionText => $"v{Assembly.GetExecutingAssembly().GetName().Version?.ToString(2) ?? "1.0"}";

    // ------------------------------------------------------- transport / wireless

    /// <summary>
    /// Which control path the window is showing (plan §5.10). This is a UI
    /// preference only: it never forces a connection, and picking Wi-Fi does not
    /// call <c>adb pair</c>/<c>adb connect</c> on its own (plan §1.3).
    /// </summary>
    public TransportKind SelectedTransport
    {
        get => _settings.Current.PreferredTransport;
        set
        {
            if (_settings.Current.PreferredTransport == value)
            {
                return;
            }

            _settings.Current.PreferredTransport = value;
            _connection.PreferredTransport = value;
            _settings.Save(_settings.Current);
            OnPropertyChanged();
            OnPropertyChanged(nameof(IsUsbMode));
            OnPropertyChanged(nameof(IsWifiMode));
            OnPropertyChanged(nameof(EmptyDeviceHint));

            // The device list belongs to the mode, so switching modes re-filters
            // it from the last scan rather than waiting for the next one.
            ApplyTransportFilter();
        }
    }

    /// <summary>Two-way bound to the USB radio button.</summary>
    public bool IsUsbMode
    {
        get => SelectedTransport == TransportKind.Usb;
        set
        {
            if (value)
            {
                SelectedTransport = TransportKind.Usb;
            }
        }
    }

    /// <summary>Two-way bound to the Wi-Fi radio button.</summary>
    public bool IsWifiMode
    {
        get => SelectedTransport == TransportKind.Wifi;
        set
        {
            if (value)
            {
                SelectedTransport = TransportKind.Wifi;
            }
        }
    }

    /// <summary>
    /// The <c>ip:port</c> the user is about to pair with or connect to. Kept as
    /// free text on purpose: mDNS discovery is only a convenience, and plan §5.10
    /// requires a manual fallback that always works.
    /// </summary>
    public string WifiEndpoint
    {
        get => _wifiEndpoint;
        set
        {
            if (SetProperty(ref _wifiEndpoint, value))
            {
                RefreshWifiCommands();
            }
        }
    }

    /// <summary>
    /// The 6-digit pairing code from the phone's dialog. Held in memory only —
    /// never written to settings, logs or diagnostics (plan §5.9).
    /// </summary>
    public string PairingCode
    {
        get => _pairingCode;
        set
        {
            if (SetProperty(ref _pairingCode, value))
            {
                RefreshWifiCommands();
            }
        }
    }

    /// <summary>Result of the last pair/connect attempt, in the user's words.</summary>
    public string WifiStatusMessage
    {
        get => _wifiStatusMessage;
        private set => SetProperty(ref _wifiStatusMessage, value);
    }

    /// <summary>
    /// The Wi-Fi controls are locked while a session exists: pairing in the
    /// middle of a running audio path would change the device list under it
    /// (plan §1.4 — one active session, switched in order).
    /// </summary>
    public bool IsWifiPanelEnabled => !IsSessionUp && !IsBusy && !_isWifiBusy;

    public bool CanPairWifi => IsWifiPanelEnabled && !string.IsNullOrWhiteSpace(WifiEndpoint);

    /// <summary>
    /// Connect needs an address too, but not a code — pairing is a one-time step
    /// and the phone stays paired afterwards.
    /// </summary>
    public bool CanConnectWifi => IsWifiPanelEnabled && !string.IsNullOrWhiteSpace(WifiEndpoint);

    /// <summary>
    /// Search needs neither an address nor a code: it is how the user finds out
    /// what the address should be in the first place.
    /// </summary>
    public bool CanDiscoverWifi => IsWifiPanelEnabled;

    /// <summary>Offered only when there is a remembered link to release.</summary>
    public bool CanForgetWifi =>
        IsWifiPanelEnabled && !string.IsNullOrWhiteSpace(_connection.LastWirelessEndpoint);

    /// <summary>
    /// Whether the Wi-Fi text boxes and the reconnect checkbox accept input.
    /// </summary>
    /// <remarks>
    /// Deliberately weaker than <see cref="IsWifiPanelEnabled"/>, which also
    /// means "an operation is in flight". A button must go dead while a pair,
    /// connect or search runs; an input box must not. Greying the address and
    /// code boxes out is what made them look broken — the code is normally typed
    /// while an earlier step is still running, because the phone only shows it
    /// once its pairing dialog is open. Only a live session locks them, since
    /// the address then belongs to a tunnel that is already up (plan §1.4).
    /// </remarks>
    public bool IsWifiInputEnabled => !IsSessionUp;

    /// <summary>True while a pair/connect/release call is in flight.</summary>
    public bool IsWifiBusy
    {
        get => _isWifiBusy;
        private set
        {
            if (SetProperty(ref _isWifiBusy, value))
            {
                // Only the panel — the input boxes are deliberately left alone
                // while an operation runs (see IsWifiInputEnabled).
                OnPropertyChanged(nameof(IsWifiPanelEnabled));
                RefreshWifiCommands();
            }
        }
    }

    // ------------------------------------------------------------ commands

    /// <summary>Connect / disconnect the phone. The tray entry shares this.</summary>
    public AsyncRelayCommand PrimaryCommand { get; }

    /// <summary>Route the sound through the phone, without dropping the session.</summary>
    public AsyncRelayCommand TogglePhoneOutputCommand { get; }

    /// <summary>Re-read the ADB device list. The only thing that scans when idle.</summary>
    public AsyncRelayCommand RefreshCommand { get; }

    /// <summary>One-time <c>adb pair</c> with the phone's 6-digit code.</summary>
    public AsyncRelayCommand PairCommand { get; }

    /// <summary><c>adb connect</c> to the entered address.</summary>
    public AsyncRelayCommand ConnectWifiCommand { get; }

    /// <summary>Asks ADB which wireless-debugging phones are on the local network.</summary>
    public AsyncRelayCommand DiscoverCommand { get; }

    /// <summary>Releases the remembered wireless endpoint from the ADB server.</summary>
    public AsyncRelayCommand ForgetWifiCommand { get; }

    public RelayCommand OpenLogsCommand { get; }

    public RelayCommand CopyDiagnosticsCommand { get; }

    public RelayCommand OpenSettingsFileLocationCommand { get; }

    public RelayCommand ToggleSettingsCommand { get; }

    /// <summary>
    /// Opens the Windows mobile-hotspot settings page (plan §5.10: guide the
    /// user, never configure the hotspot for them). The URI is an official
    /// supported one, but a machine without hotspot hardware may not have the
    /// page — hence the silent fallback. Bound by the help window, which is where
    /// the hotspot guidance lives.
    /// </summary>
    public RelayCommand OpenHotspotSettingsCommand { get; }

    /// <summary>
    /// One scan of the device list. Public because startup calls it once so the
    /// window is not empty on first paint (plan §5.2).
    /// </summary>
    public async Task RefreshDevicesAsync()
    {
        if (IsRefreshing)
        {
            return;
        }

        IsRefreshing = true;
        try
        {
            await _discovery.RefreshAsync(CancellationToken.None);
        }
        catch (Exception ex)
        {
            _logger.Warn($"Device refresh failed: {ex.Message}");
            StatusMessage = "刷新设备失败，可查看日志了解详情。";
        }
        finally
        {
            IsRefreshing = false;
        }
    }

    private async Task ConnectAsync()
    {
        var device = SelectedDevice;
        if (device is null || !device.IsConnectable)
        {
            return;
        }

        await _connection.ConnectAsync(device);
    }

    private async Task ToggleSessionAsync()
    {
        if (IsSessionUp)
        {
            await _connection.DisconnectAsync(userInitiated: true);
        }
        else
        {
            await ConnectAsync();
        }
    }

    private Task TogglePhoneOutputAsync() =>
        _connection.SetAudioEnabledAsync(!IsPhoneOutputEnabled);

    // ------------------------------------------------------------ wireless actions

    /// <summary>
    /// <c>adb pair</c> with the code from the phone's dialog (plan §5.9).
    /// </summary>
    /// <remarks>
    /// Pairing and connecting are two separate steps and the window says so:
    /// pairing only exchanges keys, it does not open the audio path. The code is
    /// cleared once it has been accepted, because it is single-use and expiring —
    /// keeping it on screen would invite a confusing retry with a dead code.
    /// <para>
    /// A successful pair is immediately followed by a discovery pass. That is the
    /// whole point of pairing from AnPod's side: the phone is advertising its
    /// connect port on the network, so the user should never have to copy it off
    /// the phone's screen and risk pasting the pairing port instead.
    /// </para>
    /// </remarks>
    private async Task PairAsync()
    {
        bool paired;

        IsWifiBusy = true;
        try
        {
            WifiStatusMessage = "正在配对…";
            var result = await _wireless.PairAsync(WifiEndpoint, PairingCode, CancellationToken.None);
            WifiStatusMessage = result.Message;
            paired = result.Success;

            if (result.Endpoint is not null)
            {
                WifiEndpoint = result.Endpoint;
            }

            if (paired)
            {
                PairingCode = string.Empty;
            }
        }
        finally
        {
            IsWifiBusy = false;
        }

        if (paired)
        {
            await DiscoverAsync();
        }
    }

    /// <summary>
    /// Asks ADB which wireless-debugging phones are advertising themselves on the
    /// local network, and fills in the connect address when the answer is
    /// unambiguous (plan §5.10).
    /// </summary>
    private async Task DiscoverAsync()
    {
        IsWifiBusy = true;
        try
        {
            // The address box stays editable while this runs, so remember what it
            // held: if the user types their own address in the meantime, the
            // search result must not overwrite it.
            var addressBeforeSearch = WifiEndpoint;

            WifiStatusMessage = "正在搜索局域网设备…";

            var discovery = await _wireless.DiscoverAsync(CancellationToken.None);
            if (!discovery.Available || discovery.Connect.Count == 0)
            {
                WifiStatusMessage = discovery.Message;
                return;
            }

            var preferredHost = WirelessAdb.HostOf(_wireless.LastPairingEndpoint);
            var picked = WirelessAdb.PickConnectEndpoint(discovery.Connect, preferredHost);
            var addressUnchanged = string.Equals(WifiEndpoint, addressBeforeSearch, StringComparison.Ordinal);

            if (picked is not null)
            {
                if (addressUnchanged)
                {
                    WifiEndpoint = picked;
                    WifiStatusMessage = discovery.Connect.Count == 1
                        ? $"已找到设备，连接地址已填好：{picked}。点「连接」继续。"
                        : $"已找到 {discovery.Connect.Count} 个设备，连接地址填好了刚配对的那台：{picked}。";
                }
                else
                {
                    // The user typed an address while the search ran, so report the
                    // finding and leave their text alone — silently replacing what
                    // someone is mid-way through typing is never the right call.
                    WifiStatusMessage = $"搜索到设备：{picked}（你已手动改过地址栏，没有覆盖）。";
                }

                return;
            }

            // Several phones and no way to tell which one the user means. Showing
            // the list and letting them choose beats guessing: a wrong silent
            // choice would send audio to someone else's phone (plan §6).
            WifiStatusMessage =
                $"找到 {discovery.Connect.Count} 个设备，请把要用的那个填进地址栏：{string.Join("、", discovery.Connect.Select(s => s.Endpoint))}";
        }
        finally
        {
            IsWifiBusy = false;
        }
    }

    /// <summary>
    /// <c>adb connect</c> to the entered address, then re-scan so the device
    /// appears in the list and the normal connect button can be used.
    /// </summary>
    private async Task ConnectWifiAsync()
    {
        // Caught before spending a 10-second timeout on it: pasting the pairing
        // address is the single most common way this step fails, and the two
        // ports look equally plausible on the phone's screen. Rather than just
        // refusing, go and fetch the right one.
        if (WirelessAdb.TryNormalizeEndpoint(WifiEndpoint, out var normalized, out _) &&
            string.Equals(normalized, _wireless.LastPairingEndpoint, StringComparison.Ordinal))
        {
            WifiStatusMessage = "这是配对用的端口，不能用来连接。正在自动搜索连接端口…";
            await DiscoverAsync();
            return;
        }

        IsWifiBusy = true;
        try
        {
            WifiStatusMessage = "正在连接…";
            var result = await _wireless.ConnectAsync(WifiEndpoint, CancellationToken.None);
            WifiStatusMessage = result.Message;

            if (result.Endpoint is not null)
            {
                WifiEndpoint = result.Endpoint;

                // Only remember an address that actually worked; a typo must not
                // become the thing the next launch retries (plan §1.1a).
                if (result.Success)
                {
                    _connection.RememberWirelessEndpoint(result.Endpoint);
                }
            }
        }
        finally
        {
            IsWifiBusy = false;
        }

        // The device list only changes on a scan, so without this the freshly
        // connected phone would not show up until the user pressed 「刷新」.
        await RefreshDevicesAsync();
    }

    /// <summary>Drops the remembered wireless link and forgets its address.</summary>
    private async Task ForgetWifiAsync()
    {
        IsWifiBusy = true;
        try
        {
            await _connection.DisconnectWirelessAsync();
            WifiEndpoint = string.Empty;
            WifiStatusMessage = "已断开无线连接，手机不再出现在设备列表中。";
        }
        finally
        {
            IsWifiBusy = false;
        }

        await RefreshDevicesAsync();
    }

    private void OpenHotspotSettings()
    {
        try
        {
            Process.Start(new ProcessStartInfo("ms-settings:network-mobilehotspot")
            {
                UseShellExecute = true,
            });
        }
        catch (Exception)
        {
            // No hotspot hardware, or the settings page is missing on this build.
            WifiStatusMessage = "无法打开系统热点设置，请在「设置 → 网络和 Internet → 移动热点」中手动开启。";
        }
    }

    // ------------------------------------------------------------ events

    private void OnConnectionStateChanged(object? sender, ConnectionStateChangedEventArgs e)
    {
        State = e.State;
        StatusMessage = e.Message;

        _connection.AutoConnectLastDevice = _settings.Current.AutoConnectLastDevice;
        _connection.LastDeviceId = _settings.Current.LastDeviceId;
        _connection.AutoReconnectWifi = _settings.Current.AutoReconnectWifi;
        _connection.PreferredTransport = _settings.Current.PreferredTransport;

        // The endpoint is not re-seeded here on purpose: a session may have just
        // written a newer one, and clobbering it with the stored value would undo
        // the very thing that makes reconnect work.
        RefreshWifiCommands();
    }

    private void OnDevicesChanged(object? sender, IReadOnlyList<DeviceInfo> devices) =>
        _dispatcher.BeginInvoke(() => SyncDevices(devices));

    private void OnDiscoveryProblem(object? sender, string message)
    {
        _lastProblem = message;
        _dispatcher.BeginInvoke(() => StatusMessage = message);
    }

    private void SyncDevices(IReadOnlyList<DeviceInfo> devices)
    {
        _allDevices = devices;
        ApplyTransportFilter();
    }

    /// <summary>
    /// Rebuilds the visible list from the last scan, keeping only the devices
    /// that belong to the mode the window is showing.
    /// </summary>
    /// <remarks>
    /// <para>
    /// One phone can be reachable two ways at once and therefore appears twice in
    /// <c>adb devices</c> — once under its USB serial, once as <c>ip:port</c>.
    /// Showing both rows in both modes is what made the two modes look like one
    /// screen: choosing Wi-Fi still presented "vivo V2243A 已连接 USB", and the
    /// only way to tell which row a click would act on was the row's own label.
    /// Each mode now lists its own devices, so the list can never offer a device
    /// the current mode cannot use (plan §1.4 — the mode decides the transport).
    /// </para>
    /// <para>
    /// A device whose transport is still unclassified is shown in <em>both</em>
    /// modes on purpose. Hiding it would leave an empty list and "plug in a
    /// cable" advice while a phone is plainly attached, and the row already says
    /// "连接方式待确认" — which is the honest answer, not a mode.
    /// </para>
    /// </remarks>
    private void ApplyTransportFilter()
    {
        var previousSerial = SelectedDevice?.Serial;

        var visible = _allDevices.Where(d => d.MatchesTransport(SelectedTransport)).ToList();

        Devices.Clear();
        foreach (var device in visible)
        {
            Devices.Add(device);
        }

        // Selection is per-mode too. A device from the other mode must not stay
        // selected, or 连接手机 would act on something the user cannot see.
        var connectable = visible.Where(d => d.IsConnectable).ToList();

        SelectedDevice = visible.FirstOrDefault(d => d.Serial == previousSerial)
            ?? (connectable.Count == 1 ? connectable[0] : null);

        OnPropertyChanged(nameof(IsDeviceListEmpty));
        OnPropertyChanged(nameof(HasDevices));
        RefreshCommands();
    }

    private void RefreshCommands()
    {
        PrimaryCommand.RaiseCanExecuteChanged();
        TogglePhoneOutputCommand.RaiseCanExecuteChanged();
        RefreshCommand.RaiseCanExecuteChanged();
        RefreshWifiCommands();
    }

    private void RefreshWifiCommands()
    {
        PairCommand.RaiseCanExecuteChanged();
        ConnectWifiCommand.RaiseCanExecuteChanged();
        DiscoverCommand.RaiseCanExecuteChanged();
        ForgetWifiCommand.RaiseCanExecuteChanged();
    }

    // ------------------------------------------------------------ actions

    private void OpenLogFolder() => OpenInExplorer(_logger.Directory);

    private void OpenSettingsFolder() => OpenInExplorer(AppPaths.Root);

    private static void OpenInExplorer(string path)
    {
        try
        {
            Process.Start(new ProcessStartInfo { FileName = path, UseShellExecute = true });
        }
        catch (Exception)
        {
            // Opening a folder is a convenience; never surface a crash for it.
        }
    }

    /// <summary>
    /// Copies a short, redaction-safe diagnostic block (plan §5.8). No serials,
    /// tokens or audio data are included.
    /// </summary>
    private void CopyDiagnostics()
    {
        var text = string.Join(
            Environment.NewLine,
            "AnPod diagnostics",
            $"version: {VersionText}",
            $"state: {State}",
            $"status: {StatusMessage}",
            $"transport: {SelectedTransport}",
            $"devices: {Devices.Count}",
            $"adb present: {_adb.AdbPresent}",
            $"native capture loaded: {_capture.IsAvailable}",
            $"log folder: {_logger.Directory}",
            _lastProblem is null ? string.Empty : $"last problem: {_lastProblem}");

        try
        {
            Clipboard.SetText(text);
            StatusMessage = "诊断信息已复制";
        }
        catch (Exception)
        {
            StatusMessage = "无法写入剪贴板";
        }
    }
}
