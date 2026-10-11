# AnPod v2 P0 手动验收清单

## 基线与范围

- P0 基线分支：`codex/receiver-udp-v2`
- P0 基线提交：`00543589e641ae0911e660e433f55e08d4afcb6b`
- 回退基线 `main`：`8a757b4a84f5d25749b163c482dfb1262df206c0`
- 本阶段只做基线检查、构建验证和验收准备；未启动新的 AnPod 构建、未播放或录制真实音频。
- `AnPod_plan_v2.md` 是用户已有的未跟踪文件，本次保留未改。

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
| 断开连接与重连 | PASS（用户确认） | 用户确认断开与重连正常；不等于连续 20 次物理拔插通过。 |
| USB 物理拔插与设备重新出现 | NOT_TESTED | 本次未确认连续物理拔插或自动恢复的具体次数。 |
| 系统托盘退出 | PASS（用户确认） | 用户确认托盘退出正常。 |
| 系统托盘暂停、恢复 | NOT_TESTED | 暂停和恢复已确认，但未单独确认由托盘入口操作。 |
| PC 输出静音恢复 | PASS（用户确认） | 用户确认电脑声音可恢复。 |
| 切换播放模式 / 连接方式 | NOT_TESTED | 验证切换会自动断开并重连，状态提示与实际连接一致。 |
| 手机锁屏、短时网络中断和恢复 | NOT_TESTED | 记录恢复耗时；区分系统限制与程序错误。 |
| 长时间播放与设备睡眠/唤醒 | NOT_TESTED | 记录中断、恢复和音频异常，不以短测代替稳定性结论。 |

## 阶段审查结论

- P0 基线构建及相关自动检查通过；预览包的关键 USB、输出切换、暂停/恢复、断开/重连、托盘退出与电脑声音恢复经用户确认正常。
- 满足进入独立 P1 APK 可行性验证的门槛。P1 不改变现有播放路径，也不据此启用新 Receiver 音频。
- 连续物理拔插、多手机切换、托盘暂停/恢复入口、模式/传输真机切换、锁屏、长期播放与异常静音恢复仍未完整验收；不能将 P0 结论解释为全矩阵通过或 Wi-Fi 稳定。保持为后续集成及发布阻断检查。

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
| Rust 离线检查 | PASS | `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline`：22/22 通过，含 fake-adb 生命周期、安全拒绝及当前用户路径。 |
| Android 普通应用权限 Probe | NOT_TESTED | 无权限 Probe APK 已构建；Receiver 已安装，但普通应用调用 Provider/Bootstrap 的实际拒绝结果尚未验证。 |

### target 与兼容范围

- 真实构建目标是 targetSdk 35，不沿用早先 API 34 APK。Android 16 / API 36 设备只能覆盖该 APK 在 API 36 系统上的运行，不覆盖 targetSdk 36 专属行为。
- API 35 对音频焦点有 targetSdk 35+ 规则；P1 不请求音频焦点。P2 音频实现必须单独验证前台服务/音频焦点约束。
- Android 16 / targetSdk 36 的专属行为为 NOT_TESTED；API 36 目标构建尚未选择为 P1 目标。Android 12/14/15 真机也为 NOT_TESTED。
- Android 17 / targetSdk 37 的局域网权限兼容差异仍未解决；P1 不请求 `ACCESS_LOCAL_NETWORK`，也不启用网络接收。
- 锁屏后短时服务、force-stop 后状态及合法 USB 恢复路径尚未真机验证。短时服务不能作为 P2 音频服务方案。

### 真机记录

- 目标设备：Android 16 / API 36，省略型号和 ADB 标识；ADB 同时显示 USB 与 Wi-Fi 两条连接记录，使用 `adb -d get-serialno` 唯一确认 USB 目标。
- 安装前只读检查：当前 Android user 为 0，Receiver 未安装，旧 `com.anpod.server.Main` 音频服务进程为 0。没有启动或停止旧 AnPod 音频。
- 首次 install：**FAIL（历史记录）**。首次 `adb install -r` 返回 `INSTALL_FAILED_ABORTED: User rejected permissions`；用户当时反馈未看到 USB 安装提示。未绕过系统确认。
- 后续 install：**PASS**。设备解锁亮屏后，本机 APK SHA-256 与 metadata 一致且 `apksigner` 证书校验通过；对已确认的 USB 目标执行一次 `adb install -r` 返回 `Success`。Android 显示系统通知授权页，用户明确允许通知。未卸载应用。
- 真机生命周期（直接 ADB）：**PASS**。按当前 Android 用户读取到 `serviceActive=true`、`foreground=true`、`healthy=true`；显式 STOP 后三项均为 `false`。此次结果验证直接 ADB 入口，不代表本次 Rust 用户绑定修复已在手机上独立运行。
- 安装 APK 字节与随包 metadata SHA-256 匹配。普通应用权限 Probe：**NOT_TESTED**。修复后 Rust 管理路径的已安装生命周期真机复验仍待执行。
- 不执行 `pm grant`，不触发 `adb kill-server`，不为测试卸载已安装 Receiver。

### 候选包与回退

- 普通旧 Tauri 包只要求原有五个桌面运行文件和 Platform-Tools notice；Receiver APK/JSON 不再是 `tauri.conf` 或普通打包路径的强制依赖。
- 上一版候选包 `artifacts/AnPod-1.0.4-receiver-p1-candidate-tauri-win-x64-portable.zip` 的 SHA-256 为 `C6E3E9A6EB0627732EE6CACF24F5A03B269BA396E5E86E0CF6120AE2EA464B70`，包含 APK、JSON、README 图标 `icon.png`、原有桌面运行文件和 notices/licenses/upstream；该包早于当前 Rust 用户绑定修复，不能作为最新候选交付。待修复后的 Rust 已安装生命周期真机复验通过后再刷新候选包。稳定 ZIP `AnPod-1.0.4-tauri-win-x64-portable.zip` 未覆盖，SHA-256 仍为 `95D707F54C4037E4A7E8126D496FB9B2F1E2E31B6349ACEF4159E03B54870B9F`。
- 回退继续使用既有稳定 ZIP。Receiver 不自动卸载；若候选验收未完成，停止使用候选包并完成人工复测，不进入 P2。
