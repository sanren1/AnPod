<p align="center">
  <img src="icon.png" alt="AnPod 图标" width="160">
</p>

<h1 align="center">AnPod</h1>

把 Windows 电脑的声音发送到 Android 手机，支持 USB 和无线连接。目前处于 Preview 阶段。本地候选为 PC 1.10.5 + Receiver 0.17.4-v3-flow-candidate/code 38，尚非公开发布，真机和音频效果仍未验证；公开下载以 Releases 为准。

## 使用要求

- Windows 10/11 x64；需 Microsoft Edge WebView2 Runtime，便携包不包含该运行时。
- Android 6.0 或更高版本。
- USB 首次连接需要开启 USB 调试并在手机上授权电脑。
- 无线 UDP 需要电脑和手机网络可互访，并在手机安装本版附带的 Receiver APK。
- 默认使用 Receiver 手机应用接收；Legacy 兼容方式和无线调试 ADB 可在高级设置中选择。

## USB 快速开始

1. 启动 PC 端 AnPod，用 USB 连接手机并授权调试。
2. 按提示安装或升级手机应用，然后选择手机并点击“播放到手机”。
3. 点击“停止”或“切回电脑”结束播放；也可在手机停止接收。

## USB Direct AOA（实验性）

本地候选为 PC 1.10.5 + Receiver 0.17.4-v3-flow-candidate/code 38；尚非公开发布，也未通过实机验证。USB Direct 使用独立的 AOA 认证会话，不是 USB/ADB 的别名。当前仅对已验证的 iQOO 11 提供 WinUSB 预切换；切换前需明确允许停止电脑上的全局 ADB server。选择 Direct 模式只是连接偏好，不代表手机已完成物理 USB 模式切换，也不会自动开始播放。

1. 首次使用先通过 USB/ADB 安装 Receiver 并完成配对，再在 USB 下选择持久化的 ADB 或 USB Direct 模式。打开 Direct 主按钮或手机卡片可进入配置页。
2. AnPod 自动刷新只读发现的设备；发现唯一设备时会预选。需要物理切换 USB 模式时，展开切换选项并按提示操作；切换仅支持已验证的 iQOO 11，且须明确允许停止电脑上的全局 ADB server。然后在手机 Receiver 中允许配件权限。
3. 选择已配对的接收端，或使用同一设备的已认证记忆，再明确点击“开始连接”才会播放。初次连接始终手动发起；意外断线仅在同一连接方式内有界恢复，不会自动切换 Direct、ADB 或 LAN；任一端手动停止后不会重连。

ADB 仅支持 Standard（48 kHz、16 bit），切换到有效 ADB 连接会自动使用 Standard。Direct 与 LAN 支持三档音质；HiRes 要求真实 96 kHz 采集且不做采样率转换，手机输出能力尚未实机验证。音频偏好未设置时默认使用 Oboe；已手选的 AudioTrack 会保留，也作为 Oboe 不可用时的兼容回退。AOA 权限、实际播放和声学表现仍为 NOT_TESTED，见 [AOA 传输记录](docs/audio-quality/06-aoa-transport.md)。

## 无线 UDP 快速开始

1. 首次使用先用 USB 连接手机，按 PC 端“首次设置”完成配对和手机应用安装。
2. 手机点“开启后台等待”；连接请求到来时允许本次或 8 小时，也可明确选择记住电脑，直到取消。后台等待需主动开启，不会随开机自动启动；授权记忆只保留授权，不会自动发起 PC 连接。
3. PC 选择“无线”，手动点击手机卡片或“连接手机”，进入同一个连接页。找不到手机时点“找不到手机？”按提示处理。

手机可开启后台等待并处理授权提示；这不会自动建立连接，初次连接仍须在 PC 选择手机并手动点击。授权可在两端查看、取消；关闭后台连接不等于撤销已保存的授权。

## 手机常显（AOD）

手机端提供“伪 AOD”常显：纯黑横屏，显示大号时钟、由真实 PCM 驱动的双线波形，以及状态、协议、接收码率、接收端缓冲延迟、UDP 缺帧率五项指标。USB Direct 只在真实 PCM 入流时显示接收统计；Oboe 与 AudioTrack 共用这一路统计。无法进一步识别的有线或无线 ADB 设备统一显示“ADB”。

