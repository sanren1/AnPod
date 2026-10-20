# AnPod 下一阶段最终开发规划

> 版本：1.1（工程执行基线；补充开源项目、官方指南及可复用实现调研）  
> 制定日期：2026-10-19  
> 代码基线：`c299c805f8917df0f961b5174dff47fb074517e6` 为历史调研基线；实际本地开发基线为 `dev@a406ced`，后续按 `dev` 提交推进并复核差异。
> 适用平台：Windows 10/11 x64；Android 6.0+；重点验证 vivo iQOO 11 / Android 16 (API 36)  
> 项目阶段：Phase 1 音质配置与 v2 离线协商模拟已完成源码与离线验收；Phase 2 Float32 WASAPI/PCM16 兼容路径已完成源码审查及 Rust/Java/Native 离线门禁，记录见 `docs/audio-quality/02-float-capture.md`。Phase 3 48k/24 与 Phase 4 条件式 96k/24 已完成源码及离线验收。Phase 5 Oboe 可选输出实现已通过源码审查和离线构建/JVM 验收；AudioTrack 仍是默认，Oboe 是需本地选择的实验后端。**真实 WASAPI capture、手机硬件输出格式/精度、声学/延迟/长时稳定性，以及 release 签名升级仍为 NOT_TESTED**；后续阶段推进不代表这些门槛通过。Phase 0 旧实机前置核查仍为 NOT_TESTED。历史调研基线 `c299c805f8917df0f961b5174dff47fb074517e6` 保留。

## 0. 开发指令与决策摘要

**核心目标**：在不改变现有无线 UDP 技术路径、不中断既有低延迟/均衡功能和安全配对的前提下，实现三个无损 PCM 传输音质档位；以 Float32 为高精度内部处理格式，保持 Java AudioTrack 兼容后端，实验性接入 Oboe/AAudio 输出，并在后续阶段正式接入已完成底层验证的 USB Direct (AOA)。

已经确定的产品决策：

| 维度 | 最终决定 |
|---|---|
| 无线传输 | **保留现有 UDP + AES-256-GCM**；本项目不引入 QUIC、WebRTC 或新无线配对方式 |
| 音质选择 | 三档：**标准 48kHz/16-bit、 高保真 48kHz/24-bit、Hi-Res 96kHz/24-bit**；双声道、无损原始 PCM |
| 32-bit 定位 | **Float32 用于内部采集/处理与必要的 Android 输出适配**；不提供 32-bit 整数传输档位；暂不将 F32 作为用户可选的第四个档位 |
| 延迟选择 | 保留独立的「低延迟／均衡」；与音质档正交，不自动绑定 |
| 默认兼容 | 老用户配置迁移后依旧为 **标准 PCM16**；切换到高保真需能力协商与真机验证 |
| Android 默认输出 | 保留 Java `AudioTrack`，优先支持 24-bit packed/Float32 路径；不允许无提示降成 16-bit |
| Android 实验后端 | `Oboe + AAudio Shared + Native Callback`；仅实验性可选，不预设比 AudioTrack 更快 |
| USB Direct (AOA) | **底层可行性已验证，确定后续集成**；先保留证据与独立探针，不在音质升级期间破坏 USB ADB |
| 未支持的能力 | 必须报告并安全降级到完整的**另一档格式**，禁止仅修改显示文字或在设备端无提示截断 |
| 发行管理 | 严格遵守既有 Receiver 签名、版本码、制品哈希白名单和 GPL/LGPL 义务 |

**重要限制**：更高传输位深/采样率不等于声学音质必定可闻地提高，更不等于更低延迟。Windows 的共享模式混音结果、Android 的实际输出格式及硬件 DSP 都可能是上限。严禁将 48kHz 音源软件升采样后标称为“原生 96kHz Hi-Res”。

---

## 1. 已验证事实、未验证范围和数据基线

### 1.1 USB Direct AOA：保留验证结果，未来整合

已在 iQOO 11 / Android 16 上使用独立工具完成：

- AOA `GET_PROTOCOL` 返回 **v2**；`SEND_STRING(52)` 和 `START_ACCESSORY(53)` 成功。
- 切换后 Windows 枚举 `VID_18D1&PID_2D01`；`MI_00` 通过 WinUSB 可用、`MI_01` ADB 正常、父设备 `usbccgp` 正常。
- 可访问 Bulk OUT `0x01` 与 Bulk IN `0x81`，MPS 512 B。
- 使用应用层固定 4096 B 测试帧：**16/16 + 64/64 校验通过**；16 轮 RTT P50 **3.29 ms**、P95 **10.31 ms**；64 轮 P50 **0.69 ms**、P95 **0.92 ms**。
- 旧回显帧 `seq=1` 曾造成误判，后续探针识别并丢弃完整合法旧帧；因此未来正式协议必须包含 session ID、sequence 和重连清理。
- 每轮出现一次 0 字节 USB IN read；应按 USB 短包/ZLP 可能性正确处理，**未证明成因**。

上述是**应用层回显 RTT**，不能视为单向音频传输延迟，也不能和手机声学播放延迟直接比较；未进行 PCM 连续传输、长时稳定性或 AOA 音频链路验收。

### 1.2 iQOO 11 Android 原生输出事实

- 系统 `android.hardware.audio.low_latency=true`、`android.hardware.audio.pro=true`；`aaudio.mmap_policy=2`、`aaudio.mmap_exclusive_policy=2`。
- OboeTester 在 Media、Game、指定或默认设备/采样率条件下，内置扬声器实际显示 `Legacy / MMAP NO / Exclusive NO`；**不把策略 AUTO 误读为 MMAP 已启用**。
- 48kHz、PCM16 Stereo、192 frames/burst（4ms）、384 帧设置缓冲（8ms）能运行，OboeTester 时间戳输出延迟估算约 **30–31ms**，所测截图中 XRuns = 0。
- 独立双引擎 CSV（前次实验）：Java AudioTrack P50 **30.10ms**、P95 **31.86ms**、累计 underrun 4；Oboe P50 **30.33ms**、P95 **32.19ms**、XRun 0。该数据只是**一轮非等效计数器的对照**，不足以证明 Oboe 具有延迟或音质优势。
- `HW format=I32` 不等于内置扬声器有效精度为 32-bit。

### 1.3 现有 AnPod 历史端到端基准（不得冒充新版）

README 中 **PC 1.2.1 + Receiver 0.9.1** 的旧声学短测中，无线 UDP 低延迟中位约 **187.394ms**、P95 **200.240ms**，USB/ADB 低延迟中位约 **123.269ms**。当前 `preview` 的音质、声学延迟和长期稳定性尚需重新测量。

---

## 2. 现有代码审查与影响范围

以下内容针对已读取的 `preview@c299c805`。开始实现前必须基于**最新**分支重新检查并执行差异分析，不应覆盖新提交。

| 现有位置 | 已确认的假设 | 必须修改之处 |
|---|---|---|
| `src/AnPod.Native/audio_capture.cpp` / `.h` | 采集固定 48000Hz / 2ch / 16-bit，C ABI `const int16_t*`；WASAPI shared-mode loopback 转换为该格式 | 保留原 ABI，新增 Float32 格式能力查询和新的回调/数据接口；正确报告 Windows mix/native 采集格式 |
| `src-tauri/src/audio.rs` | FFI `PcmCallback` 仅 `i16` | 扩展版 C ABI 与 Rust 音频 actor 的格式管理，绝不让 DLL 版本错配导致 UB |
| `src-tauri/src/network.rs` | `PcmQueue` 预分配 `i16`，`MAX_PAYLOAD_LEN` 打包 `(i16).to_le_bytes()` | 抽象预分配、帧单位的 PCM 源；3 字节 packed24 序列化；保持 v1 路径完全可回归 |
| `src-tauri/src/protocol.rs` | 1200B UDP datagram；34B header + 16B GCM tag = **1150B PCM 载荷**；FORMAT v1 编码 `1=PCM16LE` | 引入单独的协议/格式版本与样本表示协商；**旧 FORMAT 语义不变** |
| `src-tauri/src/security.rs` | 认证 `AudioParameters` 固定 7 字节，编码首字节 1 表示 PCM16；模式 0/1 | 新格式必须有新的认证绑定语义/协议版本，不能在不改认证 transcript 时私自追加新字段 |
| `src-tauri/src/p5c.rs` | 无线认证、音频参数、会话生命周期、状态传递 | 新旧能力协商、单会话格式锁定、升级/降级与状态传播 |
| `src/AnPod.Receiver/.../PairingProtocol.java` | 手机端对应 7 字节 PCM16 参数 | 与 Rust 逐字节一致的新参数协议；保留 v1 decoder |
| `src/AnPod.Receiver/.../UdpPacketDecoder.java` | FORMAT 必须首字节为 1，PCM16 假设 | 版本化 FORMAT/高位深检查，拒绝不合法长度与错位帧 |
| `src/AnPod.Receiver/.../UdpPcmQueue.java` | 多处 `frameBytes = channels * 2`，网络抖动队列 20/40ms 初值 | 帧数据格式抽象、时间基准不变、PLC/缓存、漂移相关的高精度适配 |
| `src/AnPod.AndroidProtocol/.../Pcm16FrameStream.java` | PCM16 帧、插/删帧、淡化，AudioTrack buffer 15ms（低延迟） | v1 保留；新建通用 PCM/Float32 漂移与转换组件，不用全局替换为 `*3` |
| `src/AnPod.Receiver/.../ReceiverService.java` | 仅 `ENCODING_PCM_16BIT`、`WRITE_BLOCKING`、源读 `InputStream`，目前 15ms 缓冲/5ms 启动阈值请求 | 可插拔输出端、PCM24/Float32 支持、实际格式查询、回退和运行时统计 |
| `frontend/src/App.tsx`、`appTypes.ts`、`strings.zh-CN.ts`、`strings.en.ts`、Rust 设置状态模块 | 已有「低延迟／均衡」UI，不含音质选择 | 增加独立三档音质和请求/实际信息，不重写现有模式控制 |
| `src-tauri/src/receiver.rs`、`packaging/*` | APK 精确哈希/签名验收与版本迁移 | 新 APK 仍用原签名，递增 versionCode，更新 audited hashes/构建记录 |

