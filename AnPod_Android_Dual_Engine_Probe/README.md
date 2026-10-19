# AnPod Android Audio Engine Probe — 独立双引擎对照工程

**目的**：在 iQOO 11 (Android 16) 上隔离网络和解码，比较 Java AudioTrack 与 Google Oboe (AAudio Shared / Legacy) 的输出行为。工程与 AnPod 原程序完全隔离：包名 `app.anpod.audioprobe`，无需 Root、ADB 调试授权以外的系统修改，也**不使用** USB AOA 或无线 UDP。

## 当前设备能力基线（2026-10-19 真机截图）

- iQOO 11 / V2243A / Android 16/API 36。
- audio.low_latency=true, audio.pro=true；`aaudio.mmap_policy=2`, `aaudio.mmap_exclusive_policy=2`。
- OboeTester 内置扬声器：实际 AAudio Legacy、MMAP NO、Exclusive NO；Media/Game、固定/Auto 设备和采样率均未改变。
- 48kHz、立体声 I16、burst=192 帧（4ms）、buffer=384 帧（8ms）、timestamp.latency ≈ 30–31ms、xRun=0。
- **时间戳输出延迟估算不是扬声器实测声学延迟**；前述结果未证明 Oboe 优于 AudioTrack。

## 工程依赖

- Windows：Android Studio、JDK 17、Android SDK Platform 35、NDK (Side by side)、CMake 3.22.1；首次构建需要联网下载 Maven/Gradle 依赖。
- AGP 8.7.3 + Gradle 8.9；Google Oboe **1.10.0** Prefab (`com.google.oboe:oboe:1.10.0`)；C++17。
- 仅 `arm64-v8a`，安装目标 iQOO 11。
- 包含 `gradlew.bat` 和 wrapper properties；**不包含 wrapper JAR（构建脚本首次从 Gradle 官方 GitHub 下载）**。它也可通过 `-WrapperJarPath` 从现有 AnPod 项目复制。
- **未包含预编译 APK**：制作环境没有 Android SDK/NDK，暂时无法在这里构建及真机验证。工程为静态编写，若首次构建失败请回传 Gradle 出错片段。

## 构建与安装（推荐）

解压到不含特殊权限限制的路径，在根目录 CMD 执行：

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -File ".\Build-Android.ps1"
```

成功后 APK 在 `app\build\outputs\apk\debug\app-debug.apk`。

如果 Gradle 官方 GitHub 下载失败，但你已经有 AnPod 源码和其 Gradle Wrapper jar：

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -File ".\Build-Android.ps1" -WrapperJarPath "F:\Program\AnPod\src\AnPod.Receiver\gradle\wrapper\gradle-wrapper.jar"
```

该示例路径请按实际位置替换。也可以在 **Android Studio → Open → 选择解压后根目录** 后同步 Gradle（第一次需要下载 wrapper JAR）。

安装 APK（用户现有的 adb）：

```cmd
"F:\Download\platform-tools-latest-windows\platform-tools\adb.exe" install -r ".\app\build\outputs\apk\debug\app-debug.apk"
```

