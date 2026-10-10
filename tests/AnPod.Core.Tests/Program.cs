using AnPod.Core.Adb;
using AnPod.Core.Logging;
using AnPod.Core.Models;

// Minimal assertion harness. Exits non-zero on failure so CI can gate on it.
var checks = new List<(string Name, Action Body)>
{
    ("devices: parses every transport state", ParsesEveryTransportState),
    ("devices: ignores banner and junk lines", IgnoresBannerLines),
    ("devices: reads the usb: tag", ReadsUsbTag),
    ("devices: recognises 'no permissions'", RecognisesNoPermissions),
    ("serial: network endpoint detection", DetectsNetworkSerial),
    ("properties: order-independent parsing", ParsesLabelledProperties),
    ("classify: usb tag proves USB", UsbTagProvesUsb),
    ("classify: sys.usb.state proves USB without a usb: tag", UsbStateProvesUsb),
    ("classify: usb state lacking adb is not USB", UsbStateWithoutAdbIsNotUsb),
    ("classify: ip:port proves network", NetworkSerialProvesNetwork),
    ("classify: bare serial is unknown transport", BareSerialIsUnknownTransport),
    ("classify: only authorised USB is connectable", OnlyAuthorisedUsbIsConnectable),
    ("properties: query carries the transport keys", QueryCarriesTransportKeys),
    ("reverse: reads own mappings with a transport prefix", ParsesPrefixedReverseList),
    ("reverse: reads own mappings without a transport prefix", ParsesBareReverseList),
    ("reverse: never claims another tool's mapping", IgnoresForeignReverseMappings),
    ("settings: round-trips", SettingsRoundTrip),
    ("settings: corrupt json falls back to defaults", CorruptSettingsFallBack),
    ("settings: unknown schema falls back to defaults", UnknownSchemaFallsBack),
    ("settings: blank lastDeviceId normalises to null", BlankLastDeviceNormalises),
    ("redact: serial masking keeps correlatable ends", MasksSerial),
    ("redact: ip literals are scrubbed", ScrubsIps),
    ("classify: wifi transport is labelled for the list", WifiTransportIsLabelled),
    ("classify: each mode lists only its own devices", FiltersDevicesByMode),
    ("wireless: endpoint accepts plain and pasted forms", NormalisesEndpoints),
    ("wireless: endpoint rejects unusable input", RejectsBadEndpoints),
    ("wireless: pairing code is exactly six digits", ValidatesPairingCode),
    ("wireless: adb connect success is recognised", ParsesConnectSuccess),
    ("wireless: adb connect failure is caught despite exit code 0", ParsesConnectFailure),
    ("wireless: adb pair success and rejection are told apart", ParsesPairResults),
    ("wireless: mdns services lists both ports for one phone", ParsesMdnsServices),
    ("wireless: mdns parse survives layout changes and junk", ParsesMdnsRobustly),
    ("wireless: mdns check gates the device list", ProbesMdnsHealth),
    ("wireless: connect endpoint is only auto-picked when unambiguous", PicksConnectEndpoint),
    ("settings: wireless fields round-trip", SettingsWirelessRoundTrip),
    ("settings: v1.0 config without wireless fields still loads", OldSettingsStillLoad),
    ("settings: bogus transport and endpoint normalise", BogusWirelessSettingsNormalise),
};

int failures = 0;

foreach (var (name, body) in checks)
{
    try
    {
        body();
        Console.WriteLine($"  PASS  {name}");
    }
    catch (Exception ex)
    {
        failures++;
        Console.WriteLine($"  FAIL  {name}");
        Console.WriteLine($"        {ex.Message}");
    }
}

Console.WriteLine();
Console.WriteLine($"{checks.Count - failures}/{checks.Count} checks passed");

return failures == 0 ? 0 : 1;

// ---------------------------------------------------------------- tests