不属于本轮改造范围：改变既有 UDP 传输模式、重写设备发现、引入压缩音频编码、私改 AOD 开关控制、绕开 Android 系统音频权限与 HAL、自动更换 Windows USB 驱动。

---

## 3. 产品和 UX 规范

### 3.1 三档音质的精确定义

| UI 档位 | 传输格式（线缆/网络） | 双声道 PCM 裸码率 | 5ms 原始 PCM 大小 | 策略 |
|---|---|---:|---:|---|
| **标准** | PCM16LE / 48,000Hz / 2ch | **1.536 Mbit/s** | 960 B | 兼容、既有用户默认 |
| **高保真** | **Packed PCM24LE** / 48,000Hz / 2ch | **2.304 Mbit/s** | 1,440 B | 优先完成；建议支持设备可选 |
| **Hi-Res** | **Packed PCM24LE** / 96,000Hz / 2ch | **4.608 Mbit/s** | 2,880 B | 能力受限，严格如实标注 |

- PCM24 在网络线上采用 **signed little-endian、每样本恰好 3 字节**；不把 32-bit 容器误当作 32-bit 有效精度。
- 不提供 PCM32 整数档；Float32 为**内部数值表示**，不是对用户承诺的“32-bit 数字音质”。
- 优先支持 48kHz / 24-bit。96kHz 不应作为对所有手机自动启用的默认值。
- 标准和高保真档都按 **48kHz 双声道**设计，不影响「低延迟／均衡」的缓冲策略。
- 质量设置是“**请求的传输格式**”；UI 必须另有“**实际生效传输格式**”、“Windows 系统混音格式”及“Android 实际输出格式”。
- Windows 端的 24-bit 格式表示网络量化上限，不构成**原始内容达到 24-bit 精度**的保证；系统混音可能已降精度。

### 3.2 Windows 设置界面

新增独立的 `传输音质` 单选区：`标准 · 48 kHz / 16-bit`、`高保真 · 48 kHz / 24-bit`、`Hi-Res · 96 kHz / 24-bit`。沿用现有 Tauri UI 尺寸、强调色和国际化，不引入复杂参数面板。

要求：

1. 保持现有「播放模式：均衡／低延迟」原位独立选择。切换音质**不改变**播放模式、音量、连接方式。
2. 显示“请求：高保真 / 实际传输：48kHz PCM24 / PC mix：48kHz Float32 / 手机输出：48kHz Float32”；如不支持，应解释是哪一层限制。
3. 老配置字段缺失：填 `Standard`，不改变既有表现；新安装初版默认也采用 `Standard`，能力验证稳定后方可考虑调整新装默认值。
4. 高音质不可用时明确显示失败原因，并允许用户选择标准档后重新连接；绝不在 UI 标为高保真、实际传 PCM16，也不在同一会话静默改格式。
5. 正在播放时改变音质：预协商新会话并**有序停止—切换—启动**；旧会话不得混用新 PCM；失败返回原档并恢复原会话（恢复失败给出可操作错误）。
6. 手机 AOD 仅扩展显示**实际生效音质**（如不挤占原有指标可选）；不增加 AOD 开关，也不在 PC 端控制手机 AOD。
7. UI 不将 96kHz 软件升采样等同于源内容 Hi-Res；若显示采样率不一致，明确标注 `已重采样`。

### 3.3 设置模型（建议）

```text
AudioQualityProfile = Standard | HiFi | HiRes
PlaybackMode        = Balanced | LowLatency
Transport           = WirelessUdp | UsbAdb | UsbDirectAoa[future]
OutputEngine        = AudioTrack | OboeExperimental

NegotiatedAudioFormat:
    sampleRateHz: 48000 | 96000
    channels: 2
    sampleFormat: Pcm16Le | Pcm24PackedLe
    validBitsPerSample: 16 | 24
    bytesPerSample: 2 | 3
    codecVersion: v1 | v2
    sourceMixFormat: {rate, representation, validBitsIfKnown}
    deviceOutputFormat: {rate, representation, effectivePath}
    conversionFlags: [resampled, requantized, outputConverted]
```

不得把 `sourceMixFormat`、`deviceOutputFormat` 当成安全握手中的客户端自我声明能力。协商与遥测分离，安全敏感字段受认证完整性保护。

---

## 4. 总体技术架构

```text
Windows
  │ WASAPI loopback（查询实际 Windows 混音格式）
  ▼
格式适配器（原生采样 -> Float32 stereo；保留有效精度信息）
  │  可选高质量重采样 / 防溢出 / 有界漂移处理
  ▼
预分配 Float32 PCM 队列（时间戳 + sampleFrameIndex；严格有界）
  ▼
PCM Profile Encoder
  ├── 48k/16 PCM16LE（旧版 v1 兼容）
  ├── 48k/24 PCM24 Packed LE（新 v2）
  └── 96k/24 PCM24 Packed LE（新 v2，条件启用）
  ▼
统一 Transport Sink（先保留 UDP；USB ADB 逐步适配；未来 AOA）
  ▼
Android Transport Ingest（认证 / 解码 / 校验 / 去重 / 有序帧）
  ▼
Jitter/Backlog/PLC/Clock-Drift Adapter（按样本帧数而非固定字节数）
  ▼
Float32 或高精度 Native PCM 队列（仅 1 层设备输出桥接，不叠加目标时延）
  ├── AudioTrack（正式默认；输出 PCM24 packed 或 PCM_FLOAT；性能不合格时安全回退）
  └── Oboe/AAudio（实验；Shared/Legacy + callback，经过验证后才可自动启用）
  ▼
Android Audio HAL / DSP / 内置扬声器
```

实时路径原则：严禁在 WASAPI 回调和 Oboe `onAudioReady()` 中进行磁盘写、阻塞 I/O、加密计算、JNI 往返、互斥锁等待、内存分配或网络接收。发送、解密、PLC 和重采样在专用工作线程；音频回调只消费预分配环形缓冲。

---

## 5. Windows 高精度采集与 DSP 方案

### 5.1 采集的真实性

1. 初始化前先调用 `IAudioClient::GetMixFormat`，记录共享音频引擎的混音样本格式、容器位数、有效位数（若提供）及采样率；必要时 `IsFormatSupported` 查询目标格式。
2. 推荐以 Windows **实际共享混音格式**获取 loopback 数据；若其为 Float32，直接在 Float32 表示中处理。若设备返回整型 PCM，使用正确有效位深转换到 Float32（包含符号扩展与声道映射）；不可把 16-bit 数据左移/补零并称作真实 24-bit。
3. 不默认把 Windows 系统输出采样率改为 96kHz；不自动修改默认音频设备、音量、增强或独占设置。
4. 若请求 48kHz，源 mix 并非 48kHz，则使用具有可测试频响、抗混叠和有界延迟的重采样；显示转换标记。
5. **Hi-Res 96kHz 处理规则**：Hi-Res 可作为实验请求选项，但只有 Windows mix 实际为 96 kHz、且 Receiver 支持所选的准确 descriptor 时才建立该档会话。mix 低于 96 kHz 时明确报告当前格式并要求用户选择其他档位后新建会话；不升采样并冒称原生 Hi-Res，也不在同一会话静默改采样率。若未来加入用户主动选择的升采样实验，须明确标注 `96kHz（升采样）`。
6. 即使 Windows mix 为 96kHz，也可能播放源本身只是升采样内容，因此 UI 只声称 `Windows 混音采样率 96kHz`，不声称识别到音乐文件的原始采样率或 bit-perfect。

### 5.2 Float32 内部流程

- 使用 little-endian IEEE 754 `float32` 仅作为内部处理容器；值域标准化 `[-1, 1]`（必要的有限值检查、NaN/Inf 处理）。
- Float32 是在维持适度计算成本下的处理精度折中；无需自造 32-bit 整数 PCM 传输。
- 转换至 PCM16/24 采用可单测的对称饱和、四舍五入和明确的负数边界规则；避免溢出和 DC 偏置。
- 当将高精度/Float32 缩窄为整数格式时，设计 TPDF dither 的开关与验证；不能给数字静音不受控地加上可听噪声。是否在最终版本默认启用，以量化噪声/失真测试为准。
- 漂移校正优先放在 Float32 域，**不允许现有 PCM16 帧插删算法在高保真链路中隐式往返 16-bit**。初版可对比现有校正算法与异步重采样（ASRC），选择可证明稳定且误差小的方案。
- 采集快路径使用预分配 block pool；最大块、队列时间上限与内存上限需要单测，严重拥堵丢弃过期音频而不是无限累积。

### 5.3 C / Rust ABI 与回退

- **不直接改变现有 `PcmCallback(const int16_t*,...)` ABI**，保留现有有线/老版本调用者。
- 推荐新增版本化 `AudioCapture_GetCapabilitiesV2` / `AudioCapture_SetFloatPcmCallbackV2` / `AudioCapture_GetCaptureFormatV2` 等独立接口；名称为设计建议，实施前须与导出符号表及 Rust 动态加载策略核对。
- V2 ABI 结构体必须包含 `size` 和 `version`，固定整数位宽、明确缓冲所有权及回调生命周期，避免 DLL 版本不一致时访问非法内存。
- `src-tauri/src/audio.rs` 在启动前检查 V2 capability；旧 DLL 仅允许现有 PCM16 路径，不能强行使用 Float32 回调。
- `AudioCapture_Stop/Close` 后先注销回调并等待相关采集线程退出，然后释放回调上下文。保留原始 native 采集错误码。

---

