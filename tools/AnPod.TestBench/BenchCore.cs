using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Security;

namespace AnPod.TestBench
{
    internal static class BenchPlan
    {
        public static readonly string[] Modes = { "USB 均衡", "USB 低延迟", "无线均衡", "无线低延迟" };
        public static int StageSeconds(int totalSeconds, int selectedCount) { if (selectedCount < 1) throw new ArgumentOutOfRangeException("selectedCount"); return totalSeconds / selectedCount; }
    }

    internal sealed class HealthSample
    {
        public string CapturedUtc;
        public int ElapsedSeconds;
        public string Status;
        public string FailureCategory;
        public Dictionary<string, string> Fields;
    }

    internal sealed class AdbDevice
    {
        public string Serial;
        public string State;
        public string Details;
        public bool IsUsb;
        public string TransportFailure;
        public override string ToString()
        {
            string model = Regex.Match(Details ?? "", "(?:^|\\s)model:([^\\s]+)").Groups[1].Value;
            string state = State == "device" ? (IsUsb ? "已授权 USB" : (TransportFailure == null ? "已授权（非 USB）" : "USB 状态读取失败")) : (State == "unauthorized" ? "未授权" : (State == "offline" ? "离线" : State));
            return Serial + (model.Length == 0 ? "" : " — " + model) + " — " + state;
        }
    }

    internal static class AdbTools
    {
        public static List<AdbDevice> ParseDevices(string output)
        {
            List<AdbDevice> result = new List<AdbDevice>();
            ParseDeviceLines(output, d => { d.IsUsb = d.Details.Split(' ').Any(x => x.StartsWith("usb:", StringComparison.Ordinal)); result.Add(d); });
            return result;
        }

        public static List<AdbDevice> ReadDevices(string adbPath, CancellationToken cancel)
        {
            List<AdbDevice> devices = ParseDevices(ReadProcess(adbPath, "devices -l", cancel));
            foreach (AdbDevice device in devices.Where(d => d.State == "device" && !d.IsUsb))
            {
                try
                {
                    string devPath = ReadProcess(adbPath, "-s " + Quote(device.Serial) + " get-devpath", cancel).Trim();
                    device.IsUsb = devPath.StartsWith("usb:", StringComparison.OrdinalIgnoreCase);
                }
                catch (OperationCanceledException) { throw; }
                catch (HealthReadException ex) { device.TransportFailure = ex.Category; }
                catch (TimeoutException) { device.TransportFailure = "timeout"; }
                catch (FileNotFoundException) { device.TransportFailure = "adb-not-found"; }
                catch (System.ComponentModel.Win32Exception) { device.TransportFailure = "command-failed"; }
            }
            return devices;
        }

        private static void ParseDeviceLines(string output, Action<AdbDevice> add)
        {
            foreach (string line in (output ?? "").Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries))
            {
                string trimmed = line.Trim();
                if (trimmed.StartsWith("List of devices", StringComparison.OrdinalIgnoreCase) || trimmed.StartsWith("*", StringComparison.Ordinal)) continue;
                string[] parts = Regex.Split(trimmed, "\\s+");
                if (parts.Length < 2) continue;
                add(new AdbDevice { Serial = parts[0], State = parts[1], Details = String.Join(" ", parts.Skip(2).ToArray()) });
            }
        }

        public static bool CanQuery(AdbDevice device) { return device != null && device.State == "device" && device.IsUsb; }
        public static string ReadProcess(string adbPath, string arguments, CancellationToken cancel)
        {
            ProcessStartInfo info = new ProcessStartInfo(adbPath, arguments) { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true };
            using (Process process = Process.Start(info))
            {
                Task<string> stdout = Task.Run(() => process.StandardOutput.ReadToEnd());
                Task<string> stderr = Task.Run(() => process.StandardError.ReadToEnd());
                Stopwatch timer = Stopwatch.StartNew();
                while (!process.WaitForExit(100))
                {
                    if (cancel.IsCancellationRequested) { try { process.Kill(); } catch { } throw new OperationCanceledException(); }
                    if (timer.ElapsedMilliseconds >= 8000) { try { process.Kill(); } catch { } throw new TimeoutException(); }
                }
                Task.WaitAll(stdout, stderr);
                if (process.ExitCode != 0) throw new HealthReadException(HealthParser.FailureCategory(stdout.Result + "\n" + stderr.Result));
                return stdout.Result + "\n" + stderr.Result;
            }
        }

