using System.Windows;
using AnPod.Desktop.Services;

namespace AnPod.Desktop;

/// <summary>
/// The wireless-connection help, as its own window.
/// </summary>
/// <remarks>
/// A window rather than an inline panel or a <see cref="System.Windows.Controls.Primitives.Popup"/>
/// for two reasons. The text is long enough that expanding it in place pushed the
/// main window's footer off a small screen; and a popup with
/// <c>StaysOpen="False"</c> closes on the very mouse-down that also reaches the
/// button that opened it, so a toggle command reopens it immediately — a bug that
/// is invisible until someone clicks twice.
/// <para>
/// <c>ShowDialog</c> gives Esc-to-close and a modal owner relationship for free.
/// The data context is the main view model, so the hotspot button reuses the
/// existing command instead of duplicating the shell-open logic.
/// </para>
/// </remarks>
public partial class WirelessHelpWindow : Window
{
    public WirelessHelpWindow()
    {
        InitializeComponent();
        Icon = AppIcon.WindowIcon;
    }
}
