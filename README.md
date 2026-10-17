<p align="center">
  <img src="icon.png" alt="AnPod 图标" width="160">
</p>

<h1 align="center">AnPod</h1>

把 Windows 电脑的声音发送到 Android 手机，支持 USB 和无线连接。目前处于 Preview 阶段。

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

## 无线 UDP 快速开始

1. 首次使用先用 USB 连接手机，按 PC 端“首次设置”完成配对和手机应用安装。
2. 手机点“开启后台等待”；连接请求到来时允许本次或 8 小时，也可明确选择记住电脑，直到取消。后台等待需主动开启，不会随开机自动启动。
3. PC 选择“无线”，点击“连接手机”。多个目标或原选择失效时，按提示选择手机和网络。

手机也可主动发起：电脑开启等待连接，手机点“开始接收”。授权可在两端查看、取消；关闭后台连接不等于撤销已保存的授权。

## 升级

手机 APK 随 PC 便携包提供。USB 连接后从 PC 的“手机应用”入口检查并升级。预览包使用候选 debug 签名，不是正式商店签名。

## 最新延迟测试（2026-10-17）

PC 1.2.1 + Receiver 0.9.1/code 12；每档约 24 秒、10 个提示音，10/10 匹配并经独立复核。

| 连接与档位 | 中位数 | P95 |
| --- | ---: | ---: |
| USB · 均衡 | 156.720 ms | 157.111 ms |
| USB · 低延迟 | 123.269 ms | 125.299 ms |
| 无线 UDP · 均衡 | 310.645 ms | 314.294 ms |
| 无线 UDP · 低延迟 | 187.394 ms | 200.240 ms |
| 电脑本机 · 基准 | 71.699 ms | 71.778 ms |

电脑本机基准是独立路径，仅作参照，不与四档结果相减。测量包含声学与麦克风链路，是端到端时差，并非纯传输延迟。无线 UDP 低延迟组有两次低匹配分，仅供参考；这不代表音乐音质通过。

完整口径见 [测试记录](https://github.com/sanren1/AnPod/blob/preview/tests/manual-checklist-v2.md#2026-10-17-anpod-121-四档延迟与电脑基准短测)。

## 已知问题

- 1.3.0 用户短测已确认无线切档可自动恢复；低延迟播放超过 5 分钟出现 2 次短时卡顿、约 2 秒恢复，仍需改善，优先使用均衡模式。
- 音乐音质与长期稳定性尚未验收通过，离线模型和短时延迟测试不能替代实听。

## 下载与反馈

公开版本发布后可从 [GitHub Releases](https://github.com/sanren1/AnPod/releases) 下载。问题反馈请到 [GitHub Issues](https://github.com/sanren1/AnPod/issues)，附上连接方式、Windows/Android 版本和复现步骤；提交诊断前请检查设备名、网络地址等隐私信息。

## 开发

源码构建依赖 Rust stable、Node.js/npm、MSVC x64 C++ 工具；重建 Receiver 另需 JDK 17 与 Android SDK。前端使用 `cd frontend; npm ci; npm run build`。便携包入口见 [`packaging/publish-tauri.ps1`](packaging/publish-tauri.ps1)，构建前请阅读其中参数及脚本要求。旧 WPF 实现保留在 `src/AnPod.Desktop/`。

第三方许可与上游声明见 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)、[`LICENSES/`](LICENSES/) 和 [`upstream/`](upstream/)。项目自有代码未声明独立许可证。
