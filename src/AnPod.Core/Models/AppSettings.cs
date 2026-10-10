using System.Text.Json;
using System.Text.Json.Serialization;
using AnPod.Core.Adb;

namespace AnPod.Core.Models;

/// <summary>
/// Persisted user settings (plan §5.8/§5.10). Deliberately tiny — every field
/// here changes observable behaviour, so there is nothing speculative to prune.
/// <para>
/// The wireless pairing <em>code</em> is intentionally absent: it lives in memory
/// for the length of one <c>adb pair</c> call and is never written to settings,
/// logs or diagnostics (plan §5.9).
/// </para>
/// </summary>
public sealed class AppSettings
{
    /// <summary>Bumped only when the on-disk shape changes incompatibly.</summary>
    public const int CurrentSchemaVersion = 1;

    [JsonPropertyName("schemaVersion")]
    public int SchemaVersion { get; set; } = CurrentSchemaVersion;

    /// <summary>Reconnect the last used device automatically when it reappears.</summary>
    [JsonPropertyName("autoConnectLastDevice")]
    public bool AutoConnectLastDevice { get; set; } = true;

    /// <summary>
    /// Serial of the last successfully connected device. Null until the first
    /// successful session. Stored locally only, never transmitted.
    /// </summary>
    [JsonPropertyName("lastDeviceId")]
    public string? LastDeviceId { get; set; }

    /// <summary>Closing the main window hides to tray instead of exiting.</summary>
    [JsonPropertyName("minimizeToTray")]
    public bool MinimizeToTray { get; set; } = true;

    /// <summary>Register a per-user run-at-login entry. Off by default.</summary>
    [JsonPropertyName("launchAtLogin")]
    public bool LaunchAtLogin { get; set; }

    /// <summary>
    /// Transport the user last selected (plan §5.10). Defaults to USB: the cable
    /// path needs no pairing and works on every supported Android version.
    /// </summary>
    [JsonPropertyName("preferredTransport")]
    [JsonConverter(typeof(JsonStringEnumConverter<TransportKind>))]
    public TransportKind PreferredTransport { get; set; } = TransportKind.Usb;

    /// <summary>
    /// When auto-connect is on and the last device was reached over Wi-Fi, also
    /// try to re-establish the wireless ADB link (plan §5.10). A phone that has
    /// gone to sleep leaves the ADB-over-TCP endpoint gone entirely, so simply
    /// waiting for it to reappear in <c>adb devices</c> would never succeed.
    /// Ignored while <see cref="AutoConnectLastDevice"/> is off.
    /// </summary>
    [JsonPropertyName("autoReconnectWifi")]
    public bool AutoReconnectWifi { get; set; } = true;

    /// <summary>
    /// Last wireless endpoint that accepted a connection. Plan §1.1a: the phone's
    /// address and port can change, so this is a hint for reconnection and never
    /// retried forever — the user can always type a new one.
    /// </summary>
    [JsonPropertyName("lastWirelessEndpoint")]
    public string? LastWirelessEndpoint { get; set; }

    public AppSettings Clone() => new()
    {
        SchemaVersion = SchemaVersion,
        AutoConnectLastDevice = AutoConnectLastDevice,
        LastDeviceId = LastDeviceId,
        MinimizeToTray = MinimizeToTray,
        LaunchAtLogin = LaunchAtLogin,
        PreferredTransport = PreferredTransport,
        AutoReconnectWifi = AutoReconnectWifi,
        LastWirelessEndpoint = LastWirelessEndpoint,
    };
}

/// <summary>
/// Never-throwing (de)serialisation so a corrupt settings file degrades to
/// defaults instead of preventing startup (plan §5.8: "坏配置用默认值恢复").
/// </summary>
public static class AppSettingsSerializer
{
    private static readonly JsonSerializerOptions WriteOptions = new()
    {
        WriteIndented = true,
    };

    /// <summary>Deserialises <paramref name="json"/>; returns defaults on any failure.</summary>
    public static AppSettings Deserialize(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return new AppSettings();
        }

        try
        {
            var parsed = JsonSerializer.Deserialize<AppSettings>(json);
            return Normalize(parsed);
        }
        catch (JsonException)
        {
            return new AppSettings();
        }
        catch (NotSupportedException)
        {
            return new AppSettings();
        }
    }

    public static string Serialize(AppSettings settings) =>
        JsonSerializer.Serialize(Normalize(settings), WriteOptions);

    /// <summary>Coerces an out-of-range/foreign document back into a valid one.</summary>
    public static AppSettings Normalize(AppSettings? settings)
    {
        var source = settings ?? new AppSettings();

        // A document from a newer schema is not understood; fall back to defaults
        // rather than silently honouring fields we cannot interpret.
        if (source.SchemaVersion != AppSettings.CurrentSchemaVersion)
        {
            return new AppSettings();
        }

        return new AppSettings
        {
            SchemaVersion = AppSettings.CurrentSchemaVersion,
            AutoConnectLastDevice = source.AutoConnectLastDevice,
            LastDeviceId = string.IsNullOrWhiteSpace(source.LastDeviceId) ? null : source.LastDeviceId,
            MinimizeToTray = source.MinimizeToTray,
            LaunchAtLogin = source.LaunchAtLogin,
            PreferredTransport = NormalizeTransport(source.PreferredTransport),
            AutoReconnectWifi = source.AutoReconnectWifi,
            LastWirelessEndpoint = NormalizeEndpoint(source.LastWirelessEndpoint),
        };
    }

    /// <summary>
    /// Unknown is not a transport the user can pick, so a stored Unknown (a hand
    /// edit, or a value written by a build that knew a transport this one does
    /// not) falls back to USB instead of leaving the app with no usable mode.
    /// </summary>
    private static TransportKind NormalizeTransport(TransportKind transport) =>
        transport is TransportKind.Usb or TransportKind.Wifi ? transport : TransportKind.Usb;

    /// <summary>
    /// Keeps only a syntactically valid <c>ip:port</c>; anything else becomes
    /// null so a stale or hand-edited address cannot drive a reconnect.
    /// </summary>
    private static string? NormalizeEndpoint(string? endpoint) =>
        WirelessAdb.TryNormalizeEndpoint(endpoint, out var normalized, out _) ? normalized : null;
}
