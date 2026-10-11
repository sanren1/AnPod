using System.Globalization;
using System.IO;
using System.Text;
using AnPod.Core.Logging;

namespace AnPod.Desktop.Services;

/// <summary>
/// Rolling daily text log with a 7-day retention window (plan §5.8).
/// Append-only, thread-safe, and redaction-aware: anything registered through
/// <see cref="AddSecret"/> is masked before it reaches disk.
/// </summary>
public sealed class FileLogger : ILog, IDisposable
{
    private const int RetentionDays = 7;

    private readonly object _gate = new();
    private readonly List<string> _secrets = new();
    private readonly LogLevel _minimumLevel;
    private bool _disposed;

    public FileLogger(string directory, LogLevel minimumLevel = LogLevel.Info)
    {
        Directory = directory;
        _minimumLevel = minimumLevel;

        System.IO.Directory.CreateDirectory(Directory);
        PruneOldFiles();
    }

    /// <summary>Folder holding the rolling log files; also the "open logs" target.</summary>
    public string Directory { get; }

    public void AddSecret(string? secret)
    {
        if (string.IsNullOrWhiteSpace(secret))
        {
            return;
        }

        lock (_gate)
        {
            if (!_secrets.Contains(secret, StringComparer.Ordinal))
            {
                _secrets.Add(secret);
            }
        }
    }

    public void RemoveSecret(string? secret)
    {
        if (string.IsNullOrWhiteSpace(secret)) return;
        lock (_gate)
        {
            _secrets.RemoveAll(value => string.Equals(value, secret, StringComparison.Ordinal));
        }
    }

    public void Debug(string message) => Write(LogLevel.Debug, message, null);

    public void Info(string message) => Write(LogLevel.Info, message, null);

    public void Warn(string message) => Write(LogLevel.Warn, message, null);

    public void Error(string message, Exception? exception = null) => Write(LogLevel.Error, message, exception);

    private void Write(LogLevel level, string message, Exception? exception)
    {
        if (level < _minimumLevel)
        {
            return;
        }

        string line;
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            var timestamp = DateTimeOffset.Now.ToString("yyyy-MM-dd HH:mm:ss.fff", CultureInfo.InvariantCulture);
            var text = exception is null
                ? message
                : $"{message} | {exception.GetType().Name}: {exception.Message}";

            line = $"{timestamp} [{level.ToString().ToUpperInvariant()}] {LogRedactor.Redact(text, _secrets)}{Environment.NewLine}";

            try
            {
                File.AppendAllText(CurrentFile(), line, Encoding.UTF8);
            }
            catch (IOException)
            {
                // Logging must never take the app down.
            }
            catch (UnauthorizedAccessException)
            {
            }
        }
    }

    private string CurrentFile() =>
        Path.Combine(Directory, $"anpod-{DateTime.Now:yyyy-MM-dd}.log");

    private void PruneOldFiles()
    {
        try
        {
            var cutoff = DateTime.Now.Date.AddDays(-RetentionDays);
            foreach (var file in System.IO.Directory.EnumerateFiles(Directory, "anpod-*.log"))
            {
                if (File.GetLastWriteTime(file) < cutoff)
                {
                    File.Delete(file);
                }
            }
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
    }

    public void Dispose()
    {
        lock (_gate)
        {
            _disposed = true;
        }
    }
}
