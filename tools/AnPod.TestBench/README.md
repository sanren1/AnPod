# AnPod TestBench 0.2.0

独立的 Windows 延迟与稳定性记录工具，适用于 Windows 10/11 x64、.NET Framework 4.8 或更高版本。程序不依赖 Python、NuGet 包或 AnPod 主程序。

## 使用

解压 `artifacts/AnPod-TestBench-0.2.0.zip`，运行 `Start-AnPod-TestBench.cmd`。启动时不访问 ADB、不打开 AnPod，也不采集音频。选择本轮的 `AnPod.exe` 后，工具只读取版本资源、SHA-256 和同目录的白名单构建清单，不启动或安装该文件。

点“读取手机”后，工具通过所选 AnPod 目录中的 `adb.exe` 读取 `adb devices -l`。只对 `device` 状态项按指定 serial 执行只读 `get-devpath`，并将返回 `usb:` 的设备标为 USB；不会查询 `unauthorized` 或 `offline` 项。列表不会自动选中手机；请明确选择一台已授权 USB 手机。多台设备时必须手动选择。序列号只在内存中用于本机读取，不写入报告。

手机健康采集默认开启。每个阶段开始前，工具查询所选设备的 Android 当前用户和 Receiver health provider；未找到授权 USB 设备、预检失败或没有可用健康行时，会阻止声学探测，并给出重试步骤。Receiver 实际版本和状态会显示在主界面，并与构建清单中的预期版本比较；不一致时需明确确认后才能继续。版本相同只表示“版本一致”，不代表二进制身份已验证。测试期间每秒读取健康状态，主界面实时显示 Receiver 版本、手机模式以及成功/失败样本数。可关闭健康采集，进行纯声学测试。

USB/无线均衡与低延迟模式可按需选择；每阶段都需用户在 AnPod 中手动连接、切换模式并确认。无线阶段可保留 USB 线读取手机状态，界面会提示“数据线仅用于记录状态；请在 AnPod 中保持无线连接”。工具不会运行 `adb connect`、`tcpip`、安装应用、切换产品模式或扫描网络。实际手机模式按 Receiver 回报展示，包括 `balanced`、`low-latency` 及 `-limited`、`-fallback` 状态。

默认总采集预算为 30 分钟，所选模式均分后逐项计时；也可选 2 小时。开始与默认开启的结束探测都计入阶段预算；结束探测前会等待用户点按“音乐已暂停，开始结束探测”，手动等待不计入预算。阶段内由用户在电脑播放熟悉的音乐，通过 AnPod 听手机输出，并点按记录卡顿、失真或断连。

探测通过 WASAPI 同时录制默认扬声器回环和默认麦克风，播放约 24 秒低电平扫频信号。请暂停电脑上的其他音乐，将手机扬声器对准电脑默认麦克风，并确认 Windows 默认端点与 AnPod 路由正确。报告记录的是播放、扬声器/环境和麦克风链路的端到端声学时差，不是纯传输延迟或音质 PASS；工具不会自动核验 Receiver 运行实例。

阶段开始后，工具按所选 EXE 文件名查找进程，并核对 `MainModule.FileName` 完全匹配；唯一匹配时每 5 秒读取 Working Set、Private Memory 和进程 CPU 时间差。CPU 百分比按逻辑处理器数归一化，首个 CPU 样本为 `null`。工具不会启动、控制或结束该进程；报告不包含 PID 或路径。CPU/RAM 指 PC 进程，不是手机资源。PLC/丢包隐藏计数和时钟漂移不在本版采集范围内。

报告保存在 `Documents/AnPod TestBench/Runs/`，每轮更新 `run.json`、`summary.csv` 和 `summary.md`。原始录音和端点元数据只留在本机运行目录。导出报告不含录音、序列号、地址、端点标识、文件路径或原始错误文本。

首版声学分析要求回采与麦克风均为 48kHz Float32；其他采样率或格式会明确报不支持，不生成延迟结论。

## 构建与自检

在安装 Visual Studio C++ x64 工具和 .NET Framework 4.x 编译器的 Windows 开发机上，从仓库根目录执行：

```powershell
powershell -ExecutionPolicy Bypass -File tools/AnPod.TestBench/build.ps1
```

脚本创建 `artifacts/AnPod-TestBench-0.2.0/` 和同名 ZIP；若任一目标已存在则拒绝覆盖。离线自检只使用合成数据和临时 fake ADB，不访问真实设备、不录音，也不启动 AnPod：

```powershell
artifacts/AnPod-TestBench-0.2.0/AnPod.TestBench.exe --self-test
artifacts/AnPod-TestBench-0.2.0/AnPod.TestBench.exe --render-preview "$env:TEMP/AnPod-TestBench-preview.png"
artifacts/AnPod-TestBench-0.2.0/AnPod.TestBench.exe --render-preview-min "$env:TEMP/AnPod-TestBench-preview-min.png"
```

源码位于本目录，WASAPI 采集器源码在 `native/capture_probe.cpp`。程序不调用 AnPod 或 Receiver 来设置模式。
