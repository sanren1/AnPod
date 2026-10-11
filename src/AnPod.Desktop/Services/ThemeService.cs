using System.Windows;
using System.Windows.Media;
using AnPod.Core.Models;
using Microsoft.Win32;
using Color = System.Windows.Media.Color;

namespace AnPod.Desktop.Services;

public static class ThemeService
{
    public static void Apply(AppearanceTheme preference)
    {
        var app = System.Windows.Application.Current;
        if (app is null) return;

        var highContrast = SystemParameters.HighContrast;
        var dark = !highContrast && preference switch
        {
            AppearanceTheme.Dark => true,
            AppearanceTheme.Light => false,
            _ => !SystemUsesLightTheme(),
        };

        var colors = highContrast ? HighContrastColors() : dark ? DarkColors() : LightColors();
        foreach (var (key, color) in colors)
        {
            var brushKey = key.Replace("Color", "Brush", StringComparison.Ordinal);
            app.Resources[brushKey] = new SolidColorBrush(color);
        }
    }

    private static bool SystemUsesLightTheme()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            return Convert.ToInt32(key?.GetValue("AppsUseLightTheme") ?? 1) != 0;
        }
        catch (Exception)
        {
            return true;
        }
    }

    private static Dictionary<string, Color> LightColors() => new()
    {
        ["AppBackgroundColor"] = Color.FromRgb(248, 250, 252), ["CardColor"] = Colors.White,
        ["ForegroundColor"] = Color.FromRgb(15, 23, 42), ["MutedColor"] = Color.FromRgb(241, 243, 245),
        ["MutedForegroundColor"] = Color.FromRgb(71, 85, 105), ["BorderColor"] = Color.FromRgb(228, 231, 235),
        ["PrimaryColor"] = Color.FromRgb(30, 58, 95), ["PrimaryHoverColor"] = Color.FromRgb(22, 48, 79),
        ["PrimaryPressedColor"] = Color.FromRgb(16, 37, 66), ["SecondaryColor"] = Color.FromRgb(37, 99, 235),
        ["AccentColor"] = Color.FromRgb(5, 150, 105), ["DestructiveColor"] = Color.FromRgb(185, 28, 28),
        ["OnPrimaryColor"] = Colors.White, ["StatusOkColor"] = Color.FromRgb(5, 122, 85),
        ["StatusBusyColor"] = Color.FromRgb(29, 78, 216), ["StatusErrorColor"] = Color.FromRgb(185, 28, 28),
        ["StatusWarnColor"] = Color.FromRgb(146, 64, 14), ["StatusIdleColor"] = Color.FromRgb(71, 85, 105),
    };

    private static Dictionary<string, Color> DarkColors() => new()
    {
        ["AppBackgroundColor"] = Color.FromRgb(24, 24, 27), ["CardColor"] = Color.FromRgb(36, 36, 40),
        ["ForegroundColor"] = Color.FromRgb(244, 244, 245), ["MutedColor"] = Color.FromRgb(48, 48, 52),
        ["MutedForegroundColor"] = Color.FromRgb(203, 213, 225), ["BorderColor"] = Color.FromRgb(82, 82, 91),
        ["PrimaryColor"] = Color.FromRgb(147, 197, 253), ["PrimaryHoverColor"] = Color.FromRgb(191, 219, 254),
        ["PrimaryPressedColor"] = Color.FromRgb(96, 165, 250), ["SecondaryColor"] = Color.FromRgb(147, 197, 253),
        ["AccentColor"] = Color.FromRgb(52, 211, 153), ["DestructiveColor"] = Color.FromRgb(252, 165, 165),
        ["OnPrimaryColor"] = Color.FromRgb(15, 23, 42), ["StatusOkColor"] = Color.FromRgb(52, 211, 153),
        ["StatusBusyColor"] = Color.FromRgb(147, 197, 253), ["StatusErrorColor"] = Color.FromRgb(252, 165, 165),
        ["StatusWarnColor"] = Color.FromRgb(253, 230, 138), ["StatusIdleColor"] = Color.FromRgb(203, 213, 225),
    };

    private static Dictionary<string, Color> HighContrastColors() => new()
    {
        ["AppBackgroundColor"] = System.Windows.SystemColors.WindowColor, ["CardColor"] = System.Windows.SystemColors.WindowColor,
        ["ForegroundColor"] = System.Windows.SystemColors.WindowTextColor, ["MutedColor"] = System.Windows.SystemColors.ControlColor,
        ["MutedForegroundColor"] = System.Windows.SystemColors.ControlTextColor, ["BorderColor"] = System.Windows.SystemColors.WindowTextColor,
        ["PrimaryColor"] = System.Windows.SystemColors.HighlightColor, ["PrimaryHoverColor"] = System.Windows.SystemColors.HighlightColor,
        ["PrimaryPressedColor"] = System.Windows.SystemColors.HighlightColor, ["SecondaryColor"] = System.Windows.SystemColors.HighlightColor,
        ["AccentColor"] = System.Windows.SystemColors.HighlightColor, ["DestructiveColor"] = System.Windows.SystemColors.WindowTextColor,
        ["OnPrimaryColor"] = System.Windows.SystemColors.HighlightTextColor, ["StatusOkColor"] = System.Windows.SystemColors.WindowTextColor,
        ["StatusBusyColor"] = System.Windows.SystemColors.HighlightColor, ["StatusErrorColor"] = System.Windows.SystemColors.WindowTextColor,
        ["StatusWarnColor"] = System.Windows.SystemColors.WindowTextColor, ["StatusIdleColor"] = System.Windows.SystemColors.GrayTextColor,
    };
}
