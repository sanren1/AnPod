namespace AnPod.Core.Models;

/// <summary>
/// One entry from <c>adb devices -l</c>, before the (optional) property query.
/// </summary>
/// <param name="Serial">ADB serial. Never sourced from UI input (plan §5.1).</param>
/// <param name="State">Transport state from the second column.</param>
/// <param name="UsbTag">
/// The <c>usb:</c> value when ADB reported one (e.g. <c>1-2</c>). Null when the
/// transport did not advertise USB, which is the case for network devices.
/// </param>
public sealed record AdbDeviceEntry(string Serial, AdbDeviceState State, string? UsbTag);

/// <summary>
/// A fully described candidate device. <see cref="Transport"/> is only
/// <see cref="TransportKind.Usb"/> when USB was actually confirmed — plan §5.2
/// forbids inferring USB from "serial does not contain a colon" alone.
/// </summary>
public sealed record DeviceInfo
{
    public required string Serial { get; init; }

    public AdbDeviceState State { get; init; } = AdbDeviceState.Unknown;

    /// <summary>Positively identified transport, or <see cref="TransportKind.Unknown"/>.</summary>
    public TransportKind Transport { get; init; } = TransportKind.Unknown;

    public string Model { get; init; } = string.Empty;

    public string Manufacturer { get; init; } = string.Empty;

    public string AndroidVersion { get; init; } = string.Empty;

    public string ApiLevel { get; init; } = string.Empty;

    /// <summary>True only when USB was positively confirmed.</summary>
    public bool IsUsb => Transport == TransportKind.Usb;

    /// <summary>
    /// True when the device is reachable over ADB/TCP. This says nothing about
    /// whether it arrived via a shared LAN or a Windows hotspot — both are the
    /// same channel (plan §1.4).
    /// </summary>
    public bool IsWifi => Transport == TransportKind.Wifi;

    /// <summary>True when the transport could not be classified either way.</summary>
    public bool TransportUnknown => Transport == TransportKind.Unknown;

    /// <summary>Short transport label for the device list and the status area.</summary>
    public string TransportText => Transport switch
    {
        TransportKind.Usb => "USB",
        TransportKind.Wifi => "Wi-Fi",
        _ => "待确认",
    };

    /// <summary>Human label for the UI; falls back to a masked serial.</summary>
    public string DisplayName
    {
        get
        {
            if (!string.IsNullOrWhiteSpace(Model) && !string.IsNullOrWhiteSpace(Manufacturer))
            {
                return $"{Manufacturer} {Model}".Trim();
            }

            if (!string.IsNullOrWhiteSpace(Model))
            {
                return Model;
            }

            return Serial;
        }
    }

    /// <summary>
    /// One-line transport state for the device list. It lives here rather than in
    /// the view model so every row can state its own condition — with a list, the
    /// user needs to see why the device they want is not selectable, not just the
    /// state of whichever row happens to be highlighted.
    /// </summary>
    public string StatusText
    {
        get
        {
            if (TransportUnknown)
            {
                // Deliberately not "only USB is supported" any more: a device we
                // cannot classify is a different problem from a phone that is
                // simply not plugged in (plan §6).
                return "连接方式待确认";
            }

            return State switch
            {
                AdbDeviceState.Device => "可连接",
                AdbDeviceState.Unauthorized => "等待手机授权",
                AdbDeviceState.Offline => "设备暂时离线",
                AdbDeviceState.NoPermissions => "缺少 USB 驱动权限",
                _ => "状态未知",
            };
        }
    }

    /// <summary>
    /// A device is connectable when it is authorised and its transport is known.
    /// Both USB and Wi-Fi qualify; only <see cref="TransportKind.Unknown"/> is
    /// refused (plan §5.2).
    /// </summary>
    public bool IsConnectable => State == AdbDeviceState.Device && !TransportUnknown;

    /// <summary>
    /// True when this device belongs to <paramref name="transport"/> — i.e. when a
    /// window showing that mode should list it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// One phone can be reachable both ways at once and so appear twice in
    /// <c>adb devices</c>: once under its USB serial, once as <c>ip:port</c>. Each
    /// mode lists only its own, so the list can never offer a device the current
    /// mode cannot use (plan §1.4).
    /// </para>
    /// <para>
    /// An unclassified device matches <em>both</em> modes. It cannot be attributed
    /// to either, and hiding it would empty both lists while a phone is plainly
    /// attached. Callers that need something usable must test
    /// <see cref="IsConnectable"/> as well, which already refuses it.
    /// </para>
    /// </remarks>
    public bool MatchesTransport(TransportKind transport) =>
        TransportUnknown ||
        (transport == TransportKind.Wifi ? IsWifi : IsUsb);
}
