# 手动验收清单

下表的设备链路和 WPF 窗口记录来自 2026-10-10，属于历史验收，不能视为
Tauri 1.0.4 的真机通过记录。Tauri 使用后台设备检测，旧表中“空闲不轮询”
的结论仅适用于当时的 WPF 实现。

2026-10-11 的 Tauri 前端检查使用真实 React 组件和模拟 IPC：覆盖六种状态、
两种主题和两种窗口尺寸，以及输入焦点、关闭配对弹窗后的迟到结果、断开连接、
播放模式/连接方式确认与取消、大字体布局。Core 逻辑检查为 41/41 通过。
这些检查不验证真实 Windows 托盘显示、USB 拔插、手机声音或端到端播放延迟；
对应项目需要在新便携包中重新验收。

来自 `plan v1.1.md` §9 的 T01–T20 测试矩阵与无线专项 W01–W21。
自动化能覆盖的部分在 `tests/AnPod.Core.Tests`（ADB 解析、USB/网络判定、
无线 `pair`/`connect` 输出判定、配置容错、日志打码），其余必须在真机上执行。

**状态标记：** `☐ 未测` · `✅ 通过` · `⚠️ 已知限制` · `❌ 失败`

在开始之前，先在 `deps/` 里放好五个运行时文件（见 `deps/README.md`）。

---

## 前置

| 项 | 值 |
|---|---|
| 测试机（Windows） | Windows，x64 |
| 测试机（Android 型号 / ROM / 版本） | Android 测试设备（型号与 ROM 标识省略），Android 16（API 36），arm64-v8a |
| ADB 版本 | 1.0.41 / platform-tools 37.0.1-15733141 |
| 原生 DLL 版本 | `audio_capture.dll`（MinGW-w64 g++ 16.2.0 构建，x64，99 868 B，11 个导出） |
| 服务载荷版本 | `deps/server`（classes.dex 6 860 B，DEX 035） |
| 测试日期 | 2026-10-10 |

> 本轮测试由自动化工具驱动：`E2E` 集成探针（Windows↔Android 全链路）与
> 真实应用 + 故障注入。人工需要动手的项目（拔线、熄屏、托盘点击）仍需补测。

---

## P0 — 核心路径

| 编号 | 动作 | 预期 | 状态 | 备注 |
|---|---|---|---|
| T01 | 不接手机，启动 AnPod | 显示空列表与「请用 USB 数据线连接手机…」；空闲 CPU 低 | ☐ | 需要拔线，未测。空闲时**不再轮询**，CPU 开销已由设计消除（见 T24） |
| T02 | 手机已连但未授权 | 显示「等待手机授权」，**不**尝试采集 | ☐ | 需在手机上撤销授权，未测。`SyncIdleState` 的 `Unauthorized` 分支已实现（本轮新补，此前缺失） |
| T03 | 单台已授权 USB 手机，点「开始播放」 | 手机扬声器播放系统声音 | ☐ | 需要出声，未测。链路各环节已分别验证 |
| T05 | 播放 48 kHz 视频/音乐 | 正常立体声，无爆音 | ☐ | 需要出声，未测 |
| T07 | 播放中拔掉 USB | 停止采集，界面显示断线；**无残留 adb 进程** | ☐ | 需要拔线，未测。`HandleSessionLossAsync` 路径已实现 |
| T09 | 用户点「断开连接」 | 不被立即自动重连 | ☐ | 需要点界面，未测 |
| T14 | `adb devices` 显示 offline | 不误判为已连接；有限次重试 | ☐ | 未构造 offline 场景。`Offline` 不满足 `IsConnectable`，已由单元测试覆盖 |
| T15 | 先用其它工具建立一条 reverse 映射，再让 AnPod 连接并退出 | AnPod 退出后，**别人的映射仍在** | ✅ | 集成探针验证：`localabstract:someothertool_session` 在 AnPod 建/拆映射前后均存在；AnPod 只移除自己的 `anpod_e2e` |
| T17 | 托盘「退出」 | 停止采集、移除本会话映射、关闭 app_process；无残留进程 | ✅ | 集成探针 + 真实应用故障注入两条路径均确认：reverse 清空、设备上 `app_process` 计数为 0、`/data/local/tmp` 无残留进程。**未**点击托盘退出菜单项 |
| T19 | 让 Android 服务立刻崩溃（例如 push 一个损坏载荷） | 超时/失败转 Error，界面可再次连接 | ✅ | 真实应用 + 损坏载荷（随机字节冒充 classes.dex）：`app_process` 立即 abort；应用在 **~350 ms** 内给出「手机端音频服务启动后立即退出，可能不受当前系统支持。」，而不是白等 10 s 超时。失败后 reverse 映射已清理，状态可再次连接 |

