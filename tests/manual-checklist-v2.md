# AnPod v2 验收记录与必要检查

## USB / 一键接收本地审查候选（2026-10-15）

- U1 USB-only 升级与安装后核验、旧网络 APK 安装退役，U5 绑定认证的一键接收与取消边界，U6 两端界面精简已通过源码审查。首次绑定仍 USB，不以发现地址建立信任，不抢占活动会话；HMAC type17/bit0x10 关联接收意图，退役编号不复用，音频预算未改。
- 最小离线检查：Java/Rust合并回环PASS（1项，30.93秒，含错误/缺失/超时/取消意图及启动交接取消）；发现单次消费/监听停止取消PASS；保存绑定选择/后台不抢占PASS。无线19项、USB升级15项、主操作与焦点等15项headless断言PASS，24个小屏/主题布局无横向溢出，放大文字与配对输入焦点模拟PASS。模拟不等于真机无障碍或播放结果。
- APK9完整隔离assembleDebug PASS（37tasks，44秒）；旧审计规则仍要求网络安装权限的问题已修正，随后37项全复用，DUMP保护、非公开服务和退役权限禁入审计PASS。APK SHA-256 `2b5ac4120d02ef46d8a96776d82c671622020357ce3e6b9739ce638f0bfd6cc5`，签名证书 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`，与原候选连续；本地运行时metadata读取PASS。
- 桌面release隔离构建PASS（1分48秒），EXE SHA-256 `9ade3a2a3ddf18565515d01ff6a3409563059a9c70a950a52dc1cfa6be54e3fd`。基线提交 `61f32f7798f29bb39cf256997de50992a85ea519` 未变化，源码仍未提交；包内metadata用 workingTreeReview 和源码输入摘要区分实际构建内容，不能声称该基线提交本身包含本轮变更。Native/Legacy输入和已审计二进制未变，复用原608c98c来源。
- 独立 ZIP：`artifacts/AnPod-1.0.4-receiver-v3-usb-oneclick-review-tauri-win-x64-portable.zip`，17,836,433 bytes，SHA-256 `21fa5ae9d3677aac16a5f2100387a8428152f4dca7348c554592d5b3873bf24e`。31/31项与目录逐字节一致；组件/来源/源码摘要、许可证/notice/upstream、路径安全、禁入文件和私有路径（含APK内部）审计PASS，旧网络安装类不在DEX。当前扫描不证明公开Git历史已脱敏。
- 上述为本地候选构建时的记录。2026-10-15 后续已通过 USB 6→9 升级并核验原绑定保留；无线重试后用户确认出声，但均衡仍卡顿/破音，首次握手失败和 PC 操作顺序问题未关闭。USB Receiver 均衡/低延迟均完成 10 个声学短测样本，中位数分别 172.160 / 125.651 ms，含麦克风回录耗时，音质仍 NOT_PASSED；详细范围与计数见文末。通知/停止/生命周期、大字体/TalkBack等未完成项仍 NOT_TESTED。旧长测不恢复，原始证据和私有计划不提交或打包。

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

- V2 把 `audioBackend=receiver|legacy` 作为独立于传输协议的持久后端选择；`connect_device` 对 USB 与无线 ADB 均使用所选后端，无线 ADB 不会因 `wirelessProtocol=udp` 被误路由到 UDP。为遵守 USB 默认迁移门槛，当前默认仍是 Legacy；Receiver 路径须明确选择，失败不会静默切换后端。
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
- 本地回退：断开当前会话后将 `audioBackend` 明确切回 `legacy`；保留已安装 APK，不需要设备卸载或数据清理。

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

## P8 A/B/C 统一候选源码与定向检查

- Receiver 使用已认证会话的采样率和声道数初始化 PCM 格式与流头，不依赖一次性 FORMAT 包；FORMAT 若到达，必须为 sequence 0 并匹配已认证参数，否则丢弃而不改变 replay state 或播放。`Streaming` 需要近期有效会话 PCM、成功写入以及 AudioTrack 播放头推进；该状态不等同于可听出声。
- Native 采集队列按帧数与本地单调时间有界，低延迟/均衡上限分别为 20/40 ms；超大块保留较新的尾部，过期数据在发送前继续淘汰并计入 drop 统计。Android 网络 jitter reserve 从 5/15 ms 起步，可随 underrun 增至 20/40 ms；它独立于 AudioTrack 缓冲和 startup pre-roll。
- AudioTrack 均衡模式请求 50 ms，低延迟模式请求 15 ms，API 31+ 请求 5 ms start threshold；请求受 capacity 限制，日志/Provider 区分 requested、accepted、effective、capacity、threshold 和 pre-roll。pending frames 包含已写入的 pre-roll，并以 wrap-aware playback head 扣除；它是 PCM 排队帧数，不是声学延迟。倒序/过期 playback-head 样本不推进累计播放位置，显式 reset 与正常 unsigned wrap 保持有效。
- 既有定向检查复用结果：PCM framing/mode 与 playback-head 检查 PASS；Receiver Java 编译 PASS；Legacy Java 编译 PASS；`p5c::tests::java_receiver_and_rust_sender_share_authenticated_session_and_cleanup` PASS，1/1。C 修复后的 PcmCheck 和 P5c 集成检查再次通过。Receiver/Legacy 调用代码未变化；共享 PCM helper 的倒序采样修复通过 PCM 检查，本次 APK/Legacy 重建覆盖该修改。
- 历史声学测量只代表其明确记录的旧测试路径，不代表这批 A/B/C 候选。当前统一候选的真机安装、音频出声、连接恢复和 UI 检查均为 **NOT_TESTED**；此前两声预检音不作为本候选验收，不进行手机操作或真实音频验证。

### A/B/C 统一候选构建与交付审计（2026-10-12）

- 构建输入提交：`608c98c75ec04e11555e9e547caa516c96d4e0dd`（Anonymous）。Tauri Release EXE、Receiver APK metadata 的 `sourceCommit` 与本次源码构建一致；本条审计仅为之后的清单文档提交，不改变 APK/Tauri 构建输入，也不改写 metadata。
- 本次重建组件：EXE SHA-256 `cb3da8a25c291175522ada748c2479756f66b7c848b9adb3a25eb19436cca38f`；Receiver APK SHA-256 `22af84b1989011d3cd214ff321ccd37bc4fe4de03546850f65d5db778f49d8c9`；APK 为 `app.anpod.receiver` / `0.3.0-v2-candidate` / versionCode 3 / targetSdk 35 / debug，signer SHA-256 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`；Legacy `server` SHA-256 `9552af169e4c2b2ab82c39719d3afd7c2c5dd729551d8d487e836013eeb9cfa3`。APK DEX 与 Legacy `server` 均由本次构建从最新共享 Java/PCM helper 源码生成，包含 `2c2dbbc` 倒序采样修复。
- Native DLL 从既有已审计组件复用，SHA-256 `eeda608983ef520f281d72c4f0021634b4e87c251ab83dea7f8dcb413068761e`；既有审计确认 x64、静态 CRT、14 个导出，EXE 无额外 MSVCP/VCRUNTIME 依赖。Platform-Tools 是独立外部进程组件。
- Review ZIP：`artifacts/AnPod-1.0.4-receiver-v2-candidate-review-tauri-win-x64-portable.zip`，8,166,288 bytes，SHA-256 `1be79b2ee09cc88cc9d29091f01ffb70b9d7017be9b112e95351a1b5be10cdc3`。外层 30 项；3 个 ZIP（外层、APK、Legacy server）递归共 40 个文件项。根任务最终审计未发现不安全路径、禁止文件、已知私密构建路径或私钥；17 项许可逐字节匹配来源，README、notices、APK/JSON、server 与 Native 均匹配各自构建输入。旧 canonical 候选目录保留未覆盖；包中不含 PDB、密钥、临时 probe、日志、测量文件或私有计划。
- 本次前端生产构建、Tauri Release、Receiver APK assemble 与 Legacy `server` 重建均通过。既有针对性 PCM/Receiver/Legacy/P5c 检查复用；没有重复全量测试。真机安装、P1 升级、USB 授权/绑定、Receiver 通知、实际 AudioTrack 出声、WASAPI 采集、连接恢复、UI 视觉及长期稳定性均为 **NOT_TESTED**；此前两声预检音不计为当前候选验收。未使用 ADB、手机或真实音频。

