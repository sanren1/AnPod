using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using AnPod.Core.Adb;
using AnPod.Core.Logging;
using AnPod.Core.Models;
using AnPod.Desktop;
using AnPod.Desktop.Services;
using AnPod.Desktop.ViewModels;
using QRCoder;

using Application = System.Windows.Application;
using Brush = System.Windows.Media.Brush;
using Size = System.Windows.Size;

internal static class Program
{
    private const double MainWidth = 420;
    private const double MainHeight = 540;
    private static double Dpi = 96;
    private static Border? _host;
    private static Window? _mainWindow;
    private static FrameworkElement? _mainContent;
    private static double? _emptyComboWidth;

    [STAThread]
    private static int Main(string[] args)
    {
        var output = args.FirstOrDefault(arg => !arg.StartsWith("--", StringComparison.Ordinal)) ?? "artifacts/ui/snapshot";
        var dpiArgument = args.FirstOrDefault(arg => arg.StartsWith("--dpi=", StringComparison.Ordinal));
        if (dpiArgument is not null && double.TryParse(dpiArgument[6..], System.Globalization.CultureInfo.InvariantCulture, out var dpi))
            Dpi = dpi;
        Directory.CreateDirectory(output);

        var app = new Application();
        app.Resources.MergedDictionaries.Add(new ResourceDictionary
        {
            Source = new Uri("pack://application:,,,/AnPod;component/Assets/Theme.xaml", UriKind.Absolute),
        });
        app.Resources["BoolToVisibility"] = new BooleanToVisibilityConverter();
        var dispatcher = Dispatcher.CurrentDispatcher;
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(dispatcher));

        var logger = new FileLogger(Path.Combine(output, "logs"), LogLevel.Error);
        var settings = new SettingsService(logger);
        settings.Load();
        typeof(SettingsService).GetProperty(nameof(SettingsService.Current))!.SetValue(settings, new AppSettings());
        settings.Current.AutoConnectLastDevice = false;
        settings.Current.PreferredTransport = TransportKind.Usb;
        var adb = new AdbRunner(Path.Combine(AppContext.BaseDirectory, "adb.exe"), logger);
        var discovery = new DeviceDiscoveryService(adb, logger);
        var capture = new NativeAudioCapture(logger);
        var wireless = new WirelessAdbService(adb, logger);
        var connection = new ConnectionManager(adb, capture, discovery, new PlaybackEndpointMuter(logger), wireless,
            Path.Combine(AppContext.BaseDirectory, "server"), logger) { AutoConnectLastDevice = false };
        var viewModel = new MainViewModel(connection, discovery, settings, logger, adb, capture, wireless);

        var device = new DeviceInfo
        {
            Serial = "OFFLINE-USB-PREVIEW",
            State = AdbDeviceState.Device,
            Transport = TransportKind.Usb,
            Manufacturer = "示例品牌",
            Model = "USB 手机",
        };

        SetSnapshotDevices(viewModel);
        SetSnapshotState(viewModel, ConnectionState.NoDevice);
        RenderMain(viewModel, Path.Combine(output, "light-empty.png"));
        SetSnapshotDevices(viewModel, device);

        foreach (var (themeName, theme) in new[] { ("light", AppearanceTheme.Light), ("dark", AppearanceTheme.Dark) })
        {
            ThemeService.Apply(theme);
            SetSnapshotState(viewModel, ConnectionState.Ready);
            RenderMain(viewModel, Path.Combine(output, $"{themeName}-ready.png"));
            SetSnapshotState(viewModel, ConnectionState.Connecting);
            RenderMain(viewModel, Path.Combine(output, $"{themeName}-connecting.png"));
            SetSnapshotState(viewModel, ConnectionState.Connected);
            RenderMain(viewModel, Path.Combine(output, $"{themeName}-connected.png"));
            SetSnapshotState(viewModel, ConnectionState.Streaming);
            RenderMain(viewModel, Path.Combine(output, $"{themeName}-streaming.png"));
            SetSnapshotState(viewModel, ConnectionState.Reconnecting);
            RenderMain(viewModel, Path.Combine(output, $"{themeName}-reconnecting.png"));
            SetSnapshotState(viewModel, ConnectionState.Unauthorized);
            RenderMain(viewModel, Path.Combine(output, $"{themeName}-unauthorized.png"));
        }

