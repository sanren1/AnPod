using System.IO;

namespace AnPod.Desktop.Services;

/// <summary>
/// Single source of truth for on-disk locations (plan §5.8: settings and logs
/// live under <c>%LocalAppData%/AnPod</c>).
/// </summary>
public static class AppPaths
{
    public static string Root { get; } = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "AnPod");

    public static string SettingsFile => Path.Combine(Root, "settings.json");

    /// <summary>
    /// Records which render endpoints AnPod muted. It exists only so a run that
    /// never got to shut down cleanly (crash, Task Manager) can be undone on the
    /// next launch — without it the machine would stay silent with no on-screen
    /// explanation, which is indistinguishable from a Windows fault.
    /// </summary>
    public static string MuteStateFile => Path.Combine(Root, "muted-endpoints.txt");

    public static string LogDirectory => Path.Combine(Root, "logs");

    /// <summary>Directory holding AnPod.exe and its runtime dependencies.</summary>
    public static string ApplicationDirectory => AppContext.BaseDirectory;
}