static void ParsesEveryTransportState()
{
    const string output =
        "List of devices attached\n" +
        "R58M12345AB\tdevice usb:1-3 product:beyond1ltezh device:beyond1 model:SM_G9730 transport_id:1\n" +
        "0123456789ABCDEF\tunauthorized usb:1-4 transport_id:2\n" +
        "192.168.1.20:5555\tdevice product:redfin model:Pixel_5 transport_id:3\n" +
        "emulator-5554\tdevice product:sdk_gphone_x86 model:sdk_gphone_x86 transport_id:4\n" +
        "ABC123\toffline usb:2-1 transport_id:5\n";

    var devices = AdbOutputParser.ParseDevices(output);

    Equal(5, devices.Count, "device count");
    Equal("R58M12345AB", devices[0].Serial, "serial 0");
    Equal(AdbDeviceState.Device, devices[0].State, "state 0");
    Equal(AdbDeviceState.Unauthorized, devices[1].State, "state 1");
    Equal(AdbDeviceState.Device, devices[2].State, "state 2");
    Equal(AdbDeviceState.Offline, devices[4].State, "state 4");
}

static void IgnoresBannerLines()
{
    const string output =
        "* daemon not running; starting now at tcp:5037\n" +
        "* daemon started successfully\n" +
        "List of devices attached\n" +
        "\n" +
        "R58M12345AB\tdevice usb:1-3 transport_id:1\n";

    var devices = AdbOutputParser.ParseDevices(output);

    Equal(1, devices.Count, "only the real device survives");
    Equal("R58M12345AB", devices[0].Serial, "serial");
}

static void ReadsUsbTag()
{
    var devices = AdbOutputParser.ParseDevices(
        "List of devices attached\nR58M12345AB\tdevice usb:1-3 transport_id:1\n");

    Equal("1-3", devices[0].UsbTag, "usb tag");
}

static void RecognisesNoPermissions()
{
    // The state spans two whitespace-separated columns upstream.
    var devices = AdbOutputParser.ParseDevices(
        "List of devices attached\nABC123\tno permissions (user in plugdev group); see [x]\n");

    Equal(1, devices.Count, "device count");
    Equal(AdbDeviceState.NoPermissions, devices[0].State, "state");
}

static void DetectsNetworkSerial()
{
    True(AdbOutputParser.IsNetworkSerial("192.168.1.20:5555"), "ip:port is network");
    True(AdbOutputParser.IsNetworkSerial("10.0.0.2:5037"), "second ip:port is network");
    False(AdbOutputParser.IsNetworkSerial("R58M12345AB"), "hardware serial is not network");
    False(AdbOutputParser.IsNetworkSerial("emulator-5554"), "emulator is not an ip:port");
    False(AdbOutputParser.IsNetworkSerial(null), "null is not network");
}

static void ParsesLabelledProperties()
{
    // Deliberately out of order with an interleaved blank line.
    const string output = "mo:SM-G9730\n\nsn:R58M12345AB\nav:13\nal:33\nmf:samsung\n";

    var props = AdbOutputParser.ParseLabeledProperties(output);

    Equal("SM-G9730", props["mo"], "model");
    Equal("R58M12345AB", props["sn"], "serial");
    Equal("13", props["av"], "android version");
    Equal("33", props["al"], "api level");
    Equal("samsung", props["mf"], "manufacturer");
}

static void UsbTagProvesUsb()
{
    var info = DeviceInfoFactory.Create(new AdbDeviceEntry("R58M12345AB", AdbDeviceState.Device, "1-3"));

    True(info.IsUsb, "usb confirmed");
    False(info.TransportUnknown, "transport known");
    True(info.IsConnectable, "connectable");
}

static void UsbStateProvesUsb()
{
    // Real-world case: platform-tools 37 on Android 16 omits the `usb:` tag
    // from `devices -l`, so the row carries no transport evidence at all.
    // sys.usb.state is the second opinion plan §5.2 asks for.
    var props = new Dictionary<string, string> { ["us"] = "mtp,adb" };

    var info = DeviceInfoFactory.Create(
        new AdbDeviceEntry("10ACCH06X1000XM", AdbDeviceState.Device, null),
        props);

    True(info.IsUsb, "usb confirmed from sys.usb.state");
    False(info.TransportUnknown, "transport known");
    True(info.IsConnectable, "connectable");
}

static void UsbStateWithoutAdbIsNotUsb()
{
    // Plugged in for charging/MTP only: ADB is not on the USB bus, so this must
    // stay unknown rather than being assumed connectable.
    var props = new Dictionary<string, string> { ["us"] = "mtp" };

    var info = DeviceInfoFactory.Create(
        new AdbDeviceEntry("ABC123", AdbDeviceState.Device, null),
        props);

    False(info.IsUsb, "mtp alone is not USB ADB");
    True(info.TransportUnknown, "transport still unknown");
    False(info.IsConnectable, "must not auto-connect");
}