## 6. UDP 格式、会话协商与网络安全

### 6.1 保持的协议不变量

- **维持 UDP**、设备发现、现有配对/绑定和 AES-256-GCM；不改 UDP 默认 MTU 上限 `1200 B`，不依赖 IP 分片。
- 现有 v1：34 B 固定报头、16 B GCM tag、最大 **1150 B** 音频载荷；PCM16LE 格式编码值 `1` 原样保留。
- **v1 数据包、认证 transcript、AudioParameters 7 字节均不可因新增音质设置而改变。** 旧 PC ↔ 旧 Receiver 在标准档的行为应逐字节兼容。
- Frame Index 始终表示**每声道采样帧序号**而不是字节偏移，包重排/PLC/漂移处理必须保持该语义。
- 新格式不能在 GCM 认证之外明文声明并信任。用户请求的档位与**实际协商的格式**进入新会话认证 transcript，避免攻击者篡改成低品质或不同编码。

### 6.2 版本兼容方案

必须实现**并行 v1 / v2**，禁止冒险地在 v1 上附加新字段：

- **v1**：只用于 PCM16LE、既有 7 字节 AudioParameters 与原有 FORMAT；保留原入口、解析、单元测试和安全属性。
- **v2**：新增显式协商的采样格式（如 `PCM24_PACKED_LE`）、样本率、通道、实际载荷每帧字节数、可选能力位；独立版本号和消息长度；身份验证、会话密钥衍生与新 transcript 必须覆盖全部格式字段。
- 使用现有受信版本/认证控制链路决定是否允许 v2；不能仅相信未经认证的广播/网络发现信息。
- 连接到只支持 v1 的 Receiver：用户请求高保真或 Hi-Res 时明确报不支持，由用户选择标准档并重新建立干净的 v1 会话；不自动改档，也禁止在发送加密音频后突然改变字节表示。
- 任何未识别的版本/codec、非法组合、帧不对齐、超长或错序会话应明确拒绝，且不影响同机其它绑定设备。
- 后续需要由 Rust/Java 双端的已知十六进制测试向量覆盖：握手字段、AAD、nonce、密钥派生、认证错误、重放窗口、格式变更、丢包与边界大小。

### 6.3 UDP PCM 分包精确规则

| 格式 | 每样本帧字节（2ch） | 5ms 音频块 | 每包最多完整帧（载荷≤1150） | 5ms 所需包数 |
|---|---:|---:|---:|---:|
| PCM16 / 48k | 4 | 240 frames / 960 B | 287 frames | 1 |
| PCM24 / 48k | 6 | 240 frames / 1440 B | 191 frames | 2 |
| PCM24 / 96k | 6 | 480 frames / 2880 B | 191 frames | 3 |

- 继续按约 **5ms 逻辑音频时长**生成数据；发送时按 `floor(MAX_PAYLOAD_LEN / frameBytes)` 拆成 **连续、可独立认证和独立播放的小数据报**，其中每个包都有正确 `sampleFrameIndex` 与独立 GCM nonce/sequence。
- 不能以拆分 5ms 块为理由增设完整块重组等待；接收端可以在包到达后依采样帧索引直接入队，减少额外延迟。
- 禁止在包丢失时等待超长重传；沿用有界 Jitter/PLC，维护正确的 frame index 和 PCM24 帧对齐。
- 发送速率不得依赖写入完整块后的同步应答；USB AOA 后续亦为持续单向 PCM，不采用探针中的逐块回显机制。
- 所有网络测量分别报告 payload 码率、总数据报数、认证失败、乱序、缺帧和音频 underrun；不要把高位深增加的带宽当成“音质已经提高”的证据。

---

## 7. Android 解码、音频队列与输出

### 7.1 数据模型与缓冲

- 为新格式新增按 `sampleFrameIndex + frameCount` 计算的通用 PCM 队列，使用样本格式描述符；**每帧字节数由格式计算**，不再写死 `channels * 2`。
- 新旧 decoder 并存；网络线程解密、解包、顺序检查并交付有界缓冲。队列存储可为 PCM24 packed 或 Float32，但在变换前需有明确的精度账本（避免 24→16→Float 的暗中降级）。
- 低延迟初值（原 20ms）、均衡初值（原 40ms）先沿用现有参数作为基线；**不因换格式自动增加缓冲毫秒数**。时间预算而不是字节数决定上限。
- 对 PLC、淡入淡出、时钟漂移/补偿提供同格式或 Float32 实现与回归测试；高位深链路不得调用 PCM16-only 的老处理代码导致截断。
- 保留会话暂停/音频焦点/突然断线后的 flush，清理旧 session 帧及 96k→48k 重采样器状态，不允许混会话旧数据进入新输出流。

### 7.2 Java AudioTrack 正式后端

- 继续保持 Java AudioTrack 为默认稳定后端，并沿用 `AudioAttributes.USAGE_MEDIA` 与既有音频焦点处理。
- PCM16 继续 `ENCODING_PCM_16BIT`；**PCM24** 首选 Android 12+ `ENCODING_PCM_24BIT_PACKED`（仅当探测支持并实际可打开）；不支持时优先 `ENCODING_PCM_FLOAT` 并由 Float32 路径转换，**不允许无提示截成 PCM16**。
- Android API 较旧或设备高精度轨道初始化失败：协商更低一档并重建会话（或者明确显示设备输出不得不降级），而非让手机隐式执行不可观测的 16-bit 音质回退。
- OboeTester 已表明 iQOO 11 内置扬声器多次实际工作在 48kHz、非 MMAP Legacy 路径；96k PCM 传输不等于输出能够保持 96k。对于 96k 传输 + 48k 实际输出，必须报告**Android 输出重采样到 48kHz**及其开销，允许用户回到 48k/24。
- `AudioTrack.setBufferSizeInFrames` / `getStartThresholdInFrames` 请求值和实际生效值均记入诊断；运行中 buffer 变化、欠载与队列水位可见。

### 7.3 Oboe 实验性后端

- 后端采用 `Oboe + AAudio Shared (Legacy) + callback` 作为 iQOO 11 当前现实基线，不以 MMAP/Exclusive 为前置条件；其它设备可在严格运行时检测后尝试更低延迟路径。
- 基于 48kHz / 192 frames per burst / 2 burst 起步，较小 buffer 若出现 XRuns 则允许自适应增长。
- Native SPSC ring 是设备输出的**薄桥接**，必须和上游 Jitter 预算共管，不可以额外叠加不可控的 10–20ms 缓冲。
- `onAudioReady` 只填充预分配输出帧；无锁、不做 UDP/USB 读取、不做 AES-GCM、JNI、I/O 或 malloc。Native 工作线程做 Float32 混音/必要重采样与幅度控制。
- 发生设备切换、焦点变化、流断开、实际格式不符、持续 XRuns 或恢复失败时，停止并重建流；实验模式允许安全回退 AudioTrack，不在 audio callback 内直接重启。
- **不根据先前 CSV 直接改变默认引擎**：两种 API 旧测试延迟约 30ms，Oboe 零 XRun 只是进一步验证理由。

---

## 8. USB Direct（AOA）后续正式集成：独立阶段

**本阶段的决定是确定引入，但音质升级和 Oboe 尚未验收前不得破坏 USB ADB 稳定性。**

### 8.1 复用架构

Windows Float32 PCM / Profile Encoder → `TransportSink` → **WinUSB Bulk OUT 0x01** → Android USB Accessory Reader → 统一 PCM 队列 → AudioTrack / Oboe。无线 UDP 和 USB AOA 共用声音格式、样本帧索引、缓冲及统计定义，但可以使用不同可靠性/封包层。

### 8.2 连接与驱动安全

- AOA 控制阶段必须使用只针对目标设备的 `GET_PROTOCOL 51 / SEND_STRING 52 / START_ACCESSORY 53`，由用户首次明确确认可能导致 ADB/MTP 临时断开。
- 预期设备 `VID_18D1 PID_2D00/2D01`，优先支持已验证的 `2D01`；数据接口 `MI_00` 使用 WinUSB，ADB `MI_01` 不得替换，父设备 `usbccgp` 不得替换。
- 发现接口须读取 Zadig 安装注册的 GUID，不能以空 GUID 枚举；不能自动安装/篡改现有 ADB/MTP 驱动。
- Android Accessory 权限通过官方 `UsbManager` 和用户授权，先安装 Receiver 再切换 AOA；首次支持可要求人工操作，不承诺 PC 单方面静默完成所有过程。
- bulk streaming 与探针回显相互隔离，**不能把“4096 B echo RTT 0.69ms”写成音频单向延迟**。
- 新增断线、WinUSB open 失败、设备 re-enumeration 超时、权限被拒、拔插 USB、误触重连的恢复路径，允许退回 USB ADB；始终不导致原有配对被意外清除。

### 8.3 分阶段

1. AOA 单独 PCM16 单向连续传输原型，采样率 48k，借用现有 Android AudioTrack。
2. 通过帧协议及缓冲统计后，复用本规划已验证的 PCM24 48k；最后考虑 96k。
3. 进行与 USB ADB 相同手机、声音内容和输出后端的**声学延迟与稳定性比较**。只有证据足够时才考虑切换默认 USB 协议。

---

## 9. 统一诊断与真实音质表示

应显示且记录：

| 范围 | 必须记录 |
|---|---|
| Windows | Windows 默认输出设备、WASAPI mix rate、source sample representation/valid bits（若可知）、实际 capture rate、转换/重采样标记 |
| 连接与音频 | 用户请求 profile、实际 negotiated profile、协议版本、PCM bytes/frame、传输类型、包大小、payload rate、每秒包数 |
| Android | Android Receiver 版本、输出 engine、请求/实际 sampleRate/encoding、实际缓冲容量/大小、start threshold、burst、采样时间戳估算延迟、欠载 |
| 无线 | 到达抖动、Jitter target/actual queue、丢帧/认证失败/乱序、PLC/插补帧、播放断音、连通性和重建 |
| USB Direct | AOA VID/PID/MI、WinUSB 服务、端点、会话、USB 读写错误和重新枚举时间，不收集任何无关手机私密信息 |
| 精度 | `sourceMix=48kHz Float32`, `wire=48kHz PCM24`, `output=48kHz PCM_FLOAT` 等分层状态，不能简单展示“Hi-Res 96/24”掩盖中间转换 |

