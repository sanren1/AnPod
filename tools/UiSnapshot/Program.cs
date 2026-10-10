using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using AnPod.Core.Adb;
using AnPod.Core.Models;
using AnPod.Desktop;
using AnPod.Desktop.Services;
using AnPod.Desktop.ViewModels;

// UseWindowsForms + UseWPF makes these ambiguous; pin them to WPF, as the app does.
using Application = System.Windows.Application;
using Brush = System.Windows.Media.Brush;
using Size = System.Windows.Size;

namespace AnPod.Tools.UiSnapshot;

/// <summary>
/// Rasterises <see cref="MainWindow"/> so the layout can be reviewed without a
/// human sitting in front of it.
/// <para>
/// It builds the shipping window with the shipping view model, so what lands in
/// the PNG is what a user sees — including the states that are otherwise
/// impossible to reach on demand (empty list, streaming, paused). When a device
/// happens to be attached it drives the real connection, so the populated list
/// and the switch positions are rendered from real data rather than mocked.
/// </para>
/// </summary>
internal static class Program
{
    /// <summary>Matches MainWindow.Width.</summary>
    private const double Width = 380;

    private static Border? _host;

    [STAThread]
    private static int Main(string[] args)
    {
        var outputDirectory = args.Length > 0 ? args[0] : ".";
        Directory.CreateDirectory(outputDirectory);

        // Load the theme without touching the app shell. Instantiating
        // AnPod.Desktop.App is not an option: its generated entry point pairs
        // InitializeComponent with Run(), and merely constructing it drags the
        // real startup path (settings, discovery, auto-connect) into a tool that
        // is supposed to be inert.
        var app = new Application();
        app.Resources.MergedDictionaries.Add(new ResourceDictionary
        {
            Source = new Uri("pack://application:,,,/AnPod;component/Assets/Theme.xaml", UriKind.Absolute),
        });
        app.Resources["BoolToVisibility"] = new BooleanToVisibilityConverter();

        var dispatcher = Dispatcher.CurrentDispatcher;
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(dispatcher));

        // Debug level so the payload's own stderr lines ([android] ...) land in
        // the log — they are the only place a failed session explains itself.
        var logger = new FileLogger(AppPaths.LogDirectory, Core.Logging.LogLevel.Debug);
        logger.Info("UiSnapshot: rendering window states");

        // The settings file is shared with the real app, so a run of this tool
        // would otherwise auto-connect on top of the connection it is about to
        // make itself — two sessions racing for the same reverse-mapping name.
        // Forced in memory only; nothing here is saved back.
        var settings = new SettingsService(logger);
        settings.Load();
        settings.Current.AutoConnectLastDevice = false;

        // The transport is pinned too, and for the same reason: Load() reads the
        // user's real settings file, so leaving it alone makes every snapshot
        // depend on whichever mode that user last happened to pick — the USB
        // states would silently render as Wi-Fi ones. Snapshot 6 sets Wi-Fi
        // explicitly, so USB is the only correct starting point here.
        // In memory only; nothing is saved back.
        settings.Current.PreferredTransport = TransportKind.Usb;

        var adb = new AdbRunner(Path.Combine(AppContext.BaseDirectory, "adb.exe"), logger);
        var discovery = new DeviceDiscoveryService(adb, logger);
        var capture = new NativeAudioCapture(logger);
        var muter = new PlaybackEndpointMuter(logger);
        var wireless = new WirelessAdbService(adb, logger);

        var connection = new ConnectionManager(
            adb, capture, discovery, muter, wireless,
            Path.Combine(AppContext.BaseDirectory, "server"), logger)
        {
            AutoConnectLastDevice = false,
        };

        var viewModel = new MainViewModel(connection, discovery, settings, logger, adb, capture, wireless);
        Attach(viewModel);

        // 1. Before the first scan: the empty state, which is what a user sees
        //    on a machine with no phone attached.
        Render(Path.Combine(outputDirectory, "01-empty.png"));

        // 2. After a scan, with a phone attached but no session yet.
        var refresh = viewModel.RefreshDevicesAsync();
        Pump(dispatcher, refresh);
        Settle(dispatcher);
        Render(Path.Combine(outputDirectory, "02-idle.png"));

