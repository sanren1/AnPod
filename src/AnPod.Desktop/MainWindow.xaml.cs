using System.ComponentModel;
using System.Windows;
using AnPod.Desktop.Services;
using AnPod.Desktop.ViewModels;

namespace AnPod.Desktop;

public partial class MainWindow : Window
{
    private bool _forceClose;
    private bool _trayHintShown;
    private WirelessHelpWindow? _helpWindow;
    private QrPairingWindow? _addPhoneWindow;
    private SettingsWindow? _settingsWindow;
    private DiagnosticsWindow? _diagnosticsWindow;

    public MainWindow()
    {
        InitializeComponent();
        Icon = AppIcon.WindowIcon;
        DataContextChanged += OnDataContextChanged;
    }

    public void ShowHelp()
    {
        if (_helpWindow is { IsLoaded: true }) { _helpWindow.Activate(); return; }
        _helpWindow = new WirelessHelpWindow { Owner = this, DataContext = DataContext };
        _helpWindow.Closed += (_, _) => { _helpWindow = null; FocusMain(); };
        _helpWindow.ShowDialog();
    }

    public void ShowAddPhone()
    {
        if (DataContext is not MainViewModel viewModel) return;
        if (_addPhoneWindow is { IsLoaded: true }) { _addPhoneWindow.Activate(); return; }
        _addPhoneWindow = new QrPairingWindow(viewModel) { Owner = this };
        _addPhoneWindow.Closed += (_, _) => { _addPhoneWindow = null; AddPhoneButton.Focus(); };
        _addPhoneWindow.ShowDialog();
    }

    private void OnSettingsClick(object sender, RoutedEventArgs e)
    {
        ShowSettings();
    }

    public void ShowSettings()
    {
        if (_settingsWindow is { IsLoaded: true }) { _settingsWindow.Activate(); return; }
        _settingsWindow = new SettingsWindow { Owner = this, DataContext = DataContext };
        _settingsWindow.Closed += (_, _) => { _settingsWindow = null; FocusMain(); };
        _settingsWindow.ShowDialog();
    }

    private void OnDiagnosticsClick(object sender, RoutedEventArgs e)
    {
        if (_diagnosticsWindow is { IsLoaded: true }) { _diagnosticsWindow.Activate(); return; }
        _diagnosticsWindow = new DiagnosticsWindow { Owner = this, DataContext = DataContext };
        _diagnosticsWindow.Closed += (_, _) => { _diagnosticsWindow = null; FocusMain(); };
        _diagnosticsWindow.ShowDialog();
    }

    private void OnHelpClick(object sender, RoutedEventArgs e) => ShowHelp();

    private void OnAddPhoneClick(object sender, RoutedEventArgs e) => ShowAddPhone();

    private void OnDataContextChanged(object sender, DependencyPropertyChangedEventArgs e)
    {
        if (e.OldValue is MainViewModel oldViewModel)
        {
            oldViewModel.HelpRequested -= OnHelpRequested;
            oldViewModel.AddPhoneRequested -= OnAddPhoneRequested;
        }
        if (e.NewValue is MainViewModel newViewModel)
        {
            newViewModel.HelpRequested += OnHelpRequested;
            newViewModel.AddPhoneRequested += OnAddPhoneRequested;
        }
    }

    private void OnHelpRequested(object? sender, EventArgs e) => Dispatcher.BeginInvoke(ShowHelp);
    private void OnAddPhoneRequested(object? sender, EventArgs e) => Dispatcher.BeginInvoke(ShowAddPhone);

    private void FocusMain()
    {
        Activate();
        Focus();
    }

    public void ForceClose()
    {
        _forceClose = true;
        _helpWindow?.Close();
        _addPhoneWindow?.Close();
        _settingsWindow?.Close();
        _diagnosticsWindow?.Close();
        Close();
    }

    protected override void OnClosing(CancelEventArgs e)
    {
        if (_forceClose) { base.OnClosing(e); return; }

        if ((DataContext as MainViewModel)?.MinimizeToTray ?? true)
        {
            e.Cancel = true;
            Hide();
            if (!_trayHintShown)
            {
                _trayHintShown = true;
                (System.Windows.Application.Current as App)?.ShowTrayHint();
            }
            return;
        }

        e.Cancel = true;
        (System.Windows.Application.Current as App)?.RequestExit();
    }
}
