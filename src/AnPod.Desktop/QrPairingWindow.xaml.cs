using System.IO;
using System.ComponentModel;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Input;
using System.Windows.Media.Imaging;
using AnPod.Core.Adb;
using AnPod.Desktop.Services;
using AnPod.Desktop.ViewModels;
using QRCoder;

namespace AnPod.Desktop;

public partial class QrPairingWindow : Window
{
    private readonly MainViewModel _viewModel;
    private CancellationTokenSource? _qrCancellation;
    private int _qrGeneration;
    private bool _closing;

    public QrPairingWindow(MainViewModel viewModel)
    {
        InitializeComponent();
        Icon = AppIcon.WindowIcon;
        _viewModel = viewModel;
        DataContext = viewModel;
    }

    private static BitmapImage RenderQr(string payload)
    {
        using var generator = new QRCodeGenerator();
        using var data = generator.CreateQrCode(payload, QRCodeGenerator.ECCLevel.Q);
        var bytes = new PngByteQRCode(data).GetGraphic(8);
        using var stream = new MemoryStream(bytes);
        var image = new BitmapImage();
        image.BeginInit();
        image.CacheOption = BitmapCacheOption.OnLoad;
        image.StreamSource = stream;
        image.EndInit();
        image.Freeze();
        return image;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        FocusManager.SetFocusedElement(this, RetryQrButton);
        Keyboard.Focus(RetryQrButton);
        StartQrPairing();
    }

    private void OnMethodChanged(object sender, System.Windows.Controls.SelectionChangedEventArgs e)
    {
        if (MethodsTab.SelectedIndex != 0) CancelQrPairing();
        else if (IsLoaded && !_closing && _qrCancellation is null) StartQrPairing();
    }

    private async void StartQrPairing()
    {
        CancelQrPairing();
        var generation = ++_qrGeneration;
        var cancellation = new CancellationTokenSource();
        _qrCancellation = cancellation;
        try
        {
            var credentials = _viewModel.CreateQrPairingCredentials();
            QrImageControl.Source = RenderQr(credentials.Payload);
            PairAddressText.Text = "等待扫码";
            ConnectAddressText.Text = "配对后自动发现";
            QrStatusText.Text = "等待手机扫描二维码……";
            RetryQrButton.IsEnabled = false;
            var result = await _viewModel.PairQrAsync(credentials, message => UpdateQrProgress(generation, message), cancellation.Token);
            if (_closing || cancellation.IsCancellationRequested || generation != _qrGeneration) return;
            QrStatusText.Text = result.Message;
            if (result.Success)
            {
                QrStatusText.Text = "配对和手机连接已完成。返回主窗口后可选择“连接并播放”。";
                DialogResult = true;
                Close();
            }
        }
        catch (OperationCanceledException)
        {
            // Tab changes and closing cancel the discovery/ADB operation.
        }
        catch (Exception ex)
        {
            if (!_closing && generation == _qrGeneration)
                QrStatusText.Text = $"二维码配对失败：{ex.Message}。请重新生成并重试。";
        }
        finally
        {
            if (ReferenceEquals(_qrCancellation, cancellation)) _qrCancellation = null;
            cancellation.Dispose();
            if (!_closing && generation == _qrGeneration) RetryQrButton.IsEnabled = true;
        }
    }

    private void UpdateQrProgress(int generation, string message)
    {
        if (_closing) return;
        Dispatcher.BeginInvoke(() =>
        {
            if (_closing || generation != _qrGeneration) return;
            if (message.StartsWith("PAIRING_ENDPOINT:", StringComparison.Ordinal))
                PairAddressText.Text = message["PAIRING_ENDPOINT:".Length..];
            else if (message.StartsWith("CONNECT_ENDPOINT:", StringComparison.Ordinal))
                ConnectAddressText.Text = message["CONNECT_ENDPOINT:".Length..];
            else QrStatusText.Text = message;
        });
    }

    private void CancelQrPairing()
    {
        _qrGeneration++;
        try { _qrCancellation?.Cancel(); }
        catch (ObjectDisposedException) { }
        _qrCancellation = null;
    }

    private void OnRetryQrClick(object sender, RoutedEventArgs e) => StartQrPairing();

    private async void OnPairCodeClick(object sender, RoutedEventArgs e)
    {
        if (await _viewModel.PairWirelessCodeAsync())
        {
            if (!string.IsNullOrWhiteSpace(_viewModel.WifiEndpoint))
                await _viewModel.ConnectRememberedWifiAsync();
            DialogResult = true;
            Close();
        }
        else
        {
            AutomationProperties.SetLiveSetting(PairCodeBox, AutomationLiveSetting.Polite);
            PairCodeBox.Focus();
        }
    }

    private async void OnConnectExistingClick(object sender, RoutedEventArgs e)
    {
        if (await _viewModel.ConnectRememberedWifiAsync())
        {
            DialogResult = true;
            Close();
        }
    }

    private void OnClosing(object? sender, CancelEventArgs e)
    {
        _closing = true;
        CancelQrPairing();
        QrImageControl.Source = null;
    }

    protected override void OnPreviewKeyDown(System.Windows.Input.KeyEventArgs e)
    {
        if (e.Key == Key.Escape)
        {
            Close();
            e.Handled = true;
        }
        base.OnPreviewKeyDown(e);
    }
}
