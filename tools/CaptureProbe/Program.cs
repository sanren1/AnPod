using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using AnPod.Core.Logging;
using AnPod.Desktop.Services;

namespace AnPod.Tools.CaptureProbe;

/// <summary>
/// Answers one question that only real hardware can answer: does muting the
/// Windows render endpoint also silence WASAPI loopback capture?
/// <para>
/// This decides whether "automatically mute the local speakers while streaming
/// to the phone" is implementable at all. If the loopback tap sits downstream of
/// the endpoint mute, muting would mute the very audio AnPod sends, and the
/// feature would be self-defeating.
/// </para>
/// <para>
/// Method: play a steady 440 Hz tone, then alternate mute/unmute a dozen times,
/// measuring each window. Comparing the two groups rather than two sequential
/// samples is the whole point — a real desktop has other audio playing, and its
/// level drifts over the run. A sequential before/after would attribute that
/// drift to the mute. Alternating makes the drift common to both groups.
/// </para>
/// <para>
/// Each window is measured both as RMS and as the amplitude of the 440 Hz bin
/// (Goertzel). The bin isolates the tone from whatever else the machine is
/// playing; the RMS is reported alongside for context.
/// </para>
/// </summary>
internal static class Program
{
    private const string Token = "probe-token";
    private const double ToneFrequency = 440.0;

    /// <summary>Alternations. Enough that drift cannot favour either group.</summary>
    private const int Windows = 12;

    private static readonly TimeSpan Window = TimeSpan.FromSeconds(1.5);

    /// <summary>
    /// Gap between toggling the mute and measuring: the loopback tap plus the
    /// socket buffer is worth a few hundred ms, so without it the window still
    /// contains the previous state.
    /// </summary>
    private static readonly TimeSpan Settle = TimeSpan.FromMilliseconds(900);

