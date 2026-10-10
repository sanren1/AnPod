using System.Runtime.InteropServices;

namespace AnPod.Tools.CaptureProbe;

/// <summary>What a render endpoint looks like right now.</summary>
internal readonly record struct EndpointState(string? Id, bool? Muted, float? Peak)
{
    /// <summary>Last path segment of the endpoint id — enough to tell two apart.</summary>
    public string ShortId
    {
        get
        {
            if (string.IsNullOrEmpty(Id))
            {
                return "?";
            }

            int brace = Id.LastIndexOf('{');
            return brace >= 0 ? Id[brace..] : Id;
        }
    }

    public string MuteText => Muted switch
    {
        true => "MUTED",
        false => "unmuted",
        null => "unreadable",
    };

    public string PeakText => Peak is null ? "?" : Peak.Value.ToString("F4");
}

/// <summary>
/// Reads the default render endpoint's identity, mute flag and peak meter
/// through its own copy of the Core Audio declarations.
/// <para>
/// The probe needs all three. The mute flag proves the feature under test
/// actually did something (otherwise "capture unchanged" is indistinguishable
/// from "nothing was muted"), and the peak meter proves the test tone is really
/// reaching the endpoint the capture taps — otherwise the probe would be
/// measuring a tone that went to a different device.
/// </para>
/// <para>
/// Deliberately independent of AnPod's own interop: a shared declaration could
/// not tell a wrong vtable from a working mute.
/// </para>
/// </summary>
internal static class EndpointInspector
{
    /// <summary>eConsole = 0, eMultimedia = 1.</summary>
    public const int ConsoleRole = 0;

    public const int MultimediaRole = 1;

    public static EndpointState Read(int role)
    {
        try
        {
            var clsid = new Guid("BCDE0395-E52F-467C-8E3D-C4579291692E");
            var iid = typeof(IMMDeviceEnumerator).GUID;

            if (CoCreateInstance(ref clsid, IntPtr.Zero, ClsCtxAll, ref iid, out object? raw) != 0 ||
                raw is not IMMDeviceEnumerator enumerator)
            {
                return new EndpointState(null, null, null);
            }

            if (enumerator.GetDefaultAudioEndpoint(EDataFlow.Render, role, out var device) != 0 ||
                device is null)
            {
                return new EndpointState(null, null, null);
            }

            device.GetId(out string? id);

            bool? muted = null;
            var volumeIid = typeof(IAudioEndpointVolume).GUID;
            if (device.Activate(ref volumeIid, ClsCtxAll, IntPtr.Zero, out object? volumeRaw) == 0 &&
                volumeRaw is IAudioEndpointVolume volume &&
                volume.GetMute(out bool value) == 0)
            {
                muted = value;
            }

            float? peak = null;
            var meterIid = typeof(IAudioMeterInformation).GUID;
            if (device.Activate(ref meterIid, ClsCtxAll, IntPtr.Zero, out object? meterRaw) == 0 &&
                meterRaw is IAudioMeterInformation meter &&
                meter.GetPeakValue(out float peakValue) == 0)
            {
                peak = peakValue;
            }

            return new EndpointState(id, muted, peak);
        }
        catch (Exception)
        {
            return new EndpointState(null, null, null);
        }
    }

    private const int ClsCtxAll = 0x17;

    /// <summary>DEVICE_STATE_ACTIVE.</summary>
    private const int DeviceStateActive = 0x1;

    /// <summary>
    /// Forcibly unmutes every active render endpoint. Recovery tool, not part of
    /// the product: it exists so a killed AnPod (which cannot run its own
    /// teardown) never leaves the machine silent with no way back.
    /// </summary>
    public static int ForceUnmuteAll()
    {
        int changed = 0;

        var clsid = new Guid("BCDE0395-E52F-467C-8E3D-C4579291692E");
        var iid = typeof(IMMDeviceEnumerator).GUID;

        if (CoCreateInstance(ref clsid, IntPtr.Zero, ClsCtxAll, ref iid, out object? raw) != 0 ||
            raw is not IMMDeviceEnumerator enumerator)
        {
            return 0;
        }

        if (enumerator.EnumAudioEndpoints(EDataFlow.Render, DeviceStateActive, out var collection) != 0 ||
            collection is null ||
            collection.GetCount(out int count) != 0)
        {
            return 0;
        }

        for (int index = 0; index < count; index++)
        {
            if (collection.Item(index, out var device) != 0 || device is null)
            {
                continue;
            }

            var volumeIid = typeof(IAudioEndpointVolume).GUID;
            if (device.Activate(ref volumeIid, ClsCtxAll, IntPtr.Zero, out object? volumeRaw) != 0 ||
                volumeRaw is not IAudioEndpointVolume volume)
            {
                continue;
            }

            var context = Guid.Empty;
            if (volume.SetMute(false, ref context) == 0)
            {
                changed++;
            }
        }

        return changed;
    }

