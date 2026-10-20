<p align="center">
  <img src="icon.png" alt="AnPod 图标" width="160">
</p>

<h1 align="center">AnPod</h1>

把 Windows 电脑的声音发送到 Android 手机，支持 USB 和无线连接。目前处于 Preview 阶段。本地开发候选版本为 PC 1.10.0 + Receiver 0.17.0/code 34，尚未公开发布；AOA 与 Oboe 属于实验功能，公开下载以 Releases 为准。

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

USB Direct 使用 Android Open Accessory（AOA）模式，不是 USB/ADB 的别名。它有独立的认证会话与密钥；三种音质档都走该会话的 v2 音频格式，不能因为 Standard 是 PCM16 就把它当作旧 ADB/v1 会话。AOA 只在用户明确选择时启用，不会自动切换手机 USB 模式，也不回退到 ADB 或 LAN。

当前 dev 源码已接入显式 USB Direct 入口。Receiver 0.17.0-v3-flow-candidate / code 34 的发布签名候选 APK 已完成离线构建与签名校验；PC 1.10.0 与该 Receiver 是本地开发候选组合，尚不是公开发布版本。不要用旧版 PC/Receiver 组合尝试播放。只有安装声明 AnPod / AnPod USB Direct 配件身份且包含匹配 AOA 会话实现的 Receiver 后，才可按以下实验步骤操作：

1. 先通过现有路径完成 PC 与 Receiver 配对。手机端 Android 版本需为 API 24 或更高；App 的 minSdk 仍为 23，原 USB/ADB 与 LAN/UDP 路径不受此限制。
2. 手动切换 helper 仅支持 iQOO 11 V2243A，切换前只检查 VID_2D95&PID_6013&MI_02。关闭 AnPod 和其他可能占用该接口的程序；若 adb.exe 仍在运行，脚本只检查并退出，不会停止 ADB、切换驱动或枚举其它手机。
3. 从源码 checkout 的 PowerShell 运行 `./packaging/enter-usb-direct.ps1`，并在提示时手动输入 `SWITCH AOA`。它复用 `AnPod_AOA_Phase2_Probe/Windows/AoaHost.cs`；独立便携候选把脚本和 helper 放在同一 `USB-Direct/` 目录。脚本只发送 AOA `GET_PROTOCOL (51)`、产品身份 `SEND_STRING (52)` 和 `START_ACCESSORY (53)`；手机 USB 会重新枚举，ADB/MTP 可能暂时断开。产品播放端不会替你切换 USB 模式，也不会安装或替换驱动。
4. 切换后在手机前台打开 Receiver，点“USB Direct”并确认 Android 配件权限。然后在 PC AnPod 点“USB Direct”，手动选择已枚举的 accessory target 和对应的已保存配对 binding，再开始连接。Receiver 在等待主机 HELLO 时最多等待约 30 秒；两端未完成授权或连接失败会显示错误，不会改走 ADB/LAN。
5. 当前 Windows 产品路径要求 accessory 接口已由 WinUSB 驱动提供；复合 18D1:2D01 设备只允许使用 MI_00，MI_01 是 ADB 接口，音频路径不会打开它。取消或拔线结束当前会话；要回原 USB 模式，按手机和线缆提示断开并重新连接。

这条手动切换仅为受限实验入口，不是自动 AOA 流程或兼容任意设备的工具。现有 `AnPod_AOA_Phase2_Probe/Windows/Run-AoaPhase2.ps1 -Mode Switch` 使用 `AnPod Lab` / `AOA Bulk Probe` 身份，只用于独立回显探针；它不是产品播放器，不能与 `AnPod` / `AnPod USB Direct` 混用。产品 State、权限和跨语言离线测试已接线；AOA 真机权限、拔插恢复、Windows 实际 WinUSB 访问、音频播放和声学表现仍未测试，见 [AOA 传输记录](docs/audio-quality/06-aoa-transport.md)。

## 无线 UDP 快速开始

1. 首次使用先用 USB 连接手机，按 PC 端“首次设置”完成配对和手机应用安装。
2. 手机点“开启后台等待”；连接请求到来时允许本次或 8 小时，也可明确选择记住电脑，直到取消。后台等待需主动开启，不会随开机自动启动。
3. PC 选择“无线”，点击手机卡片或“连接手机”，进入同一个连接页。找不到手机时点“找不到手机？”按提示处理。

手机也可主动发起：电脑开启等待连接，手机点“开始接收”。授权可在两端查看、取消；关闭后台连接不等于撤销已保存的授权。

## 手机常显（AOD）

手机端提供“伪 AOD”常显：纯黑横屏，显示大号时钟、由真实 PCM 驱动的双线波形，以及状态、协议、接收码率、接收端缓冲延迟、UDP 缺帧率五项指标。

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