数据分为三类：**格式/链路能力**、**软件时间戳估算**、**外部声学测量**。严禁将 RTT 估算值或音频时间戳估算值标为麦克风测得的真实声学发声延迟。

默认不记录原始用户音频数据、网络密钥、绑定密钥、设备敏感标识、私有地址；诊断导出需要脱敏和用户明确操作。

---

## 10. 分阶段实施顺序（Agent 必须按顺序交付）

### Phase 0 — 代码冻结、风险审计和基线回归（必须首先完成）

**任务**：拉取最新 `preview`，确认版本与已审查基线差异；备份既有测试结果，冻结 v1 golden vectors；自动化测试所有现有标准/无线/USB ADB 基础工作流。分析 C++ 导出 ABI、Rust status settings、Java pairing、Receiver 签名打包链。

**交付**：`docs/audio-quality/00-baseline.md`、兼容测试列表、风险清单。**通过门槛**：项目现有构建及测试通过；新的分支开发不影响 `preview` 默认路径。

### Phase 1 — 音质配置和 v2 协商协议（不接真实 PCM24）

**任务**：配置枚举、UI/i18n、能力协商和格式 descriptor；新旧协议并行的 Rust/Java 解析器与 golden vectors；新增状态 API；模拟数据打通 v2 协商。

**交付**：字段规范、跨语言 codec 测试、兼容矩阵。**通过门槛**：老 v1 无变更，故意构造的无效/降级/重放测试被拒绝。

### Phase 2 — Float32 WASAPI/PCM16 无回归（先不启用 24-bit）

**任务**：读取 Windows mix 格式，实现独立 V2 Float32 采集处理路径及安全 DLL ABI；保留旧 PCM16 回调；float→PCM16 测试；新内部队列与预分配处理。参考第 14 章 Microsoft WASAPI 示例的事件驱动采集、设备失效与缓冲读取循环；只借鉴相关模式，不直接更换当前采集组件。

**交付**：Float32 capture trace 和 bit-exact PCM16 老回归。**通过门槛**：旧功能播放、延迟、稳定性、录音精度没有异常回归；不覆盖原签名。

**源码/离线状态（2026-10-20）**：FloatV2 ABI、Native capture thread、Rust 动态符号/格式检查、独立预分配 Float 队列与 PCM16 量化纯函数已完成源码审查。Native ABI 离线测试、Rust 103 passed/0 failed/1 ignored、Java 离线协议回归通过；详情及边界见 `docs/audio-quality/02-float-capture.md`。测试使用 DLL/合成内存数据，没有运行 WASAPI 初始化/Start 或真实设备，因此没有产生真实 Float capture trace；设备切换、旧 PCM16 声学精度、播放/延迟/长时稳定性仍为 **NOT_TESTED**。源代码与离线门禁可以继续进入 Phase 3 开发；Phase 2 实机验收保持未完成。

### Phase 3 — 高保真 48kHz / 24-bit 端到端（最优先的实用增量）

**任务**：PCM24 packed encode/decode + 按 ANPD v2 最大 1150 B payload 进行帧对齐分包、Android 24-bit/Float32 输出通道及高精度 PLC/漂移处理；UI 状态与格式不支持时的明确失败提示。当前不静默切换格式；Standard/HiFi 变更由用户选择并重连。48 kHz 双声道 PCM24 每帧 6 B，每包最多 191 帧；240 帧按 191+49 分包，480 帧按 191+191+98 分包，不要求固定 5 ms 采集块。

**交付**：48/24 真机可用版本、格式一致性 golden vectors、30–60 分钟实测。**通过门槛**：从 PC 采集到 Android 输出前无暗中 16-bit 量化；无新增不稳定行为。

**Phase 3 离线状态（2026-10-20）**：P5-C coordinator 加入仅查询的 v1 `0x82` 能力探测、等待 EOF 后的新 v2 会话、认证 descriptor 锁定、既有连接意图/后台授权复用、格式级 BIND 门控和 78-byte 实际输出进度；该阶段产品格式仅为 48 kHz stereo packed PCM24。JDK 17 P5-C 与 discovery-recovery 检查、Rust↔Java fresh-session/拒绝及两代 owner pause/恢复测试通过；当时 Android Receiver JVM 44 项及 Java、Frontend 离线检查通过。Phase 3 的真实设备验收仍未完成。

### Phase 4 — Hi-Res 96kHz / 24-bit（设备能力有条件启用）

**任务**：在设置页提供明确标为实验的 Hi-Res 请求；仅 Windows mix 实际为 96 kHz 且 Receiver 支持精确 profile-3/96 kHz/PCM24 descriptor 时建立会话。48 kHz source 不升采样冒充 96 kHz；mix 率或 Receiver 能力不符时明确失败，由用户选择其他档位并新建会话。按 ANPD v2 payload 上限进行帧对齐分包：96 kHz 双声道 PCM24 每包最多 191 帧；480 帧（5 ms）按 191+191+98 分包。descriptor 必须与已认证的选定格式完全一致；不在同会话静默改率。只有证明确有重采样需要、且延迟/频响/失真已测量后才按第 14 章评估 Rubato/SpeexDSP；先保持 48k/24 直通。

**交付**：真实 Windows mix 为 96k 时的链路报告；48k→96k 的误导性 UI 被禁止。**通过门槛**：实际采样率真实展示，必要重采样的延迟、频响/失真和稳定性经测试；不支持时提示并由用户选择启动新的 48k/24 会话，不在已认证会话内改格式。

**离线开发状态（2026-10-20）**：设置页开放实验性 Hi-Res 请求；只有实际 Windows mix/capture 为 96 kHz 且 Receiver 接受精确 profile-3/96 kHz/packed-24 descriptor 时才开始会话。48 kHz mix 不升采样冒充原生 Hi-Res；整数或多声道 mix 可由 Native 转成同速 stereo Float32 capture，UI 仍显示 mix 实际格式与当前会话实际传输/手机软件输出格式。unsupported/mix mismatch 明确失败，需用户选其他档并新建会话；不在同一会话改率。96 kHz PCM24 Rust↔Java fresh-session/UDP/Q23 低位 loopback、profile3 owner pause-stop-mode fresh-session、Java offline checks、Android Receiver JVM **47/47**、Rust fmt 与 normal strict Clippy 均通过。锁定 offline 单线程 Rust 全量门禁 **114 passed / 0 failed / 1 ignored / 2 filtered**（93.89 秒）；两项人工过滤为 PowerShell artifact 检查和显式真机 USB Receiver 检查。Frontend production build 与目标 Playwright 场景通过，覆盖 Hi-Res 请求、未知与实际格式分离、明确错误、窄屏/键盘。真实 96 kHz WASAPI capture、实际设备 AudioTrack 96 kHz 打开、物理输出精度、声学测量和 30–60 分钟稳定性仍为 **NOT_TESTED**；本轮没有引入重采样器。

### Phase 5 — Oboe Native 可选输出后端（与音质档兼容）

**任务**：抽象 `PlaybackOutput`，提供默认 AudioTrack 与本地设置选择的 Oboe/AAudio Shared Float32 实验后端；用预分配 SPSC ring，并将 ring 与实际 stream buffer 放入既有 15/50 ms 总预算。请求格式不匹配或首次打开失败时同格式使用 AudioTrack；运行中 Oboe 错误 fail-close 当前 audio generation，并在下一次用户请求 Oboe 的新会话中使用 AudioTrack，保留可诊断原因。不得热切换正在播放的会话、静默改用户偏好或引入 SRC。

**交付**：源码与 CPU/离线构建、生命周期及回归门禁；同源 PCM、真机延迟/XRun/声学 A/B 是后续设备验收，不由合成数据代替。Oboe 未经设备实测证明胜出前，AudioTrack 继续作为默认。焦点、AOD、通知和升级行为的设备验证仍须单独完成。

**离线验收状态（2026-10-20）**：Android Receiver `assembleDebug` 与 JVM 测试通过，55/55；四 ABI CMake targets 重建通过，APK 四 ABI 各含 9 个匹配 JNI exports。`libanpod_oboe_output.so` 与 `liboboe.so` 的 ELF LOAD 对齐在四 ABI 均为 0x4000；这不代表所有依赖 ELF 均为 16 KiB 对齐，32 位 `libc++_shared.so` 仍为 0x1000。手动 MSVC Release SPSC CPU 测试通过；Windows MSVC CMake/CTest 因 C1902 未通过，不声称 CTest 通过。APK 中 16 个 `.so` 的 ZIP 数据偏移均可被离线脚本验证为 16 KiB 对齐；Build Tools 34 不支持 `zipalign -P 16`，故此项不是 zipalign 检查结果。Debug APK SHA-256：`96101F5451004E039778196E5CBFF7ECB08B88EEB1547EC9EEB29C5BA2C8DA40`；签名仅为 debug，未验证 release 签名升级。Rust 未在 Phase 5 改动，沿用 Phase 4 离线证据，不计作 Phase 5 新运行门禁。Android UI 视觉检查、真实设备安装/播放、硬件采样率与精度、A/B 声学/延迟/XRun、路由/焦点和长时稳定性仍为 **NOT_TESTED**。复现命令与日志见 `docs/audio-quality/05-oboe-output.md`。

### Phase 6 — USB Direct AOA 正式接入（已承诺的后续路径）

**任务**：按独立阶段把经验证的 AOA 探针发展为受控 transport；不得把 Echo RTT 探针直接作为流协议。