## P1 — 稳定性与兼容性

| 编号 | 动作 | 预期 | 状态 | 备注 |
|---|---|---|---|---|
| T04 | 同时接两台已授权手机 | 列表出现两台；**不自动选中**，只连接用户选中的那一台 | ☐ | 只有一台设备可用，未测。多条设备时 `SyncDevices` 刻意不自动选择 |
| T06 | Windows 默认端点设为 44.1 kHz | 无速率漂移、无周期性断音 | ☐ | 需要出声，未测。DLL 已按 `GetMixFormat` 自适应并在 8000–192000 Hz 内校验 |
| T08 | 开启「自动连接上次设备」后重插 | 识别原设备并恢复 | ✅ | 应用启动后日志出现 `Auto-connecting last device`，随即走完 push → app_process → 握手。**本轮修复后才可用**（此前自动连接永远不会触发） |
| T10 | 连续插拔 20 次 | 无死锁、无残留资源 | ☐ | 需要拔线，未测 |
| T11 | 手机熄屏 30 分钟 | 不中断，或明确提示兼容性限制 | ☐ | 未测 |
| T12 | Windows 睡眠后唤醒 | 能重新识别并恢复 | ☐ | 未测 |
| T13 | 连接前先占用候选端口 | 自动换端口，不异常退出 | ☐ | 未测。`ConnectAsync` 已实现 5 次换端口重试 |
| T16 | 关闭窗口 | 留在托盘，音频继续 | ☐ | 需要点界面，未测 |
| T18 | 禁用所有 Windows 播放设备 | 明确提示，不崩溃 | ☐ | 未测。`ANPOD_ERR_NO_ENDPOINT` 已实现并有对应文案 |
| T20 | 空闲 30 分钟 + 连续播放 2 小时 | 记录 CPU / 内存 / 延迟 / 断流次数 | ☐ | 未测 |

## P2 — 两个控件相互独立（本轮新增）

| 编号 | 动作 | 预期 | 状态 | 备注 |
|---|---|---|---|---|
| T21 | 播放中关掉「从手机播放声音」 | 手机静音、电脑扬声器恢复；**手机保持连接**（状态为「已连接」，按钮仍是「断开连接」） | ✅ | `tools/SessionProbe` 真机验证：状态 `Connected`、reverse 映射仍在、`app_process` 载荷仍在、本机端点已恢复。**听感**（手机确实无声）未由人工确认 |
| T22 | 上一步之后立刻再打开开关 | 立即恢复播放，**不重新握手、不重启载荷** | ✅ | `tools/SessionProbe`：仍只有一条 reverse 映射、只有一个载荷进程（`sh -c` 包装不算），且无新的握手 |
| T23 | 关掉开关后点「断开连接」 | 正常断开，无残留 | ✅ | `tools/SessionProbe`：`Ready`、映射清空、载荷进程归零、端点恢复 |
| T24 | 空闲（无会话）时静置 | **不**轮询 `adb devices`；设备列表只在点「刷新」时变化 | ✅ | `tools/SessionProbe` 空闲 8 秒，`DevicesChanged` 触发 0 次 |
| T25 | 关掉开关后拔线，再插回 | 不静默恢复播放：重连后仍是「已连接（未播放）」 | ☐ | 需要拔线，未测。`HandleSessionLossAsync` 已记住 `wasPlaying` 并在重连后恢复该状态 |
| T26 | 界面版式（空列表 / 已就绪 / 播放中 / 已暂停 / 已断开 / Wi-Fi 面板 / Wi-Fi 帮助展开） | 七种状态版式正确，控件不重叠、不溢出 | ✅ | `tools/UiSnapshot` 渲染为 PNG 逐一核对（`06-wifi.png`、`07-wifi-help.png` 为本轮新增）；另有 `artifacts/check-xaml.py` 静态检查资源键与绑定路径 |

