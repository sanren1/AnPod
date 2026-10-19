# AnPod AOA USB Direct — 第二阶段独立验证工具（实验性）

**适配验证对象：** iQOO 11 (V2243A) / Android 16；原 USB 设备 `VID_2D95&PID_6013`，ADB `MI_02`，WinUSB `oem55.inf`。你已实测官方 `GET_PROTOCOL (51)` 返回 **AOA v2**。Windows 10/11 x64。

> **先读风险：** 本工具有能力主动向手机发送 AOA `SEND_STRING (52)`、`START_ACCESSORY (53)`；手机会重枚举，**ADB/MTP 暂时消失属预期行为**。切换前必须安装 Android 测试 APK、退出 AnPod、停掉 ADB，并手动输入确认词。它不会 Root、不会更改手机系统分区、不会静默安装或替换 Windows USB 驱动。断开并重新插入数据线通常即可恢复原 MTP/ADB 模式。测试代码未经 iQOO 11 真机执行验证，不对设备/驱动兼容性作保证。

## 交付文件

- `Windows/Run-AoaPhase2.ps1`：`Preflight` / `Switch` / `Inspect` / `Bulk` 四个阶段；`Switch` 必须人工确认。
- `Windows/AoaHost.cs`：SetupAPI / WinUSB 控制传输、Accessory 接口查找、Bulk 回显及统计。
- `AndroidProbe/`：独立 Android 应用源码 `app.anpod.aoaprobe`（不是 AnPod Receiver）；只接受指定的 `AnPod Lab / AOA Bulk Probe / 1.0` 配件身份，无网络、音频或读写存储权限。
- `Build-AndroidProbe.ps1`：在你拥有 AnPod Android 工程 Gradle Wrapper 时，用它编译独立测试 APK。
- `PROTOCOL.md`：Windows ↔ Android 回显格式、成功判据。

**限制：当前交付的是完整源码与可在 Windows PowerShell 上运行的脚本，不含预编译 APK。** 制作环境没有 Android SDK/Gradle/.NET Windows 环境，无法构建 APK 或执行 Windows USB 真机测试。请勿将它当作已验收的正式软件。

## 0. 先编译并安装手机测试 App

方式 A：Android Studio 打开本包 `AndroidProbe` 文件夹，等待 Gradle Sync，选择 `app > assembleDebug`。需要 JDK 17、Android SDK Platform 35、Gradle 8.9（兼容 AGP 8.7.3，也与 AnPod 当前 Wrapper 相同）。

方式 B：如果你已有 AnPod 源码中的 Gradle wrapper（`src\AnPod.Receiver\gradlew.bat`），可在解压根目录执行：

```powershell
.\Build-AndroidProbe.ps1 -GradlewPath 'D:\source\AnPod\src\AnPod.Receiver\gradlew.bat'
```

其中 `D:\source\AnPod\...` 只是占位例子，替换成你实际源码路径。该脚本编译本包的独立 Gradle 项目，不会构建或替换 AnPod Receiver；首次构建需要能够获得 Gradle 插件依赖。

APK 输出：`AndroidProbe\app\build\outputs\apk\debug\app-debug.apk`

在 **ADB 仍正常时**安装并打开：

```powershell
& 'F:\Download\platform-tools-latest-windows\platform-tools\adb.exe' install -r '.\AndroidProbe\app\build\outputs\apk\debug\app-debug.apk'
& 'F:\Download\platform-tools-latest-windows\platform-tools\adb.exe' shell am start -n 'app.anpod.aoaprobe/.MainActivity'
```

此时 App 显示 `No accessory` 是正常的。无需卸载或覆盖现有 AnPod Receiver。

## 1. 只读 Preflight（不切换）

完全退出 AnPod、Android Studio 的设备连接界面、scrcpy 等会重启 ADB 的应用；执行：

```powershell
& 'F:\Download\platform-tools-latest-windows\platform-tools\adb.exe' kill-server
Get-Process adb -ErrorAction SilentlyContinue
powershell -NoProfile -ExecutionPolicy Bypass -File '.\Windows\Run-AoaPhase2.ps1' -Mode Preflight
```

期望 `SUCCESS AOA protocol=2`。如果 `CreateFile Win32=5`，确认未重启 adb.exe；暂不替换驱动。

## 2. 人工确认后启动 AOA（会造成 USB 重枚举）