## Receiver v3 兼容性 V0（本地源码与协议契约）

- V0 在 Rust 安装身份检查中只信任当前本地 Receiver APK 的精确 SHA-256，或下列具有来源提交、versionCode 与 signer 记录的历史 APK；身份判断要求 package 与 signer 匹配，不用本机目标 APK 版本高低否定已审计安装身份。已审计条目：`20e42fac0274252dccb8ccc8b7d5e416ab8e958b` / code 3 / SHA-256 `de3924d1e7801103b1fb9c9ebcde323c35493bb4fdd8aa63aa204c2998ebdb77`；`81dda7630e2728881d4aa556698ba8f6bb36c453` / code 3 / SHA-256 `4d984fa8534a7c65ce0792de217688fad9b536a939cae0e59c58f82afcd46050`；`608c98c75ec04e11555e9e547caa516c96d4e0dd` / code 3 / SHA-256 `22af84b1989011d3cd214ff321ccd37bc4fe4de03546850f65d5db778f49d8c9`。三者 signer SHA-256 均为 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`。候选更新仍须 versionCode 高于已安装版本且 signer 相同，因此不会自动降级。
- P1 code 1 因 signer 不同而排除。P2 code 2 的审计 SHA-256 为 `eebb9fe4f566f6e23165f1494d1c5b08491ed2a090653bd56ce6ab78845e988c`，但可用审计记录没有保留其 `sourceCommit`，所以不加入受信历史名单；不推测来源提交。公开 Releases 页面当前没有发布包，已审计本地候选最高为 code 3，因此 V1 新 APK 的首个 versionCode 至少为 4。
- `protocol.md` 记录 V3 管理能力协商草案：沿用 P5-C 外层 v1 帧和原有 13-byte ACK，新增固定 8-byte 能力查询与独立 20-byte 响应。只有已认证、身份可信、管理 major 为 3 且明确通告 `apk-update-v1`，才允许网络更新命令；音频 major 不兼容时仍可更新。未知/未协商管理端、旧 major 或缺更新能力只允许本机预检与 ADB/人工引导，不能发送未知更新命令；认证失败或未知身份拒绝。V0 不实现能力网络交换或 APK 网络更新；v2→v3 首次升级仍须通过已授权 ADB/人工方式安装同 signer 包。
- 保留的音频基线：Native 队列低延迟/均衡上限 20/40 ms，Android jitter reserve 从 5/15 ms 独立增长至 20/40 ms，AudioTrack 请求 15/50 ms，API 31+ 仅低延迟请求 5 ms start threshold；播放头按 unsigned wrap 处理并拒绝倒序/过期样本。相关修复提交：`e6ad36a`、`d56a3a3`、`9d2c4f7`、`2c2dbbc`。
- install 命令返回成功后，Receiver APK 摘要还必须精确匹配本次目标 APK；仅同 versionCode/versionName 的旧受信包不得作为成功安装报告。
- `cargo test --manifest-path src-tauri\Cargo.toml --locked --offline --bin anpod-tauri receiver::tests::`：PASS，20/20；覆盖兼容性权限断言和“安装命令成功但仍读回同 code/name 历史 APK” fake ADB 场景。显式真机 USB 用例因未设置 `ANPOD_P1_USB_SERIAL` 而跳过。未重建 APK/ZIP、未运行产品构建，也未安装/操作手机；本候选真机安装、视觉和音频仍为 **NOT_TESTED**。此前手机两声预检音只确认手机发声，不作为本候选验收。

## Receiver v3 V1 Launcher candidate

- `MainActivity` 是唯一普通 Launcher，使用 Compose Material 3；`ReceiverService` 提供进程内 Binder 状态快照、可用私有 IPv4 列表以及开始/停止入口。音频、网络协调器和恢复意图仍由服务持有；停止会先使旧 generation 失效再清理会话与监听器。UI 不展示绑定名、binding commitment，也不提供 V2/V3 搜索、模式切换或网络更新功能。
- `packaging/build-receiver.ps1`：PASS。Gradle 8.9 / JDK 17 `assembleDebug` 成功；合并 Manifest 仅允许 `MainActivity`、受 DUMP 保护的 Bootstrap/Provider 与受 DUMP 保护的 AndroidX ProfileInstallReceiver 导出；ReceiverService 保持私有。APK 为 `app.anpod.receiver`、code 4、`0.4.0-v3-candidate`、targetSdk 35，debug signer SHA-256 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`。源码提交 `ca380b04e6b1d6e774eef45240dd995385f67bce`。
- 产物：`artifacts/v3-v1-receiver/receiver.apk`，SHA-256 `ef4a37ea1e5ffeb733cbc6483f914f2b6c51aec3cd28c1c1cb99ba17fd70702a`；`receiver.json` 内的 SHA、签名和源码提交已与产物核对。
- 桌面端声音输出卡片保持图标在左、名称在右，高度调整为 48px；播放模式下的额外说明段已移除。前端 `tsc -b` 与 Vite production build 均通过。
- Receiver UI 停止入口覆盖 UDP 与 USB/本地会话，并以 generation 拒绝已停止会话的过期回调；只显示当前会话报告的实际播放模式。
- 没有运行 ADB、安装 APK、启动模拟器或操作手机；Receiver 真实 Binder 生命周期、通知授权流程、TalkBack、横竖屏视觉和实际音频仍为 **NOT_TESTED**。离线构建曾因 Kotlin Compose 插件缓存缺失停止；在线解析依赖后，构建通过。Windows 沙箱中 AAPT2 曾异常退出，最终包在核准的本地构建环境中成功链接并完成审计。

## Receiver v3 V1 review correction

