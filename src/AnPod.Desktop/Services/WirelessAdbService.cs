using AnPod.Core.Adb;
using AnPod.Core.Logging;

namespace AnPod.Desktop.Services;

/// <summary>
/// Outcome of a wireless ADB step, already phrased for the status line.
/// </summary>
/// <param name="Endpoint">
/// The normalised <c>ip:port</c> when the input was usable, so the caller can
/// remember it without re-parsing what the user typed.
/// </param>
public sealed record WirelessResult(bool Success, string Message, string? Endpoint = null)
{
    public static WirelessResult Ok(string message, string? endpoint = null) =>
        new(true, message, endpoint);

    public static WirelessResult Fail(string message, string? endpoint = null) =>
        new(false, message, endpoint);
}

/// <summary>
/// What <c>adb mdns services</c> currently knows about.
/// </summary>
/// <param name="Available">
/// False when the search itself could not be trusted: the ADB server's mDNS
/// daemon is not running, or it did not answer the health probe in time.
/// Distinguishing this from "found nothing" matters: an empty list means the
/// phone is not advertising, while an unavailable daemon means the search cannot
/// work at all and the user should stop retrying and type the address instead.
/// </param>
public sealed record WirelessDiscovery(
    bool Available,
    string Message,
    IReadOnlyList<AdbMdnsService> Connect,
    IReadOnlyList<AdbMdnsService> Pairing)
{
    public static WirelessDiscovery Unavailable(string message) =>
        new(false, message, Array.Empty<AdbMdnsService>(), Array.Empty<AdbMdnsService>());
}

/// <summary>
/// The wireless half of the ADB control path (plan §5.9): pairing, connecting
/// and dropping a single TCP endpoint.
/// <para>
/// It owns no audio logic and does not touch the Android payload. Once
/// <c>adb connect</c> has produced an authorised device, the session is built by
/// the same <c>adb reverse</c> + <c>app_process</c> + PCM16 path USB uses — the
/// only difference is the transport the serial refers to (plan §2.4).
/// </para>
/// </summary>
public sealed class WirelessAdbService
{
    /// <summary>Pairing includes a round trip through the phone's dialog.</summary>
    private static readonly TimeSpan PairTimeout = TimeSpan.FromSeconds(20);

    private static readonly TimeSpan ConnectTimeout = TimeSpan.FromSeconds(10);
    private static readonly TimeSpan DisconnectTimeout = TimeSpan.FromSeconds(5);

    /// <summary>
    /// <c>adb mdns services</c> prints what the server has already discovered and
    /// returns immediately — measured at 0.05 s, warm or cold, healthy or wedged.
    /// It is a lookup, not a search that waits, so this is only a guard against a
    /// server that has stopped answering entirely.
    /// </summary>
    private static readonly TimeSpan DiscoverTimeout = TimeSpan.FromSeconds(6);

    /// <summary>
    /// Budget for the <c>adb mdns check</c> health probe, whose timeout is not a
    /// safety net but the signal itself.
    /// </summary>
    /// <remarks>
    /// Measured on this machine, three states, three very different durations:
    /// <list type="bullet">
    /// <item>healthy daemon, server warm — <b>0.05 s</b></item>
    /// <item>healthy daemon, server cold (the probe pays to spawn it) — <b>2.14 s</b></item>
    /// <item>wedged daemon — <b>11.25 s</b>, then <c>ERROR: mdns daemon unavailable</c></item>
    /// </list>
    /// Five seconds clears the cold-start cost with room to spare on a loaded
    /// machine while staying well under the eleven-second wedge, so "did not
    /// answer in time" and "answered that it is broken" collapse into one honest
    /// verdict. A tighter budget is actively wrong: at two seconds a legitimate
    /// cold start would be aborted and reported as an outage, which is the same
    /// class of mistake as reporting an outage as "no devices" — just inverted.
    /// </remarks>
    private static readonly TimeSpan MdnsProbeTimeout = TimeSpan.FromSeconds(5);

    private readonly AdbRunner _adb;
    private readonly ILog _log;

