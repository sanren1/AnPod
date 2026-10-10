namespace AnPod.Core.Models;

/// <summary>
/// How the phone is attached to the PC (plan §1.2/§1.4).
/// <para>
/// USB and Wi-Fi are mutually exclusive sessions: audio is never sent down both
/// paths at once, and switching means stopping one session before starting the
/// other. The transport is therefore a property of the device, not a global mode.
/// </para>
/// </summary>
public enum TransportKind
{
    /// <summary>
    /// Not determinable from the evidence available. Plan §5.2 forbids guessing
    /// here — such a device must never auto-connect.
    /// </summary>
    Unknown = 0,

    /// <summary>USB cable + ADB debugging.</summary>
    Usb,

    /// <summary>
    /// ADB over TCP: either a shared LAN or a Windows mobile hotspot the phone
    /// joined. Both are the same channel (plan §1.4); the hotspot is only a
    /// label, never a second audio protocol.
    /// </summary>
    Wifi,
}
