using System.Diagnostics;
using System.IO;
using System.Text;
using AnPod.Core.Adb;
using AnPod.Core.Logging;
using AnPod.Core.Models;
using AnPod.Desktop.Services;

namespace AnPod.Tools.SessionProbe;

/// <summary>
/// Answers the one question the two-control design rests on: does turning the
/// sound off drop the phone?
/// <para>
/// That is a claim about the reverse tunnel, the <c>app_process</c> payload and
/// the TCP socket all surviving a pause, so it can only be checked against a
/// real device. This drives the shipping <see cref="ConnectionManager"/> and
/// watches the device from outside with raw adb, so a bug in the manager cannot
/// also be a bug in the thing that measures it.
/// </para>
/// </summary>
internal static class Program
{
    private static int _failures;

    private static async Task<int> Main()
    {
        Console.OutputEncoding = Encoding.UTF8;

        var baseDir = AppContext.BaseDirectory;
        var adbPath = Path.Combine(baseDir, "adb.exe");
        var log = new ConsoleLog();

        var adb = new AdbRunner(adbPath, log);
        var capture = new NativeAudioCapture(log);
        var discovery = new DeviceDiscoveryService(adb, log);
        var muter = new PlaybackEndpointMuter(log);
        var payload = Path.Combine(baseDir, "server");

        if (!capture.IsAvailable)
        {
            Console.WriteLine($"FAIL  capture DLL unusable: {capture.LoadError}");
            return 1;
        }

        Console.WriteLine("== baseline ==");

        // Mirrors App.Compose(): undo a mute left by a run that was killed before
        // it could tear down. Without this the probe would inherit that mute, the
        // session would find nothing left to silence, and the probe would leave
        // the machine silent — the exact failure the marker exists to prevent.
        int recovered = muter.RecoverFromPreviousRun();
        if (recovered > 0)
        {
            Console.WriteLine($"recovered    : {recovered} endpoint(s) from a previous run");
        }

        await discovery.RefreshAsync(CancellationToken.None);
        var device = discovery.Devices.FirstOrDefault(d => d.IsConnectable);
        if (device is null)
        {
            Console.WriteLine("FAIL  no connectable USB device. Plug the phone in and authorise it.");
            return 1;
        }

        Console.WriteLine($"device       : {device.DisplayName}");
        Console.WriteLine($"reverse      : {CountOwnReverseMappings(adbPath)}");
        Console.WriteLine($"payload procs: {CountPayloadProcesses(adbPath)}");

        // A stale mapping from a previous run must not be mistaken for ours.
        Check("baseline has no AnPod reverse mapping", CountOwnReverseMappings(adbPath) == 0);

        // Nor must a leftover payload: it would inflate every count below and
        // make the pause/resume assertions meaningless. Fail loudly instead.
        Check("baseline has no leftover payload", CountPayloadProcesses(adbPath) == 0);

        await using var connection = new ConnectionManager(adb, capture, discovery, muter, payload, log)
        {
            AutoConnectLastDevice = false,
        };

        // ------------------------------------------------------------- connect
        Console.WriteLine();
        Console.WriteLine("== connect ==");
        await connection.ConnectAsync(device);
        Check("state is Streaming after connect", connection.State == ConnectionState.Streaming);
        Check("reverse mapping exists", CountOwnReverseMappings(adbPath) == 1);
        Check("payload process is running", CountPayloadProcesses(adbPath) >= 1);
        Check("local speakers silenced for the session", File.Exists(AppPaths.MuteStateFile));

        // ------------------------------------------------- pause (the point)
        Console.WriteLine();
        Console.WriteLine("== turn the sound off (the phone must stay connected) ==");
        await connection.SetAudioEnabledAsync(false);

        Check("state is Connected, not Ready/NoDevice", connection.State == ConnectionState.Connected);
        Check("session still reports itself as up", connection.IsSessionUp);
        Check("reverse mapping survived the pause", CountOwnReverseMappings(adbPath) == 1);
        Check("payload process survived the pause", CountPayloadProcesses(adbPath) >= 1);

        // Pausing is about the sound only: the machine must get its speakers
        // back the moment the user stops routing audio to the phone.
        Check("local speakers handed back while paused", !File.Exists(AppPaths.MuteStateFile));

        // ------------------------------------------------------- resume
        Console.WriteLine();
        Console.WriteLine("== turn it back on ==");
        await connection.SetAudioEnabledAsync(true);
        Check("state is Streaming again", connection.State == ConnectionState.Streaming);
        Check("still exactly one reverse mapping (no re-handshake)", CountOwnReverseMappings(adbPath) == 1);
        Check("still exactly one payload process (no restart)", CountPayloadProcesses(adbPath) == 1);
        Check("local speakers silenced again on resume", File.Exists(AppPaths.MuteStateFile));

        // ------------------------------------------------------- disconnect
        Console.WriteLine();
        Console.WriteLine("== disconnect ==");
        await connection.DisconnectAsync(userInitiated: true);
        Check("state is Ready after disconnect", connection.State == ConnectionState.Ready);
        Check("reverse mapping removed", CountOwnReverseMappings(adbPath) == 0);
        Check("payload process stopped", CountPayloadProcesses(adbPath) == 0);
        Check("local speakers handed back", !File.Exists(AppPaths.MuteStateFile));

        // --------------------------------------------------- idle = no polling
        Console.WriteLine();
        Console.WriteLine("== idle for 8 s: discovery must not poll ==");
        int refreshes = 0;
        void OnDevicesChanged(object? sender, IReadOnlyList<DeviceInfo> _) => Interlocked.Increment(ref refreshes);
        discovery.DevicesChanged += OnDevicesChanged;
        await Task.Delay(TimeSpan.FromSeconds(8));
        discovery.DevicesChanged -= OnDevicesChanged;

        Check($"no discovery refresh while idle (saw {refreshes})", refreshes == 0);

        Console.WriteLine();
        Console.WriteLine(_failures == 0 ? "PASS  all session checks passed" : $"FAIL  {_failures} check(s) failed");
        return _failures == 0 ? 0 : 1;
    }

