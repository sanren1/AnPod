using System.IO;
using System.Threading;
using System.Windows;
using System.Windows.Threading;
using Microsoft.Win32;
using AnPod.Core.Logging;
using AnPod.Core.Models;
using AnPod.Desktop.Services;
using AnPod.Desktop.ViewModels;

// UseWindowsForms + UseWPF makes these two names ambiguous; pin them to WPF.
using Application = System.Windows.Application;
using MessageBox = System.Windows.MessageBox;

namespace AnPod.Desktop;

/// <summary>
/// Application shell: single-instance guard, composition root, tray wiring and
/// the one code path that actually exits (plan §5.7).
/// </summary>
public partial class App : Application
{
    // Local\ scopes both objects to the current session, which is what a
    // per-user tray utility wants.
    private const string MutexName = @"Local\AnPod.SingleInstance";
    private const string ShowWindowEventName = @"Local\AnPod.ShowWindow";

    private Mutex? _instanceMutex;
    private EventWaitHandle? _showWindowEvent;
    private CancellationTokenSource? _signalCts;

    private FileLogger? _logger;
    private SettingsService? _settings;
    private AdbRunner? _adb;
    private DeviceDiscoveryService? _discovery;
    private NativeAudioCapture? _capture;
    private PlaybackEndpointMuter? _endpointMuter;
    private WirelessAdbService? _wireless;
    private ConnectionManager? _connection;
    private TrayService? _tray;
    private MainWindow? _window;
    private bool _exiting;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        _instanceMutex = new Mutex(initiallyOwned: true, MutexName, out bool createdNew);
        if (!createdNew)
        {
            // A second launch just surfaces the existing window (plan §5.7).
            SignalExistingInstance();
            Shutdown();
            return;
        }

