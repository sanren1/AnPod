# AnPod v2 验收记录与必要检查

## 当前验证政策（2026-10-11 用户修订）

- **20 次拔插、30 分钟锁屏和延迟对照测试：`CANCELLED_BY_USER`**。这些不再是阶段或发布关卡；不自动替换为固定次数切换、两小时播放或其他长测矩阵。
- 后续开发和规划仅保留三组必要验证：受影响代码/契约检查、一次合并的核心功能冒烟、最终交付包的签名/依赖/许可/脱敏检查。只检查本阶段受影响且已实现的功能。
- 主路径相关的安装/启动、实际播放、输出与模式切换、暂停/恢复、断开/停止、资源清理和电脑声音恢复合并为一轮；自动检查已覆盖的错误路径不再逐项真机复现。真实系统行为缺陷仅补对应最小检查。
- 复用已有检查和未变化的通过结果，不逐阶段重复全套构建/测试；非平凡逻辑缺少覆盖时补一个最小可运行检查，不搭新框架。核心功能失败、鉴权/签名错误、数据丢失及隐私泄露仍需解决。
- 未测平台和长期表现如实记录为限制，不要求穷举兼容矩阵。历史性能数据保留，没有新测量就不宣称新收益；声学对照和 Oboe 实验仅在用户重新明确要求后安排。
- 以下 P0/P1/P2 内容为历史实施与验证记录；已执行结果不改写为取消或新的通过，旧的长测/全矩阵要求由本政策替代。

## 基线与范围

- P0 基线分支：`codex/receiver-udp-v2`
- P0 基线提交：`00543589e641ae0911e660e433f55e08d4afcb6b`
- 回退基线 `main`：`8a757b4a84f5d25749b163c482dfb1262df206c0`
- 本阶段只做基线检查、构建验证和验收准备；未启动新的 AnPod 构建、未播放或录制真实音频。
- P0 时保留用户已有的未跟踪 `AnPod_plan_v2.md`；2026-10-11 按用户新要求修订验证政策，文件仍不纳入 Git。

## 自动检查与构建记录

| 检查 | 结果 | 说明 |
|---|---|---|
| Native CMake Release x64 构建 | PASS | MSVC 17.14 / Windows SDK 10.0.26100.0；输出构建在 `artifacts/p0-build-0054358/`，没有覆盖 `deps/` 或现有发布包。 |
| Native 精确发送检查 | PASS | `Native exact-send checks passed.` |
| Native DLL 架构与导出 | PASS | DLL 为 x64；`AudioCapture_*` 导出符号存在。 |
| Android PCM 纯 Java 检查 | PASS | 用可用 JDK 编译 `Options.java`、`Pcm16FrameStream.java` 与 `tests/AnPod.AndroidServer.PcmCheck.java`；输出 `Android PCM framing and mode checks passed.` |
| Android payload 构建 | PASS（审查补测） | 显式指定已有 JDK 17 和 Android SDK API 34，使用 SDK direct 路径构建。`classes.dex` 为 14,592 B，隔离输出 `server` 为 8,141 B；未覆盖 `deps/server` 或便携包。此前“缺少 SDK”结论为工具链未被默认发现，已纠正。 |
| Core .NET 测试 | PASS（审查补测） | 使用仓库本地 .NET SDK 8.0.425 和已有 NuGet 缓存，`Release --no-restore` 检查为 41/41 通过。此前“机器没有 SDK”结论已纠正。 |
| Rust、Frontend、Tauri Release 与便携预览包 | PASS（既有结果） | 据本会话对同一基线提交的既有检查结果：Rust 11/11、Frontend 交互/布局检查与生产构建、Tauri Release 和预览包构建均通过；本轮未重复运行。 |

本轮 Native 构建还修复了 `packaging/build-native.ps1` 中 CMake 输出被误并入函数返回值的问题：配置与编译输出改为写到 host stream，使后续输出文件列表只包含构建目录。此次实际构建通过。

审查补测命令如下；本机工具链绝对路径已替换为环境变量，环境变量须指向已存在的工具链，并非要求重新安装：

```powershell
& "$env:DOTNET_ROOT/dotnet.exe" run --project tests/AnPod.Core.Tests -c Release --no-restore
& ./packaging/build-android.ps1 -JavaHome $env:JAVA_HOME -AndroidSdkRoot $env:ANDROID_SDK_ROOT -ForceSdkDirect -OutputDirectory ./artifacts/p0-review-0054358/deps
```

## 设备与网络条件

- README 中既有 USB 声学测量设备为 Android 16 / API 36；这是旧版 1.0.4 基准记录，不代表本轮 v2 真机验收。
- 执行会话本轮未查询设备，不能据此判定本机没有 Android 设备或 ADB 会话。审查期间已发现 USB 与无线授权连接记录；尚未据此宣称新构建通过真机回归。实际待测 Android 版本、系统音频路由需现场确认，公开报告省略型号和设备标识。
- 计划记录 PC 热点仅能使用 2.4 GHz；热点连接时实际协商频段及 PHY/802.11 模式尚未测量。不得据此标注 Wi-Fi 6。
- README 中保留的历史声学结果：USB 耳机基线中位数 71.1 ms / P95 71.2 ms；USB 低延迟 177.6 / 178.2 ms；USB 均衡 274.1 / 274.8 ms。Wi-Fi ADB 均衡 10/10，中位数 561.6 / P95 592.1 ms；Wi-Fi 低延迟因掉线没有有效样本组，不能作模式对比。

## 真机手动验收

用户在 P0 审查时明确确认：已用手机卡片预览包验证 USB 手机播放、电脑/手机输出切换、暂停/恢复、断开/重连、托盘退出及电脑声音恢复，均正常。下列 PASS 为用户真机确认，不是执行会话自动操作结果；本轮隔离重建的 DLL 和 Android 载荷未替换预览包，不能据此宣称新载荷已实机播放。

开始前记录：

- PC / Windows 版本与音频输出设备：待填写
- 手机 Android 版本、API Level：待填写；公开记录省略型号和设备标识
- 连接方式（USB / PC 热点 / 同一 LAN）：待填写
- 热点实际频段及 PHY（可确认时）：待填写；无法确认则记录 UNKNOWN
- AnPod 提交 / 包版本：待填写

