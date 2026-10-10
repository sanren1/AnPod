<p align="center">
  <img src="icon.png" alt="AnPod 图标" width="160">
</p>

<h1 align="center">AnPod</h1>

<p align="center">把 Windows 的声音，送到 Android 手机播放。</p>

<p align="center">
  <img src="https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4" alt="Windows 10 / 11 x64">
  <img src="https://img.shields.io/badge/.NET-8-512BD4" alt=".NET 8">
  <img src="https://img.shields.io/badge/Android-6.0%2B-3DDC84" alt="Android 6.0 及以上">
</p>

<p align="center">
  <a href="#快速开始">快速开始</a> ·
  <a href="#源码构建">源码构建</a> ·
  <a href="#常见问题">常见问题</a> ·
  <a href="https://github.com/sanren1/AnPod/issues">反馈问题</a>
</p>

AnPod 是一个 Windows 桌面音频转发工具，支持通过 USB 数据线或 Wi-Fi，将电脑当前默认输出设备的声音实时传到 Android 手机。手机端无需安装 App，授权 ADB 调试后，由 AnPod 启动无界面的音频接收服务。

## 功能

- **USB / Wi-Fi 连接**：支持数据线、同一局域网及 Windows 移动热点。
- **无线调试配对**：支持 Android 11+ 的配对码连接和 mDNS 设备搜索。
- **播放开关**：暂时停止手机播放时保持连接，方便随时恢复。
- **本机静音与恢复**：手机播放期间静音电脑播放端点，暂停或断开后恢复原状态。
- **托盘控制**：支持最小化到托盘、连接控制和查看日志。
- **连接恢复**：提供自动连接上次设备和无线掉线重连选项。

> 当前仓库仍处于开发阶段，无线功能需要继续进行真机验收。二维码配对弹窗和更紧凑的无线界面正在开发与审查中，尚未纳入本次提交。验收记录见 [手动检查清单](tests/manual-checklist.md)。

## 环境要求

| 项目 | 要求 |
| --- | --- |
| 电脑 | Windows 10 / 11，x64 |
| 运行时 | .NET 8 Desktop Runtime |
| 手机 | Android 6.0+；无线模式需要 Android 11+ |
| USB 连接 | 支持数据传输的 USB 线，已开启并授权 USB 调试 |
| 无线连接 | 已开启无线调试，电脑与手机处于可互访的网络 |

Windows 需要有正常工作的默认音频输出设备。AnPod 使用 WASAPI Loopback 采集系统声音，手机不会作为新的 Windows 音频输出设备出现。

## 快速开始