        // Guarantee a dispatcher-bound context before any service captures it.
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(Dispatcher));

        DispatcherUnhandledException += OnDispatcherUnhandledException;

        try
        {
            Compose();
        }
        catch (Exception ex)
        {
            _logger?.Error("Startup failed", ex);
            MessageBox.Show(
                $"AnPod 启动失败：{ex.Message}",
                "AnPod",
                MessageBoxButton.OK,
                MessageBoxImage.Error);
            Shutdown();
        }
    }

    private void Compose()
    {
        _logger = new FileLogger(AppPaths.LogDirectory, ResolveMinimumLogLevel());
        _logger.Info("AnPod starting");

        _settings = new SettingsService(_logger);
        _settings.Load();
        ThemeService.Apply(_settings.Current.AppearanceTheme);
        SystemEvents.UserPreferenceChanged += OnUserPreferenceChanged;

        _adb = new AdbRunner(Path.Combine(AppPaths.ApplicationDirectory, "adb.exe"), _logger);
        _discovery = new DeviceDiscoveryService(_adb, _logger);
        _capture = new NativeAudioCapture(_logger);
        _endpointMuter = new PlaybackEndpointMuter(_logger);

        // Before anything else: undo a mute left by a run that never shut down.
        _endpointMuter.RecoverFromPreviousRun();

        // Last-resort net. The normal paths all restore through the session
        // teardown, but a crash or a kill would otherwise leave the user's
        // speakers muted with nothing on screen explaining why.
        AppDomain.CurrentDomain.ProcessExit += (_, _) => _endpointMuter?.Restore();

        var payloadPath = Path.Combine(AppPaths.ApplicationDirectory, "server");

        _wireless = new WirelessAdbService(_adb, _logger);

        _connection = new ConnectionManager(
            _adb,
            _capture,
            _discovery,
            _endpointMuter,
            _wireless,
            payloadPath,
            _logger)
        {
            AutoConnectLastDevice = _settings.Current.AutoConnectLastDevice,
            LastDeviceId = _settings.Current.LastDeviceId,
            AutoReconnectWifi = _settings.Current.AutoReconnectWifi,
            PreferredTransport = _settings.Current.PreferredTransport,
            LastWirelessEndpoint = _settings.Current.LastWirelessEndpoint,
        };
        _connection.LastDeviceIdChanged += OnLastDeviceIdChanged;
        _connection.LastWirelessEndpointChanged += OnLastWirelessEndpointChanged;

        var viewModel = new MainViewModel(
            _connection,
            _discovery,
            _settings,
            _logger,
            _adb,
            _capture,
            _wireless);

        _window = new MainWindow { DataContext = viewModel };
        MainWindow = _window;

        _tray = new TrayService();
        _tray.ShowRequested += (_, _) => ShowMainWindow();
        _tray.PrimaryActionRequested += (_, _) => viewModel.PrimaryCommand.Execute(null);
        _tray.DisconnectRequested += (_, _) => viewModel.DisconnectCommand.Execute(null);
        _tray.SettingsRequested += (_, _) => _window.ShowSettings();
        _tray.ExitRequested += (_, _) => ExitApplication();
        _connection.StateChanged += (_, args) => _tray.UpdateStatus(args.State, args.Message, viewModel.PrimaryActionText);
        _tray.UpdateStatus(viewModel.State, viewModel.StatusMessage, viewModel.PrimaryActionText);

        ReportMissingDependencies();

        _window.Show();

        // One scan at startup so the device list is populated and auto-connect
        // has something to match against. After this nothing polls until the
        // user asks or a session is running (plan §5.2).
        _ = viewModel.RefreshDevicesAsync();

        StartShowSignalListener();
    }

    /// <summary>
    /// Log threshold, overridable with <c>ANPOD_LOG_LEVEL=debug</c>. Verbose
    /// mode is what surfaces the Android payload's own stderr lines
    /// (<c>[android] ...</c>), which are the only place playback failures are
    /// explained — without it a broken session just looks like a reconnect loop.
    /// </summary>
    private static LogLevel ResolveMinimumLogLevel()
    {
        var configured = Environment.GetEnvironmentVariable("ANPOD_LOG_LEVEL");
        return Enum.TryParse<LogLevel>(configured, ignoreCase: true, out var level)
            ? level
            : LogLevel.Info;
    }

    /// <summary>
    /// Logs (does not block on) missing runtime pieces. The UI stays usable so
    /// the user can read the status line and open the log folder.
    /// </summary>
    private void ReportMissingDependencies()
    {
        if (_adb is null || !_adb.AdbPresent)
        {
            _logger?.Warn("adb.exe not found next to the executable; device discovery will not work");
        }

        if (_capture is { IsAvailable: false })
        {
            _logger?.Warn($"Native capture unavailable: {_capture.LoadError}");
        }
    }

    private void OnLastDeviceIdChanged(object? sender, string? serial)
    {
        if (_settings is null)
        {
            return;
        }

        _settings.Current.LastDeviceId = serial;
        _settings.Save(_settings.Current);
    }

    /// <summary>
    /// Persists the last wireless endpoint that worked (plan §1.1a). Only the
    /// address is stored — never the pairing code, which lives in memory for the
    /// length of one <c>adb pair</c> call (plan §5.9).
    /// </summary>
    private void OnLastWirelessEndpointChanged(object? sender, string? endpoint)
    {
        if (_settings is null)
        {
            return;
        }

        _settings.Current.LastWirelessEndpoint = endpoint;
        _settings.Save(_settings.Current);
    }

    private void StartShowSignalListener()
    {
        try
        {
            _showWindowEvent = new EventWaitHandle(false, EventResetMode.AutoReset, ShowWindowEventName);
        }
        catch (Exception ex)
        {
            _logger?.Debug($"Could not create show-window event: {ex.Message}");
            return;
        }

        _signalCts = new CancellationTokenSource();
        var token = _signalCts.Token;
        var handle = _showWindowEvent;

        _ = Task.Run(() =>
        {
            while (!token.IsCancellationRequested)
            {
                if (!handle.WaitOne(500))
                {
                    continue;
                }

                Dispatcher.BeginInvoke(ShowMainWindow);
            }
        });
    }

    private static void SignalExistingInstance()
    {
        try
        {
            using var handle = EventWaitHandle.OpenExisting(ShowWindowEventName);
            handle.Set();
        }
        catch (WaitHandleCannotBeOpenedException)
        {
            // The owner is mid-startup; nothing to surface yet.
        }
        catch (UnauthorizedAccessException)
        {
        }
    }

    private void ShowMainWindow()
    {
        if (_window is null)
        {
            return;
        }

        _window.Show();

        if (_window.WindowState == WindowState.Minimized)
        {
            _window.WindowState = WindowState.Normal;
        }

        _window.Activate();

        // Briefly forcing top-most is the reliable way to pull a background
        // window to the foreground without stealing focus permanently.
        _window.Topmost = true;
        _window.Topmost = false;
        _window.Focus();
    }

    /// <summary>
    /// The only exit path. Stops capture, removes AnPod's own reverse mapping,
    /// kills the Android shell, then shuts down (plan §1.1.7/§5.7).
    /// </summary>
    public void RequestExit() => ExitApplication();

    public void ShowTrayHint() => _tray?.ShowHideHint();

    private void OnUserPreferenceChanged(object? sender, UserPreferenceChangedEventArgs e)
    {
        if (_settings?.Current.AppearanceTheme == AppearanceTheme.System)
            Dispatcher.BeginInvoke(() => ThemeService.Apply(AppearanceTheme.System));
    }

    private async void ExitApplication()
    {
        if (_exiting)
        {
            return;
        }

        _exiting = true;

        try
        {
            _signalCts?.Cancel();

            if (_connection is not null)
            {
                await _connection.DisposeAsync();
            }

            _capture?.Dispose();
        }
        catch (Exception ex)
        {
            _logger?.Error("Shutdown raised", ex);
        }

        _tray?.Dispose();
        SystemEvents.UserPreferenceChanged -= OnUserPreferenceChanged;
        _window?.ForceClose();

        _logger?.Info("AnPod exited");
        _logger?.Dispose();

        try
        {
            _instanceMutex?.ReleaseMutex();
        }
        catch (ApplicationException)
        {
        }

        _instanceMutex?.Dispose();
        _showWindowEvent?.Dispose();

        Shutdown();
    }

    private void OnDispatcherUnhandledException(object sender, DispatcherUnhandledExceptionEventArgs e)
    {
        _logger?.Error("Unhandled UI exception", e.Exception);

        // Keep the app alive: a rendering/logging hiccup must not kill an
        // in-progress audio session.
        e.Handled = true;
    }
}
