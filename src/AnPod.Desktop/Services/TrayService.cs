using AnPod.Core.Models;
using Forms = System.Windows.Forms;

namespace AnPod.Desktop.Services;

/// <summary>
/// Owns the notification-area icon and its menu (plan §5.7). State is conveyed
/// by both the tooltip text and the menu label — colour alone is never the only
/// signal.
/// </summary>
public sealed class TrayService : IDisposable
{
    private readonly Forms.NotifyIcon _notifyIcon;
    private readonly Forms.ToolStripMenuItem _toggleItem;
    private readonly Icon _icon;
    private bool _disposed;

    public TrayService()
    {
        _icon = AppIcon.CreateTrayIcon();

        _toggleItem = new Forms.ToolStripMenuItem("连接");
        _toggleItem.Click += (_, _) => ToggleConnectionRequested?.Invoke(this, EventArgs.Empty);

        var showItem = new Forms.ToolStripMenuItem("显示 AnPod");
        showItem.Click += (_, _) => ShowRequested?.Invoke(this, EventArgs.Empty);

        var exitItem = new Forms.ToolStripMenuItem("退出");
        exitItem.Click += (_, _) => ExitRequested?.Invoke(this, EventArgs.Empty);

        var menu = new Forms.ContextMenuStrip();
        menu.Items.Add(showItem);
        menu.Items.Add(_toggleItem);
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add(exitItem);

        _notifyIcon = new Forms.NotifyIcon
        {
            Icon = _icon,
            Text = "AnPod",
            Visible = true,
            ContextMenuStrip = menu,
        };

        _notifyIcon.DoubleClick += (_, _) => ShowRequested?.Invoke(this, EventArgs.Empty);
    }

    public event EventHandler? ShowRequested;

    public event EventHandler? ToggleConnectionRequested;

    public event EventHandler? ExitRequested;

    /// <summary>Reflects the current connection state in the menu and tooltip.</summary>
    public void UpdateStatus(ConnectionState state, string message)
    {
        _toggleItem.Text = state switch
        {
            // Connected means the phone is still attached, just not playing, so
            // the menu must still offer to drop the session.
            ConnectionState.Connected or ConnectionState.Streaming => "断开连接",
            ConnectionState.Connecting or ConnectionState.Reconnecting => "连接中…",
            _ => "连接",
        };

        _toggleItem.Enabled = state is not (ConnectionState.Connecting or ConnectionState.Reconnecting);

        // NotifyIcon.Text is capped at 63 characters.
        var text = $"AnPod — {message}";
        _notifyIcon.Text = text.Length > 63 ? text[..60] + "..." : text;
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }

        _disposed = true;
        _notifyIcon.Visible = false;
        _notifyIcon.Dispose();
        _icon.Dispose();
    }
}