| 场景 | 结果 | 验收记录 |
|---|---|---|
| USB 连接并切换到手机输出 | PASS（用户确认） | 用户确认预览包手机播放及电脑/手机输出切换正常。 |
| 手机输出暂停、恢复 | PASS（用户确认） | 用户确认暂停和恢复正常。 |
| 断开连接与重连 | PASS（用户确认） | 用户确认断开与重连正常；不据此宣称长时稳定性。 |
| 连续 USB 物理拔插 | CANCELLED_BY_USER | 取消固定次数重复验证，不再阻断推进；必要的断开/重连纳入受影响主路径。 |
| 系统托盘退出 | PASS（用户确认） | 用户确认托盘退出正常。 |
| 系统托盘暂停、恢复 | NOT_TESTED | 暂停和恢复已确认，但未单独确认由托盘入口操作。 |
| PC 输出静音恢复 | PASS（用户确认） | 用户确认电脑声音可恢复。 |
| 切换播放模式 / 连接方式 | NOT_TESTED | 验证切换会自动断开并重连，状态提示与实际连接一致。 |
| 短时网络中断和恢复 | NOT_TESTED | 仅在相关恢复逻辑实现或变化时纳入合并检查，区分系统限制与程序错误。 |
| 锁屏长测、长时间播放与睡眠矩阵 | CANCELLED_BY_USER（默认长测取消） | 不安排固定时长测试，不以短测代替长期稳定性结论；已复现问题仅补对应最小检查。 |

## 阶段审查结论

- P0 基线构建及相关自动检查通过；预览包的关键 USB、输出切换、暂停/恢复、断开/重连、托盘退出与电脑声音恢复经用户确认正常。
- 满足进入独立 P1 APK 可行性验证的门槛。P1 不改变现有播放路径，也不据此启用新 Receiver 音频。
- P0 不代表全矩阵通过或 Wi-Fi/长期稳定性已验证。固定次数拔插、锁屏长测和延迟对照已取消；其他未测场景不统一列为后续阻断项，仅按受影响核心功能和真实风险选择必要检查。

## 回退与现有包

- 代码回退参考：本分支基线提交 `00543589e641ae0911e660e433f55e08d4afcb6b`；主线回退点为 `8a757b4a84f5d25749b163c482dfb1262df206c0`。
- 现有稳定便携包：`artifacts/AnPod-1.0.4-tauri-win-x64-portable.zip`。
- 本轮未覆盖稳定包或预览包。预览包中的 Android `server` 载荷已检查，ZIP 根目录包含 `classes.dex`。
- 若真机验收失败，停止使用候选构建，保留日志与失败步骤并回到稳定包；不要覆盖现有包或设备数据。

## P1 Receiver Spike

### 实现与信任边界

- P1 包身份：`app.anpod.receiver`，version `0.1.0-p1` / versionCode `1`，`compileSdk=35`、`targetSdk=35`、`minSdk=23`。
- P1 只验证独立 APK、短时前台管理服务和健康读取；不启用音频、UDP 或常驻服务。Receiver 使用 `shortService`，应用自身在 120 秒停止，另实现系统超时回调；Android 23 的 `onDestroy` 路径使用旧版 `stopForeground(boolean)`。
- Bootstrap Activity 和健康 Provider 均由 `android.permission.DUMP` 保护；Receiver Service 不导出、无 Launcher。APK 申请通知权限时必须由用户处理，不执行 `pm grant`。
- Windows 端先读取 Android 当前 user ID，仅在该用户范围内查询 Receiver；未安装状态用 `pm list packages --user <当前用户>` 判断。安装后，先用 Package Manager 返回的单一 `/data/app/.../base.apk` 路径，再由设备 `toybox sha256sum` 比较该 APK 与随包受信 APK 的 SHA-256。每次读取 Provider 或发送 START/STOP 前都重验 APK 字节。旧版本升级只提交 `adb install -r` 给 Package Manager 校验签名；签名不符时不卸载、不调用旧 APK 的入口。
- 已留 fake-adb 管理路径检查：同版本伪造健康但 APK 不同则在 Provider/START/STOP 前拒绝；安装失败不继续；覆盖未安装→安装→健康→启动→停止；未授权及 Wi-Fi 目标被拒绝。

### 构建与自动检查

| 检查 | 结果 | 说明 |
|---|---|---|
| Android API 35 编译与 merged Manifest | PASS | Google SDK `platform-35_r02.zip`，SHA-1 `0bb560a90a7a2cbd0dd8348224d518b638fe7949`，仅放入忽略的 `.tmp-android-sdk35/`；AGP 8.7.3 官方支持最高 API 35，Gradle 8.9。 |
| Receiver Gradle 构建 | PASS | 在 Windows PowerShell 5.1 执行；apksigner 验证成功。merged Manifest 确认通知/前台服务权限、DUMP 边界、private `shortService`，无 Launcher、无媒体播放 FGS、无局域网权限。 |
| Receiver APK 摘要 / 签名 | PASS | APK SHA-256：`b32eeaf06d40372c4a243eadf3b1c48067875af44f9e3a7b1d23d67628004f62`；证书 SHA-256：`fea24331dc877cb9537ef3596a42374871104b0b949a4cdc88b64b1ad6a45436`。当前为本机临时 debug 签名，未生成 release 密钥、未声称 release 签名。 |
| PowerShell 5.1 JSON → Rust `read_artifact` | PASS | `receiver.json` 为无 BOM UTF-8；Rust 对由脚本实际生成的 APK/JSON 调用 `read_artifact` 成功。 |
| Rust 离线检查 | PASS | `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline`：22/22 通过；`ANPOD_RECEIVER_ARTIFACT_DIR` 与 `ANPOD_P1_USB_SERIAL` 两个环境变量入口未设置时默认跳过。真机生命周期入口另行在已安装 Receiver 上执行：1/1 PASS，覆盖 status/health/start/health/stop/health。 |
| Android 普通应用权限 Probe | PASS | 无权限 Probe 声明 Receiver `<queries>`；Status ok 后由 `run-as` 读取私有结果 `provider=denied bootstrap=denied`，两入口均因 SecurityException 被拒绝。只卸载临时 Probe。 |

### target 与兼容范围

