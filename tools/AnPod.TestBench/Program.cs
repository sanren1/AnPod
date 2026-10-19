using System;
using System.Drawing;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Diagnostics;
using System.Reflection;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;
using System.Web.Script.Serialization;

[assembly: AssemblyVersion("0.2.0.0")]
[assembly: AssemblyFileVersion("0.2.0.0")]
[assembly: AssemblyInformationalVersion("0.2.0")]

namespace AnPod.TestBench
{
    internal static class Program
    {
        [STAThread]
        private static int Main(string[] args)
        {
            try
            {
                if (Environment.GetEnvironmentVariable("ANPOD_TESTBENCH_FAKE_ADB") == "1" &&
                    String.Equals(Path.GetFileName(Environment.GetCommandLineArgs()[0]), "adb.exe", StringComparison.OrdinalIgnoreCase)) return FakeAdb.Run(args);
                if (args.Length > 0 && String.Equals(args[0], "--self-test", StringComparison.OrdinalIgnoreCase))
                {
                    MeasurementChecks.Run();
                    BenchChecks.Run();
                    Console.WriteLine("Self-test passed.");
                    return 0;
                }
                if (args.Length > 0 && (String.Equals(args[0], "--render-preview", StringComparison.OrdinalIgnoreCase) || String.Equals(args[0], "--render-preview-min", StringComparison.OrdinalIgnoreCase)))
                {
                    bool minimum = String.Equals(args[0], "--render-preview-min", StringComparison.OrdinalIgnoreCase);
                    string target = args.Length > 1 ? Path.GetFullPath(args[1]) : Path.Combine(Path.GetTempPath(), "AnPod-TestBench-preview.png");
                    Application.EnableVisualStyles();
                    Application.SetCompatibleTextRenderingDefault(false);
                    using (MainForm form = new MainForm(true))
                    {
                        if (minimum) form.Size = form.MinimumSize;
                        using (Bitmap image = form.RenderPreview(minimum)) image.Save(target, System.Drawing.Imaging.ImageFormat.Png);
                    }
                    Console.WriteLine(target);
                    return 0;
                }
                Application.EnableVisualStyles();
                Application.SetCompatibleTextRenderingDefault(false);
                Application.Run(new MainForm(false));
                return 0;
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine(ex.Message);
                return 1;
            }
        }
    }

    internal static class BenchChecks
    {
        public static void Run()
        {
            if (BenchPlan.StageSeconds(1800, 4) != 450 || BenchPlan.StageSeconds(7200, 4) != 1800 || BenchPlan.StageSeconds(1800, 2) != 900)
                throw new InvalidOperationException("Four-stage budget allocation is incorrect.");
            if (!BenchPlan.Modes.SequenceEqual(new[] { "USB 均衡", "USB 低延迟", "无线均衡", "无线低延迟" }))
                throw new InvalidOperationException("The mode plan is incorrect.");
            System.Collections.Generic.Dictionary<string, string> health = HealthParser.Parse("Row: 0 actualMode=balanced actualSampleRate=48000 serial=secret applicationId=app.anpod.receiver");
            if (health.ContainsKey("serial") || health["actualSampleRate"] != "48000" || health["versionName"] != "not-collected")
                throw new InvalidOperationException("Health parser accepted a non-whitelisted field.");
            if (HealthParser.Parse("Row: 0 actualSampleRate=-1")["actualSampleRate"] != "not-collected")
                throw new InvalidOperationException("Negative health metrics must be reported as uncollected.");
            if (HealthParser.Parse("Row: 0 actualMode=NULL actualSampleRate=0 playbackAdvancing=false")["actualMode"] != "not-collected" ||
                HealthParser.Parse("Row: 0 actualSampleRate=0 playbackAdvancing=false")["actualSampleRate"] != "0")
                throw new InvalidOperationException("Health NULL, zero, and false values are not distinguished correctly.");
            string query = HealthQuery.Build("device-123", "10");
            if (query != "-s \"device-123\" shell content query --user 10 --projection " + HealthQuery.Projection + " --uri " + HealthQuery.Uri || HealthQuery.Projection.IndexOf(',') >= 0)
                throw new InvalidOperationException("Health query arguments are incorrect.");
            if (HealthParser.FailureCategory("Error: Permission Denial requires android.permission.DUMP") != "permission-denied" || HealthParser.HasUsableRow("No result found"))
                throw new InvalidOperationException("Health query failure classification is incorrect.");
            if (ProcessMatch.Classify("C:\\AnPod\\AnPod.exe", new string[0], false, false) != "no-match" ||
                ProcessMatch.Classify("C:\\AnPod\\AnPod.exe", new string[0], true, false) != "access-denied" ||
                ProcessMatch.Classify("C:\\AnPod\\AnPod.exe", new string[0], false, true) != "process-exited" ||
                ProcessMatch.Classify("C:\\AnPod\\AnPod.exe", new[] { "C:\\AnPod\\AnPod.exe", "D:\\Old\\AnPod.exe" }, false, false) != "matched" ||
                ProcessMatch.Classify("C:\\AnPod\\AnPod.exe", new[] { "C:\\AnPod\\AnPod.exe", "C:\\AnPod\\AnPod.exe" }, false, false) != "multiple-matches")
                throw new InvalidOperationException("Fake process matching classifications are incorrect.");
            if (ProcessMatch.CpuPercent(null, null, 50.0, 1000.0, 4).HasValue ||
                Math.Abs(ProcessMatch.CpuPercent(1000.0, 1000.0, 1500.0, 2000.0, 4).Value - 12.5) > 0.001)
                throw new InvalidOperationException("CPU first-sample or logical-processor normalization is incorrect.");
            CheckAdbAndOperationPaths();
            CheckPhoneReadCompletionPaths();
            CheckTwoHourHealthReportCapacity();
        }

