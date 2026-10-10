using System.Text.RegularExpressions;

namespace AnPod.Core.Logging;

/// <summary>
/// Log hygiene (plan §5.8/§7.1): device serials, network endpoints and anything
/// the caller marks as secret must not land in the rolling log verbatim.
/// </summary>
public static partial class LogRedactor
{
    private const string Mask = "***";

    [GeneratedRegex(@"\b\d{1,3}(?:\.\d{1,3}){3}(?::\d{1,5})?\b", RegexOptions.CultureInvariant)]
    private static partial Regex IpRegex();

    /// <summary>
    /// Masks a device serial while keeping enough of it to correlate two log
    /// lines about the same device (first two / last two characters).
    /// </summary>
    public static string MaskSerial(string? serial)
    {
        if (string.IsNullOrWhiteSpace(serial))
        {
            return string.Empty;
        }

        if (serial.Length <= 4)
        {
            return Mask;
        }

        return $"{serial[..2]}{Mask}{serial[^2..]}";
    }

    /// <summary>Replaces every occurrence of each supplied secret, then scrubs IPv4 literals.</summary>
    public static string Redact(string? text, IEnumerable<string>? secrets = null)
    {
        if (string.IsNullOrEmpty(text))
        {
            return string.Empty;
        }

        var result = text;

        if (secrets is not null)
        {
            foreach (var secret in secrets)
            {
                if (!string.IsNullOrWhiteSpace(secret))
                {
                    result = result.Replace(secret, Mask, StringComparison.Ordinal);
                }
            }
        }

        return IpRegex().Replace(result, Mask);
    }
}
