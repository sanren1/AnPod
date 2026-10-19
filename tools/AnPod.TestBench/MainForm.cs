using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;
using System.Web.Script.Serialization;

namespace AnPod.TestBench
{
    internal sealed class MainForm : Form
    {
        private readonly CheckedListBox _modes = new CheckedListBox();
        private TableLayoutPanel _rootLayout;
        private readonly Label _nextMode = new Label();
        private readonly RadioButton _budget30 = new RadioButton();
        private readonly RadioButton _budget120 = new RadioButton();
        private readonly ComboBox _devices = new ComboBox();
        private readonly Button _readPhone = new Button();
        private readonly CheckBox _healthEnabled = new CheckBox();
        private readonly Label _phoneStatus = new Label();
        private readonly CheckBox _endProbe = new CheckBox();
        private readonly Label _status = new Label();
        private readonly Label _clock = new Label();
        private readonly Label _output = new Label();
        private readonly Button _start = new Button();
        private readonly Button _stop = new Button();
        private readonly Button _build = new Button();
        private readonly Button _endProbeConfirm = new Button();
        private CancellationTokenSource _cancel;
        private Task<StageOutcome> _stageTask;
        private Task _deviceTask;
        private CancellationTokenSource _operationCancel;
        private readonly OperationGate _operationGate = new OperationGate();
        private bool _busy;
        private int _requestGeneration;
        private int _healthSuccessCount;
        private int _healthFailureCount;
        private Stopwatch _stageClock;
        private ManualResetEvent _endProbeReady;
        private RunOutcome _run;
        private StageOutcome _activeStage;
        private Dictionary<string, string> _pcApp;
        private string _selectedExePath;
        private string _adbPath;
        private string[] _sessionModes;
        private int _slotSeconds;
        private bool _allowClose;
        private bool _closeAfterSave;

        public MainForm(bool preview)
        {
            Text = "AnPod TestBench"; StartPosition = FormStartPosition.CenterScreen;
            MinimumSize = new Size(820, 700); ClientSize = new Size(980, 780);
            Font = new Font("Segoe UI", 9F); BackColor = Color.FromArgb(247, 249, 252);
            BuildUi(); _stop.Enabled = false; FormClosing += OnFormClosing;
        }