- P1 阻断验收在 Android 16 / API 36 系统、Receiver targetSdk 35 的目标设备通过；结论仅适用于这一个目标组合，不代表 v2 发布验收通过。Android 16 / targetSdk 36 的专属行为及其他 Android 版本均为 NOT_TESTED。
- API 35 对音频焦点有 targetSdk 35+ 规则；P1 不请求音频焦点。P2 音频实现必须单独验证前台服务/音频焦点约束。
- Android 12/14/15 设备及其他 OS 版本：NOT_TESTED。Receiver 没有 release 签名密钥，正式 release 签名：NOT_TESTED；候选 APK 使用本机临时 debug 证书。
- Android 17 / targetSdk 37 的局域网权限兼容差异仍未解决；P1 不请求 `ACCESS_LOCAL_NETWORK`，也不启用网络接收。
- Receiver 在前台健康状态约 121 秒后按 shortService 期限自行停止，health error 为空；强制停止 Receiver 自身包后 dumpsys 显示无该服务、health inactive，再通过合法 USB Bootstrap/START 恢复健康并 STOP 成功。锁屏与重启后纯网络唤醒为 NOT_TESTED / 未实现；P1 不含网络接收。短时服务不能作为 P2 音频服务方案。

### 真机记录

- 目标设备：Android 16 / API 36，省略型号和 ADB 标识。ADB 同时显示 USB 与 Wi-Fi 记录；依据 `adb devices -l` 选择其中唯一已授权的物理 USB 记录，Rust `ensure_usb_device` 对此选择实测通过。`adb -d get-serialno` 曾返回 `unknown`，没有用它宣称唯一确认 USB。
- 安装前只读检查：当前 Android user 为 0，Receiver 未安装，旧 `com.anpod.server.Main` 音频服务进程为 0。没有启动或停止旧 AnPod 音频。
- 首次 install：**FAIL（历史记录，原因未知）**。首次 `adb install -r` 返回 `INSTALL_FAILED_ABORTED: User rejected permissions`；用户当时反馈未看到 USB 安装提示。未据此推断 ROM 设置或绕过确认。
- 后续 install：**PASS**。本机 APK SHA-256 与 metadata 一致且 `apksigner` 证书校验通过；经用户授权的 USB 目标安装返回 `Success`。Android 随后显示通知授权页，用户明确允许通知。Receiver 未卸载或重装。
- 修复后的 Rust 管理路径真机生命周期：**PASS，1/1**。在已安装 Receiver 上验证 status、health、START、health、STOP、health；安装路径含合法 `=` 填充，所有包管理与 Provider/Activity 操作绑定当前 user 0。未执行卸载。
- 安装 APK 字节与随包 metadata SHA-256 匹配。普通应用权限 Probe：**PASS**。新版 Probe 仅申请自身可见性 `<queries>`，Status ok 后 `run-as` 私有文件读取成功，内容精确为 `provider=denied bootstrap=denied`；两入口均被实际普通应用 UID 拒绝。验证完成后只卸载临时 Probe，Receiver 保留且处于 stopped / foreground=false / healthy=false。
- 本次未执行 `pm grant`、未触发 `adb kill-server`，未操作旧 AnPod 音频。

### 候选包与回退

- 普通旧 Tauri 包只要求原有五个桌面运行文件和 Platform-Tools notice；Receiver APK/JSON 不再是 `tauri.conf` 或普通打包路径的强制依赖。
- Receiver 候选包已刷新：`artifacts/AnPod-1.0.4-receiver-p1-candidate-tauri-win-x64-portable.zip`，SHA-256 `73EF0826D605D55ACD19CEC1C75D205F4347B0C4C8D35960B30FE42BB0F59650`。含当前 Receiver APK/JSON、icon.png、原有五个桌面运行依赖及 notices/licenses/upstream；归档内 APK SHA-256 与 JSON metadata 相符。归档文件列表与文本资料未检出 Probe、密钥、日志或设备标识；release EXE 仍含本机绝对构建路径，候选仅限本机验证，公开分发前必须做编译路径重映射并复验。稳定 ZIP 未覆盖，SHA-256 仍为 `95D707F54C4037E4A7E8126D496FB9B2F1E2E31B6349ACEF4159E03B54870B9F`。
- 回退继续使用既有稳定 ZIP。候选包是 Android 16 / targetSdk 35 上的 P1 验收包，不代表 v2 完整发布；P2 与其他发布门槛仍由后续音频会话处理。

## P2 Installed Receiver 音频候选

### 本地实现与构建

- 新增 opt-in 后端：只有在启动 AnPod 的进程环境中将 `ANPOD_AUDIO_BACKEND` 设为 `installed-receiver` 时，USB 音频连接才走 APK Receiver；未设置时仍走 `app_process`，Wi-Fi 也继续走原有路径。P2 不自动安装或升级 APK。
- Receiver version `0.2.0-p2-candidate` / versionCode `2`。音频服务使用 `mediaPlayback` 前台服务类型，管理健康检查继续使用原来的 120 秒 `shortService`；两者不共用短服务时限。
- 两条 Android 路径复用同一 PCM16 帧对齐实现、8-byte little-endian 音频头及已有随机 token 握手。原 Native C ABI 保留，并新增只读本会话 PCM 成功发送字节计数。
- P2 Native DLL、APK、Legacy Android payload 和 Rust target 都必须在隔离目录构建，例如分别使用 `build-native.ps1 -BuildDirectory artifacts/p2-native-build -OutputDirectory artifacts/p2-native`、`build-receiver.ps1 -OutputDirectory artifacts/p2-receiver`、`build-android.ps1 -ForceSdkDirect -BuildDirectory artifacts/p2-legacy-build -OutputDirectory artifacts/p2-legacy-payload`，再用 `publish-tauri.ps1 -IncludeReceiverCandidate -ReceiverCandidate p2 -NativeArtifactDirectory artifacts/p2-native -LegacyPayloadDirectory artifacts/p2-legacy-payload -ReceiverArtifactDirectory artifacts/p2-receiver -RustTargetDirectory artifacts/p2-tauri-target` 生成独立 portable candidate。不得指向 `deps/` 或原 Rust target，不得覆盖 P1 或稳定包。
- P2 构建脚本审计 private Service、DUMP 保护、无 Launcher、mediaPlayback 权限及 Android 包身份。桌面 Release 启用 Rust 编译路径重映射，并扫描候选 EXE、Native DLL、APK 与文本资料中的本机源码路径。
- Receiver 前台健康表示接收服务已就绪，不代表 AudioTrack 已实际播放；连接不等待首批 PCM，也不把静音等待报成错误。原有低频会话监控会在保留 APK 身份校验的前提下检查 Receiver 健康/错误与 `audioActive`；服务停止或失败时走既有清理/恢复路径，只有 `audioActive=true` 才将手机输出更新为播放中。
- 码率按 Native TCP 成功发送的 PCM payload 字节增量及单调时钟计算；新会话/断开后重置采样，包含实际发送的静音帧，不含 8-byte 头和传输层开销。
- P2 真机音频尚未在本轮编码会话中测试；本轮未安装/升级 APK、未操作 ADB reverse、未重启 AnPod 或手机、未发出真实音频。