- **6a 协议与离线编解码**：AOA-only ANPB 派生密钥、ANPU 有界外层记录、已有 P5-A/P5-C/ANPD codecs 的协议复用，以及共享合成向量和跨语言消费者检查。不接 USB 设备。
- **6b1 平台 I/O 基础**：实现 Windows WinUSB 接口发现、端点与流 I/O owner，以及 Android API 24+ AOA `FileChannel` 读写、记录 deadline 和后台取消关闭；主 app `minSdk 23`、稳定 UDP/ADB 路径不变。按平台原生取消能力处理阻塞 I/O，不把 Android 描述为依赖 `poll` 的非阻塞实现。
- **6b2 认证 session 与共享 ingress**：把已授权 AOA session 接入现有 Android ingress / 音频队列和主机 owner/session 生命周期。旧 UDP 语义保持，V2 PCM16 worker 与队列需明确建立；Service 不得在自身停止路径 join AOA reader。依次验证 PCM16、PCM24、96k。不要把 UI 或 codec 测试当作设备链路通过。
- **6c UI/state 产品接线**：接入用户可见的 AOA 选择、权限/连接状态、实际格式和可诊断失败/恢复状态；保持现有 USB ADB 与无线 UDP 路径及用户授权边界。

**6a 离线状态（2026-10-20）**：Rust 与 Java 实现 AOA 独立密钥派生、ANPU 有界记录 read/write，并用共享合成向量覆盖 3 个 profile 的 P5-A proof/key、P5-C query/BIND、ANPD FORMAT/AUDIO 与错误域/replay 校验。JDK 17 Java offline checks 与 Android Receiver JVM 55/55 通过；Rust fmt、normal strict Clippy、focused 7/7 通过。Rust 全量锁定离线单线程套件为 120 passed / 0 failed / 1 ignored / 2 filtered（126.92 秒），但此全量运行早于最终新增的部分读写取消测试；新增后 focused、fmt 与 Clippy 已复跑通过，全量未重复。详见 `docs/audio-quality/06-aoa-transport.md`。这只验收 6a 协议/离线编解码，不表示 6b/6c 已实现，也不表示设备测试、AOA USB 传输、权限流程、真实吞吐、延迟或声学验收通过，相关项目仍为 **NOT_TESTED**。


**设备与声学验收门槛**：依用户授权决定是否进行实际手机/PC/USB 组合的权限、拔插恢复、吞吐、声学延迟和长时稳定性测试；该门槛不阻塞继续实现后续源码阶段。未实测前保持 **NOT_TESTED**，不得把离线 I/O owner 测试写成真实传输通过。

**6b1 离线状态（2026-10-20，源码已冻结）**：Android AOA `FileChannel` I/O owner 与基于生产 ANPU codec 的 fake-channel 检查通过，定向 JUnit 5/5；验证覆盖阻塞读取消、记录 deadline、短读写及 owner descriptor 的单次关闭，不代表 OEM/真机行为。Windows WinUSB I/O owner 的 focused 7/7、Rust fmt、严格 Clippy 与 `x86_64-pc-windows-msvc` `cargo check` 均通过。6b1 仅是平台 I/O 基础，不包含 6b2 session/ingress 或 6c UI/state 接线，也没有真实 accessory 传输；Android OEM 行为、Windows 实机、完整产品会话和音频/声学仍为 **NOT_TESTED**。命令与结果见 `docs/audio-quality/06-aoa-transport.md`；日志为 `artifacts/phase6b-focused-tests.log`、`phase6b-fmt-check.log`、`phase6b-clippy.log`、`phase6b-windows-check.log` 与 Android `artifacts/phase6b1-android-build/test-results/testDebugUnitTest/TEST-app.anpod.receiver.AoaAccessoryIoTest.xml`。

**6b2a 离线状态（2026-10-20，源码已冻结）**：Android 增加共享 ANPD 认证 ingress、预建 profile 匹配队列及 Receiver Service 音频 worker 接点；旧 v1 UDP PCM 队列入口保持，V2 PCM16 与 packed-PCM24 走各自 worker。Service 不接管或 join AOA I/O reader，context 销毁仍由尚待完成的 Coordinator 在 I/O 停止并 join 后负责。`packaging/check-java-offline.ps1` 的 7 项检查通过；Android Receiver JVM 测试 61/61（9 XML suites，0 failures/errors/skips），日志见 `artifacts/phase6b2-java-offline.log` 与 `artifacts/phase6b2-android-jvm.log`。这只验收共享 ingress/worker 基础，不是完整认证 session 或产品 USB Direct 接入；6b2 Coordinator 和 6c UI/state 仍待完成。USB 权限/拔插、真实 Windows/Android accessory、播放链路、吞吐、延迟和声学仍为 **NOT_TESTED**。

**6b2b 离线状态（2026-10-20，源码已冻结）**：Rust AOA owner 与 Android `AoaReceiverCoordinator`/Service endpoint 核心已有生产路径和 fake-stream session checks；实现 P5-A 认证、CAP_AUDIO_V2、共享 P5-C raw-frame verifier、descriptor 锁定后 START 前 ingress 注册、PAUSE/RESUME/STOP、队列/worker 清理。Rust owner focused 6/6、WinUSB focused 7/7、P5-C 向量 1/1、fmt 与 normal strict Clippy 通过；最终 elevated locked/offline 单线程 Rust suite 135 passed / 0 failed / 0 ignored / 2 filtered（95.81 秒）。两个过滤项为 `adb_stdin_child_helper` 与 `p1_real_usb_installed_receiver_health_start_stop_when_explicitly_requested`。`reads_the_artifact_written_by_windows_powershell_when_requested` 本轮未设置 `ANPOD_RECEIVER_ARTIFACT_DIR`，测试走无 artifact 的提前返回分支，因此不算 artifact I/O 验证。Android Receiver JVM 64/64（10 XML suites，0 failures/errors/skips），专类 focused 3/3，Java offline 7/7。日志和精确结果见 `docs/audio-quality/06-aoa-transport.md`。该批仅验收认证 session 核心，不包含生产 `UsbManager.openAccessory`/权限入口和 Coordinator 的产品创建/保留接线；这两项由 6c 完成。Android permission/拔插、Windows PnP/WinUSB 实机、真实 USB 播放、吞吐/延迟/声学仍为 **NOT_TESTED**。

**交付**：AOA 技术文档、可恢复连接、可重复安装与回退流程、完整声学测量。**通过门槛**：不伤害 ADB 驱动，不要求 Root，不在无用户授权时静默重置手机 USB 模式；任何失败可退回原模式。

### Phase 7 — 发行、文档与最终验收

**任务**：稳定档默认值策略、混合老/新版本兼容、错误上报、文档和 release APK 精确校验。**交付**：打包日志、签名/制品哈希、版本更新和最终报告。

**阶段推进规则**：Phase 2 之后允许在隔离开发路径中先推进下一阶段的源码和相关离线门禁；这不代表前阶段尚未执行的设备、声学、延迟或稳定性门槛已验收。每个阶段仍须保留 `PASS`、`NOT_TESTED` 与失败证据，不能以源码/合成测试代替真实链路验收。发行、稳定默认值变更和最终验收仍须满足相应设备实测门槛；`preview` 已有稳定工作路径不得回退。

---

## 11. 测试矩阵和验收标准

### 11.1 功能与安全（强制）

| ID | 场景 | 必须达到的标准 |
|---|---|---|
| Q-01 | 标准 48/16（无线 UDP） | 同源旧版协议与 PCM16 载荷一致；无额外系统采集异常 |
| Q-02 | 48/24 | 解码 PCM24 全量范围正确，符号扩展正确，端到端（输出前）无暗中 16-bit 截断 |
| Q-03 | 96/24 | 96k 实际数据可识别，帧索引/帧长正确；源 48k 时 UI 不伪称原生 Hi-Res |
| Q-04 | 1200B UDP 上限 | 所有包≤1200 B，帧对齐，极值/随机包通过；无 IP 分片依赖 |
| Q-05 | v1↔v2 交互 | 新 PC+旧 Receiver 可回退 v1；新 Receiver+旧 PC 仍按 v1 工作；错版本安全拒绝 |
| Q-06 | 加密认证 | v2 profile、rate、format 受认证；篡改/重放/旧会话帧拒绝；密钥不泄露 |
| Q-07 | 不完整数据 | 缺包、乱序、重复包、片段错位、非法编码不崩溃、不乱播；PLC 行为可统计 |
| Q-08 | 播放中切换 | 不混格式、不混会话、不残留旧 PCM、不爆音，失败可恢复旧会话 |
| Q-09 | 音频焦点/中断 | 音乐/通知/来电/暂停/恢复/拔插耳机不会卡死、泄露或无声假运行 |
| Q-10 | 签名升级 | 已安装正式签名 Receiver 可原地升级，版本码递增、hash 白名单正确、用户绑定保持 |
| Q-11 | AOA | 经过用户授权成功传 PCM 并可恢复；MI_01 ADB 驱动与复合父设备不受影响 |

### 11.2 音质客观验证

- **测试信号**：双声道极值 + 1kHz 正弦 + 高频扫频 + 低电平信号 + 数字静音 + 长时随机 PCM；跨语言 golden vectors 覆盖负数/最小负值/字节序/饱和边界。
- **线缆/网络数据保真**：在 Android 输出引擎之前做 PCM payload CRC/hash 或逐样本比较；应用字节序编码与解码必须 bit-exact（故意做 Dither/Resample 的阶段分别验证误差规格）。
- **高精度有效性**：用源 Float32 高精度测试信号确认 24-bit wire 含有超过 16-bit 的有效低位信息，不能只把 int16 扩展为 int24；测试包含接近噪声底的低幅度信号。
- **频响与失真**：改变采样率前后验证抗混叠和重采样器滤波；音质对比需要外部录音或可观测数字 PCM 对照，不能只依赖听感或 XRun。
- **静音与声道**：保持全零静音（若启用 dither 需明确策略）、LR 相位/声道定位和 0dBFS 裁剪一致。

