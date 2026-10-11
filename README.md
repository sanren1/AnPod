# AnPod

AnPod 将 Windows 当前默认播放设备的声音实时传到 Android 手机。音频通过 WASAPI Loopback 采集，经 USB 或 Wi-Fi ADB 转发；手机端无需安装可见 App。

## 功能

- USB 和 Wi-Fi ADB 连接，支持无线调试二维码、配对码和手动地址。
- 手机播放、电脑播放切换；断开或暂停后恢复此前未静音的电脑音频端点。
- 设备重新出现后自动连接；Wi-Fi 音频会话掉线后进行有限次数恢复。
- 系统托盘、主题、大字体和本地诊断日志。

## 运行要求

| 项目 | 要求 |
| --- | --- |
| 电脑 | Windows 10/11 x64 |
| WebView | Microsoft Edge WebView2 Runtime；便携包不包含该运行时 |
| 手机 | Android 6.0+；无线调试配对需要 Android 11+ |
| USB | 支持数据传输的 USB 线，已开启并授权 USB 调试 |
| Wi-Fi | 手机与电脑处于可互访的网络，无线调试已开启 |

AnPod 需要可用的 Windows 默认音频输出设备。手机不会显示为 Windows 音频设备。ADB 授权会授予电脑 ADB 调试能力；请只连接信任的电脑和手机。

## 使用便携包

从 `artifacts/` 解压完整 ZIP 到可写目录并运行 `AnPod.exe`。`adb.exe`、Platform-Tools DLL、`audio_capture.dll` 和 Android `server` 载荷必须与 EXE 同目录；请勿单独移动 EXE。包内含第三方许可与通知。首次运行需要系统已安装 WebView2 Runtime。

## 从源码构建

主要桌面应用使用 Tauri 2、Rust、React、TypeScript 和 Vite。Windows 原生音频采集继续使用 C++ WASAPI DLL，Android 服务使用 Java。需要：

- Rust stable（至少 1.90）、Cargo 和 MSVC x64 C++ Build Tools。
- Node.js 与 npm。
- JDK 17 与 Android SDK API 34 / Build-Tools，用于重建 Android 服务。
- Android SDK Platform-Tools，用于准备 ADB 运行文件。

准备本机运行依赖：

```powershell
# 在 Visual Studio x64 Native Tools PowerShell 中执行
powershell -ExecutionPolicy Bypass -File packaging\build-native.ps1
powershell -ExecutionPolicy Bypass -File packaging\build-android.ps1
```

将受信任 Platform-Tools 同一发行包中的 `adb.exe`、`AdbWinApi.dll`、`AdbWinUsbApi.dll` 和该发行包的 `NOTICE.txt` 放入 `deps/`（NOTICE 文件命名为 `PLATFORM_TOOLS_NOTICE.txt`）。其余两个文件由上面的脚本生成。运行依赖不会提交到 Git，文件说明见 [deps/README.md](deps/README.md)。

构建前端：

```powershell
cd frontend
npm ci
npm run build
```

从仓库根目录创建 Windows x64 便携包：

```powershell
powershell -ExecutionPolicy Bypass -File packaging\publish-tauri.ps1 -Version 1.0.4
```

ZIP 和解压目录会写入 `artifacts/`。脚本会检查运行依赖、构建 React 前端和 Rust 桌面程序，并将许可证、上游记录和运行文件放进同一目录。打包脚本只构建 Tauri 主应用；WPF 回退版本保留在 `src/AnPod.Desktop/`，其旧打包入口为 `packaging/publish.ps1`。

开发运行时由 Tauri 启动 Vite，在仓库根目录中执行：

```powershell
npm run tauri --prefix frontend -- dev
```

图标由根目录 `icon.png` 统一生成给 Tauri、网页 UI 和 WPF 回退程序：

```powershell
python packaging\make-icon.py
```

脚本需要 Python Pillow。它会裁去透明空白并保留原图比例；Windows 桌面 ICO 与 PNG 会更新，Android/iOS 模板图标不属于 Windows 发布源。

## 检查

```powershell
# 纯逻辑自检
dotnet run --project tests/AnPod.Core.Tests -c Release

# Tauri Rust 编译与单元检查
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

可选 UI 检查见 [前端检查说明](frontend/README.md)。连接、音频质量、延迟与掉线恢复仍需按 [手动检查清单](tests/manual-checklist.md) 在目标 Windows / Android 设备上验收。

## 工作原理

```text
Windows 默认输出设备
       │ WASAPI Loopback
       ▼
audio_capture.dll ── PCM16 ── 本机 TCP ── ADB reverse ── Android AudioTrack
```

| 路径 | 内容 |
| --- | --- |
| `frontend/` | React / TypeScript 界面 |
| `src-tauri/` | Tauri 命令、生命周期与 Windows 桌面配置 |
| `src/AnPod.Native/` | C++ WASAPI 音频采集源码 |
| `src/AnPod.AndroidServer/` | 手机端 Java 音频服务源码 |
| `src/AnPod.Desktop/` | 保留的 WPF 回退实现 |
| `packaging/` | 原生/Android 构建、图标和便携包脚本 |
| `tests/` | 自检、前端检查与人工设备验收清单 |

## 配置和日志

用户配置及静音恢复记录位于 `%LocalAppData%\AnPod\`，日志位于 `%LocalAppData%\AnPod\logs\`。可以在应用的诊断面板打开日志目录。提交诊断信息前请检查其中的设备标识和网络地址。

## 许可

音频采集与 Android 播放服务基于 ysbing/AudioShare 的 LGPL-3.0-or-later 源码修改而来。第三方许可见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)、`LICENSES/` 与 `upstream/`。项目自有代码尚未声明独立许可证；再分发时应逐项核对第三方许可与通知要求。