    public WirelessAdbService(AdbRunner adb, ILog log)
    {
        _adb = adb;
        _log = log;
    }

    /// <summary>
    /// The address that most recently accepted a pairing code, or null.
    /// </summary>
    /// <remarks>
    /// Held so the two things users actually get wrong can be answered: which of
    /// several advertised phones is the one they just paired with, and whether
    /// the address they are about to connect to is the pairing port rather than
    /// the connect port. Never persisted (plan §5.9).
    /// </remarks>
    public string? LastPairingEndpoint { get; private set; }

    /// <summary>
    /// Runs <c>adb pair &lt;endpoint&gt; &lt;code&gt;</c> (Android 11+).
    /// </summary>
    /// <remarks>
    /// The code is registered as a log secret <em>before</em> the command runs,
    /// because <see cref="AdbRunner"/> logs its argument vector when a command
    /// fails to start and when it exits non-zero. Registering it first is what
    /// keeps the code out of the rolling log (plan §5.9).
    /// </remarks>
    public async Task<WirelessResult> PairAsync(
        string? endpoint,
        string? code,
        CancellationToken cancellationToken)
    {
        if (!WirelessAdb.TryNormalizeEndpoint(endpoint, out var normalized, out var problem))
        {
            return WirelessResult.Fail(problem ?? "请填写手机的 IP:端口。");
        }

        if (!WirelessAdb.IsValidPairingCode(code))
        {
            return WirelessResult.Fail("配对码是手机弹窗里的 6 位数字，请重新填写。", normalized);
        }

        var trimmedCode = code!.Trim();
        _log.AddSecret(trimmedCode);

        var result = await _adb
            .RunAsync(
                serial: null,
                new[] { "pair", normalized, trimmedCode },
                PairTimeout,
                cancellationToken)
            .ConfigureAwait(false);

        var outcome = WirelessAdb.ParsePairResult(result.StdOut, result.StdErr, out var detail);

        if (outcome == AdbPairOutcome.Succeeded)
        {
            LastPairingEndpoint = normalized;
            _log.Info("Wireless pairing succeeded");
            return WirelessResult.Ok("配对成功。现在点「连接」建立音频通道。", normalized);
        }

        if (outcome == AdbPairOutcome.Failed)
        {
            // `detail` comes from adb and never contains the code we passed.
            _log.Warn($"Wireless pairing failed: {detail}");
            return WirelessResult.Fail(DescribePairFailure(detail), normalized);
        }

        _log.Warn($"Wireless pairing gave no recognisable result ({result.DescribeFailure()})");
        return WirelessResult.Fail(
            "配对没有成功。请确认手机上「使用配对码配对设备」的弹窗仍然开着，配对码没有过期，然后重试。",
            normalized);
    }

    /// <summary>
    /// Runs <c>adb connect &lt;endpoint&gt;</c>. A returned success only means the
    /// ADB link is up; whether the phone is <em>authorised</em> is decided by the
    /// next device scan (plan §5.10).
    /// </summary>
    public async Task<WirelessResult> ConnectAsync(string? endpoint, CancellationToken cancellationToken)
    {
        if (!WirelessAdb.TryNormalizeEndpoint(endpoint, out var normalized, out var problem))
        {
            return WirelessResult.Fail(problem ?? "请填写手机的 IP:端口。");
        }

        var result = await _adb
            .RunAsync(
                serial: null,
                new[] { "connect", normalized },
                ConnectTimeout,
                cancellationToken)
            .ConfigureAwait(false);

        var outcome = WirelessAdb.ParseConnectResult(result.StdOut, result.StdErr);

        if (outcome is AdbConnectOutcome.Connected or AdbConnectOutcome.AlreadyConnected)
        {
            _log.Info($"Wireless ADB link established ({outcome})");
            return WirelessResult.Ok("已连接到手机，正在准备音频通道。", normalized);
        }

        // Exit code 0 with "failed to connect" is normal here, so the outcome —
        // not result.Success — decides (plan §3.4.1).
        _log.Warn($"Wireless ADB connect failed: {result.DescribeFailure()}");
        return WirelessResult.Fail(DescribeConnectFailure(result), normalized);
    }