---

## P3 — 无线专项（W01–W16）

全部依赖真实无线环境（Android 11+ 真机 + 同 LAN 或 Windows 移动热点）。
**W01–W16 均未真机验收**：本批只完成代码、Core 断言和离线界面/二维码校验；缺 Android 11+ 真机及可控无线环境。

自动化覆盖了配对/连接解析、二维码凭据、mDNS 精确匹配、TLS serial 和传输分类等纯逻辑路径（`tests/AnPod.Core.Tests`）。

* `adb connect` 失败时**退出码仍为 0** —— 必须靠输出文本判定，不能靠退出码。
* `already connected` 与 `connected to` 与失败文案的区分顺序。
* `adb pair` 的成功 / 拒绝（密码错）分别识别。
* `IP:端口` 的归一化与非法输入拒绝；配对码必须是恰好 6 位数字。
* 配置里 `preferredTransport` / `autoReconnectWifi` / `lastWirelessEndpoint`
  的往返；旧 v1.0 配置（无这三个字段）仍能加载；非法传输值回退 `Usb`、
  非法地址回退 `null`。

| 编号 | 动作 | 预期 | 状态 | 备注 |
|---|---|---|---|---|
| W01 | Android 11+ 同 LAN，用配对码完成首次无线配对 | 配对成功；配对码**不落盘**（`settings.json` 与日志里都搜不到） | ☐ | 未测。手动配对码在执行 `adb pair` 前注册为日志密钥；QR 共享密码也只在本次流程内注册并在结束时移除 |
| W02 | 配对端口 ≠ 连接端口 | UI 分别标注手动配对地址、连接地址；连接使用连接端口 | ☐ | 未测。手动地址分成两个输入框；二维码弹窗也分别显示配对地址和连接地址 |
| W03 | 关掉 mDNS 发现，手动填 `IP:连接端口` | 能完成连接 | ☐ | 未测。手动输入是**始终可用**的兜底路径（plan §5.10），也是搜索不可用时的唯一出路 |
| W04 | Windows 移动热点：手机加入热点后无线连接并播放 | 正常播放 | ☐ | 未测。热点只提供 IP 链路，音频仍走 ADB Reverse；AnPod 只给跳转入口（`ms-settings:network-mobilehotspot`），不改系统设置 |
| W05 | Wi-Fi 播放 30 分钟 | 无累积延迟、无长时间静音、无崩溃 | ☐ | 未测 |
| W06 | WLAN 断开 / 恢复 | 状态准确；可控重连；**主动断开后不重连** | ☐ | 未测。重连路径：`HandleSessionLossAsync` 记住 `wasWireless` → 退避循环里先 `adb connect` 再刷新设备列表。主动断开由 `_suppressedAutoConnectSerial` 抑制 |
| W07 | 手机 IP / 端口变化 | 不无限重试旧地址；允许重新发现或重新输入 | ☐ | 未测。重试有上限（1s/2s/5s 三次）；`LastWirelessEndpoint` 只是提示，界面永远可以改 |
| W08 | USB 与 Wi-Fi 同时识别同一台手机 | 单实例会话；不双重 push；不错误选设备 | ☐ | 未测。两条记录是**两个不同的 serial**（`TESTUSB…` 与 `ip:port`）；列表**按当前模式过滤**，USB 模式只列 USB 那条、Wi-Fi 模式只列无线那条，所以同一台手机不会在同一个列表里出现两次。`ConnectCoreAsync` 只对用户选中的那个 serial 下命令 |
| W09 | USB ↔ Wi-Fi 切换 | 有序清理旧通道；可重新播放 | ☐ | 未测。会话进行中连接方式不可切换（要先前断开）；`TeardownSessionAsync` 定点移除本会话 reverse 映射 |
| W10 | 关闭 Android 无线调试后点「连接」 | 提示需重新开启或重新配对，**不**提示 USB 故障 | ☐ | 未测。`DescribeConnectFailure` 刻意不提 USB——那不是数据线问题，说 USB 会把用户引到错的方向 |
| W11 | 非可信 / 隔离的公共 LAN | 提示网络限制；不绕过安全设置 | ☐ | 未测。帮助文案点名「客户端隔离」；AnPod 不关防火墙 |
| W12 | Android 10 及以下的 `adb tcpip` 引导（可选） | 仅主动触发，有安全说明和关闭方式 | ⚠️ | **v1.1 未实现，按计划范围记为可选兼容。** 界面不提供该入口，也从不调用 `adb tcpip`。旧 Android 请继续用 USB |
| W13 | 同 LAN 内开启无线调试后点 **连接 / 搜索** | 读出手机广播的 mDNS 服务，连接端口自动填好 | ☐ | 未测。仅在没有已配对目标且唯一设备时自动填入；已有关联目标时要求主机匹配，其他端点不代选 |
| W14 | ADB 服务的 mDNS 组件僵死时点 **搜索设备** | 明确提示「搜索不可用 / 没有响应，请手动填地址」，**不**报成「没有搜索到设备」 | ☐ | 未测（但**僵死状态已在本机实测复现**，见 D7）。搜索不可用时手动填地址仍然可用 |
| W15 | 切换 USB / Wi-Fi 模式看设备列表 | 两个模式的列表**互相独立**，同一台手机不会两边都出现 | ☐ | 未测。过滤逻辑在 `DeviceInfo.MatchesTransport`，已有单元测试（`FiltersDevicesByMode`）；快照 `06-wifi.png` 可见 Wi-Fi 模式下 USB 手机已不在列表里 |
| W16 | 展开 **连接选项**，地址为空时输入手动配对码 | 地址、配对码可输入并可手动配对 | ☐ | 未测。连接选项默认折叠以压缩主窗；输入框不依赖已有连接地址才可编辑 |