### 11.3 延迟/稳定性（与真正音质相互独立）

- 在**同一 PC、Wi-Fi、手机、音量、输出设备**下，标准/高保真/Hi-Res各测低延迟与均衡；至少每组连续 30 分钟，再做一次较长稳定性压力测试。
- 声学端到端延迟使用相同外部方法记录 P50/P95；同时记录网络 queue、PCM callback、AudioTrack/Oboe buffer、XRuns、packet loss 和重建次数。
- 首版发布门槛：在稳定网络、相同输出后端与播放档位下，48/24 的 P95 **不得显著恶化**；建议工程警戒阈值为相比 PCM16 基线 **增加超过 10ms 或 10% 时必须诊断并阻止默认启用**（两者以更严格者为准）。此为**验收目标，不是已实现承诺**。
- 连续运行期间不允许可重复的爆音、持久性失真、内存持续增长、失去停止按钮响应；正常网络下不得出现由新增格式逻辑引起的持续数据断裂。
- Android 音频 XRun/underrun **按引擎分别统计**，不要把两个 API 的计数器直接当完全等价测量。均需做突发网络干扰、音频焦点抢占和后台功耗/熄屏相关测试。
- 不以 AOA echo RTT <1ms 或 OboeTester 约30ms timestamp 估算推出最终声学延迟。

### 11.4 自动测试要求

- `cargo test`/Rust fmt+clippy：加密帧跨语种向量、profile encode/decode、PCM24 拆包、乱序/重放、队列/超时。
- C++ native：Float32/Int16 ABI 版本、格式转换单测、采集重新初始化/设备变更/内存和同步。
- Android JVM 单元测试：PCM16/24 编码、Jitter/PLC/Format 转换、输出状态机、回退、重复 stop。
- Android 仪器与真机测试：AAudio/AudioTrack 实际流参数，真实 PCM 播放，包到达和音频焦点。
- UI 自动化：配置迁移、不可用选项、实际格式状态、两种语言、切档失败回退。

---

## 12. 工程执行规则（给 AI 开发 Agent）

1. 先输出基线代码审查与**可检验文件清单**，再按 Phase 0–7 分 PR 或小提交实现；每个提交要说明它影响哪个现有模块及怎样验证。
2. **最小改动、可回退**。旧 v1、旧 AudioTrack、旧 USB ADB、旧 UDP 与无线配对保持独立安全通路。不要为了共用逻辑删除已运行的稳定实现。
3. 每个 Phase 都附实际执行的 build/test 命令、通过/失败日志、已知缺陷和回退方法；**未运行的测试必须如实标注为未验证**。不得输出“已经完成”但没有代码或运行记录。
4. 传输协议和能力协商**先定义 wire spec，再写实现和 golden vectors**；Rust 与 Java 必须独立解析同一份向量，禁止根据单端实现猜测另一端。
5. JNI/C ABI 生命周期、Android 同签名升级、Rust tauri snapshot 字段兼容与前端 TypeScript 类型编译列为不可跳过的评审项。
6. 任何扩大字节缓冲必须换算为**音频毫秒数**审查，禁止为容纳 PCM24/96k 而无意增加网络缓冲或 AudioTrack 实际延迟。
7. 不随意修改 `.gitignore`、签名、绑定文件格式、发布密钥路径、依赖许可证；新增 Oboe 保留 upstream attribution 与 license。**任何第三方源代码进入仓库前，先完成第 14 章的依赖/许可证审查与版本固定，并记录到 `THIRD_PARTY_NOTICES.md`。**
8. 不自动更改用户 Windows 系统默认音频采样率，不后台静默切换 Android USB 配件模式，不自动替换 WinUSB/ADB 驱动。
9. 质量档、传输方式、输出引擎三者独立；每个组合须有合法/受支持的 capability 判断和明确 fallback。
10. 完成后提交 `docs/audio-quality/` 内的 `FORMAT_V2.md`、`COMPATIBILITY.md`、`BENCHMARKS.md`、`ROLLBACK.md` 及更新 README，确保其他开发者能复现实验。

---

## 13. 风险登记与优先规避

| 风险 | 发生条件 | 缓解和停止条件 |
|---|---|---|
| 假高保真 | 16-bit 已量化的 WASAPI 数据被零填充成 PCM24 | 精度账本 + 低位有效信号测试；不满足不得报告无损精度升级 |
| 假 Hi-Res | 48k mix 被无提示升为 96k | Hi-Res 实验请求仅接受真实 96k Windows mix 与 Receiver 的精确 96k descriptor；不支持时明确失败并由用户选择新会话 |
| v1 兼容性破坏 | 直接改变 7 字节认证参数或 FORMAT 标签 | 并行 v2 协议与黄金向量；老设备回退 v1 |
| 无线断音增加 | 高位深每 5ms 拆成 2–3 包，队列不适配 | 1200B UDP 上限不变，逐帧片段可入队；真实丢包压力测试 |
| 音频精度回退 | Android 24-bit 轨道失败时静默降 PCM16 | 明确报告实际输出格式；支持 Float32 输出转换，不支持时明确失败并由用户选择新格式会话 |
| Oboe 不更快 | iQOO 内置扬声器仍走 Legacy/Shared | 保留 AudioTrack 默认，隔离实验开关，A/B 数据说话 |
| 不安全 USB 恢复 | 意外替换 ADB `MI_01` 或无提示重枚举 | 只绑定 Accessory `MI_00`，用户确认，断线可手工拔插或退回 USB ADB |
| 版本/签名不一致 | Receiver 重新打包后失去原签名或 hash 白名单 | 原密钥签名、versionCode 递增、精确 artifact 审核 |
| 缓冲无意叠加 | Float32 queue + jitter + native ring 各自设目标 | 统一时延预算和帧单位统计，限定额外 bridge 缓冲 |

---

## 14. 开源项目与官方指南调查：可复用实现与集成边界

> 调研更新：2026-10-19；类型为**源码/公开文档级别的可行性调研**，不是已经导入第三方依赖、完成构建或取得实际音质性能收益。下表的“直接集成”只表示**条件满足后可以纳入工程的候选**，并不意味着现在已经采用。开工前锁定 release/tag/commit 并重新审查版本及许可证。

### 14.1 技术选型总表（优先级、来源、许可证与对应 AnPod 模块）

