using System.Windows;

namespace AnPod.Desktop;

public partial class DiagnosticsWindow : Window
{
    public DiagnosticsWindow()
    {
        InitializeComponent();
        Icon = Services.AppIcon.WindowIcon;
    }
}