        var device = discovery.Devices.FirstOrDefault(d => d.IsConnectable);
        if (device is not null)
        {
            // 3. Connected and playing.
            viewModel.SelectedDevice = device;
            Settle(dispatcher);
            Pump(dispatcher, connection.ConnectAsync(device));
            Settle(dispatcher);
            Render(Path.Combine(outputDirectory, "03-streaming.png"));

            // 4. Connected but silent — the state this change exists to make possible.
            Pump(dispatcher, connection.SetAudioEnabledAsync(false));
            Settle(dispatcher);
            Render(Path.Combine(outputDirectory, "04-paused.png"));

            // 5. Disconnected again.
            Pump(dispatcher, connection.DisconnectAsync(userInitiated: true));
            Settle(dispatcher);
            Render(Path.Combine(outputDirectory, "05-disconnected.png"));
        }
        else
        {
            Console.WriteLine("No connectable device; the connected states were skipped.");
        }

        // 6-7. Wi-Fi mode. The transport is seeded through the settings object
        //      rather than the view model setter on purpose: the setter persists,
        //      and this tool must not rewrite the user's real settings file.
        settings.Current.PreferredTransport = TransportKind.Wifi;

        var wifiViewModel = new MainViewModel(
            connection, discovery, settings, logger, adb, capture, wireless);
        Attach(wifiViewModel);

        Settle(dispatcher);
        Render(Path.Combine(outputDirectory, "06-wifi.png"));

        // 8. The help window. It is a separate Window, so it gets the same
        //    content-host treatment rather than being skipped: it is a surface
        //    the user actually reads, and its text is long enough to overflow.
        RenderWindow(
            new WirelessHelpWindow { DataContext = wifiViewModel },
            420,
            Path.Combine(outputDirectory, "07-wifi-help.png"));

        Console.WriteLine("Done.");
        return 0;
    }

    /// <summary>
    /// Takes the window's content out of the window and into a standalone host.
    /// A Window that is never shown has no layout of its own, so the content is
    /// measured and arranged directly; the host supplies the window background
    /// (which lives on the Window, not on the content) so the PNG is not
    /// transparent.
    /// </summary>
    private static void Attach(MainViewModel viewModel)
    {
        var window = new MainWindow { DataContext = viewModel };
        var content = (FrameworkElement)window.Content;

        window.Content = null;
        content.DataContext = viewModel;

        _host = new Border
        {
            Width = Width,
            Background = (Brush)Application.Current.Resources["AppBackgroundBrush"],
            Child = content,
        };
    }

    /// <summary>
    /// Hosts a second window's content so it can be rasterised the same way.
    /// </summary>
    /// <remarks>
    /// The wireless help lives in its own <see cref="Window"/>, which a snapshot
    /// of MainWindow would never show. It is a surface the user actually reads —
    /// six paragraphs of text that can overflow — so it is worth the same check as
    /// the main window rather than being the one screen nobody looks at.
    /// </remarks>
    private static void RenderWindow(Window window, double width, string path)
    {
        var content = (FrameworkElement)window.Content;
        var dataContext = window.DataContext;

        window.Content = null;
        content.DataContext = dataContext;

        _host = new Border
        {
            Width = width,
            Background = (Brush)Application.Current.Resources["AppBackgroundBrush"],
            Child = content,
        };

        Render(path, width);
    }

    private static void Render(string path, double width = Width)
    {
        var host = _host!;

        host.Measure(new Size(width, double.PositiveInfinity));
        host.Arrange(new Rect(new Size(width, host.DesiredSize.Height)));
        host.UpdateLayout();

        int pixelWidth = (int)Math.Ceiling(host.ActualWidth);
        int pixelHeight = (int)Math.Ceiling(host.ActualHeight);

        var bitmap = new RenderTargetBitmap(pixelWidth, pixelHeight, 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(host);

        var encoder = new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(bitmap));

        using var stream = File.Create(path);
        encoder.Save(stream);

        Console.WriteLine($"wrote {Path.GetFileName(path)} ({pixelWidth}x{pixelHeight})");
    }

    /// <summary>
    /// Runs a task to completion while pumping the dispatcher. The view model
    /// posts every state change through the dispatcher, so without pumping the
    /// window would be rendered before any of it arrived.
    /// </summary>
    private static void Pump(Dispatcher dispatcher, Task task)
    {
        var frame = new DispatcherFrame();

        _ = task.ContinueWith(
            _ => dispatcher.BeginInvoke(DispatcherPriority.Normal, new Action(() => frame.Continue = false)),
            TaskScheduler.Default);

        Dispatcher.PushFrame(frame);
    }

    /// <summary>Drains queued dispatcher work so bindings are applied before rendering.</summary>
    private static void Settle(Dispatcher dispatcher)
    {
        for (int i = 0; i < 3; i++)
        {
            dispatcher.Invoke(() => { }, DispatcherPriority.ContextIdle);
        }
    }
}