- `ReceiverUiOperation` 是 Binder/UI 与 `ReceiverService` 回调共同使用的生产 owner/phase gate。`tests/ReceiverUiOperationCheck.java` 由 JDK 17 直接编译运行：PASS；覆盖外部 owner 停止、停止及失败后重开、过期 owner 回调拒绝，以及监听 prepared 不等同于已连接。
- 外部 Bootstrap 会获得稳定 owner ID 并显示在 Binder 快照；UI 停止必须匹配当前 owner。会话终止统一先撤销 generation，再取消音频/关闭 coordinator；完成后即使 Activity 仍绑定也会清除 FGS 通知并留下可重开的 stopped/error 状态。回调在主线程执行副作用时再次检查 owner 和 generation。监听打开状态显示“接收已准备，等待电脑连接”。
- OutputCard 使用原生 Vector 电脑/手机图标并保持装饰图标不重复读名；卡片、主按钮和网络选择按钮采用内容自适应高度并保留至少 48dp 触控高度。
- 修复源码提交：`9a2d97d5560532428bf6eef4bfc9d69316f50f1b`。独立候选 `artifacts/v3-v1-receiver-review-2/receiver.apk`，SHA-256 `6805e13e6149fbceeab5ad23a9d9f974d2112255a4df5132b45d6c774023bb8b`；签名仍为已审计 debug signer `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`。打包脚本完整构建与包审计：PASS，metadata 源码提交与上列源码提交一致。
- 真机 Binder 生命周期、通知权限交互、TalkBack、放大字体/横竖屏视觉和真实播放均未运行，保持 **NOT_TESTED**；未使用 ADB、模拟器或手机。


### Receiver v3 V1 `startId` lifecycle correction (2026-10-13)

- `ReceiverService` separately records the latest Android-delivered `startId` before every early return; operation ownership retains its own `operationStartId`. After current-owner resources are cleaned up, the started lifecycle stops using the latest delivered ID, including a higher ignored management request.
- A late cancelled `UI_START` is rejected before foreground-service work. It stops through the latest delivered ID only when there is no active operation; an active newer owner suppresses that stop decision.
- `ReceiverUiOperationCheck`: PASS. Covers late rejected start cleanup, suppression while a newer owner is active, and an ignored management ID being included when the owner terminates.
- Final debug APK: `artifacts/v3-v1-receiver-review-3/receiver.apk`; SHA-256 `2b2c3c5c80f33b4d999e3445c83d78150df506b6e6608425966500db7c6872d1`; source commit `55fa1b070c7c6386f0298f4b263c52ff129b8817`; metadata source commit and APK digest match. Package: `app.anpod.receiver`, version `0.4.0-v3-candidate`, versionCode 4, targetSdk 35, debug signer SHA-256 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`.
- Android-delivered `startId` and foreground-service behavior remain **NOT_TESTED** on a device. No ADB, emulator, or phone operation was performed; installation prompts and device installation remain **NOT_TESTED**.

### V2 Receiver backend handoff (2026-10-13)

- Canonical backend setting contract: `transport=usb|wireless`; wireless selection separately persists `wirelessProtocol=udp|adb`, defaulting new settings to UDP. A legacy saved `preferredTransport=Wifi` with no protocol field migrates to ADB; explicit `udp`/`adb` remains unchanged. The old `ANPOD_AUDIO_BACKEND=installed-receiver` opt-in is migrated only when the new `audioBackend` field is missing. Snapshot adds `connectionTransport=usb|wireless` plus active-session transport/protocol fields; the current frontend's `transport=usb|wifi` and `preferredTransport` remain compatible aliases.
- Active `reconfigure_session` validates `transport`, `wirelessProtocol`, `audioBackend`, and `playback` values before beginning an operation. A route change cleans the old owner/session before reconnecting to a known same-device target. When UDP endpoint/binding/interface information or an ADB device is missing, it returns to computer output with an explicit ready/pending-selection status; UDP does not guess a peer or fall back to ADB. Monitoring and automatic reconnect honor the saved wireless protocol, while explicit `connect_device*` commands remain ADB operations.
- `connect_device` / `connect_device_only` / `switch_device` are ADB commands. The existing `start_network_audio` entry explicitly selects the authenticated UDP path. `audioBackend=receiver|legacy` is a separate persisted choice, not a wireless protocol. Receiver uses the existing ADB reverse + LocalSocket schedule on USB or wireless ADB; failure cleans the selected path and does not retry Legacy.
- The USB default remains Legacy because the minimum real-device Receiver smoke is unavailable and no USB default migration has been performed. Selecting the Receiver backend explicitly is available for source-level review. ADB preflight accepts only exact local or audited same-signer APK identities, compares service health with the installed artifact's version (so a newer compatible Receiver is not rejected by an older PC target), blocks older targets, and requires exact target APK digest after install.
- Operation/session generations, owner teardown, output restoration, 20/40 ms PC queue bounds, Android jitter reserve, AudioTrack settings, FORMAT checks and playback-head stale/wrap fixes remain unchanged. The existing stale PCM callback test now checks both rejection of the old session sample and acceptance of the current session sample.
- Targeted V2 checks: prior unchanged fake ADB group passed 12/12. After the routing/migration changes, `cargo test --locked --offline --manifest-path src-tauri/Cargo.toml --bin anpod-tauri state::tests::` passed (14 passed, 1 ignored), covering legacy Wifi-to-ADB migration, explicit protocol preservation, auto-ADB gating, selection validation, and stale operation completion. `ReceiverUiOperationCheck` passed with bootstrap STOP owner routing and duplicate-stop coverage; `:app:compileDebugJavaWithJavac` passed. The compile needed elevated local execution because sandboxed AAPT2 resource linking exited unexpectedly. No APK/ZIP rebuild, full portable build, real ADB/phone operation, or audio playback test was performed. USB Receiver smoke and Binder/foreground-service STOP runtime remain **NOT_TESTED**.

### V2 desktop connection selection UI (2026-10-13)

- Connection controls now use canonical `connectionTransport=usb|wireless`; wireless exposes separate `wirelessProtocol=udp|adb` choices. Idle changes persist through `update_setting`; older `preferredTransport=Wifi` migrations are rendered from backend settings and are not overwritten on load. Active selections use `reconfigure_session` and apply its returned snapshot. Displayed active route labels use `sessionConnectionTransport/sessionWirelessProtocol`.
- UDP intent routes the phone output, add-phone, device-card action and device list to the existing Receiver network dialog; it does not expose Wi-Fi ADB devices or call `connect_device*`. ADB intent reuses the existing wireless-debug pairing/address flow and explicit ADB connect commands. Candidate addresses remain labelled unverified and require an explicit connect action. Normal UDP bootstrap remains USB-only. The secondary-menu Receiver manager requires explicit selection from authorized, online USB or wireless ADB devices, calls `receiver_status` without network-interface/binding discovery, and only calls `receiver_install` after an explicit install click; it cannot start UDP. Receiver/Legacy remains a separate preference, with USB default Legacy.
- UDP intent or active UDP session shows a fixed Receiver backend and hides the irrelevant Legacy/Receiver selector without changing the saved preference. Missing UDP interface/binding/endpoint leaves the backend's ready/computer-output snapshot in place and opens the existing network selection dialog; no peer is guessed and there is no UDP-to-ADB fallback. Busy states disable duplicate selection; when cleanup is pending, connection switches stay disabled but the device-card action still allows a retry of `disconnect`. Cancelling confirmation leaves the backend selection unchanged.
- Frontend verification: TypeScript project build (`tsc -b`) PASS; Vite production build PASS (1,911 modules). No browser/mock interaction or visual viewport check was run: the repository has no mock snapshot entry, and outside the Tauri runtime the page remains on its startup screen. USB/ADB/UDP connection behavior and audio remain **NOT_TESTED** in this UI turn.

### V3 authenticated negotiation and bidirectional discovery (2026-10-13)

- P5-C authenticates P5-A first, then negotiates management control major 3, audio major 1, and the implemented `receiver-ui` / `authenticated-status` bits before BIND/START. Unsupported control, missing required management bits, replay, malformed payload, invalid HMAC, or management-incompatible capabilities terminate that owner without retry or downgrade. If management is compatible but audio is incompatible, the authenticated owner remains online in management-only state on computer output; it sends no BIND, START, or PCM. `versionCode` remains informational and APK update is not advertised.
- V2 discovery records reuse UDP port 49772. Android binds its discovery sockets to the explicitly selected Android `Network`, with its receive socket bound wildcard; it searches for ten seconds and exposes candidate/request state through the existing private service Binder. PC offers are enabled only by an explicit command while saved transport is Wifi + UDP. The PC receive socket also binds wildcard on 49772, filters source port and subnet against the selected private IPv4 interface, and sends replies through a socket bound to that selected local address. Both directions correlate nonce/source/port, expire pending requests, and reject replay. A candidate remains unauthenticated until the user chooses a saved binding and explicitly accepts; then the PC initiates P5-A.
- Targeted verification: Rust/Java localhost P5-C negotiation and cleanup `PASS`; the merged loopback check now runs the Java search/PC-candidate/CONNECT_OFFER/cancel path against the Rust host listener; Rust offer replay and expiry checks `PASS`. Android `:app:compileDebugJavaWithJavac` `PASS`. The selected-Network/wildcard binding and PC wildcard-receive/selected-interface-reply behavior are verified in source and localhost checks; these checks do not verify actual LAN broadcast delivery, Binder interaction, or device network-loss revocation. LAN/hotspot discovery, Android runtime, real P5-A, audio output and installation remain **NOT_TESTED**.

### V3 P5-C isolated candidate device install and Launcher check (2026-10-13)

- Candidate `artifacts/v3-p5c-role-fix-review-1/receiver.apk`: package `app.anpod.receiver`, version `0.4.0-v3-candidate` / code 4 / targetSdk 35; source commit `b3e57077c61be64c6f0db3b1c1fad61080d40313`; APK SHA-256 `48d0ff9d6b4437343dedb97c752143b84969ceb72d9c2a9579850cd59f545345`; debug signer SHA-256 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`.
- The first authorized `adb install -r` returned `INSTALL_FAILED_ABORTED: User rejected permissions` while the phone was locked. After the user unlocked the phone and explicitly authorized one retry, the same in-place update returned successfully. Read-back reported code 4 and version `0.4.0-v3-candidate`; pulling the installed `base.apk` and hashing it produced the exact candidate SHA-256 above. No uninstall, data clear, permission grant, downgrade, or additional install attempt was performed.
- Launched the existing `MainActivity` and visually inspected its Launcher screen. It showed “等待连接” and the expected candidate version; `ReceiverService` was bound for its UI snapshot, with `startForegroundCount=0`. The screen reported no available private IPv4 interface, and both network-interface selection and “开始接收” were disabled. ADB showed Wi-Fi enabled but only loopback had an IPv4 address. No BIND, START, audio, or network-discovery operation was triggered.
- After the user connected the phone to the PC hotspot, Android reported a private `wlan0` IPv4 address and `/24` prefix; the UI listed and selected that interface. Pressing “开始接收” changed the screen to “接收已准备，等待电脑连接”; the foreground notification/service became active and UDP 49772 appeared in the socket list. No audio session started (`actualMode` was still not reported), and no user audio was played.
- Pressing the existing UI “停止接收” ended the started service lifecycle. After cleanup, `startRequested=false`, the foreground-service state was gone, and UDP 49772 no longer appeared in the socket list. Returning the Activity to Home then removed the bound `ReceiverService` record as well; no receiver listener remained active.
- The current Launcher explicitly says computer search is not open. Although the PC Tauri commands for `set_wireless_connection_offers` and `accept_wireless_connection_offer` are registered, repository search found no frontend caller; the Android Binder `startSearch`/`requestConnection` methods likewise have no Launcher controls. The available authenticated `start_network_audio` route can start the audio path, so it was not used as a substitute for a management-only P5-A/capability query.
- Result: package install, Launcher rendering, hotspot interface preparation, UI start, and UI STOP/socket cleanup **PASS**. Actual Android↔PC V3 broadcast discovery, real P5-A/capability query, active-session cancellation, network-loss cleanup, and candidate audio output remain **NOT_TESTED** because neither product UI exposes the required discovery/request controls or management-only authenticated query path. P5-C localhost Java/Rust and source-level checks remain separate evidence and are not device results.

