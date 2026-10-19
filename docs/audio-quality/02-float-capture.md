# Phase 2 — Float32 WASAPI capture

## 状态

Phase 2 的 Native/Rust 源码审查和离线门禁已通过。Native ABI 的 CPU-only 测试、Rust ABI/队列/量化测试以及旧 PCM16 回调回归均通过；Java 离线协议回归也通过。**本阶段不包含真实 WASAPI loopback、设备切换、扬声器播放/录音、端到端 PCM16 声学精度或长时稳定性验证，这些仍为 `NOT_TESTED`。** 因此这里只确认代码与离线合同，不把实机验收标为完成。

该实现只增加独立 Float32 采集能力。已有 PCM16 ABI 和默认产品路径继续使用旧 `Initialize`、48 kHz 双声道 PCM16 回调；FloatV2 尚未接入 PCM16 网络发送队列、v1 wire 或 Android 播放链。

## V2 ABI 合同

新增导出为 `AudioCapture_GetCapabilitiesV2`、`AudioCapture_GetCaptureFormatV2`、`AudioCapture_InitializeFloatV2` 和 `AudioCapture_SetFloatPcmCallbackV2`。结构体仅含固定宽度 `uint32_t` 字段并按自然 C 布局：Capabilities 为 16 字节，Format 为 48 字节；两者首字段均为 `size`、`version`。V2 只接受 `version == 2` 且 `size` 至少覆盖已知结构体的调用方，较大的结构体保留未知尾部以供前向扩展。

能力查询可在初始化前执行；采集格式只在 PCM16 或 FloatV2 初始化成功后可读。旧 PCM16 初始化不能被误当成 FloatV2 初始化复用，Float 模式也拒绝 PCM16 格式查询、旧 PCM16 callback 和 TCP listener。缺少完整 V2 导出组的旧 DLL 继续走旧 PCM16 路径；显式 Float 请求返回可诊断的更新 DLL 错误。

格式元数据记录系统 mix 的采样率、声道数、样本表示、容器位数、有效位数及已知声道 mask。普通 `WAVEFORMATEX` 的 mask 为 0；`mixValidBits == 0` 表示系统没有提供有效位数，保留为未知而不推测。FloatV2 回调格式是 mix 采样率、双声道 IEEE Float32。转换标志分别表示采样率、声道布局、样本表示/容器变化。

Float callback 的输入内存只在本次调用期间借用。Native capture thread 启动后、进入采集循环前一次性分配最大 4096 帧缓冲；线程将较大的 WASAPI packet 分块完整提交，静音、暂停数据填精确的 0，并连续维护帧序号。回调/分块处理不获取 Native PCM callback mutex、不分配、不做网络/JNI 工作。Host callback 使用 `try_lock` 并复制到预分配有界 Float 队列；格式错误或大于声明最大帧数的块整体拒绝，队列拥塞按时间上限丢弃较旧数据并记录丢帧。

停止过程先 signal，再由 Cleanup 等待 capture thread 退出并清除 native callback 状态；Host 在确认线程已 join 后注销 setter 并释放 Arc/context。任何无法确认安全注销的失败路径会保留对应 Arc。Native 所有权规则要求 callback/context 在 Start 期间不变。

Float→PCM16 纯函数只作为后续 wire worker 的量化准备，不处于 Float capture callback 或现有 PCM16 捕获路径中。约定有限输入先 clip 到 `[-1, 1]`，乘以 32768 后使用对称四舍五入（ties away from zero），再饱和到 `i16`；NaN/Inf 返回错误，不默认加 dither，0 保持 0。覆盖 `N/32768` 全部 PCM16 网格的测试确认值 bit-exact，包括满量程和正负边界。

## 离线验证记录

| 检查 | 结果 |
|---|---|
| Native x64 Release 隔离构建（MSVC 19.44） | PASS；build 在 `artifacts/phase2-native-audit/build`，不覆盖 `deps`。构建有既有 C4819 中文注释代码页警告。 |
| Native CTest ABI/分块用例 | PASS，1/1（覆盖 DLL 加载、旧/新导出、结构尺寸与版本校验、较大 V2 尾部兼容、能力查询、初始化前格式拒绝，以及纯 CPU 8193 帧完整分块、连续帧索引、静音/暂停输出、低位 Float 保真和双声道容量约束；不初始化 WASAPI）。 |
| Native `dumpbin /exports` | PASS；18 个导出，包含既有 14 个和新增 4 个 V2 导出。 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | PASS。 |
| `cargo clippy --locked --offline --manifest-path src-tauri/Cargo.toml -- -D warnings` | PASS。 |
| 完整 Rust 离线单测（Tauri bundle resources 置空，JDK 17，单线程；过滤两项明确要求设备/PowerShell 工件的手动测试） | 提权进程 PASS：103 passed，0 failed，1 ignored，2 filtered。受限沙箱首跑有 3 项 fake ADB/DPAPI 临时文件 `Access Denied`；相同测试和断言提权重跑后通过，未弱化断言。Float 定向测试包含 6 项，旧 PCM16 callback exactness 定向回归 1 项也通过。 |
| `packaging/check-java-offline.ps1` | PASS；既有 pairing、replay、v2 vector/handshake、PCM framing/mode、audio focus/write-generation 检查通过。 |
| `git diff --check` | PASS；Git 提示 Rust 文件工作树 LF 将在后续 Git 写入时转换为 CRLF。 |

Rust 测试采用的完整命令：

```powershell
$env:TAURI_CONFIG = '{"bundle":{"resources":[]}}'
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml -- --test-threads=1 --skip receiver::tests::p1_real_usb_installed_receiver_health_start_stop_when_explicitly_requested --skip receiver::tests::reads_the_artifact_written_by_windows_powershell_when_requested
```

本次门禁没有设备或真实音频会话。Native CTest 是 ABI/输入验证测试；Rust Float queue、格式和量化测试是合成内存数据。当前没有真实 Float capture trace，不能由这些结果推断 WASAPI 设备兼容、混音精度、转换行为、声学 bit-exact、延迟或长期稳定性。

## Phase 3 移交边界

- FloatV2 保留 Windows 实际 mix 采样率；独立 Float 队列不接旧 PCM16 sender/v1 wire。Phase 3 的 48 kHz/24-bit 直通仅在真实 PC mix 为 48 kHz 时启用；mix 不是 48 kHz 时明确拒绝或整档回退，不静默丢块。96 kHz 与真实重采样属于后续 Phase 4。
- PCM24 codec 的建议合同为 Q.23、有符号二补码、3 字节 little-endian；Float→Q.23 clip 后对称舍入、ties away from zero，非有限输入返回可诊断错误，不默认 dither。纯 sample codec 测 ±1 LSB 和 16-bit 网格之外的低位；跨任意样本片段拼接留给 stream adapter。
- Android 运行时必须 probe 实际打开的输出流。`ENCODING_PCM_24BIT_PACKED` 是 API 31+ 能力且设备支持不保证；`AudioTrack.write(float[])` 从 API 21 提供。格式 probe 只能证明软件流配置，不能证明物理 DAC 有效精度。来源：[AudioFormat](https://developer.android.com/reference/android/media/AudioFormat#ENCODING_PCM_24BIT_PACKED)、[AudioTrack](https://developer.android.com/reference/android/media/AudioTrack)。
