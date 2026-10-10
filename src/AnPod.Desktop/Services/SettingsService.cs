using System.IO;
using System.Text;
using AnPod.Core.Logging;
using AnPod.Core.Models;

namespace AnPod.Desktop.Services;

/// <summary>
/// Loads and saves <see cref="AppSettings"/> with an atomic replace so a crash
/// mid-write cannot leave a truncated file (plan §5.8).
/// </summary>
public sealed class SettingsService
{
    private readonly ILog _log;

    public SettingsService(ILog log)
    {
        _log = log;
    }

    public AppSettings Current { get; private set; } = new();

    /// <summary>
    /// Reads settings, recovering defaults on a missing or corrupt file rather
    /// than failing startup.
    /// </summary>
    public void Load()
    {
        try
        {
            if (!File.Exists(AppPaths.SettingsFile))
            {
                Current = new AppSettings();
                return;
            }

            var json = File.ReadAllText(AppPaths.SettingsFile, Encoding.UTF8);
            Current = AppSettingsSerializer.Deserialize(json);
        }
        catch (IOException ex)
        {
            _log.Warn($"Could not read settings, using defaults: {ex.Message}");
            Current = new AppSettings();
        }
        catch (UnauthorizedAccessException ex)
        {
            _log.Warn($"Could not read settings, using defaults: {ex.Message}");
            Current = new AppSettings();
        }
    }

    /// <summary>Atomically persists the supplied settings.</summary>
    public void Save(AppSettings settings)
    {
        var normalized = AppSettingsSerializer.Normalize(settings);
        Current = normalized;

        var temp = AppPaths.SettingsFile + ".tmp";
        try
        {
            System.IO.Directory.CreateDirectory(AppPaths.Root);
            File.WriteAllText(temp, AppSettingsSerializer.Serialize(normalized), Encoding.UTF8);

            // File.Move(overwrite) is an atomic replace on the same volume on Windows.
            File.Move(temp, AppPaths.SettingsFile, overwrite: true);
        }
        catch (IOException ex)
        {
            _log.Warn($"Could not persist settings: {ex.Message}");
            TryDelete(temp);
        }
        catch (UnauthorizedAccessException ex)
        {
            _log.Warn($"Could not persist settings: {ex.Message}");
            TryDelete(temp);
        }
    }

    private static void TryDelete(string path)
    {
        try
        {
            if (File.Exists(path))
            {
                File.Delete(path);
            }
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
    }
}