    private static async Task<int> Main(string[] args)
    {
        Console.OutputEncoding = Encoding.UTF8;

        // `check` mode: report the endpoint state and exit. Used to observe the
        // app's mute lifecycle from outside — before, during and after a session.
        if (args.Contains("check"))
        {
            foreach (int role in new[] { EndpointInspector.ConsoleRole, EndpointInspector.MultimediaRole })
            {
                var state = EndpointInspector.Read(role);
                string name = role == EndpointInspector.ConsoleRole ? "console" : "multimedia";
                Console.WriteLine($"{name,-10} {state.ShortId}  {state.MuteText,-10} peak={state.PeakText}");
            }

            return 0;
        }

        // `unmute` mode: recovery. A killed AnPod cannot run its own teardown, so
        // this puts the machine's speakers back.
        if (args.Contains("unmute"))
        {
            int changed = EndpointInspector.ForceUnmuteAll();
            Console.WriteLine($"unmuted {changed} active render endpoint(s)");
            foreach (int role in new[] { EndpointInspector.ConsoleRole, EndpointInspector.MultimediaRole })
            {
                var state = EndpointInspector.Read(role);
                string name = role == EndpointInspector.ConsoleRole ? "console" : "multimedia";
                Console.WriteLine($"{name,-10} {state.ShortId}  {state.MuteText}");
            }

            return 0;
        }

        var log = new ConsoleLog();
        using var capture = new NativeAudioCapture(log);
        var muter = new PlaybackEndpointMuter(log);

        if (!capture.IsAvailable)
        {
            Console.WriteLine($"FAIL  capture DLL unusable: {capture.LoadError}");
            return 2;
        }

        string wav = Path.Combine(Path.GetTempPath(), "anpod-probe-tone.wav");
        WriteSineWave(wav, ToneFrequency);
        Console.WriteLine($"tone: {wav}");

        if (!capture.Initialize())
        {
            Console.WriteLine("FAIL  AudioCapture_Initialize failed — no default render endpoint?");
            return 2;
        }

        int port = FindFreeLoopbackPort();
        if (!capture.Listen(port, Token))
        {
            Console.WriteLine($"FAIL  AudioCapture_Listen({port}) failed");
            return 2;
        }

        using var client = new TcpClient();
        await client.ConnectAsync(IPAddress.Loopback, port);
        var stream = client.GetStream();
        await stream.WriteAsync(Encoding.UTF8.GetBytes(Token));

        if (!capture.Start())
        {
            Console.WriteLine("FAIL  AudioCapture_Start failed");
            return 2;
        }

        var header = new byte[8];
        if (!await TryReadExactAsync(stream, header))
        {
            Console.WriteLine("FAIL  no format header received");
            return 2;
        }

        uint sampleRate = BitConverter.ToUInt32(header, 0);
        ushort channels = BitConverter.ToUInt16(header, 4);
        ushort bits = BitConverter.ToUInt16(header, 6);
        int bytesPerSecond = (int)(sampleRate * channels * bits / 8);
        Console.WriteLine($"format: {sampleRate} Hz, {channels} ch, {bits} bit  ({bytesPerSecond} B/s)");

        if (bits != 16 || channels < 1)
        {
            Console.WriteLine("FAIL  probe expects 16-bit PCM");
            return 2;
        }

        // Is the tone reaching the endpoint at all? The peak meter answers that
        // without going through the capture path, so a flat reading here means the
        // probe's own signal source is broken rather than the capture being deaf.
        double quietPeak = await SamplePeakAsync(TimeSpan.FromSeconds(2));

        bool toneStarted = PlaySound(wav, IntPtr.Zero, SndFilename | SndAsync | SndLoop | SndNoDefault);
        await Task.Delay(600);
        double tonePeak = await SamplePeakAsync(TimeSpan.FromSeconds(2));

        Console.WriteLine($"PlaySound accepted the tone: {toneStarted}");
        Console.WriteLine($"endpoint peak: tone off {quietPeak:F4} | tone on {tonePeak:F4}");
        Console.WriteLine();

        var unmutedRms = new List<double>();
        var unmutedTone = new List<double>();
        var mutedRms = new List<double>();
        var mutedTone = new List<double>();
        var errors = new List<string>();

        Console.WriteLine("  #  endpoint   RMS      440Hz");
        for (int i = 0; i < Windows; i++)
        {
            bool shouldBeMuted = i % 2 == 1;

            if (shouldBeMuted)
            {
                muter.MuteAll();
            }
            else
            {
                muter.Restore();
            }

            await Task.Delay(Settle);

            var state = EndpointInspector.Read(EndpointInspector.ConsoleRole);
            var measured = await MeasureAsync(stream, bytesPerSecond, (int)sampleRate, channels);

            Console.WriteLine(
                $" {i,2}  {state.MuteText,-10} {measured.Rms,8:F1} {measured.Tone,8:F1}   peak={state.PeakText}");

            // Only trust a window whose endpoint state matches what we asked for.
            if (shouldBeMuted && state.Muted != true)
            {
                errors.Add($"window {i} should have been MUTED but was '{state.MuteText}'");
                continue;
            }

            if (!shouldBeMuted && state.Muted != false)
            {
                errors.Add($"window {i} should have been unmuted but was '{state.MuteText}'");
                continue;
            }

            if (shouldBeMuted)
            {
                mutedRms.Add(measured.Rms);
                mutedTone.Add(measured.Tone);
            }
            else
            {
                unmutedRms.Add(measured.Rms);
                unmutedTone.Add(measured.Tone);
            }
        }

        StopTone();
        muter.Restore();

        if (errors.Count > 0)
        {
            Console.WriteLine();
            foreach (string error in errors)
            {
                Console.WriteLine($"WARN  {error}");
            }
        }

        if (unmutedRms.Count < 3 || mutedRms.Count < 3)
        {
            Console.WriteLine();
            Console.WriteLine("INCONCLUSIVE — not enough valid windows in one of the groups.");
            return 3;
        }

        double meanUnmutedRms = unmutedRms.Average();
        double meanMutedRms = mutedRms.Average();
        double meanUnmutedTone = unmutedTone.Average();
        double meanMutedTone = mutedTone.Average();

        Console.WriteLine();
        Console.WriteLine($"windows: {unmutedRms.Count} unmuted, {mutedRms.Count} muted");
        Console.WriteLine($"RMS  mean: unmuted {meanUnmutedRms,8:F1}   muted {meanMutedRms,8:F1}   " +
                          $"ratio {Ratio(meanMutedRms, meanUnmutedRms)}");
        Console.WriteLine($"440Hz mean: unmuted {meanUnmutedTone,8:F1}   muted {meanMutedTone,8:F1}   " +
                          $"ratio {Ratio(meanMutedTone, meanUnmutedTone)}");

        // A mute downstream of the loopback tap is a gain of zero, so the captured
        // audio would collapse to the noise floor — not merely dip. Anything close
        // to parity means the tap is upstream of the mute.
        double ratio = meanMutedRms / Math.Max(meanUnmutedRms, 1e-9);
        bool muteKilledCapture = ratio < 0.1;

        Console.WriteLine();
        Console.WriteLine(muteKilledCapture
            ? "verdict: captured audio collapses while muted -> mute SILENCES loopback"
            : "verdict: captured audio survives the mute -> mute does NOT silence loopback");

        if (muteKilledCapture)
        {
            Console.WriteLine();
            Console.WriteLine("=> Auto-mute is NOT viable on this machine: it would mute the audio");
            Console.WriteLine("   being sent to the phone.");
            return 1;
        }

        Console.WriteLine();
        Console.WriteLine("=> Auto-mute is viable: local speakers go quiet, the phone keeps playing.");
        return 0;
    }

    private static string Ratio(double muted, double unmuted) =>
        unmuted <= 1e-9 ? "n/a" : $"{muted / unmuted:F2}x";

    /// <summary>Highest endpoint peak meter reading over a short sampling period.</summary>
    private static async Task<double> SamplePeakAsync(TimeSpan duration)
    {
        double highest = 0;
        var deadline = DateTime.UtcNow + duration;

        while (DateTime.UtcNow < deadline)
        {
            var state = EndpointInspector.Read(EndpointInspector.ConsoleRole);
            if (state.Peak is { } peak && peak > highest)
            {
                highest = peak;
            }

            await Task.Delay(60);
        }

        return highest;
    }

