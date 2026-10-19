# Phase 0 基线（Phase 1 开工前）

基线提交：`a406cedacb4939e9647d6046fca4f09cfefb84f9`（`dev`，2026-10-19）。这是 Phase 1 修改前的源码基线。下列开工前记录针对该提交；当前源码包含后续 Phase 1 改动，因此这些结果不能证明当前工作树通过。

## 已记录的自动检查

| 检查 | 开工前记录 | 备注 |
|---|---|---|
| `cargo fmt --check` | PASS | 基线源码 |
| 前端 TypeScript 检查 | PASS | 开工前记录通过；未保留确切命令行 |
| Vite 生产构建 | PASS | 开工前记录通过；未保留确切命令行 |
| `packaging/check-java-offline.ps1` | PASS | 基线源码，离线 Java 检查 |
| Rust 测试首轮 | 74 passed，15 failed，1 ignored，2 filtered | 12 个 P5-C loopback 失败与缺少 `JAVA_HOME` 有关；3 个 Binding `hard_link` 失败为 `Os { code: 5, PermissionDenied }`。之后重跑曾报告前 5 个 P5-C loopback PASS，但整套结果未取得；不据此声称 Rust 基线全过。 |
| Native x64 Release 隔离构建 | PASS | 14 个旧 ABI 导出齐全。依赖仅 `WS2_32`、`WINMM`、`ole32`、`AVRT`、`KERNEL32`。存在既有 C4819 代码页警告，未采集完整构建日志。 |

部分原始命令行和完整日志未保留；此处不补造命令或日志。

## Phase 1 相关基线与检查范围

- PC 版本 `1.9.0`，Receiver `0.16.0`、versionCode `33`。既有无线音频路径为 48 kHz PCM16；本阶段只增加配置和离线 v2 协商，尚不发送真实 PCM24。
- v1 配对、UDP 音频和控制路径属于兼容基线。Phase 1 的评审需核对 v1 字节编码、认证标签、密钥派生和既有向量保持不变。
- 旧实机前置核查按用户明确要求跳过；相关手机、声学输出、长期稳定性及 APK 实机升级状态为 **NOT_TESTED**。这只覆盖实机前置门槛，Phase 0 其余项目没有因此自动通过。
- 本阶段离线验证计划：Rust 格式检查、定向/全套离线 locked Rust tests（设置 `TAURI_CONFIG={"bundle":{"resources":[]}}`；排除 CI 中依赖真实设备或 PowerShell 工件的手动测试）、前端 TypeScript/Vite 生产构建、`packaging/check-java-offline.ps1`。测试如因 Windows 硬链接权限失败，应保留断言并在获准的测试进程权限下重跑，不得改弱测试。

## 可检验文件与风险

| 范围 | 主要入口 | 基线审查要点 |
|---|---|---|
| Rust 配置和状态 | `src-tauri/src/state/`、`src-tauri/src/main.rs` | serde 默认值和旧配置迁移；snapshot 字段兼容；状态不能把未协商能力报告成实际播放格式 |
| Rust 认证/网络协议 | `src-tauri/src/protocol.rs`、`src-tauri/src/network.rs`、`src-tauri/src/security.rs` | v1 bytes/labels 不变；v2 版本边界、认证后状态提交、重放与格式长度检查 |
| 前端设置 | `frontend/src/appTypes.ts`、`frontend/src/components/SettingsDialog.tsx`、`frontend/src/strings.*.ts` | 类型与状态 API 对齐；中英文设置项完整；只表示所选/协商状态，不宣称真实 PCM24 播放 |
| Receiver | `src/AnPod.Receiver/app/src/main/java/app/anpod/receiver/` | v1 pairing、P5-C 能力与 v1 UDP PCM16 通路未变；v2 解析不得隐式进入真实播放链 |
| Native ABI | `src/AnPod.Native/` | 本阶段不改旧导出 ABI；基线已记录 14 个旧导出 |

主要风险是协议或界面把“配置/模拟协商”误报为真实高精度播放，以及 v1 兼容回归。需独立检查 v1 原始编码与 Rust/Java 向量；设备输出与 APK 升级仍需后续实机阶段验证。

## Phase 1 集成验证记录

执行环境使用仓库已有 `.tmp-rustup-home`、`.tmp-cargo-home` 与 `.tmp-jdk17`；未下载工具链或依赖。为 Tauri 测试设置 `TAURI_CONFIG={"bundle":{"resources":[]}}`，Rust 测试采用 Windows CI 已有的两个明确排除项和单线程设置。

| 实际命令 | 结果 |
|---|---|
| `cargo fmt --check` | PASS；发现并仅格式化新增的 state 代码后复查通过。 |
| `cargo clippy --locked --offline --manifest-path src-tauri/Cargo.toml -- -D warnings` | PASS。 |
| `cargo test --locked --offline --manifest-path src-tauri/Cargo.toml -- --test-threads=1 --skip receiver::tests::p1_real_usb_installed_receiver_health_start_stop_when_explicitly_requested --skip receiver::tests::reads_the_artifact_written_by_windows_powershell_when_requested` | PASS：96 passed，0 failed，1 ignored，2 filtered。测试进程使用 JDK 17；硬链接测试在提升权限下运行，断言未更改。 |
| `packaging/check-java-offline.ps1` | PASS；包含 v1 回归、共享 v2 向量和 Java v2 离线握手/包测试，含取消后状态拒绝检查。 |
| `src/AnPod.Receiver/gradlew.bat --offline :app:compileDebugJavaWithJavac` | PASS；针对最新 Java 取消状态修复完成增量 JVM 编译。Gradle 报告本机 Android metrics 目录权限警告、platform-tools 未安装；编译成功，未构建 APK、签名或连接设备。 |

前端 TypeScript、Vite production build、设置页 Playwright 场景与截图检查由设置代理记录为 PASS；其具体命令行未保留。新增协议目前仅离线协商模拟；真实 PCM24、设备输出、声学表现、长期稳定性和 APK 实机升级继续为 **NOT_TESTED**。
