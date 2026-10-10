using System.IO;
using System.Windows;
using System.Windows.Media.Imaging;

// UseWindowsForms + UseWPF makes Icon/Application ambiguous; pin what we need.
using Application = System.Windows.Application;
using Forms = System.Windows.Forms;

namespace AnPod.Desktop.Services;

/// <summary>
/// The application icon, loaded from the .ico embedded in this assembly
/// (<c>Assets/icon.ico</c>, built from the repository's <c>icon.png</c> by
/// <c>packaging/make-icon.py</c>).
/// <para>
/// One .ico carries every size Windows asks for — 16 px in the notification
/// area, 32 px in the title bar, 256 px in Explorer — so the tray and the window
/// share a single asset instead of each one scaling the same bitmap down.
/// </para>
/// </summary>
public static class AppIcon
{
    /// <summary>Resource path inside this assembly (see AnPod.Desktop.csproj).</summary>
    private const string ResourcePath = "Assets/icon.ico";

    private static readonly Lazy<BitmapSource> WindowIconLazy = new(LoadWindowIcon);
    private static readonly Lazy<Icon> TrayIconLazy = new(LoadTrayIcon);

    /// <summary>Window and taskbar icon. Frozen, so it is safe across threads.</summary>
    public static BitmapSource WindowIcon => WindowIconLazy.Value;

    /// <summary>
    /// A fresh tray icon sized for the shell's current small-icon metric. The
    /// caller owns the result, which is why this hands out a clone rather than
    /// the cached instance.
    /// </summary>
    public static Icon CreateTrayIcon() => (Icon)TrayIconLazy.Value.Clone();

    /// <summary>
    /// Opens the embedded .ico.
    /// </summary>
    /// <remarks>
    /// The URI is assembly-qualified rather than relative on purpose: a relative
    /// one resolves against the <em>entry</em> assembly, which is not this one
    /// when a tool (UiSnapshot, SessionProbe) loads the app's types without being
    /// the app itself.
    /// </remarks>
    private static Stream OpenResource()
    {
        var assembly = typeof(AppIcon).Assembly.GetName().Name;
        var uri = new Uri(
            $"pack://application:,,,/{assembly};component/{ResourcePath}",
            UriKind.Absolute);

        var stream = Application.GetResourceStream(uri);
        if (stream?.Stream is not { } resource)
        {
            throw new InvalidOperationException($"Icon resource is missing: {ResourcePath}");
        }

        return resource;
    }

    /// <summary>
    /// Picks the largest frame in the .ico for the window/taskbar icon.
    /// </summary>
    /// <remarks>
    /// The largest, deliberately not the first. Windows chooses a size per
    /// context — 32 px for the taskbar button, 48/64 px for Alt-Tab, 256 px for
    /// the large-icon views — and scales down from whatever HICON it is handed.
    /// <see cref="BitmapFrame.Create(System.IO.Stream, BitmapCreateOptions, BitmapCacheOption)"/>
    /// returns the icon's <em>first</em> frame, which in our file is 16 px, so the
    /// taskbar was upscaling a 16-pixel image: soft edges, and a glyph that read
    /// as too small next to other apps.
    /// <para>
    /// <see cref="IconBitmapDecoder"/> exposes every frame, so this is a choice
    /// rather than a guess — and the selection is by pixel area so a future
    /// non-square frame cannot win by being wide.
    /// </para>
    /// </remarks>
    private static BitmapSource LoadWindowIcon()
    {
        using var stream = OpenResource();

        // OnLoad so the frames do not keep the stream alive after we dispose it.
        var decoder = BitmapDecoder.Create(stream, BitmapCreateOptions.None, BitmapCacheOption.OnLoad);
        if (decoder.Frames.Count == 0)
        {
            throw new InvalidOperationException($"Icon resource has no frames: {ResourcePath}");
        }

        var frame = decoder.Frames
            .OrderByDescending(f => (long)f.PixelWidth * f.PixelHeight)
            .First();

        frame.Freeze();
        return frame;
    }

    private static Icon LoadTrayIcon()
    {
        // Match the shell's own metric so the glyph stays crisp at 100 %, 150 %
        // and 200 % display scaling instead of being resampled from a fixed 32 px.
        int edge = Forms.SystemInformation.SmallIconSize.Width;
        if (edge <= 0)
        {
            edge = 16;
        }

        using var stream = OpenResource();
        return new Icon(stream, edge, edge);
    }
}