### V3 bidirectional discovery UI (2026-10-13)

- Android: start Receiver on a selected private IPv4 interface; verify the interface picker is locked while running, then start/stop the ten-second PC search. Candidate rows must show only the address and an unverified hint; select a row and explicitly request connection. Verify errors are visible, the button reports a sent request, and leaving/reopening the Activity shows service state without stale callbacks or a second Receiver session.
- Desktop: from the wireless UDP network dialog, explicitly select an interface and allow phone searches. Verify the main window retains a visible stop control after closing the dialog; changing interface/transport/protocol and changing or accepting the listener are disabled while an audio session is active.
- Desktop offers: when one current interface, saved binding, or unexpired request exists, verify it is preselected; multiple choices remain explicit. Keep still-valid choices across snapshot refresh and clear a request selection when that request expires. The single selected request remains unauthenticated until the user clicks **接受并连接所选手机**. Its PC acceptance window is 30 seconds from `CONNECT_OFFER` receipt; Android search nonce expiry remains 10 seconds. Search/accept controls stay disabled during an audio session or update lock.
- Management-only session: verify computer output remains selected, the phone output is disabled, and the status never says phone-ready or streaming. Capability majors, compatibility flags, capability bits, and Receiver version code belong only in Diagnostics. No network APK update control is present.
- Source verification: TypeScript project build and Vite production build **PASS**. `packaging/build-receiver.ps1` **PASS**, including Kotlin/Java compilation, merged-manifest/package checks, signature verification, and metadata write. Isolated APK: `artifacts/v3-ui-review-1/receiver.apk`, package `app.anpod.receiver`, version `0.4.0-v3-candidate` / code 4 / targetSdk 35; SHA-256 `b52a326f234ab84d6aba9b5ffbb840e75f90a101cb5479461da7a53d184a4abb`; signer SHA-256 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`; metadata source commit `8f868708a83a32973be902fabf7dc8357f3c55a6`.
- Device update: before installation, the installed code-4 APK digest matched the isolated `b3e5707` candidate exactly. One same-signature `adb install -r` succeeded after Android's `PackageInterceptActivity` confirmation; the installed package now reports code 4 / `0.4.0-v3-candidate`, and its `base.apk` SHA-256 exactly matches the new candidate (`b52a326f234ab84d6aba9b5ffbb840e75f90a101cb5479461da7a53d184a4abb`). User data was preserved; no repeat install, uninstall, data clear, or permission bypass was used.
- Android visual lifecycle check: the installed Launcher showed the Receiver search section with an enabled “搜索电脑” button only while Receiver was prepared on the selected hotspot's private IPv4 interface. The button was not pressed; no LAN search, P5-A, or audio started. Pressing “停止接收” returned the page to “等待连接” and `ReceiverService.startRequested=false`. After Home and reopening MainActivity, it rebound cleanly with no started Receiver service and no stale search controls. At reopen Android reported no available private IPv4 interface, so “开始接收” was disabled; network settings were not changed.
- Desktop visual/browser check, LAN discovery, real P5-A/capability query, and audio remain **NOT_TESTED**. No AnPod desktop process was running for this device check.

### V3 PC review candidate hotspot smoke (2026-10-13)

- Built the isolated Windows x64 portable candidate from desktop commit `840b457a664d8249e9f9b672b02ef708c55cced3`. Package: `artifacts/AnPod-1.0.4-receiver-v3-candidate-review-tauri-win-x64-portable.zip`, 17,745,536 bytes, SHA-256 `2258b3eaf5c37e86eb9d54961487eb8ddd97f30fee4c2302268be0c56fbfa0d8`. The ZIP audit found 31 entries, 18 license/notice entries, and no prohibited paths or private plan files.
- Used a child-only isolated `LOCALAPPDATA` copy for desktop settings and the encrypted Receiver binding; the original user data was left untouched. The candidate showed Receiver `0.4.0-v3-candidate`, version code 4, installed on the phone, but its service was not running.
- **PARTIAL PASS:** selected the PC hotspot interface, enabled the ten-second discovery listener, and confirmed the phone could find the PC and send its first connection request. The desktop displayed the request as unauthenticated and the saved binding was selected. The offer expired before acceptance, so no authenticated session was established.
- **FAIL / NOT COMPLETED:** the subsequent paired-phone attempt reported that no Receiver answered on the selected interface; another attempt surfaced `BootstrapTimeout`. The desktop still reported the Receiver service stopped. No P5-A/capability query, audio output, or session cleanup path was reached. The listener was stopped; no pending offer or active Receiver session remained.
- Usability follow-up from the user: reduce the manual mode/interface/listener/request steps and automate discovery/connection where possible, while keeping the first unauthenticated request subject to explicit acceptance.
- Hotspot follow-up after the listener was enabled: the phone's resent request appeared on the PC as an unauthenticated offer. After the user authorized acceptance, AnPod returned to its main screen showing **Authenticated · waiting for phone playback confirmation**, wireless UDP, and 0 bps. Discovery delivery and the authenticated connection handshake **PASS**; audio playback and capability-query verification remain **NOT_TESTED**. The installed Receiver remains code 4; the new code-5 review APK was not installed, and no desktop package was rebuilt.

### V3 hotspot request-window and Receiver bootstrap fix (2026-10-13)

- Source fix: the PC now gives each received unauthenticated connection offer a separate 30-second acceptance window; the 10-second search nonce window remains unchanged. This addresses the observed offer expiring before the user could accept it. The first request still requires explicit PC-side acceptance before P5-A.
- Source fix: UI-owned Receiver network preparation now waits up to the existing 120-second operation bound, while the ADB bootstrap path retains its 25-second timeout. The verified code-4 V3 Receiver artifact is recognized by exact digest and signer so code 5 can be offered as a same-signer update.
- Targeted Rust tests `selected_interface_listener_correlates_single_use_connection_offers` and `receiver_identity_compatibility_and_update_decisions_are_independent` **PASS**. Android `:app:compileDebugAndroidTestJavaWithJavac` and `:app:assembleDebug` **PASS**.
- Isolated review APK: `artifacts/v3-host-smoke-fix-review-2/receiver.apk`, package `app.anpod.receiver`, version `0.4.1-v3-candidate` / code 5 / targetSdk 35; SHA-256 `da69435dae3b521a02db030b8c4351ab2dde0af7df64062c3eed018124bb1e8a`; signer SHA-256 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`; metadata source commit `5c726672250413cf55da7e874a820fd1126e793c`.
- No phone reinstall or new desktop package was performed. Hotspot discovery after these fixes, authenticated P5-A/capability exchange, audio, and the shortened/automated connection UI remain **NOT_TESTED**. The user's request to reduce manual discovery/connection steps is carried forward; the first unauthenticated request remains explicitly confirmable.
- The user then confirmed that audio was audible on the phone. During that playback, the PC preview package built from source commit `840b457a664d8249e9f9b672b02ef708c55cced3` showed authenticated wireless UDP playback and approximately 1.66 Mbps PCM payload rate. The phone still had Receiver code 4 (`b52a326f…` installed artifact); this confirms the existing connection/authentication/playback path only, not the new 30-second offer window, code-5 installation, or real-device capability fields. No new acoustic latency or long-term stability conclusion was measured.