    /// <summary>
    /// Asks the ADB server which wireless-debugging devices it can see on the
    /// local network (plan §5.10: mDNS as a convenience, with manual entry kept
    /// as the always-available fallback).
    /// </summary>
    /// <remarks>
    /// <para>
    /// This is the answer to "配对端口和连接端口是哪个": the phone advertises both
    /// as separate mDNS services, so AnPod can read the connect port off the
    /// network instead of asking the user to copy it off the phone's screen.
    /// </para>
    /// <para>
    /// Two steps, in this order, because one command cannot answer both
    /// questions. <c>adb mdns services</c> prints a bare header with exit code 0
    /// in <em>every</em> case — quiet network, dead daemon, no phone in sight
    /// (measured: 0.05 s and rc 0 either way). So an empty list from it is
    /// meaningless on its own, and reporting one as "没有搜索到设备" would send the
    /// user to debug a phone that was never the problem. <c>mdns check</c> is the
    /// only command that distinguishes the two, and it does so by <em>how long it
    /// takes</em> — hence the probe first, the list second.
    /// </para>
    /// </remarks>
    public async Task<WirelessDiscovery> DiscoverAsync(CancellationToken cancellationToken)
    {
        // Step 1 — prove the search can work at all before believing its answer.
        var probe = await _adb
            .RunAsync(serial: null, new[] { "mdns", "check" }, MdnsProbeTimeout, cancellationToken)
            .ConfigureAwait(false);

        if (!WirelessAdb.IsMdnsHealthy(probe.StdOut, probe.StdErr))
        {
            return MdnsOutage(probe);
        }

        // Step 2 — the daemon is answering, so an empty list now means something.
        var result = await _adb
            .RunAsync(serial: null, new[] { "mdns", "services" }, DiscoverTimeout, cancellationToken)
            .ConfigureAwait(false);

        if (!result.Success)
        {
            _log.Warn($"adb mdns services failed: {result.DescribeFailure()}");
            return WirelessDiscovery.Unavailable(
                $"无法搜索局域网设备（{result.DescribeFailure()}）。请手动填写 IP:连接端口。");
        }

        var services = WirelessAdb.ParseMdnsServices(result.StdOut);
        var connect = services.Where(s => s.Kind == AdbMdnsKind.Connect).ToList();
        var pairing = services.Where(s => s.Kind == AdbMdnsKind.Pairing).ToList();

        _log.Info($"mDNS discovery: {connect.Count} connect, {pairing.Count} pairing service(s)");

        return new WirelessDiscovery(
            true,
            connect.Count switch
            {
                0 => "没有搜索到无线调试设备。请确认手机上「无线调试」已开启，等一两秒再点一次。",
                _ => $"搜索到 {connect.Count} 个可连接的无线设备。",
            },
            connect,
            pairing);
    }

    /// <summary>
    /// Builds the verdict for a health probe that did not come back healthy.
    /// </summary>
    /// <remarks>
    /// Both branches carry the same user action — type the address by hand — so
    /// they differ only in how bluntly the message says the search is broken and
    /// in what the log records. Naming a definitive "unavailable" is worth it
    /// because it tells the user that retrying is pointless; a timeout may just
    /// be a busy machine. Neither is fixable from inside AnPod: the only cure is
    /// restarting the ADB server, which another tool may own (plan §3.2), so the
    /// remedy offered is manual entry first and a reboot second.
    /// </remarks>
    private WirelessDiscovery MdnsOutage(AdbResult probe)
    {
        // The phone-side cause comes first because it is both more common and
        // something the user can fix in seconds: a phone with wireless debugging
        // switched off advertises nothing, so no search can ever find it — and
        // the daemon being down is not something AnPod may repair (plan §3.2).
        const string remedy =
            "请先确认手机上「无线调试」是开启的——没开启时手机不会广播，任何搜索都找不到它。" +
            "然后手动填写「无线调试」页面上的 IP:连接端口（注意不是配对端口）；" +
            "想恢复搜索功能，可在关掉其它占用 adb 的工具后重启电脑。";

        if (WirelessAdb.IsMdnsUnavailable(probe.StdOut, probe.StdErr))
        {
            _log.Warn("adb mdns check: the ADB server's mDNS daemon is unavailable");
            return WirelessDiscovery.Unavailable(
                "系统的 mDNS 搜索当前不可用（ADB 服务的发现组件没在运行）。" + remedy);
        }

        _log.Warn($"adb mdns check did not answer: {probe.DescribeFailure()}");
        return WirelessDiscovery.Unavailable("系统的 mDNS 搜索没有响应，无法搜索局域网设备。" + remedy);
    }

