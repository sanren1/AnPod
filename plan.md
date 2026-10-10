# AnPod — Windows / Android USB + Wi-Fi 音频播放工具开发计划

> 版本：v1.1（USB + 无线连接扩展开发执行规格）  
> 日期：2026-10-11（本轮更新：mDNS 僵死探测、按模式过滤设备列表、配对码框可用性修复）  
> 项目名称：**AnPod**  
> 项目定位：通过 USB 数据线、同一局域网或 Android 连接 Windows 热点，将 Windows PC 的系统音频实时传到 Android 手机扬声器播放；**仅 Windows 端提供图形界面**。  
> 原则：**个人使用优先、尽量轻量、可靠性优先、先复用后优化、不增加手机端可见 App，复用单一音频传输链路**。

---

## 0. 交付目标与实施摘要

AnPod 是便携式 Windows 小工具。用户可通过 USB 数据线、同一局域网或 Windows 移动热点连接 Android 手机。USB 模式下，用户通过数据线连接 Android 手机，首次在手机开发者选项中启用 USB 调试并授权该 PC，之后在 AnPod 中点击“连接”，Windows 系统音频在手机端播放。手机不需要安装可见 Android App；AnPod 通过 `adb push` 推送服务载荷，使用 `app_process` 启动没有 UI 的 Android Java 接收进程。该方式**不是**标准 USB Audio Class 设备模拟，手机**不会**作为一个真实音频输出设备出现在 Windows 的声音设置中。Windows 端仍需有可用的音频输出端点供 WASAPI Loopback 采集。

**v1.0 USB 基线必须具备：** USB ADB 设备发现、授权状态诊断、设备选择（多个手机时）、连接/断开、WASAPI PCM16 采集、ADB Reverse、Android AudioTrack 播放、基本自动重连、托盘、简单日志、整洁卸载/退出。

**v1.1 新增必须具备：** 同一局域网无线 ADB 配对/连接、Windows 热点网络下无线 ADB 连接、配对码输入与设备地址手动兜底、USB/Wi-Fi 模式切换、无线连接恢复与可理解的错误提示。旧版 Android 的 USB 引导 TCP/IP 为可选兼容功能。

**v1.1 不做：** 独立音频 UDP/Opus 协议、无需 ADB 授权的无线播放、蓝牙、Root、虚拟声卡、Android 可见 App、云端账号、联网服务、多台手机同时播放、系统级驱动、编解码器选择、均衡器、频谱、音乐管理、音频输入/麦克风转发。

**首选技术路线：** `C# / .NET 8 WPF`（Windows UI 和调度）+ **改造自 ysbing 的 C++ WASAPI DLL**（采集与流发送）+ **改造自 ysbing 的 Java `app_process` 服务**（接收与 AudioTrack）。参考 HeHang0 的 WPF/NAudio 结构及音量事件逻辑，但不直接合并两套音频链路。

> 重要：源码核查确认的上游接口和行为见 §3。初期以**原项目可编译且可运行**为第一关卡，而不是立即进行 UI 重写。

## 1. 需求与用户操作

### 1.1 核心使用流程

1. 用户启动 `AnPod.exe`，应用在主窗口显示 USB 调试授权状态。
2. 用支持数据传输的 USB 线将 Android 手机连接到 Windows PC（界面标注为「USB 连接」）。
3. 首次连接时，提示用户到安卓系统开启开发者选项、开启 USB 调试、在手机授权弹窗允许该电脑。**不得绕过授权。**
4. AnPod 自动识别授权设备，显示设备型号和状态；只有一台设备时自动预选。
5. 点击“连接”，自动建立本地传输链路并播放系统声音。用户无需在手机上单独开启接收 App。
6. 勾选“自动连接上次设备”后，下次设备上线时尝试自动连接；用户主动断开时，本次会话不应立即自动重连。
7. 关闭窗口默认转到托盘；在托盘中可显示窗口、连接/断开、退出。真正“退出”必须停止采集与服务并清理**AnPod 自己**创建的映射与进程。

### 1.1a 无线模式用户流程（v1.1 新增）

**场景 A：PC 与手机同一局域网。** Windows 和 Android 接入可相互访问的网络；用户在 Android 11+ 的「开发者选项 → 无线调试」中启用无线调试，并选择使用配对码配对设备。Windows 端 AnPod 提供「手机地址 IP:端口 + 配对码」输入入口，调用 `adb pair` 完成首次配对，然后点「搜索设备」自动发现连接端口、或手动输入**连接用** IP:端口执行 `adb connect`。配对端口与连接端口通常不同，不能混用；mDNS 发现是便利手段，手动录入始终可用。

**场景 B：安卓手机连接 Windows 移动热点。** Windows 先在系统设置中开启移动热点，Android 加入此热点；之后的配对、连接、播放流程**与场景 A 相同**。AnPod 只检查网络与给出跳转/指引，首版不实现管理员权限热点配置，也不自动修改系统网络共享设置。Windows 必须具备可工作的热点硬件与系统支持；若 PC 无合适无线网卡，无法承诺此场景可用。

**场景 C：Android 10 及以下的无线连接。** 由于旧 Android 通常没有配对码无线调试，提供可选的「先 USB 授权 → `adb tcpip 5555` → 拔线后 `adb connect IP:5555`」兼容流程，必须展示 TCP/IP 调试的安全风险和关闭方法。默认不自动开启旧式 TCP/IP 调试；如果不接受相关风险，应继续使用 USB。

**重新连接：** USB 线断开或 Wi-Fi 临时掉线时仅在用户已授权、选择自动重连的情况下尝试指数退避重连。设备 IP、无线调试连接端口可能变化；允许重新发现/重新输入，不应无限重复旧地址。不同网络可能需要重新确认无线调试配对与权限。

### 1.2 默认功能决策

| 项目 | 决定 | 备注 |
|---|---|---|
| Windows 支持 | Windows 10/11 x64 | 先 x64，不做 x86/ARM64 |
| Android 支持 | Android 6.0+ 为构建基线，实际按实测兼容表标识 | `minSdk 23` 是上游配置，不等于所有 ROM 均正常 |
| 传输 | USB ADB Reverse + Wi-Fi ADB Reverse | 局域网和 PC 热点共用无线 ADB 通道；不将 Wi-Fi 设备误标 USB |
| 用户授权 | USB 调试授权或 Android 无线调试配对 | Android 11+ 优先使用系统无线调试配对码；旧版可选择 USB 引导 TCP/IP 调试 |
| 音频采集 | Windows 默认输出端点的 WASAPI Loopback | 不自动将 Windows 输出设备切换到手机 |
| 音频 | 48kHz / stereo / signed PCM16 LE 优先 | 兼容回退按协议传递实际采样率，严格校验 |
| 安卓播放 | Java AudioTrack | 无独立用户界面 |
| 自动连接 | 默认开启、只针对上次使用设备 | 首次授权不自动抢占连接 |
| 最大设备数 | 同一时间仅 1 台 | 多台时提供设备选择，不做混音或广播 |
| 自启动 | 默认关闭 | 自用轻量工具不强推开机启动 |
| 后台 | 关闭主窗口到托盘、明确退出 | 任务管理器中运行状态透明 |
| 日志 | 本地滚动日志，保留最近 7 日 | 必须屏蔽 ADB 密钥与不必要设备识别信息 |
| 配置 | JSON，小而清晰 | 默认存放 `%LocalAppData%/AnPod/` |
| 发布 | x64 免安装便携包优先 | EXE + DLL + ADB + server 分文件可接受 |