### V4 Receiver network update backend source checks (2026-10-14)

- P5-C authenticated update frames, bounded private staging, hash/package/version/signer validation, PackageInstaller handoff, status persistence, and host-side authenticated reconnect/completion confirmation are implemented in the backend. APK `versionCode` is 6. The update capability is advertised only when the Android endpoint can use PackageInstaller for this source.
- Targeted Rust checks: **PASS** (atomic concurrent reservation, exact compatible-version/transaction confirmation checks, Java↔Rust authenticated transfer/status/completion, commit ACK loss retained as pending, cancellation state, and staging cleanup). The merged endpoint uses the production authenticated parser/stager. Android receiver Java sources compile with JDK 17 against SDK API 35 `android.jar`: **PASS**. Actual Gradle `:app:compileDebugJavaWithJavac`: **PASS** after elevated local execution; no APK was built.
- Real V4 PackageInstaller install/confirmation callback, Android status persistence across process restart, and device-side reconnect/completion remain **NOT_TESTED**. A later attempt through the V3 preview package could not reach the stopped/unreachable Receiver service and left phone output disabled; it provides no playback result and does not change the earlier user-confirmed UDP audio result. No USB/bootstrap was triggered.

### V4 Receiver network update backend review fixes (2026-10-14)

- Fixed the PackageInstaller status PendingIntent mutability on API 23–30; moved APK verification and session streaming out of the manager state lock; definite synchronous commit failures now abandon the uncommitted session and clear staging, while accepted commits remain pending for Android's asynchronous result.
- Append failures now close and clear the staging transaction while preserving cleanup diagnostics, allowing a later update transaction. Foreground demotion checks the active owner/generation and current persisted update phase and uses the API 26 notification-channel guard.
- PC treats a successful COMMIT ACK as `Installing` until an authenticated status query reports Android's result; phase order prevents stale status rollback. Pending update confirmation is tied to its saved Receiver binding. `AppSnapshot` exposes `receiverUpdateBindingRef` alongside `receiverUpdate` so UI can hide another phone's update and avoid offering its cancel action.
- Targeted Rust update/binding checks: **PASS**, 4/4. Java↔Rust authenticated P5-C localhost check: **PASS**, 1/1; the production `transfer_update` path and status-query sequence `Installing` → `AwaitingUser` → `AwaitingReconnect` → `Completed` passed with exact target-version confirmation, plus failed-staging cleanup and retry. No Android source changed in this review-fix turn, so the previously passing Java/Kotlin compile is reused; no APK was built or installed. PackageInstaller callback, process-restart recovery, and real-device update confirmation remain **NOT_TESTED**.

### V4 Receiver update UI (2026-10-14)