        private static void CheckPhoneReadCompletionPaths()
        {
            foreach (string status in new[] { "completed", "failed", "aborted" })
            {
                using (MainForm form = new MainForm(true))
                {
                    RunOutcome run = new RunOutcome { RunId = "synthetic", StartedUtc = DateTime.UtcNow, EndedUtc = DateTime.UtcNow, Status = status,
                        SelectedModes = new string[0], HealthCollectionEnabled = true, Folder = Path.Combine(Path.GetTempPath(), "AnPod.TestBench.ui-state." + Path.GetRandomFileName()) };
                    FieldInfo runField = typeof(MainForm).GetField("_run", BindingFlags.Instance | BindingFlags.NonPublic);
                    runField.SetValue(form, run); form.FinishPhoneRead();
                    Button start = (Button)typeof(MainForm).GetField("_start", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(form);
                    if (!start.Enabled || start.Text != "开始新一轮") throw new InvalidOperationException("A finished run did not restore the new-run action after phone read.");
                }
            }

            string folder = Path.Combine(Path.GetTempPath(), "AnPod.TestBench.ui-close." + Path.GetRandomFileName()); Directory.CreateDirectory(folder);
            try
            {
                using (MainForm form = new MainForm(true))
                {
                    RunOutcome run = new RunOutcome { RunId = "synthetic", StartedUtc = DateTime.UtcNow, EndedUtc = DateTime.UtcNow, Status = "in-progress",
                        SelectedModes = new string[0], HealthCollectionEnabled = true, Folder = folder };
                    typeof(MainForm).GetField("_run", BindingFlags.Instance | BindingFlags.NonPublic).SetValue(form, run);
                    typeof(MainForm).GetField("_closeAfterSave", BindingFlags.Instance | BindingFlags.NonPublic).SetValue(form, true);
                    form.FinishPhoneRead();
                    Dictionary<string, object> saved = new JavaScriptSerializer().DeserializeObject(File.ReadAllText(Path.Combine(folder, "run.json"))) as Dictionary<string, object>;
                    if (saved == null || Convert.ToString(saved["status"]) != "aborted") throw new InvalidOperationException("Closing after inter-stage phone read did not save the unfinished run as aborted.");
                }
            }
            finally { Directory.Delete(folder, true); }
        }

        private static void CheckAdbAndOperationPaths()
        {
            OperationGate gate = new OperationGate();
            if (!gate.TryBegin() || gate.TryBegin() || !gate.IsCurrent(gate.Generation, false) || gate.IsCurrent(gate.Generation - 1, false))
                throw new InvalidOperationException("Operation gate did not block overlapping starts or stale results.");
            gate.End(); if (gate.IsBusy || !gate.TryBegin()) throw new InvalidOperationException("Operation gate did not release after completion."); gate.End();
            if (!RequestDecision.VersionMatches("2.0", "20", "2.0", "20") || RequestDecision.VersionMatches("2.0", "20", "1.9", "19"))
                throw new InvalidOperationException("Receiver version comparison is incorrect.");

            string folder = Path.Combine(Path.GetTempPath(), "AnPod.TestBench.fake-adb." + Path.GetRandomFileName());
            Directory.CreateDirectory(folder);
            string adb = Path.Combine(folder, "adb.exe");
            File.Copy(Assembly.GetExecutingAssembly().Location, adb);
            string log = Path.Combine(folder, "calls.txt");
            Dictionary<string, string> previousEnvironment = new Dictionary<string, string>();
            foreach (string key in new[] { "ANPOD_TESTBENCH_FAKE_ADB", "ANPOD_FAKE_ADB_LOG", "ANPOD_FAKE_ADB_DEVICES", "ANPOD_FAKE_ADB_USB_SERIALS", "ANPOD_FAKE_ADB_HEALTH", "ANPOD_FAKE_ADB_BLOCK" }) previousEnvironment[key] = Environment.GetEnvironmentVariable(key);
            SetEnv("ANPOD_TESTBENCH_FAKE_ADB", "1");
            SetEnv("ANPOD_FAKE_ADB_LOG", log); SetEnv("ANPOD_FAKE_ADB_DEVICES", "List of devices attached\r\nusb-1 device model:Pixel_8 device:husky transport_id:1\r\nusb-2 device model:Pixel_7\r\nlocked unauthorized usb:2\r\noffline-3 offline usb:3\r\n");
            SetEnv("ANPOD_FAKE_ADB_USB_SERIALS", "usb-1,usb-2"); SetEnv("ANPOD_FAKE_ADB_HEALTH", "permission"); SetEnv("ANPOD_FAKE_ADB_BLOCK", null);
            try
            {
                List<AdbDevice> devices = AdbTools.ReadDevices(adb, CancellationToken.None);
                AdbDevice usb = devices.FirstOrDefault(d => d.Serial == "usb-1");
                if (usb == null || !AdbTools.CanQuery(usb) || devices.Count(d => AdbTools.CanQuery(d)) != 2 || AdbTools.CanQuery(devices.First(d => d.Serial == "locked")) || AdbTools.CanQuery(devices.First(d => d.Serial == "offline-3")))
                    throw new InvalidOperationException("Fake ADB device states or USB classification are incorrect.");
                string calls = File.ReadAllText(log);
                if (calls.Contains("locked") || calls.Contains("offline-3") || calls.Contains("-d devices"))
                    throw new InvalidOperationException("Unauthorized/offline devices were queried or fake ADB was incorrectly filtered with -d.");
                HealthSample failed = AdbTools.ReadHealth(adb, "usb-1", CancellationToken.None);
                if (failed.Status != "failed" || failed.FailureCategory != "permission-denied") throw new InvalidOperationException("Fake health query failure was not classified for retry.");
                SetEnv("ANPOD_FAKE_ADB_HEALTH", "row");
                HealthSample retry = AdbTools.ReadHealth(adb, "usb-1", CancellationToken.None);
                if (retry.Status != "collected" || retry.Fields["versionName"] != "2.0" || retry.Fields["actualMode"] != "low-latency-fallback") throw new InvalidOperationException("Fake health query retry or mode preservation failed.");
                SetEnv("ANPOD_FAKE_ADB_BLOCK", "1");
                using (CancellationTokenSource cancel = new CancellationTokenSource())
                {
                    Task<bool> task = Task.Run(() => { try { AdbTools.ReadProcess(adb, "devices -l", cancel.Token); return false; } catch (OperationCanceledException) { return true; } });
                    Thread.Sleep(300); cancel.Cancel();
                    if (!task.Wait(5000) || !task.Result) throw new InvalidOperationException("Fake ADB cancellation did not stop the owned child process.");
                }
            }
            finally
            {
                foreach (KeyValuePair<string, string> variable in previousEnvironment) SetEnv(variable.Key, variable.Value);
                Directory.Delete(folder, true);
            }
        }

        private static void SetEnv(string key, string value) { Environment.SetEnvironmentVariable(key, value); }

        private static void CheckTwoHourHealthReportCapacity()
        {
            string folder = Path.Combine(Path.GetTempPath(), "AnPod.TestBench.report-check." + Path.GetRandomFileName());
            Directory.CreateDirectory(folder);
            try
            {
                RunOutcome run = new RunOutcome { RunId = "synthetic", StartedUtc = DateTime.UtcNow, EndedUtc = DateTime.UtcNow,
                    Status = "completed", SelectedModes = new[] { "USB 均衡" }, HealthCollectionEnabled = true, Folder = folder };
                StageOutcome stage = new StageOutcome { Mode = "USB 均衡", StartedUtc = DateTime.UtcNow, EndedUtc = DateTime.UtcNow };
                for (int i = 0; i < 7200; i++)
                    stage.Health.Add(new HealthSample { CapturedUtc = DateTime.UtcNow.AddSeconds(i).ToString("o"), ElapsedSeconds = i,
                        Status = "collected", Fields = HealthParser.Parse(String.Empty) });
                run.Stages.Add(stage); ReportWriter.Write(run);
                string path = Path.Combine(folder, "run.json");
                if (new FileInfo(path).Length <= 2 * 1024 * 1024) throw new InvalidOperationException("Long report self-test did not exceed the default JSON limit.");
                JavaScriptSerializer serializer = new JavaScriptSerializer(); serializer.MaxJsonLength = 8 * 1024 * 1024;
                Dictionary<string, object> json = serializer.DeserializeObject(File.ReadAllText(path)) as Dictionary<string, object>;
                object[] stages = json == null ? null : json["stages"] as object[];
                Dictionary<string, object> first = stages == null || stages.Length == 0 ? null : stages[0] as Dictionary<string, object>;
                object[] health = first == null ? null : first["health"] as object[];
                if (health == null || health.Length != 7200) throw new InvalidOperationException("Long report self-test lost health samples.");
            }
            finally { Directory.Delete(folder, true); }
        }
    }