    private enum EDataFlow
    {
        Render = 0,
    }

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoCreateInstance(
        ref Guid clsid,
        IntPtr outer,
        int clsCtx,
        ref Guid iid,
        [MarshalAs(UnmanagedType.IUnknown)] out object? instance);

    [ComImport]
    [Guid("A95664D2-9614-4F35-A746-DE8DB63617E6")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDeviceEnumerator
    {
        [PreserveSig]
        int EnumAudioEndpoints(EDataFlow dataFlow, int stateMask, out IMMDeviceCollection? devices);

        [PreserveSig]
        int GetDefaultAudioEndpoint(EDataFlow dataFlow, int role, out IMMDevice? endpoint);

        [PreserveSig]
        int GetDevice([MarshalAs(UnmanagedType.LPWStr)] string id, out IMMDevice? device);

        [PreserveSig]
        int RegisterEndpointNotificationCallback(IntPtr client);

        [PreserveSig]
        int UnregisterEndpointNotificationCallback(IntPtr client);
    }

    [ComImport]
    [Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDeviceCollection
    {
        [PreserveSig]
        int GetCount(out int count);

        [PreserveSig]
        int Item(int index, out IMMDevice? device);
    }

    [ComImport]
    [Guid("D666063F-1587-4E43-81F1-B948E807363F")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDevice
    {
        [PreserveSig]
        int Activate(
            ref Guid iid,
            int clsCtx,
            IntPtr activationParams,
            [MarshalAs(UnmanagedType.IUnknown)] out object? instance);

        [PreserveSig]
        int OpenPropertyStore(int access, out IntPtr properties);

        [PreserveSig]
        int GetId([MarshalAs(UnmanagedType.LPWStr)] out string? id);

        [PreserveSig]
        int GetState(out int state);
    }

    /// <summary>Slot order follows endpointvolume.h; GetMute is the 13th member.</summary>
    [ComImport]
    [Guid("5CDF2C82-841E-4546-9722-0CF74078229A")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IAudioEndpointVolume
    {
        [PreserveSig]
        int RegisterControlChangeNotify(IntPtr notify);

        [PreserveSig]
        int UnregisterControlChangeNotify(IntPtr notify);

        [PreserveSig]
        int GetChannelCount(out int count);

        [PreserveSig]
        int SetMasterVolumeLevel(float levelDb, ref Guid eventContext);

        [PreserveSig]
        int SetMasterVolumeLevelScalar(float level, ref Guid eventContext);

        [PreserveSig]
        int GetMasterVolumeLevel(out float levelDb);

        [PreserveSig]
        int GetMasterVolumeLevelScalar(out float level);

        [PreserveSig]
        int SetChannelVolumeLevel(uint channel, float levelDb, ref Guid eventContext);

        [PreserveSig]
        int SetChannelVolumeLevelScalar(uint channel, float level, ref Guid eventContext);

        [PreserveSig]
        int GetChannelVolumeLevel(uint channel, out float levelDb);

        [PreserveSig]
        int GetChannelVolumeLevelScalar(uint channel, out float level);

        [PreserveSig]
        int SetMute([MarshalAs(UnmanagedType.Bool)] bool mute, ref Guid eventContext);

        [PreserveSig]
        int GetMute([MarshalAs(UnmanagedType.Bool)] out bool mute);
    }

    [ComImport]
    [Guid("C02216F6-8C67-4B5B-9D00-D008E73E0064")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IAudioMeterInformation
    {
        [PreserveSig]
        int GetPeakValue(out float peak);

        [PreserveSig]
        int GetMeteringChannelCount(out uint count);

        [PreserveSig]
        int GetChannelsPeakValues(uint count, [Out] float[] peaks);

        [PreserveSig]
        int QueryHardwareSupport(out uint mask);
    }
}