        public static HealthSample ReadHealth(string adbPath, string serial, CancellationToken cancel)
        {
            HealthSample sample = new HealthSample { CapturedUtc = DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture), Fields = HealthParser.Parse(String.Empty) };
            try
            {
                if (!Regex.IsMatch(serial ?? "", "^[A-Za-z0-9._:-]+$")) throw new InvalidOperationException("invalid-serial");
                string user = ReadProcess(adbPath, "-s " + Quote(serial) + " shell am get-current-user", cancel).Trim();
                if (!Regex.IsMatch(user, "^[0-9]+$")) throw new InvalidOperationException("invalid-user");
                string output = ReadProcess(adbPath, HealthQuery.Build(serial, user), cancel);
                if (!HealthParser.HasUsableRow(output)) throw new HealthReadException(HealthParser.FailureCategory(output));
                sample.Status = "collected"; sample.Fields = HealthParser.Parse(output);
            }
            catch (OperationCanceledException) { throw; }
            catch (HealthReadException ex) { sample.Status = "failed"; sample.FailureCategory = ex.Category; }
            catch (TimeoutException) { sample.Status = "failed"; sample.FailureCategory = "timeout"; }
            catch (FileNotFoundException) { sample.Status = "failed"; sample.FailureCategory = "adb-not-found"; }
            catch (InvalidOperationException ex) { sample.Status = "failed"; sample.FailureCategory = ex.Message == "invalid-serial" || ex.Message == "invalid-user" ? ex.Message : "command-failed"; }
            catch { sample.Status = "failed"; sample.FailureCategory = "query-failed"; }
            return sample;
        }
        private static string Quote(string value) { return "\"" + value.Replace("\"", "\\\"") + "\""; }
    }

    internal static class RequestDecision
    {
        public static bool IsCurrent(int currentGeneration, int resultGeneration, bool cancelled) { return currentGeneration == resultGeneration && !cancelled; }
        public static bool VersionMatches(string expectedName, string expectedCode, string actualName, string actualCode)
        { return expectedName != null && expectedName == actualName && (expectedCode == null || expectedCode == actualCode); }
    }

    internal sealed class OperationGate
    {
        private int _active;
        private int _generation;
        public bool TryBegin() { if (Interlocked.CompareExchange(ref _active, 1, 0) != 0) return false; Interlocked.Increment(ref _generation); return true; }
        public int Generation { get { return Thread.VolatileRead(ref _generation); } }
        public bool IsBusy { get { return Thread.VolatileRead(ref _active) != 0; } }
        public bool IsCurrent(int generation, bool cancelled) { return generation == Generation && IsBusy && !cancelled; }
        public void End() { Interlocked.Exchange(ref _active, 0); }
    }

    internal sealed class ProcessMetricSample
    {
        public string CapturedUtc;
        public int ElapsedSeconds;
        public string Status;
        public string FailureCategory;
        public long? WorkingSetBytes;
        public long? PrivateMemoryBytes;
        public double? CpuPercentLogicalProcessorNormalized;
    }

    internal sealed class MarkerEvent
    {
        public string Type;
        public string CapturedUtc;
        public int ElapsedMilliseconds;
    }

    internal sealed class StageOutcome
    {
        public string Mode;
        public DateTime StartedUtc;
        public DateTime EndedUtc;
        public int PlannedCaptureSeconds;
        public int CaptureSeconds;
        public int StabilitySeconds;
        public int ManualInterStageWaitSeconds;
        public int EndProbePauseWaitSeconds;
        public bool Aborted;
        public string FailureCategory;
        public string FailureReason;
        public string ReceiverIdentity = "运行实例未自动核验";
        public List<HealthSample> Health = new List<HealthSample>();
        public List<ProcessMetricSample> ProcessMetrics = new List<ProcessMetricSample>();
        public List<MarkerEvent> Events = new List<MarkerEvent>();
        public MeasurementResult StartMeasurement;
        public MeasurementResult EndMeasurement;
        public string RawFolder;
    }

    internal sealed class RunOutcome
    {
        public string RunId;
        public DateTime StartedUtc;
        public DateTime EndedUtc;
        public string Status;
        public string[] SelectedModes;
        public Dictionary<string, string> PcApp;
        public bool HealthCollectionEnabled;
        public List<StageOutcome> Stages = new List<StageOutcome>();
        public string Folder;
    }

    internal static class HealthParser
    {
        private static readonly string[] Allowed = {
            "applicationId", "versionName", "versionCode", "actualMode", "actualSampleRate", "actualChannels",
            "trackPendingFrames", "underrunCount", "underrunIncrement", "udpJitterTargetFrames", "udpQueuedFrames",
            "udpExpiredFrames", "tcpDroppedBacklogFrames", "playbackAdvancing"
        };

        public static Dictionary<string, string> Parse(string text)
        {
            Dictionary<string, string> fields = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (Match match in Regex.Matches(text ?? String.Empty, "([A-Za-z][A-Za-z0-9]*)=([^\\s,]+)"))
            {
                string key = match.Groups[1].Value;
                if (Allowed.Contains(key))
                {
                    string value = match.Groups[2].Value;
                    int numeric;
                    if (String.Equals(value, "null", StringComparison.OrdinalIgnoreCase) ||
                        (IsMetric(key) && Int32.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out numeric) && numeric < 0))
                        value = "not-collected";
                    fields[key] = value;
                }
            }
            foreach (string key in Allowed) if (!fields.ContainsKey(key)) fields[key] = "not-collected";
            return fields;
        }

        public static bool HasUsableRow(string text)
        {
            return Regex.IsMatch(text ?? String.Empty, "\\bRow:\\s*\\d+") && Regex.IsMatch(text ?? String.Empty, "\\b(actualMode|actualSampleRate|actualChannels|trackPendingFrames|underrunCount|playbackAdvancing)=");
        }

        public static string FailureCategory(string text)
        {
            string value = text ?? String.Empty;
            if (value.IndexOf("permission", StringComparison.OrdinalIgnoreCase) >= 0 || value.IndexOf("SecurityException", StringComparison.OrdinalIgnoreCase) >= 0 || value.IndexOf("DUMP", StringComparison.OrdinalIgnoreCase) >= 0) return "permission-denied";
            if (value.IndexOf("no result", StringComparison.OrdinalIgnoreCase) >= 0 || value.IndexOf("No row", StringComparison.OrdinalIgnoreCase) >= 0) return "no-row";
            return "query-failed";
        }

        private static bool IsMetric(string key)
        {
            return key == "actualSampleRate" || key == "actualChannels" || key == "trackPendingFrames" || key == "underrunCount" ||
                key == "underrunIncrement" || key == "udpJitterTargetFrames" || key == "udpQueuedFrames" || key == "udpExpiredFrames" || key == "tcpDroppedBacklogFrames";
        }
    }

    internal sealed class HealthReadException : Exception
    {
        public readonly string Category;
        public HealthReadException(string category) { Category = category; }
    }

    internal sealed class ProbeCancelledException : OperationCanceledException
    {
        public readonly int ElapsedMilliseconds;
        public ProbeCancelledException(int elapsedMilliseconds) { ElapsedMilliseconds = elapsedMilliseconds; }
    }

    internal static class ProcessMatch
    {
        public static string Classify(string selectedPath, IList<string> modulePaths, bool accessDenied, bool allExited)
        {
            if (accessDenied) return "access-denied";
            if (allExited) return "process-exited";
            int matches = 0;
            foreach (string path in modulePaths)
                if (path != null && String.Equals(Path.GetFullPath(path), Path.GetFullPath(selectedPath), StringComparison.OrdinalIgnoreCase)) matches++;
            if (matches > 1) return "multiple-matches";
            if (matches == 1) return "matched";
            return "no-match";
        }

        public static double? CpuPercent(double? previousCpuMs, double? previousElapsedMs, double currentCpuMs, double currentElapsedMs, int logicalProcessors)
        {
            if (!previousCpuMs.HasValue || !previousElapsedMs.HasValue || logicalProcessors < 1) return null;
            double elapsed = currentElapsedMs - previousElapsedMs.Value;
            double cpu = currentCpuMs - previousCpuMs.Value;
            if (elapsed <= 0.0 || cpu < 0.0) return null;
            return cpu / (elapsed * logicalProcessors) * 100.0;
        }
    }

    internal sealed class ProcessSampler : IDisposable
    {
        private Process _process;
        private readonly int _logicalProcessors;
        private double? _previousCpuMs;
        private double? _previousElapsedMs;
        private bool _terminal;

        private ProcessSampler(Process process, int logicalProcessors) { _process = process; _logicalProcessors = Math.Max(1, logicalProcessors); }

        public static ProcessSampler Attach(string selectedPath, int elapsedSeconds, out ProcessMetricSample initial)
        {
            string normalized = Path.GetFullPath(selectedPath);
            Process[] candidates;
            try { candidates = Process.GetProcessesByName(Path.GetFileNameWithoutExtension(normalized)); }
            catch (System.ComponentModel.Win32Exception) { initial = Failed(elapsedSeconds, "access-denied"); return null; }
            if (candidates.Length == 0) { initial = Failed(elapsedSeconds, "no-match"); return null; }

            List<string> modulePaths = new List<string>();
            bool denied = false;
            bool allExited = true;
            foreach (Process candidate in candidates)
            {
                try
                {
                    string modulePath = candidate.MainModule == null ? null : candidate.MainModule.FileName;
                    modulePaths.Add(modulePath); allExited = false;
                }
                catch (UnauthorizedAccessException) { denied = true; modulePaths.Add(null); }
                catch (SecurityException) { denied = true; modulePaths.Add(null); }
                catch (System.ComponentModel.Win32Exception ex) { if (ex.NativeErrorCode == 5) denied = true; modulePaths.Add(null); }
                catch (InvalidOperationException) { modulePaths.Add(null); }
            }
            string classification = ProcessMatch.Classify(normalized, modulePaths, denied, allExited);
            int selectedIndex = -1;
            if (classification == "matched")
                for (int i = 0; i < modulePaths.Count; i++)
                    if (modulePaths[i] != null && String.Equals(Path.GetFullPath(modulePaths[i]), normalized, StringComparison.OrdinalIgnoreCase)) { selectedIndex = i; break; }
            Process selected = selectedIndex < 0 ? null : candidates[selectedIndex];
            foreach (Process candidate in candidates) if (!Object.ReferenceEquals(candidate, selected)) candidate.Dispose();
            if (selected == null) { initial = Failed(elapsedSeconds, classification); return null; }

            ProcessSampler sampler = new ProcessSampler(selected, Environment.ProcessorCount);
            initial = sampler.Read(elapsedSeconds);
            if (initial.Status != "collected") { sampler.Dispose(); return null; }
            return sampler;
        }

        public ProcessMetricSample Read(int elapsedSeconds)
        {
            if (_terminal || _process == null) return Failed(elapsedSeconds, "process-exited");
            ProcessMetricSample sample = new ProcessMetricSample { CapturedUtc = DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture), ElapsedSeconds = elapsedSeconds };
            try
            {
                _process.Refresh();
                if (_process.HasExited) { _terminal = true; sample.Status = "failed"; sample.FailureCategory = "process-exited"; return sample; }
                sample.WorkingSetBytes = _process.WorkingSet64;
                sample.PrivateMemoryBytes = _process.PrivateMemorySize64;
                double cpuMs = _process.TotalProcessorTime.TotalMilliseconds;
                double elapsedMs = (double)Stopwatch.GetTimestamp() * 1000.0 / Stopwatch.Frequency;
                sample.CpuPercentLogicalProcessorNormalized = ProcessMatch.CpuPercent(_previousCpuMs, _previousElapsedMs, cpuMs, elapsedMs, _logicalProcessors);
                _previousCpuMs = cpuMs; _previousElapsedMs = elapsedMs;
                sample.Status = "collected";
            }
            catch (UnauthorizedAccessException) { _terminal = true; sample.Status = "failed"; sample.FailureCategory = "access-denied"; }
            catch (SecurityException) { _terminal = true; sample.Status = "failed"; sample.FailureCategory = "access-denied"; }
            catch (System.ComponentModel.Win32Exception ex) { _terminal = true; sample.Status = "failed"; sample.FailureCategory = ex.NativeErrorCode == 5 ? "access-denied" : "process-exited"; }
            catch (InvalidOperationException) { _terminal = true; sample.Status = "failed"; sample.FailureCategory = "process-exited"; }
            return sample;
        }

        private static ProcessMetricSample Failed(int elapsed, string category)
        {
            return new ProcessMetricSample { CapturedUtc = DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture), ElapsedSeconds = elapsed, Status = "failed", FailureCategory = category };
        }

        public void Dispose() { if (_process != null) { _process.Dispose(); _process = null; } }
    }

    internal static class HealthQuery
    {
        public const string Uri = "content://app.anpod.receiver.state/health";
        public const string Projection = "applicationId:versionName:versionCode:actualMode:actualSampleRate:actualChannels:trackPendingFrames:underrunCount:underrunIncrement:udpJitterTargetFrames:udpQueuedFrames:udpExpiredFrames:tcpDroppedBacklogFrames:playbackAdvancing";
        public static string Build(string serial, string user)
        {
            if (!Regex.IsMatch(serial ?? String.Empty, "^[A-Za-z0-9._:-]+$")) throw new ArgumentException("Invalid serial.", "serial");
            if (!Regex.IsMatch(user ?? String.Empty, "^[0-9]+$")) throw new ArgumentException("Invalid Android user.", "user");
            return "-s " + "\"" + serial + "\" shell content query --user " + user + " --projection " + Projection + " --uri " + Uri;
        }
    }

    internal sealed class BenchRunner
    {
        private readonly string _appFolder;
        private readonly string _processPath;
        private readonly string _adbPath;
        private readonly string _serial;
        private readonly bool _endProbe;
        private readonly CancellationToken _cancel;
        private readonly Action<string> _status;
        private readonly Action<int> _elapsed;
        private readonly Action<HealthSample> _healthUpdate;
        private readonly Action<bool> _endProbePrompt;
        private readonly ManualResetEvent _endProbeReady;
        private readonly Stopwatch _stageClock;

        public BenchRunner(string appFolder, string processPath, string adbPath, string serial, bool endProbe, CancellationToken cancel,
            Action<string> status, Action<int> elapsed, Action<HealthSample> healthUpdate, Stopwatch stageClock,
            Action<bool> endProbePrompt, ManualResetEvent endProbeReady)
        {
            _appFolder = appFolder; _processPath = processPath; _adbPath = adbPath; _serial = serial; _endProbe = endProbe; _cancel = cancel;
            _status = status; _elapsed = elapsed; _healthUpdate = healthUpdate; _stageClock = stageClock;
            _endProbePrompt = endProbePrompt; _endProbeReady = endProbeReady;
        }

        public RunOutcome CreateRun(string[] modes, Dictionary<string, string> pcApp, bool healthCollectionEnabled)
        {
            RunOutcome run = new RunOutcome();
            run.RunId = DateTime.UtcNow.ToString("yyyyMMdd'T'HHmmss'Z'", CultureInfo.InvariantCulture) + "-" + Guid.NewGuid().ToString("N").Substring(0, 8);
            run.StartedUtc = DateTime.UtcNow; run.Status = "in-progress"; run.SelectedModes = modes; run.PcApp = pcApp; run.HealthCollectionEnabled = healthCollectionEnabled;
            run.Folder = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments), "AnPod TestBench", "Runs", run.RunId);
            Directory.CreateDirectory(run.Folder);
            return run;
        }

        public StageOutcome RunStage(RunOutcome run, StageOutcome stage, int slotSeconds, HealthSample preflight)
        {
            stage.RawFolder = Path.Combine(run.Folder, "capture-" + run.Stages.Count.ToString("00", CultureInfo.InvariantCulture) + "-local");
            Directory.CreateDirectory(stage.RawFolder);
            int captureMillis = 0;
            string phase = "start-probe";
            ProcessSampler processSampler = null;
            run.Status = "in-progress";
            try
            {
                ProcessMetricSample initialProcessSample;
                processSampler = ProcessSampler.Attach(_processPath, 0, out initialProcessSample);
                stage.ProcessMetrics.Add(initialProcessSample);
                DateTime nextProcessSample = DateTime.UtcNow.AddSeconds(5);
                if (preflight != null) { preflight.ElapsedSeconds = 0; stage.Health.Add(preflight); _healthUpdate(preflight); }
                _status("正在采集开始声学探测…");
                Stopwatch probe = Stopwatch.StartNew();
                _status("请先暂停电脑播放的其他音乐，准备采集开始探测…");
                try { captureMillis += RunProbe(stage.RawFolder); }
                catch (ProbeCancelledException ex) { captureMillis += ex.ElapsedMilliseconds; throw; }
                probe.Stop();
                phase = "start-analysis";
                stage.StartMeasurement = Measurement.Analyze(stage.RawFolder);
                _cancel.ThrowIfCancellationRequested();
                int reserveEndProbe = _endProbe ? 26 : 0;
                int stabilityTarget = Math.Max(0, slotSeconds - (int)Math.Ceiling(probe.Elapsed.TotalSeconds) - reserveEndProbe);
                Stopwatch stable = Stopwatch.StartNew();
                phase = "stability";
                _status("声学探测已完成。请在电脑播放熟悉的音乐并听手机声音；遇到卡顿、破音或断连时点按事件按钮。正在记录剩余时间…");
                DateTime nextHealth = DateTime.MinValue;
                while (stable.Elapsed.TotalSeconds < stabilityTarget)
                {
                    _cancel.ThrowIfCancellationRequested();
                    int seconds = Math.Min(stabilityTarget, (int)stable.Elapsed.TotalSeconds);
                    stage.StabilitySeconds = seconds;
                    _elapsed(captureMillis / 1000 + (int)stable.Elapsed.TotalSeconds);
                    if (!String.IsNullOrWhiteSpace(_serial) && DateTime.UtcNow >= nextHealth)
                    {
                        nextHealth = DateTime.UtcNow.AddSeconds(1);
                        HealthSample sample = AdbTools.ReadHealth(_adbPath, _serial, _cancel);
                        sample.ElapsedSeconds = (int)(captureMillis / 1000 + stable.Elapsed.TotalSeconds);
                        stage.Health.Add(sample); _healthUpdate(sample);
                    }
                    if (processSampler != null && DateTime.UtcNow >= nextProcessSample)
                    {
                        nextProcessSample = DateTime.UtcNow.AddSeconds(5);
                        ProcessMetricSample sample = processSampler.Read((int)(captureMillis / 1000 + stable.Elapsed.TotalSeconds));
                        stage.ProcessMetrics.Add(sample);
                        if (sample.Status != "collected") { processSampler.Dispose(); processSampler = null; }
                    }
                    Thread.Sleep(100);
                }
                stage.StabilitySeconds = (int)stable.Elapsed.TotalSeconds;
                if (_endProbe)
                {
                    _cancel.ThrowIfCancellationRequested();
                    _status("稳定性阶段完成。请暂停电脑播放的音乐，再点击下方“音乐已暂停，开始结束探测”；手动等待不计入预算。");
                    _endProbePrompt(true);
                    Stopwatch pauseWait = Stopwatch.StartNew();
                    while (!_endProbeReady.WaitOne(100)) _cancel.ThrowIfCancellationRequested();
                    pauseWait.Stop(); stage.EndProbePauseWaitSeconds = (int)pauseWait.Elapsed.TotalSeconds;
                    _endProbePrompt(false);
                    _cancel.ThrowIfCancellationRequested();
                    string end = Path.Combine(run.Folder, "end-capture-" + run.Stages.Count.ToString("00", CultureInfo.InvariantCulture) + "-local");
                    Directory.CreateDirectory(end);
                    phase = "end-probe";
                    try { captureMillis += RunProbe(end); }
                    catch (ProbeCancelledException ex) { captureMillis += ex.ElapsedMilliseconds; throw; }
                    phase = "end-analysis";
                    stage.EndMeasurement = Measurement.Analyze(end);
                }
                run.Status = "in-progress";
            }
            catch (OperationCanceledException) { stage.Aborted = true; run.Status = "aborted"; }
            catch (Exception ex)
            {
                stage.FailureCategory = phase;
                stage.FailureReason = ReportWriter.DiagnosticReason(ex);
                run.Status = "failed"; _status("本项失败：" + stage.FailureReason + " / " + phase);
            }
            finally
            {
                if (processSampler != null) processSampler.Dispose();
                stage.CaptureSeconds = (captureMillis / 1000) + stage.StabilitySeconds;
                _endProbePrompt(false);
                stage.EndedUtc = DateTime.UtcNow; run.EndedUtc = stage.EndedUtc;
                ReportWriter.Write(run);
            }
            return stage;
        }

        private int RunProbe(string folder)
        {
            _cancel.ThrowIfCancellationRequested();
            string exe = Path.Combine(_appFolder, "capture_probe.exe");
            if (!File.Exists(exe)) throw new FileNotFoundException("找不到 WASAPI 采集组件。", exe);
            ProcessStartInfo info = new ProcessStartInfo(exe, "formal " + Quote(folder));
            info.UseShellExecute = false; info.CreateNoWindow = true; info.RedirectStandardError = true; info.RedirectStandardOutput = true;
            Stopwatch clock = Stopwatch.StartNew();
            using (Process process = Process.Start(info))
            {
                while (!process.WaitForExit(150))
                {
                    if (_cancel.IsCancellationRequested)
                    {
                        try { File.WriteAllText(Path.Combine(folder, "cancel.signal"), "cancel"); } catch { }
                        if (!process.WaitForExit(1800)) try { process.Kill(); } catch { }
                        clock.Stop(); throw new ProbeCancelledException((int)clock.ElapsedMilliseconds);
                    }
                }
                string error = process.StandardError.ReadToEnd();
                process.StandardOutput.ReadToEnd();
                if (process.ExitCode != 0) throw new InvalidOperationException(String.IsNullOrWhiteSpace(error) ? "WASAPI 采集没有完成。" : error.Trim());
            }
            clock.Stop(); return (int)clock.ElapsedMilliseconds;
        }

        private static string Quote(string value) { return "\"" + value.Replace("\"", "\\\"") + "\""; }

        public static HealthSample Preflight(string adbPath, string serial, CancellationToken cancel) { return AdbTools.ReadHealth(adbPath, serial, cancel); }
    }

    internal static class ReportWriter
    {
        public static void Write(RunOutcome run)
        {
            JavaScriptSerializer json = new JavaScriptSerializer();
            json.MaxJsonLength = 8 * 1024 * 1024;
            Dictionary<string, object> root = new Dictionary<string, object>();
            root["schemaVersion"] = 1; root["runId"] = run.RunId; root["status"] = run.Status;
            root["toolVersion"] = "0.2.0";
            root["startedUtc"] = run.StartedUtc.ToString("o"); root["endedUtc"] = run.EndedUtc.ToString("o");
            root["selectedModes"] = run.SelectedModes; root["pcApp"] = run.PcApp;
            root["healthCollectionEnabled"] = run.HealthCollectionEnabled;
            root["healthCollectionState"] = run.HealthCollectionEnabled ? "enabled" : "disabled-by-user";
            root["pcProcessSampling"] = new Dictionary<string, object> { { "source", "user-selected-exe-exact-main-module-match" },
                { "cpuNormalization", "logical-processor-normalized-percent" }, { "firstCpuSample", null } };
            root["notCollectedMetrics"] = new string[] { "plcCount", "clockDrift" };
            List<object> stages = new List<object>();
            foreach (StageOutcome stage in run.Stages)
            {
                MarkerEvent[] events;
                lock (stage.Events) events = stage.Events.ToArray();
                Dictionary<string, object> item = new Dictionary<string, object>();
                item["mode"] = stage.Mode; item["status"] = stage.Aborted ? "aborted" : (stage.FailureCategory == null ? "completed" : "failed");
                item["failureCategory"] = stage.FailureCategory;
                item["failureReason"] = stage.FailureReason;
                item["receiverIdentity"] = stage.ReceiverIdentity; item["plannedCaptureSeconds"] = stage.PlannedCaptureSeconds;
                item["captureSeconds"] = stage.CaptureSeconds; item["stabilitySeconds"] = stage.StabilitySeconds;
                item["manualInterStageWaitSeconds"] = stage.ManualInterStageWaitSeconds;
                item["endProbePauseWaitSeconds"] = stage.EndProbePauseWaitSeconds;
                item["startedUtc"] = stage.StartedUtc.ToString("o"); item["endedUtc"] = stage.EndedUtc.ToString("o");
                item["events"] = events.Select(e => new Dictionary<string, object> { { "type", e.Type }, { "capturedUtc", e.CapturedUtc }, { "elapsedMilliseconds", e.ElapsedMilliseconds } }).ToArray();
                item["startProbe"] = SafeMeasurement(stage.StartMeasurement); item["endProbe"] = SafeMeasurement(stage.EndMeasurement);
                item["health"] = stage.Health.Select(SafeHealth).ToArray();
                item["pcProcessMetrics"] = stage.ProcessMetrics.Select(SafeProcessMetric).ToArray();
                item["receiverVersionCheck"] = ReceiverVersionCheck(run.PcApp, stage.Health);
                stages.Add(item);
            }
            root["stages"] = stages;
            File.WriteAllText(Path.Combine(run.Folder, "run.json"), json.Serialize(root), new UTF8Encoding(false));
            StringBuilder csv = new StringBuilder("mode,status,capture_seconds,stability_seconds,manual_inter_stage_wait_seconds,end_probe_pause_wait_seconds,expected_pulses,detected_source_pulses,matched_pulses,valid_pulses,median_ms,p95_ms,health_samples,health_failures,pc_process_samples,pc_process_failures,pc_cpu_avg_percent_logical_normalized,pc_peak_working_set_bytes,pc_peak_private_memory_bytes,events\r\n");
            foreach (StageOutcome stage in run.Stages)
            {
                MarkerEvent[] events;
                lock (stage.Events) events = stage.Events.ToArray();
                MeasurementResult m = stage.StartMeasurement;
                csv.Append(Csv(stage.Mode)).Append(',').Append(stage.Aborted ? "aborted" : (stage.FailureCategory == null ? "completed" : "failed")).Append(',')
                    .Append(stage.CaptureSeconds.ToString(CultureInfo.InvariantCulture)).Append(',').Append(stage.StabilitySeconds.ToString(CultureInfo.InvariantCulture)).Append(',')
                    .Append(stage.ManualInterStageWaitSeconds.ToString(CultureInfo.InvariantCulture)).Append(',').Append(stage.EndProbePauseWaitSeconds.ToString(CultureInfo.InvariantCulture)).Append(',')
                    .Append(Expected(m)).Append(',').Append(m == null ? "" : m.DetectedSourceCount.ToString(CultureInfo.InvariantCulture)).Append(',')
                    .Append(m == null ? "" : m.MatchedCount.ToString(CultureInfo.InvariantCulture)).Append(',').Append(m == null ? "" : m.ValidCount.ToString(CultureInfo.InvariantCulture)).Append(',')
                    .Append(Number(m == null ? null : m.MedianMs)).Append(',').Append(Number(m == null ? null : m.P95Ms)).Append(',')
                    .Append(stage.Health.Count.ToString(CultureInfo.InvariantCulture)).Append(',').Append(stage.Health.Count(h => h.Status == "failed").ToString(CultureInfo.InvariantCulture)).Append(',')
                    .Append(stage.ProcessMetrics.Count(s => s.Status == "collected").ToString(CultureInfo.InvariantCulture)).Append(',')
                    .Append(stage.ProcessMetrics.Count(s => s.Status == "failed").ToString(CultureInfo.InvariantCulture)).Append(',')
                    .Append(Number(AvgCpu(stage.ProcessMetrics))).Append(',').Append(MaxMemory(stage.ProcessMetrics, true)).Append(',').Append(MaxMemory(stage.ProcessMetrics, false)).Append(',')
                    .Append(Csv(String.Join(";", events.Select(e => e.Type).ToArray()))).Append("\r\n");
            }
            File.WriteAllText(Path.Combine(run.Folder, "summary.csv"), csv.ToString(), new UTF8Encoding(true));
            StringBuilder md = new StringBuilder();
            md.AppendLine("# AnPod TestBench 测试记录").AppendLine().AppendLine("- 整轮状态：" + run.Status).AppendLine("- 开始时间（UTC）：" + run.StartedUtc.ToString("u"))
                .AppendLine("- 结束时间（UTC）：" + run.EndedUtc.ToString("u")).AppendLine("- PC 构建：" + BuildText(run.PcApp)).AppendLine("- Receiver 身份：运行实例未自动核验").AppendLine()
                .AppendLine("声学数值表示用户所选 AnPod 路由从输入到输出的声学时差；测试路由由用户选择，工具未自动核验。它不代表音质 PASS。").AppendLine();
            md.AppendLine("健康查询：" + (run.HealthCollectionEnabled ? "已启用（读取是否成功见各阶段样本和失败分类；serial 不写入报告）" : "用户已关闭健康采集；本轮为纯声学测试。")).AppendLine();
            md.AppendLine("PC CPU/RAM 仅在阶段开始后，对所选 EXE 的唯一精确 MainModule 匹配进程采样；不会启动或控制该进程。CPU 按逻辑处理器数归一化，首样 CPU 为空。PLC/丢包隐藏计数和时钟漂移未采集。").AppendLine();
            foreach (StageOutcome stage in run.Stages)
            {
                MarkerEvent[] events;
                lock (stage.Events) events = stage.Events.ToArray();
                md.AppendLine("## " + stage.Mode).AppendLine().AppendLine("状态：" + (stage.Aborted ? "中止" : (stage.FailureCategory == null ? "完成" : "失败")) + (stage.FailureCategory == null ? "" : "（" + stage.FailureCategory + ": " + stage.FailureReason + "）") + "；采集时长：" + stage.CaptureSeconds + " / " + stage.PlannedCaptureSeconds + " 秒；稳定阶段：" + stage.StabilitySeconds + " 秒；阶段间手动切换/等待：" + stage.ManualInterStageWaitSeconds + " 秒（不计入预算）；结束探测前暂停等待：" + stage.EndProbePauseWaitSeconds + " 秒（不计入预算）；健康样本：" + stage.Health.Count + "（失败 " + stage.Health.Count(h => h.Status == "failed") + "）；事件：" + String.Join("、", events.Select(e => e.Type).ToArray()) + "。");
                string reportedVersion = stage.Health.Select(h => Value(h.Fields, "versionName")).FirstOrDefault(v => v != null);
                if (reportedVersion != null) md.AppendLine("Receiver 报告版本：" + reportedVersion + "（运行实例未自动核验）。");
                List<ProcessMetricSample> processSamples = stage.ProcessMetrics.Where(s => s.Status == "collected").ToList();
                md.AppendLine("PC 进程资源：有效样本 " + processSamples.Count + "，CPU 平均 " + Number(AvgCpu(stage.ProcessMetrics)) + "%（逻辑处理器归一化），Working Set 峰值 " + FormatBytes(MaxMemoryValue(processSamples, true)) + "，Private Memory 峰值 " + FormatBytes(MaxMemoryValue(processSamples, false)) + ".");
                AppendMeasurement(md, "开始声学探测", stage.StartMeasurement); AppendMeasurement(md, "结束声学探测", stage.EndMeasurement);
                foreach (var group in stage.Health.Where(x => x.Status == "failed").GroupBy(x => x.FailureCategory))
                    md.AppendLine("- 健康读取“" + group.Key + "”：" + group.Count() + " 次；首/末时间 " + group.First().CapturedUtc + " / " + group.Last().CapturedUtc + "（详细样本见 run.json）。");
                foreach (var group in stage.ProcessMetrics.Where(x => x.Status == "failed").GroupBy(x => x.FailureCategory))
                    md.AppendLine("- PC 进程资源未采集“" + group.Key + "”：" + group.Count() + " 次；详细样本见 run.json。");
            }
            md.AppendLine().AppendLine("原始麦克风/回环录音和端点元数据只保存在本机运行目录，未放入这三份报告。");
            File.WriteAllText(Path.Combine(run.Folder, "summary.md"), md.ToString(), new UTF8Encoding(false));
        }

        private static object SafeMeasurement(MeasurementResult value)
        {
            if (value == null) return null;
            List<object> samples = new List<object>();
            foreach (MeasurementSample sample in value.Samples)
                samples.Add(new Dictionary<string, object> { { "pulseIndex", sample.PulseIndex }, { "loopbackFrameIndex", sample.LoopbackFrameIndex },
                    { "microphoneFrameIndex", sample.MicrophoneFrameIndex }, { "loopbackQpc100ns", sample.LoopbackQpc100ns },
                    { "microphoneQpc100ns", sample.MicrophoneQpc100ns }, { "delayMs", sample.DelayMs }, { "score", sample.Score },
                    { "loopbackScore", sample.LoopbackScore }, { "valid", sample.Valid }, { "reason", sample.Reason },
                    { "timestampError", sample.TimestampError }, { "discontinuity", sample.Discontinuity },
                    { "maxPacketTimestampResidualMs", sample.MaxPacketTimestampResidualMs }, { "ambiguousCandidateCount", sample.AmbiguousCandidateCount } });
            return new Dictionary<string, object> { { "loopbackSampleRate", value.LoopbackSampleRate }, { "microphoneSampleRate", value.MicrophoneSampleRate },
                { "expectedPulseCount", value.ExpectedPulseCount }, { "detectedSourceCount", value.DetectedSourceCount }, { "matchedCount", value.MatchedCount }, { "pulseCount", value.PulseCount }, { "validCount", value.ValidCount },
                { "medianMs", value.MedianMs }, { "p95Ms", value.P95Ms }, { "minMs", value.MinMs }, { "maxMs", value.MaxMs },
                { "measurementType", value.MeasurementType }, { "interpretation", value.Interpretation },
                { "warnings", value.Warnings.ToArray() }, { "samples", samples } };
        }
        private static object SafeHealth(HealthSample h)
        {
            Dictionary<string, object> fields = new Dictionary<string, object>();
            foreach (KeyValuePair<string, string> field in h.Fields) fields[field.Key] = field.Value == "not-collected" ? null : field.Value;
            return new Dictionary<string, object> { { "capturedUtc", h.CapturedUtc }, { "elapsedSeconds", h.ElapsedSeconds }, { "status", h.Status },
                { "failureCategory", h.FailureCategory }, { "fields", fields } };
        }
        private static object SafeProcessMetric(ProcessMetricSample sample)
        {
            return new Dictionary<string, object> { { "capturedUtc", sample.CapturedUtc }, { "elapsedSeconds", sample.ElapsedSeconds },
                { "status", sample.Status }, { "failureCategory", sample.FailureCategory }, { "workingSetBytes", sample.WorkingSetBytes },
                { "privateMemoryBytes", sample.PrivateMemoryBytes }, { "cpuPercentLogicalProcessorNormalized", sample.CpuPercentLogicalProcessorNormalized } };
        }
        private static double? AvgCpu(IEnumerable<ProcessMetricSample> samples)
        {
            List<double> values = samples.Where(s => s.Status == "collected" && s.CpuPercentLogicalProcessorNormalized.HasValue).Select(s => s.CpuPercentLogicalProcessorNormalized.Value).ToList();
            return values.Count == 0 ? (double?)null : values.Average();
        }
        private static long? MaxMemoryValue(IEnumerable<ProcessMetricSample> samples, bool workingSet)
        {
            List<long> values = samples.Where(s => s.Status == "collected").Select(s => workingSet ? s.WorkingSetBytes : s.PrivateMemoryBytes).Where(v => v.HasValue).Select(v => v.Value).ToList();
            return values.Count == 0 ? (long?)null : values.Max();
        }
        private static string MaxMemory(IEnumerable<ProcessMetricSample> samples, bool workingSet)
        {
            long? bytes = MaxMemoryValue(samples, workingSet); return bytes.HasValue ? bytes.Value.ToString(CultureInfo.InvariantCulture) : "";
        }
        private static string FormatBytes(long? bytes)
        {
            return bytes.HasValue ? (bytes.Value / (1024.0 * 1024.0)).ToString("F1", CultureInfo.InvariantCulture) + " MiB" : "未采集";
        }
        private static object ReceiverVersionCheck(Dictionary<string, string> build, List<HealthSample> health)
        {
            string expectedName = Value(build, "expectedReceiverVersionName");
            string expectedCode = Value(build, "expectedReceiverVersionCode");
            HealthSample collected = health.FirstOrDefault(h => h.Status == "collected");
            string reportedName = collected == null ? null : Value(collected.Fields, "versionName");
            string reportedCode = collected == null ? null : Value(collected.Fields, "versionCode");
            string status = "unavailable";
            if (expectedName != null && reportedName != null)
                status = String.Equals(expectedName, reportedName, StringComparison.Ordinal) && (expectedCode == null || String.Equals(expectedCode, reportedCode, StringComparison.Ordinal)) ? "reported-match" : "reported-mismatch";
            return new Dictionary<string, object> { { "status", status }, { "expectedVersionName", expectedName }, { "expectedVersionCode", expectedCode },
                { "reportedVersionName", reportedName }, { "reportedVersionCode", reportedCode }, { "identityVerified", false } };
        }
        private static string Value(Dictionary<string, string> values, string key)
        {
            string value; return values != null && values.TryGetValue(key, out value) && value != "not-collected" ? value : null;
        }
        private static void AppendMeasurement(StringBuilder md, string title, MeasurementResult m)
        {
            if (m == null) return;
            md.AppendLine().AppendLine(title + "：有效 " + m.ValidCount + "/" + Expected(m) + " 预期脉冲（检出源脉冲 " + m.DetectedSourceCount + "，匹配 " + m.MatchedCount + "），中位数 " + Number(m.MedianMs) + " ms，P95 " + Number(m.P95Ms) + " ms。");
            md.AppendLine("解释：包含播放呈现、扬声器/环境声学与麦克风采集链路，不是纯传输延迟；独立电脑基准不可从手机结果中相减。");
            foreach (string warning in m.Warnings) md.AppendLine("- " + warning);
        }
        private static string BuildText(Dictionary<string, string> build)
        {
            if (build == null || build.Count == 0) return "未选择";
            if (!build.ContainsKey("fileVersion")) return "元数据不可用";
            string result = build["fileVersion"] + " / EXE SHA-256 " + build["sha256"];
            string expected = Value(build, "expectedReceiverVersionName");
            if (expected != null) result += " / 预期 Receiver " + expected;
            if (build.ContainsKey("desktopSourceInputsSha256")) result += " / 源摘要 " + build["desktopSourceInputsSha256"];
            if (build.ContainsKey("workingTreeDirty")) result += " / 源树 dirty=" + build["workingTreeDirty"];
            if (build.ContainsKey("buildMetadataStatus")) result += " / 构建清单 " + build["buildMetadataStatus"];
            return result;
        }
        internal static string DiagnosticReason(Exception ex)
        {
            string reason = ex is InvalidDataException ? "invalid-capture-data" :
                (ex is FileNotFoundException ? "required-component-missing" :
                (ex is System.ComponentModel.Win32Exception ? "process-launch-failed" :
                (ex is UnauthorizedAccessException ? "access-denied" : ex.GetType().Name)));
            string message = (ex.Message ?? String.Empty).Replace('\r', ' ').Replace('\n', ' ');
            message = Regex.Replace(message, "[A-Za-z]:\\\\[^\\s,;]+", "[local path omitted]");
            message = Regex.Replace(message, "\\\\\\\\[^\\s,;]+", "[local path omitted]");
            if (message.Length > 240) message = message.Substring(0, 240);
            return reason + (String.IsNullOrWhiteSpace(message) ? "" : ": " + message);
        }
        private static string Expected(MeasurementResult m) { return m == null ? "" : (m.ExpectedPulseCount.HasValue ? m.ExpectedPulseCount.Value.ToString(CultureInfo.InvariantCulture) : m.PulseCount.ToString(CultureInfo.InvariantCulture)); }
        private static string Number(double? v) { return v.HasValue ? v.Value.ToString("F2", CultureInfo.InvariantCulture) : "未采集"; }
        private static string Csv(string v) { return "\"" + (v ?? "").Replace("\"", "\"\"") + "\""; }
    }
}
