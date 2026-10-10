using AnPod.Core.Adb;
using AnPod.Core.Logging;
using AnPod.Core.Models;

namespace AnPod.Desktop.Services;

/// <summary>
/// Reads the ADB device list on demand and publishes the classified result
/// (plan §5.2).
/// <para>
/// There is deliberately no background poll loop. An idle AnPod has nothing to
/// watch, and polling a machine with no phone attached just spins up an
/// <c>adb devices</c> process every couple of seconds for a list nobody reads.
/// The UI refreshes explicitly; session recovery refreshes as part of its retry
/// loop; the connection watchdog handles the one case that genuinely needs
/// periodic attention — an established session (see ConnectionManager).
/// </para>
/// <para>
/// Property queries are cached per serial because they never change while a
/// device stays connected, so a manual refresh is one <c>adb devices</c> plus at
/// most one <c>getprop</c> per newly seen device.
/// </para>
/// </summary>
public sealed class DeviceDiscoveryService
{
    private static readonly TimeSpan DevicesTimeout = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan PropertyTimeout = TimeSpan.FromSeconds(5);

    private readonly AdbRunner _adb;
    private readonly ILog _log;
    private readonly Dictionary<string, IReadOnlyDictionary<string, string>> _propertyCache =
        new(StringComparer.Ordinal);

    /// <summary>Serialises refreshes: the UI and session recovery can both ask.</summary>
    private readonly SemaphoreSlim _refreshGate = new(1, 1);

    private bool _reportedAdbFailure;

    public DeviceDiscoveryService(AdbRunner adb, ILog log)
    {
        _adb = adb;
        _log = log;
    }

    public IReadOnlyList<DeviceInfo> Devices { get; private set; } = Array.Empty<DeviceInfo>();

    /// <summary>Raised whenever the device list materially changes or refreshes.</summary>
    public event EventHandler<IReadOnlyList<DeviceInfo>>? DevicesChanged;

    /// <summary>Raised with a user-facing hint when discovery cannot run at all.</summary>
    public event EventHandler<string>? DiscoveryProblem;

    /// <summary>
    /// Runs one discovery cycle. This is the only way the list changes — call it
    /// from the refresh button, or from code that needs a current answer.
    /// </summary>
    public async Task RefreshAsync(CancellationToken cancellationToken)
    {
        await _refreshGate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            await RefreshCoreAsync(cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            _refreshGate.Release();
        }
    }

    private async Task RefreshCoreAsync(CancellationToken cancellationToken)
    {
        var result = await _adb
            .RunAsync(null, new[] { "devices", "-l" }, DevicesTimeout, cancellationToken)
            .ConfigureAwait(false);

        if (!result.Success)
        {
            // ADB itself is unavailable: report once, keep the previous list.
            if (!_reportedAdbFailure)
            {
                _reportedAdbFailure = true;
                _log.Warn($"adb devices failed: {result.DescribeFailure()}");
                DiscoveryProblem?.Invoke(this, "ADB 服务不可用，请检查其他工具是否占用了 ADB。");
            }

            return;
        }

        _reportedAdbFailure = false;

        var entries = AdbOutputParser.ParseDevices(result.StdOut);
        var devices = new List<DeviceInfo>(entries.Count);

        foreach (var entry in entries)
        {
            IReadOnlyDictionary<string, string>? properties = null;

            // Properties are only meaningful once the device is authorised.
            if (entry.State == AdbDeviceState.Device)
            {
                properties = await GetPropertiesAsync(entry.Serial, cancellationToken).ConfigureAwait(false);
            }

            devices.Add(DeviceInfoFactory.Create(entry, properties));
        }

        EvictCache(devices);
        Devices = devices;
        DevicesChanged?.Invoke(this, devices);
    }

    private async Task<IReadOnlyDictionary<string, string>> GetPropertiesAsync(
        string serial,
        CancellationToken cancellationToken)
    {
        if (_propertyCache.TryGetValue(serial, out var cached))
        {
            return cached;
        }

        var result = await _adb
            .RunAsync(
                serial,
                new[] { "shell", AdbOutputParser.PropertyQueryCommand },
                PropertyTimeout,
                cancellationToken)
            .ConfigureAwait(false);

        IReadOnlyDictionary<string, string> props = result.Success
            ? AdbOutputParser.ParseLabeledProperties(result.StdOut)
            : new Dictionary<string, string>();

        // Only cache a successful read; a transient failure should be retried.
        if (result.Success)
        {
            _propertyCache[serial] = props;
        }

        return props;
    }

    private void EvictCache(IReadOnlyList<DeviceInfo> current)
    {
        if (_propertyCache.Count == 0)
        {
            return;
        }

        var present = new HashSet<string>(current.Select(d => d.Serial), StringComparer.Ordinal);
        foreach (var serial in _propertyCache.Keys.Where(k => !present.Contains(k)).ToList())
        {
            _propertyCache.Remove(serial);
        }
    }
}
