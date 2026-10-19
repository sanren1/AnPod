# AnPod 双引擎对照结果表（填写）

测试设备：iQOO 11（V2243A） / Android 16；内置扬声器；音量及音效设置一致。

| 项目 | AudioTrack 2 Bursts | Oboe 2 Bursts | AudioTrack 3 Bursts | Oboe 3 Bursts |
|---|---:|---:|---:|---:|
| 真实 BufferFrames | | | | |
| BurstFrames | 系统提示： | 实测： | 系统提示： | 实测： |
| 输出路径 | | | | |
| 时长（秒） | | | | |
| 可用时间戳样本数 | | | | |
| 时间戳延迟估算 P50（ms） | | | | |
| 时间戳延迟估算 P95（ms） | | | | |
| XRuns 增量 | | | | |
| 听到杂音 / 突然静音 | | | | |
| 操作系统/其他音效 | | | | |

> 统计时排除预热 5 秒；如两引擎真实 bufferFrames、硬件输出路径或时间戳的定义不同，不直接将估算差值解释为真实延迟收益。

## 开发决策

1. 若 Native Oboe 在相同实际缓冲下显示明显更少的断音、更少 XRuns，且另行测得声学延迟更低，才启动集成。
2. 若差异很小，优先优化 AnPod `UdpPcmQueue`、AudioTrack 和漂移校正。
3. 无论结果如何，保留无线 UDP 传输与独立归档的 AOA USB Direct 验证；不自动切换原 Receiver 默认引擎。