或者一步构建并安装：

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -File ".\Build-Android.ps1" -Install -Adb "F:\Download\platform-tools-latest-windows\platform-tools\adb.exe"
```

若 ADB 启动时出现 `daemon not running` 信息，先执行 `adb start-server`。此工程不改变 AnPod 的已配对设备状态、无线 UDP 或 AOA 驱动。

## 快速测试

1. 手机断开其他蓝牙音频输出，仅使用**内置扬声器**；保持音量、DSP/音效和媒体设置一致。打开 App；建议音量先设较低。
2. 缓冲先选 **2 Bursts**。点击 **开始 AudioTrack 基准测试**，保持前台和亮屏运行约 **120 秒**，记下 P50、P95、XRuns、实际 BufferFrames，点击 **停止**。
3. 不改变环境，点击 **开始 Oboe / AAudio 测试**，同样运行约 **120 秒**，然后停止。
4. 点击 **导出两组 CSV 结果**，选择文件位置。把 CSV 发回以便对比。
5. 如果某一组出现断音、XRuns 增加，再分别选 **3 Bursts、4 Bursts** 重测，不要直接扩大队列掩盖缺陷。

`AudioTrack` 的 Burst 为 Android AudioManager 报告的**系统输出提示帧数**，不是 AudioTrack 独立的实测回调帧数；Oboe 的 FramesPerBurst 是实际输出流报告值。每秒采集一次 telemetry；首 5 秒数据标记 `warmup=1` 并排除 P50/P95。

## 实现说明

| 组件 | Java AudioTrack | Native Oboe |
|---|---|---|
| 输入数据 | **同一份 Java 预生成 PCM16 4 秒循环片段** | JNI 只在启动时复制相同数组 |
| 输出 | Java `AudioTrack.write(WRITE_BLOCKING)` | C++ `onAudioReady()` 回调 |
| 采样率/声道 | 48000 / 2ch | 48000 / 2ch |
| 属性 | Usage Media / Content Music / LowLatency | 相同 |
| Sharing | Android 自行决定 | 请求 Shared，并回报实际值 |
| 缓冲 | 请求 `bursts × AudioManager.PROPERTY_OUTPUT_FRAMES_PER_BUFFER` | 请求 `bursts × getFramesPerBurst()` |
| 首层音频协议/网络 | **无** | **无** |
| 统计 | `AudioTrack.getTimestamp`, `getUnderrunCount` | `AudioStream.getTimestamp`, `getXRunCount`, MMAP, callback gap |

**为什么不接 UDP**：该阶段比较纯播放路径，排除网络 jitter 与抖动缓冲。仅当本地对照证明有收益，才在不更改 UDP 协议前提下接入现有 Android Receiver。**为什么不宣称音质更好**：PCM 完全相同，API 不能创造额外音频细节；差异可能表现为可闻爆音、欠载、补偿/重采样及设备处理路径。

## 输出指标解释

- `latency_est_ms`：音频写入计数 + Android 时间戳计算的**输出管线估算值**；非 acoustic latency；可能包含不同 API 的 timestamp 实现误差和隐藏 DSP 延迟。
- `xruns`：平台提供的硬件/音频流欠载计数，`-1` 表示设备不支持查询；不应把 `-1` 当成 0。
- `callback_gap_ms`：Oboe 最近两次 callback 时间间隔；AudioTrack 不提供，填空。
- `max_callback_gap_or_write_ms`：Oboe 最大 callback 间隔；AudioTrack **单次阻塞 write 的最大耗时**，两列单位相同但语义不同，不可直接比较。
- `burst_frames`：Oboe 实际值 vs AudioTrack 系统提示值；要据实际 bufferFrames 判断对比是否公平。
- `buffer_frames` 与 `capacity_frames`：申请后实际得到的缓冲，不同引擎可能有不同约束；绝不可按请求值比较。
- **应用 PCM 输出的时间戳估算值不能单独证明 AnPod 全链路会降低多少毫秒**；最终需要在相同 UDP/WASAPI/扬声器条件下测量真实声学延迟。

## 稳定性与安全

- 不申请 INTERNET、RECORD_AUDIO 或 USB 权限；无录音，无网络上传，也不会修改系统低延迟/MMAP 策略。
- 无前台服务；离开界面自动停止测试。进行长时间测试时保持屏幕点亮。
- Native audio callback 不进行 JNI、分配、互斥锁、网络、磁盘 I/O 或日志。
- A/B 互斥；AudioTrack 为 `WRITE_BLOCKING`，停止时尝试暂停/清空，避免阻塞 writer 未退出就释放。
- 如果有异常：点击停止，退出测试 App 后重开；不要执行任何 setprop/sys.usb.config 或驱动替换。

## 限制与后续判断

- 没有物理声学测量设备，本工程只能给出可复现的**音频管线估算**；精确发声延迟要用外部录音/LED、click 对齐等方法另行测试。
- 回调持续循环预生成数据，不模拟 UDP jitter、解密、GC 压力或系统长时间休眠；这是刻意隔离变量的第一阶段。
- 本工程无 AAudio Exclusive 模式开关：你的 iQOO 11 已在 OboeTester 验证其内置扬声器走 Legacy Shared 路径；先在**已确认可用的路径**做公平对照。
- AudioTrack 可能也占用 Fast Mixer，不能以 Oboe 是 Native 就直接视为更快。
- 交付代码为静态审查版本，尚未经 Android SDK 构建和 iQOO 真机调试。

## 官方资料

- Oboe 官方：https://github.com/google/oboe
- 预编译/Prefab 接入：https://github.com/google/oboe/blob/main/docs/GettingStarted.md
- AudioTrack：https://developer.android.com/reference/android/media/AudioTrack
- Oboe Low Latency：https://developer.android.com/games/sdk/oboe/low-latency-audio
