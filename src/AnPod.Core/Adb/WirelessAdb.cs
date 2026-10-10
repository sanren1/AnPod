using System.Globalization;
using System.Text.RegularExpressions;

namespace AnPod.Core.Adb;

/// <summary>How an <c>adb connect</c> attempt ended (plan §3.5).</summary>
public enum AdbConnectOutcome
{
    /// <summary>Output was not recognised — treat as a failure, never as success.</summary>
    Unknown = 0,

    /// <summary>The endpoint accepted the connection.</summary>
    Connected,

    /// <summary>ADB already had a session to this endpoint.</summary>
    AlreadyConnected,

    /// <summary>ADB refused, or could not reach, the endpoint.</summary>
    Failed,
}

/// <summary>How an <c>adb pair</c> attempt ended (plan §3.5).</summary>
public enum AdbPairOutcome
{
    /// <summary>Output was not recognised — treat as a failure, never as success.</summary>
    Unknown = 0,

    /// <summary>The pairing code was accepted.</summary>
    Succeeded,

    /// <summary>Pairing was rejected (wrong code, expired dialog, unreachable host).</summary>
    Failed,
}

/// <summary>Which step of the wireless-debugging handshake a discovered service serves.</summary>
public enum AdbMdnsKind
{
    /// <summary>Not one of the two ADB wireless services.</summary>
    Other = 0,

    /// <summary><c>_adb-tls-pairing._tcp</c> — the address that goes in the pairing-code dialog.</summary>
    Pairing,

    /// <summary><c>_adb-tls-connect._tcp</c> — the address <c>adb connect</c> wants.</summary>
    Connect,
}

/// <summary>
/// One service instance advertised over mDNS, as printed by <c>adb mdns services</c>.
/// </summary>
/// <param name="Name">Service instance name, e.g. <c>adb-3DNN192P3ZPE-4KPB2X._adb-tls-connect._tcp.</c></param>
/// <param name="Endpoint">Normalised <c>ip:port</c>.</param>
public sealed record AdbMdnsService(string Name, string Endpoint, AdbMdnsKind Kind);

/// <summary>
/// The wireless-ADB pieces that are pure logic and therefore testable without a
/// phone: validating what the user typed, and classifying what <c>adb pair</c>,
/// <c>adb connect</c> and <c>adb mdns services</c> printed.
/// </summary>
/// <remarks>
/// Classifying the output is mandatory rather than a nicety: <c>adb connect</c>
/// exits 0 even when it failed to connect, so the exit code alone would report
/// success for a phone that never answered (plan §3.4.1).
/// </remarks>
public static partial class WirelessAdb
{
    /// <summary>Android shows a six-digit code in the wireless-debugging dialog.</summary>
    public const int PairingCodeLength = 6;

    /// <summary>Header printed by <c>adb mdns services</c> even when it finds nothing.</summary>
    private const string MdnsListHeader = "List of discovered mdns services";

    /// <summary>Texts that mean "this did not work", checked before the success text.</summary>
    /// <remarks>
    /// Deliberately no bare socket error codes. Windows reports those as
    /// "(10061)" inside a message that already says "cannot connect to", while a
    /// <em>port</em> can legitimately be 10061 — matching the digits alone would
    /// turn a successful connection into a reported failure.
    /// </remarks>
    private static readonly string[] FailureMarkers =
    {
        "failed",
        "failure",
        "unable to",
        "cannot",
        "can't",
        "couldn't",
        "connection refused",
        "no route to host",
        "unknown host",
        "missing port",
        "timed out",
        "protocol fault",
        "error:",
    };

    [GeneratedRegex(@"^\d{1,3}(?:\.\d{1,3}){3}$", RegexOptions.CultureInvariant)]
    private static partial Regex Ipv4Regex();