## P4 — 二维码配对（W17–W21）

均需 Android 11+ 真机完成端到端验收；Core 断言与离线渲染不等于真机通过。

| 编号 | 动作 | 预期 | 状态 | 备注 |
|---|---|---|---|---|
| W17 | 点 **扫码配对**，手机选择「使用二维码配对设备」并扫码 | 手机配对成功、ADB 列表出现扫码目标；不开始音频播放 | ☐ | 未真机验证。QR 码使用 AOSP `WIFI:T:ADB;S:studio-…;P:…;;` 协议 |
| W18 | 局域网有另一台无线调试手机时扫描本次二维码 | 只按本次二维码的 mDNS service instance 配对扫码手机 | ☐ | 未真机验证。纯逻辑已覆盖精确实例匹配；目标/其他设备无法唯一关联时不会自动选择 |
| W19 | 扫码等待时按 Esc、点取消、关闭窗口或退出应用 | 扫描和 ADB 配对进程取消；无后台轮询；再次打开生成新凭据 | ☐ | 受控 fake adb 已验证配对进程随取消终止、后续步骤不执行、重试使用新流程并成功；Esc/窗口关闭仍未做桌面 UI 自动化或真机验收 |
| W20 | 等待扫码两分钟无操作 | 显示超时，可关闭后重新生成二维码 | ☐ | 未真机验证 |
| W21 | 查看二维码 UI 与扫码结果 | 二维码清晰、有白边；明确区分配对地址/连接地址；显示等待、配对、成功/失败 | ☐ | PNG 渲染和 QR payload 解码为离线检查，不代替手机扫码 |

---

## 已修复缺陷

本表记录此前已修复项与本批审查修复项。