    internal static class FakeAdb
    {
        public static int Run(string[] args)
        {
            string joined = String.Join(" ", args);
            string log = Environment.GetEnvironmentVariable("ANPOD_FAKE_ADB_LOG");
            if (!String.IsNullOrEmpty(log)) File.AppendAllText(log, joined + "\r\n");
            if (Environment.GetEnvironmentVariable("ANPOD_FAKE_ADB_BLOCK") == "1") Thread.Sleep(60000);
            if (joined.IndexOf("devices", StringComparison.Ordinal) >= 0)
            { Console.WriteLine(Environment.GetEnvironmentVariable("ANPOD_FAKE_ADB_DEVICES") ?? "List of devices attached"); return 0; }
            if (joined.IndexOf("get-devpath", StringComparison.Ordinal) >= 0)
            {
                string[] usb = (Environment.GetEnvironmentVariable("ANPOD_FAKE_ADB_USB_SERIALS") ?? "").Split(',');
                string serial = args.Length > 1 ? args[1] : "";
                if (usb.Contains(serial)) Console.WriteLine("usb:1-1"); else Console.WriteLine("");
                return 0;
            }
            if (joined.IndexOf("get-current-user", StringComparison.Ordinal) >= 0) { Console.WriteLine("10"); return 0; }
            if (joined.IndexOf("content query", StringComparison.Ordinal) >= 0)
            {
                if (Environment.GetEnvironmentVariable("ANPOD_FAKE_ADB_HEALTH") == "permission") { Console.Error.WriteLine("Error: Permission Denial"); return 1; }
                if (Environment.GetEnvironmentVariable("ANPOD_FAKE_ADB_HEALTH") == "none") { Console.WriteLine("No result found."); return 0; }
                Console.WriteLine("Row: 0 applicationId=app.anpod.receiver versionName=2.0 versionCode=20 actualMode=low-latency-fallback actualSampleRate=48000"); return 0;
            }
            return 0;
        }
    }
}
