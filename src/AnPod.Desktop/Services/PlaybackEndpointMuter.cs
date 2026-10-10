using System.IO;
using System.Runtime.InteropServices;
using AnPod.Core.Logging;

namespace AnPod.Desktop.Services;

/// <summary>
/// Silences the machine's own speakers for the duration of a streaming session
/// so the audio is heard from the phone alone, then puts back exactly the state
/// it found.
/// <para>
/// Ownership rule: an endpoint is only ever unmuted if AnPod muted it <em>and</em>
/// it was unmuted beforehand. A device the user had already muted is left alone
/// and never touched again — AnPod must not be the reason sound comes back on.
/// </para>
/// <para>
/// Every failure degrades to "not muted"; nothing here may abort a session.
/// </para>
/// </summary>
public sealed class PlaybackEndpointMuter
{
    /// <summary>DEVICE_STATE_ACTIVE — present, enabled and plugged in.</summary>
    private const int DeviceStateActive = 0x1;

    /// <summary>CLSCTX_ALL — the only context Core Audio activation needs.</summary>
    private const int ClsCtxAll = 0x17;

    private const int CoInitMultithreaded = 0x0;

    /// <summary>RPC_E_CHANGED_MODE — thread already in a different apartment.</summary>
    private const int RpcChangedMode = unchecked((int)0x80010106);

    private readonly ILog _log;
    private readonly object _gate = new();

    /// <summary>
    /// Endpoint ids AnPod muted. Ids rather than interface pointers on purpose:
    /// the muter is called from different thread-pool threads, and a raw
    /// <c>IAudioEndpointVolume</c> is bound to the apartment that created it.
    /// Re-enumerating on restore keeps every call self-contained.
    /// </summary>
    private readonly List<string> _mutedIds = new();

    public PlaybackEndpointMuter(ILog log)
    {
        _log = log;
    }

    /// <summary>
    /// Mutes every active render endpoint that is not already muted.
    /// Returns how many AnPod silenced; 0 means "nothing changed".
    /// </summary>
    public int MuteAll()
    {
        lock (_gate)
        {
            // Never stack two rounds: a stale ownership list would leak mutes.
            RestoreLocked();

            var apartment = InitializeCom();
            if (apartment == ComApartment.Unavailable)
            {
                _log.Warn("COM could not be initialised; local speakers were left unmuted.");
                return 0;
            }

            try
            {
                foreach (var (id, volume) in EnumerateActiveRenderVolumes())
                {
                    try
                    {
                        if (volume.GetMute(out bool alreadyMuted) != 0)
                        {
                            continue;
                        }

                        if (alreadyMuted)
                        {
                            // Silent because the user chose so. Not ours to undo.
                            continue;
                        }

                        var context = Guid.Empty;
                        if (volume.SetMute(true, ref context) != 0)
                        {
                            continue;
                        }

                        _mutedIds.Add(id);
                    }
                    catch (Exception ex)
                    {
                        _log.Debug($"Could not mute an audio endpoint: {ex.Message}");
                    }
                }
            }
            finally
            {
                if (apartment == ComApartment.Owned)
                {
                    CoUninitialize();
                }
            }

            if (_mutedIds.Count == 0)
            {
                _log.Debug("No unmuted render endpoint needed silencing.");
            }
            else
            {
                WriteMarker();
                _log.Info($"Muted {_mutedIds.Count} local playback device(s) for this session.");
            }

            return _mutedIds.Count;
        }
    }

    /// <summary>
    /// Undoes a mute left behind by a run that never shut down cleanly — a crash,
    /// or being killed from Task Manager, neither of which gets to run the normal
    /// teardown. Call once at startup, before anything else touches the endpoints.
    /// </summary>
    public int RecoverFromPreviousRun()
    {
        lock (_gate)
        {
            string[] ids;
            try
            {
                if (!File.Exists(AppPaths.MuteStateFile))
                {
                    return 0;
                }

                ids = File.ReadAllLines(AppPaths.MuteStateFile)
                    .Where(line => !string.IsNullOrWhiteSpace(line))
                    .ToArray();
            }
            catch (Exception ex)
            {
                _log.Debug($"Could not read the previous mute state: {ex.Message}");
                return 0;
            }

            if (ids.Length == 0)
            {
                DeleteMarker();
                return 0;
            }

            _mutedIds.Clear();
            _mutedIds.AddRange(ids);
            int count = _mutedIds.Count;
            RestoreLocked();

            _log.Warn(
                $"A previous AnPod run did not shut down cleanly; restored {count} local playback device(s).");
            return count;
        }
    }

    /// <summary>
    /// Unmutes the endpoints AnPod muted. Safe to call when nothing was muted.
    /// Returns how many were restored.
    /// </summary>
    public int Restore()
    {
        lock (_gate)
        {
            int restored = _mutedIds.Count;
            RestoreLocked();
            return restored;
        }
    }

    private void RestoreLocked()
    {
        if (_mutedIds.Count == 0)
        {
            return;
        }

        var apartment = InitializeCom();
        if (apartment == ComApartment.Unavailable)
        {
            // The mute would otherwise outlive the app and look like a Windows bug.
            _log.Warn("COM could not be initialised; local speakers may still be muted.");
        }
        else
        {
            try
            {
                var wanted = new HashSet<string>(_mutedIds, StringComparer.Ordinal);
                foreach (var (id, volume) in EnumerateActiveRenderVolumes())
                {
                    if (!wanted.Contains(id))
                    {
                        continue;
                    }

                    try
                    {
                        var context = Guid.Empty;
                        volume.SetMute(false, ref context);
                    }
                    catch (Exception ex)
                    {
                        _log.Debug($"Could not restore an audio endpoint: {ex.Message}");
                    }
                }
            }
            finally
            {
                if (apartment == ComApartment.Owned)
                {
                    CoUninitialize();
                }
            }
        }

        _mutedIds.Clear();
        DeleteMarker();
    }

