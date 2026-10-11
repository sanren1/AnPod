using AnPod.Core.Models;

namespace AnPod.Core.Adb;

/// <summary>
/// Turns a raw <c>adb devices -l</c> row plus its property map into a
/// <see cref="DeviceInfo"/>. The transport-classification rule lives here and
/// nowhere else so it can be tested directly (plan §5.2).
/// </summary>
public static class DeviceInfoFactory
{
    /// <summary>
    /// Classifies the transport. USB is asserted when ADB reported a <c>usb:</c>
    /// tag, or — because recent platform-tools releases omit that tag even for
    /// real USB transports — when the device's own <c>sys.usb.state</c> lists
    /// <c>adb</c> as an active USB function. An IP or TLS mDNS serial proves
    /// network. Anything else stays <see cref="TransportKind.Unknown"/> and must
    /// not auto-connect (plan §5.2).
    /// </summary>
    /// <remarks>
    /// The ADB row identifies its own transport. In particular, global
    /// <c>sys.usb.state</c> must not override an IP/TLS network serial when the
    /// same phone is also plugged in.
    /// </remarks>
    public static DeviceInfo Create(
        AdbDeviceEntry entry,
        IReadOnlyDictionary<string, string>? properties = null)
    {
        ArgumentNullException.ThrowIfNull(entry);

        var props = properties ?? new Dictionary<string, string>();

        bool usbTagConfirmed = !string.IsNullOrEmpty(entry.UsbTag);
        bool networkConfirmed = AdbOutputParser.IsNetworkSerial(entry.Serial);
        bool usbStateConfirmed = IsUsbStateConfirmed(props);

        TransportKind transport = usbTagConfirmed
            ? TransportKind.Usb
            : networkConfirmed
                ? TransportKind.Wifi
                : usbStateConfirmed
                    ? TransportKind.Usb
                : TransportKind.Unknown;

        return new DeviceInfo
        {
            Serial = entry.Serial,
            State = entry.State,
            Transport = transport,
            Model = Get(props, "mo"),
            Manufacturer = Get(props, "mf"),
            AndroidVersion = Get(props, "av"),
            ApiLevel = Get(props, "al"),
        };
    }

    /// <summary>
    /// Reads the <c>us</c> field (<c>sys.usb.state</c>), a comma-separated list
    /// of the active USB functions such as <c>mtp,adb</c>. It contains
    /// <c>adb</c> exactly when ADB is running over USB, so it is a positive
    /// USB proof rather than an absence-of-evidence guess.
    /// </summary>
    private static bool IsUsbStateConfirmed(IReadOnlyDictionary<string, string> props)
    {
        if (!props.TryGetValue("us", out var value) || string.IsNullOrWhiteSpace(value))
        {
            return false;
        }

        foreach (var function in value.Split(
            ',',
            StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            if (function.Equals("adb", StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }

        return false;
    }

    private static string Get(IReadOnlyDictionary<string, string> props, string key) =>
        props.TryGetValue(key, out var value) ? value : string.Empty;
}