| # | 缺陷 | 影响 | 修复 |
|---|---|---|---|
| D1 | `usb:` 标签在 platform-tools 37 的 `adb devices -l` 中不再输出 | 设备被判为「连接方式待确认」，`IsConnectable = false` → **无法连接**，自动连接也不触发 | `DeviceInfoFactory` 增加二次确认：读取 `sys.usb.state`（如 `mtp,adb`）作为 USB 正向证据。该字段并入**已有的**同一次 `getprop` shell 调用，零额外往返（plan §5.2 要求的「其他 ADB 信息验证」） |
| D2 | 状态机缺少 `NoDevice → Unauthorized / Ready` 转换 | 状态永远停在 `NoDevice`；自动连接以 `Ready` 为门槛 → **自动连接永不触发**；`Unauthorized` 从未被设置过 | `ConnectionManager.SyncIdleState` 依据设备列表推导空闲态，仅在 `NoDevice`/`Ready`/`Unauthorized` 之间迁移，不干扰 `Error` 与进行中的会话（plan §5.3） |
| D3 | `LastDeviceId` / `AutoConnectLastDevice` 只在状态变化回调里推给管理器 | 启动时 `LastDeviceId` 为空，自动连接缺少目标 | `MainViewModel` 构造函数显式播种（属性 setter 在同值时短路，原写法是空操作） |
| D4 | 握手等待不响应 Android 进程退出 | 载荷立即崩溃时要白等 10 s，再给出泛化的「连接超时」；已写好的精确提示**不可达** | `WaitForHandshakeAsync` 订阅 `session.Exited` 立即完成等待；调用方把 `HasExited` 判断提到令牌比对之前 |
| D5 | `AndroidProcessSession` 在 `Start()` **之后**才订阅 `Process.Exited` | 瞬间退出的进程可能丢事件，使 D4 的快速失败失效 | 订阅移到 `Start()` 之前 |
| D6 | 便携包 zip 条目使用反斜杠（`LICENSES\ADB-NOTICE.txt`） | 违反 ZIP 规范（APPNOTE 4.4.17.1），严格解压器会生成字面含反斜杠的文件名 | `publish.ps1` 手工写条目并归一化为 `/` |
| D7 | 用 `adb mdns services` 判断 mDNS 组件是否可用 | 组件僵死时它**照样立刻返回空列表、退出码照样是 0**（本机实测 0.05 s，与健康时无法区分），于是「搜索坏了」被报成「没有搜索到设备」→ 用户去查一台本来没问题的手机 | 先跑 `adb mdns check` 探测，确认组件在应答后才去读设备列表。实测三态：健康 0.05 s、冷启动 2.14 s、僵死 11.25 s，超时取 5 s（必须大于冷启动、远小于僵死）。探测失败时明确报告「搜索不可用，请手动填地址」。`IsMdnsUnavailable` 的文档注释已标明**只能**用于 `mdns check` 的输出 |
| D8 | 任务栏图标偏小 | WPF `BitmapFrame.Create` 只取多帧 `.ico` 的**第 1 帧**（16×16），Windows 再把它放大到任务栏尺寸 | `AppIcon.LoadWindowIcon` 改用 `BitmapDecoder`，按像素面积取最大帧（256×256）后冻结 |
| D9 | 图标画布留白过多，进一步显小 | 源 PNG 全画布有一层 alpha=1 的雾，`getbbox()` 因此永不裁剪（1254×1254 → 1254×1254）；图形只占画布高度约 43% | `make-icon.py` 先按 alpha 阈值（8）再求 `getbbox()`，裁到实际墨迹并重新居中（1254 → 940）；图形占宽 95% × 高 59% |
| D10 | 配对码输入框在地址为空时被禁用 | `settings.json` 的 `lastWirelessEndpoint` 默认为 `null`，所以 Wi-Fi 模式一启动地址栏就是空的；而 `IsPairingCodeEnabled` 要求地址非空 → **配对码框每次启动都是灰的**，用户报「无法点击无法填写」 | 新增 `IsWifiInputEnabled`（只受「会话进行中」限制），地址框、配对码框与重连复选框都改绑它；按钮的可用性仍由 `IsWifiPanelEnabled`（含「操作进行中」）控制，所以「配对」不会在缺地址时被误触发 |
| D11 | 自动连接忽略当前模式 | 存下来的序列号属于另一种连接方式时（切换模式后通常如此），仍会自动开会话：用户界面停在 Wi-Fi 面板，实际却连上了 USB 手机——这正是「两个模式显示的是同一个」的根因 | `TryAutoConnect` 增加 `d.MatchesTransport(PreferredTransport)`；同时设备列表改为按模式过滤（`DeviceInfo.MatchesTransport`），两种模式的列表与选中项彻底独立 |
| D12 | `sys.usb.state=mtp,adb` 覆盖网络 serial | 同一手机同时插 USB 与 Wi-Fi 时，网络 ADB 行可能被误分类为 USB 并从 Wi-Fi 列表过滤 | `DeviceInfoFactory` 先按当前 ADB 行的明确 `usb:` tag / IP 或 TLS mDNS serial 分类，再把设备全局 USB 状态仅用于无传输证据的 serial |
| D13 | TLS 无线调试 serial 不属于 IPv4:端口格式；唯一 mDNS 端点也可能不是刚扫码设备 | QR 配对后设备被标为 Unknown，或在目标广播未出现时误填另一手机地址 | shared serial parser 识别 `_adb-tls-connect._tcp`；mDNS parser 单独提取 instance；明确目标时必须 host 匹配，单个不匹配端点也不自动选择 |