    // ------------------------------------------------------------- observation

    /// <summary>
    /// Counts AnPod's own reverse mappings. Uses raw adb rather than the manager
    /// under test, so a broken cleanup cannot hide itself.
    /// </summary>
    private static int CountOwnReverseMappings(string adbPath)
    {
        var output = RunAdb(adbPath, "reverse --list");
        return output
            .Split('\n', StringSplitOptions.RemoveEmptyEntries)
            .Count(line => line.Contains("localabstract:anpod_", StringComparison.Ordinal));
    }

    /// <summary>
    /// Counts OUR payload, not every <c>app_process</c> on the device. Two
    /// details matter:
    /// <list type="bullet">
    ///   <item><description>Android's zygote and zygote64 are both launched as
    ///   <c>app_process</c>, so matching the executable name alone counts two
    ///   system processes and none of ours.</description></item>
    ///   <item><description><c>adb shell app_process …</c> leaves a
    ///   <c>sh -c app_process …</c> wrapper next to the real process, so one
    ///   session shows up as two lines. Anchoring the match at the start of the
    ///   command line drops the wrapper.</description></item>
    /// </list>
    /// </summary>
    private static int CountPayloadProcesses(string adbPath)
    {
        var output = RunAdb(adbPath, "shell ps -A -o ARGS");
        return output
            .Split('\n', StringSplitOptions.RemoveEmptyEntries)
            .Count(line => line.TrimStart().StartsWith("app_process ", StringComparison.Ordinal) &&
                           line.Contains("anpod_server", StringComparison.Ordinal));
    }

    private static string RunAdb(string adbPath, string arguments)
    {
        var startInfo = new ProcessStartInfo(adbPath, arguments)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            CreateNoWindow = true,
        };

        using var process = Process.Start(startInfo);
        if (process is null)
        {
            return string.Empty;
        }

        string stdout = process.StandardOutput.ReadToEnd();
        process.StandardError.ReadToEnd();
        process.WaitForExit(15000);
        return stdout;
    }

    private static void Check(string what, bool ok)
    {
        Console.WriteLine($"  {(ok ? "PASS" : "FAIL")}  {what}");
        if (!ok)
        {
            _failures++;
        }
    }

    private sealed class ConsoleLog : ILog
    {
        public void Debug(string message) => Console.WriteLine($"  [debug] {message}");

        public void Info(string message) => Console.WriteLine($"  [info ] {message}");

        public void Warn(string message) => Console.WriteLine($"  [warn ] {message}");

        public void Error(string message, Exception? exception = null) =>
            Console.WriteLine($"  [error] {message} {exception?.Message}");

        public void AddSecret(string? secret)
        {
            // The probe prints to a console; nothing here needs redaction.
        }
    }
}
