# 音频阶段依赖决策

更新时间：2026-10-20。Phase 1 的历史范围仍是只复用仓库当前依赖；后续阶段决议见下表。此处记录采用边界，不代替某个具体版本的发行许可证复核。

| 组件/候选 | 决定 | 用途与边界 |
|---|---|---|
| 仓库现有 Rust、Tauri、serde、sha2、aes-gcm、hmac、hkdf、tokio、windows crates | adopt（现有） | 继续承担配置、序列化、认证和 Windows 平台能力；沿用现有 `Cargo.lock`，本阶段不升级。各依赖的既有锁定版本与许可记录以仓库清单为准。 |
| 仓库现有 Android/Gradle/Java 工具链 | adopt（现有） | 用于离线编译与解析检查；Phase 1 不增加播放 SDK 或 Native 音频依赖。 |
| Google Oboe 1.10.0 | adopt（实验性 Receiver 后端） | Phase 5 正式 Receiver 增加可选 Oboe/AAudio Shared Float32 输出，AudioTrack 仍是默认后端。采用仅表示代码和依赖接入，不表示性能收益或实机声学验收；许可证与 APK 随包文本见 `THIRD_PARTY_NOTICES.md`、`upstream/SOURCE_VERSIONS.md`。 |
| Rubato | evaluate | Phase 4 在证明存在真实重采样需求后评估；当前不引入、不锁版本。 |
| SpeexDSP | reference-only | 作为 SRC 实现和质量/性能比较资料；不复制源码、不链接。 |
| libsoxr | reference-only | 仅研究候选；未引入。若将来考虑链接或分发，需先单独完成 LGPL 合规审查。 |
| ReaderWriterQueue | reference-only | Phase 5 使用 AnPod 自有的预分配有界 SPSC Float32 ring；未引入 ReaderWriterQueue。 |
| Microsoft WASAPI / WinUSB 示例与 Windows、Android 官方 API | reference-only | 作为平台接口文档和实现参考，不构成二进制或源码依赖。 |
| OboeTester | reference-only | 独立诊断工具，不进入 AnPod 产品依赖。 |

Phase 5 引入锁定的 Oboe 1.10.0 Prefab AAR；未复制其源码，AAR 的 Apache-2.0 许可证文本和归属说明随 Receiver APK 分发。Phase 5 未引入重采样器；其余候选仍按表中状态处理。未来若新增第三方组件，需在引入前锁定具体 tag/commit，核对许可证及分发方式，并更新 NOTICE 与可复现构建说明。