static void ParsesPrefixedReverseList()
{
    // platform-tools 37 output. A start-of-line match finds nothing here, which
    // is what left stale mappings behind after a force-kill.
    const string output =
        "UsbFfs localabstract:anpod_1 tcp:6043\n" +
        "UsbFfs localabstract:anpod_999 tcp:1\n";

    var names = AdbOutputParser.ParseOwnReverseMappings(output);

    Equal(2, names.Count, "mapping count");
    Equal("localabstract:anpod_1", names[0], "first mapping");
    Equal("localabstract:anpod_999", names[1], "second mapping");
}

static void ParsesBareReverseList()
{
    // Older releases emitted the mapping alone.
    const string output = "localabstract:anpod_7 tcp:5000\n";

    var names = AdbOutputParser.ParseOwnReverseMappings(output);

    Equal(1, names.Count, "mapping count");
    Equal("localabstract:anpod_7", names[0], "mapping");
}

static void IgnoresForeignReverseMappings()
{
    const string output =
        "UsbFfs localabstract:someothertool_session tcp:9\n" +
        "UsbFfs tcp:8080 tcp:8080\n" +
        "UsbFfs localabstract:anpod_2 tcp:5001\n";

    var names = AdbOutputParser.ParseOwnReverseMappings(output);

    Equal(1, names.Count, "only AnPod's mapping is claimed");
    Equal("localabstract:anpod_2", names[0], "claimed mapping");
    Equal(0, AdbOutputParser.ParseOwnReverseMappings(null).Count, "null yields nothing");
    Equal(0, AdbOutputParser.ParseOwnReverseMappings("").Count, "empty yields nothing");
}

static void QueryCarriesTransportKeys()
{
    // The transport keys ride along in the existing per-device shell call, so
    // the extra classification costs no additional ADB round-trip.
    True(AdbOutputParser.PropertyQueryCommand.Contains("us:$(getprop sys.usb.state)"),
        "query reads sys.usb.state");
    True(AdbOutputParser.PropertyQueryCommand.Contains("tp:$(getprop service.adb.tcp.port)"),
        "query reads service.adb.tcp.port");

    // ParseLabeledProperties drops keys longer than 4 characters.
    var props = AdbOutputParser.ParseLabeledProperties("us:mtp,adb\ntp:\n");
    Equal("mtp,adb", props["us"], "us key survives parsing");
}

static void NetworkSerialProvesNetwork()
{
    var info = DeviceInfoFactory.Create(
        new AdbDeviceEntry("192.168.1.20:5555", AdbDeviceState.Device, null));

    False(info.IsUsb, "not usb");
    False(info.TransportUnknown, "transport positively identified as network");
    Equal(TransportKind.Wifi, info.Transport, "transport is wifi");
    // v1.1 (plan §1.2): a paired wireless device is a first-class target, not a
    // device we merely tolerate. USB and Wi-Fi are exclusive sessions, not a
    // supported/unsupported split.
    True(info.IsConnectable, "wifi devices are connectable in v1.1");
}

static void BareSerialIsUnknownTransport()
{
    // Plan §5.2: a serial without a colon is NOT proof of USB.
    var info = DeviceInfoFactory.Create(
        new AdbDeviceEntry("emulator-5554", AdbDeviceState.Device, null));

    False(info.IsUsb, "not usb");
    True(info.TransportUnknown, "transport unknown");
    False(info.IsConnectable, "must not auto-connect");
}

static void OnlyAuthorisedUsbIsConnectable()
{
    var unauthorized = DeviceInfoFactory.Create(
        new AdbDeviceEntry("R58M12345AB", AdbDeviceState.Unauthorized, "1-3"));

    False(unauthorized.IsConnectable, "unauthorized is not connectable");
    True(unauthorized.IsUsb, "but it is still recognised as USB");
}