- Desktop update candidate is read from the packaged, locally verified Receiver artifact and is returned only for the active authenticated network binding when the installed endpoint advertises compatible `apk-update-v1` and the candidate is newer. Start requires a review dialog; it explains playback stop, computer output restore, and Android system confirmation. Progress and cancel/stop-waiting actions use the real Receiver update snapshot and its binding. Main output, connection, device, and playback controls are disabled while that binding has a nonterminal update.
- Android Launcher displays the actual update phase, byte transfer progress, target version code, and system-reported errors. The install-source button appears only when PackageManager reports that this app cannot install packages; the system-confirmation button appears only when Android reported `AwaitingUser` and the PackageInstaller confirmation Intent is present. Neither system screen launches automatically.
- Historical note: during an earlier install attempt, the user replied **未看到** to a USB installation confirmation prompt. No installation prompt or confirmation was requested during this V4 UI review; no device or ADB action occurred. The V4 system confirmation and install-source settings flows remain **NOT_TESTED**.
- TypeScript project build and Vite production build: **PASS**. Android Kotlin/Java source compilation previously passed only with resource processing excluded and a temporary `R` stub; this does not count as a complete Android build. Rust IPC compilation is rechecked from the existing local cache for this review. No APK or desktop package was built in this phase.
- V4 UI review fixes: the saved update binding and its recovery action remain reachable after closing the manager; a completed update is not offered again at the same version; an update belonging to another binding cannot lock or cancel the current session. Candidate display and retry now use the active authenticated session even before a saved update record exists. Playback mode no longer shows the obsolete disconnect instruction, and output cards use icon-left/name-right layout.
- Existing frontend browser harness, scoped to mocked Receiver update snapshots and layout at 420×590 and 360×500 in light/dark themes: **PASS**; no page errors or horizontal overflow. This checks simulated UI only. It is not Android installer, USB prompt, real-device, or audio evidence. Final `tsc -b`, Vite production build, and cached offline Rust `cargo check --locked` all **PASS**; Rust reports 12 existing dead-code warnings. No Android source changed, no Android build was repeated, and no package was built.

### V5 Receiver observability and lifecycle source checks (2026-10-14)

- Added negotiated authenticated P5-C `PLAYBACK_PROGRESS_QUERY` (type 16), capability bit `0x00000008`, and the fixed 70-byte diagnostic extension after the unchanged 13-byte ACK. `protocol.md` now documents exact offsets, widths, frame units, unknown sentinels, capability gating, and the one-second host query cadence. Legacy endpoints receive no query.
- Connected protected USB health metrics to the same `playbackDiagnostics` AppSnapshot field as P5-C. USB host format comes from the current audio session; PC queue/drop fields remain unknown when not collected. Both paths compute underrun increment from cumulative count samples in the same session; first samples and counter resets are unknown. The Provider's historical `underrunIncrement` remains available but is not treated as that query-window delta.
- Receiver notifications now provide owner/generation-bound Stop during UI and ADB network bootstrap as well as active playback. `ReceiverUiOperationCheck` covers rejection of stale owner and stale generation Stop attempts. App notification taps retain the single-top MainActivity path; update notifications keep their existing system-confirmation/cancel behavior.
- `packaging/build-receiver.ps1` now guards the V5 candidate as `0.6.0-v5-candidate` / code 7. No package metadata/APK was generated in this source batch.
- Rust P5-C tests: **PASS**, 4/4 including Java↔Rust authenticated localhost; USB diagnostic merge/session interval test: **PASS**, 1/1; health parser unknown-value test: **PASS**, 1/1. JDK `ReceiverUiOperationCheck`: **PASS**. Android Gradle Java/Kotlin compile with real resource processing, no `R` stub: **PASS** in an isolated build directory. No APK was assembled.
- Real-device notification stop/stale-generation behavior, V5 metrics on phone, and lock-screen interaction remain **NOT_TESTED**. No phone, ADB, desktop AnPod UI, package installation, or audio routing was used in this source-only batch.

### V5 desktop playback diagnostics UI (2026-10-14)

- The existing secondary Diagnostics dialog now reads the single `playbackDiagnostics` field from `AppSnapshot`; it adds no polling path. It presents separate PC and Receiver formats, frame-count queue/drop data, AudioTrack pending/underrun values, and requested/accepted/effective/capacity/start-threshold/pre-roll buffers. Optional `null` values show `未提供`; measured zero remains `0`. Frame values are not converted to latency or combined across endpoints.
- The dialog labels diagnostics as the latest snapshot and distinguishes computer output, paused playback, Receiver update, and no active session from current phone playback. Missing UDP diagnostics explain that the Receiver may be old or not have negotiated the capability; connection/playback continue. No keys, transaction IDs, or local paths were added.
- Existing browser harness, one merged mocked-snapshot scenario at 360px: **PASS** for USB and UDP formats/queues, null versus zero, buffer labels, paused-state wording, unsupported old UDP capability, no-session wording, keyboard focus inside the modal, and no horizontal overflow. The inspected compact screenshot is `.tmp-frontend-check/receiver-diagnostics-compact.png`; the long metrics view scrolls vertically. This is simulated UI evidence only, not evidence of device playback or real Receiver values.
- TypeScript `tsc -b` and Vite production build: **PASS**. Rust and Android sources were unchanged; reuse the V5 Rust/P5-C and real-resource Gradle compile results above. No phone/ADB/PC audio operation, APK/desktop package build, or installation was performed; actual V5 metrics on the device remain **NOT_TESTED**.

### V6 PC wireless request flow UI (2026-10-14)

- The PC search area now reuses the selected connection interface and saved binding. A single available interface/binding/request is preselected; multiple items remain explicit. A request list, 30-second PC acceptance message, selected binding summary, and explicit **接受并连接所选手机** action are in the dialog's primary area. Search is never enabled automatically; an unauthenticated request is never accepted automatically. Existing backend offer acceptance and P5-A authentication paths are unchanged.
- Existing browser mock harness, one merged flow: **PASS** for single-item defaults, duplicate selector removal, listener not starting on open, valid selection retention and invalid selection revocation after refresh, automatic preselection without acceptance for one request, multi-request manual selection and refresh retention, expired request deselection, explicit acceptance payload, 30-second PC message, plus active-session and Receiver-update locks. Mocked browser only; real PC/phone discovery, 30-second wall-clock expiry, and audio remain **NOT_TESTED**.
- TypeScript `tsc -b` and Vite production build: **PASS**. No Rust, protocol, Receiver, or Gradle source changed. No APK or portable package was built; no real AnPod window, phone, ADB, audio, or network was operated.

### V6 本地候选构建与合并包审计（2026-10-14）