| 方向 / 项目 | 关键可参考源码 / 官方入口 | 可复用部分（精确到工作任务） | 与 AnPod 对应位置 | 引入策略及许可证风险 |
|---|---|---|---|---|
| **Windows WASAPI：Microsoft Windows-classic-samples** | [ApplicationLoopback 示例](https://github.com/microsoft/Windows-classic-samples/tree/main/Samples/ApplicationLoopback/cpp) / [LoopbackCapture.cpp](https://github.com/microsoft/Windows-classic-samples/blob/main/Samples/ApplicationLoopback/cpp/LoopbackCapture.cpp) | `AUDCLNT_STREAMFLAGS_LOOPBACK`、事件驱动采集、`GetNextPacketSize` 循环排空、`GetBuffer`/`ReleaseBuffer` 生命周期与设备失效处理 | `src/AnPod.Native/audio_capture.cpp`、`audio_capture.h` | **参考范式，非整体迁移**；根目录 MIT，仍检查样例中的额外 LICENSE。该示例使用的进程级采集 API 有 Windows 构建版本前提；AnPod 仍使用通用 endpoint loopback，不能因此提高 Win10 最低兼容版本。 |
| **WASAPI 混音格式：Microsoft 官方文档** | [GetMixFormat](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getmixformat) / [IsFormatSupported](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-isformatsupported) / [Loopback Recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording) | 查询 `WAVEFORMATEXTENSIBLE` 中子格式、有效位数、通道、采样率；`CoTaskMemFree`；先能力检测再请求格式；不要把 Windows mix Float32 理解为源文件具有 32-bit 有效精度 | Phase 0、Phase 2–4 的采集能力检测与高精度状态报告 | **必读官方规范，无第三方库**；不能绕过系统混音处理；不自动修改 Windows 音频设备的系统格式。 |
| **Rust 重采样：Rubato** | [仓库](https://github.com/HEnquist/rubato) / [Cargo.toml](https://github.com/HEnquist/rubato/blob/master/Cargo.toml) / [示例](https://github.com/HEnquist/rubato/tree/master/examples) | 48↔96k 以及 Android 输出采样率与传输采样率不一致时的 Float32 分块 SRC；支持可调比例，可对比基于微小时钟偏差的连续 ASRC | `src-tauri/src/audio.rs` 后新增专用 `resample`/`pcm` 模块；移动端如需独立重采样不能简单调用 Rust 主机模块 | **条件优先候选**；MIT OR Apache-2.0；当前发布示例 5.x 要求 Rust >=1.87，而 AnPod `src-tauri/Cargo.toml` 当前 `rust-version=1.90`，初步兼容。必须先跑 `cargo check`、延迟/CPU、无额外分配与重采样器 flush 测试；确认 API 后锁定精确版本。 |
| **轻量 SRC 备用：SpeexDSP** | [项目镜像](https://github.com/xiph/speexdsp) / [resample.c](https://github.com/xiph/speexdsp/blob/master/libspeexdsp/resample.c) / [头文件](https://github.com/xiph/speexdsp/blob/master/include/speex/speex_resampler.h) / [主开发站点](https://gitlab.xiph.org/xiph/speexdsp) | 成熟的定点/浮点重采样思路，对比不同质量等级时的 CPU、群延迟与噪声；若需两端同一 C 实现可作为候选 | Windows Native 和 Android JNI 层的可选 `Resampler` 实现 | **基准比较/备用，不默认引入**；重采样源码为宽松 BSD 类许可，须保留版权和声明；GitHub 仓库是镜像，更新和修复以项目主维护站点为准。 |
| **音质优先 SRC 候选：libsoxr** | [项目代码镜像](https://github.com/chirlu/soxr) / [soxr.h](https://github.com/chirlu/soxr/blob/master/src/soxr.h) | 高质量 SRC 的滤波配置及可变速率模式，与 Rubato、SpeexDSP 做频响、混叠、CPU 和延迟比较 | `resample` 接口的第三种评估候选 | **默认不集成**；LGPL-2.1 义务比 MIT/Apache 更复杂，需单独审查链接、替换/再链接及分发合规，并评估维护成本。 |
| **Android Native 播放：Google Oboe** | [项目](https://github.com/google/oboe) / [完整指南](https://github.com/google/oboe/blob/main/docs/FullGuide.md) / [MinimalOboe](https://github.com/google/oboe/tree/main/samples/MinimalOboe) / [hello-oboe](https://github.com/google/oboe/tree/main/samples/hello-oboe) | AAudio/OpenSL ES 后备 API、`AudioStreamBuilder`、`AudioStreamDataCallback`、`onAudioReady`、真实流配置读取、缓冲调优与错误恢复 | Android 新增 `OboeOutputEngine` + CMake/NDK/JNI 接口，复用现有 Receiver 生命周期 | **实验后端推荐实际集成**；Apache-2.0，锁定经过构建和真机验证的 Oboe 版本。用 `Shared/Legacy` 作为 iQOO 11 当前基线，不预设 MMAP 或 Exclusive。 |
| **Oboe 断流恢复：SoundBoard** | [SoundBoard](https://github.com/google/oboe/tree/main/samples/SoundBoard) / [Oboe 错误回调文档](https://github.com/google/oboe/blob/main/docs/FullGuide.md) | `onErrorAfterClose` 处理设备断开/路由切换，安全地在非实时线程重新创建流；关闭过程中保护状态机 | `ReceiverService.java` 与 `OboeOutputEngine` 的 stop/restart/fallback 状态机 | **借鉴生命周期模式**；错误回调不允许直接执行复杂阻塞恢复，不把现有播放/焦点/AOD 功能丢掉。 |
| **C++ SPSC：ReaderWriterQueue** | [项目](https://github.com/cameron314/readerwriterqueue) / [readerwriterqueue.h](https://github.com/cameron314/readerwriterqueue/blob/master/readerwriterqueue.h) / [LICENSE](https://github.com/cameron314/readerwriterqueue/blob/master/LICENSE.md) | 生产者写入已经解码或转换的 PCM，消费者在音频回调中无锁读取；减少 JNI/Java 写入调度影响 | Oboe 的 Native PCM 桥接队列，限定在单生产者/单消费者模型 | **条件候选**；简化 BSD；不应为了“无锁”盲目增加第三方依赖。先证明纯预分配简单 SPSC 不能满足需求；不得在 `onAudioReady` 中触发队列自动扩容/内存分配；检查实际线程模型是否真的 1P1C。 |
| **Android 格式与 AudioTrack：Google 官方 API** | [AudioFormat](https://developer.android.com/reference/android/media/AudioFormat) / [AudioTrack](https://developer.android.com/reference/android/media/AudioTrack) / [AAudio](https://developer.android.com/ndk/guides/audio/aaudio/aaudio) | Float32 和 API 31+ Packed24 支持、`ByteBuffer`/字节序约束、实际初始化成功与流配置检测、`getTimestamp`、`getUnderrunCount` | `ReceiverService.java`、`UdpPcmQueue.java`、Android 输出能力查询 | **必读平台规范**；`ENCODING_PCM_FLOAT` 可用于较早 Android，`ENCODING_PCM_24BIT_PACKED` 在 API 31+；低系统版本由**明确的兼容适配**负责，不能把高精度格式支持硬编码为所有系统都有。 |
| **测试工具：Google OboeTester** | [源码](https://github.com/google/oboe/tree/main/apps/OboeTester) / [用户与自动化测试说明](https://github.com/google/oboe/blob/main/apps/OboeTester/docs/README.md) | 验证 Actual API/Sharing/Perf、MMAP、frames-per-burst、XRun 与输出时间戳；可作为新后端回归基线 | Phase 5 真机验证；`docs/audio-quality/BENCHMARKS.md` | **测试辅助，不作为正式应用运行依赖**。时间戳延迟≠手机扬声器声学延迟；设备端报告和实测不能互相代替。 |
| **Windows USB：Microsoft WinUSB 官方样例** | [WinUSB 使用指南](https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/using-winusb-api-to-communicate-with-a-usb-device) | `WinUsb_QueryInterfaceSettings/QueryPipe`、Bulk `WritePipe/ReadPipe`、同步/异步传输、超时、short read、WinUSB 句柄清理 | 后续新增 Windows `AoaTransport`，沿用独立探针已测端点 | **参考 API 使用方式**；每次运行重新枚举当前设备端点，不把 `0x01/0x81` 视为跨设备固定值，用户授权后只操作目标 MI_00。WinUSB 是 Bulk，不是 AOA 2.0 的废弃 USB Audio Class 音频模式。 |
| **Android USB Accessory 官方接入** | [AOA 协议](https://source.android.google.cn/docs/core/interaction/accessories/protocol?hl=en) / [Android Accessory 指南](https://developer.android.com/develop/connectivity/usb/accessory) / [UsbManager](https://developer.android.com/reference/android/hardware/usb/UsbManager) | Accessory 权限请求、`getAccessoryList`、`openAccessory`、FD 读取与应用断开恢复、`GET_PROTOCOL(51)/SEND_STRING(52)/START_ACCESSORY(53)` | 后续新增 Android `AoaReceiver`、`TransportSource` | **按官方协议自行实现与已有探针复用**；不得把 AOA 回声 RTT 当 PCM 播放延迟、不得静默改 USB 驱动或覆盖 ADB MI_01。 |

### 14.2 实施细节：哪些建议直接使用，哪些只参考

**A. Windows WASAPI（Phase 2）：对照 Microsoft 示例，保留 AnPod 本地 C++ 桥接。**

1. 开机或设备变更时通过 `GetMixFormat` 查询真实系统混音格式，用 `WAVEFORMATEXTENSIBLE` 区分 PCM/FLOAT、container bits 与 `wValidBitsPerSample`。只把这些称为**格式元数据**，不自动宣称源声音有同等动态范围；查询得到的指针用 `CoTaskMemFree` 回收。
2. 对照 Microsoft `LoopbackCapture.cpp` 的事件驱动获取数据：循环调用 `GetNextPacketSize()` 直到当前可用包被消费、成对调用 `GetBuffer/ReleaseBuffer`、处理 `AUDCLNT_BUFFERFLAGS_SILENT`、设备移除及重建。评估将现有 1ms 轮询切换为事件驱动的复杂度与时间分布；只有基准证明更稳定才改变当前运行路径。
3. 新增独立的 V2 Float32 回调/ABI 与混音格式读取，不更改已发布的 `const int16_t*` 老接口语义；高精度路径先跑 PCM16 回归，再启用 PCM24 编码器。
4. Microsoft 进程级 ApplicationLoopback 示例要求较新的 Windows 版本，**只用其采集生命周期范式**，不把进程树捕获特性当作本次需求，也不提高 Windows 10 兼容门槛。

**B. 重采样和漂移补偿（Phase 3–4）：优先构建接口，先测再选库。**

建议接口：`Resampler::prepare(input_rate, output_rate, channels, max_frames)`、`process(input, output)->(consumed_frames, produced_frames)`、`reset()`、`latency_frames()`。使用 Float32 数据和预分配缓冲；不在 AudioTrack/Oboe 回调中直接重采样。将固定 48↔96k 重采样与微小 drift correction 视为**两个独立目标**，不可简单使用整数跳帧替代可持续的精度平滑处理。

- **Rubato 首选评估**：其动态调整比例、Rust 工程兼容性及 MIT/Apache 许可证最接近 Windows 主机侧需求。但 AnPod 当前已有低差异帧插/删策略，不能直接删掉；先做长时间音频不漂移、频率响应、CPU、p95 处理时间与端到端声学延迟的 A/B。移动端如需独立 SRC，应考虑 Android 端 Kotlin/Java/NDK 接口，不要默认 Rust crate 能在 Java 端直接重用。
- **SpeexDSP 备选**：如果 Android Native 与 Windows C++ 更需要共享同一实现，或 Rubato 效率不达标，对比其性能与品质。不能因为其含 AEC/降噪功能就引入不需要的 DSP 处理。
- **libsoxr 仅性能/质量研究候选**：在商业/开源分发与 LGPL 合规、部署、维护评估完成前，不加入发布依赖。
- **无不必要 SRC**：若 PC 48k Float32 → 48k PCM24 → 手机 48k，先做到无采样率转换的纯直通；96k 只是可选，必须披露任何 48→96k 软件升采样。

**C. 位深转换和精度防欺骗（Phase 2–4）：优先写可测试的小型独立 codec。**

- PCM24 little-endian 每通道占 **3 bytes/sample**，有符号二补码 Q.23；测试极值 -8388608、8388607、0、1、-1、左右声道交错和任意分片边界。
- Float32 → Integer PCM 的 `clip`、饱和、舍入应明确；必要量化时评估 **TPDF dither**（尤其在 24→16 与低电平内容时），但不要在接收/播放回调内分配内存、使用阻塞 RNG 或超预算 DSP。输入已经为 16-bit 的样本不因填充/重采样而获得真实 24-bit 精度。
- 将 `CaptureMixFormat`、`TransportSampleFormat`、`AndroidTrackFormat`、`PlaybackRouteFormat` 分开存储。Android 内部 `getFormat/getSampleRate` 未必等于物理 DAC 的实际有效位深；不应仅根据 `HW format=I32` 推断其声学解析力。

**D. Android 高精度与 Oboe（Phase 3/5）：借鉴 Google 的实际实现，不照搬测试 App。**

1. PCM24 首选尝试 Android API 31+ 的 packed24 流；旧版本或设备不支持时可尝试 Float32 `AudioTrack`，但要分别报告真实请求值和实际软件处理链路，并提示硬件侧精度不可直接证明。对不支持的流绝不“初始化成功后假装是 24-bit”。
2. `hello-oboe` 用于最小 Native 输出流/JNI 与 Oboe CMake 接入；`MinimalOboe` 用于应用后台/断开后恢复的基础结构；`SoundBoard` 用于 `onErrorAfterClose`、设备路由变更的生命周期参考。
3. Oboe 回调只执行预分配 PCM ring → 输出缓冲的无锁拷贝和必要的有限量转换。参考 [回调指导](https://github.com/google/oboe/blob/main/docs/FullGuide.md)，不得在 `onAudioReady()` 中阻塞、加密/解密、网络读取、`malloc/new`、JNI 往返、关闭/重建流。流关闭/重建必须在控制线程按状态机执行。
4. SPSC 选择：优先简单有界 ring。只有经过基准、并确实需要时才引入 ReaderWriterQueue；如果 Android 多个解码/播放器线程可同时生产，先在工作线程聚合为唯一生产者，不能把 SPSC 用在 MPSC 结构上。
5. AudioTrack `getUnderrunCount()` 与 Oboe `getXRunCount()` 属于不同计数口径；报告不得直接宣称 `0 XRun < 4 Underrun` 就能证明绝对音质优劣。按真实的相同音源与相同声学测法做 A/B，保留 AudioTrack 默认。

**E. AOA（Phase 6）：优先复用经过实测的主机探针与 Android Echo 原型。**

- 在 WinUSB 官方样例上对照端点枚举、超时处理和异步/连续写；把先前 AOA 探针成功的接口切换与 `MI_00` 设备绑定机制抽取成独立 `AoaTransport`，不要把探针的往返 echo 循环原样放进持续 PCM 传输。
- Android 侧通过 `UsbManager` 获取真实附件和用户授权，读取到的 Bulk 数据使用**独立帧协议**（含 session ID、sequence、codec profile、frame index、payload length、CRC/校验策略）；不依赖一次 read 对应一次完整音频帧，因为 Bulk 是字节流概念。
- 明确处理 WinUSB 读写返回 0/短包、ZLP、超时和旧会话残留；断开即停止把旧数据推给播放引擎。物理连接/重新枚举经由用户确认，绝不动正常 ADB MI_01 驱动。

### 14.3 从调研直接生成的开发任务清单与验收关联

| Phase | 给开发 Agent 的具体任务 | 参考资料 | 不可跳过的完成标准 |
|---|---|---|---|
| 0 | 生成依赖决策表 `DEPENDENCY_DECISIONS.md`，将每个候选标为 `adopt / evaluate / reference-only / rejected`；确认构建工具链、Windows/Android 最低系统版本 | 本章 14.1；AnPod `Cargo.toml`/Gradle/CMake | 无未经批准的库拷贝、无许可冲突、无意外提高系统版本门槛 |
| 2 | 提取 WASAPI 设备混音格式与有效位深，建立 V2 Float32 ABI 与捕获事件记录；和 Microsoft 样例逐项对照 | Microsoft ApplicationLoopback、GetMixFormat | PCM16 回归 bit-exact；设备切换无资源泄漏，采样率/格式报告真实 |
| 3 | 完成 24-bit codec 独立纯函数、Rust↔Java golden vectors、Float→24 与 24→Float、Android 实际输出诊断 | Android AudioFormat、AudioTrack API | 24-bit 边界/字节序/通道/切片覆盖率；端到端未经 16-bit 截断 |
| 4 | 如存在真实样率不匹配，评测 Rubato 与备选实现，登记每个 SRC 的 group delay、CPU、吞吐和设备时钟偏差 | Rubato、SpeexDSP、可选 libsoxr | 48/24 直通不引入多余 SRC；96k 的原生/升采样状态可见，重采样前后报告频响与延迟 |
| 5 | Oboe 最小流、回调安全审计、断流重建、SPSC 有界缓冲与设备路由/音频焦点回归 | Oboe hello-oboe/MinimalOboe/SoundBoard/OboeTester；ReaderWriterQueue | iQOO 11 仍以 Legacy/Shared 为基线，观察真机 XRun/P95，AudioTrack 继续默认且可安全回退 |
| 6 | 由 AOA Echo 探针迁移接口枚举/权限与 Bulk 操作，重写持续 PCM session 帧协议，不实施回声式“每包等待确认” | Microsoft WinUSB、AOSP AOA、Android UsbManager | MI_01 ADB 保留、Root 非必要、PCM 连续传输 30–60min、拔插/恢复通过 |
| 7 | 汇总第三方许可证、版本/哈希、真实依赖清单、NOTICE 和 README 来源归属 | 各仓库 LICENSE 与本项目已有 GPL/LGPL 文件 | 对所有随包第三方组件列出确切版本、许可证、源代码/notice 获取方式；可复现构建 |

### 14.4 项目许可证与复用审查规则

1. AnPod 仓库根目录为 **GPL-3.0**，现有 `src/AnPod.Native/audio_capture.cpp` 还标明继承上游 **LGPL-3.0-or-later** 的部分。依赖被引入前必须确认与**实际二进制分发方式、静态/动态链接方式及原著作权声明**的兼容性；不能把许可证不同的源文件全部当作 AnPod 自写代码重新授权。
2. Google Oboe：**Apache-2.0**；Rubato：**MIT OR Apache-2.0**；ReaderWriterQueue：**Simplified BSD**；SpeexDSP 的 SRC 文件有 **BSD 风格版权声明**；Microsoft Windows-classic-samples 根目录 **MIT**（个别文件/样例可能另有声明）。复制代码需保留相应版权、许可证及 NOTICE/附加声明。
3. libsoxr：**LGPL-2.1**，不是无约束的“随意复制”；默认只评估，不直接引入。**不要从来源不清楚的 GitHub Fork 粘贴音频处理代码**。
4. 依赖必须在 PR 中提供：**来源 URL、tag/commit、取得日期、实际纳入文件清单、许可证、修改说明、构建复现方法**。本章列出的 URL 是研究入口，不等于依赖锁定或授权复核已经完成。
5. 如果一个需求可以由几十行、独立、可审计的 PCM24 endian codec 正确解决，应优先本地实现并测试，不为简单编解码引入庞大第三方依赖。

### 14.5 官方指南索引（开发时应直接查阅）

- **Windows 系统音频**：[Loopback recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)、[IAudioClient::GetMixFormat](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getmixformat)、[IAudioClient::IsFormatSupported](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-isformatsupported)。
- **Android Audio**：[AudioFormat](https://developer.android.com/reference/android/media/AudioFormat)、[AudioTrack](https://developer.android.com/reference/android/media/AudioTrack)、[AAudio](https://developer.android.com/ndk/guides/audio/aaudio/aaudio)、[Oboe Full Guide](https://github.com/google/oboe/blob/main/docs/FullGuide.md)、[Android 低延迟音频](https://developer.android.com/games/sdk/oboe/low-latency-audio)。
- **Android 测试**：[OboeTester 测试与自动化说明](https://github.com/google/oboe/blob/main/apps/OboeTester/docs/README.md)。
- **USB/AOA**：[WinUSB API 用法](https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/using-winusb-api-to-communicate-with-a-usb-device)、[AOSP AOA 协议](https://source.android.google.cn/docs/core/interaction/accessories/protocol?hl=en)、[Android USB Accessory](https://developer.android.com/develop/connectivity/usb/accessory)、[UsbManager](https://developer.android.com/reference/android/hardware/usb/UsbManager)。

### 14.6 AnPod 当前基线源码入口

- 项目仓库：<https://github.com/sanren1/AnPod/tree/preview>，调研基线 `c299c805f8917df0f961b5174dff47fb074517e6`。
- Windows Native 捕获：<https://github.com/sanren1/AnPod/blob/preview/src/AnPod.Native/audio_capture.cpp>；对应 C ABI：`src/AnPod.Native/audio_capture.h`。
- Rust 队列/UDP：<https://github.com/sanren1/AnPod/blob/preview/src-tauri/src/network.rs>；加密封包：`src-tauri/src/protocol.rs`；会话认证：<https://github.com/sanren1/AnPod/blob/preview/src-tauri/src/security.rs>。
- Android 播放：<https://github.com/sanren1/AnPod/blob/preview/src/AnPod.Receiver/app/src/main/java/app/anpod/receiver/ReceiverService.java>；UDP 抖动队列：<https://github.com/sanren1/AnPod/blob/preview/src/AnPod.Receiver/app/src/main/java/app/anpod/receiver/UdpPcmQueue.java>；PCM16 漂移修正：`src/AnPod.AndroidProtocol/src/main/java/com/anpod/server/Pcm16FrameStream.java`。
- 构建工具链：`src-tauri/Cargo.toml` 记录 Rust 1.90 基线，`src/AnPod.Native/CMakeLists.txt` 使用 C++17，Receiver Gradle 配置在 `src/AnPod.Receiver/app/build.gradle`。注意：**本次只读源码与文档，不代表新增依赖已编译通过**。

---

**结论**：先以 **48kHz/24-bit Packed PCM** 为真实可交付音质升级核心；仅在能力条件达成时提供 96kHz/24-bit；Float32 限定为内部高精度处理；保留无线 UDP、AudioTrack 默认与 v1 兼容；以实验性 Oboe 改善潜在稳定性；最终将已验证的 AOA USB Direct 纳入统一音频链路。所有档位必须以**实际采集→传输→输出格式**和真实声学测试为准。第三方库采用**先评测/审查许可证、后锁版本并集成**的原则。