    private readonly record struct Measurement(double Rms, double Tone);

    /// <summary>
    /// Reads one window of PCM and returns its overall RMS plus the amplitude of
    /// <see cref="ToneFrequency"/>, taken from the first channel only.
    /// </summary>
    private static async Task<Measurement> MeasureAsync(
        Stream stream,
        int bytesPerSecond,
        int sampleRate,
        int channels)
    {
        long target = (long)(bytesPerSecond * Window.TotalSeconds);
        var buffer = new byte[64 * 1024];
        var channel0 = new List<short>((int)(sampleRate * Window.TotalSeconds) + 1024);

        long received = 0;
        double sumOfSquares = 0;
        long sampleCount = 0;

        while (received < target)
        {
            int read = await stream.ReadAsync(buffer);
            if (read <= 0)
            {
                break;
            }

            received += read;

            // 16-bit frames: sample k of channel c sits at (k * channels + c) * 2.
            int frameBytes = channels * 2;
            for (int offset = 0; offset + frameBytes <= read; offset += frameBytes)
            {
                short value = BitConverter.ToInt16(buffer, offset);
                sumOfSquares += (double)value * value;
                sampleCount++;
                channel0.Add(value);
            }
        }

        double rms = sampleCount == 0 ? 0 : Math.Sqrt(sumOfSquares / sampleCount);
        return new Measurement(rms, GoertzelAmplitude(channel0, sampleRate, ToneFrequency));
    }

    /// <summary>
    /// Amplitude of a single frequency, via the Goertzel algorithm. Scales the
    /// result so it is directly comparable to a sine wave's peak amplitude.
    /// </summary>
    private static double GoertzelAmplitude(List<short> samples, int sampleRate, double frequency)
    {
        int n = samples.Count;
        if (n == 0)
        {
            return 0;
        }

        double omega = 2 * Math.PI * frequency / sampleRate;
        double coefficient = 2 * Math.Cos(omega);
        double s1 = 0;
        double s2 = 0;

        foreach (short sample in samples)
        {
            double s0 = sample + coefficient * s1 - s2;
            s2 = s1;
            s1 = s0;
        }

        double real = s1 - s2 * Math.Cos(omega);
        double imaginary = s2 * Math.Sin(omega);

        // /(n/2) turns the DFT magnitude of a real signal into peak amplitude.
        return 2.0 * Math.Sqrt(real * real + imaginary * imaginary) / n;
    }

    private static async Task<bool> TryReadExactAsync(Stream stream, byte[] buffer)
    {
        int offset = 0;
        while (offset < buffer.Length)
        {
            int read = await stream.ReadAsync(buffer.AsMemory(offset));
            if (read <= 0)
            {
                return false;
            }

            offset += read;
        }

        return true;
    }

    private static int FindFreeLoopbackPort()
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        try
        {
            return ((IPEndPoint)listener.LocalEndpoint).Port;
        }
        finally
        {
            listener.Stop();
        }
    }

    /// <summary>Writes a mono 16-bit PCM sine wave, the one format PlaySound accepts.</summary>
    private static void WriteSineWave(string path, double frequency, int seconds = 300, int rate = 48000)
    {
        int frames = rate * seconds;
        int dataBytes = frames * 2;

        using var writer = new BinaryWriter(File.Create(path));
        writer.Write("RIFF"u8);
        writer.Write(36 + dataBytes);
        writer.Write("WAVE"u8);
        writer.Write("fmt "u8);
        writer.Write(16);
        writer.Write((short)1);           // PCM
        writer.Write((short)1);           // mono
        writer.Write(rate);
        writer.Write(rate * 2);           // byte rate
        writer.Write((short)2);           // block align
        writer.Write((short)16);          // bits per sample
        writer.Write("data"u8);
        writer.Write(dataBytes);

        for (int i = 0; i < frames; i++)
        {
            double value = Math.Sin(2 * Math.PI * frequency * i / rate);

            // Near full scale: the tone has to dominate whatever else the machine
            // is playing, otherwise the 440 Hz bin is measuring the room.
            writer.Write((short)(value * 30000));
        }
    }

    private const uint SndAsync = 0x0001;
    private const uint SndNoDefault = 0x0002;
    private const uint SndLoop = 0x0008;
    private const uint SndFilename = 0x00020000;

    [DllImport("winmm.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern bool PlaySound(string? sound, IntPtr module, uint flags);

    private static void StopTone() => PlaySound(null, IntPtr.Zero, 0);

    private sealed class ConsoleLog : ILog
    {
        public void Debug(string message) => Console.WriteLine($"  [debug] {message}");

        public void Info(string message) => Console.WriteLine($"  [info ] {message}");

        public void Warn(string message) => Console.WriteLine($"  [warn ] {message}");

        public void Error(string message, Exception? exception = null) =>
            Console.WriteLine($"  [error] {message} {exception?.Message}");

        public void AddSecret(string? secret)
        {
        }
    }
}
