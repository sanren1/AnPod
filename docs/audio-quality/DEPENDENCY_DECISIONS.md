# 音频阶段依赖决策

更新时间：2026-10-19。此表针对 `plan_dev.md` Phase 1 及后续候选。Phase 1 只复用仓库当前依赖，不增加 crate、Android SDK 组件或拷贝第三方源码。此处记录采用边界，不代替某个具体版本的发行许可证复核。

| 组件/候选 | 决定 | 用途与边界 |
|---|---|---|
| 仓库现有 Rust、Tauri、serde、sha2、aes-gcm、hmac、hkdf、tokio、windows crates | adopt（现有） | 继续承担配置、序列化、认证和 Windows 平台能力；沿用现有 `Cargo.lock`，本阶段不升级。各依赖的既有锁定版本与许可记录以仓库清单为准。 |
| 仓库现有 Android/Gradle/Java 工具链 | adopt（现有） | 用于离线编译与解析检查；Phase 1 不增加播放 SDK 或 Native 音频依赖。 |
| Google Oboe | evaluate | 仓库有隔离探针及其记录，正式 Receiver 尚未采用。仅在 Phase 5 按真机数据评估；不能把探针存在视为产品依赖或证明性能收益。本轮未重新审查其具体版本许可证。 |
| Rubato | evaluate | Phase 4 在证明存在真实重采样需求后评估；当前不引入、不锁版本。 |
| SpeexDSP | reference-only | 作为 SRC 实现和质量/性能比较资料；不复制源码、不链接。 |
| libsoxr | reference-only | 仅研究候选；未引入。若将来考虑链接或分发，需先单独完成 LGPL 合规审查。 |
| ReaderWriterQueue | evaluate | 仅在 Oboe 阶段证明现有有界队列不足后评估；Phase 1 不需要并发音频队列。 |
| Microsoft WASAPI / WinUSB 示例与 Windows、Android 官方 API | reference-only | 作为平台接口文档和实现参考，不构成二进制或源码依赖。 |
| OboeTester | reference-only | 独立诊断工具，不进入 AnPod 产品依赖。 |

除仓库现有锁文件外，本轮未下载、安装、复制或新增第三方组件；也未对上述候选重新核实许可证。未来若从候选转为采用，需在引入前锁定具体 tag/commit，核对对应许可证及分发方式，并按 `plan_dev.md` 更新 NOTICE 与可复现构建说明。