### 构建记录与升级兼容性

| 检查 | 结果 | 说明 |
|---|---|---|
| Java PCM framing/mode 检查 | PASS | 编译共享 PCM helper、Legacy `Options` 与 `PcmCheck`；输出 `Android PCM framing and mode checks passed.` |
| Legacy Android payload 隔离构建 | PASS | JDK 17 + SDK API 35 direct javac/d8；生成 15,020 B `classes.dex`，未覆盖 `deps/server`。 |
| Receiver 主代码与生命周期测试编译 | PASS | Gradle 8.9 / AGP 8.7.3，API 35；`compileDebugJavaWithJavac` 与 `compileDebugAndroidTestJavaWithJavac` 成功。设备端生命周期测试已编译，未运行。 |
| P2 Receiver APK 与 Manifest 审计 | PASS | 独立 Gradle 构建及 Manifest 身份审计通过；APK SHA-256：`eebb9fe4f566f6e23165f1494d1c5b08491ed2a090653bd56ce6ab78845e988c`；临时候选 signer SHA-256：`f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`。该 signer 与 P1 不同，未安装或升级。 |
| Native DLL 隔离 Release 构建与 ABI | PASS | CMake/MSVC Release x64；`dumpbin /exports` 确认旧导出保留，并新增 `AudioCapture_GetSentPcmBytes`；本机源码路径扫描通过。 |
| Native 成功发送计数 | PASS | MSVC 编译并运行 `AnPod.Native.SendCheck.cpp`；确认短写逐段计数，部分发送后失败只保留成功写出的字节数。 |
| Rust PCM 码率采样 | PASS | `state::tests::pcm_bitrate_uses_successful_byte_deltas_and_resets_between_sessions`：1/1；按单调时间与每秒成功 PCM 字节增量计算，新会话归零。 |
| Rust 码率采样与 Receiver 低频监控修复 | PASS | `state::tests::stale_pcm_sample_is_rejected_after_session_changes`：1/1；会话锁覆盖身份复核及采样/视图提交。`receiver::tests::fake_adb_health_distinguishes_waiting_playback_and_receiver_failure`：1/1；fake ADB 覆盖无 PCM 就绪、实际播放、音频焦点错误及服务停止。`state::tests::receiver_health_monitor_skips_known_local_or_transport_failures`：1/1；Native/设备已确认故障时不进入远端 Receiver 健康查询。 |
| Rust Receiver 就绪时序 | PASS | `receiver::tests::fake_adb_receiver_connect_succeeds_before_first_pcm_reaches_audiotrack`：1/1；服务健康即可完成连接，不再因无首批 PCM 而等待 `audioActive` 超时。 |
| Frontend 生产构建 | PASS | TypeScript 与 Vite 构建通过；码率在原紧凑状态行内显示、使用等宽数字，不增加每秒播报。 |
| 前台服务、音频会话与 Provider 诊断修复 | PASS（编译） | 音频会话生命周期由单会话 worker 持有并释放 AudioTrack/focus；取消后阻止新播放/焦点操作；Provider 暴露实际采样率、缓冲与 underrun（未知时显式 unknown）。设备生命周期测试覆盖 stop、EOF 与快速替换，但只编译，真机未运行。 |
| Cargo 空格路径及环境恢复检查 | PASS | Cargo remap 参数以 `CARGO_ENCODED_RUSTFLAGS` 传递；用含空格的模拟 checkout 路径验证两种既有 flags 来源及 Cargo 环境变量恢复。 |
| Rust Release 与便携候选打包 | PASS | Tauri Release、Frontend 生产构建成功。标准 P2 输出目录中的旧 `AnPod.exe` 被其他进程占用，未覆盖它；在新的 `-review` 文件名生成隔离候选。归档包含 Receiver APK/JSON、归属与上游说明；APK 与 JSON 摘要一致，未打入签名密钥，归档二进制/文本扫描未发现 workspace 绝对路径。候选 ZIP：`artifacts/AnPod-1.0.4-receiver-p2-candidate-review-tauri-win-x64-portable.zip`；SHA-256：`5D7B8BD3CD1395DF69E8E410CBE97DFA34196819AA68B478DFA67528DDBE53E7`。 |
| P1 → P2 APK 更新签名兼容性 | BLOCKED | P2 候选临时 signer 与 P1 不同，Package Manager 原位更新无法通过签名校验。未尝试升级、重签或卸载。P1 原签名密钥此前确认无法恢复；需要维护者明确批准一次卸载旧 P1 测试 APK 并安装新 P2 候选的迁移，安装动作尚未执行。 |

升级方案：P1 原签名密钥无法恢复，因此不使用 `adb install -r` 绕过签名检查。候选已完成；仅在用户明确批准后，才卸载现有 P1 测试 Receiver（会清除该应用数据）并安装 P2 候选。批准前保持设备与安装状态不变。

### 真机前置与验收