static void SettingsRoundTrip()
{
    var original = new AppSettings
    {
        AutoConnectLastDevice = false,
        LastDeviceId = "R58M12345AB",
        MinimizeToTray = false,
        LaunchAtLogin = true,
    };

    var restored = AppSettingsSerializer.Deserialize(AppSettingsSerializer.Serialize(original));

    Equal(false, restored.AutoConnectLastDevice, "auto connect");
    Equal("R58M12345AB", restored.LastDeviceId, "last device");
    Equal(false, restored.MinimizeToTray, "minimize to tray");
    Equal(true, restored.LaunchAtLogin, "launch at login");
    Equal(AppSettings.CurrentSchemaVersion, restored.SchemaVersion, "schema");
}

static void CorruptSettingsFallBack()
{
    var restored = AppSettingsSerializer.Deserialize("{ this is not json");

    Equal(true, restored.AutoConnectLastDevice, "default auto connect");
    Equal(true, restored.MinimizeToTray, "default minimize to tray");
    Equal(null, restored.LastDeviceId, "default last device");
}

static void UnknownSchemaFallsBack()
{
    var restored = AppSettingsSerializer.Deserialize(
        "{\"schemaVersion\":99,\"autoConnectLastDevice\":false,\"minimizeToTray\":false}");

    Equal(true, restored.AutoConnectLastDevice, "unreadable schema must not be honoured");
    Equal(true, restored.MinimizeToTray, "unreadable schema must not be honoured");
}

static void BlankLastDeviceNormalises()
{
    var restored = AppSettingsSerializer.Deserialize(
        "{\"schemaVersion\":1,\"lastDeviceId\":\"   \"}");

    Equal(null, restored.LastDeviceId, "blank serial becomes null");
}

static void MasksSerial()
{
    var masked = LogRedactor.MaskSerial("R58M12345AB");

    Equal("R5***AB", masked, "masked serial");
    Equal("***", LogRedactor.MaskSerial("ABC"), "short serial fully masked");
}

static void ScrubsIps()
{
    var scrubbed = LogRedactor.Redact(
        "connecting to 127.0.0.1:5000 and 192.168.1.20:5555 for R58M12345AB",
        new[] { "R58M12345AB" });

    False(scrubbed.Contains("127.0.0.1"), "loopback scrubbed");
    False(scrubbed.Contains("192.168.1.20"), "lan ip scrubbed");
    False(scrubbed.Contains("R58M12345AB"), "secret scrubbed");
}

static void WifiTransportIsLabelled()
{
    var wifi = DeviceInfoFactory.Create(
        new AdbDeviceEntry("192.168.1.20:5555", AdbDeviceState.Device, null));
    var usb = DeviceInfoFactory.Create(
        new AdbDeviceEntry("R58M12345AB", AdbDeviceState.Device, "1-3"));

    Equal("Wi-Fi", wifi.TransportText, "wifi label");
    Equal("USB", usb.TransportText, "usb label");
    Equal("已连接 Wi-Fi", wifi.StatusText, "wifi status line");
    Equal("已连接 USB", usb.StatusText, "usb status line");

    // An unauthorised wireless device behaves exactly like an unauthorised USB
    // one: visible, with its transport stated, but not connectable (plan §6).
    var pending = DeviceInfoFactory.Create(
        new AdbDeviceEntry("192.168.1.20:5555", AdbDeviceState.Unauthorized, null));

    False(pending.IsConnectable, "unauthorised wifi is not connectable");
    Equal(TransportKind.Wifi, pending.Transport, "transport is still known");
    Equal("等待手机授权", pending.StatusText, "authorisation guidance");
}

// One phone reachable both ways appears twice in `adb devices` — once under its
// USB serial, once as ip:port. Each mode must list only its own, or choosing
// Wi-Fi still shows a row reading "已连接 USB" and there is no way to tell which
// device a click will act on.
static void FiltersDevicesByMode()
{
    var wifi = DeviceInfoFactory.Create(
        new AdbDeviceEntry("192.168.1.20:5555", AdbDeviceState.Device, null));
    var usb = DeviceInfoFactory.Create(
        new AdbDeviceEntry("R58M12345AB", AdbDeviceState.Device, "1-3"));

    True(usb.MatchesTransport(TransportKind.Usb), "usb device shows in usb mode");
    False(usb.MatchesTransport(TransportKind.Wifi), "usb device is hidden in wifi mode");

    True(wifi.MatchesTransport(TransportKind.Wifi), "wifi device shows in wifi mode");
    False(wifi.MatchesTransport(TransportKind.Usb), "wifi device is hidden in usb mode");

    // An unclassified device is listed in both modes on purpose: it cannot be
    // attributed to either, and hiding it would empty both lists while a phone
    // is plainly attached.
    var unknown = DeviceInfoFactory.Create(
        new AdbDeviceEntry("R58M12345AB", AdbDeviceState.Device, null));

    Equal(TransportKind.Unknown, unknown.Transport, "a bare serial stays unclassified");
    True(unknown.MatchesTransport(TransportKind.Usb), "unclassified is visible in usb mode");
    True(unknown.MatchesTransport(TransportKind.Wifi), "unclassified is visible in wifi mode");

    // ...but it is never usable, which is what keeps it from being auto-connected.
    False(unknown.IsConnectable, "unclassified is never connectable");

    // The two rows for one phone filter down to exactly one per mode.
    var all = new[] { usb, wifi };
    Equal(1, all.Count(d => d.MatchesTransport(TransportKind.Usb)), "one row in usb mode");
    Equal(1, all.Count(d => d.MatchesTransport(TransportKind.Wifi)), "one row in wifi mode");
}

