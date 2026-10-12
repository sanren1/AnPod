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

AnPod 将 Windows 当前默认播放设备的声音实时传到 Android 手机。音频通过 WASAPI Loopback 采集；Windows 桌面端通过经认证的直接网络会话，将 PCM 音频发送给 Receiver。Receiver 是需单独安装的 Android 前台服务应用。

## 功能

- Wi-Fi 主入口直接连接 Receiver；可选电脑与手机网络接口，使用已配对绑定进行认证。无线调试扫码、配对码和地址连接保留为过渡/诊断方式。
- 首次通过已授权 USB 连接安装或更新 Receiver、检查手机网络地址并完成绑定；已配对设备发现只提供地址提示，连接仍须通过双向认证。
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
| USB 首次绑定 | 支持数据传输的 USB 线，已开启并授权 USB 调试 |
| Wi-Fi | 手机与电脑处于可互访的网络；分别明确选择电脑和手机当前网络接口 |
| 过渡/诊断 | 无线调试配对需要 Android 11+ |

AnPod 需要可用的 Windows 默认音频输出设备。手机不会显示为 Windows 音频设备。ADB 授权会授予电脑 ADB 调试能力；请只连接信任的电脑和手机。

## 快速开始

从 `artifacts/` 解压完整 ZIP 到可写目录并运行 `AnPod.exe`。`adb.exe`、Platform-Tools DLL、`audio_capture.dll` 和 Android `server` 载荷必须与 EXE 同目录；请勿单独移动 EXE。包内含第三方许可与通知。首次运行需要系统已安装 WebView2 Runtime。

### 首次设置与 USB

1. 在手机开发者选项中开启 **USB 调试**，用数据线连接电脑，并在手机上确认调试授权。
2. 在 AnPod 中打开 **直接连接 Receiver**，选择已授权 USB 手机，并明确点击 **安装/更新 Receiver**。按 Android 提示确认安装及通知权限。
3. 选择电脑正在使用的网络接口，以及手机当前使用的网络接口；两端必须能互访。首次连接会通过 USB 绑定 Receiver。
4. 连接后点击 **手机输出** 开始播放。选择 **电脑输出** 可恢复电脑播放并保持手机连接。

手机卡片会区分 Receiver 安装状态、服务状态和手机已确认的播放状态。连接就绪不代表手机已开始播放。Receiver 没有 Launcher 图标，运行时通过前台服务通知显示。若用户强行停止 Receiver 或手机重启，需要用已授权 USB 再次启动；保留配对绑定即可，不需清除应用数据。点击卡片可展开设备列表；切换手机会先要求确认。结束时点击卡片旁的 **断开连接**，或从托盘菜单退出程序。

### Wi-Fi 连接

1. 按上面的首次设置步骤，通过 USB 安装 Receiver 并完成一次绑定。手机与电脑接入可互访的网络。
2. 在 AnPod 中选择 **Wi-Fi → 直接连接 Receiver**，分别选择电脑网络接口和手机网络接口。
3. 选择已配对 Receiver；也可先发现候选地址或手动输入手机地址。发现结果只是提示，连接时会执行双向认证。
4. 认证连接后点击 **手机输出**。若 Receiver 被强行停止或手机重启，使用 USB 再次启动服务。

无线调试二维码、配对码与地址连接位于 **无线调试（过渡/诊断）** 入口，不是直接 Receiver 主路径。若 Android Package Manager 因签名不同拒绝更新，请停止并保留应用数据；不要卸载 Receiver 或清除数据来绕过签名检查。候选包的 debug signer 与旧 P2 候选相同、与 P1 不同，因此没有验证或承诺从 P1 原地升级。

## 延迟测试

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

Receiver P1 候选包使用单独名称，不覆盖上面的稳定 ZIP。先运行 `packaging/build-receiver.ps1` 生成本机 debug 签名的 `deps/receiver.apk` 和校验清单，再执行：

```powershell
powershell -ExecutionPolicy Bypass -File packaging\publish-tauri.ps1 -Version 1.0.4 -IncludeReceiverCandidate
```

候选包会额外携带 Receiver APK 和 README 所用的 `icon.png`。Receiver 文件对常规旧版便携包是可选的，原有五个桌面运行依赖仍可独立构建。P1 APK 以 targetSdk 35 验收；targetSdk 36 的 target 专属行为尚未覆盖。debug 签名只用于本地 Spike，不代表 release 签名。

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
| 播放延迟明显 | 尝试低延迟模式并确认重连；实际效果取决于设备和链路，见上面的实测限制。 |
| 程序异常结束后电脑静音 | 重启 AnPod 恢复记录的静音状态，或在系统音量设置中手动取消静音。 |
| 提示缺少运行依赖 | 按 [deps/README.md](deps/README.md) 准备依赖，再重新构建或打包。 |

## 参与贡献

欢迎通过 [Issues](https://github.com/sanren1/AnPod/issues) 反馈问题，或通过 Pull Request 提交改动。请附上复现步骤、连接方式、系统版本和相关检查结果；尚未真机验证的行为应明确标注。

提交或推送前，检查全部待提交文件和 Git 暂存差异，删除本机绝对路径、用户名、设备序列号、音频端点 ID、真实网络地址、配对凭据和密钥。文档与测试使用明确的虚构设备标识及文档示例地址；原始录音、诊断转储、用户配置、工具链和构建产物留在本地。图片也应检查 EXIF、文本块等元数据，提交作者信息使用公开身份或匿名身份。第三方许可要求的署名和项目公开链接应保留。

## 许可与致谢

音频采集与 Android 播放服务基于 ysbing/AudioShare 的 LGPL-3.0-or-later 源码修改而来。第三方许可见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)、`LICENSES/` 与 `upstream/`。项目自有代码尚未声明独立许可证；再分发时应逐项核对第三方许可与通知要求。