Android 手机保持解锁，预先安装测试 App，并确保有实体 USB 数据线可直接拔插。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File '.\Windows\Run-AoaPhase2.ps1' -Mode Switch
```

需输入 **`SWITCH AOA`**。此操作发送：

1. `GET_PROTOCOL (51)`，预期得到 AOA 2；
2. `SEND_STRING (52)`，发送 manufacturer=`AnPod Lab`，model=`AOA Bulk Probe`，version=`1.0` 等指定身份；
3. `START_ACCESSORY (53)`，触发 USB 重枚举。

后续识别 `VID_18D1 / PID_2D00` 或 `PID_2D01`。如果 53 请求返回“设备断开”一类错误，不一定意味着失败，应以重新枚举后的设备为准。**此时不再运行原来的 `adb devices` 来控制手机，因为 ADB 可能不再存在。**

Android 可能出现“允许 AnPod AOA Probe 访问 USB 配件？”或应用选择界面；允许测试 App 访问。若系统没自动启动，手动打开 **AnPod AOA Probe**，点击 `Find accessory / request permission / connect`，确保出现 `OPEN: USB accessory stream ready`。

## 3. 识别重新枚举的 USB 接口与驱动

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File '.\Windows\Run-AoaPhase2.ps1' -Mode Inspect
```

如果 Windows 识别到 `18D1:2D00` 或 `18D1:2D01` 但 `Inspect` 报 `No accessible WinUSB...`，说明 **AOA 设备已出现但当前尚没有可用的 WinUSB 设备接口**；这是 Android 端支持与 Windows 驱动绑定之间的区别。

**不要修改现有 `VID_2D95&PID_6013&MI_02` 的 ADB 驱动；也不要修改 `USB Composite Device (usbccgp)`、USB Root Hub、MTP 驱动。** 如果确需为新 AOA 接口安装 WinUSB，要先把 `-Mode Inspect` 输出发来，确认具体接口 InstanceId；仅在单独确认后的下一步为 **新出现的 `VID_18D1&PID_2D00` 单接口，或 `VID_18D1&PID_2D01&MI_00` Accessory 数据接口**选择 WinUSB。`MI_01` 可能是 ADB 接口，绝不可误替换。当前脚本 **不自动安装任何驱动**。

此步如果无法确认安全的接口与驱动归属，**在此停止，先不要运行 Bulk**。

## 4. USB Bulk 实际回显验证

手机端 App 显示 `OPEN: USB accessory stream ready`，Windows 端 `Inspect` 确认有 **Bulk OUT 和 Bulk IN** 两个端点时：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File '.\Windows\Run-AoaPhase2.ps1' -Mode Bulk -Rounds 64
```

脚本最多允许 `-Rounds 512`。Android 收到固定 4096 字节消息后先验证固定模式，再原样回显。Windows 按字节比较并计算每轮 RTT / P50 / P95 / 双向请求-回显 goodput。

**统计不等于音频端到端延迟、也不等于单方向 USB 理论吞吐率。** 只证明数据端点、完整性与短时回显能力。该协议不接入 WASAPI、PCM、AudioTrack 或现有 AnPod 服务。

## 5. 恢复正常 USB / ADB

1. 先停止 Windows Bulk 进程，手机端按 `Close USB stream`。
2. 物理拔出 USB 数据线，等待几秒，再插回。
3. 手机上按需要选择原来的文件传输或 USB 调试模式。
4. Windows 端验证：

```powershell
& 'F:\Download\platform-tools-latest-windows\platform-tools\adb.exe' start-server
& 'F:\Download\platform-tools-latest-windows\platform-tools\adb.exe' devices
```

如果 Android 不再出现原来的 `V2243A`，优先检查 USB 线缆、设备管理器和手机 USB 模式；不要直接卸载设备或盲目切换驱动。若之前人为为 AOA 的 Google VID/PID 安装过驱动，它属于另一设备实例，不应影响原 vivo VID/PID 的 ADB 绑定。所有实验完成后可卸载独立测试 App `app.anpod.aoaprobe`，**不要卸载 `app.anpod.receiver`**。

## 错误解读

| 结果 | 判断 |
|---|---|
| `Preflight SUCCESS protocol=2` | 设备级 AOA v2 握手通过 |
| 发送 53 后无 `VID_18D1` | 尚未证实重枚举成功，可能是系统或 USB 枚举限制 |
| `VID_18D1` 出现，但 `Inspect` 无路径 | 新配件数据接口缺可用 WinUSB 映射/驱动，需先确认设备实例 |
| `Inspect` 有 Bulk OUT + IN | WinUSB 管道访问具备条件，尚未验证应用通信 |
| `Bulk SUCCESS` | Windows↔Android Bulk 回显成功 |
| `WinUsb_ReadPipe` 超时 | 可能是 Android App 未打开/授权，或双方帧长度不匹配 |
| Android App 显示权限被拒绝 | 重新在手机上允许对应测试配件，别修改系统权限 |

## 官方参考

- AOA 握手与 PID: https://source.android.com/docs/core/interaction/accessories/aoa
- Android Accessory App: https://developer.android.com/develop/connectivity/usb/accessory
- Windows WinUSB: https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/using-winusb-api-to-communicate-with-a-usb-device

本实验**不复用**其他项目源代码，独立源码仅用于验证。后续是否接入 AnPod 取决于实际 Bulk 结果，以及与已有 USB ADB/USB 网络共享模式的相同音频引擎对照。
