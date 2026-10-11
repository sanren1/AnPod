using System.Text.RegularExpressions;
using AnPod.Core.Models;

namespace AnPod.Core.Adb;

/// <summary>
/// Parsing for the two ADB outputs AnPod depends on. Kept pure and separate so
/// it can be tested without spawning <c>adb</c> (plan §5.1/§5.2).
/// </summary>
public static partial class AdbOutputParser
{
    [GeneratedRegex(@"^\d{1,3}(\.\d{1,3}){3}:\d{1,5}$", RegexOptions.CultureInvariant)]
    private static partial Regex NetworkSerialRegex();

    [GeneratedRegex(@"^adb-.+\._adb-tls-connect\._tcp\.?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase)]
    private static partial Regex TlsMdnsSerialRegex();

    /// <summary>
    /// Parses <c>adb devices -l</c>. Unknown/garbage lines are ignored rather
    /// than mis-parsed, because ADB interleaves daemon banners on the same stream.
    /// </summary>
    public static IReadOnlyList<AdbDeviceEntry> ParseDevices(string? stdout)
    {
        var result = new List<AdbDeviceEntry>();
        if (string.IsNullOrWhiteSpace(stdout))
        {
            return result;
        }

        foreach (var rawLine in stdout.Split('\n'))
        {
            var line = rawLine.Trim();
            if (line.Length == 0 || line.StartsWith("List of devices", StringComparison.Ordinal))
            {
                continue;
            }

            var parts = line.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
            if (parts.Length < 2)
            {
                continue;
            }

            var serial = parts[0];
            var state = ParseState(line, parts);
            if (state == AdbDeviceState.Unknown)
            {
                // Banner text, "* daemon started successfully *", etc.
                continue;
            }

            string? usbTag = null;
            foreach (var token in parts)
            {
                if (token.StartsWith("usb:", StringComparison.Ordinal) && token.Length > 4)
                {
                    usbTag = token[4..];
                    break;
                }
            }

            result.Add(new AdbDeviceEntry(serial, state, usbTag));
        }

        return result;
    }

    private static AdbDeviceState ParseState(string line, string[] parts)
    {
        // "no permissions" spans two columns, so it cannot be read from parts[1].
        if (line.Contains("no permissions", StringComparison.OrdinalIgnoreCase))
        {
            return AdbDeviceState.NoPermissions;
        }

        return parts[1] switch
        {
            "device" => AdbDeviceState.Device,
            "unauthorized" => AdbDeviceState.Unauthorized,
            "offline" => AdbDeviceState.Offline,
            // Transient or non-target transports: reported, but never connectable.
            "authorizing" or "connecting" or "recovery" or "bootloader" or "sideload" or "rescue" or "host"
                => AdbDeviceState.Unknown,
            _ => AdbDeviceState.Unknown,
        };
    }

    /// <summary>
    /// Parses the labelled <c>echo "tag:$(getprop ...)"</c> output used by
    /// DeviceDiscoveryService. Order-independent, so stray daemon lines cannot
    /// shift the field indices.
    /// </summary>
    public static IReadOnlyDictionary<string, string> ParseLabeledProperties(string? stdout)
    {
        var map = new Dictionary<string, string>(StringComparer.Ordinal);
        if (string.IsNullOrWhiteSpace(stdout))
        {
            return map;
        }

        foreach (var rawLine in stdout.Split('\n'))
        {
            var line = rawLine.Trim();
            var separator = line.IndexOf(':');
            if (separator <= 0)
            {
                continue;
            }

            var key = line[..separator].Trim();
            var value = line[(separator + 1)..].Trim();
            if (key.Length is > 0 and <= 4)
            {
                map[key] = value;
            }
        }

        return map;
    }

    /// <summary>
    /// True when the ADB serial is an <c>ip:port</c> endpoint or Android's TLS
    /// wireless-debugging mDNS serial. The inverse is NOT a USB proof.
    /// </summary>
    public static bool IsNetworkSerial(string? serial) =>
        !string.IsNullOrWhiteSpace(serial) &&
        (NetworkSerialRegex().IsMatch(serial) || TlsMdnsSerialRegex().IsMatch(serial));

    /// <summary>
    /// Prefix shared by every abstract socket name AnPod creates. Only names
    /// carrying it may ever be removed (plan §3.4.2: never <c>--remove-all</c>).
    /// </summary>
    public const string OwnReversePrefix = "localabstract:anpod_";

    /// <summary>
    /// Extracts AnPod's own reverse mappings from <c>adb reverse --list</c>.
    /// </summary>
    /// <remarks>
    /// The mapping is matched as a token anywhere in the line rather than at the
    /// start: platform-tools 37 prefixes each row with the transport type, e.g.
    /// <c>UsbFfs localabstract:anpod_1 tcp:6043</c>, while older releases emitted
    /// the bare <c>localabstract:anpod_1 tcp:6043</c>. A start-of-line match
    /// silently finds nothing on current releases, leaving stale mappings behind
    /// after a crash or a force-kill. Mappings belonging to other tools share no
    /// prefix with ours and are therefore never returned.
    /// </remarks>
    public static IReadOnlyList<string> ParseOwnReverseMappings(string? stdout)
    {
        var names = new List<string>();
        if (string.IsNullOrWhiteSpace(stdout))
        {
            return names;
        }

        foreach (var rawLine in stdout.Split('\n'))
        {
            var line = rawLine.Trim();
            var start = line.IndexOf(OwnReversePrefix, StringComparison.Ordinal);
            if (start < 0)
            {
                continue;
            }

            // The name ends at the whitespace before the tcp: target.
            var end = line.IndexOfAny(new[] { ' ', '\t' }, start);
            names.Add(end < 0 ? line[start..] : line[start..end]);
        }

        return names;
    }

    /// <summary>
    /// The batched property query issued per device. Single shell invocation so
    /// we pay one ADB round-trip per device per poll.
    /// </summary>
    /// <remarks>
    /// <c>us</c> and <c>tp</c> exist for transport classification. Recent
    /// platform-tools releases do not always emit a <c>usb:</c> tag in
    /// <c>adb devices -l</c> even for a genuine USB transport, so the device's
    /// own USB state is needed as a second opinion (plan §5.2). They ride along
    /// in this same call, so the extra check costs no additional round-trip.
    /// Keys must stay within the 4-character limit ParseLabeledProperties
    /// enforces.
    /// </remarks>
    public const string PropertyQueryCommand =
        "echo \"sn:$(getprop ro.serialno)\"; " +
        "echo \"mo:$(getprop ro.product.model)\"; " +
        "echo \"mf:$(getprop ro.product.manufacturer)\"; " +
        "echo \"av:$(getprop ro.build.version.release)\"; " +
        "echo \"al:$(getprop ro.build.version.sdk)\"; " +
        "echo \"us:$(getprop sys.usb.state)\"; " +
        "echo \"tp:$(getprop service.adb.tcp.port)\"";
}