- 本地 Tauri 生产候选从源码提交 `88bc3d0b89589aebf85f8cefa96824c0718a696e` 构建，使用全新隔离目标 `artifacts/v3-v5-candidate-tauri-target`。`publish-tauri.ps1` 的 Cargo release 调用带 `--locked --offline`，其非零退出会直接抛错；流程已到达最终 ZIP 和 SHA-256 输出，故 Cargo 生产构建通过。包装器没有保留可单独读取的数值退出码。
- Receiver 候选 APK：`artifacts/v3-v5-candidate-receiver/receiver.apk`，源提交 `6c73a95d2a27b9e13db96572bae348d4f3434468`，`app.anpod.receiver`、code 7、`0.6.0-v5-candidate`、targetSdk 35、debug 签名；9,773,790 bytes，SHA-256 `d4a00b25ef5b023b9c2a2afc30763e2c9d99095077b9f767b349562fe1cf6381`，签名证书 SHA-256 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`。Android code 6 更新种子另存于 `artifacts/v4-v6-network-update-seed/receiver.apk`，源提交 `20f2389af0526ebdbc20924cad42570ce51110ce`，9,773,796 bytes，SHA-256 `c514dfd555aa2734178131db47037875cf1086d934b45639b799471297668be3`，同签名证书；该种子未放入 portable ZIP。
- Portable 目录：`artifacts/AnPod-1.0.4-receiver-v3-v5-candidate-review-tauri-win-x64-portable/`；ZIP：`artifacts/AnPod-1.0.4-receiver-v3-v5-candidate-review-tauri-win-x64-portable.zip`，17,852,741 bytes，SHA-256 `3cf78f40e4f14d50c1457525e5976e2783a05691e95ab560ff2c3f6d9da2fe25`。ZIP 有 31 个文件，与 staging 目录 31 个文件逐项 SHA-256 一致；build metadata 与文件实测一致。桌面 `AnPod.exe` 为 12,210,688 bytes，SHA-256 `1d5e493cf64a0c1226b12b2b5527d77fee9f181bb19bccc86aa0f8268232d31a`。
- Native `audio_capture.dll` 与 Legacy `server` 复用已审查源码提交 `608c98c75ec04e11555e9e547caa516c96d4e0dd` 的二进制，SHA-256 分别为 `eeda608983ef520f281d72c4f0021634b4e87c251ab83dea7f8dcb413068761e`、`9552af169e4c2b2ab82c39719d3afd7c2c5dd729551d8d487e836013eeb9cfa3`；ZIP 内副本哈希一致。
- 离线合并审计：归档路径安全检查、禁入文件名与本机私有路径扫描 **PASS**，未发现匹配项；APK 嵌套内容含 Android manifest 与签名元数据；`LICENSES` 16 个来源文件、`upstream` 2 个来源文件、`THIRD_PARTY_NOTICES.md` 均与仓库逐字节一致，无遗漏或额外许可证文件。`aapt` 确认 minSdk 23 / targetSdk 35；`apksigner verify --print-certs` **PASS**，证书摘要与 metadata 一致（工具另对未受 JAR 签名覆盖的 META-INF 信息文件发出警告）。
- 本条只证明本地构建和归档审计。此次代码 7 候选安装/启动、USB/UDP 实机连接与手机出声、code 6→7 Android 安装确认/更新恢复、锁屏通知生命周期仍为 **NOT_TESTED**；不将历史试听或旧测试音计为本候选验收。

### PC flow 精简候选包（2026-10-14）

- 包含已审查的 PC flow 提交 `5a614573c8d6ff8313e681b771a0baa10a9ae06f`；包装器仅将 V3 review 输出重命名为 flow 专属路径，并写入候选标识 `receiver-v3-v5-flow-candidate-review`。包装器改动提交 `ae99285f55d3623fefbcd3b86a1464a4142a27d6`；portable metadata 的 desktop `sourceCommit` / `buildCommit` 均为该提交。
- 输出目录：`artifacts/AnPod-1.0.4-receiver-v3-v5-flow-candidate-review-tauri-win-x64-portable/`；ZIP：`artifacts/AnPod-1.0.4-receiver-v3-v5-flow-candidate-review-tauri-win-x64-portable.zip`，17,852,857 bytes，SHA-256 `ff24fb5e7516d44fec855d7bd789882136ec9d0c17550bfa547e7235bddce94c`。桌面 `AnPod.exe` SHA-256 `fddc89e133f4482cb55d30eac6dc16ca0e9dbc4713b60957b8829bb7fb1ce5aa`，与隔离目标 `artifacts/v3-v5-flow-candidate-tauri-target` 中 release executable 相同。
- 首次包装器调用在 `apksigner.bat` 预检处因当前进程未设置 `JAVA_HOME` 停止，未进入 Cargo 构建，也未生成目标产物。使用已存在的 JDK/SDK 进程环境后，唯一一次生产构建退出码 **0**；Vite 构建 **PASS**（1,911 modules），Cargo release **Finished**（1m 38s，12 条非致命 dead-code warnings）。
- 复用的 Receiver code 7 APK 保持 9,773,790 bytes，SHA-256 `d4a00b25ef5b023b9c2a2afc30763e2c9d99095077b9f767b349562fe1cf6381`，来源 `6c73a95d2a27b9e13db96572bae348d4f3434468`，debug signer SHA-256 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439`；Android code 6 更新种子继续保留在 `artifacts/v4-v6-network-update-seed/receiver.apk`，未重建。Native/Legacy 复用 `608c98c75ec04e11555e9e547caa516c96d4e0dd` 的已审查产物，哈希不变。
- 最终归档 **31/31** 条目与 staging SHA-256 一致；ZIP 路径安全、禁入文件名与本机私有路径扫描 **PASS**，无发现；`LICENSES` 16 个来源文件、upstream 2 个文件及 `THIRD_PARTY_NOTICES.md` 逐字节一致。已存在的 V6 ZIP 未覆盖，SHA-256 仍为 `3cf78f40e4f14d50c1457525e5976e2783a05691e95ab560ff2c3f6d9da2fe25`。
- 本轮没有重跑已通过的合并 mock、Android 或旧协议测试；没有构建桌面包或安装 code 7 APK。此 flow 候选后续真实认证和音频实测见下方 R2 记录；系统 code 6→7 更新确认/恢复及锁屏通知生命周期仍 **NOT_TESTED**。

### Receiver 一键连接 UI（2026-10-14，待真机验收）

- 空闲且已 USB 绑定时，手机只需点击一次 **连接电脑**；验证服务快照进入同一 `operationId` 的 `prepared` 后，搜索只启动一次。通知权限缺失时仍须 Android 系统授权；拒绝授权时不启动服务或搜索。
- Receiver 已处于 `prepared` 时，点击主按钮应直接开始搜索且保持原 `operationId`；搜索中点击主按钮可停止搜索。打开或重开页面不会自动扫描。
- 搜索结果每一行都提供明确的 **请求连接此电脑** 操作；发现或选中候选本身不发送请求。发送后需等待 PC 明确接受，候选地址始终视为未认证输入。
- 验证多个可用网络接口初始不自动选择；单接口可预填。界面重开且服务运行时显示服务快照中的实际接口；已有效选择在刷新时保留。
- 验证已认证、播放/暂停、Receiver 更新期间不可再次发起连接；停止、启动失败或 Activity 离开/重开后，不会因旧连接意图自动开始搜索。服务失败文案优先于本地发送提示，并显示为错误。
- 源码编译：JDK 17 / SDK 35 隔离目录下，`:app:processDebugResources`、`:app:compileDebugKotlin` 和 `:app:compileDebugJavaWithJavac` **PASS**；Java 编译报告 deprecated API 提示。未 assemble APK。
- 本条真机验收步骤尚未在手机、PC 或 ADB 上执行；Receiver 一键连接、生命周期取消、网络接口恢复及音频均 **NOT_TESTED**。

### R2 无线认证与音频反馈（2026-10-14）