## 已知限制登记

未通过的 P1 项在这里登记，必须说明是否影响核心路径。

| 编号 | 限制描述 | 影响核心路径？ | 处理决定 |
|---|---|---|---|
| — | 会话结束后 `/data/local/tmp/anpod_server` 保留在手机上 | 否 | 有意为之：作为载荷缓存，避免每次连接重复 push（plan §5.3 允许）。如需清除可手工 `adb shell rm` |
| W12 | 未实现旧 Android（≤10）的 `adb tcpip` 引导流程 | 否 | 按 `plan v1.1.md` §1.1a 场景 C 属**可选**兼容，且默认不得自动开启。界面不提供入口，也从不调用 `adb tcpip`；旧设备继续用 USB |
| W01–W21 | 无线与 QR 专项未真机验收（缺 Android 11+ 真机与可控无线环境） | 否（USB 路径不受影响） | 代码、Core 断言、离线窗口渲染和 QR payload 解码不代表端到端通过。真机验收前，无线和扫码配对均标记为**未验证**；mDNS 僵死状态另有本机历史复现（见 D7） |
| — | 本机 5353 端口有 6 个以上进程在监听（Bonjour 等），ADB 服务的 mDNS 组件长期处于僵死状态 | 否 | 环境问题，非代码缺陷。AnPod 探测到后如实报告并引导手动填地址；**不会**用 `kill-server` 去修（plan §3.2 明令禁止）。手动填 `IP:端口` 是完整可用的替代路径 |
| — | 手机「无线调试」在重启、Wi-Fi 断开、长时间无设备连接后会自动关闭 | 否 | Android 的设计行为，应用侧无法改变。README §8.1 已列出可缓解的系统设置，并说明**配对是一次性的、端口不是**，重开无线调试后不必重新配对 |

---

## 每次发布前必查

- [x] `upstream/SOURCE_VERSIONS.md` 里没有「not pinned」
- [x] `deps/server` 由本仓库源码构建（不是网上下载的二进制）
- [x] `unzip -l deps/server | grep classes.dex` 有输出
- [x] `deps/audio_capture.dll` 是 x64
- [x] 便携包内包含 `LICENSES/`、`THIRD_PARTY_NOTICES.md`、`upstream/`
- [ ] P0 全通过；P1 未通过项已登记为已知限制
      —— **未完成**：T01/T02/T03/T05/T07/T09/T14 需要人工动手（拔线、点界面、出声），
      尚未执行。T15/T17/T19 已通过。
- [ ] W01–W21 无线与 QR 专项完成（`plan v1.1.md` §12 DoD #10）
      —— **未完成**：缺 Android 11+ 真机与可控无线环境。W01–W16 均未真机验收；W17–W21
      的扫码与目标设备关联仍未真机验证；W19 的 service-level fake adb 取消/重试检查已通过，但 Esc/窗口关闭未做 UI 自动化。Core 断言、QR 解码和离线界面渲染不代表端到端通过；
      W12 按可选兼容登记，W14 的 mDNS 僵死状态有本机历史复现（D7）。
- [ ] `LICENSES/ADB-NOTICE.txt` 的条款复核（人类/法务步骤，见该文件）
