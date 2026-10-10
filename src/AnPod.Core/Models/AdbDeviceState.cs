namespace AnPod.Core.Models;

/// <summary>
/// Device state exactly as reported by the <c>adb devices</c> transport column.
/// Do not collapse these into a boolean: <see cref="Unauthorized"/> and
/// <see cref="Offline"/> require very different user guidance (plan §6).
/// </summary>
public enum AdbDeviceState
{
    /// <summary>State column could not be recognised.</summary>
    Unknown = 0,

    /// <summary>Transport reports <c>device</c>: authorised and usable.</summary>
    Device,

    /// <summary>Transport reports <c>unauthorized</c>: user has not accepted the RSA prompt.</summary>
    Unauthorized,

    /// <summary>Transport reports <c>offline</c>: device present but not answering.</summary>
    Offline,

    /// <summary>Transport reports <c>no permissions</c>: udev/driver level problem.</summary>
    NoPermissions,
}
