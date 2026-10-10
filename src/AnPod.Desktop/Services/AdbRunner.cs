using System.Diagnostics;
using System.IO;
using System.Text;
using AnPod.Core.Logging;

namespace AnPod.Desktop.Services;

/// <summary>Outcome of one short-lived ADB invocation.</summary>
public sealed record AdbResult(
    int ExitCode,
    string StdOut,
    string StdErr,
    TimeSpan Duration,
    bool TimedOut,
    bool Canceled)
{
    /// <summary>True only when the command completed normally with exit code 0.</summary>
    public bool Success => !TimedOut && !Canceled && ExitCode == 0;

    /// <summary>One-line reason suitable for a log entry or the status bar.</summary>
    public string DescribeFailure()
    {
        if (Canceled)
        {
            return "cancelled";
        }

        if (TimedOut)
        {
            return "timed out";
        }

        var detail = string.IsNullOrWhiteSpace(StdErr) ? StdOut : StdErr;
        detail = detail.Trim();
        if (detail.Length > 200)
        {
            detail = detail[..200];
        }

        return string.IsNullOrEmpty(detail) ? $"exit code {ExitCode}" : $"exit code {ExitCode}: {detail}";
    }
}

/// <summary>
/// Wraps the bundled <c>adb.exe</c>. Fixes the upstream Dart layer's gaps
/// (plan §3.4.1): non-zero exit codes, timeouts and cancellation are first-class
/// results rather than swallowed strings, and the process tree is killed on
/// cancel without touching the shared ADB server.
/// </summary>
public sealed class AdbRunner
{
    private readonly ILog _log;

    public AdbRunner(string adbPath, ILog log)
    {
        AdbPath = adbPath;
        _log = log;
    }

    public string AdbPath { get; }

    public bool AdbPresent => File.Exists(AdbPath);

    /// <summary>
    /// Runs ADB with the given arguments. When <paramref name="serial"/> is
    /// non-null, <c>-s SERIAL</c> is prepended so a command can never hit the
    /// wrong device (plan §5.1).
    /// </summary>
    public async Task<AdbResult> RunAsync(
        string? serial,
        IReadOnlyList<string> arguments,
        TimeSpan timeout,
        CancellationToken cancellationToken)
    {
        var argv = new List<string>(arguments.Count + 2);
        if (!string.IsNullOrEmpty(serial))
        {
            argv.Add("-s");
            argv.Add(serial);
        }

        argv.AddRange(arguments);
        return await RunRawAsync(argv, timeout, cancellationToken).ConfigureAwait(false);
    }

    private async Task<AdbResult> RunRawAsync(
        IReadOnlyList<string> argv,
        TimeSpan timeout,
        CancellationToken cancellationToken)
    {
        var started = Stopwatch.StartNew();

        // Short-circuit a missing adb.exe. Without this the discovery poll would
        // raise (and log) a process-start failure every few seconds, which is
        // both noisy and needless I/O (plan §1.3). The caller reports the
        // condition once, in user-facing terms.
        if (!AdbPresent)
        {
            return new AdbResult(
                -1,
                string.Empty,
                $"adb not found at {AdbPath}",
                started.Elapsed,
                false,
                false);
        }

        var psi = new ProcessStartInfo
        {
            FileName = AdbPath,
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };

        // ArgumentList never goes through a shell, so no argument needs quoting
        // and no untrusted string can be interpreted as a command (plan §1.3).
        foreach (var arg in argv)
        {
            psi.ArgumentList.Add(arg);
        }

        using var process = new Process { StartInfo = psi };

        try
        {
            process.Start();
        }
        catch (Exception ex)
        {
            _log.Error($"Failed to start adb ({string.Join(' ', argv)})", ex);
            return new AdbResult(-1, string.Empty, ex.Message, started.Elapsed, false, false);
        }

        // Both pipes must be drained concurrently or a chatty device deadlocks us.
        var stdoutTask = process.StandardOutput.ReadToEndAsync();
        var stderrTask = process.StandardError.ReadToEndAsync();

        using var timeoutCts = new CancellationTokenSource(timeout);
        using var linkedCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, timeoutCts.Token);

        bool timedOut = false;
        bool canceled = false;

        try
        {
            await process.WaitForExitAsync(linkedCts.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            canceled = cancellationToken.IsCancellationRequested;
            timedOut = !canceled && timeoutCts.IsCancellationRequested;
            KillProcessTree(process, argv);
        }

        var stdout = await SafeReadAsync(stdoutTask).ConfigureAwait(false);
        var stderr = await SafeReadAsync(stderrTask).ConfigureAwait(false);

        int exitCode = -1;
        try
        {
            if (process.HasExited)
            {
                exitCode = process.ExitCode;
            }
        }
        catch (InvalidOperationException)
        {
        }

        var result = new AdbResult(exitCode, stdout, stderr, started.Elapsed, timedOut, canceled);

        if (!result.Success)
        {
            _log.Debug($"adb {string.Join(' ', argv)} -> {result.DescribeFailure()} ({result.Duration.TotalMilliseconds:F0} ms)");
        }

        return result;
    }

    /// <summary>Kills only this command's process tree; the ADB daemon is left alone.</summary>
    private void KillProcessTree(Process process, IReadOnlyList<string> argv)
    {
        try
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
            }
        }
        catch (Exception ex)
        {
            _log.Debug($"Could not kill adb process tree ({string.Join(' ', argv)}): {ex.Message}");
        }
    }

    private static async Task<string> SafeReadAsync(Task<string> readTask)
    {
        try
        {
            return await readTask.ConfigureAwait(false);
        }
        catch (Exception)
        {
            return string.Empty;
        }
    }
}
