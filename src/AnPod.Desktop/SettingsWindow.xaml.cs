using System.Windows;
using MessageBox = System.Windows.MessageBox;

namespace AnPod.Desktop;

public partial class SettingsWindow : Window
{
    public SettingsWindow()
    {
        InitializeComponent();
        Icon = Services.AppIcon.WindowIcon;
    }

    private void OnForgetAddressClick(object sender, RoutedEventArgs e)
    {
        if (DataContext is not ViewModels.MainViewModel viewModel || !viewModel.CanForgetSavedWifiAddress) return;
        var answer = MessageBox.Show(
            this,
            "这会清除 AnPod 保存的无线连接地址，不会断开当前 ADB 连接或撤销手机端配对。要继续吗？",
            "忘记无线地址",
            MessageBoxButton.YesNo,
            MessageBoxImage.Question,
            MessageBoxResult.No);
        if (answer == MessageBoxResult.Yes) viewModel.ForgetSavedWifiAddress();
    }
}