static void NormalisesEndpoints()
{
    True(WirelessAdb.TryNormalizeEndpoint("192.168.1.20:5555", out var plain, out _), "plain form");
    Equal("192.168.1.20:5555", plain, "plain endpoint");

    // Tutorials hand out the whole command; pasting it must just work.
    True(WirelessAdb.TryNormalizeEndpoint("  adb connect 10.0.0.5:37099 ", out var pasted, out _), "pasted form");
    Equal("10.0.0.5:37099", pasted, "pasted endpoint");

    True(WirelessAdb.TryNormalizeEndpoint("adb pair 10.0.0.5:37099", out var paired, out _), "pair form");
    Equal("10.0.0.5:37099", paired, "pair endpoint");
}

static void RejectsBadEndpoints()
{
    False(WirelessAdb.TryNormalizeEndpoint(null, out _, out var empty), "null rejected");
    True(empty is not null, "null explains itself");
    False(WirelessAdb.TryNormalizeEndpoint("   ", out _, out _), "blank rejected");

    // The pairing port and the connect port differ on Android 11+, so a missing
    // port is the single most common mistake (plan §1.1a) and has to be named.
    False(WirelessAdb.TryNormalizeEndpoint("192.168.1.20", out _, out var noPort), "port required");
    True(noPort!.Contains("IP:端口"), "missing port explains the expected shape");

    False(WirelessAdb.TryNormalizeEndpoint("192.168.1.20:0", out _, out _), "port 0 rejected");
    False(WirelessAdb.TryNormalizeEndpoint("192.168.1.20:70000", out _, out _), "port too large rejected");
    False(WirelessAdb.TryNormalizeEndpoint("192.168.1.20:+55", out _, out _), "signed port rejected");
    False(WirelessAdb.TryNormalizeEndpoint("my-laptop:5555", out _, out _), "hostname rejected");
    False(WirelessAdb.TryNormalizeEndpoint("192.168.1.20:55a5", out _, out _), "non-numeric port rejected");
}

static void ValidatesPairingCode()
{
    True(WirelessAdb.IsValidPairingCode("123456"), "six digits");
    True(WirelessAdb.IsValidPairingCode(" 654321 "), "surrounding space is tolerated");
    False(WirelessAdb.IsValidPairingCode("12345"), "five digits rejected");
    False(WirelessAdb.IsValidPairingCode("1234567"), "seven digits rejected");
    False(WirelessAdb.IsValidPairingCode("12345a"), "letters rejected");
    False(WirelessAdb.IsValidPairingCode(null), "null rejected");
    False(WirelessAdb.IsValidPairingCode(""), "empty rejected");
}

static void ParsesConnectSuccess()
{
    Equal(AdbConnectOutcome.Connected,
        WirelessAdb.ParseConnectResult("connected to 192.168.1.20:5555\n", string.Empty),
        "fresh connection");

    Equal(AdbConnectOutcome.AlreadyConnected,
        WirelessAdb.ParseConnectResult("already connected to 192.168.1.20:5555\n", string.Empty),
        "repeat connection");

    // No output at all is never success; guessing here would report a phone that
    // never answered as connected (plan §3.4.1).
    Equal(AdbConnectOutcome.Unknown,
        WirelessAdb.ParseConnectResult(string.Empty, string.Empty),
        "no output is not success");

    // A port can look like a Windows socket error code. Only the message counts.
    Equal(AdbConnectOutcome.Connected,
        WirelessAdb.ParseConnectResult("connected to 192.168.1.20:10061\n", string.Empty),
        "a port resembling an error code is not an error");

    // A benign stderr note must not out-vote the success line.
    Equal(AdbConnectOutcome.Connected,
        WirelessAdb.ParseConnectResult(
            "connected to 192.168.1.20:5555\n",
            "adb server version (41) doesn't match this client (39)\n"),
        "success survives an unrelated stderr note");
}