当前提供源码构建方式，步骤见下方 [源码构建](#源码构建)。如果使用自行构建的便携包，完整解压后运行 `AnPod.exe`；运行依赖需要与 EXE 放在同一目录。

### USB 连接

1. 在手机的开发者选项中开启 **USB 调试**。
2. 用数据线连接电脑，在手机上确认 **允许 USB 调试**。
3. 打开 AnPod，选择 **USB 连接**，点击 **刷新**。
4. 选择已授权的手机，点击 **连接手机**。

连接后可通过 **从手机播放声音** 开关切换播放。结束时点击断开连接，或从托盘菜单退出程序。

### Wi-Fi 连接

1. 让手机和电脑连接同一网络，在手机的开发者选项中开启 **无线调试**。
2. 在 AnPod 中选择 **Wi-Fi 无线**。
3. 手机上选择 **使用配对码配对设备**，将弹窗中的 `IP:配对端口` 和 6 位配对码填入 AnPod，点击 **配对**。
4. 配对完成后搜索设备，或从手机无线调试主页面填写 `IP:连接端口`，点击 **连接**。
5. 手机出现在设备列表后，选择它并点击 **连接手机**，开始音频播放。

**配对端口和连接端口不同。** 配对负责授权，连接负责建立 ADB 链路，两者都不等于已经开始传输音频。搜索发现多个设备时，请确认并使用目标手机的地址。

使用 Windows 移动热点时，先在系统设置中开启热点并让手机加入，再按上述步骤连接。网络必须允许设备互访；开启客户端隔离的公共 Wi-Fi 通常不适用。

## 源码构建

### 准备工具链

- .NET 8 SDK。
- Visual Studio 2022 的 **使用 C++ 的桌面开发** 工作负载与 CMake；原生构建脚本也支持 MinGW-w64。
- JDK 17、Android SDK（API 34 和 Build-Tools）；Android 构建支持 Gradle 或直接使用 SDK 工具。
- Android SDK Platform-Tools：将同一发行包中的 `adb.exe`、`AdbWinApi.dll`、`AdbWinUsbApi.dll` 放入 `deps/`。

运行依赖不提交到 Git。文件用途与准备要求见 [deps/README.md](deps/README.md)，已记录的上游版本见 [SOURCE_VERSIONS.md](upstream/SOURCE_VERSIONS.md)。

### 编译并运行

在 Windows PowerShell 中执行：

```powershell
git clone https://github.com/sanren1/AnPod.git
cd AnPod

# 编译 Windows 音频采集 DLL，输出 deps/audio_capture.dll
powershell -ExecutionPolicy Bypass -File packaging\build-native.ps1

# 从仓库源码编译 Android 服务载荷，输出 deps/server
powershell -ExecutionPolicy Bypass -File packaging\build-android.ps1

# 编译并运行桌面程序
dotnet build AnPod.sln -c Release
dotnet run --project src/AnPod.Desktop -c Release --no-build
```

Android SDK 或 JDK 不在默认位置时，可为 `build-android.ps1` 指定 `-AndroidSdkRoot` 和 `-JavaHome`。`deps/server` 是含有 `classes.dex` 的载荷，通过 `app_process` 运行，无需安装到手机。

只构建 .NET 项目也能启动界面，但缺少 ADB、采集 DLL 或 Android 载荷时，无法完成音频连接。

### 检查与打包

```powershell
# 运行不依赖手机的 Core 自检
dotnet run --project tests/AnPod.Core.Tests -c Release

# 生成 x64 便携包，输出到 artifacts/
powershell -ExecutionPolicy Bypass -File packaging\publish.ps1 -Version 1.0.0
```

打包前需备齐 `deps/` 中的五项运行依赖。发布脚本遇到缺失依赖只会警告，生成的包需要核对后再使用。USB、无线连接、播放和断线恢复的人工验证步骤见 [手动检查清单](tests/manual-checklist.md)。

应用图标由根目录 `icon.png` 生成，EXE、窗口与托盘共用多尺寸 ICO。更新 PNG 后运行以下命令（需要 Python 和 Pillow）：

```powershell
python packaging\make-icon.py
```

## 工作原理

```text
Windows 默认输出设备
       │ WASAPI Loopback
       ▼
audio_capture.dll ── PCM16 ── 本机 TCP ── ADB reverse ── Android AudioTrack
                                             │
                                         USB / Wi-Fi
```

USB 与 Wi-Fi 复用同一条音频链路，仅 ADB 的设备连接方式不同。Windows 音频监听地址为 `127.0.0.1`，Android 接收服务由 AnPod 推送并启动。

| 目录 | 内容 |
| --- | --- |
| `src/AnPod.Core/` | 设备模型、ADB 解析与其他纯逻辑 |
| `src/AnPod.Desktop/` | WPF 界面、连接管理、配置和托盘 |
| `src/AnPod.Native/` | C++ WASAPI 音频采集 DLL |
| `src/AnPod.AndroidServer/` | Java 无界面音频接收服务 |
| `tests/` | Core 自检与真机验收清单 |
| `packaging/` | 构建、打包及图标生成脚本 |
| `tools/` | 界面截图与音频诊断工具 |
| `upstream/` | 上游来源、版本及修改记录 |

## 常见问题

| 问题 | 排查方式 |
| --- | --- |
| USB 手机未出现 | 检查数据线、USB 调试授权和设备驱动，再点击刷新。 |
| 配对成功但未连接 | 使用无线调试主页面的连接端口，而非配对弹窗中的端口。 |
| 搜索不到无线设备 | 确认网络可互访、无线调试已开启；mDNS 不可用时手动填写连接地址。 |
| 无线掉线后连不上 | 检查手机是否息屏、切换网络或关闭无线调试；连接端口可能已变化。 |
| 已连接但手机没声音 | 检查手机媒体音量、蓝牙音频路由及 Windows 默认输出设备。 |
| 程序异常结束后电脑静音 | 重新启动 AnPod 以恢复记录的静音状态，也可在系统音量设置中手动取消静音。 |
| 提示缺少运行依赖 | 按 `deps/README.md` 准备依赖，再重新构建或打包。 |

配置位于 `%LocalAppData%\AnPod\settings.json`，日志位于 `%LocalAppData%\AnPod\logs\`。可通过界面的 **查看日志** 入口打开。报告问题时请附上复现步骤、连接方式、Windows / Android 版本及相关日志片段，并检查是否包含个人设备信息。

## 参与贡献

欢迎通过 [Issues](https://github.com/sanren1/AnPod/issues) 提交问题与功能建议，或通过 Pull Request 提交改动。修改前请先查看现有实现；涉及连接逻辑时运行 Core 自检，涉及设备、音频或 UI 时补充对应的人工验证结果。真机尚未验证的行为请明确标注。

## 许可与致谢

音频采集和 Android 播放服务基于 [ysbing/AudioShare](https://github.com/ysbing/AudioShare) 的 LGPL-3.0-or-later 源码修改而来。上游版本与改动范围记录在 [upstream/](upstream/)，完整许可文本位于 [LICENSES/](LICENSES/)。

第三方组件说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。当前仓库尚未为项目自有代码声明独立许可证；第三方许可不代表整个项目采用相同许可。再分发时请核对各组件对应的条款与通知。