### 1.3 非功能要求

- 空闲状态不启动音频采集；检测手机时尽量低 CPU、低 I/O。
- 不偷偷启用网络 ADB。`adb pair`、`adb connect` 仅在用户明确选择无线模式/已保存授权设备后调用；`adb tcpip` 仅在用户主动启动旧 Android 兼容引导流程并看到风险提示后调用。
- 服务监听优先限制本机 loopback（见 §5 安全修复）；不开放对外音频接收端口。
- 所有 ADB 命令必须设置**设备序列号**、超时、退出码检查和取消机制；不得直接拼接不可信 shell 字符串。
- 任何一次失败都给用户友好的状态，允许再次连接；退出不留下失控的子进程。
- 不尝试提升管理员权限来绕过 USB 驱动/授权问题。

### 1.4 无线连接的范围约束

- **无线 ≠ 免授权。** 在不安装 Android App 的限制下，手机仍需开启无线调试且 PC 获得 ADB 授权；这不是任意同网手机直接点击就播放的方案。
- **热点 ≠ 另一套音频协议。** Windows 热点只提供 IP 链路，音频仍通过无线 ADB Reverse 传输；热点模式可能遭遇客户端隔离或防火墙限制，失败需明确诊断。
- 仅 1 台 Android 手机处于播放状态；USB/Wi-Fi 是**互斥会话**，不允许音频同时向两种通道发送；切换先有序停止再重连。
- 若用户未开启无线调试/配对，Windows 不应试图绕过 Android 授权或隐藏地修改系统设置。

## 2. 开源代码来源与复用边界

### 2.1 主要项目