    /// <summary>
    /// Persists the ownership list. Best-effort: failing to write it must never
    /// stop the mute, it only costs us the crash-recovery safety net.
    /// </summary>
    private void WriteMarker()
    {
        try
        {
            Directory.CreateDirectory(AppPaths.Root);
            File.WriteAllLines(AppPaths.MuteStateFile, _mutedIds);
        }
        catch (Exception ex)
        {
            _log.Debug($"Could not record the mute state: {ex.Message}");
        }
    }

    private void DeleteMarker()
    {
        try
        {
            if (File.Exists(AppPaths.MuteStateFile))
            {
                File.Delete(AppPaths.MuteStateFile);
            }
        }
        catch (Exception ex)
        {
            // A stale marker only costs a spurious restore on the next launch.
            _log.Debug($"Could not clear the mute state: {ex.Message}");
        }
    }

    private static List<(string Id, IAudioEndpointVolume Volume)> EnumerateActiveRenderVolumes()
    {
        var found = new List<(string, IAudioEndpointVolume)>();

        var enumerator = CreateDeviceEnumerator();
        if (enumerator is null)
        {
            return found;
        }

        if (enumerator.EnumAudioEndpoints(EDataFlow.Render, DeviceStateActive, out var collection) != 0 ||
            collection is null)
        {
            return found;
        }

        if (collection.GetCount(out int count) != 0)
        {
            return found;
        }

        for (int index = 0; index < count; index++)
        {
            if (collection.Item(index, out var device) != 0 || device is null)
            {
                continue;
            }

            if (device.GetId(out string? id) != 0 || string.IsNullOrEmpty(id))
            {
                continue;
            }

            var iid = typeof(IAudioEndpointVolume).GUID;
            if (device.Activate(ref iid, ClsCtxAll, IntPtr.Zero, out object? instance) != 0)
            {
                continue;
            }

            if (instance is IAudioEndpointVolume volume)
            {
                found.Add((id, volume));
            }
        }

        return found;
    }

    /// <summary>
    /// CoCreateInstance rather than <c>new MMDeviceEnumerator()</c>: the C#
    /// coclass-cast shorthand does not compile for this interop shape, and this
    /// form says exactly which IID is being asked for.
    /// </summary>
    private static IMMDeviceEnumerator? CreateDeviceEnumerator()
    {
        var clsid = new Guid("BCDE0395-E52F-467C-8E3D-C4579291692E");
        var iid = typeof(IMMDeviceEnumerator).GUID;

        return CoCreateInstance(ref clsid, IntPtr.Zero, ClsCtxAll, ref iid, out object? instance) == 0
            ? instance as IMMDeviceEnumerator
            : null;
    }

    /// <summary>How COM ended up initialised on the calling thread.</summary>
    private enum ComApartment
    {
        /// <summary>Usable, and this call must be balanced with <c>CoUninitialize</c>.</summary>
        Owned,

        /// <summary>Usable; another apartment already owns the thread, so it must not be uninitialised.</summary>
        Borrowed,

        /// <summary>COM cannot be used on this thread.</summary>
        Unavailable,
    }

    /// <summary>
    /// Initialises COM for the calling thread and reports what the caller owes back.
    /// </summary>
    private static ComApartment InitializeCom()
    {
        int hr = CoInitializeEx(IntPtr.Zero, CoInitMultithreaded);

        // S_OK: initialised now. S_FALSE: already multithreaded, and this call
        // still took a reference. Both must be balanced.
        if (hr is 0 or 1)
        {
            return ComApartment.Owned;
        }

        // RPC_E_CHANGED_MODE: the thread is already in a single-threaded
        // apartment. COM is perfectly usable — the model simply cannot be changed,
        // and no reference was taken. Treating this as a failure used to skip the
        // restore and leave the user's speakers muted, which is the worst possible
        // outcome for a bug that only shows up on one kind of thread.
        if (hr == RpcChangedMode)
        {
            return ComApartment.Borrowed;
        }

        return ComApartment.Unavailable;
    }

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoInitializeEx(IntPtr reserved, int coInit);

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern void CoUninitialize();

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoCreateInstance(
        ref Guid clsid,
        IntPtr outer,
        int clsCtx,
        ref Guid iid,
        [MarshalAs(UnmanagedType.IUnknown)] out object? instance);
}

// ---------------------------------------------------------------------------
// Core Audio interop. Declared at namespace scope so the marshaller sees plain
// [ComImport] interfaces; the activation itself goes through CoCreateInstance.
// ---------------------------------------------------------------------------

internal enum EDataFlow
{
    Render = 0,
}

[ComImport]
[Guid("A95664D2-9614-4F35-A746-DE8DB63617E6")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IMMDeviceEnumerator
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
internal interface IMMDeviceCollection
{
    [PreserveSig]
    int GetCount(out int count);

    [PreserveSig]
    int Item(int index, out IMMDevice? device);
}

[ComImport]
[Guid("D666063F-1587-4E43-81F1-B948E807363F")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IMMDevice
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

/// <summary>
/// Vtable order is the ABI — do not reorder, insert or remove members.
/// </summary>
[ComImport]
[Guid("5CDF2C82-841E-4546-9722-0CF74078229A")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IAudioEndpointVolume
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