- 仅手机端控制：在 Receiver 主界面打开“常显（AOD）”，可点“预览常显”立即进入；PC 没有 AOD 开关，也不新增控制协议。
- 双线波形分别对应立体声的两个声道，共用同一条基线、没有文字标记；右声道反相，所以即使左右声道音频完全一致，也会一条向上、一条向下。
- 指标口径：Recv 为音频 payload 码率，Latency 为接收端可观测缓冲，Loss 为 UDP 缺帧率（USB/ADB 显示 `—`）。
- 只显示当前有效会话；数字静音仍保持显示，波形回到中心直线。连续约 15 秒没有 PCM 才会退出，因此在 PC 上切换“均衡/低延迟”不会让常显退出重进。
- 退出后可从通知的“打开常显”重进；手机不在前台时受 Android 后台启动限制，不会自动弹出。
- 时钟、波形与底部指标栏作为一个整体缓慢漂移（横向约 ±16dp、纵向约 ±9dp），不会长期停在同一批像素上；停留约 6 秒无操作后右上角“退出”淡出，触摸任意位置恢复。
- AMOLED 纯黑、遵从系统亮度；长时间常亮仍有耗电与烧屏风险，锁屏/灭屏后不显示。
- 右侧上下滑动调整手机媒体音量，从第一段滑动就生效，操作时显示系统音量条；实体音量键继续可用。
- 连接、切换播放模式等操作在后台完成，不会弹出手机界面。

完整说明、指标口径与限制见 [`docs/AOD.md`](docs/AOD.md)。

![手机常显预览](docs/aod-preview.jpg)

## 升级

手机 APK 随 PC 便携包提供。USB 连接后从 PC 的“手机应用”入口检查并升级。1.5.0 起 Receiver 使用独立的发布密钥签名；从 1.4.0 及更早的 debug 预览包切过来需要先卸载一次（配对会一起清掉，需重新走 USB 配对），此后同签名的版本可原地升级。密钥与打包流程见 [`packaging/SIGNING.md`](packaging/SIGNING.md)。

每次出包都会递增 Receiver `versionCode`，并同步更新 PC 端的 `AUDITED_RECEIVER_ARTIFACTS`。不要用同 `versionCode` 的 APK 覆盖安装：部分手机（vivo 等）会要求确认“已安装相同版本”，且 PC 会按精确哈希判定为未核验。PC 与手机请使用同一套发布包。

## 版本与变更

手机 APK 随 PC 便携包提供，两端需配套使用。各版本改动与制品见 [GitHub Releases](https://github.com/sanren1/AnPod/releases) 和提交历史。

## 延迟测试记录

PC 1.2.1 + Receiver 0.9.1/code 12；每档约 24 秒、10 个提示音，10/10 匹配并经独立复核。

下表是 **1.2.1 的历史测量**。后续版本尚未重新测量，因此这些数字不能当作当前版本的延迟结论。

| 连接与档位 | 中位数 | P95 |
| --- | ---: | ---: |
| USB · 均衡 | 156.720 ms | 157.111 ms |
| USB · 低延迟 | 123.269 ms | 125.299 ms |
| 无线 UDP · 均衡 | 310.645 ms | 314.294 ms |
| 无线 UDP · 低延迟 | 187.394 ms | 200.240 ms |
| 电脑本机 · 基准 | 71.699 ms | 71.778 ms |

电脑本机基准是独立路径，仅作参照，不与四档结果相减。测量包含声学与麦克风链路，是端到端时差，并非纯传输延迟。无线 UDP 低延迟组有两次低匹配分，仅供参考；这不代表音乐音质通过。

完整口径见 [测试记录](https://github.com/sanren1/AnPod/blob/preview/tests/manual-checklist-v2.md#2026-10-17-anpod-121-四档延迟与电脑基准短测)。

独立的 [TestBench 测试工具](tools/AnPod.TestBench/README.md) 可重复测试后续版本：默认四档共采集 30 分钟，由用户手动切档，并导出延迟、健康状态和人工故障标记；可选两小时长测。30 分钟快速回归不等同于两小时稳定性验收。

## 已知问题

- 低延迟档长时播放可能出现短时卡顿，日常优先使用均衡档。
- 常显仍是预览功能：长时间烧屏与功耗、通知兜底、锁屏/灭屏不显示等仍待完整真机验收。
- 音乐音质与长期稳定性尚未验收；短时延迟测试不能替代实听。
- Receiver 使用正式发布签名，与更早的 debug 预览包不能互相覆盖安装；从旧包升级需先卸载一次。

## 下载与反馈

公开版本发布后可从 [GitHub Releases](https://github.com/sanren1/AnPod/releases) 下载。问题反馈请到 [GitHub Issues](https://github.com/sanren1/AnPod/issues)，附上连接方式、Windows/Android 版本和复现步骤；提交诊断前请检查设备名、网络地址等隐私信息。

## 开发

源码构建依赖 Rust stable、Node.js/npm、MSVC x64 C++ 工具；重建 Receiver 另需 JDK 17 与 Android SDK。前端使用 `cd frontend; npm ci; npm run build`。便携包入口见 [`packaging/publish-tauri.ps1`](packaging/publish-tauri.ps1)，构建前请阅读其中参数及脚本要求。旧 WPF 实现保留在 `src/AnPod.Desktop/`。

第三方许可与上游声明见 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)、[`LICENSES/`](LICENSES/) 和 [`upstream/`](upstream/)。项目自有代码以 **GPL-3.0-or-later** 发布，完整文本见 [`LICENSE`](LICENSE)；其中改编自 AudioShare 的部分仍按 LGPL-3.0-or-later 说明处理。