    /// <summary>
    /// Normalises user input into a bare <c>ip:port</c> endpoint.
    /// </summary>
    /// <param name="problem">
    /// A message meant for the status line when the input cannot be used. The
    /// pairing port and the connect port are different values on Android 11+,
    /// which is the single most common way to get this wrong, so the guidance
    /// names the expected shape.
    /// </param>
    public static bool TryNormalizeEndpoint(string? input, out string endpoint, out string? problem)
    {
        endpoint = string.Empty;
        problem = null;

        var text = (input ?? string.Empty).Trim();
        if (text.Length == 0)
        {
            problem = "请填写手机的 IP:端口。";
            return false;
        }

        // People paste the whole command out of a tutorial; accept that.
        foreach (var prefix in new[] { "adb connect ", "adb pair ", "adb " })
        {
            if (text.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
            {
                text = text[prefix.Length..].Trim();
                break;
            }
        }

        var separator = text.LastIndexOf(':');
        if (separator <= 0 || separator == text.Length - 1)
        {
            problem = "格式应为 IP:端口，例如 192.168.1.20:5555。";
            return false;
        }

        var host = text[..separator].Trim();
        var portText = text[(separator + 1)..].Trim();

        if (!Ipv4Regex().IsMatch(host))
        {
            problem = "目前只支持 IPv4 地址，例如 192.168.1.20。";
            return false;
        }

        // NumberStyles.None rejects "+5555", "1e3" and surrounding whitespace, so
        // a typo cannot be silently coerced into a different port.
        if (!int.TryParse(portText, NumberStyles.None, CultureInfo.InvariantCulture, out int port) ||
            port is < 1 or > 65535)
        {
            problem = "端口应为 1–65535 之间的数字。";
            return false;
        }

        endpoint = $"{host}:{port}";
        return true;
    }

    /// <summary>True when <paramref name="code"/> is exactly six digits.</summary>
    public static bool IsValidPairingCode(string? code)
    {
        var text = (code ?? string.Empty).Trim();
        if (text.Length != PairingCodeLength)
        {
            return false;
        }

        foreach (var character in text)
        {
            if (character is < '0' or > '9')
            {
                return false;
            }
        }

        return true;
    }

    /// <summary>Classifies <c>adb pair</c> output.</summary>
    /// <param name="detail">The offending line, when one can be identified.</param>
    public static AdbPairOutcome ParsePairResult(string? stdout, string? stderr, out string? detail)
    {
        detail = null;
        var combined = Combine(stdout, stderr);
        if (combined.Length == 0)
        {
            return AdbPairOutcome.Unknown;
        }

        if (combined.Contains("Successfully paired", StringComparison.OrdinalIgnoreCase))
        {
            return AdbPairOutcome.Succeeded;
        }

        var line = FirstOffendingLine(combined);
        if (line is not null)
        {
            detail = line;
            return AdbPairOutcome.Failed;
        }

        return AdbPairOutcome.Unknown;
    }

    /// <summary>Classifies <c>adb connect</c> output.</summary>
    public static AdbConnectOutcome ParseConnectResult(string? stdout, string? stderr)
    {
        var combined = Combine(stdout, stderr);
        if (combined.Length == 0)
        {
            return AdbConnectOutcome.Unknown;
        }

        // Checked first: "already connected to" contains "connected to".
        if (combined.Contains("already connected", StringComparison.OrdinalIgnoreCase))
        {
            return AdbConnectOutcome.AlreadyConnected;
        }

        // Before the success text on purpose. None of the failure phrasings
        // ("failed to connect to", "unable to connect to") contains the success
        // phrase "connected to", so this ordering cannot mask a real success.
        if (FirstOffendingLine(combined) is not null)
        {
            return AdbConnectOutcome.Failed;
        }

        if (combined.Contains("connected to", StringComparison.OrdinalIgnoreCase))
        {
            return AdbConnectOutcome.Connected;
        }

        return AdbConnectOutcome.Unknown;
    }

    // ------------------------------------------------------------- mDNS discovery

    /// <summary>
    /// True when the ADB server's mDNS backend is not running, so discovery
    /// cannot work at all.
    /// </summary>
    /// <remarks>
    /// This is a real and observed state, not a hypothetical: a long-lived ADB
    /// server whose mDNS daemon died reports <c>ERROR: mdns daemon unavailable</c>
    /// to <c>adb mdns check</c> while a freshly started server on another port
    /// reports a healthy version. It matters because the only cure is restarting
    /// the ADB server, and AnPod must never do that (plan §3.2 — another tool may
    /// be using it). So the honest answer is to say discovery is unavailable and
    /// send the user back to the manual address, not to silently show an empty
    /// list that looks like "no phone on the network".
    /// <para>
    /// Only meaningful on the output of <c>adb mdns check</c>.
    /// <c>adb mdns services</c> prints an empty list with exit code 0 in this
    /// state — measured — so it cannot be used to tell an outage from a quiet
    /// network, and this predicate must not be pointed at it.
    /// </para>
    /// </remarks>
    public static bool IsMdnsUnavailable(string? stdout, string? stderr)
    {
        var combined = Combine(stdout, stderr);
        return combined.Contains("mdns daemon unavailable", StringComparison.OrdinalIgnoreCase)
            || combined.Contains("mdns daemon not running", StringComparison.OrdinalIgnoreCase)
            || combined.Contains("mdns daemon", StringComparison.OrdinalIgnoreCase)
               && combined.Contains("unavailable", StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>
    /// True when <c>adb mdns check</c> reported a running daemon.
    /// </summary>
    /// <remarks>
    /// Paired with <see cref="IsMdnsUnavailable"/> so a positive answer is
    /// required before an empty device list is reported as "nothing out there":
    /// the two outcomes are otherwise indistinguishable, and guessing wrong sends
    /// the user to debug a phone that was never the problem.
    /// </remarks>
    public static bool IsMdnsHealthy(string? stdout, string? stderr) =>
        Combine(stdout, stderr).Contains("mdns daemon version", StringComparison.OrdinalIgnoreCase);

    /// <summary>
    /// Parses <c>adb mdns services</c> output.
    /// </summary>
    /// <remarks>
    /// Deliberately tolerant of the column layout, which has changed between
    /// platform-tools releases: rather than trusting fixed positions, each line
    /// is scanned for the last whitespace-separated token that is a valid
    /// <c>ip:port</c>, and the service kind is read from anywhere in the line.
    /// Lines that do not yield an endpoint are skipped, so a header, a blank line
    /// or a future extra column cannot break the parse.
    /// </remarks>
    public static IReadOnlyList<AdbMdnsService> ParseMdnsServices(string? stdout)
    {
        var services = new List<AdbMdnsService>();
        if (string.IsNullOrWhiteSpace(stdout))
        {
            return services;
        }

        foreach (var rawLine in stdout.Split('\n'))
        {
            var line = rawLine.Trim();
            if (line.Length == 0 ||
                line.StartsWith(MdnsListHeader, StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            string? endpoint = null;
            foreach (var token in line.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries))
            {
                if (TryNormalizeEndpoint(token, out var normalized, out _))
                {
                    endpoint = normalized;
                }
            }

            if (endpoint is null)
            {
                continue;
            }

            // The instance name is whatever precedes the endpoint; it is only used
            // for display, so an unusual layout degrades to the endpoint itself.
            var name = line[..line.LastIndexOf(endpoint, StringComparison.Ordinal)].Trim();
            services.Add(new AdbMdnsService(
                name.Length == 0 ? endpoint : name,
                endpoint,
                ClassifyMdnsService(line)));
        }

        return services;
    }

    /// <summary>Reads the service kind out of a discovery line.</summary>
    /// <remarks>
    /// Pairing is checked first: <c>_adb-tls-pairing._tcp</c> does not contain
    /// <c>_adb-tls-connect._tcp</c>, so the order only matters for the reverse
    /// case, but naming it explicitly keeps the intent obvious.
    /// </remarks>
    private static AdbMdnsKind ClassifyMdnsService(string line)
    {
        if (line.Contains("_adb-tls-pairing", StringComparison.OrdinalIgnoreCase))
        {
            return AdbMdnsKind.Pairing;
        }

        if (line.Contains("_adb-tls-connect", StringComparison.OrdinalIgnoreCase) ||
            line.Contains("_adb._tcp", StringComparison.OrdinalIgnoreCase))
        {
            return AdbMdnsKind.Connect;
        }

        return AdbMdnsKind.Other;
    }

    /// <summary>
    /// The host part of a normalised <c>ip:port</c> endpoint, or null when the
    /// input is not one.
    /// </summary>
    public static string? HostOf(string? endpoint)
    {
        if (!TryNormalizeEndpoint(endpoint, out var normalized, out _))
        {
            return null;
        }

        return normalized[..normalized.LastIndexOf(':')];
    }

    /// <summary>
    /// Picks which advertised connect endpoint to fill in automatically.
    /// </summary>
    /// <param name="preferredHost">
    /// The host the user just paired with, when there is one. Two phones on the
    /// same network both advertise, so this is what keeps the answer to "the
    /// phone I am holding" instead of "some phone".
    /// </param>
    /// <returns>
    /// The endpoint, or null when the choice is genuinely ambiguous — a wrong
    /// silent guess would be worse than showing the list and letting the user
    /// pick (plan §6: never choose a device on the user's behalf).
    /// </returns>
    public static string? PickConnectEndpoint(
        IReadOnlyList<AdbMdnsService> connect,
        string? preferredHost)
    {
        if (connect.Count == 0)
        {
            return null;
        }

        if (connect.Count == 1)
        {
            return connect[0].Endpoint;
        }

        if (string.IsNullOrEmpty(preferredHost))
        {
            return null;
        }

        var matching = connect
            .Where(s => string.Equals(HostOf(s.Endpoint), preferredHost, StringComparison.Ordinal))
            .ToList();

        return matching.Count == 1 ? matching[0].Endpoint : null;
    }

    private static string Combine(string? stdout, string? stderr)    {
        var first = (stdout ?? string.Empty).Trim();
        var second = (stderr ?? string.Empty).Trim();

        if (first.Length == 0)
        {
            return second;
        }

        return second.Length == 0 ? first : $"{first}\n{second}";
    }

    /// <summary>The first line carrying a failure marker, or null when there is none.</summary>
    private static string? FirstOffendingLine(string text)
    {
        foreach (var rawLine in text.Split('\n'))
        {
            var line = rawLine.Trim();
            if (line.Length == 0)
            {
                continue;
            }

            foreach (var marker in FailureMarkers)
            {
                if (line.Contains(marker, StringComparison.OrdinalIgnoreCase))
                {
                    return line.Length > 200 ? line[..200] : line;
                }
            }
        }

        return null;
    }
}