static void ParsesConnectFailure()
{
    // adb connect exits 0 for every one of these, which is the whole reason the
    // text has to be classified rather than the exit code trusted.
    Equal(AdbConnectOutcome.Failed,
        WirelessAdb.ParseConnectResult("failed to connect to 192.168.1.20:5555\n", string.Empty),
        "failed to connect");
    Equal(AdbConnectOutcome.Failed,
        WirelessAdb.ParseConnectResult("unable to connect to 192.168.1.20:5555: Connection refused\n", string.Empty),
        "unable to connect");
    Equal(AdbConnectOutcome.Failed,
        WirelessAdb.ParseConnectResult("cannot connect to 192.168.1.20:5555: 由于目标计算机积极拒绝 (10061)\n", string.Empty),
        "localised refusal");
    Equal(AdbConnectOutcome.Failed,
        WirelessAdb.ParseConnectResult(string.Empty, "missing port in specification: 192.168.1.20\n"),
        "missing port");
    Equal(AdbConnectOutcome.Failed,
        WirelessAdb.ParseConnectResult("failed to connect to 192.168.1.20:5555: error: 10060\n", string.Empty),
        "socket error code");
}

static void ParsesPairResults()
{
    Equal(AdbPairOutcome.Succeeded,
        WirelessAdb.ParsePairResult(
            "Successfully paired to 192.168.1.20:37099 [guid=adb-R58M12345AB-xyz]\n",
            string.Empty,
            out _),
        "pair success");

    var outcome = WirelessAdb.ParsePairResult(string.Empty, "Failed: Wrong password\n", out var detail);
    Equal(AdbPairOutcome.Failed, outcome, "a wrong code is a failure");
    Equal("Failed: Wrong password", detail, "the reason is surfaced");

    Equal(AdbPairOutcome.Unknown,
        WirelessAdb.ParsePairResult(string.Empty, string.Empty, out _),
        "no output is not success");

    // A pathological line must not flood the status line.
    WirelessAdb.ParsePairResult(new string('x', 400) + " failed\n", string.Empty, out var longDetail);
    True(longDetail!.Length <= 200, "detail is truncated");
}

// The real shape of `adb mdns services`. The whole point of discovery is that
// the phone advertises the pairing port and the connect port as two separate
// services, which is what removes the need for the user to copy the right one
// off the screen — so the test asserts both are read and told apart.
static void ParsesMdnsServices()
{
    const string output =
        "List of discovered mdns services\n" +
        "adb-R58M12345AB-4KPB2X._adb-tls-connect._tcp.\t192.168.1.20:37041\n" +
        "adb-R58M12345AB-ZZ9QQ1._adb-tls-pairing._tcp.\t192.168.1.20:41287\n";

    var services = WirelessAdb.ParseMdnsServices(output);
    Equal(2, services.Count, "both services are parsed");

    var connect = services.Single(s => s.Kind == AdbMdnsKind.Connect);
    Equal("192.168.1.20:37041", connect.Endpoint, "connect endpoint");
    True(connect.Name.StartsWith("adb-R58M12345AB", StringComparison.Ordinal), "instance name kept");

    var pairing = services.Single(s => s.Kind == AdbMdnsKind.Pairing);
    Equal("192.168.1.20:41287", pairing.Endpoint, "pairing endpoint");

    // The two ports really are different values — the trap this exists to close.
    True(connect.Endpoint != pairing.Endpoint, "the two ports differ");

    Equal(0, WirelessAdb.ParseMdnsServices("List of discovered mdns services\n").Count,
        "an empty list is empty, not an error");
}

