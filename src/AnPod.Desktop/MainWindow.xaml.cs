using System.ComponentModel;
using System.Windows;
using AnPod.Desktop.Services;
using AnPod.Desktop.ViewModels;

namespace AnPod.Desktop;

public partial class MainWindow : Window
{
    private bool _forceClose;
    private WirelessHelpWindow? _helpWindow;

    public MainWindow()
    {
        InitializeComponent();

        // Set here rather than in XAML: the source is a static property, and a
        // {x:Static} binding to it would be resolved before the resource is
        // loaded in the designer.
        Icon = AppIcon.WindowIcon;

        // SizeToContent grows the window to its content, and the Wi-Fi panel is
        // taller than the work area on a small screen. Cap it to whatever is
        // actually available so the ScrollViewer takes over instead of the footer
        // being pushed past the bottom edge.
        MaxHeight = SystemParameters.WorkArea.Height;
    }

    /// <summary>
    /// Shows the wireless help. Handled here rather than through a command
    /// because the view model has no business owning a window, and the reused
    /// instance is what makes a second click bring the existing help forward
    /// instead of stacking another copy behind it.
    /// </summary>
    private void OnWifiHelpClick(object sender, RoutedEventArgs e)
    {
        if (_helpWindow is { IsLoaded: true })
        {
            _helpWindow.Activate();
            return;
        }

        // The main view model is the data context, so the hotspot button inside
        // the help reuses the command that already exists.
        _helpWindow = new WirelessHelpWindow { Owner = this, DataContext = DataContext };
        _helpWindow.Closed += (_, _) => _helpWindow = null;
        _helpWindow.ShowDialog();
    }

    /// <summary>Closes the window for real, bypassing the minimise-to-tray rule.</summary>
    public void ForceClose()
    {
        _forceClose = true;
        _helpWindow?.Close();
        Close();
    }

    protected override void OnClosing(CancelEventArgs e)
    {
        var minimizeToTray = (DataContext as MainViewModel)?.MinimizeToTray ?? true;

        // Plan §5.7: closing hides to the tray; only the tray "退出" ends the app.
        if (!_forceClose && minimizeToTray)
        {
            e.Cancel = true;
            Hide();
            return;
        }

        base.OnClosing(e);
    }
}
