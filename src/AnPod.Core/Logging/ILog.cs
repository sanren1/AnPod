namespace AnPod.Core.Logging;

/// <summary>Log levels actually used by AnPod. Deliberately small.</summary>
public enum LogLevel
{
    Debug = 0,
    Info,
    Warn,
    Error,
}

/// <summary>
/// Minimal logging seam. Two implementations exist (file logger, null logger);
/// it is not a speculative abstraction.
/// </summary>
public interface ILog
{
    void Debug(string message);

    void Info(string message);

    void Warn(string message);

    void Error(string message, Exception? exception = null);

    /// <summary>
    /// Registers a value that must never appear verbatim in the log
    /// (device serial, session token). See <see cref="LogRedactor"/>.
    /// </summary>
    void AddSecret(string? secret);

    void RemoveSecret(string? secret);
}