    /// <summary>
    /// Drops one wireless endpoint.
    /// </summary>
    /// <remarks>
    /// Always with an explicit endpoint. A bare <c>adb disconnect</c> removes
    /// every TCP device the ADB server knows about, including ones another tool
    /// is using — the wireless equivalent of <c>reverse --remove-all</c>
    /// (plan §3.5).
    /// </remarks>
    public async Task DisconnectAsync(string endpoint)
    {
        if (string.IsNullOrWhiteSpace(endpoint))
        {
            return;
        }

        var result = await _adb
            .RunAsync(
                serial: null,
                new[] { "disconnect", endpoint },
                DisconnectTimeout,
                CancellationToken.None)
            .ConfigureAwait(false);

        if (!result.Success)
        {
            _log.Debug($"Wireless disconnect reported: {result.DescribeFailure()}");
        }
    }

    private static string DescribePairFailure(string? detail)
    {
        if (detail is null)
        {
            return "配对失败，请确认配对码正确并且弹窗没有关闭。";
        }

        if (detail.Contains("password", StringComparison.OrdinalIgnoreCase) ||
            detail.Contains("code", StringComparison.OrdinalIgnoreCase))
        {
            return "配对码不正确或已过期。请在手机上重新打开「使用配对码配对设备」再填一次。";
        }

        if (detail.Contains("refused", StringComparison.OrdinalIgnoreCase) ||
            detail.Contains("unable to connect", StringComparison.OrdinalIgnoreCase) ||
            detail.Contains("cannot connect", StringComparison.OrdinalIgnoreCase) ||
            detail.Contains("timed out", StringComparison.OrdinalIgnoreCase))
        {
            return "连不上配对地址。请确认手机与电脑在同一网络、无线调试已开启，并且填的是「配对用」端口。";
        }

        return $"配对失败：{detail}";
    }

    private static string DescribeConnectFailure(AdbResult result)
    {
        if (result.TimedOut)
        {
            return "连接手机超时。请确认手机与电脑在同一网络，然后重试。";
        }

        var text = $"{result.StdOut}\n{result.StdErr}";

        if (text.Contains("missing port", StringComparison.OrdinalIgnoreCase))
        {
            return "地址缺少端口，请填写「连接用」的 IP:端口。";
        }

        if (text.Contains("refused", StringComparison.OrdinalIgnoreCase) ||
            text.Contains("unable to connect", StringComparison.OrdinalIgnoreCase) ||
            text.Contains("cannot connect", StringComparison.OrdinalIgnoreCase) ||
            text.Contains("failed to connect", StringComparison.OrdinalIgnoreCase) ||
            text.Contains("no route to host", StringComparison.OrdinalIgnoreCase))
        {
            // The causes users actually hit, in the order they hit them. Not
            // mentioning USB on purpose: this is not a cable problem, and saying
            // so would send them to check the wrong thing (plan §9.1 W10).
            return "无法连接到该地址。请确认：① 手机与电脑在同一网络；② 手机的「无线调试」仍然开着；" +
                   "③ 填的是「连接用」端口（不是配对端口）。如果还没配对过，请先用配对码配对。";
        }

        return $"连接手机失败（{result.DescribeFailure()}）。";
    }
}