- 安装/升级候选 APK 前先由用户确认安装动作；检查候选 signer 与设备当前已安装 Receiver 的 signer 相同。未知或不符时停止，不能卸载或绕过 Package Manager 签名校验。当前正式 release keystore 状态仍需单独确认。
- P2 硬件验收限定在用户授权的在线 USB ADB 设备；保持默认后端回退操作可用。使用同一唯一设备记录，并仅建立本会话的 `adb reverse localabstract:<本会话名> tcp:<本机端口>` 映射。
- 安装并启动后，用一次 USB 核心功能冒烟确认实际手机播放、Provider `audioActive` 与 `actualMode`、两种模式与电脑/手机输出切换、暂停/恢复、断开/停止后资源释放和电脑音量恢复；相关 token/PCM 逻辑引用已有检查，不逐项重复真机验证。
- 当前核心冒烟仍为 `NOT_TESTED`，不写成已通过。20 次拔插、30 分钟锁屏和延迟对照为 `CANCELLED_BY_USER`，不再等待执行。生命周期/并发等已发现缺陷用最小针对性检查验证，只有真实系统行为仍缺证据时追加相应操作。
- 若 LocalSocket abstract endpoint 被应用 UID / SELinux 拒绝，先保存脱敏的拒绝证据；只有复现后才评估 TCP reverse adapter。不得仅因假设修改现有 P1 `app_process` 路径。
- 本地快速回退：关闭启动进程的 `ANPOD_AUDIO_BACKEND` 环境变量并重启 AnPod，即回到默认 `LegacyAppProcess`；保持已安装 APK，不需要设备卸载或数据清理。

## P3 Rust/C++ PCM Sink + localhost UDP 原型

- `protocol.md` 固定 34-byte BE 头、AES-256-GCM/96-bit nonce/128-bit tag、方向域、PCM16LE 格式、1200-byte datagram 上限和 64 序号重放窗。测试 key 是确定性向量，不可用于会话。
- Native 新增 `AudioCapture_SetPcmCallback`；Rust 动态加载时将该导出视为可选，旧 DLL 的 TCP 路径仍可加载。PCM sink 仅可在停止采集且 TCP listener 未启用时选择。此方法目前不被产品连接流程调用。
- Rust `network::tests::loopback_pcm_queue_protocol_auth_replay_pause_and_freshness`：PASS，1/1；覆盖合成 PCM、8-slot 队列溢出与旧块淘汰、loopback UDP、5 ms 分包、统计、认证失败不提交 replay、重复包拒绝、暂停丢弃/恢复、过期帧和 bounded jitter queue。
- Rust `protocol::tests`：PASS，2/2；覆盖固定 JCA 完整数据报向量，以及版本、类型、flags、长度和 session identity 头边界拒绝。
- Java 17 `tests/AnPodUdpVectorCheck.java`：PASS；JCA 输出与 Rust 测试的完整 AES-256-GCM 数据报相同。
- Native MSVC/CMake x64 Release 隔离构建：PASS，输出 `artifacts/p3-native/audio_capture.dll`（23,552 bytes）；`dumpbin /exports` 确认原有导出保留并新增 PCM callback 导出。编译器报告 `audio_capture.h` 的 C4819 代码页警告，DLL 构建成功。
- Rust loopback sender 每次消费由 `getrandom` 生成的独立 session ID/key；mock 只在内存中接收同一材料。固定 JCA 向量仍仅作测试，不作为会话密钥。设备绑定、HKDF、USB 配对和远程密钥传输留到 P5。
- P3 不构建 Android APK，不启用默认连接路径，不发送真实系统音频，不连接或操作手机；未做真机、网络性能或单向时延测试。P3 Windows UDP mock sender 只允许发往 `127.0.0.1`；后续 P4-A Android adapter 仍未接入产品启动/发现/配对流程。P5 authenticated sender 的显式 private IPv4 路径见下节。

## P4-A Android UDP parser / playback adapter（未启用）

- 文件边界：新增 `UdpSessionContext.java`、`UdpPacketDecoder.java`、`UdpPcmQueue.java`、`UdpReceiver.java`；只在 `ReceiverService.java` 增加包内 in-memory 启动/暂停 hook，并复用现有单 `AudioSession`、AudioTrack/focus、状态发布和 `Pcm16FrameStream` 播放路径。`AndroidManifest.xml` 未改：没有新增 exported 组件、网络启动 Intent 或默认 UDP listener。
- `tests/AnPodUdpVectorCheck.java`：本次合并检查 PASS。固定 JCA 向量经真实 `UdpPacketDecoder` 解包；覆盖实际 loopback recv/close、peer/session 检查、坏 tag 后同序号合法重试、重复和乱序、unsigned 序号/帧索引跨 signed 边界、非零首个 Format 序号拒绝且不提交 replay、64 序号重放窗拒收、坏帧格式不提交 replay、暂停水位拒绝暂停期间晚到帧、丢弃输入流已取出半包的剩余历史、暂停丢帧与 8 包上限。还覆盖 receiver 创建后、接收线程启动前关闭时释放绑定端口。
- Android 源码接线：Gradle 8.9 / 指定 JDK 17 / API 35 `:app:compileDebugJavaWithJavac -x :app:processDebugResources`：PASS。编译时 AAPT2 资源处理被跳过；此前资源链接曾报 `Unexpected error during link`，本次未改资源。没有组装 APK。
- 测试只在本机 JDK loopback socket 上运行；没有 Android AudioTrack 真机播放、Android 网络收包、USB/热点/LAN 联通或时延/性能检查。P4-A adapter 只接受包内提供的已解析 peer/session/key 与指定本地 bind 地址；不处理发现、配对、START/STOP/SET_MODE，不接收任意 IP 作为信任依据，也未由产品流程调用。P4 真机联通和音频表现仍为 NOT_TESTED。

## P5-A 离线绑定与会话密钥

- Rust 定向单测：PASS，4 项覆盖双方握手得到相同方向密钥、每次建立会话均使用新材料、固定 transcript/HKDF 跨语言向量，以及错误 identity/role/proof/跨会话响应拒绝。Windows DPAPI 隔离临时目录存取单测：PASS，1 项覆盖当前用户加密后读取、create-only 不覆盖和损坏密文拒绝；只使用合成测试材料，不读写应用配置。
- Java 17 `tests/AnPodUdpVectorCheck.java`：PASS。和 Rust 比对固定 transcript、双方 HMAC proof、Ready proof 及四把 HKDF 密钥；覆盖双向握手、参数篡改/身份/角色/错误证明/旧响应拒绝及新会话密钥差异。另覆盖前述 UDP parser/queue/loopback 行为。
- Android Gradle `:app:compileDebugJavaWithJavac -x :app:processDebugResources`：PASS；AAPT2 资源处理被跳过，没有组装 APK。`BindingStore` 的真实 Android app-private `AtomicFile` 行为、Android 真机存储、APK 打包均为 NOT_TESTED。
- Windows DPAPI 绑定存储及 Android app-private 绑定存储实现均是 create-only；Android 记录由系统 app-private 目录隔离，未加密 at rest。P5-F 已接入 USB provisioning、已保存绑定选择与显式网络会话；本段的历史验证未运行真实 USB。真实 USB 配对、P4-A 手机网络收包、AudioTrack 播放及远程主机交接均为 NOT_TESTED。

