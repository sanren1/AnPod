using System.Diagnostics;
using System.IO;
using System.Text;
using AnPod.Core.Logging;

namespace AnPod.Desktop.Services;

/// <summary>
/// Owns the long-lived <c>adb shell app_process</c> invocation that hosts the
/// Android audio service (plan §5.1). Unlike a short command it must not carry
/// a request timeout; its stdout/stderr are consumed continuously so the pipe
/// buffer cannot fill and stall the remote process.
/// </summary>
public sealed class AndroidProcessSession : IAsyncDisposable
{
    private readonly Process _process;
    private readonly ILog _log;
    private readonly StringBuilder _stderrTail = new();
    private readonly object _gate = new();
    private bool _stopping;

    private AndroidProcessSession(Process process, ILog log)
    {
        _process = process;
        _log = log;
    }

    /// <summary>Raised for each line the Android service writes to stderr.</summary>
    public event EventHandler<string>? OutputLine;

    /// <summary>Raised once when the shell exits (including crashes).</summary>
    public event EventHandler? Exited;

    public bool HasExited
    {
        get
        {
            try
            {
                return _process.HasExited;
            }
            catch (InvalidOperationException)
            {
                return true;
            }
        }
    }

    /// <summary>Last few stderr lines, used to explain a failed launch (plan §6).</summary>
    public string StderrTail
    {
        get
        {
            lock (_gate)
            {
                return _stderrTail.ToString();
            }
        }
    }

    public static AndroidProcessSession Start(
        string adbPath,
        string serial,
        IReadOnlyList<string> appProcessArguments,
        ILog log)
    {
        var psi = new ProcessStartInfo
        {
            FileName = adbPath,
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };

        psi.ArgumentList.Add("-s");
        psi.ArgumentList.Add(serial);
        psi.ArgumentList.Add("shell");
        psi.ArgumentList.Add("app_process");
        foreach (var arg in appProcessArguments)
        {
            psi.ArgumentList.Add(arg);
        }

        var process = new Process { StartInfo = psi, EnableRaisingEvents = true };
        var session = new AndroidProcessSession(process, log);

        // Subscribed before Start(): a payload that dies instantly (corrupt
        // container, unsupported ROM) would otherwise be able to exit in the
        // window between Start() and the handler being attached, and the
        // Exited event would be lost — leaving the handshake wait to run to
        // its full timeout.
        process.Exited += session.OnProcessExited;

        process.Start();
        session.BeginPump();
        return session;
    }

    private void OnProcessExited(object? sender, EventArgs e)
    {
        if (!_stopping)
        {
            _log.Warn("Android audio process exited unexpectedly");
        }

        Exited?.Invoke(this, EventArgs.Empty);
    }

    private void BeginPump()
    {
        // stdout carries little, but must be drained anyway.
        _ = Task.Run(async () =>
        {
            try
            {
                while (await _process.StandardOutput.ReadLineAsync().ConfigureAwait(false) is { } line)
                {
                    if (!string.IsNullOrWhiteSpace(line))
                    {
                        _log.Debug($"[android] {line}");
                    }
                }
            }
            catch (Exception)
            {
                // Stream closed during shutdown; nothing useful to report.
            }
        });

        _ = Task.Run(async () =>
        {
            try
            {
                while (await _process.StandardError.ReadLineAsync().ConfigureAwait(false) is { } line)
                {
                    if (string.IsNullOrWhiteSpace(line))
                    {
                        continue;
                    }

                    lock (_gate)
                    {
                        _stderrTail.AppendLine(line);
                        // Keep the tail bounded; only the last lines matter.
                        if (_stderrTail.Length > 4096)
                        {
                            _stderrTail.Remove(0, _stderrTail.Length - 4096);
                        }
                    }

                    OutputLine?.Invoke(this, line);
                }
            }
            catch (Exception)
            {
            }
        });
    }

    /// <summary>
    /// Stops the remote service. Killing the local shell tears down the ADB
    /// transport, which closes the Android LocalSocket and lets the Java side
    /// release its AudioTrack (plan §3.4.9).
    /// </summary>
    public async Task StopAsync(TimeSpan grace)
    {
        _stopping = true;

        try
        {
            if (!_process.HasExited)
            {
                _process.Kill(entireProcessTree: true);
                using var cts = new CancellationTokenSource(grace);
                try
                {
                    await _process.WaitForExitAsync(cts.Token).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    _log.Warn("Android process did not exit within the grace period");
                }
            }
        }
        catch (Exception ex)
        {
            _log.Debug($"Stopping Android process raised: {ex.Message}");
        }
    }

    public async ValueTask DisposeAsync()
    {
        await StopAsync(TimeSpan.FromSeconds(2)).ConfigureAwait(false);
        _process.Dispose();
    }
}