static void ParsesMdnsRobustly()
{
    // Column layout has moved between platform-tools releases, so the parser
    // finds the endpoint wherever it is rather than trusting fixed positions.
    var tabs = WirelessAdb.ParseMdnsServices(
        "adb-X-1._adb-tls-connect._tcp\t_adb-tls-connect._tcp\t10.0.0.5:5555\n");
    Equal("10.0.0.5:5555", tabs.Single().Endpoint, "extra type column");

    var legacy = WirelessAdb.ParseMdnsServices("adb-X-1._adb._tcp.  10.0.0.5:5555\n");
    Equal(AdbMdnsKind.Connect, legacy.Single().Kind, "legacy _adb._tcp counts as connect");

    var other = WirelessAdb.ParseMdnsServices("printer._ipp._tcp.  10.0.0.9:631\n");
    Equal(AdbMdnsKind.Other, other.Single().Kind, "an unrelated service is not an ADB device");

    // Junk must not become a device, and must not throw.
    Equal(0, WirelessAdb.ParseMdnsServices("\n\n   \nnot an address\n").Count, "junk is ignored");
    Equal(0, WirelessAdb.ParseMdnsServices(null).Count, "null is tolerated");
}

// The outage probe and the device list are two different commands, and this is
// why: the list prints its header with exit code 0 in every state, so only
// `mdns check` can say whether the search works at all. Measured on a wedged
// server, `mdns services` answered in 0.05 s with rc 0 while `mdns check` took
// 11.25 s to report the outage — and 0.05 s to confirm a healthy daemon. These
// assertions pin that contract so the two probes cannot be quietly swapped back.
static void ProbesMdnsHealth()
{
    // Healthy, warm: what a working daemon prints.
    True(WirelessAdb.IsMdnsHealthy("mdns daemon version [adb discovery 0.0.0]\n", string.Empty),
        "a version line means the daemon is up");

    // Healthy, cold: adb spawns the server and announces it on stderr while the
    // version still arrives on stdout. Both streams are read, so this stays
    // healthy — a probe that watched only stderr would call it an outage and
    // send the user off to debug a phone that was fine.
    True(WirelessAdb.IsMdnsHealthy(
            "mdns daemon version [adb discovery 0.0.0]\n",
            "* daemon not running; starting now at tcp:5039\n* daemon started successfully\n"),
        "a cold start is healthy, not an outage");

    // The wedged server's actual words, which it takes eleven seconds to say.
    const string wedged = "ERROR: mdns daemon unavailable\n";
    True(!WirelessAdb.IsMdnsHealthy(wedged, string.Empty), "the outage is not healthy");
    True(WirelessAdb.IsMdnsUnavailable(wedged, string.Empty),
        "and is reported as unavailable, so the message can say retrying is pointless");

    // A timeout yields no output at all: not healthy, and not provably
    // "unavailable" either — a third state the caller must still handle.
    True(!WirelessAdb.IsMdnsHealthy(null, string.Empty), "silence is not healthy");
    True(!WirelessAdb.IsMdnsUnavailable(null, string.Empty),
        "silence is not a definitive outage");

    // The trap this whole two-step design exists to avoid: the services list
    // proves nothing about the daemon, in either direction — not even when it
    // has a real device in it. Believing it would report a dead daemon as
    // "no devices found" and send the user to debug the wrong end.
    const string listHeader = "List of discovered mdns services\n";
    const string listWithDevice =
        "List of discovered mdns services\nadb-A-1._adb-tls-connect._tcp.  10.0.0.5:5555\n";

    True(!WirelessAdb.IsMdnsHealthy(listHeader, string.Empty),
        "the services header never proves health");
    True(!WirelessAdb.IsMdnsHealthy(listWithDevice, string.Empty),
        "nor does a populated list, which is the same file over a dead daemon");
    True(!WirelessAdb.IsMdnsUnavailable(listHeader, string.Empty),
        "the services header never proves an outage either");
}

