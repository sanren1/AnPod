# AnPod

把 Windows 当前默认播放设备的声音发送到 Android 手机播放。USB 可使用随包 Legacy 服务；无线 UDP 使用 AnPod Receiver。项目目前仍处于 Preview 阶段。

## 使用要求

- Windows 10/11 x64；需 Microsoft Edge WebView2 Runtime，便携包不包含该运行时。
- Android 6.0 或更高版本。
- USB 首次连接需要开启 USB 调试并在手机上授权电脑。
- 无线 UDP 需要电脑和手机网络可互访，并在手机安装本版附带的 Receiver APK。
- 无线调试 ADB 仍可在 PC 设置中显式选择；USB 默认保持 Legacy 服务。

## USB 快速开始

1. 启动 PC 端 AnPod，用 USB 连接手机并授权调试。
2. 保持默认 USB Legacy 方式，在 PC 端选择手机并点击“播放到手机”；无需在手机打开 Receiver。
3. 要停止播放时，在 PC 端停止播放或切回电脑输出；如在 PC 设置中选择 USB Receiver，则按手机端提示停止接收。

## 无线 UDP 快速开始

1. 首次绑定及安装/升级 Receiver 时先用 USB 连接手机，并在 PC 端完成相应操作。
2. 切换到无线 UDP 后，在 PC 端选择“等待手机接收”。
3. 在手机 Receiver 必要时选择网络，然后点“开始接收”。手机会保持本次接收意图，正常重连无需重复开始接收。
4. 要停止播放时，在手机 Receiver 点“停止接收”或在 PC 端断开连接。

## 升级

Receiver APK 随 PC 便携包提供。USB 连接后可从 PC 的 Receiver 升级入口检查并升级；升级后重新连接并核对手机端状态。预览包中的 APK 使用候选 debug 签名，不是正式商店签名。

## 已知问题

- 无线播放时切换均衡/低延迟可能断连，当前不会自动恢复；请手动重新连接。
- 无线 UDP 低延迟播放音乐可能卡顿或破音。优先使用均衡模式；切换档位后如未恢复，请在两端手动重新连接。
- 四档短时提示音测试与电脑基准见 [测试记录](https://github.com/sanren1/AnPod/blob/preview/tests/manual-checklist-v2.md#2026-10-17-anpod-121-四档延迟与电脑基准短测)；结果不代表纯传输延迟、音乐音质或长期稳定性通过。

## 下载与反馈

公开版本发布后可从 [GitHub Releases](https://github.com/sanren1/AnPod/releases) 下载。问题反馈请到 [GitHub Issues](https://github.com/sanren1/AnPod/issues)，附上连接方式、Windows/Android 版本和复现步骤；提交诊断前请检查设备名、网络地址等隐私信息。

## 开发

源码构建依赖 Rust stable、Node.js/npm、MSVC x64 C++ 工具；重建 Receiver 另需 JDK 17 与 Android SDK。前端使用 `cd frontend; npm ci; npm run build`。便携包入口见 [`packaging/publish-tauri.ps1`](packaging/publish-tauri.ps1)，构建前请阅读其中参数及脚本要求。旧 WPF 实现保留在 `src/AnPod.Desktop/`。

第三方许可与上游声明见 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)、[`LICENSES/`](LICENSES/) 和 [`upstream/`](upstream/)。项目自有代码未声明独立许可证。
