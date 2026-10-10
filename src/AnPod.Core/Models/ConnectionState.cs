namespace AnPod.Core.Models;

/// <summary>
/// The single application-level state machine (plan §5.3). Transitions are
/// serialised by the ConnectionManager; this enum is the only thing the UI
/// binds to for button/status decisions.
/// </summary>
public enum ConnectionState
{
    /// <summary>No authorised USB device present. Audio capture must not run.</summary>
    NoDevice = 0,

    /// <summary>A USB device is present but has not accepted the debugging prompt.</summary>
    Unauthorized,

    /// <summary>An authorised USB device is present and idle.</summary>
    Ready,

    /// <summary>Building a session (push / reverse / handshake). Cancellable.</summary>
    Connecting,

    /// <summary>
    /// Session established (reverse tunnel, payload running, handshake done) but
    /// audio is not being forwarded — the user turned the "从手机播放声音"
    /// switch off. The phone stays connected; only the audio is paused, so
    /// turning the switch back on is instant and never re-handshakes.
    /// </summary>
    Connected,

    /// <summary>Handshake accepted, PCM is flowing to the phone.</summary>
    Streaming,

    /// <summary>Session lost (unplug, endpoint change); bounded retry in progress.</summary>
    Reconnecting,

    /// <summary>Session failed and will not retry without user action.</summary>
    Error,
}
