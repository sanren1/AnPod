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
