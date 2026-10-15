<p align="center">
  <img src="icon.png" alt="AnPod 图标" width="160">
</p>

<h1 align="center">AnPod</h1>

<p align="center">把 Windows 的声音，送到 Android 手机播放。</p>

<p align="center">
  <img src="https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4" alt="Windows 10 / 11 x64">
  <img src="https://img.shields.io/badge/Tauri-2-24C8D8" alt="Tauri 2">
  <img src="https://img.shields.io/badge/Android-6.0%2B-3DDC84" alt="Android 6.0 及以上">
</p>

<p align="center">
  <a href="#快速开始">快速开始</a> ·
  <a href="#延迟测试">延迟测试</a> ·
  <a href="#源码构建">源码构建</a> ·
  <a href="#常见问题">常见问题</a> ·
  <a href="https://github.com/sanren1/AnPod/issues">反馈问题</a>
</p>

AnPod 将 Windows 当前默认播放设备的声音发送到 Android 手机。USB 默认仍使用随包携带的 Legacy `server`，无需安装 Receiver；无线 UDP 使用单独安装的 AnPod Receiver。无线 ADB 作为兼容方式保留，需在设置中明确选择。

当前为 **PC 1.0.4 + Receiver 9 的 Preview**，下载见 [Releases](https://github.com/sanren1/AnPod/releases)。包含 Windows x64 便携客户端和 Android Receiver APK。USB 6→9 升级、USB Receiver 两种模式出声及无线重试后出声已有真机记录；**无线均衡仍有卡顿/破音，PC 连接操作顺序仍不清晰**，本版不宣称音质或完整流程验收通过。

## 功能

- USB 默认使用 Legacy 连接，播放无需安装 Receiver；也可在设置中选择已安装的 Receiver，均衡和低延迟模式均已完成一次真机声学短测。
- 无线默认推荐 UDP 直连 Receiver；无线 ADB 是显式兼容方式。首次安装和建立绑定使用已授权 USB；之后在 PC 允许连接，手机点击一次“开始接收”即可发现并认证连接。
- Android Receiver 提供接收状态、开始/停止和必要的网络选择，音频服务在后台运行；升级通过 PC 的 USB 入口进行。
- 手机播放、电脑播放切换；断开或暂停后恢复此前未静音的电脑音频端点。
- 均衡和低延迟播放模式，支持确认后重连切换。
- 设备重新出现后自动连接；Wi-Fi 音频会话掉线后进行有限次数恢复。
- 系统托盘、主题、大字体和本地诊断日志。

## 运行要求

| 项目 | 要求 |
| --- | --- |
| 电脑 | Windows 10/11 x64 |
| WebView | Microsoft Edge WebView2 Runtime；便携包不包含该运行时 |
| 手机 | Android 6.0+，需允许 Receiver 通知和前台服务运行 |
| USB | Legacy 连接和首次 Receiver 绑定都需要支持数据传输的 USB 线及已授权 USB 调试；Legacy 无需安装 Receiver |
| Wi-Fi | 手机与电脑处于同一可互访网络；UDP 直连需明确选择电脑接口，首次绑定时还要读取手机网络接口 |
| 过渡/诊断 | 无线调试配对需要 Android 11+ |

AnPod 需要可用的 Windows 默认音频输出设备。手机不会显示为 Windows 音频设备。ADB 授权会授予电脑 ADB 调试能力；请只连接信任的电脑和手机。

## 快速开始

从 [Releases](https://github.com/sanren1/AnPod/releases) 下载 Windows x64 便携 ZIP，完整解压到可写目录并运行 `AnPod.exe`。`adb.exe`、Platform-Tools DLL、`audio_capture.dll` 和 Android `server` 载荷必须与 EXE 同目录；请勿单独移动 EXE。包内含 Receiver APK、第三方许可与通知。首次运行需要系统已安装 WebView2 Runtime。本地构建输出位于 `artifacts/`。

### USB：Legacy 播放

1. 在手机开发者选项中开启 **USB 调试**，用数据线连接电脑，并在手机上确认调试授权。
2. 在 AnPod 选择 **USB**，从手机卡片选择目标手机并点击 **连接**。
3. 点击 **手机** 输出开始播放；点击 **电脑** 可恢复电脑输出。此 USB Legacy 路径无需安装 Receiver。

### USB：Receiver 播放

1. 使用已授权的 USB 手机，在 PC 设置中打开 **检查或安装 Receiver**，检查版本；需要时明确点击安装或升级，并在手机确认系统要求的权限。
2. 没有活动连接时，在 PC 设置将 **接收后端** 改为 **Receiver**，主界面选择 **USB** 和目标手机，再连接。
3. 选择声音输出 **手机**。播放模式可选均衡或低延迟；模式切换需要按界面提示重连。

该路径仍经 USB/ADB 传输 PCM，由手机 Receiver 缓冲并通过 AudioTrack 播放；USB 安装成功不等于已经选择了 Receiver 播放后端。

### 无线 UDP：直接连接 Receiver

在设置中选择 **无线 · UDP**（推荐），将手机和电脑连接到同一可互访的 Wi-Fi 或热点。首次安装和绑定需要一条已授权的 USB 调试连接：在 AnPod 的 **添加手机 / 检查或安装 Receiver** 页面选择 USB 手机，检查状态；必要时点击安装并在 Android 上确认安装和所需权限。之后选择电脑网络接口及手机 IPv4 接口完成首次绑定。不要卸载或清除已有应用数据来绕过签名检查。

已有绑定后，PC 主界面选择 **无线 → UDP 推荐**，打开 **选择目标**。在 **连接选项** 确认电脑网络接口和已配对接收端，再点击 **开始监听**。可用接口和绑定各只有一个时会预先选中；多项时仍须明确选择。手机打开 AnPod Receiver，点击 **开始接收**，即会准备服务、广播有效接收意图，并请求发现的电脑；PC 使用所选保存绑定认证后自动连接。连接后确认声音输出为 **手机**。发现地址不代表可信身份；没有绑定仍需 USB，多个待选目标保留必要选择，已有会话不会被抢占。手机搜索意图十秒后过期，取消或停止后旧请求不能重新开始播放。此流程需要两端新版本，旧端保留显式连接路径。

本次真机首次尝试停在等待状态，PC 记录握手关闭；用户手动重试后连接并出声。该失败原因尚未定位，不能将一次成功解释为自动连接可靠性通过。连接菜单和操作顺序仍待后续优化。

### 无线 ADB：兼容连接

在设置中明确选择 **无线 · ADB** 后，按 Android 无线调试页面完成配对和连接。该方式沿用 ADB 设备连接，不是 UDP 发现流程；无线调试扫码、配对码和地址连接也可作为过渡/诊断入口。首次授权仍应确认连接的是受信任电脑。

### Receiver USB/ADB 管理与重新启动

Receiver 安装和升级只通过明确选择的已授权 USB/ADB 设备进行。USB 升级要求本机 DPAPI 绑定材料、所选设备与 Android 用户匹配、手机仍保存相同绑定，并且候选 APK 高于已验证版本且签名一致；检查失败时不会发送 APK。网络 APK 上传和手机端自更新已退役。手机端显示的当前版本应来自经过本机 APK 摘要核验的 `receiver_status`，不能由发现信息或失败认证推断。

当前 Receiver 为 `versionCode` 9。本次已通过 PC 内 USB 入口从 code 6 升级到 9，核验安装后版本、APK 摘要与原绑定保留；签名与既有候选连续。发布 APK 使用候选 debug 签名，不代表生产签名。Receiver 不保证冷启动后自动恢复网络接收；手机重启或 Android 强行停止后，先在手机手动打开 AnPod Receiver。保留应用与绑定数据即可，不需要清除数据。

### 诊断说明

诊断页显示最近一次 PC/Receiver 状态快照中的格式、队列帧数、欠载计数和缓冲参数；未知值会留空。帧数、播放头推进状态或欠载数都不是声学延迟，也不能代替手机扬声器回录测试。code 9 无线均衡模式仍有用户实听的卡顿/破音；USB 短测也有欠载，音质验收未通过。通知停止、冷启动恢复、Activity 生命周期、大字体/TalkBack及停止后电脑声音恢复未完成本版合并真机验收。历史记录仅代表各自版本。

## 延迟测试

### 当前 USB Receiver 9 短测（2026-10-15）

PC 1.0.4 本轮候选、Android Receiver code 9，48 kHz 双声道 PCM16；每种模式播放 10 个扫频音，约 24 秒，同时采集 Windows WASAPI Loopback 和手机声音的麦克风回录，按每包 QPC 时间戳与波形匹配计算时差。两组使用相同录放采集端点和格式，先后测量，非严格受控长期对照。

| USB Receiver 模式 | 有效样本 | 回录延迟中位数 | P95 | 范围 | 本组新增 AudioTrack 欠载 | 本组新增积压丢弃帧 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 均衡 | 10/10 | 172.2 ms | 172.7 ms | 171.5–172.8 ms | 1 | 0 |
| 低延迟 | 10/10 | 125.7 ms | 127.0 ms | 121.4–127.4 ms | 6 | 3840 |

所有正式匹配片段未见采集断点或时间戳错误，并用第二脚本复核样点与时差。低延迟组中位数比均衡组低约 46.5 ms，但欠载更多；均衡的两次短预检约 194.9 ms，与正式组也有变化。这些结果不证明长期稳定或音质通过。

回录包含手机扬声器、声学路径及麦克风硬件/驱动/处理耗时；没有重测电脑基线，**不能视为纯 USB 传输延迟、绝对手机出声延迟，或与下列历史版本的受控收益对比**。原始录音、设备标识和诊断仅保留本地。

### 历史 USB / Wi-Fi ADB 测试（2026-10-11）

以下数据是 2026-10-11 对 AnPod 1.0.4 旧版 USB / Wi-Fi ADB 路径所做的声学回录测试，测试环境为 Windows x64、Android 16（API 36）、手机内置扬声器和同一套电脑耳机/麦克风，音频为 48 kHz 双声道 PCM16。它们不代表当前 Receiver v2 直接网络路径；v2 的真机出声、时延和长期稳定性尚未测试。设备型号、序列号和音频端点标识已省略。

每组播放 10 个短扫频测试音，约 24 秒；同步采集电脑 WASAPI Loopback 和麦克风回录，用每个音频包的 QPC 时间戳及波形相关匹配计算时差。另测电脑耳机输出作为基线，并独立复核匹配样点。

| 播放路径 | 回录延迟中位数 | P95 | 范围 | 相对电脑耳机基线的中位数差值 |
| --- | ---: | ---: | ---: | ---: |
| 电脑耳机基线 | 71.1 ms | 71.2 ms | 71.0–71.2 ms | — |
| 手机低延迟 | 177.6 ms | 178.2 ms | 177.1–178.2 ms | +106.4 ms |
| 手机均衡 | 274.1 ms | 274.8 ms | 273.5–274.8 ms | +203.0 ms |

本次低延迟模式比均衡模式快约 **96.5 ms**。手机接受的 AudioTrack 有效缓冲分别为 720 帧（15 ms）和 7688 帧（约 160.2 ms）；缓冲大小不等于实际播放延迟。均衡模式的 AudioTrack 欠载计数在测试期间为 0；低延迟模式开始测试时增加 1 次，之后未增长。全部正式测试音片段均无采集断点或时间戳错误。

### 历史 Wi-Fi ADB 测试

同日对 Windows 移动热点下的 Wi-Fi ADB TCP 连接进行了相同口径的测试，仍使用 Android 16 手机内置扬声器、48 kHz 双声道 PCM16 和同一套声学采集设备。未测定实际无线频段或协商标准。用户确认预检声音仅由手机发出。

| 播放模式 | 有效正式样本 | 回录延迟中位数 | P95 | 范围 |
| --- | ---: | ---: | ---: | ---: |
| 均衡 | 10 / 10 | 561.6 ms | 592.1 ms | 558.8–592.1 ms |
| 低延迟 | 无有效完整组 | — | — | — |

均衡模式两次预检音的中位数约为 303.1 ms，随后同一连接会话的正式测试升至上表水平；正式组后四次采样较前六次增加约 30 ms，AudioTrack 欠载计数由 8 增至 14。正式测试音片段无采集断点或时间戳错误，波形匹配结果经独立复核。这说明本次无线会话存在明显时延变化，短时预检不足以代表持续播放表现。

低延迟模式正式测试期间发生音频发送超时和设备离线，采样不完整；恢复后的预检也未得到可靠的手机出声匹配。因此本次不能比较 Wi-Fi 下两种模式的收益。测试期间电脑还有构建任务，且未重新测量电脑耳机基线，不将本次数据与 USB 测试的差值解释为纯网络开销。

> 回录延迟包含麦克风硬件、驱动和音频处理的耗时。基线差值是两条播放路径的比较，不能视为手机的绝对出声延迟或纯 AnPod 传输耗时。结果来自单台设备的短时测试，不代表其他设备或长期稳定性。原始录音、日志和设备转储仅保留在本地，不上传仓库。

2026-10-12 的 Receiver v2 源码检查发现并修复了低延迟模式在 AudioTrack 起播预滚动后队列余量为 0 的问题，本机合成合并检查通过。这不证明先前报告的严重卡顿已全部解决；用户重选均衡后音质改善是主观反馈，没有新的声学实测。首次断联后无法重连的情况没有复现，也没有修改重连状态机。v2 真机音频、连接恢复和界面视觉检查仍为 **NOT_TESTED**；此前取消的长时间测试不恢复执行。

当前 Receiver 使用协商格式初始化网络 PCM 流；只在收到有效 PCM、写入 AudioTrack 且近期观察到播放头推进时报告播放状态，这不是手机扬声器实际出声的证明。已认证的网络播放指标与 USB 读取的播放指标各有来源；队列和缓冲帧数不换算成延迟，AudioTrack 请求也不构成端到端声学延迟承诺。code 9 的真实安装、出声和 USB 短测范围见上文，其他未测行为不据此标为通过。

## 源码构建

主要桌面应用使用 Tauri 2、Rust、React、TypeScript 和 Vite。Windows 原生音频采集继续使用 C++ WASAPI DLL，Android 服务使用 Java。需要：

- Rust stable（至少 1.90）、Cargo 和 MSVC x64 C++ Build Tools。
- Node.js 与 npm。
- JDK 17 与 Android SDK API 34 / Build-Tools，用于重建 Android 音频服务；独立 Receiver Spike 使用 API 35。
- Android SDK Platform-Tools，用于准备 ADB 运行文件。

准备本机运行依赖：

```powershell
# 在 Visual Studio x64 Native Tools PowerShell 中执行
powershell -ExecutionPolicy Bypass -File packaging\build-native.ps1
powershell -ExecutionPolicy Bypass -File packaging\build-android.ps1
```

将受信任 Platform-Tools 同一发行包中的 `adb.exe`、`AdbWinApi.dll`、`AdbWinUsbApi.dll` 和该发行包的 `NOTICE.txt` 放入 `deps/`（NOTICE 文件命名为 `PLATFORM_TOOLS_NOTICE.txt`）。其余两个文件由上面的脚本生成。运行依赖不会提交到 Git，文件说明见 [deps/README.md](deps/README.md)。

构建前端：

```powershell
cd frontend
npm ci
npm run build
```

从仓库根目录创建 Windows x64 便携包：

```powershell
powershell -ExecutionPolicy Bypass -File packaging\publish-tauri.ps1 -Version 1.0.4
```

ZIP 和解压目录会写入 `artifacts/`。脚本会检查运行依赖、构建 React 前端和 Rust 桌面程序，并将许可证、上游记录和运行文件放进同一目录。打包脚本只构建 Tauri 主应用；WPF 回退版本保留在 `src/AnPod.Desktop/`，其旧打包入口为 `packaging/publish.ps1`。

历史 Receiver P1 候选包使用单独名称，不覆盖稳定 ZIP。先运行 `packaging/build-receiver.ps1` 生成本机 debug 签名的 `deps/receiver.apk` 和校验清单，再执行：

```powershell
powershell -ExecutionPolicy Bypass -File packaging\publish-tauri.ps1 -Version 1.0.4 -IncludeReceiverCandidate
```

候选包会额外携带 Receiver APK 和 README 所用的 `icon.png`。Receiver 文件对常规旧版便携包是可选的，原有五个桌面运行依赖仍可独立构建。P1 APK 以 targetSdk 35 验收；targetSdk 36 的 target 专属行为尚未覆盖。debug 签名只用于本地 Spike，不代表 release 签名。

### Receiver v3 本地候选包

当前 Receiver 为 `0.7.0-v3-flow-candidate` / `versionCode` 9 / minSdk 23 / targetSdk 35，隔离 APK 已构建、核验连续候选签名并完成一次 USB 6→9 升级。构建和打包均使用 `artifacts/` 下的新隔离目录，保留已有候选。准备 JDK 17、Android SDK Platform 35 / Build-Tools；桌面生产构建还需要 Rust stable 1.90+、MSVC x64、Node.js 和已安装的前端依赖。以下示例复用已保留且校验通过的 flow 候选 Native/Legacy 文件，需先设置本机连续候选签名的 `ANPOD_RECEIVER_DEBUG_KEYSTORE`：

```powershell
$previous = 'artifacts\AnPod-1.0.4-receiver-v3-v5-flow-candidate-review-tauri-win-x64-portable'
powershell -ExecutionPolicy Bypass -File packaging\build-receiver.ps1 -Variant debug -Offline -JavaHome $env:JAVA_HOME -AndroidSdkRoot $env:ANDROID_SDK_ROOT -DebugKeyStorePath $env:ANPOD_RECEIVER_DEBUG_KEYSTORE -OutputDirectory artifacts\v3-mobile-flow-receiver-new
powershell -ExecutionPolicy Bypass -File packaging\publish-tauri.ps1 -Version 1.0.4 -IncludeReceiverCandidate -ReceiverCandidate v3 -ReviewPackage -ReceiverArtifactDirectory artifacts\v3-mobile-flow-receiver-new -NativeArtifactDirectory $previous -LegacyPayloadDirectory $previous -RustTargetDirectory artifacts\v3-mobile-flow-tauri-new -AndroidSdkRoot $env:ANDROID_SDK_ROOT -Offline
```

密钥文件不应放入仓库或候选包。签名必须与已审计候选及已安装版本连续；不匹配时停止，不卸载或清除数据。上述入口构建桌面生产程序。源码未提交时，仅本地 V3 审查包可显式增加 `-ReviewWorkingTree`；它要求 APK 源码摘要匹配，并验证桌面构建前后输入未变。`sourceCommit` 是基线提交，`sourceInputsSha256` 标识已跟踪及未忽略的实际源码输入；`build-metadata.json` 明示工作区审查包和各组件来源。常规包仍要求干净源码。候选包不代表 release 签名或发布版本。

开发运行时由 Tauri 启动 Vite，在仓库根目录中执行：

```powershell
npm run tauri --prefix frontend -- dev
```

图标由根目录 `icon.png` 统一生成给 Tauri、网页 UI 和 WPF 回退程序：

```powershell
python packaging\make-icon.py
```

脚本需要 Python Pillow。它会裁去透明空白并保留原图比例；Windows 桌面 ICO 与 PNG 会更新，Android/iOS 模板图标不属于 Windows 发布源。

## 检查

```powershell
# 纯逻辑自检
dotnet run --project tests/AnPod.Core.Tests -c Release

# Tauri Rust 编译与单元检查
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

可选 UI 检查见 [前端检查说明](frontend/README.md)。连接、音频质量、延迟与掉线恢复仍需按 [手动检查清单](tests/manual-checklist.md) 在目标 Windows / Android 设备上验收。

## 工作原理

```text
Windows 默认输出设备
       │ WASAPI Loopback
       ▼
audio_capture.dll ── PCM16 ── 本机 TCP ── ADB reverse ── Android AudioTrack
```

| 路径 | 内容 |
| --- | --- |
| `frontend/` | React / TypeScript 界面 |
| `src-tauri/` | Tauri 命令、生命周期与 Windows 桌面配置 |
| `src/AnPod.Native/` | C++ WASAPI 音频采集源码 |
| `src/AnPod.AndroidServer/` | 手机端 Java 音频服务源码 |
| `src/AnPod.Desktop/` | 保留的 WPF 回退实现 |
| `packaging/` | 原生/Android 构建、图标和便携包脚本 |
| `tests/` | 自检、前端检查与人工设备验收清单 |

## 配置和日志

用户配置及静音恢复记录位于 `%LocalAppData%\AnPod\`，日志位于 `%LocalAppData%\AnPod\logs\`。可以在应用的诊断面板打开日志目录。提交诊断信息前请检查其中的设备标识和网络地址。

## 常见问题

| 问题 | 排查方式 |
| --- | --- |
| USB 手机未出现 | 检查数据线、USB 调试授权和设备驱动。 |
| 配对成功但未连接 | 使用无线调试主页面的连接端口，而非配对弹窗中的端口。 |
| 搜索不到无线设备 | 确认网络可互访、无线调试已开启；mDNS 不可用时手动填写连接地址。 |
| 手机没有声音 | 选择手机输出，检查手机媒体音量、音频路由及 Windows 默认播放设备。 |
| 播放卡顿或破音 | 先尝试均衡模式；仍有问题时核对诊断中的发送丢帧、接收过期帧与 AudioTrack 欠载，不能以显示“播放中”判断音质通过。 |
| 播放延迟明显 | 尝试低延迟模式并确认重连；实际效果取决于设备和链路，见上面的实测限制。 |
| 程序异常结束后电脑静音 | 重启 AnPod 恢复记录的静音状态，或在系统音量设置中手动取消静音。 |
| 提示缺少运行依赖 | 按 [deps/README.md](deps/README.md) 准备依赖，再重新构建或打包。 |

## 参与贡献

欢迎通过 [Issues](https://github.com/sanren1/AnPod/issues) 反馈问题，或通过 Pull Request 提交改动。请附上复现步骤、连接方式、系统版本和相关检查结果；尚未真机验证的行为应明确标注。

提交或推送前，检查全部待提交文件和 Git 暂存差异，删除本机绝对路径、用户名、设备序列号、音频端点 ID、真实网络地址、配对凭据和密钥。文档与测试使用明确的虚构设备标识及文档示例地址；原始录音、诊断转储、用户配置、工具链和构建产物留在本地。图片也应检查 EXIF、文本块等元数据，提交作者信息使用公开身份或匿名身份。第三方许可要求的署名和项目公开链接应保留。

## 许可与致谢

音频采集与 Android 播放服务基于 ysbing/AudioShare 的 LGPL-3.0-or-later 源码修改而来。第三方许可见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)、`LICENSES/` 与 `upstream/`。项目自有代码尚未声明独立许可证；再分发时应逐项核对第三方许可与通知要求。