**ysbing/AudioShare**  
- 仓库：<https://github.com/ysbing/AudioShare>
- 许可证：LGPL-3.0-or-later（以实际引入版本的 LICENSE 为准）。
- 已确认文件及作用：
  - [`client/native/audio_capture.cpp`](https://github.com/ysbing/AudioShare/blob/main/client/native/audio_capture.cpp)：Windows WASAPI Loopback + PCM16 转换 + TCP Socket 发送。
  - [`client/lib/services/audio_capture.dart`](https://github.com/ysbing/AudioShare/blob/main/client/lib/services/audio_capture.dart)：对 C DLL 的 FFI 调用及导出函数定义。
  - [`client/lib/services/adb_service.dart`](https://github.com/ysbing/AudioShare/blob/main/client/lib/services/adb_service.dart)：设备扫描、属性查询、ADB Reverse、推送服务、`app_process` 启动和停止。
  - [`client/lib/data_source.dart`](https://github.com/ysbing/AudioShare/blob/main/client/lib/data_source.dart)：轮询、连接序列、自动连接与状态处理。
  - [`server/src/main/java/com/ysbing/audioshare/Main.java`](https://github.com/ysbing/AudioShare/blob/main/server/src/main/java/com/ysbing/audioshare/Main.java)：Android LocalSocket + AudioTrack 播放。
  - [`server/build.gradle`](https://github.com/ysbing/AudioShare/blob/main/server/build.gradle)：Android 服务载荷构建（含输出文件复制逻辑）。

**HeHang0/AudioShare**  
- 仓库：<https://github.com/HeHang0/AudioShare>
- 许可证：Apache-2.0（以实际引入版本为准）。
- 已确认 [`windows/AudioManager.cs`](https://github.com/HeHang0/AudioShare/blob/master/windows/AudioManager.cs) 使用 `NAudio.CoreAudioApi`、`WasapiLoopbackCapture`，有端点音量通知和声道处理。它**仅作参考实现**，不要把其 Android APK/协议与 ysbing 服务直接混接；不复制其云音乐、氛围灯、多设备、完整管理界面。

### 2.2 复用原则

1. 建立 `upstream/` 文档记录所采用的仓库 **commit SHA、URL、许可证、修改清单**；不要在提交前固定“main”这种浮动版本。
2. Windows DLL 和 Android 服务按功能原样复制进入 `src/Native`、`src/AndroidServer`，先验证构建；**第一阶段不改协议**。
3. `adb_service.dart` 和 `data_source.dart` 不在运行时使用，而是**按逻辑移植**为 C# 服务，利用已有 ADB 命令设计但修复其异常处理。
4. WPF UI 独立新建，避免 Flutter 依赖；HeHang0 的代码作为备选实现参考，除非确认需要，不复制其 NAudio 模块。
5. LGPL 代码改动应保留版权/许可证说明；如果将来对外分发，准备 LGPL 源码或对应修改源码与重建/替换必要信息，保留 third-party notices。许可证义务由所选版本、链接方式和具体分发方式决定，发布前复核。

### 2.3 必须先确认的技术关口（Spike）

- 是否能在目标 Windows 环境编译并运行 ysbing 的 Windows DLL。
- 当前原版 Android `server` 载荷是否能在目标 Android 系统通过 `app_process` 稳定启动（**不是普通 APK 安装**）。
- 是否能在 USB 连接后正常接收并播放音频，息屏不立即终止。
- DLL 现有 C ABI 能否被 x64 C# `DllImport` 稳定调用（线程回调与调用约定必须确认）。
- 目标设备 Windows 音频默认端点的采样格式与回退方式是否正常。

**阻断规则**：Spike 失败时先修复底层或调整技术方案，禁止以“先做漂亮 UI”绕过风险。

### 2.4 无线复用结论与参考资料

- [ysbing/AudioShare](https://github.com/ysbing/AudioShare)：其 README 声明支持 **USB 以及 ADB over Wi-Fi**；同一个 Android `app_process` 服务、LocalSocket、WASAPI 与 PCM16 数据链路无需另做无线版。现有 `AdbService` 已有识别 IP:端口型序列号的逻辑，但 UI 配对/连接、重连诊断需要新增。
- [HeHang0/AudioShare](https://github.com/HeHang0/AudioShare)：继续用于 Windows UI、设备管理与异常处理参考，不把 Android APK 作为无线方案依赖。
- [Android 官方无线调试文档](https://developer.android.com/tools/adb#connect-to-a-device-over-wi-fi)：Android 11+ 配对码、mDNS 服务发现、`adb pair`/`adb connect` 指引；实现前核对目标 SDK Platform-Tools 行为。
- [Windows 移动热点设置说明（Microsoft）](https://support.microsoft.com/windows/use-your-windows-device-as-a-mobile-hotspot)：热点启用与支持条件；不假定所有 Windows PC 都能开启热点。

## 3. 已确认源码行为与协议

### 3.1 DLL 导出函数（已核查）

来自 `client/native/audio_capture.cpp`：

```c
int  AudioCapture_Initialize(void);
int  AudioCapture_Listen(int port, ConnectCallback callback);
int  AudioCapture_Start(void);
void AudioCapture_Stop(void);
void AudioCapture_Cleanup(void);
int  AudioCapture_IsConnected(char* outBuf, int bufSize);
```

`ConnectCallback` 的形态为 `void (*)(const char* connectCode)`。新 C# 工程必须将委托保持为存活状态，并明确 `CallingConvention`（Windows x64 下 ABI 实际一致，仍建议在头文件中显式规范）。回调在原生线程调用，不允许直接访问 WPF 控件；通过 `Dispatcher` 切到 UI 线程。应提供新的 `AudioCapture.h`，声明导出、回调与错误语义，而不是到处重复手写 P/Invoke 签名。

**注意**：当前 Windows 原生代码的错误查询接口不与 macOS 完全一致（Dart 文件对部分错误函数仅在 macOS 加载）。不要直接假设 Windows 可调用 `AudioCapture_GetLastErrorCode` 等符号；若新增诊断 API 应同时实现和测试。

### 3.2 ADB 隧道与 Android 进程（已核查）

上游连接顺序：先创建本机 TCP 监听 → `adb reverse localabstract:<socket> tcp:<port>` → `adb push server /data/local/tmp/audioshare` → 通过 `adb shell app_process -Djava.class.path=/data/local/tmp/audioshare /data/local/tmp com.ysbing.audioshare.Main socketName=... connectCode=...` 启动 → Android 通过 `LocalSocket` 建立隧道连接 → 回传 connectCode → Windows 开始采集。

这里的 `server` 是 Android 端**可执行 Java 类载荷**，构建脚本目前从 APK 输出复制并重命名；**不意味着需要在 Android 手机上安装 APK**。要按真实原始构建脚本校验载荷格式，不要擅自更换为普通 `.jar` 或 `.dex` 而不经测试。

ADB 进程和 socket 名由 AnPod 独立管理；保留宿主 ADB 服务，不能在退出时无条件 `adb kill-server`（可能影响 Android Studio/其他工具）。

### 3.3 音频协议（已核查）

- Android 建立 socket 后**先回传连接标识**，Windows 接收并触发回调。
- Windows 随后发送 **8 字节**小端头：`uint32 sampleRate` + `uint16 channels` + `uint16 bitsPerSample`。
- 后续为连续原始 PCM 数据，既无文件封装也无压缩。
- 安卓使用 `AudioTrack` `MODE_STREAM`、`ENCODING_PCM_16BIT`；Android 8.0+ 请求低延迟模式，使用 minimum buffer 倍数预缓冲。

**必须补强**：原接收端解析头部后实际使用采样率和声道数；`bitsPerSample` 字段虽存在，却未参与拒绝不兼容格式的判断。新版本须验证 `sampleRate`（例如可支持范围 8000–192000）、`channels`（仅 1/2）、`bitsPerSample == 16`；异常时终止连接并记录日志。不要宣称实现了未适配的编码格式。

### 3.4 原项目已发现的优先修复点

1. `adb_service.dart` 的 `_exec` 主要读取 stdout/stderr 文本，未系统化处理非零退出码和超时；C# 实现必须解决。
2. 原 `removeAllReverse()` 调用 `adb reverse --remove-all`，会删除其他软件的映射；**只能删除 AnPod 自己的映射**：`adb -s SERIAL reverse --remove localabstract:NAME`，并留意 `reverse --remove` 的具体设备/ADB 版本兼容性。
3. 原 Windows TCP `bind` 使用 `INADDR_ANY`；目标改为 loopback `127.0.0.1` 以降低暴露风险，且确认 ADB Reverse 可以正确访问该地址。
4. 原 `connectCode` 握手直接回传设备标识；新版本必须验证与当前目标 session 一致，再启动捕获。为了不破坏首版协议，至少严格校验并限制长度；后续可升级成随机令牌握手（需要 Windows、Android 两端同步修改）。
5. 原生 `SendAll` 面对发送失败、慢消费者、连接突然断开时，需要明确短写、错误和积压策略；不可在热路径上无限阻塞。
6. `AudioCapture_Initialize` 对非 Float32/PCM16 的格式回退不能简单地将所有数据按 float32 解释；需检查 `WAVEFORMATEX/WAVEFORMATEXTENSIBLE` 的 subtype、位深、有效位数及声道，必要时使用格式转换器；不要在格式不支持时播放噪声。
7. 原生代码使用 `Sleep(1)` + `timeBeginPeriod(1)` 轮询；首版维持可行实现，测 CPU 与稳定性，确认资源释放时总调用 `timeEndPeriod(1)`。
8. Windows 默认音频端点改变、系统待机恢复、音频采集句柄失效应能识别并允许重建采集链路。
9. `app_process` 关闭时不要仅依赖 Windows 侧 shell 进程 `kill` 就认定 Android 服务已停止；结合 Android 端连接关闭/退出、会话标识或进程清理机制实测。

### 3.5 新增无线传输控制命令（计划接口，非原仓库既有接口）

```powershell
adb pair <phone-ip>:<pair-port> <pair-code>    # Android 11+，仅由显式用户操作触发
adb connect <phone-ip>:<connect-port>          # 连接端口不等于配对端口
adb devices -l                                 # 确认授权后的 Wi-Fi 设备序列号
adb -s <ip>:<connect-port> reverse localabstract:<socket> tcp:<port>
adb -s <ip>:<connect-port> push server /data/local/tmp/audioshare
# 随后通过 -s 指定设备的 adb shell app_process 启动原 Android Java 服务
```

所有外部命令均通过 `ArgumentList` 传参、设置超时和检查 ExitCode；禁止把用户输入直接拼为 shell。只有匹配本次已选择的设备且校验握手成功，才能开始音频采集。主动断开无线会话时，对自己创建的 reverse 映射执行定点移除；必要时 `adb disconnect <endpoint>`，**不可全局 `adb disconnect`**。

## 4. 推荐工程结构

```text
AnPod/
├── README.md
├── plan.md
├── LICENSES/
│   ├── LGPL-3.0-or-later.txt
│   ├── Apache-2.0.txt             # 仅在引入 HeHang0 代码后必需
│   └── ADB-NOTICE.txt             # 按 Android platform-tools 分发条款核实
├── THIRD_PARTY_NOTICES.md
├── upstream/
│   ├── SOURCE_VERSIONS.md         # 固定仓库 commit、引用路径、变更
│   └── PATCHES.md
├── src/
│   ├── AnPod.Desktop/
│   │   ├── App.xaml
│   │   ├── MainWindow.xaml
│   │   ├── ViewModels/MainViewModel.cs
│   │   ├── Models/{DeviceInfo,ConnectionState,AppSettings}.cs
│   │   ├── Services/
│   │   │   ├── AdbRunner.cs
│   │   │   ├── DeviceDiscoveryService.cs
│   │   │   ├── ConnectionManager.cs
│   │   │   ├── NativeAudioCapture.cs
│   │   │   ├── TrayService.cs
│   │   │   ├── SettingsService.cs
│   │   │   └── FileLogger.cs
│   │   └── Assets/
│   ├── AnPod.Native/
│   │   ├── audio_capture.cpp
│   │   ├── audio_capture.h
│   │   └── CMakeLists.txt
│   └── AnPod.AndroidServer/
│       ├── build.gradle
│       └── src/main/java/com/ysbing/audioshare/Main.java
├── tests/
│   ├── AnPod.Core.Tests/
│   └── manual-checklist.md
└── packaging/
    ├── publish.ps1
    └── THIRD_PARTY_NOTICE_TEMPLATE.md
```

除明确必要的文件外不要过度拆分模块；首版只创建一个 WPF 桌面项目和少量 service 类，不引入微服务、消息总线、数据库或复杂 DI 容器。

## 5. 模块设计与契约

### 5.1 `AdbRunner` — ADB 命令封装

输入：ADB 参数列表、目标序列号、超时时间、取消令牌。输出：`exitCode`、`stdout`、`stderr`、耗时及分类错误。使用 `ProcessStartInfo.ArgumentList`，并行读取 stdout/stderr 防止死锁；隐藏命令行窗口。取消或超时时仅终止**本次命令进程树**，而不是全局 ADB 服务。打包的 adb 三件套为 `adb.exe`、`AdbWinApi.dll`、`AdbWinUsbApi.dll`；实际分发需遵守平台工具许可。

建议默认超时（可按实际再调）：`adb devices` 5s；查询属性 5s；建立 reverse 5s；push 15s；app_process 启动后等待握手 10s。ADB 长生命周期的 `shell app_process` 不套用短时命令超时，而应作为 `AndroidProcessSession` 独立管理并持续消费输出。

命令必须带 `-s SERIAL`（除全局 `adb devices -l`）；对于 `offline`、`unauthorized`、`device`、`no device` 分开处理。ADB 序列号只来自 `adb devices -l` 结构化解析，不接受 UI 任意注入参数。

### 5.2 `DeviceDiscoveryService`

- **无空闲轮询。** 设备列表只在用户点「刷新」或会话建立/恢复时扫描一次；空闲状态不发任何 ADB 命令（plan §1.3：空闲低 CPU）。`adb push` 只在连接会话内发生。
- 识别 USB 与 Wi-Fi 两种连接方式：从 `adb devices -l` 的 `usb:` 标签和 `sys.usb.state` 正向确认 USB；从 `IP:PORT` 序列号确认 Wi-Fi；无法判断时标记「连接方式待确认」。两种方式的设备均可连接，也不自动推断「型号相同就是同一台」。
- 用户插拔手机时不会自动更新列表；点「刷新」重新扫描。如需 ADB `track-devices` 可后续评估，首版不实现。
- 多个设备只允许连接一个，用户选择后持久保存设备标识（日志中隐藏完整序列号）。

### 5.3 `ConnectionManager` — 唯一状态机控制器

```text
NoDevice ─USB出现─> Unauthorized / Ready
Unauthorized ─用户授权─> Ready
Ready ─点击连接/允许自动连接─> Connecting
Connecting ─握手通过─> Streaming
Connecting ─超时或失败─> Error
Streaming ─用户断开─> Ready
Streaming ─USB断开/音频错误─> Reconnecting 或 NoDevice
Reconnecting ─重连成功─> Streaming
Reconnecting ─重试上限─> Error
Error ─重试─> Connecting
```

状态变化必须序列化：使用 `SemaphoreSlim` 或单一异步命令队列防止重复连接、断开与轮询回调交叉执行；所有任务携带 session generation ID，老的回调不能污染新连接状态。

**连接步骤（均须可取消、可回滚）：**

1. 确认设备是已授权 USB 设备，并只允许选定 SERIAL。
2. 预先清理 AnPod **上一会话**遗留的映射（不可 `--remove-all`）。
3. 加载 DLL，初始化 WASAPI；若无默认音频输出设备，明确告知用户。
4. 分配本机空闲 loopback TCP 端口（只在当前连接生命周期持有）。
5. DLL 启动监听；必须确认监听成功后才继续。
6. 创建本机端口到 `localabstract:anpod_<session>` 的 ADB Reverse。
7. `adb push` 已构建的 Android 载荷（可通过哈希缓存避免每次重复推送，但首版可直接 push）。
8. 启动 `app_process`，维持进程与 stdout/stderr 读取。
9. 在 10 秒内收到 Android 握手，校验会话和目标设备。
10. DLL 开始采集，状态设为 `Streaming`。

**断开/失败统一 Cleanup：** 停止采集 → 关闭监听/Socket → 请求 Android 端退出或关闭 ADB shell → 只移除本会话 reverse → 释放资源 → 状态更新。清理操作应容错、幂等、逐步执行，即使某一步失败也继续。

**自动重连：** USB 恢复后重新发现、校验授权并重新建立整个会话。采用有上限的退避（如 1s、2s、5s，最多 3 次），明确提醒用户点击重新尝试；“用户主动断开”需阻止立即重连。

### 5.4 `NativeAudioCapture` — C# ⇄ DLL 互操作

- 使用 `DllImport` / source-generated `LibraryImport`（根据工程需要二选一）绑定 §3.1 的固定导出接口。
- 使用 `SafeHandle` 或专门的封装类确保 Dispose；防止在原生回调尚未结束时释放托管委托。
- 回调内容不能携带裸指针跨线程使用；第一时间拷贝 connectCode，验证上限后调度到业务线程。
- 新增高层 `Initialize`、`Listen`、`Start`、`Stop`、`Cleanup` 方法；不要在 ViewModel 中直接操作原生接口。
- **建议第一版不替换 C++ 为 NAudio**，只有经测试发现兼容性/可维护性问题时再比较。

### 5.5 Windows C++ 采集和发送

- 复用 WASAPI Loopback、混音格式处理、双声道 PCM16、TCP 发送。
- 网络监听仅 `127.0.0.1`，一次最多服务一个 Android 连接。
- 监听回调要验证连接 token、长度和生命周期。
- 发送队列应有上限；如果接收端明显落后，可通过断开重连/受控丢弃过期 PCM 来限制无限积压，不能让声音持续越播越晚。
- 记录单次会话最基本的性能指标：音频格式、连接耗时、socket 错误、采集异常，不输出音频内容。
- 原生模块务必正确处理 `CoInitializeEx` / `CoUninitialize` 所属线程与 COM 对象释放；在实际确认语义后修补。

### 5.6 Android Java 播放端

- 不要求用户安装 APK；通过 ADB push + `app_process` 启动无 UI Java 服务。
- `LocalSocket` 连接指定 abstract socket 并发送握手；解析 8 字节格式头。
- 对头部进行完整字段校验；仅支持 PCM16，声道数 1/2，合理采样率。
- `AudioTrack` 用 `MODE_STREAM`；先预缓冲再调用 `play()`；所有异常必须 `stop`/`release`。
- 建议记录可解析的启动、播放、停止信息到标准错误，PC 端将其映射到本地日志；不需要手机端 UI 或长期驻留服务。
- 如果 Java 服务遇到厂商 ROM 禁止音频输出或 `app_process` 限制，应明确报错，不加入 Root 绕过方案。

### 5.7 主界面和托盘

主窗口保持 **280–420px 宽的单屏简洁布局**：

```text
AnPod                          [设置]
────────────────────────────────────
       [手机图标]
       <设备型号>
       ● 已连接 USB / 等待授权 / 正在播放

          [开始播放 / 断开连接]

☑ 自动连接上次设备
☑ 关闭窗口时最小化到托盘

[查看日志]                    v1.0
```

多设备情况下在设备名处展开下拉列表；无设备时显示连接引导；授权中显示明确指令。按钮根据状态禁用或显示进度；错误消息简短但可查看详情。界面视觉风格遵循 Windows 11 原生简洁设计，不引入 WebView、Electron、Flutter、动画框架。可先使用纯 WPF 样式，后续再判断是否有必要引入外部 Fluent UI 包。

托盘菜单：`显示 AnPod` / `连接或断开` / `退出`。状态颜色仅作为辅助，需同时使用文字/Tooltip。主窗口关闭默认隐藏，托盘“退出”执行完整清理。支持重复运行保护（mutex），第二实例仅唤起已有窗口。

### 5.8 设置与日志

设置 JSON 示例：

```json
{
  "schemaVersion": 1,
  "autoConnectLastDevice": true,
  "lastDeviceId": null,
  "minimizeToTray": true,
  "launchAtLogin": false
}
```

配置更改使用原子写入（临时文件再替换），坏配置用默认值恢复。日志采用每日/大小滚动，默认 INFO，近期保留 7 日且设置最大总占用；严禁保存音频 PCM 流、ADB 私钥、授权令牌。提供“打开日志文件夹”和“复制诊断信息”即可，不增加专用复杂日志界面。

### 5.9 `WirelessAdbService` — 无线配对与发现（v1.1 新增）

- 负责：配对码输入/验证、`adb pair`、`adb connect`、mDNS 发现、无线端点保存、重连、网络变化响应。
- 统一使用现有 `AdbRunner`；不创建第二套 Windows 音频采集链路，不修改 Android `Main.java` 现有 8 字节格式头。
- 配对码仅在内存使用，不写入设置、命令日志或崩溃报告；保存设备标识与最后成功连接端点即可。ADB 密钥交由 Platform-Tools 默认机制管理。
- 用户手动断开后禁止自动重连。
- **mDNS 发现是两步探测。** `adb mdns services` 在 ADB 服务的 mDNS 守护进程**僵死时仍然 0.05 s 返回空列表、退出码 0**，与健康时无法区分。因此必须先跑 `adb mdns check`：健康守护进程 0.05 s（热）/2.14 s（冷启动）应答；僵死时约 11.25 s 后报告 `ERROR: mdns daemon unavailable`。探测超时取 **5 s**——大于冷启动、远小于僵死，超时即视为不可用。不可用时不报「没有搜索到设备」，而明确提示「搜索组件未运行，请手动填地址」。
- 手机广播 `_adb-tls-pairing._tcp`（配对端口）和 `_adb-tls-connect._tcp`（连接端口）两条 mDNS 服务。搜索成功后：只有一台手机时自动填入连接地址；多台时只认刚配对过的 IP；仍无法区分则列出地址让用户选，绝不猜测。
- 搜索过程中用户手动修改地址栏时，结果**不覆盖**已输入内容。
- 热点仅作为 Wi-Fi 类型的网络来源标记：通过网络接口和地址信息做建议性检测；无法可靠识别时以「Wi-Fi」显示，不伪造热点类型。
- 所有网络断线重连操作必须串行化，有取消令牌、最大重试间隔和应用退出停止保证。

### 5.10 连接模式及 UI 变更

- 主窗口显示「连接方式：USB 连接 / Wi-Fi 无线」，默认优先已授权 USB；用户可手动切换。
- **设备列表按模式过滤。** 同一台手机可同时以 USB 和 Wi-Fi 两种 transport 出现在 `adb devices` 中（两个不同 serial）。USB 模式只列 USB 设备，Wi-Fi 模式只列无线设备；`DeviceInfo.MatchesTransport(TransportKind)` 是过滤谓词，未知设备两边都显示（不隐藏，否则列表变空反而误导）。列表、选中项、自动连接均彼此独立：切模式后不会保留另一模式的选中项，也不会用另一模式的序列号自动连接。
- Wi-Fi 区包含「手机地址 IP:端口」「配对码」「配对」「连接」「搜索设备」「连接帮助」。配对端口与连接端口是两个不同数字，UI 明确标注；搜索设备自动读取手机广播的连接端口填入地址栏。
- 配对成功不代表音频已连通，UI 分别显示 ADB 连接、安卓服务、音频播放状态。
- 首次热点连接由用户先在 Windows 设置开启移动热点，再让手机加入；主界面提供引导文字和系统设置入口（使用官方受支持的设置 URI 时做容错）。
- 旧 Android 兼容入口放到「高级设置」，仅在明确确认后启用 `adb tcpip`，并提供关闭无线调试指导。
- 只有连接模式相关字段新增到 `settings.json`，例如 `preferredTransport`、`autoReconnectWifi`、`lastWirelessEndpoint`；不保存配对码、敏感输出或 Wi-Fi 密码。
- 输入框（地址、配对码、重连复选框）仅在**会话进行中**锁定；搜索/配对/连接操作进行中只禁用按钮，不锁定输入框——因为用户常在上一操作还没返回时就开始输入配对码。

## 6. 关键异常场景与处理表

| 场景 | 程序必须怎样做 | 用户看到什么 |
|---|---|---|
| 未连接手机 | 不启动采集；保持检测 | 请使用 USB 连接（数据线）Android 手机 |
| USB 只有充电线/驱动缺失 | 不猜测具体问题；提供排查指引 | 未发现 ADB 设备，请检查数据线/驱动/调试设置 |
| `unauthorized` | 等待用户手机确认 | 请在手机上允许此电脑进行 USB 调试 |
| `offline` | 停止旧会话，有限重试 | 设备暂时离线，正在重新连接 |
| 多个设备 | 不自动任选其一 | 请选择需要作为扬声器的手机 |
| 推送服务失败 | 清理自己映射、停止监听 | 无法启动手机音频服务；可查看日志 |
| `app_process` 启动失败 | 停止并记录 stderr | 手机系统不支持当前启动方式或拒绝启动 |
| 握手超时/错误 | 不开始采集，清理、有限重试 | 连接超时，请重新连接 |
| Windows 无可用音频端点 | 退出连接流程 | 未发现可采集的 Windows 输出设备 |
| 音频格式不支持 | 返回明确错误而不是错误播放 | Windows 音频格式暂不支持 |
| USB 播放中拔线 | 停止采集，清理，等待恢复 | 连接已断开，等待重新插入 |
| 手机息屏音频中断 | 按策略重连并记录 | 手机音频被系统中断 |
| Windows 睡眠/唤醒 | 唤醒后重新识别设备与端点 | 正在恢复连接 |
| ADB 被其他程序使用 | 不杀全局 server，协调端口 | ADB 服务不可用，请检查其他工具 |
| 电脑端口被占用 | 重新分配可用端口 | 连接中，必要时显示错误 |
| 快速多次点连接 | 同一时刻只执行一条连接操作 | 按钮进入忙碌状态 |
| mDNS 搜索组件僵死 | 先 `mdns check` 探测，不可用则引导手动填地址 | 系统的 mDNS 搜索当前不可用，请手动填地址 |
| 手机无线调试自动关闭 | 属于 Android 设计行为，应用无法阻止；提示用户重新开启 | 搜索不到设备，请确认手机「无线调试」已开启 |
| 主窗口关闭 | 到托盘，不影响现有音频 | 托盘常驻 |
| 真正退出 | 停止音频、映射、子进程 | 完全退出 |

## 7. 资源、安全与兼容性检查

### 7.1 安全边界

- 应用无需管理员权限（安装 USB 驱动可能单独需要）。
- 仅使用已授权 USB ADB，不改变手机调试设置、不开启网络 ADB、不获取 Root。
- 本机 TCP 只监听 loopback，不允许意外暴露到局域网。
- ADB Reverse 只撤销 AnPod 自己创建的映射，不执行 `reverse --remove-all`。
- 避免让任何其他本地进程伪造 Android 握手，最低要求会话标识一致；优选后续使用随机短期令牌（两端同时修改）。
- 不运行来自网络下载的 Android 服务二进制；只加载程序自带、版本可确认的载荷。

### 7.2 平台兼容

- 在 Windows 10 与 Windows 11 分别测试，无预装开发工具的机器也应启动。
- USB 驱动依厂商：部分安卓手机在 Windows 下需要对应 OEM ADB 驱动；文档应清楚列出排查方法，不打包未经授权的厂商驱动。
- 测试至少两类 Android 系统/厂商；目标设备若只有一台，也要记录限制，不虚称全面兼容。
- Android ROM 的音频后台限制、USB 调试权限、`app_process` 行为不可通过 Windows 端保证，需实测。

### 7.3 体积与性能目标（建议验收阈值，非现成实测）

| 指标 | 初版目标/原则 |
|---|---|
| 可操作播放 | USB 授权完成后，一键开始 |
| 连续播放 | 30 分钟无崩溃或不断累积延迟；最终扩大到 2 小时回归 |
| USB 插拔 | 20 次重复插拔，无卡死或遗留会话 |
| 自动重连 | 在已授权且 ADB 恢复后有限次自动恢复 |
| 音画同步 | 争取端到端延迟 <150ms；初期实际记录并优先稳定 |
| CPU 空闲 | 以测试机基线比较，尽量 <1% 的长期平均值 |
| RAM / 包体 | 记录实际值，不为追求极致单文件增加复杂度 |
| 质量 | 不出现持续爆音/乱码格式音频；偶发断流要能明确恢复 |

*延迟测量建议：PC 上生成突发脉冲，在手机扬声器近场录音或使用有线测量回路，记录发送、安卓接收与声学输出的差异。不要把 TCP 发送延迟当成端到端音频延迟。*

### 7.4 无线安全与音频延迟边界

- Android 无线调试只应在用户信任的 LAN/热点使用；公共 Wi-Fi 上提示不要启用。默认绝不主动开放对外可访问的 Windows 音频端口；音频 TCP 监听应限制 `127.0.0.1`，由 ADB Reverse 转发。
- Windows 防火墙、路由器/AP 客户端隔离和移动热点功能限制可能阻止 ADB 连接；不得通过自动关闭防火墙解决。
- 16-bit/48kHz/stereo PCM 约 1.536 Mbps 原始码率，可用于稳定 Wi-Fi 链路；弱网与丢包造成的卡顿需测试。首版不加入 Opus，以避免扩大实现范围。
- Wi-Fi 模式端到端延迟可能高于 USB，不能保证固定 <150ms；测试需要标注网络环境、5GHz/2.4GHz 和设备型号，记录实际测量结果。

## 8. 阶段计划与 AI 开发执行指令

### P0 — 上游基线与技术可行性（必须先完成）

**开发任务**
1. 克隆两仓库并将 commit SHA、LICENSE 固定到 `upstream/SOURCE_VERSIONS.md`。
2. 阅读 ysbing `audio_capture.cpp` 全部实现、对应 `audio_capture.dart`、ADB 逻辑和 Java 播放服务，确认 DLL C ABI/资源释放行为。
3. 编译 ysbing Windows DLL 和 Android 载荷；验证上游程序的端到端 USB 声音播放。
4. 记录测试：USB 授权、音量、44.1k/48k 设备格式、息屏、短断线。

**完成条件**：Windows 音频能够通过 USB 由 Android 扬声器播放；如失败，记录具体失败层、日志及修复方案，不开始 P2 UI。

### P1 — Windows 最小连接核心

**开发任务**
1. 建立 .NET 8 WPF 工程；先用简单窗口或控制台显示日志。
2. 实现 `AdbRunner`、`DeviceDiscoveryService`、`NativeAudioCapture`。
3. 构建并装载 DLL；实现从 PC 连接、推送、启动、握手、采集、停止完整流程。
4. 只连接一台手动指定设备，暂不做自动重连和托盘。
5. 修复 ADB 非零退出码、超时、选定设备、映射清理和端口绑定限制。

**完成条件**：不用 Flutter 就能连接、播放、断开，且所有资源能释放。

### P2 — 稳定会话状态机与自动连接

**开发任务**
1. 实现 `ConnectionManager` 状态机、取消机制、会话代数。
2. 实现授权状态、USB 设备识别、多设备选择、自动连接上次设备。
3. 处理插拔、超时、ADB offline、Android 进程退出、Windows 睡眠与音频端点变化。
4. 实施有界重连策略、用户主动断开抑制自动重连。

**完成条件**：20 次插拔/反复连接无 UI 卡死，授权恢复后可重连，失败可诊断。

### P3 — 极简 WPF UI、托盘、配置

**开发任务**
1. 单窗口状态 + 手机名 + 主操作按钮 + 两项设置。
2. 托盘状态、托盘菜单与明确退出流程；防重复实例。
3. JSON 配置、轻量日志与日志路径入口。
4. UI 不阻塞后台进程，错误只显示针对用户的简短引导。

**完成条件**：无需命令行即可完成连接、断开、重连和退出。

### P4 — 音频可靠性与兼容性回归

**开发任务**
1. 完整审查 WAV 格式 fallback（Float32/PCM16/其他情况），不支持的格式必须显式拒绝。
2. 评估 socket 限时发送和消费者落后时的积压；必要时调整缓冲，但保持协议简洁。
3. 测试 30 分钟、2 小时连续播放，系统负载变化、屏幕熄灭、充电状态、Windows 睡眠唤醒。
4. 检查原生线程、COM、AudioTrack、ADB 子进程与 DLL 多次初始化/销毁资源泄漏。

**完成条件**：填写测试矩阵，所有已知问题有明确状态；没有导致明显失败的 P0/P1 级别 bug。

### P5 — 便携发布和使用文档

**开发任务**
1. 使用脚本编译 C++、WPF、Android 服务并复制运行依赖。
2. 生成 Windows x64 ZIP 便携包，提供 `AnPod.exe`、原生 DLL、ADB、Android 服务等。
3. 编写 README：首次启用 USB 调试、授权、已连接但无声音等常见问题。
4. 记录 upstream 许可证及修改、第三方二进制来源和版本；发布前复核 LGPL 再分发义务。
5. 不使用未签名驱动或自动安装 USB 驱动作为首版前提。

**完成条件**：一台未安装 Flutter、Visual Studio、Android SDK 的 Windows 设备，解压即可启动；连接设备所需 ADB 驱动支持有清晰说明。

### P6 — v1.1 Wi-Fi / Windows 热点扩展（在 P0–P5 USB 稳定后实施）

**P6.1 无线配对实验：** Android 11+ 与 PC 同 LAN，通过官方无线调试配对码使用 `adb pair`，核对配对端口和连接端口差异；确认 `adb connect` 后复用 `adb reverse`、`app_process` 和 AudioCapture.dll，先用命令行跑通。

**P6.2 服务抽象：** 增加 `WirelessAdbService` 与 `TransportKind`；`ConnectionManager` 管理 USB/Wi-Fi 两条控制路径，但只能有一条活跃音频会话。ADB 命令结果封装沿用既有实现。

**P6.3 交互：** 增加 Wi-Fi 配对页、手动 IP:端口兜底和诊断文案，显示 USB / 局域网 / 热点标签（能可靠判断时才细分）。不自动管理 Windows 热点。

**P6.4 可靠性：** 测试网络切换、IP/端口变化、无线调试关停、电脑睡眠唤醒、USB/Wi-Fi 同时可用与手动切换；验证定点断开和资源清理。

**P6.5 旧 Android 可选兼容：** 经用户显式许可，验证 USB 引导 `adb tcpip` 的连接，提示网络调试暴露风险；无法验证时不纳入默认支持清单。

**P6 验收：** 同 LAN、Windows 热点各至少一台 Android 11+ 实机正常播放 30 分钟；重复断开/连接 10 次不死锁；无 USB 时完成无线配对+播放；USB/Wi-Fi 切换无僵尸进程；Wi-Fi 断开后不残留本应用的音频连接；错误有可操作提示。

### AI 开发必须遵守的工作纪律

- **每个阶段要能运行、能测试、能回滚**；不允许一次性重构全部模块。
- 先修复正确性问题，再做功能扩展；不添加未经需求批准的功能。
- 对未核实的 Android 服务载荷格式、ADB 生命周期、DllImport ABI 不能靠猜测实现，应依据实际构建及日志验证。
- 不得把项目里的 Java 载荷改成普通 Android App，不引入安装 APK 步骤。
- 对已修改的 LGPL 源代码标注来源、修改，并保存对应源码。
- 每阶段输出变更文件列表、关键实现说明、测试命令、已通过/未通过的验收点及剩余风险。
- 生成的代码必须有合理错误处理与注释；禁止无意义抽象层、无必要第三方依赖和未使用功能。

## 9. 测试矩阵

| 测试编号 | 前提/动作 | 预期结果 | 优先级 |
|---|---|---|---|
| T01 | 手机未接入，启动 AnPod | UI 显示未发现设备、CPU 低 | P0 |
| T02 | 手机 USB 调试未授权 | 显示授权指导，不尝试采集 | P0 |
| T03 | 单台授权 USB 手机接入 | 可识别并连接，扬声器播放 | P0 |
| T04 | 两台已授权手机 | 设备可选择，只连接选中一台 | P1 |
| T05 | 电脑播放 48kHz 视频/音乐 | 正常立体声，无明显爆音 | P0 |
| T06 | Windows 默认音频端点 44.1kHz | 无速率漂移和周期性断音 | P1 |
| T07 | 播放中拔 USB | 停止采集并显示断线 | P0 |
| T08 | 自动连接开启后重插 | 识别原设备并恢复连接 | P1 |
| T09 | 用户主动断开 | 不被自动连接立即重新连接 | P0 |
| T10 | 连续插拔 20 次 | 无死锁/残留资源 | P1 |
| T11 | 手机熄屏 30 分钟 | 无中断，或提示兼容性限制 | P1 |
| T12 | Windows 睡眠后唤醒 | 能重新识别及手动/自动恢复 | P1 |
| T13 | 连接前占用预定端口 | 选其他端口，不异常退出 | P1 |
| T14 | ADB 当前设备 offline | 不误标为已连接，有限重试 | P0 |
| T15 | 其他工具存在 Reverse 映射 | AnPod 退出不删除其映射 | P0 |
| T16 | 关闭窗口 | 留在托盘并按设置继续音频 | P1 |
| T17 | 托盘点击退出 | 停止采集、关闭本会话，无遗留进程 | P0 |
| T18 | 无 Windows 音频输出端点 | 给明确提示，不崩溃 | P1 |
| T19 | Android 服务立即崩溃 | 超时/失败转 Error，可再次连接 | P0 |
| T20 | 空闲 30 分钟与持续播放 2 小时 | 记录 CPU/RAM、延迟、断流统计 | P1 |

### 9.1 v1.1 无线专项验收矩阵（W01–W16）

| 编号 | 场景 | 通过标准 |
|---|---|---|
| W01 | Android 11+ 同 LAN 配对码配对 | 首次无线配对成功，配对码不落盘 |
| W02 | 配对端口≠连接端口 | UI 引导正确，实际连接端口使用正确 |
| W03 | mDNS 发现失败 | 能手动填入设备 IP:连接端口完成连接 |
| W04 | Windows 热点 | 手机加入热点后可无线 ADB 连接并播放 |
| W05 | Wi-Fi 播放 30 分钟 | 无持续累积延迟、长时间静音或崩溃 |
| W06 | WLAN 断开/恢复 | 状态准确，可控重连，主动断开不重连 |
| W07 | IP/端口变化 | 不无限重试旧地址，允许重新发现或输入 |
| W08 | USB 与 Wi-Fi 同时识别 | 单实例会话；列表按模式过滤，同一手机不会两边都出现；不双重推送、不错误选设备 |
| W09 | USB ↔ Wi-Fi 切换 | 有序清理旧通道、可重新播放；切模式后列表即时重刷，不保留另一模式的选中项 |
| W10 | Android 无线调试关闭 | 正确提示需重新开启或配对，不提示 USB 故障；搜索失败消息优先提示「请确认无线调试已开启」 |
| W11 | 非可信/隔离 LAN | 提示网络限制，不绕过安全设置 |
| W12 | 旧 Android USB 引导 TCP/IP（可选） | 仅主动触发，有安全说明和停止方式 |
| W13 | 同 LAN 内开启无线调试后点「搜索设备」 | 读出两条 mDNS 服务，连接端口自动填入地址栏；多台手机时只认刚配对过的 IP，认不出则列出地址 |
| W14 | ADB 服务 mDNS 组件僵死时点「搜索设备」 | 明确提示「搜索不可用，请手动填地址」，不报成「没有搜索到设备」 |
| W15 | 切换 USB / Wi-Fi 模式看设备列表 | 两个模式列表互相独立；USB 模式不出现 Wi-Fi 设备，Wi-Fi 模式不出现 USB 设备；未知设备两边都显示 |
| W16 | 地址栏为空时看配对码输入框 | 配对码框可点击、可输入；只在会话进行中才锁定，不因地址为空而禁用 |

## 10. 明确不做与未来扩展

**不纳入 v1.1：** 无 ADB 授权的任意 Wi-Fi 直连、Android 可见 App、无需调试的手机端接收程序、Wi-Fi UDP/Opus 独立协议、Windows 设备级 USB 音响模拟、低延迟驱动、Audio Endpoint 创建、麦克风转发、蓝牙、多手机同步、音频文件录制/上传、账号/自动更新/遥测。

无线模式按 P6 在 USB 稳定后开发。未来版本才考虑：更好的端到端延迟测试、更优雅的 WASAPI 事件驱动、自定义采样率、音量控制及错误诊断导出。功能是否进入下一版本由实际需求决定，不默认实施。

## 11. 项目风险与止损决策

| 风险 | 等级 | 应对措施 |
|---|---|---|
| Android `app_process` 在目标手机/ROM 上不能稳定播放 | 高 | P0 优先验证；若不支持，要明确“无 Android App”约束与兼容性不可同时满足的情况，不能擅自改变用户要求 |
| 原上游 Windows DLL 回退格式错误或线程管理缺陷 | 中高 | P0/P4 重点修复；必要时把 Windows 采集替换为成熟的 C# NAudio 实现，但先通过对照测试 |
| ADB 与其他软件冲突、权限丢失 | 中 | 精确操作本设备/本映射、重试有上限、清楚提示 |
| 息屏中断、系统音频策略冲突 | 中高 | Android 真机测试、记录 ROM 差异，不引入 Root |
| LGPL 再分发要求 | 中 | 保留 notices、修改记录、可重建源码和版本；发布前复核 |
| 要求“真正显示为 Windows 输出设备” | 范围外 | 说明这是另一种 USB Gadget/驱动项目，不在 AnPod 范围内 |

## 12. 项目完成定义（Definition of Done）

AnPod v1.1 视为完成必须同时满足以下 USB 基础要求，以及新增的无线验收要求：

1. 只有 Windows GUI，用户无须安装或打开 Android 图形 App。
2. Windows 10/11 x64 便携包可运行；目标手机 USB 授权后可一键播放系统音频。
3. 用户可以暂停本工具传输/断开、恢复，主动断开不会被自动重连覆盖。
4. Windows 音频与安卓接收端发生异常时 UI 不死锁，并可重新连接。
5. USB 断线、重新插线、授权丢失、端口冲突和 app_process 启动失败都有明确处理。
6. 已完成 T01–T20 测试矩阵中 P0 全通过；P1 未通过项目需记录为已知限制且不影响核心路径。
7. 关闭到托盘、退出彻底清理 AnPod 自己创建的进程和映射。
8. 保留原项目和依赖的许可证、源码来源、修改记录；不隐藏依赖的安全授权步骤。
9. 代码结构和文件数量保持简洁，无未使用的跨平台、独立网络音频协议模块。
10. W01–W16 无线专项测试完成，Android 11+ 可在同 LAN 和 PC 热点环境下无需 USB 完成配对和播放；W12 按可选兼容范围记录。

---

## 附录 A：复用检查清单（给编程 AI 的顺序）

- [ ] 固定 ysbing/AudioShare、HeHang0/AudioShare 对应 commit SHA 和许可证。
- [ ] 检查 Windows DLL `audio_capture.cpp` 全文件的导出和资源清理。
- [ ] 检查 `audio_capture.dart` 签名以及 native callback 的线程语义。
- [ ] 检查 `server/build.gradle` 产生 `server` 的真实字节格式。
- [ ] Android `Main.java` 保留 8 字节头协议和基本播放路径。
- [ ] 先验证上游 USB 传输，再替换 WPF UI。
- [ ] C# `AdbRunner` 替代 Dart ADB 层并增加错误检查。
- [ ] 仅移除本 App 自己的 ADB Reverse，绝不用 `--remove-all`。
- [ ] 保留单设备模式，测试多设备错误选择情形。
- [ ] 完成验收矩阵后再制作 x64 便携包。

## 附录 B：构建建议

- **Windows Desktop**：Visual Studio 2022 / .NET 8 SDK，WPF，发布 `win-x64`。
- **Windows Native**：MSVC / CMake 编译 x64 DLL，确保与 WPF 程序同位数。
- **Android Server**：按照上游 Gradle/Android SDK 工具链构建 `server` 载荷，并保留可复现构建过程。
- **ADB**：使用可信 Android SDK Platform-Tools，记录确切版本及分发条款。
- **CI**：首版只需最基本本地 PowerShell 构建脚本，不必先接入 GitHub Actions。

**开发启动指令：** “严格按照 `plan.md` 的 P0 → P5 阶段建立 USB 基线，随后实施 P6 无线扩展。不要跳过上游运行基线和 Android `app_process` 兼容性验证。每阶段先提交可运行的最小变更，报告测试结果，仅按 P6 增加受控 Wi-Fi ADB 和 Windows 热点支持，不得增加 Android 可见 App、UDP 音频协议或其他扩展功能。”


## 附录 C：v1.1 相对于 v1.0 的变更摘要

- 新增：LAN Wi-Fi、Windows 热点、Android 11+ 无线调试配对码流程。
- 扩展：`AdbRunner` 支持 `adb pair/connect/disconnect`；增加 `WirelessAdbService`、传输模式切换与诊断。
- **mDNS 发现是两步探测**（`mdns check` 先证健康，`mdns services` 后读列表），探测超时 5 s；不可用时不报成「没设备」而是「搜索不可用」。
- **设备列表按模式过滤**：`DeviceInfo.MatchesTransport(TransportKind)` 让 USB/Wi-Fi 两个模式的列表与选中项彼此独立；自动连接只匹配当前模式的设备。
- **输入框可用性**：地址框、配对码框、重连复选框仅在「会话进行中」锁定，不因地址为空而禁用配对码输入。
- 复用：WASAPI 采集 DLL、ADB Reverse、Android `app_process` + AudioTrack、8 字节格式头、PCM16。
- 可选：旧版 Android USB 引导 TCP/IP 调试，不自动开启。
- 保持：仅 Windows GUI、无需 Android 独立 App、单手机/单会话、轻量优先。
- 实施顺序：P0–P5 完成 USB 基线，再执行 P6 无线扩展；USB 测试 T01–T20 不因无线功能而跳过。
