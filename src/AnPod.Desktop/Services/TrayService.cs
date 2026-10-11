using AnPod.Core.Models;
using Forms = System.Windows.Forms;

namespace AnPod.Desktop.Services;

public sealed class TrayService : IDisposable
{
    private readonly Forms.NotifyIcon _notifyIcon;
    private readonly Forms.ToolStripMenuItem _primaryItem;
    private readonly Forms.ToolStripMenuItem _disconnectItem;
    private readonly Icon _icon;
    private bool _disposed;

    public TrayService()
    {
        _icon = AppIcon.CreateTrayIcon();
        _primaryItem = new Forms.ToolStripMenuItem("连接并播放");
        _primaryItem.Click += (_, _) => PrimaryActionRequested?.Invoke(this, EventArgs.Empty);
        _disconnectItem = new Forms.ToolStripMenuItem("断开连接");
        _disconnectItem.Click += (_, _) => DisconnectRequested?.Invoke(this, EventArgs.Empty);
        var showItem = new Forms.ToolStripMenuItem("显示 AnPod");
        showItem.Click += (_, _) => ShowRequested?.Invoke(this, EventArgs.Empty);
        var settingsItem = new Forms.ToolStripMenuItem("设置");
        settingsItem.Click += (_, _) => SettingsRequested?.Invoke(this, EventArgs.Empty);
        var exitItem = new Forms.ToolStripMenuItem("退出");
        exitItem.Click += (_, _) => ExitRequested?.Invoke(this, EventArgs.Empty);

        var menu = new Forms.ContextMenuStrip();
        menu.Items.Add(showItem);
        menu.Items.Add(_primaryItem);
        menu.Items.Add(_disconnectItem);
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add(settingsItem);
        menu.Items.Add(exitItem);
        _notifyIcon = new Forms.NotifyIcon
        {
            Icon = _icon,
            Text = "AnPod",
            Visible = true,
            ContextMenuStrip = menu,
        };
        _notifyIcon.MouseClick += (_, e) =>
        {
            if (e.Button == Forms.MouseButtons.Left) ShowRequested?.Invoke(this, EventArgs.Empty);
        };
    }

    public event EventHandler? ShowRequested;
    public event EventHandler? PrimaryActionRequested;
    public event EventHandler? DisconnectRequested;
    public event EventHandler? SettingsRequested;
    public event EventHandler? ExitRequested;

    public void UpdateStatus(ConnectionState state, string message, string primaryAction)
    {
        _primaryItem.Text = primaryAction;
        // The primary action is also the cancel action while connecting/reconnecting.
        _primaryItem.Enabled = true;
        _disconnectItem.Enabled = state is ConnectionState.Connected or ConnectionState.Streaming;
        var text = $"AnPod — {message}";
        _notifyIcon.Text = text.Length > 63 ? text[..60] + "..." : text;
    }

    public void ShowHideHint() => _notifyIcon.ShowBalloonTip(3500, "AnPod 仍在运行", "单击托盘图标可重新打开窗口；右键菜单中可退出。", Forms.ToolTipIcon.Info);

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        _notifyIcon.Visible = false;
        _notifyIcon.Dispose();
        _icon.Dispose();
    }
}