## P5-B USB binding provision（仅源码与 fake 检查）

- `receiver::tests::fake_adb_secure_provision_confirms_persists_and_rejects_failures_without_leaks`：PASS。fake runner 覆盖固定 USB `content write` 参数与 stdin 承诺匹配、`content write` 错误文本但退出码为 0 时仍须远端摘要确认、远端已有不同材料、取消后使用同一 DPAPI 记录重试、非 USB/current-user 改变/APK 字节不匹配拒绝；错误诊断不回显合成 secret。所有文件仅位于隔离测试临时目录，没有运行真实 ADB。
- `state::tests::adb_stdin_cancellation_reaps_child_and_output_drains`：检查 fake 子进程取消，以及 stdout JoinHandle 已消费、stderr drain 阻塞时取消会返回错误并 abort/join 剩余 drain；不调用 ADB、不访问设备。
- `tests/AnPodUdpVectorCheck.java`：既有纯 Java 记录解析检查 PASS，覆盖 69-byte 记录解析、短包/长包/未知版本拒绝。本次 Android 源码使用 Gradle `:app:compileDebugJavaWithJavac -x :app:processDebugResources` 编译 PASS；未执行 AAPT2、未组装 APK。Android fd poll、CancellationSignal、15 秒 EOF 截止时间及设备 pipe 错误行为仅编译检查，运行行为 NOT_TESTED。
- Provider 使用已有 `android.permission.DUMP` 读写保护，只扩展固定绑定 URI；health 查询保持只读。P5-F 通过显式 Tauri backend command 调用 provisioning；本段 fake 检查不覆盖真实 USB、系统 DUMP 鉴权、Android `filesDir`/AtomicFile 落盘及设备端状态确认，均为 NOT_TESTED。

## P5-C authenticated localhost session prototype

- `p5c::tests::java_receiver_and_rust_sender_share_authenticated_session_and_cleanup`：PASS，1/1。单项合并检查由 JDK 17 编译并启动真实 Java P5-A/UDP 接收代码，通过 Rust TCP localhost 完成双向握手、认证 BIND 与 mode、方向控制 HMAC、START/重复 START ACK 去重、旧 session FORMAT 拒绝、合成 PCM 到 fake sink、Streaming 心跳、PAUSE/RESUME 与 STOP；另以错误 receiver identity、错误 binding secret、坏 MAC、旧 session 控制帧确认拒绝。
- 后续定向复核在同一合并检查中增加了断开控制通道后的 STOP 本地 sender/queue/socket/key 清理、远端 START ACK 后本地 UDP sender handoff 失败清理、25 ms idle timeout 间隔中的分段控制帧读取，以及使用虚拟单调时钟的 5 秒 heartbeat 截止检查。Android 实际 `ReceiverService` UDP 队列也按已认证参数校验 FORMAT；P4 上下文的 `parameters == null` 保持原行为。
- 双模式沿用同一 5 ms UDP packetization target（48 kHz stereo 的数据报仍受 1200-byte 限制），模式差异由 Android AudioTrack buffer 设置承载；本地 fake 检查不代表音质或时延收益。
- Rust 定向 Cargo 测试：PASS。Android `:app:compileDebugJavaWithJavac -x :app:processDebugResources`：PASS；未执行资源处理或组装 APK。JDK 合并检查只编译 P5-C 所需 Java protocol/UDP 文件及 `tests/P5cLoopbackEndpoint.java`。
- 所有检查仅在 127.0.0.1 使用合成 binding 与 PCM；没有启动 AnPod 产品入口、真实音频设备、ADB、手机或局域网。P5-F 后端接线见下节；真实 WASAPI/AudioTrack 输出和设备行为仍为 NOT_TESTED。

## P5-D local single-session recovery owner

- 同一个 `p5c::tests::java_receiver_and_rust_sender_share_authenticated_session_and_cleanup`：PASS，1/1。新增场景使用静默 localhost peer 检查同步 socket 会等待 50 ms poll timeout，并检查取消可在 250 ms 内结束；Java fake endpoint 对 RESUME 返回确定失败 ACK，确认重连期间 snapshot 仍保持上次已确认的 paused 状态。显式供给重建 endpoint 后，fresh P5-A session 依最新 RESUME 意图恢复为 ReceiverRunning，再将 synthetic PCM 送到 fake sink。另检查生产退避表为 1/2/5/10 秒且上限 10 秒，测试实例注入短间隔；显式 owner stop 收到认证 STOP ACK，attempt 数不再增长；失败连接 owner 在退避中收到 caller cancellation 并 join 到 Stopped。
- `SessionOwner` 在单独线程拥有唯一 HostSession，运行 1 秒认证心跳；对每次恢复复制并清零由调用方提供的现有 binding，使用新 nonce/session/key/queue。只向显式提供的 IPv4 loopback endpoint 重连；`update_endpoint` 是切换端口的唯一入口。用户 pause/resume 意图与最近已 ACK 确认状态分开保存，失败命令不会虚报状态，重连后按最新意图操作；停止或 token 取消后禁恢复，不会清除/覆盖 binding。
- 本轮受影响的 Rust 合并 Cargo 检查：PASS，1/1，耗时 8.35 秒。静默 peer 及失败 RESUME ACK 场景在该入口一并执行。没有改 Android 生产源码，因此未重复 Gradle；Java 合并 helper 在该 Rust 检查中按 JDK 17 编译。退避测试注入短时间，不实耗 1/2/5/10 秒。
- P5-F 已接入产品状态、Tauri backend、Native PCM callback 与显式连接流程。Receiver 在断开后最多保留 30 秒 listener 以等待恢复；本段 loopback 检查未验证真实 Receiver 重启、USB/bootstrap、LAN、音量、AudioTrack/WASAPI 或实际出声，均为 NOT_TESTED。

## P4-B / P5-E explicit IPv4 and discovery backend