static void PicksConnectEndpoint()
{
    var one = WirelessAdb.ParseMdnsServices(
        "adb-A-1._adb-tls-connect._tcp.  10.0.0.5:5555\n");
    Equal("10.0.0.5:5555", WirelessAdb.PickConnectEndpoint(one, null),
        "a single device needs no disambiguation");

    // Two phones: the one just paired with wins, by host.
    var two = WirelessAdb.ParseMdnsServices(
        "adb-A-1._adb-tls-connect._tcp.  10.0.0.5:5555\n" +
        "adb-B-1._adb-tls-connect._tcp.  10.0.0.6:6666\n");
    Equal("10.0.0.6:6666", WirelessAdb.PickConnectEndpoint(two, "10.0.0.6"),
        "the paired host is preferred");

    // No basis for a choice: refusing beats silently routing audio to a phone the
    // user did not mean (plan §6).
    Equal(null, WirelessAdb.PickConnectEndpoint(two, "10.0.0.99"),
        "an unrelated host does not justify a guess");
    Equal(null, WirelessAdb.PickConnectEndpoint(two, null),
        "two phones and no hint is ambiguous");

    Equal(null, WirelessAdb.PickConnectEndpoint(Array.Empty<AdbMdnsService>(), "10.0.0.5"),
        "nothing discovered, nothing picked");

    Equal("10.0.0.5", WirelessAdb.HostOf("10.0.0.5:5555"), "host extraction");
    Equal(null, WirelessAdb.HostOf("nonsense"), "host of nonsense is null");
}

static void SettingsWirelessRoundTrip(){
    var original = new AppSettings
    {
        PreferredTransport = TransportKind.Wifi,
        AutoReconnectWifi = false,
        LastWirelessEndpoint = "192.168.1.20:5555",
    };

    var restored = AppSettingsSerializer.Deserialize(AppSettingsSerializer.Serialize(original));

    Equal(TransportKind.Wifi, restored.PreferredTransport, "transport");
    Equal(false, restored.AutoReconnectWifi, "auto reconnect wifi");
    Equal("192.168.1.20:5555", restored.LastWirelessEndpoint, "endpoint");
}

static void OldSettingsStillLoad()
{
    // A file written by v1.0 has none of the wireless fields. It must keep its
    // values and gain defaults, not be discarded for being old (plan §5.10).
    var restored = AppSettingsSerializer.Deserialize(
        "{\"schemaVersion\":1,\"autoConnectLastDevice\":false,\"lastDeviceId\":\"R58M12345AB\"," +
        "\"minimizeToTray\":false,\"launchAtLogin\":true}");

    Equal(false, restored.AutoConnectLastDevice, "v1.0 value kept");
    Equal("R58M12345AB", restored.LastDeviceId, "v1.0 value kept");
    Equal(true, restored.LaunchAtLogin, "v1.0 value kept");
    Equal(TransportKind.Usb, restored.PreferredTransport, "transport defaults to USB");
    Equal(true, restored.AutoReconnectWifi, "wifi reconnect defaults on");
    Equal(null, restored.LastWirelessEndpoint, "no endpoint stored yet");
}

static void BogusWirelessSettingsNormalise()
{
    // An unrecognised transport name is not convertible, so the document counts
    // as unreadable and defaults are used (plan §5.8). What matters is that the
    // app still ends up with a usable transport rather than an undefined mode.
    var unreadable = AppSettingsSerializer.Deserialize(
        "{\"schemaVersion\":1,\"preferredTransport\":\"bluetooth\"}");
    Equal(TransportKind.Usb, unreadable.PreferredTransport, "unreadable transport yields USB");

    // A stale or hand-edited address is dropped rather than retried forever
    // (plan §1.1a: do not keep retrying an address that no longer exists).
    var stale = AppSettingsSerializer.Normalize(new AppSettings
    {
        LastWirelessEndpoint = "not an address",
    });
    Equal(null, stale.LastWirelessEndpoint, "unusable endpoint is dropped");

    // Unknown is representable in the enum but is not a choice a user can make.
    var unknown = AppSettingsSerializer.Normalize(new AppSettings
    {
        PreferredTransport = TransportKind.Unknown,
    });
    Equal(TransportKind.Usb, unknown.PreferredTransport, "Unknown is not a preference");
}

// ---------------------------------------------------------------- helpers
static void Equal<T>(T expected, T actual, string what)
{
    if (!EqualityComparer<T>.Default.Equals(expected, actual))
    {
        throw new InvalidOperationException($"{what}: expected <{expected}>, got <{actual}>");
    }
}

static void True(bool value, string what)
{
    if (!value)
    {
        throw new InvalidOperationException($"{what}: expected true");
    }
}

static void False(bool value, string what)
{
    if (value)
    {
        throw new InvalidOperationException($"{what}: expected false");
    }
}