- 核实电脑上唯一运行的程序为已审查的 flow 候选：SHA-256 `fddc89e133f4482cb55d30eac6dc16ca0e9dbc4713b60957b8829bb7fb1ce5aa`，metadata 源码/构建提交 `ae99285f55d3623fefbcd3b86a1464a4142a27d6`。手机仍安装 Receiver code 6，签名为已核验的 `f5`，本次未重装。
- Receiver code 6 的已认证 P5-C `CAPABILITIES_INFO` 报告管理协议 3、音频协议 1、兼容、能力位 `0x7`。这证明已认证 INFO 及 `apk-update-v1` 能力已宣告；code 7 更新预检、传输和安装确认仍为 **NOT_TESTED**。
- 用户确认手机可听到声音，但低延迟模式有卡顿/破音。48 kHz 实测快照中的 effective/start/jitter 缓冲为 720/240/960 帧，欠载从 3973 增至 5254、6084；过期帧从 1,714,320 增至 2,182,080、2,470,560。用户反馈均衡模式有所改善但仍偶发；其快照为 2400/—/720 帧、34 次欠载、29,040 个过期帧。这不构成音质通过。
- 播放后 Receiver 报告 `healthy=false`、`audioActive=false` 和 `listener_failed`。本地 Java/localhost 检查复现了发现监听器初始化/会话结束恢复竞态；协调器修复后检查通过。修复尚未安装到手机复测；本源码批次未构建 APK 或桌面包。
### Code 8 本地候选（2026-10-14）

- Receiver 实际完整构建来源 `d7e5ed5c44d670f6a327ca6b8f04cf50339d7d63`，版本 `0.7.0-v3-flow-candidate` / code 8，minSdk 23 / targetSdk 35。唯一一次隔离 `assembleDebug` **PASS**（42 秒），包含真实资源、Kotlin、Java、DEX 和签名处理；APK 9,773,791 bytes，SHA-256 `6d631a768931367dbca3c6851584cd421fba10ab00dd03cba3a52d2d71d46baa`，候选 signer SHA-256 `f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439` 与已安装 code 6 连续。
- 独立 ZIP `artifacts/AnPod-1.0.4-receiver-v3-mobile-flow-candidate-review-tauri-win-x64-portable.zip`，17,855,967 bytes，SHA-256 `0126c9f40e3cbafc7fc82815a18029a1ca256151fa29c7a9b46ed41350239936`。组合包记录提交 `5237aeecd2269030b9ad796634962f65983b9ac2`；PC 实际生产来源仍为 `ae99285f55d3623fefbcd3b86a1464a4142a27d6`，EXE `fddc89e133f4482cb55d30eac6dc16ca0e9dbc4713b60957b8829bb7fb1ce5aa`，没有重新编译 PC。
- 已核对 PC 输入及原审核范围的 Native / AndroidServer / AndroidProtocol 输入未变。Native 与 Legacy 文件保持 `608c98c75ec04e11555e9e547caa516c96d4e0dd` 来源及原审核哈希；原 flow 和 V6 ZIP 哈希未变。
- ZIP 与 staging **31/31** 项逐字节一致，27 项与原包未变；只更新 APK、receiver.json、build-metadata.json、README。来源、版本、签名、APK 实际新界面/恢复代码、归档路径及禁文件检查 **PASS**；新文件与 APK 内部文件的私有构建路径扫描 **PASS**，未变许可/notice 复用原审核。
- 新候选尚未部署；网络 code 6→8 系统授权/确认、安装后认证版本证明、修复后的手机恢复/音质及两步 UI 真机操作仍 **NOT_TESTED**。旧 code 6 的出声和均衡改善不能代替新包验收；偶发卡顿仍未解决，未新增声学性能结论。

### USB / 一键连接候选真机反馈（2026-10-15）

- 本次为新的 usb-oneclick-review PC（EXE SHA-256 `9ade3a2a3ddf18565515d01ff6a3409563059a9c70a950a52dc1cfa6be54e3fd`）与 Receiver code 9；不沿用旧 code 6/8 的播放结果。
- 软件内 USB 6→9 升级核心路径 **PASS**：所选 USB 身份与 Android user 0 核验、安装后实际 code 9、APK SHA-256 `2b5ac4120d02ef46d8a96776d82c671622020357ce3e6b9739ce638f0bfd6cc5` 与候选一致，原绑定 saved，应用执行升级前后目标绑定核验。
- 首次手机开始接收后停在 prepared，PCM 为 0；PC 日志为 `P5-C peer closed during P5-A READY`，原因未定位。用户要求切换为手动验证后，重试确认连接、手机出声成功；成功会话未另采认证 INFO/播放指标，不能据此将连接可靠性或完整一键体验记为 PASS。
- 用户实听：均衡模式仍卡顿、破音，**音质 NOT_PASSED**；PC 连接逻辑混乱、操作顺序不明确，**交互目标未达成**。没有新声学性能结论。
- 当前版本低延迟、停止与电脑声音恢复、通知/生命周期、大字体/TalkBack继续 **NOT_TESTED**。用户决定本版本到此为止，后续改动另行规划；停止开发、排查、补测和派发，自动化保持暂停。

### USB Receiver 均衡声学短测（2026-10-15，用户专项授权）

- 用户自行选择 USB Receiver 后要求测量当前有线延迟，并确认手机扬声器靠近当前电脑麦克风、暂停其他播放、允许本地录音。真实状态为 Receiver code 9 / USB / Balanced / 48 kHz stereo，PC TCP 仅 loopback，手机非网络 AudioTrack，未切模式或调整音量。
- 既有扫频/QPC 回录工具：预检 2/2，中位数 194.931 ms；约 24 秒正式组 **10/10**，中位数 **172.160 ms**、P95 **172.688 ms**、范围 **171.523–172.800 ms**，匹配 0.936–0.945，正式匹配片段无采集断点或时间戳错误。第二脚本样点/时差复核一致。私有分析副本修正了近静音归一化伪峰，并通过静音/自相关检查；原始录音、原始和修正分析均保留。
- 正式组前后同一播放会话且 healthy=true；AudioTrack effective 2400、capacity/threshold 7688、pre-roll 2400 帧；欠载 **1→2**，TCP 丢积压帧 **960→960**。不能凭末次 underrunIncrement=0 认定整个测试无欠载。
- 结果含麦克风/驱动/声学耗时；未重测电脑基线，不是纯传输延迟，不与历史数值作受控收益对比。预检和正式组约 23 ms 差异及新增欠载保留，不宣布长期稳定或音质 PASS；本次低延迟模式未测。
- 本地证据 `.tmp-usb-receiver-latency-20261015` 不提交/打包。测试已结束并清理失效测量二进制，不改产品、不恢复自动化或旧长测，后续开发继续等用户规划。

### USB Receiver 低延迟声学短测（2026-10-15，用户专项授权）

- 用户自行切换模式后要求再测；核实 USB Receiver / code 9 / 实际 low-latency / 48 kHz stereo。复用上一组测量方法，音量/模式/产品参数未经代理修改。
- 预检 **2/2**、中位数 **124.567 ms**；约 24 秒正式 **10/10**、中位数 **125.651 ms**、P95 **126.977 ms**、范围 **121.438–127.353 ms**；相关匹配 0.921–0.934，正式匹配片段无采集断点或时间戳错误，第二脚本复核一致。
- 同一会话健康，effective 720、capacity 7688、threshold/pre-roll 240 帧；正式组欠载 **0→6**、TCP 丢积压帧 **4320→8160**。未声明这些计数必然对应扫频中可听故障，音质和长期稳定性不判 PASS。
- 与前次均衡中位数相差 **46.509 ms**，两组录放采集端点与格式一致；先后短测不构成严格受控收益评估。数值包含麦克风/声学耗时，没有重测基线，不是纯 USB 传输延迟。
- 证据仅在 `.tmp-usb-receiver-latency-20261015/low-latency`；测试结束、清理失效 EXE/OBJ、不改产品、不恢复自动化，后续改动仍等用户规划。