- Rust 合并入口 `p5c::tests::java_receiver_and_rust_sender_share_authenticated_session_and_cleanup`：PASS，1/1，8.24 秒。覆盖合成 private IPv4 地址/前缀契约、selected-prefix 内外来源校验、发现伪造候选上限（16 项后停止接收）、无关 token 拒绝、畸形探测忽略、取消，以及候选经 P5-A/BIND/START 后 synthetic PCM 到 fake sink。错误 HELLO、错误绑定身份/密钥和认证后但未 START 的错误控制帧均不调用 playback.stop；成功 START 后合法 STOP 只调用一次。其余 explicit-interface owner 重建/STOP 清理仍受覆盖。所有 socket 目标均为 127.0.0.1；未执行真实接口枚举、LAN 广播、网关探测或热点传输。
- Windows 接口枚举使用现有 `windows` crate 的 IP Helper API；只在运行时返回当前启用的私有 IPv4 地址、前缀、gateway、接口索引/名称，未写入日志、协议或持久配置。发现最多对调用方选定接口的 directed broadcast 和第一项已报告 gateway 各发一个 26-byte 探测；响应仍是候选，后续 P5-A 互认证才允许 BIND/START。手动 literal IPv4 路径不调用 discovery。
- Android Gradle `:app:compileDebugJavaWithJavac -x :app:processDebugResources --offline --no-daemon`：PASS，JDK 17、Android SDK 35 显式环境变量，9 秒，3 个任务（compileDebugJavaWithJavac 实际执行）；未执行资源处理或组装 APK。先前未配置 SDK 环境时的失败不是外部阻塞。P5-F 新增显式 Tauri backend commands；没有增加 UI caller、设置持久化、默认 listener 或自动启动。
- P4/P5 本地代码尚未在 Android runtime、真实 LAN/热点广播、ADB、真实网卡切换、WASAPI、AudioTrack 或可听输出上验证；未自动启动热点、改防火墙或连接真实网络。发现失败只报告未响应/端点不可达或可能需要 USB/bootstrap，不推断 ForceStop/防火墙原因。上述仍为 NOT_TESTED，P4/P5 整体未完成。

## P5-F product backend wiring

- `receiver::tests::fake_adb_secure_provision_confirms_persists_and_rejects_failures_without_leaks`: PASS，1/1。更新 fake fixture 使用当前 USB serial / Android user 的 opaque binding ref；覆盖合法 provisioning、相同材料重试、取消后保留并复用 DPAPI 材料、remote conflict、身份变化与拒绝路径。
- `p5c::tests::java_receiver_and_rust_sender_share_authenticated_session_and_cleanup`: PASS，1/1。新增同一 owner 的 fresh-session 输出恢复场景，确认新队列能接收 frame 0 并让合成 PCM 到达 fake playback sink；另在 sender teardown 后检查成功 payload 计数仍保留。
- `cargo check --locked --offline --target x86_64-pc-windows-msvc`: PASS。
- Android `:app:compileDebugJavaWithJavac -x :app:processDebugResources --offline --no-daemon`: PASS；未处理资源、未组装 APK。Native 隔离 Release build：PASS，`artifacts/p3-native-build/bin/audio_capture.dll`。`dumpbin /exports` 确认新增 `AudioCapture_GetPcmFormat`，既有 13 个 ABI 导出均保留；未启动 WASAPI capture。
- Native 初始化后的 negotiated PCM metadata 决定握手采样率；StartPcmSink 在开始采集/静音前复核格式，不等待首包。Computer→Phone 通过 owner fresh auth session/new queue 后再挂 capture；START 后、队列创建前收到的 PAUSE 会在队列附着时应用。异常 worker、失败/超时 START 会立即启动有限 recovery/cleanup window；healthy idle 且没有 PCM 仍报告 ReceiverRunning。
- 本轮仅运行上述两个定向 Rust 用例；两项均 PASS。Rust 编译输出有既有 unused/dead-code 警告。真实 USB、Android runtime、LAN/hotspot、WASAPI capture、AudioTrack output、可听结果及设备恢复仍为 NOT_TESTED。未运行 ADB、未组装/安装 APK、未替换稳定包或候选包。

## P8-B Windows x64 Receiver v2 portable review candidate（本机构建与包审计）