        private void BuildUi()
        {
            TableLayoutPanel root = new TableLayoutPanel { Dock = DockStyle.Fill, Padding = new Padding(24), ColumnCount = 1, RowCount = 7 };
            root.RowStyles.Add(new RowStyle(SizeType.Absolute, 50)); root.RowStyles.Add(new RowStyle(SizeType.Absolute, 190));
            root.RowStyles.Add(new RowStyle(SizeType.Absolute, 72)); root.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
            root.RowStyles.Add(new RowStyle(SizeType.Absolute, 116)); root.RowStyles.Add(new RowStyle(SizeType.Absolute, 28)); root.RowStyles.Add(new RowStyle(SizeType.Absolute, 28));
            _rootLayout = root; Controls.Add(root);
            root.Controls.Add(new Label { Text = "AnPod 延迟与稳定性测试", Font = new Font("Segoe UI Semibold", 19F), ForeColor = Color.FromArgb(22, 43, 68), Dock = DockStyle.Fill, TextAlign = ContentAlignment.MiddleLeft }, 0, 0);

            TableLayoutPanel setup = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 4, RowCount = 3, Padding = new Padding(12), BackColor = Color.White };
            setup.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 150)); setup.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50)); setup.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 100)); setup.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50));
            setup.RowStyles.Add(new RowStyle(SizeType.Percent, 100)); setup.RowStyles.Add(new RowStyle(SizeType.Absolute, 32)); setup.RowStyles.Add(new RowStyle(SizeType.Absolute, 32));
            setup.Controls.Add(Field("测试模式"), 0, 0);
            _modes.Items.AddRange(BenchPlan.Modes.Cast<object>().ToArray()); for (int i = 0; i < BenchPlan.Modes.Length; i++) _modes.SetItemChecked(i, true);
            _modes.Dock = DockStyle.Fill; _modes.Height = 90; setup.Controls.Add(_modes, 1, 0);
            setup.Controls.Add(Field("整轮预算"), 2, 0);
            FlowLayoutPanel budgets = new FlowLayoutPanel { Dock = DockStyle.Top, AutoSize = true, WrapContents = false };
            _budget30.Text = "30 分钟"; _budget30.Checked = true; _budget30.AutoSize = true; _budget30.Margin = new Padding(3, 4, 18, 3);
            _budget120.Text = "2 小时"; _budget120.AutoSize = true; _budget120.Margin = new Padding(3, 4, 3, 3);
            budgets.Controls.Add(_budget30); budgets.Controls.Add(_budget120); setup.Controls.Add(budgets, 3, 0);
            setup.Controls.Add(Field("手机（USB）"), 0, 1);
            _devices.DropDownStyle = ComboBoxStyle.DropDownList; _devices.Dock = DockStyle.Fill; _devices.AccessibleName = "已读取的 USB 手机"; _devices.AccessibleDescription = "仅可选择 adb devices 显示为 device 且由 get-devpath 确认为 USB 的设备。"; setup.Controls.Add(_devices, 1, 1);
            _devices.SelectedIndexChanged += delegate { AdbDevice value = _devices.SelectedItem as AdbDevice; if (value != null && !AdbTools.CanQuery(value)) { string state = value.State == "unauthorized" ? "手机尚未授权 USB 调试" : (value.State == "offline" ? "手机离线" : "设备不是已确认的 USB 连接"); _devices.SelectedIndex = -1; SetPhoneStatus(state + "；请选一台已授权 USB 手机。该项未查询。"); } };
            _readPhone.Text = "读取手机"; _readPhone.Dock = DockStyle.Fill; _readPhone.AccessibleName = "读取手机"; _readPhone.AccessibleDescription = "使用所选 AnPod 程序同目录中的 adb.exe 读取设备列表；不会自动选择设备。"; _readPhone.Click += ReadPhoneClicked; setup.Controls.Add(_readPhone, 2, 1);
            _healthEnabled.Text = "采集手机健康信息"; _healthEnabled.Checked = true; _healthEnabled.AutoSize = true; _healthEnabled.Dock = DockStyle.Fill; _healthEnabled.AccessibleName = "采集手机健康信息"; _healthEnabled.AccessibleDescription = "启用时，每项开始前先读取 Receiver 健康状态，阶段内每秒读取一次。"; setup.Controls.Add(_healthEnabled, 3, 1);
            _endProbe.Text = "结束探测"; _endProbe.Checked = true; _endProbe.AutoSize = true; _endProbe.Dock = DockStyle.Fill; _endProbe.AccessibleName = "每项结束时再探测一次"; _endProbe.AccessibleDescription = "默认开启；每项结束时，音乐已暂停后由用户确认开始探测。"; setup.Controls.Add(_endProbe, 0, 2);
            _phoneStatus.Text = "点“读取手机”并选择已授权手机；连接和切档仍在 AnPod 操作。"; _phoneStatus.Dock = DockStyle.Fill; _phoneStatus.AutoEllipsis = true; _phoneStatus.AccessibleName = "手机读取和预检状态"; _phoneStatus.AccessibleDescription = _phoneStatus.Text; setup.Controls.Add(_phoneStatus, 1, 2); setup.SetColumnSpan(_phoneStatus, 3);
            root.Controls.Add(setup, 0, 1);

            TableLayoutPanel actions = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 5, RowCount = 1, Padding = new Padding(12), BackColor = Color.White };
            actions.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 150)); actions.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100)); actions.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 150)); actions.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 125)); actions.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 140));
            _nextMode.Text = "下一项：USB 均衡"; _nextMode.Dock = DockStyle.Fill; _nextMode.TextAlign = ContentAlignment.MiddleLeft; actions.Controls.Add(_nextMode, 0, 0);
            _clock.Text = "每项 7 分 30 秒（含声学探测）"; _clock.Dock = DockStyle.Fill; _clock.TextAlign = ContentAlignment.MiddleLeft; _clock.Font = new Font("Segoe UI Semibold", 9F); actions.Controls.Add(_clock, 1, 0);
            _start.Text = "开始此项"; _start.Dock = DockStyle.Fill; _start.Click += StartClicked; actions.Controls.Add(_start, 2, 0);
            _stop.Text = "停止并保存"; _stop.Dock = DockStyle.Fill; _stop.Click += StopClicked; actions.Controls.Add(_stop, 3, 0);
            _build.Text = "选 AnPod 程序"; _build.Dock = DockStyle.Fill; _build.Click += SelectBuildClicked; actions.Controls.Add(_build, 4, 0);
            root.Controls.Add(actions, 0, 2);

            TableLayoutPanel info = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, RowCount = 2, Padding = new Padding(14), BackColor = Color.White };
            info.RowStyles.Add(new RowStyle(SizeType.Absolute, 34)); info.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
            _status.Text = "就绪。启动时不会扫描设备或采集音频；点击“开始此项”后才开始。"; _status.Dock = DockStyle.Fill; _status.Font = new Font("Segoe UI Semibold", 10F); _status.ForeColor = Color.FromArgb(35, 66, 92); info.Controls.Add(_status, 0, 0);
            TextBox guide = new TextBox { Multiline = true, ReadOnly = true, ScrollBars = ScrollBars.Vertical, Dock = DockStyle.Fill, BackColor = Color.FromArgb(250, 251, 253), BorderStyle = BorderStyle.None };
            guide.Text = "1. 选择程序并读取、选择授权手机；连接和切档在 AnPod 中操作。\r\n2. 暂停其他声音再开始；探测后播放熟悉音乐，听到异常就点按记录。\r\n3. 暂停音乐并确认结束探测，再手动切档；无线测试时 USB 仅用于采集状态。";
            info.Controls.Add(guide, 0, 1); root.Controls.Add(info, 0, 3);

            TableLayoutPanel markers = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 4, RowCount = 3, Padding = new Padding(12, 8, 12, 8), BackColor = Color.White };
            markers.RowStyles.Add(new RowStyle(SizeType.Absolute, 24)); markers.RowStyles.Add(new RowStyle(SizeType.Absolute, 34)); markers.RowStyles.Add(new RowStyle(SizeType.Absolute, 38));
            for (int i = 0; i < 4; i++) markers.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 25));
            Label markerLabel = new Label { Text = "观察到事件时点按记录：", Dock = DockStyle.Fill, TextAlign = ContentAlignment.MiddleLeft }; markers.Controls.Add(markerLabel, 0, 0); markers.SetColumnSpan(markerLabel, 4);
            AddMarker(markers, "卡顿", "glitch", 0); AddMarker(markers, "失真", "distortion", 1); AddMarker(markers, "断连", "disconnect", 2);
            _endProbeConfirm.Text = "音乐已暂停，开始结束探测"; _endProbeConfirm.Dock = DockStyle.Fill; _endProbeConfirm.Visible = false;
            _endProbeConfirm.Click += delegate { if (_endProbeReady != null) _endProbeReady.Set(); SetEndProbePrompt(false); };
            markers.Controls.Add(_endProbeConfirm, 0, 2); markers.SetColumnSpan(_endProbeConfirm, 4);
            root.Controls.Add(markers, 0, 4);
            _output.Text = "报告保存到“文档\\AnPod TestBench\\Runs”。"; _output.Dock = DockStyle.Fill; _output.TextAlign = ContentAlignment.MiddleLeft; _output.ForeColor = Color.FromArgb(87, 105, 122); root.Controls.Add(_output, 0, 5);
            Label privacy = new Label { Text = "序列号仅在本机读取、不写入报告；结果不代表音质 PASS。", Dock = DockStyle.Fill, ForeColor = Color.FromArgb(93, 112, 129), TextAlign = ContentAlignment.MiddleLeft };
            root.Controls.Add(privacy, 0, 6);
            _modes.ItemCheck += delegate { BeginInvoke(new Action(UpdateModeOptions)); };
            _budget30.CheckedChanged += delegate { UpdateModeOptions(); };
            _budget120.CheckedChanged += delegate { UpdateModeOptions(); };
            UpdateModeOptions();
        }

        private static Label Field(string text) { return new Label { Text = text, Dock = DockStyle.Fill, TextAlign = ContentAlignment.TopLeft, ForeColor = Color.FromArgb(50, 67, 84) }; }
        private void AddMarker(TableLayoutPanel panel, string label, string value, int column)
        {
            Button button = new Button { Text = label, Dock = DockStyle.Fill, Height = 32 };
            button.Click += delegate
            {
                if (_activeStage == null || _stageClock == null || !_stageClock.IsRunning) { SetStatus("请先开始一个测试项，再记录事件。"); return; }
                lock (_activeStage.Events) _activeStage.Events.Add(new MarkerEvent { Type = value, CapturedUtc = DateTime.UtcNow.ToString("o"), ElapsedMilliseconds = (int)_stageClock.ElapsedMilliseconds });
                SetStatus("已记录“" + label + "”事件，时间 " + TimeSpan.FromMilliseconds(_stageClock.ElapsedMilliseconds).ToString(@"hh\:mm\:ss") + "。");
            };
            panel.Controls.Add(button, column, 1);
        }

        private int TotalSeconds { get { return _budget120.Checked ? 7200 : 1800; } }
        private string[] CheckedModes() { return _modes.CheckedItems.Cast<string>().ToArray(); }
        private void UpdateModeOptions()
        {
            if (_run != null || _busy || IsDisposed) return;
            string[] checkedModes = CheckedModes();
            int seconds = checkedModes.Length == 0 ? 0 : BenchPlan.StageSeconds(TotalSeconds, checkedModes.Length);
            _clock.Text = checkedModes.Length == 0 ? "请至少勾选一个模式" : "每项 " + TimeSpan.FromSeconds(seconds).ToString(@"h\:mm\:ss") + "（探测计入预算）";
            _nextMode.Text = checkedModes.Length > 0 ? "下一项 " + checkedModes[0] : "未选择测试模式";
            _start.Enabled = checkedModes.Length > 0;
        }

        private async void StartClicked(object sender, EventArgs e)
        {
            if (_busy) return;
            if (_run != null && _run.Status != "in-progress") { ResetSession(); return; }
            if (_run == null && CheckedModes().Length == 0) return;
            if (_run == null && _pcApp == null) { MessageBox.Show(this, "请先选择本轮使用的 AnPod.exe，以便记录版本和 SHA-256。", "需要构建信息", MessageBoxButtons.OK, MessageBoxIcon.Information); return; }
            string[] modes = _run == null ? CheckedModes() : _sessionModes;
            string requestedMode = _run == null ? modes[0] : PendingModes().FirstOrDefault();
            if (requestedMode == null) return;
            string connectionHint = requestedMode.StartsWith("无线", StringComparison.Ordinal) ? "数据线仅用于记录状态；请在 AnPod 中保持无线连接。" : "请在 AnPod 中保持 USB 连接。";
            if (MessageBox.Show(this, "请先在 AnPod 中手动连接并切换到“" + requestedMode + "”。\r\n确认手机扬声器对准电脑默认麦克风，并暂停电脑上其他音乐后，点“是”开始。\r\n\r\n" + connectionHint, "确认测试路由", MessageBoxButtons.YesNo, MessageBoxIcon.Information) != DialogResult.Yes) return;
            if (!_operationGate.TryBegin()) return;
            _busy = true; int generation = _requestGeneration = _operationGate.Generation; _operationCancel = new CancellationTokenSource(); _cancel = _operationCancel;
            SetInputsEnabled(false); _start.Enabled = false; _stop.Enabled = true;
            try
            {
                AdbDevice selected = _devices.SelectedItem as AdbDevice;
                HealthSample preflight = null;
                if (_healthEnabled.Checked)
                {
                    if (!AdbTools.CanQuery(selected) || String.IsNullOrEmpty(_adbPath))
                    { SetPhoneStatus("未预检：选择授权 USB 手机，确保所选 AnPod 程序同目录有 adb.exe，再点读取手机重试。未开始声学探测。"); return; }
                    SetPhoneStatus("正在读取 Android 用户与 Receiver 健康状态…");
                    string serial = selected.Serial; CancellationToken token = _operationCancel.Token;
                    preflight = await Task.Run(() => BenchRunner.Preflight(_adbPath, serial, token));
                    if (!_operationGate.IsCurrent(generation, token.IsCancellationRequested) || generation != _requestGeneration) return;
                    if (preflight.Status != "collected")
                    { SetPhoneStatus("预检失败（" + preflight.FailureCategory + "）。请解锁手机、保持 USB 调试授权并确认 Receiver 已运行，再点“读取手机”重试；声学探测未开始。"); return; }
                    string actualName = HealthField(preflight, "versionName"), actualCode = HealthField(preflight, "versionCode");
                    string expectedName = BuildField("expectedReceiverVersionName"), expectedCode = BuildField("expectedReceiverVersionCode");
                    bool mismatch = expectedName != null && !RequestDecision.VersionMatches(expectedName, expectedCode, actualName, actualCode);
                    string text = actualName == null ? "Receiver 状态已读取（版本未报告）" : "Receiver " + actualName + (actualCode == null ? "" : " (" + actualCode + ")");
                    string modeValue = HealthField(preflight, "actualMode");
                    SetPhoneStatus(text + (mismatch ? "；与预期版本不一致" : (expectedName == null ? "；预期版本不可用" : "；版本一致")) + (modeValue == null ? "" : "；手机模式：" + HumanMode(modeValue)));
                    if (mismatch && MessageBox.Show(this, "手机报告 Receiver " + (actualName ?? "未知") + "，所选构建清单预期 " + expectedName + "。版本不一致，仍继续吗？", "Receiver 版本不一致", MessageBoxButtons.YesNo, MessageBoxIcon.Warning) != DialogResult.Yes) return;
                }
                else SetPhoneStatus("手机健康采集已关闭；本轮进行纯声学测试。");
                if (!_operationGate.IsCurrent(generation, _operationCancel.IsCancellationRequested) || generation != _requestGeneration) return;
                if (_run == null)
                {
                    _sessionModes = modes; _slotSeconds = BenchPlan.StageSeconds(TotalSeconds, modes.Length);
                    BenchRunner creator = new BenchRunner(AppDomain.CurrentDomain.BaseDirectory, _selectedExePath, _adbPath, _healthEnabled.Checked && selected != null ? selected.Serial : null, _endProbe.Checked, _operationCancel.Token, delegate { }, delegate { }, delegate { }, new Stopwatch(), delegate { }, null);
                    _run = creator.CreateRun(modes, _pcApp, _healthEnabled.Checked);
                }
                DateTime stageStart = DateTime.UtcNow;
                int manualWait = _run.Stages.Count == 0 ? 0 : Math.Max(0, (int)(stageStart - _run.Stages[_run.Stages.Count - 1].EndedUtc).TotalSeconds);
                _activeStage = new StageOutcome { Mode = requestedMode, StartedUtc = stageStart, PlannedCaptureSeconds = _slotSeconds, ManualInterStageWaitSeconds = manualWait };
                _run.Stages.Add(_activeStage); _stageClock = Stopwatch.StartNew(); _endProbeReady = new ManualResetEvent(false);
                _modes.Enabled = false; _budget30.Enabled = false; _budget120.Enabled = false; _devices.Enabled = false; _healthEnabled.Enabled = false; _endProbe.Enabled = false; _build.Enabled = false;
                _healthSuccessCount = 0; _healthFailureCount = 0;
                BenchRunner runner = new BenchRunner(AppDomain.CurrentDomain.BaseDirectory, _selectedExePath, _adbPath, _healthEnabled.Checked && selected != null ? selected.Serial : null, _endProbe.Checked, _operationCancel.Token, SetStatus, UpdateElapsed, UpdateHealthStatus, _stageClock, SetEndProbePrompt, _endProbeReady);
                _stageTask = Task.Run(delegate { return runner.RunStage(_run, _activeStage, _slotSeconds, preflight); });
                StageOutcome result = await _stageTask;
                if (result.Aborted) _run.Status = "aborted";
                else if (result.FailureCategory != null) _run.Status = "failed";
                else if (_run.Stages.Count == _sessionModes.Length) _run.Status = "completed";
                _run.EndedUtc = DateTime.UtcNow; ReportWriter.Write(_run);
                if (_run.Status == "completed") SetStatus("整轮已完成。报告已保存；工具不会判定音质 PASS。");
                else if (_run.Status == "aborted") SetStatus("本轮已中止。部分结果已保存，不是 PASS。");
                else if (_run.Status == "failed") SetStatus("本轮失败。已保存诊断信息；状态不是 PASS。");
                else SetStatus("此项完成。请手动切换到下一模式，再点击“开始此项”。");
                _output.Text = "本轮报告：" + _run.Folder; RebuildPendingModes();
            }
            catch (Exception ex) { SetStatus("无法完成本项：" + ex.Message); }
            finally
            {
                if (_stageClock != null) _stageClock.Stop(); _activeStage = null; _stageTask = null;
                if (_endProbeReady != null) { _endProbeReady.Dispose(); _endProbeReady = null; }
                if (_cancel != null) { _cancel.Dispose(); _cancel = null; _operationCancel = null; }
                _busy = false; _operationGate.End();
                if (!IsDisposed)
                {
                    _stop.Enabled = false;
                    if (_run != null)
                    {
                        bool ongoing = _run.Status == "in-progress";
                        SetInputsEnabled(!ongoing);
                        if (ongoing) { _devices.Enabled = true; _readPhone.Enabled = true; }
                        _start.Enabled = ongoing ? PendingModes().Length > 0 : true; _start.Text = ongoing ? "开始此项" : "开始新一轮";
                    }
                    else { SetInputsEnabled(true); UpdateModeOptions(); _start.Enabled = true; }
                    FinishDeferredClose();
                }
            }
        }

        private void RebuildPendingModes()
        {
            string[] pending = PendingModes();
            _nextMode.Text = pending.Length > 0 ? "下一项 " + pending[0] : "本轮已完成";
            _start.Enabled = pending.Length > 0 || _run.Status != "in-progress";
        }

        private string[] PendingModes() { return _sessionModes == null ? new string[0] : _sessionModes.Where(mode => !_run.Stages.Any(stage => stage.Mode == mode)).ToArray(); }

        private void ResetSession()
        {
            _run = null; _sessionModes = null; _slotSeconds = 0; _activeStage = null;
            _modes.Enabled = true; _budget30.Enabled = true; _budget120.Enabled = true; _devices.Enabled = true; _healthEnabled.Enabled = true; _endProbe.Enabled = true; _build.Enabled = true;
            _start.Text = "开始此项"; _output.Text = "报告保存到“文档\\AnPod TestBench\\Runs”。"; _status.Text = "新一轮已就绪。请先选择模式和 PC 程序构建。";
            UpdateModeOptions();
        }

        private void StopClicked(object sender, EventArgs e) { if (_operationCancel != null) { _operationCancel.Cancel(); if (_endProbeReady != null) _endProbeReady.Set(); SetStatus("正在取消当前读取并保存部分结果…"); } }

        private void OnFormClosing(object sender, FormClosingEventArgs e)
        {
            if (_allowClose) return;
            if (_operationGate.IsBusy)
            {
                e.Cancel = true; _closeAfterSave = true; if (_operationCancel != null) _operationCancel.Cancel(); if (_endProbeReady != null) _endProbeReady.Set(); SetStatus("关闭窗口前正在取消当前操作并保存结果…"); return;
            }
            if (_run != null && _run.Status == "in-progress")
            {
                _run.Status = "aborted"; _run.EndedUtc = DateTime.UtcNow; ReportWriter.Write(_run);
            }
        }

        private void UpdateElapsed(int seconds)
        {
            if (IsDisposed || !IsHandleCreated) return;
            if (InvokeRequired) { try { BeginInvoke(new Action<int>(UpdateElapsed), seconds); } catch { } return; }
            int left = Math.Max(0, _slotSeconds - seconds); _clock.Text = "本项预算剩余 " + TimeSpan.FromSeconds(left).ToString(@"h\:mm\:ss");
        }
        private void SetStatus(string text)
        {
            if (IsDisposed || !IsHandleCreated) return;
            if (InvokeRequired) { try { BeginInvoke(new Action<string>(SetStatus), text); } catch { } return; }
            _status.Text = text;
        }
        private void SetEndProbePrompt(bool visible)
        {
            if (IsDisposed || !IsHandleCreated) return;
            if (InvokeRequired) { try { BeginInvoke(new Action<bool>(SetEndProbePrompt), visible); } catch { } return; }
            _endProbeConfirm.Visible = visible;
        }

        private async void ReadPhoneClicked(object sender, EventArgs e)
        {
            if (String.IsNullOrEmpty(_adbPath) || !File.Exists(_adbPath))
            { SetPhoneStatus("未找到所选 AnPod 程序同目录的 adb.exe。请重新选择包含 platform-tools adb.exe 的 AnPod 目录后重试。"); return; }
            if (!_operationGate.TryBegin()) return;
            _busy = true; int generation = _requestGeneration = _operationGate.Generation; _operationCancel = new CancellationTokenSource();
            CancellationToken token = _operationCancel.Token; string adbPath = _adbPath;
            AdbDevice oldSelection = _devices.SelectedItem as AdbDevice;
            string oldSerial = oldSelection == null ? null : oldSelection.Serial;
            SetInputsEnabled(false); _start.Enabled = false; _stop.Enabled = true; SetPhoneStatus("正在读取 USB 设备列表…");
            try
            {
                _deviceTask = Task.Run(() => AdbTools.ReadDevices(adbPath, token));
                List<AdbDevice> found = await (Task<List<AdbDevice>>)_deviceTask;
                if (!_operationGate.IsCurrent(generation, token.IsCancellationRequested) || generation != _requestGeneration) return;
                _devices.Items.Clear(); foreach (AdbDevice device in found) _devices.Items.Add(device); _devices.SelectedIndex = -1;
                int previousIndex = found.FindIndex(d => d.Serial == oldSerial && AdbTools.CanQuery(d));
                if (previousIndex >= 0) _devices.SelectedIndex = previousIndex;
                int eligible = found.Count(AdbTools.CanQuery);
                SetPhoneStatus(eligible == 0 ? "未发现授权 USB 手机；请连接数据线、解锁并在手机上允许 USB 调试，然后重试。" : (previousIndex >= 0 ? "已保留之前明确选择的 USB 手机。" : "发现 " + eligible + " 台授权 USB 手机；请从列表中明确选择。未自动选中设备。"));
                _phoneStatus.AccessibleDescription = _phoneStatus.Text;
            }
            catch (OperationCanceledException) { }
            catch (Exception ex) { SetPhoneStatus("读取失败（" + ReportWriter.DiagnosticReason(ex) + "）。请检查 adb.exe 与 USB 调试授权，再点“读取手机”重试。"); }
            finally
            {
                FinishPhoneRead();
            }
        }

        internal void FinishPhoneRead()
        {
            _deviceTask = null; if (_operationCancel != null) { _operationCancel.Dispose(); _operationCancel = null; }
            _busy = false; _operationGate.End();
            if (!IsDisposed)
            {
                _stop.Enabled = false; bool idle = _run == null; bool ongoing = _run != null && _run.Status == "in-progress";
                SetInputsEnabled(idle || !ongoing);
                if (ongoing) { _devices.Enabled = true; _readPhone.Enabled = true; }
                _start.Enabled = !ongoing || PendingModes().Length > 0;
                _start.Text = ongoing ? "开始此项" : "开始新一轮";
                if (idle) UpdateModeOptions();
            }
            FinishDeferredClose();
        }

        private void FinishDeferredClose()
        {
            if (!_closeAfterSave || IsDisposed) return;
            if (_run != null && _run.Status == "in-progress")
            { _run.Status = "aborted"; _run.EndedUtc = DateTime.UtcNow; ReportWriter.Write(_run); }
            _allowClose = true;
            if (IsHandleCreated) BeginInvoke(new Action(Close)); else Close();
        }

        private void SetInputsEnabled(bool enabled)
        {
            _modes.Enabled = enabled; _budget30.Enabled = enabled; _budget120.Enabled = enabled; _devices.Enabled = enabled;
            _healthEnabled.Enabled = enabled; _endProbe.Enabled = enabled; _build.Enabled = enabled; _readPhone.Enabled = enabled;
        }

        private void SetPhoneStatus(string text)
        {
            if (IsDisposed || !IsHandleCreated) return;
            if (InvokeRequired) { try { BeginInvoke(new Action<string>(SetPhoneStatus), text); } catch { } return; }
            _phoneStatus.Text = text; _phoneStatus.AccessibleDescription = text;
        }

        private void UpdateHealthStatus(HealthSample sample)
        {
            if (IsDisposed || !IsHandleCreated) return;
            if (InvokeRequired) { try { BeginInvoke(new Action<HealthSample>(UpdateHealthStatus), sample); } catch { } return; }
            if (sample.Status == "collected") _healthSuccessCount++; else _healthFailureCount++;
            string version = HealthField(sample, "versionName") ?? "版本未读取";
            string mode = HealthField(sample, "actualMode");
            _phoneStatus.Text = sample.Status == "collected" ? "Receiver " + version + (mode == null ? "" : "；手机模式：" + HumanMode(mode)) + "；健康读取成功 " + _healthSuccessCount + " 次、失败 " + _healthFailureCount + " 次。" : "健康读取失败（" + sample.FailureCategory + "）；成功 " + _healthSuccessCount + " 次、失败 " + _healthFailureCount + " 次。";
            _phoneStatus.AccessibleDescription = _phoneStatus.Text;
        }

        private static string HealthField(HealthSample sample, string key) { string value; return sample != null && sample.Fields != null && sample.Fields.TryGetValue(key, out value) && value != "not-collected" ? value : null; }
        private string BuildField(string key) { string value; return _pcApp != null && _pcApp.TryGetValue(key, out value) && value != "unknown" ? value : null; }
        private static string HumanMode(string value)
        {
            switch (value) { case "balanced": return "均衡"; case "low-latency": return "低延迟"; case "balanced-limited": return "均衡（受限）"; case "low-latency-limited": return "低延迟（受限）"; case "balanced-fallback": return "均衡（回退）"; case "low-latency-fallback": return "低延迟（回退）"; default: return value; }
        }

        private void SelectBuildClicked(object sender, EventArgs e)
        {
            using (OpenFileDialog dialog = new OpenFileDialog { Filter = "AnPod 程序 (*.exe)|*.exe|所有文件 (*.*)|*.*", Title = "选择要记录版本的 AnPod 程序" })
            {
                if (dialog.ShowDialog(this) != DialogResult.OK) return;
                FileVersionInfo version = FileVersionInfo.GetVersionInfo(dialog.FileName);
                _selectedExePath = Path.GetFullPath(dialog.FileName);
                string hash;
                using (SHA256 sha = SHA256.Create()) using (FileStream input = File.OpenRead(dialog.FileName)) hash = BitConverter.ToString(sha.ComputeHash(input)).Replace("-", "").ToLowerInvariant();
                _adbPath = Path.Combine(Path.GetDirectoryName(dialog.FileName), "adb.exe");
                if (!File.Exists(_adbPath)) _adbPath = null;
                _pcApp = new Dictionary<string, string> { { "fileVersion", version.FileVersion ?? "unknown" }, { "productVersion", version.ProductVersion ?? "unknown" },
                    { "productName", version.ProductName ?? "unknown" }, { "sha256", hash }, { "metadataSource", "本机 EXE 版本资源与 SHA-256" }, { "runtimeInstance", "未自动核验" } };
                ReadBuildMetadata(Path.Combine(Path.GetDirectoryName(dialog.FileName), "build-metadata.json"), hash, _pcApp);
                string metadataStatus = _pcApp["buildMetadataStatus"] == "matched-to-exe-sha256" ? "清单摘要与该 EXE 匹配" : (_pcApp["buildMetadataStatus"] == "hash-mismatch" ? "清单摘要不匹配，未采用清单内容" : (_pcApp["buildMetadataStatus"] == "invalid" ? "清单无效，未采用清单内容" : "未找到同目录构建清单"));
                MessageBox.Show(this, "已读取 EXE 元数据和 SHA-256。路径不会写入报告，文件未启动、未安装。\r\n\r\n版本：" + _pcApp["fileVersion"] + "\r\nSHA-256：" + hash + "\r\n构建清单：" + metadataStatus, "构建信息", MessageBoxButtons.OK, MessageBoxIcon.Information);
            }
        }

        private static void ReadBuildMetadata(string path, string exeSha256, Dictionary<string, string> output)
        {
            output["buildMetadataStatus"] = "not-found";
            if (!File.Exists(path)) return;
            try
            {
                JavaScriptSerializer serializer = new JavaScriptSerializer();
                Dictionary<string, object> root = serializer.DeserializeObject(File.ReadAllText(path)) as Dictionary<string, object>;
                if (root == null) { output["buildMetadataStatus"] = "invalid"; return; }
                Dictionary<string, object> desktop = GetObject(root, "desktop");
                string metadataExeHash = null;
                if (desktop != null)
                {
                    object hashValue;
                    if (desktop.TryGetValue("sha256", out hashValue) && hashValue != null) metadataExeHash = Convert.ToString(hashValue, System.Globalization.CultureInfo.InvariantCulture);
                }
                if (!String.Equals(metadataExeHash, exeSha256, StringComparison.OrdinalIgnoreCase)) { output["buildMetadataStatus"] = "hash-mismatch"; return; }
                output["buildMetadataStatus"] = "matched-to-exe-sha256";
                CopyText(root, "productVersion", "buildProductVersion", output, "^[A-Za-z0-9._+-]{1,80}$");
                CopyText(root, "buildCommit", "buildCommit", output, "^[0-9a-fA-F]{40}$");
                CopyBool(root, "workingTreeDirty", "workingTreeDirty", output);
                CopyBool(root, "workingTreeReview", "workingTreeReview", output);
                if (desktop != null)
                {
                    CopyText(desktop, "sourceCommit", "desktopSourceCommit", output, "^[0-9a-fA-F]{40}$");
                    CopyText(desktop, "sourceInputsSha256", "desktopSourceInputsSha256", output, "^[0-9a-fA-F]{64}$");
                }
                Dictionary<string, object> receiver = GetObject(root, "receiver");
                if (receiver != null)
                {
                    CopyText(receiver, "versionName", "expectedReceiverVersionName", output, "^[A-Za-z0-9._+-]{1,100}$");
                    CopyText(receiver, "versionCode", "expectedReceiverVersionCode", output, "^[0-9]{1,12}$");
                    CopyText(receiver, "sha256", "expectedReceiverSha256", output, "^[0-9a-fA-F]{64}$");
                    CopyText(receiver, "sourceCommit", "receiverSourceCommit", output, "^[0-9a-fA-F]{40}$");
                    CopyText(receiver, "sourceInputsSha256", "receiverSourceInputsSha256", output, "^[0-9a-fA-F]{64}$");
                    CopyBool(receiver, "sourceModified", "receiverSourceModified", output);
                }
                output["buildMetadataSource"] = "same-directory build-metadata.json allowlist";
            }
            catch { output["buildMetadataStatus"] = "invalid"; }
        }

        private static Dictionary<string, object> GetObject(Dictionary<string, object> parent, string key)
        {
            object value; return parent.TryGetValue(key, out value) ? value as Dictionary<string, object> : null;
        }
        private static void CopyText(Dictionary<string, object> source, string key, string target, Dictionary<string, string> output, string pattern)
        {
            object value; string text;
            if (source.TryGetValue(key, out value) && value != null && (text = Convert.ToString(value, System.Globalization.CultureInfo.InvariantCulture)) != null && System.Text.RegularExpressions.Regex.IsMatch(text, pattern)) output[target] = text;
        }
        private static void CopyBool(Dictionary<string, object> source, string key, string target, Dictionary<string, string> output)
        {
            object value; if (source.TryGetValue(key, out value) && value is bool) output[target] = ((bool)value) ? "true" : "false";
        }

        public Bitmap RenderPreview(bool showEndProbeConfirmation = false)
        {
            _endProbeConfirm.Visible = showEndProbeConfirmation;
            Control parent = _rootLayout.Parent;
            if (parent != null) parent.Controls.Remove(_rootLayout);
            try
            {
                _rootLayout.Visible = true; _rootLayout.Dock = DockStyle.None; _rootLayout.Location = Point.Empty; _rootLayout.Size = ClientSize;
                _rootLayout.PerformLayout(); CreateControlTree(_rootLayout);
                Bitmap image = new Bitmap(ClientSize.Width, ClientSize.Height);
                _rootLayout.DrawToBitmap(image, new Rectangle(Point.Empty, _rootLayout.Size));
                return image;
            }
            finally
            {
                if (parent != null) { _rootLayout.Dock = DockStyle.Fill; parent.Controls.Add(_rootLayout); parent.PerformLayout(); }
            }
        }

        private static void CreateControlTree(Control control)
        {
            control.CreateControl(); control.PerformLayout();
            foreach (Control child in control.Controls) CreateControlTree(child);
        }
    }
}