        SetSnapshotDevices(viewModel);
        SetSnapshotState(viewModel, ConnectionState.NoDevice);
        ThemeService.Apply(AppearanceTheme.Light);
        SetSnapshotDevices(viewModel, device);

        foreach (var (themeName, theme) in new[] { ("light", AppearanceTheme.Light), ("dark", AppearanceTheme.Dark) })
        {
            ThemeService.Apply(theme);
            RenderWindow(new SettingsWindow { DataContext = viewModel }, 470, 360, Path.Combine(output, $"{themeName}-settings.png"));
            RenderWindow(new WirelessHelpWindow { DataContext = viewModel }, 520, 400, Path.Combine(output, $"{themeName}-help.png"));
            RenderWindow(new DiagnosticsWindow { DataContext = viewModel }, 560, 460, Path.Combine(output, $"{themeName}-diagnostics.png"));

            var qrWindow = new QrPairingWindow(viewModel);
            SetQrPreview(qrWindow);
            RenderWindow(qrWindow, 540, 680, Path.Combine(output, $"{themeName}-pair-wireless.png"));
        }

        connection.DisposeAsync().AsTask().GetAwaiter().GetResult();
        capture.Dispose();
        logger.Dispose();
        Console.WriteLine($"Wrote offline UI snapshots to {Path.GetFullPath(output)} at {Dpi:0} DPI.");
        return 0;
    }

    private static void RenderMain(MainViewModel viewModel, string path)
    {
        _mainWindow ??= new MainWindow { DataContext = viewModel };
        if (_mainContent is null)
        {
            _mainContent = (FrameworkElement)_mainWindow.Content;
            _mainWindow.Content = null;
            _mainContent.DataContext = viewModel;
            _host = new Border
            {
                Width = MainWidth,
                Height = MainHeight,
                Background = (Brush)Application.Current.Resources["AppBackgroundBrush"],
                Child = _mainContent,
            };
        }

        RenderHost(_host!, path);
        VerifyMainWindow(viewModel, _host!, Path.GetFileName(path));
    }

    private static void RenderWindow(Window window, double width, double height, string path)
    {
        var content = (FrameworkElement)window.Content;
        window.Content = null;
        content.DataContext = window.DataContext;
        _host = new Border
        {
            Width = width,
            Height = height,
            Background = (Brush)Application.Current.Resources["AppBackgroundBrush"],
            Child = content,
        };
        RenderHost(_host, path);
    }

    private static void RenderHost(Border host, string path)
    {
        host.Measure(new Size(host.Width, host.Height));
        host.Arrange(new Rect(0, 0, host.Width, host.Height));
        host.UpdateLayout();

        var bitmap = new RenderTargetBitmap((int)Math.Ceiling(host.Width * Dpi / 96), (int)Math.Ceiling(host.Height * Dpi / 96), Dpi, Dpi, PixelFormats.Pbgra32);
        bitmap.Render(host);
        var encoder = new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(bitmap));
        using var stream = File.Create(path);
        encoder.Save(stream);
        Console.WriteLine($"wrote {Path.GetFileName(path)} ({bitmap.PixelWidth}x{bitmap.PixelHeight})");
    }

    private static void VerifyMainWindow(MainViewModel viewModel, Border host, string snapshotName)
    {
        var root = (DependencyObject)host.Child!;
        var combo = FindVisualChildren<System.Windows.Controls.ComboBox>(root).Single();
        var disconnect = FindVisualChildren<System.Windows.Controls.Button>(root).Single(button => Equals(button.Content, "断开连接"));
        var scrollViewer = FindVisualChildren<ScrollViewer>(root).Single();
        var modeButtons = FindVisualChildren<System.Windows.Controls.RadioButton>(root)
            .Where(button => Equals(button.GroupName, "Transport") || Equals(button.GroupName, "PlaybackMode"))
            .ToArray();
        if (disconnect.IsEnabled != viewModel.IsDisconnectEnabled ||
            disconnect.Command?.CanExecute(disconnect.CommandParameter) != viewModel.IsDisconnectEnabled ||
            combo.ActualHeight < 60 ||
            combo.ActualWidth <= 0 ||
            modeButtons.Length != 4 ||
            modeButtons.Any(button => button.IsEnabled != viewModel.IsTransportSwitchEnabled && button.GroupName == "Transport") ||
            modeButtons.Any(button => button.IsEnabled != viewModel.IsPlaybackModeSwitchEnabled && button.GroupName == "PlaybackMode"))
        {
            throw new InvalidOperationException($"MainWindow binding verification failed for {snapshotName}: state={viewModel.State}, button={disconnect.IsEnabled}, command={disconnect.Command?.CanExecute(disconnect.CommandParameter)}, combo={combo.ActualWidth:0.##}x{combo.ActualHeight:0.##}.");
        }

        if (snapshotName == "light-empty.png")
        {
            _emptyComboWidth = combo.ActualWidth;
        }
        else if (snapshotName == "light-ready.png")
        {
            if (_emptyComboWidth is null || Math.Abs(_emptyComboWidth.Value - combo.ActualWidth) > 0.1)
            {
                throw new InvalidOperationException($"Device selector width changed between empty ({_emptyComboWidth:0.##}) and ready ({combo.ActualWidth:0.##}) states.");
            }

            Console.WriteLine($"verified live NoDevice → Ready → Connecting → Connected → Streaming → disconnect-to-Ready state bindings on one MainWindow; combo width={combo.ActualWidth:0.##}, scroll={scrollViewer.ComputedVerticalScrollBarVisibility}");
        }
    }

    private static IEnumerable<T> FindVisualChildren<T>(DependencyObject parent) where T : DependencyObject
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); index++)
        {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is T match)
            {
                yield return match;
            }

            foreach (var descendant in FindVisualChildren<T>(child))
            {
                yield return descendant;
            }
        }
    }

    private static void SetSnapshotDevices(MainViewModel viewModel, params DeviceInfo[] devices) =>
        typeof(MainViewModel).GetMethod("SyncDevices", BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(viewModel, new object[] { devices });

    private static void SetSnapshotState(MainViewModel viewModel, ConnectionState state) =>
        typeof(MainViewModel).GetProperty(nameof(MainViewModel.State))!.SetValue(viewModel, state);

    private static void SetQrPreview(QrPairingWindow window)
    {
        const string payload = "WIFI:T:ADB;S:AnPod-preview;P:offline-preview-only;;";
        using var generator = new QRCodeGenerator();
        using var data = generator.CreateQrCode(payload, QRCodeGenerator.ECCLevel.Q);
        var bytes = new PngByteQRCode(data).GetGraphic(8);
        using var stream = new MemoryStream(bytes);
        var bitmap = new BitmapImage();
        bitmap.BeginInit();
        bitmap.CacheOption = BitmapCacheOption.OnLoad;
        bitmap.StreamSource = stream;
        bitmap.EndInit();
        bitmap.Freeze();

        SetPrivateField(window, "QrImageControl", bitmap, "Source");
        SetPrivateField(window, "QrStatusText", "离线版式预览；此二维码不可配对。", "Text");
        SetPrivateField(window, "PairAddressText", "192.0.2.12:37123", "Text");
        SetPrivateField(window, "ConnectAddressText", "配对后自动发现", "Text");
    }

    private static void SetPrivateField(object target, string name, object value, string propertyName)
    {
        var field = target.GetType().GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException($"Control {name} not found");
        field.GetValue(target)!.GetType().GetProperty(propertyName)!.SetValue(field.GetValue(target), value);
    }
}