- Receiver APK 的构建提交为 `20e42fac0274252dccb8ccc8b7d5e416ab8e958b`，`receiver.json.sourceCommit` 保留该真实提交。审查期间 `src/AnPod.Receiver/`、`src/AnPod.AndroidProtocol/` 和 Receiver 构建输入没有变化，故复用该 APK/JSON；身份为 `app.anpod.receiver` / `0.3.0-v2-candidate` / code 3 / targetSdk 35 / debug variant。既有 `apksigner` 审计：PASS；APK SHA-256：`de3924d1e7801103b1fb9c9ebcde323c35493bb4fdd8aa63aa204c2998ebdb77`；signer SHA-256：`f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`。该构建 signer 与历史 P2 候选相同、与 P1 不同；没有验证 P1 原地升级。其他机器的默认 debug signer 可能不同。
- Legacy `server`、Platform-Tools 三文件集、Tauri EXE 与 APK/JSON 均复用前次审计组件。`server` SHA-256：`b916a3b0bfdd02ce6c369963f4bd1b33e85c712f73b46f8f959a72a0a92a2e70`；`AnPod.exe` 源构建提交为 `20e42fac0274252dccb8ccc8b7d5e416ab8e958b`、SHA-256：`043444422b90f8e95828e896e90a57c6d37ce61d5e4736d5543f2806032a1912`。本次没有重编前端、Tauri、Receiver 或 Legacy。
- Native 借用 callback PCM/userData，内部 Heap/COM 内存均由 DLL 自己释放，没有跨模块 CRT 分配所有权。CMake 静态 CRT 配置提交为 `de8527e26e940717f883d82cabfb89d1b84dafcc`；MSVC/CMake x64 Release Native 重编（`MultiThreaded`）：PASS；x64 machine `8664`，与已验证 P3 DLL 的 14 个导出逐名一致（含 `AudioCapture_GetPcmFormat`）。静态 CRT 后 `dumpbin /dependents` 仅有 `WS2_32.dll`、`WINMM.dll`、`ole32.dll` 和 `KERNEL32.dll`，无 `MSVCP140.dll`/`VCRUNTIME140*.dll` 依赖；二进制无仓库/用户/Native build 路径。编译仅有既有 C4819 注释代码页警告。WebView2 Runtime 仍为系统前置。
- 原始 PNPM fallback 意外完成了依赖安装，并写入外部共享 PNPM 缓存。之后恢复了原有项目直接依赖目录、移除生成的锁文件，并让最终构建使用本地 `tsc.cmd`/`vite.cmd` 与 Cargo `--offline`；共享缓存保留，未清理任何非任务目录。
- 打包器以明确的 `-ReviewPackage -ReuseExistingBuild` 复用已审计 Tauri 产物：Git 检查确认自 `20e42fa` 以来 `src-tauri/`、`frontend/` 未变化，并确认 APK 输入路径也未变化。复用组件的 source commit 未伪造为后续文档/Native 提交。
- Review ZIP：`artifacts/AnPod-1.0.4-receiver-v2-candidate-review-tauri-win-x64-portable.zip`；SHA-256：`eb3e719b074b4ae5890c9d1cb7374008588200e615cc60dd7483bbb15254bfb2`。Native DLL SHA-256：`eeda608983ef520f281d72c4f0021634b4e87c251ab83dea7f8dcb413068761e`。ZIP 有 30 个条目；递归检查外层 ZIP、Receiver APK 与 Legacy `server` 共展开 50 个 ZIP entry（5 层/个嵌套 ZIP 文件），无私密构建路径、绝对路径或禁止文件名。新增 `aes-gcm`、`getrandom`、`hkdf`、`hmac`、`sha2` 的原始 MIT 文本及 `subtle` 的完整原始 BSD-3-Clause 许可逐字节匹配本地锁定 crates；Apache-2.0 全文随包保留。稳定、P1、P2 包保持原样。
- 真机安装、P1 升级、USB 授权/绑定、直接网络认证、Receiver 前台服务通知、Android AudioTrack 出声、WASAPI 实际采集、手机强行停止/重启恢复及长时间稳定性均为 **NOT_TESTED**。没有对手机运行 ADB、安装 APK 或播放测试音；本节仅记录本地构建与静态包审计结果。

## P8-C Receiver v2 音频与连接流程候选复核

- 代码 `b914880` 修复低延迟起播预滚动后队列目标为 0 帧的情形：队列在预滚动后至少保留 5 ms 初始余量，仍受现有 20 ms 上限约束。此处只处理已证实的启动缓冲问题，不据此宣称之前的严重卡顿已全部解决。
- `p5c::tests::java_receiver_and_rust_sender_share_authenticated_session_and_cleanup`：PASS，1/1。本机合成场景以 48 kHz、10 ms 采集块模拟一次 WASAPI 块被切为两个 5 ms UDP 包并批量发送；验证低延迟初始会话、同模式重选不重建、切换均衡时 fresh authenticated session 与旧 PCM queue 清理、新会话从 frame 0 起播，以及后续重新认证。Java helper 在该合并检查中由 JDK 17 编译。该合成不代表真实 WASAPI、无线传输或 AudioTrack 的播放性能。
- 源码流向核查：首次 `playbackMode` 设置会映射进认证 `AudioParameters`；Receiver 将已认证模式带入 `UdpSessionContext`、播放队列预算和 `AudioTrack` 设置。相同模式调用不触发新 session；不同模式会关闭当前 session、清理旧队列并重新认证。源代码未显示 PCM partial payload 跨会话复用。
- UI 说明：只有一个选项时自动填入 USB 手机、已配对绑定、网络接口或地址；多项时在“高级选项与诊断”明确选择。发现地址仍仅是提示，连接前必须双向认证；Receiver 服务未启动时可用“USB 启动所选手机”恢复所选手机的服务。
- 用户报告重选均衡后音质改善属于主观反馈，未做新声学测量。首次断联后无法重连没有复现，重连状态机未修改。真机音频、断线恢复、UI 视觉均为 **NOT_TESTED**；取消的长时间测试继续保持 `CANCELLED_BY_USER`。本轮只做源码与本机合成检查，不使用 ADB 或真实设备。
- 2026-10-12 Windows x64 portable v2 candidate 在源码提交 `81dda7630e2728881d4aa556698ba8f6bb36c453` 上完成打包。复用了已审计 Tauri EXE、Native DLL 与 Legacy `server`，Receiver APK/JSON 来自同一提交；APK 为 `app.anpod.receiver` / `0.3.0-v2-candidate` / code 3 / targetSdk 35 / debug variant，签名 SHA-256 为 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`。APK SHA-256：`4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050`；EXE SHA-256：`72f6f03a78ba7c3db66a18bb5cc94c9487e127e4f8a76716fdce4383a9b51b2f`。
- Candidate ZIP：`artifacts/AnPod-1.0.4-receiver-v2-candidate-tauri-win-x64-portable.zip`；SHA-256：`1978dbbecfa79ddc5fe281cd54514a9b43b71d6b7334306558e0e7b2d8bac64b`。外层 ZIP 有 30 个文件；独立递归审查确认外层 ZIP、Receiver APK 与 Legacy `server` 共 3 个 ZIP、40 个文件项，路径/私密构建路径/禁止文件名审查 PASS。包内许可证与 notices 逐字节匹配源文件，新增 AES-GCM/HKDF/HMAC/SHA-2/getrandom MIT 与 subtle BSD-3-Clause 许可，并包含 Apache-2.0 全文；`AnPod_plan_v2.md` 未打包。根任务复核确认四个组件摘要匹配，Receiver 源提交及签名正确，APK/Tauri 输入相对来源提交无变化：Native DLL SHA-256 `eeda608983ef520f281d72c4f0021634b4e87c251ab83dea7f8dcb413068761e`，Legacy `server` SHA-256 `b916a3b0bfdd02ce6c369963f4bd1b33e85c712f73b46f8f959a72a0a92a2e70`。审查 PASS。
- 真机安装、P1 升级、USB 授权/绑定、直接网络认证、Receiver 前台服务通知、完整 AudioTrack 流式播放、WASAPI 实际采集、手机强行停止/重启恢复、断线恢复、UI 视觉及长时间稳定性均为 **NOT_TESTED**。未运行 ADB 或安装 APK；取消的长时间测试保持 `CANCELLED_BY_USER`。
