# AnPod UI / UX 简化规划（基于 dev 1.10.5 / Receiver 0.17.4 code 38）

> 初稿为规划；第 8 节起记录实施，最新执行状态见文末。初稿结论来自当时 `dev` 代码：`frontend/src/App.tsx`（53 KB）、
> `frontend/src/components/NetworkReceiverDialog.tsx`（74 KB）、`AoaDialog.tsx`（15 KB）、
> `strings.zh-CN.ts`（40 KB）、`src/AnPod.Receiver/.../MainActivity.kt`（56 KB）。

## 1. 现状盘点

### PC 端（单窗口 + 9 个弹窗）
- 主界面顺序：品牌栏 → 目标手机卡 → 连接方式（USB/无线；USB 下再分 ADB / USB Direct）→ 状态行 → 主按钮 → 播放模式 → 底栏（帮助 / 版本 / 诊断）。
- 主按钮文案由 10 余个并列条件拼出（`playToPhone` / `connectUsbPhone` / `addWirelessPhone` / `backgroundStart` / `waitingForPhone` / `retryDisconnect` …），状态行另有 20 种文案。
- 弹窗：`DeviceDialog`、`NetworkReceiverDialog`、`AoaDialog`、`DiagnosticsDialog`、`SettingsDialog`、`PairDialog`、`HelpDialog`、`ConfirmDialog`，其中 `NetworkReceiverDialog` 一个文件同时承担首次设置向导、无线连接、接收端管理、等待请求四种职责。
- 确认弹窗 10 个：换设备、换连接方式、换 USB 模式、换无线协议、换接收后端、换播放模式、安装/升级手机应用、切 LAN 设备、打开 USB Direct、通用网络确认；文案高度重复（“会短暂中断声音…”）。
- 设置项暴露内部概念：无线兼容方式（UDP / 无线 ADB）、手机接收方式（手机应用 / 兼容方式）、输出引擎、请求的传输音质（标准 / 高保真 / Hi-Res 实验）。

### 手机端（单屏）
- 状态卡（状态标题 + 下一步说明 + 错误 + 授权提示）→ 主按钮 → USB Direct 区 → 常显卡 → 后台等待卡 → 网络恢复卡 → 播放引擎 → 诊断入口。
- 字符串 80+，状态机状态 `starting/prepared/authenticated/receiverRunning/streaming/paused/failed` 直接映射到 UI 文案。
- 与 PC 用词不一致：PC 说“播放到手机 / 连接手机”，手机说“开始接收 / 连接电脑 / 后台等待 / USB Direct”。

## 2. 主要问题（按优先级）

**P0 · 用户会困惑或误操作**
1. **一个主按钮承担连接与播放**：连接成功后按钮变成“播放到手机”，但状态行同时在说“已连接，播放未确认”，用户不知道点它是重连还是开始出声。
2. **状态文案过多且互相重叠**：`connectedComputer` / `receivingAudio` / `playingOnPhone` / `phoneReady` / `managementOnly` / `receiverComputer` / `receiverAwaitingPlayback` … 同一种现实有多种说法。
3. **10 个中断确认**：换任何东西都弹一次，措辞几乎一样，用户会形成“无脑点确认”的习惯，真正危险的操作失去警示力。
4. **同一概念多个名字**：手机应用 / Receiver / 接收端；USB Direct / AOA / Direct；无线 / UDP / LAN；接收后端 / 手机接收方式 / 输出引擎。
5. **错误信息挤在状态行**：默认展示一行摘要，技术细节折叠在“查看详情”，但摘要里仍混入内部错误码（如 `FORMAT_RATE_MISMATCH`）。

**P1 · 明显冗余，可去掉或折叠**
6. 首次设置有两个入口（主界面“首次设置”与设备卡），向导 3 步与真实流程（选 USB 设备 → 安装 → 绑定）不完全对应。
7. `NetworkReceiverDialog` 四合一，任一改动都牵动全部流程，也是 74 KB 的根因。
8. 设置里 4 个“实验/高级”开关没有默认值说明，用户不知道要不要动。
9. 手机端常显与主界面重复展示同一状态。
10. 两端对“什么算连接成功”的口径不同（PC 以认证为准，手机以服务状态为准），文案因此对不上。

**P2 · 观感与一致性**
11. 图标来源两处（`icon.png` 与 `frontend/public/icon.png`），主题/字号/圆角/间距缺少统一 token。
12. 中英文案文件各 40 KB+，句式长短不一，部分句子超过 40 字。
13. 诊断入口藏在底栏，出问题时用户找不到。
14. 没有“上次使用的手机/方式”的显式回显，只能靠默认选中猜测。

## 3. 优化原则

- **一屏一个主动作**：状态只说“现在怎样 + 下一步一句”，动作只有一个按钮。
- **用户词表优先**：界面只出现 手机、电脑、USB、无线、直接 USB、播放/停止；内部词（AOA、UDP、LAN、Oboe、AudioTrack、后端）只出现在诊断页。
- **渐进披露**：高级选项默认折叠，每项给一句“什么时候需要”。
- **只在会改变输出或中断声音时确认**，且全局合并为一种确认样式。
- **文案硬约束**：标题 ≤ 12 字，说明 ≤ 2 行（约 36 字），错误摘要 ≤ 1 行，动词开头。

## 4. 分阶段方案

### 阶段 0 · 术语与文案统一（低风险，先做）
- 建立术语表（中文 / English / i18n key / 代码标识）：
  | 现在 | 建议 | 说明 |
  | --- | --- | --- |
  | 手机应用 / Receiver / 接收端 | **手机应用** | 界面统一；日志保留 Receiver |
  | USB Direct / AOA / Direct | **直接 USB** | 括号内首次出现注明“免 ADB” |
  | 无线 / UDP / LAN | **无线（局域网）** / **无线（USB 调试）** | 只在高级设置展开 |
  | 接收后端 / 手机接收方式 / 输出引擎 | **播放方式** | 默认“自动” |
  | 后台等待（手机端） | **允许电脑连接** | 与 PC 的“连接手机”配对 |
  | 开始接收（手机端） | **连接电脑** | 与主按钮一致 |
- 主按钮固定为 4 种：**连接手机 / 播放到手机 / 停止播放 / 重试**；其余状态一律由状态行表达。
- 错误摘要只保留一句可执行的话，内部错误码移到诊断页。
- 验收：文案 diff 清单 + 中英对照 + 两端截图对照。

### 阶段 1 · 主流程合并（核心）
- 主界面收敛为三块：**设备卡**（含连接方式，未配对时直接变成三步首次设置）→ **状态 + 主动作** → **播放模式（折叠，默认展开一次）**。
- 10 个中断确认合并为一个 `ConfirmInterruption`：标题“会中断当前播放”、一行后果、主按钮“继续”、次按钮“取消”；只替换两句文案。
- 删掉重复的“首次设置”入口与 `PairDialog` 的重复步骤；配对并入设备卡。
- 手机端：状态卡只留“状态 + 一句下一步 + 一个主按钮”；USB Direct 与后台等待收进“其他连接方式”折叠区。
- 验收：主路径点击数（首次连接 6→3，切播放模式 2→1 且无弹窗）、截图前后对比。

### 阶段 2 · 设置与高级项收敛
- 设置分两组：**常用**（主题、字号、托盘、默认播放模式）与 **高级**（无线协议、播放方式、音质）；高级默认折叠并附一句“何时需要”。
- 音质改为 3 档用户语言：**自动 / 优先音质（实验）/ 兼容**；实验项加 beta 标记，不写 96 kHz 之类参数（进诊断）。
- 播放方式默认“自动”，只有诊断页显示实际引擎（Oboe / AudioTrack）。
- 验收：可见设置项 12→7，高级项全部有默认值，默认路径不需要进设置。

### 阶段 3 · 结构与一致性（最后做）
- 拆分 `NetworkReceiverDialog` 为 连接流 / 首次设置 / 接收端管理 三个组件 + 共享 hook，先把状态机抽出来再拆 UI。
- 统一状态枚举（idle / connecting / connected / playing / paused / error）驱动 PC 与手机两端文案，避免各自维护一套。
- 视觉统一：单一图标来源、间距/圆角 token、深浅色对比度达 AA、AOD 与主界面共享术语。
- 验收：组件行数下降 30%+，两端状态文案一一对应表。

## 5. 不做什么

- 不改协议、不改审计/签名/发布流程。
- 不新增功能（必要时只加状态字段与文案 key）。
- 不重写 Compose 主题或整体视觉风格，只做一致性收敛。

## 6. 风险与验证

- 每个阶段都以“文案 diff + 截图对比 + 点击路径计数 + 真机走查清单”验收，不靠主观评价。
- 与旧版 Receiver / 旧 PC 互通时，新文案必须保持中性词，避免“直接 USB”在不支持的版本上造成误解（提示走版本门槛）。
- 回退成本：阶段 0–1 主要是文案与组件拆分，可单独回退；阶段 3 结构改动需要完整回归（连接、切模式、断线恢复、常显）。

## 7. 建议下一步

先做阶段 0（只改文案与术语，半天到一天可完成，风险最低），主人确认词表后再进入阶段 1 的主流程合并。

## 8. 阶段 0 执行记录（2026-10-21）

主人确认后的词表（与初稿的差异）：

- 手机端“后台等待” → **允许电脑连接**（采纳）
- 手机端“开始接收” → **连接电脑**（采纳）
- “USB Direct” → **保持 USB Direct**（不改）
- “兼容方式” → **保持兼容方式**（不改）

已完成（仅文案，无行为改动）：

- 界面不再出现 “Receiver”：统一为 **手机应用**（App）或 **手机**（设备）；Android 端指自身时用 **AnPod**。
- “接收端” → 手机；“接收后端 / 手机接收方式 / 接收后端选择” → **播放方式**；“无线兼容方式” → **无线方式**；“请求的传输音质” → **音质**；
  “无线 UDP” → 无线（局域网）。
- 主动作标签收敛：**连接手机 / 播放到手机 / 停止播放 / 断开连接 / 重试**，另有“取消连接”“正在处理…”“升级手机应用”三个瞬时态。
- 错误摘要保持一句可执行的话，内部错误码只在“查看详情”/诊断中出现。
- en 文案同步；`tests/AnPod.Frontend.Check.cjs` 中受影响的选择器同步更新（该脚本需要 Playwright，本机未安装，未执行）。
- 验证：`tsc -b` + `vite build` 通过；Android 单测 88 项全部通过（含 Robolectric 38 项）。

待确认的取舍：Android 端把 “Receiver” 换成 “AnPod”，与 PC 端“手机应用”的称呼略有差异；如果希望两端完全一致，可再统一。
## 9. 阶段 1 执行记录（2026-10-21）

已完成：

- **确认弹窗统一**：`ConfirmDialog` 收成一份样式——一句后果 + 一个「继续」，安装手机应用用单独的标题/说明；底层 `applyConfirmed` 的分支逻辑未动，10 个 `switch*Title` 不再被渲染。
- **手机端收束**：主界面静止时只留状态卡 + 主动作，USB Direct 与「允许电脑连接」收进「其他连接方式」折叠区；英文 Android 文案补齐阶段 0 的术语（Allow computer connections / Connect to computer / AnPod）。
- 验证：`tsc -b` + `vite build`、Android 88 项单测通过。

未完成（有意延后）：

1. **主屏结构合并**（设备卡吸收首次设置、播放模式折叠）：会改动 DOM 结构与可见性，`tests/AnPod.Frontend.Check.cjs` 里有大量可见性/布局断言，本机没有 Playwright 无法回归，需先安装依赖跑一遍再动。
2. **删除重复的「首次设置」入口**：需要改 `DeviceDialog → NetworkReceiverDialog` 的流程，属于阶段 1 后半段。
3. 折叠区的视觉顺序（USB Direct 在「播放引擎」上方、电脑连接在常显卡下方）需要真机看一遍再定是否重排。
## 10. 阶段 1 收尾（2026-10-21）

- **播放模式折叠**：改为原生 `<details>` + 一行摘要（播放模式 · 均衡/低延迟），默认展开，可收起；样式加在 `index.css` 的 `.mode-disclosure`。
- **首次设置入口**：核对后确认主界面没有第二个入口——向导只从「连接手机」弹窗进入（`DeviceDialog.onFirstSetup → setPanel("setup")`），设备卡本身就是唯一入口；若把向导改成点卡片直达，会破坏 `firstUseExplainsUsbSetup / firstUseKeepsExplicitInstall / noBindingKeepsUsbSetupAvailable` 这几条既有验收，故未改动。
- **浏览器检查可用性**：本机装了 `playwright-core`（`artifacts/tools/pw`）并用 Edge 跑通：
  - `ANPOD_UI_SIMPLIFICATION_ONLY` 17/17 通过，无布局溢出、无控制台错误；
  - 无 flag 的全量流程仍卡在 mock 快照缺失 `canChangePlaybackMode` 与监听器 fixture 上（与本次改动无关，属既有测试基建缺口），未改 mock 以免掩盖问题；
  - 选择器已跟随新文案更新（停止播放 / 会中断当前播放 / 继续 / 无线方式 / 播放方式）。
## 11. 1.10.6-dev 出包（2026-10-21）

- 代码基线：`6139e6f`（阶段 1 最后一个代码提交）；其后只加两个发布元数据提交：`01f16af`（版本 1.10.6 / 39）、`4e60403`（审计表 39 + SIGNING）。版本号必须递增，否则同 `versionCode` 不同字节会被审计表判为未核验。
- Receiver `0.17.5-v3-flow-candidate` / code 39：`a77bf771…b0d0`，source `01f16af`，`sourceModified=false`，同签名。
- 便携包：`AnPod-1.10.6-tauri-win-x64-portable.zip`，`df90ea98…910e`，18,077,068 字节；EXE 1.10.6 `dd3b2b55…1c74`；`build-metadata.json` buildCommit `4e60403`，`workingTreeDirty=true` 仍来自保留的本地缓存目录。
- Release：<https://github.com/sanren1/AnPod/releases/tag/v1.10.6-dev>，资产 digest 与本地一致。
- 备注：Android 构建需要 `com.google.oboe:oboe:1.10.0`，离线缓存缺失，本轮用 `GRADLE_OPTS` 代理在线构建通过。
## 12. 1.10.7-dev：原生组件修复与 Direct 默认（2026-10-21）

- 现象：USB Direct 连接失败，详情报“当前音频采集组件不支持 Float32 V2；PCM16 仍可使用，请更新 Native DLL”。
- 根因：1.10.6 包用 `publish-tauri.ps1 -IncludeReceiverRelease` 默认从 `deps/` 取组件，而 `deps/`（被 gitignore）仍是旧版：`audio_capture.dll` 451,720 字节（无 V2 Float32 接口）、`server` 旧版；1.10.0–1.10.5 交付的包用的是 175,104 字节的 V2 DLL。
- 修复：`deps/` 恢复为已交付组件（`audio_capture.dll dbdf5fcc…b94e`、`server 0676594a…bec7`），重新打包并核对包内哈希。
- 默认值：`AppSettings.wired_mode` 与 serde 默认改为 `aoa`（新建配置即 Direct）；已保存配置不变，非法值仍回退 ADB；删除不再使用的 `adb()` 默认函数，更新两条断言。手机端没有独立默认，跟随 PC 的有线模式，因此两端一致。
- 版本：PC `1.10.7`、Receiver 仍 `0.17.5` / code `39`（APK 未变，无需重建）。
- 包：`AnPod-1.10.7-tauri-win-x64-portable.zip`，`b68c3d86…8d17`，18,071,002 字节；EXE `eaa51716…a1b1`。
- Release：<https://github.com/sanren1/AnPod/releases/tag/v1.10.7-dev>；1.10.6-dev 的说明已加警告指向 1.10.7。
- 经验：`deps/` 不进版本库，`-IncludeReceiverRelease` 不校验原生组件版本，下次出包前需人工比对 `audio_capture.dll` 的 V2 能力（175,104 字节 / `dbdf5fcc…`）。
## 13. 上一轮完成情况总览（2026-10-22；本轮进展见第 14 节）

### 阶段 0：术语与文案 —— 已完成
- [x] 界面不再出现 “Receiver”；统一为「手机应用 / 手机」，Android 自称为 AnPod
- [x] 接收后端/手机接收方式 → 播放方式；无线兼容方式 → 无线方式；无线 UDP → 无线（局域网）；传输音质 → 音质
- [x] 手机端：后台等待 → 允许电脑连接；开始接收 → 连接电脑
- [x] 主动作收敛为 连接手机 / 播放到手机 / 停止播放 / 断开连接 / 重试
- [x] 确认弹窗统一为一句后果 + 「继续」（安装手机应用单独说明）
- [x] 英文文案与浏览器检查选择器同步
- 验证：Android 88 项单测、`tsc -b` + `vite build`

### 阶段 1：主流程收束 —— 部分完成
- [x] 十个中断确认合并成一个 `ConfirmDialog`
- [x] 播放模式折叠为一行摘要（默认展开）
- [x] 手机端：USB Direct 与「允许电脑连接」收进「其他连接方式」
- [ ] 主屏结构合并（设备卡吸收首次设置）
- [ ] `NetworkReceiverDialog` 拆成 连接区 / 首次设置 / 接收端管理 三个组件
- 说明：「首次设置」入口核对后确认只有弹窗内一处，设备卡已是唯一入口，属于“核对无需改”

### 无线连接链路 —— 本轮重点，已修但未跑通专项回归
- [x] 打开弹窗自动开始监听；去掉「开始监听」按钮
- [x] 主按钮统一：有请求→接受 / 有地址→拨号 / 都没有→等待
- [x] 自动监听每 2 秒重试（最多 6 次），首次走可见路径以触发防火墙授权
- [x] 同一 endpoint 的请求只保留最新一条（Rust `retain`）
- [x] 用户发起的连接不再要求「后台免确认」能力（修掉 `background-start authorization` 报错）
- [x] 自动监听与其它操作互斥（修掉「连接操作尚未结束」）
- [x] 弹窗只在点击对话框外部时关闭（修掉点空白误关）
- [ ] `ANPOD_WIRELESS_FLOW_ONLY` 仍卡在既有失败点，新规范未写入检查
- [ ] 「能从主界面直接等待时不弹二级窗」尚未实现

### 出包记录
| 版本 | 内容 | 去向 |
| --- | --- | --- |
| 1.10.6 / 0.17.5 (39) | 阶段 0–1 | GitHub `v1.10.6-dev`（已标记为有缺陷，指向 1.10.7） |
| 1.10.7 / 0.17.5 (39) | 原生 DLL 修复 + Direct 默认 | GitHub `v1.10.7-dev` |
| 1.10.8 | 无线弹窗自动等待（首版） | 仅本地 |
| 1.10.9 | 一键连接不再要求后台授权 | 仅本地 |
| 1.10.10 | 显式连接也不要求后台授权 | 仅本地 |
| 1.10.11 | 等待框显示真实监听状态 + 重试按钮 | 仅本地 |
| 1.10.12 | 防火墙授权后自动重试一次 | 仅本地 |
| 1.10.13 | 请求去重 + 每 2 秒重试 + 弹窗外部才关闭 | 仅本地 |

### 待办（按优先级）
1. 重写 `ANPOD_WIRELESS_FLOW_ONLY` 到新交互，覆盖自动等待、请求去重、接受请求
2. 主按钮在能直接等待时不弹二级窗
3. `NetworkReceiverDialog` 拆分 + 主屏结构合并
4. Android 侧剩余 UX 项（状态卡文案、诊断入口层级）
5. `deps/` 原生组件校验加入打包脚本（避免再发错 DLL）

## 14. 本轮源码、审查与交付（2026-10-22）

本轮输入为 `dev@0f261bb`（PC 1.10.13 / 手机应用 39）。源码与离线验收、独立审查修订已完成；PC 1.10.14 / 手机应用 0.17.6（40）本地候选已构建，独立交付审计 18 项全部通过。实机仍为 **NOT_TESTED**，不把历史勾选或浏览器模拟当作实机通过。

### 已完成的规划项

- [x] 修复监听生命周期：由 App 统一持有启动/停止、串行调用和重试；停止后迟到结果不重新开启监听。
- [x] 主屏点击后有唯一有效手机则连接，没有候选则直接等待；网络、绑定或候选不唯一时先选择。主屏等待复用现有快照轮询，能看到后端不主动推送的手机请求，避免双方互等。
- [x] 设备区集中连接方式、状态和主动作；“首次设置”直接可见。无线连接入口、首次设置向导、手机应用管理拆分展示，业务状态仍由原容器持有。
- [x] 设置常用优先、高级默认折叠，实际格式仍可查；帮助和 USB Direct 中英文说明收短，保留配件授权、ADB/MTP 中断和停止全局 ADB 的明确同意。
- [x] 手机错误详情/复制靠近状态卡，播放方式放入其他连接方式；主屏与 AOD 区分“已连接”和“正在接收”，暂停不误报错误。
- [x] 普通打包路径按审计哈希及 Native/Legacy 角色校验输入，并复验复制后的包内字节；旧版、未知和跨角色组件拒绝检查通过。

保留已确认的 USB Direct、兼容方式名称和标准/高保真/Hi-Res 三个明确档位，不采纳草稿中的隐式自动降档或 SRC。保留已有配对、签名、服务与播放状态机，不为统一枚举、共享 hook 或行数指标额外重构。新配置默认 USB Direct，既有偏好继续保留。

监听和发现只提供请求；用户明确点击连接或接受请求已授权开始播放，不增加第二次确认。任一端手动断开不自动恢复，意外中断仅按原链路有限恢复。

### 独立审查及修订

执行代理使用 GPT 6 luna high；全新、无历史上下文的 GPT 6 astra high 独立审查 `29e7ef3`，发现 1 项 P1、4 项 P2。完成下列修复后逐项复审，五项均关闭，源码验收通过；完整证据见 `artifacts/uiux-20261022-astra-review.md`。

| 问题 | 修复与验收 |
| --- | --- |
| 主屏等待没有刷新真实手机请求 | 扩展单份快照轮询；无 emit 的请求到达用例通过 |
| 主屏唯一候选连接失败无提示 | 使用现有错误恢复弹窗，保留所选网络/绑定；真实 ready 快照加格式拒绝用例通过 |
| 网络握手没有取消入口 | 启动与接受请求共用 pending/generation；取消等待后端确认，已建立会话则实际断开；取消期间禁用重复连接；主屏及确认弹窗用例通过 |
| 迟到的发现失败覆盖设置 | catch 在产生 UI 副作用前校验 generation；保留设置用例通过 |
| receiverRunning 未收到音频却显示接收中 | 该状态显示已连接，streaming 才显示正在接收；原有状态决策定向测试通过 |

### 实际验证

- 前端完整流程：57 项布尔结果全真、24 组布局无溢出、控制台错误 0；确认取消/界面专项 21 项全真。
- 独立浏览器 QA：无线 35、首次设置 27、音质 19、诊断 22 项布尔结果全真，各控制台错误 0。证据在 `artifacts/uiux-20261022-frontend-checks/astra-*`。
- 最后一个布尔表达式整理经 Astra 核对语义等价；发布构建发现的可选绑定类型问题已修正。真实 `tsc -b`、Vite production build 和 Node 语法检查通过；不把根 references tsconfig 的空检查或页面加载超时计为通过，也不声称在这两处小修后重跑全部行为回归。
- Rust 基线 151 passed / 0 failed / 1 ignored；版本、来源快照及组件门禁检查通过。指定本轮 APK 目录的真实制品读取测试 1/1 通过。
- Android 前期全套 88 项通过；最新状态映射定向 2/2 通过，未相加声称 90 项全套通过。

### 本地制品与来源

- PC 构建来源：`27c7c606048305fabfb2cedcc84abd72de4203c4`；前序实现 `29e7ef3`，监听/取消修订 `1de5c90`。发布构建已通过。
- Receiver 40：release 签名，来源 `3dfb08cd0487fb2ea3b1b9d20a7c508727a73486`，`sourceModified=false`；APK SHA-256 `56578ab87996fb12b16d919f8c6996a86f13a4230703dd07f6f58c202bf2d01f`，沿用原发布证书，已登记精确审计表。
- 便携包：`artifacts/AnPod-1.10.14-tauri-win-x64-portable.zip`，18,466,063 bytes；SHA-256 `a064fdcabb0cca90d5e96e9ecf6ea381b3b282a687cdf0dd463761a13ad0baff`。独立交付审计 18 项全通过，ZIP/目录 49 个文件路径、长度、哈希一致，README 的 11 个本地链接有效；根代理另行复算全部 ZIP 内容通过。证据见 `artifacts/uiux-20261022-delivery-audit.json` 和同名 `.md`。本轮未远程发布。
- APK 构建中间目录和本轮临时浏览器/服务缓存已清理；保留有效制品、验证证据及原有三个用户缓存。全树 dirty 标记来自这些原有缓存，桌面和 Receiver 输入均未修改。

### 状态展示与实机边界

| 真实情况 | 展示要求 |
| --- | --- |
| 服务准备或等待请求、尚未建立会话 | 等待手机 / 等待电脑，不显示播放 |
| 已认证或 receiverRunning、没有真实音频进度 | 已连接 / 等待播放确认 |
| 真实音频 payload 到达 | 手机及常显显示正在接收；PC 仅在播放进度确认后显示正在播放 |
| 暂停 / 意外恢复 / 手动断开 | 已暂停 / 恢复中 / 不自动恢复 |
| 操作失败 | 一句可执行摘要，保留详情和复制 |

三链路实机重测、防火墙授权与切网、USB 热插拔和配件权限、手机安装/原地升级、真实采集输出格式、声学/延迟/长时稳定性仍 **NOT_TESTED**；本轮按用户要求不重启旧实机核查。

## 15. 2026-10-22 · Android MD3 / PC Soft Minimal 重构

以 `dev@ffea50f` 为基线，在 `codex/ui-refresh` 实施两份新增规划。执行代理 GPT 6 luna high；独立源码及界面复审 GPT 6 astra high，无历史上下文派发。未更换底层协议、服务/音频状态机、密钥、绑定格式或 AOD 实现。

- Android：Direct/ADB/Wi-Fi 共用唯一主操作；本地选择持久化、实际状态优先；屏幕常显后连续展开其他设置，保留播放引擎、后台等待和完整授权管理。预审四项（ADB真实标签、后台多接口恢复、模式切换残留错误、权限等待误报）及重复说明已修复。
- PC：浅色银灰、深色石墨语义主题，两处原生按钮语义分段控件加 Motion 动效，设置视觉分组及弹窗轻量整理。保留所有原有 IPC、轮询、确认/取消、三档音质、实际格式和禁用规则。修复原有弹窗卸载不返回焦点，并提高焦点描边对比。
- CI：用户指出的 `discovery.rs` rustfmt 排版失败已单独修复；后续提交前执行 CI 同款 `cargo fmt --manifest-path src-tauri/Cargo.toml --check`，不改规则绕过。
- 实际验证：前端 `tsc -b` / Vite production build、六个独立专项通过；最终默认流程和24组新布局/键盘矩阵见 `artifacts/ui-refresh-20261022/qa/`。Android完整94项JVM测试通过，lint执行成功（0 Error、59 Warning、4 Information），Debug构建通过。最终独立审查记录 `artifacts/ui-refresh-20261022/review-final.md`；仅原始结果完成的范围可计PASS。
- Receiver41：0.18.0-v3-flow-candidate，release原证书，来源 `c75856fe83579c1e0650271b0ff26e5bdebf03db`，sourceModified=false；SHA-256 `cbfdd5dad8ee3e38bc704290b1bcfc95a4b488fd2d80f8d135a4baecad804e56`。PC候选1.11.0，精确APK登记与本地打包已完成，结果如下。
- 性能范围：Motion接入增加约43,882 gzip bytes JS，单块约558 kB存在Vite体积警告。Vite开发服务器的Chrome CTA代理p50 425.3→429.5ms；p95 894.2→2659.1ms含冷依赖优化且当前样本与Gradle并发，不能作为生产WebView性能结论。未凭警告进行额外架构拆分。许可证原文已随LICENSES补齐。

按用户要求不重启旧实机核查。本轮未安装或操作手机；Windows原生窗口/DPI/防火墙、三链路物理音频、USB权限与热插拔、AOD、手机横屏/动态色/大字/TalkBack、声学/延迟/长时性能仍NOT_TESTED。浏览器截图不替代这些验证；本轮仅本地候选，不远程发布。

### 本地交付验收（2026-10-23）

最终便携包 `artifacts/AnPod-1.11.0-tauri-win-x64-portable.zip`，18,526,109 bytes，SHA-256 `bb46d2772d1511e7837fd206592d0e04cd921db2220b1f5c05605e69eb53a646`。桌面冻结来源 `01dee74dcfb90361c23b5bbdccfd34bf86af64a9`；包内 EXE SHA-256 `5e8755483625ad6b61802332e7c52c2ae2461034c70caaae10e2a6a81f4a9153`，桌面输入快照 `b7fb848a1c63325238d4713fba1748b2788ff3a6614ea8d0d9099d4e2a8a60d2`。随包 Receiver41 为上述原签名 APK，输入快照 `aaa8a2a0b50a9b8433f669b506998fa52fd583c1031183f43c499b5ec1507386`，证书 SHA-256 `f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573`。

无历史上下文的 GPT 6 astra medium 独立交付审计23项全部通过，报告 `artifacts/ui-refresh-20261022/delivery-audit.json` / `.md`。根代理另行打开最终ZIP，52项路径、长度、SHA-256与目录逐项一致，记录 `root-zip-verification.json`。审计覆盖源码快照、EXE版本、APK身份与签名、Native/Legacy角色、精确制品登记、README及许可原文。初次封包的README相对CI链接已修正并重新冻结封包；未交付旧尝试已删除，历史有效包保留。指定新APK目录的真实制品读取测试1/1通过；CI同款Rust格式检查通过，记录 `final-format-check.log`。

审计依据最新原始XML确认Android13 suites /94 tests、0 failures/errors/skipped及Gradle EXIT_CODE=0，覆盖较早源码审查报告中的定向7项证据限制。lint原始报告为 `src/AnPod.Receiver/app/build/reports/lint-results-debug.xml`，实际59条Warning与4条Information，不能宣称零警告；未比较同条件基线，尚不能判定其中新旧比例。

后续审查统一使用 GPT 6 astra medium，执行代理保持 GPT 6 luna high。交付记录位于源码输入范围之外，后续文档提交不改变上述制品来源。受跟踪源码无未提交修改，全树dirty标记仅来自保留的三个用户缓存；本轮构建中间目录与临时浏览器/服务缓存已清理。实机与生产性能范围仍按上文NOT_TESTED，本轮未推送或远程发布。

## 16. 2026-10-23 · 离线质量与生产前端代理核验

基于 `4c927a2`，两项执行使用 GPT 6 luna high，独立复核使用 GPT 6 astra medium。未修改生产代码、版本或依赖，不重新封包。证据目录 `artifacts/ui-quality-20261023/`，独立结论 `review.md` 为PASS（仅本节范围）。

- PC：同一工具/依赖环境构建 `ffea50f` 与当前生产静态资源，固定baseline→current交替9对，新Chrome进程、同一无设备mock；18次各自版本harness运行成功。主按钮可见、启用且中心点可命中的代理指标p50为199.0609→197.3333ms，p95为248.1759→207.5981ms。本样本未显示退化；n=9的p95等于最大值，不能据此证明性能提升或10%预算达标。计时从Node调用goto前开始，不是Performance API navigationStart，不含Chrome启动，也不证明点击处理或连接完成。OS文件缓存可能已热，未测暖浏览器缓存；两份历史harness不完全同套，共用已安装依赖而非独立clean install，构建未含tsc。报告 `pc/report.json`，原始记录 `pc/runs/`；静态服务已关闭。JS gzip增加44,430 bytes，保留体积警告，无据此新增拆包架构。
- Android：标准完整基线lint成功，0 Error/56 Warning/4 Information；保存的当前完整报告为0/59/4。按severity/id/message比对，55条当前警告沿用基线，4条新增未使用字符串（processing、usb_direct_connect、other_connection_methods、hide_other_connection_methods），1条旧警告cancel_connection消失。未发现已证实运行时缺陷，不为静态警告更改生产行为。当前报告未重新执行；基线首次离线失败后补取已声明依赖再成功，无依赖声明改动。完整报告、差异与日志保存在 `android/`，隔离源码/build目录已清理，主树与用户缓存未动。

独立代理复核PC七项哈希、18份原始数据/分位数及Android完整XML差分通过。下一阶段仍缺Windows WebView2、真实三链路、音频/AOD及长时资源测量；继续按既定要求保持NOT_TESTED，不把本轮浏览器代理结果迁移为实机通过。原1.11.0/Receiver41候选仍有效，未推送或远程发布。

## 17. 2026-10-23 · PC参考图视觉修订（进行中）

用户指出PC实际界面未达参考图最终需求，独立Astra medium核对后确认：前轮仅完成部分视觉，撤回“PC UI重构最终完成”的验收结论。历史构建与功能检查结果保留其原范围，不作为视觉达标证据。以 `f2a690b` 为修订基线。

本轮按图1–5修订胶囊CSS冲突、三处分段控件统一、状态与主按钮重心、播放模式行比例、设置四组及音质三档对比卡。由单个Luna high串行实现共享CSS/展示JSX，独立Luna high验证真实App截图/功能/访问性，再由Astra medium对照参考图验收。保留原连接/取消/确认规则、实际状态来源、高级默认折叠及全部安全限制；不扩展协议、遥测或依赖。尚未完成项不记PASS。

前版修订源码已提交 `e0e36e7`，PC 1.11.1浏览器视觉/交互复审通过，包已在本地生成；其后独立包审计因Astra额度中断，未完成，不能称独立交付审计通过。后续用户提供v2规划，以下以新规格继续实施，不把前版52px主按钮/999px圆角沿用为v2硬门槛。

## 18. 2026-10-23 · PC v2 / Android横向布局与AOD设置

输入为用户新增 `AnPod_PC_Soft_Minimal_UI_UX_Plan_v2/`；基线 `e0e36e7`（PC1.11.1 / Receiver41）。PC按该目录 `plan.md` 4.5–4.14的实施规格核对已有成果，文字规格高于参考图：仅改展示/样式/访问性，不修改连接与音频状态机；音频摘要保持可见，高级设置默认折叠，弹窗固定标题/单一内容滚动。两端执行Luna high，无历史派发，不嵌套。

Android实施契约：

- 首页顶部AnPod固定，safeDrawing内仅下方主体滚动；三种连接方式始终横向排列，短名与完整辅助名称并存，不再按窄屏或字体比例变成竖排。保留原主操作、实际状态优先、选中与禁用规则。
- 常显卡“预览常显”和“常显设置”同一行等宽入口，触达高度至少48dp。设置菜单可滚动、关闭明确，文案简短且中英对应。
- 时钟字号小/默认/大（以当前100%为默认，目标80–120%）；波形细/默认/粗（仅绘制笔宽倍率，默认保持原值）。字号结合实际宽高及既有几何锚点约束，避免横屏/大字遮挡波形和底部信息。
- 详细数据可分别显示协议、接收速率、接收缓冲、丢包率，默认全显示；真实连接状态仍保留。数据始终来自既有telemetry，缺失按未知，不伪填。窄屏可换行，隐藏项不留悬空分隔符。
- 本地设置复用 `anpod_receiver` SharedPreferences，明确用户操作才保存，重开菜单和AOD预览仍生效；不新增IPC或后台配置服务。不改变黑底、防烧屏漂移、会话退出宽限、退出按钮和音量手势。

验收：PC真实React截图/状态黄金对照/相关专项与尺寸访问性矩阵；Android偏好默认及roundtrip、字号边界、原相关JVM、Debug构建与lint原始结果。实机、音频与系统权限检查仍单列NOT_TESTED；编译和浏览器mock不替代实测。

### 源码与离线验收

业务冻结 `aa365903929074b4abcf3872066184140023fee9`；PC1.12.0 / Receiver0.19.0-v3-flow-candidate/code42。PC三文件增量落实v2差距；九项浏览器检查通过、32组无横向溢出，11状态记录的App/appRuntime文件blob与基线相同。版本1.12.0主屏及设置图位于 `pc-qa/version-1.12.0/`，仅重采展示版本，功能矩阵未因此重复跑。PC全部证据根为 `artifacts/ui-v2-aod-20261023/`。

Android已实现固定顶栏、横向连接选择、同排设置/预览、主题内可滚动常显设置弹窗。时钟80–120%按真实字形宽高与clock/footer锚点限制，波形80/100/120%作用于静音和真实PCM绘制；协议/接收/缓冲/丢包四项本地开关默认全开，状态保留，窄屏可换行。设置页不启动AOD采样/超时逻辑。

Android此前全量14 suites/97 tests零失败；审查修订后定向3项、lintDebug及assembleDebug通过，不把两批计数相加或称最后重跑全量。最终lint为0 Error、59 Warning、5 Information（后续独立审查复核原始XML，更正此前记录的4条）。证据 `android/full-suite-lint.log`、`android/final-checks.log`、原始XML。独立Astra medium发现并关闭两项P2（开关无辅助名称、横屏大字缺高度约束），最终代码/PC浏览器/JVM构建范围PASS，详见 `review.md`。所有实机/模拟器显示、TalkBack、系统权限、真实音频仍NOT_TESTED。

### 本地制品

PC便携包 `artifacts/AnPod-1.12.0-tauri-win-x64-portable.zip`，18,533,994 bytes，SHA-256 `59b8958ec4b8fb28e7cbfbaade9bb43ed262904020a6293e14b5ceea37e8af41`。构建来源 `a703bb268055033637711aa37460e9066fc3a192`，EXE SHA-256 `1c5e58ba01fc08e36b4e01ded67a6ca7d3ffaaa99a3e41c7d8998610760a2de9`，桌面输入快照 `b945e4cd96022a0ca49f0c14a709df59ca48f22e9c1e4c0e7bb5a0e04d280426`。

Receiver42：`artifacts/ui-v2-aod-20261023-receiver42/receiver.apk`，13,708,905 bytes；SHA-256 `96d9eb04f73ef3aed8054842f4d8b5f34e12f4a4e10e75d1ceb10f7045e749fb`。release / target35 / 原证书，source `aa365903929074b4abcf3872066184140023fee9`，sourceModified=false，输入快照 `19d464c84a2c67f64c8cd0ab7b41eb3c37dd6bf4fbcd7b244872c261b0719dd4`。精确审计登记 `4a8272d`，指定本轮APK目录的真实Rust artifact-read 1/1及CI同款fmt检查通过。APK中间构建目录已清理，用户缓存与旧包保留。

根代理另行打开ZIP逐项核对52个文件路径/长度/哈希通过，结果 `artifacts/ui-v2-aod-20261023/root-zip-verification.json`。独立Astra medium制品审计PASS，同目录 `delivery-audit.json` / `.md` 记录EXE版本、APK原签名/来源/精确登记、Native/Legacy、README冻结/10个本地链接及许可证核验。构建时受跟踪源码干净，全树dirty仅因三个既有用户缓存；本节文档在源码输入范围外，不改变制品来源。本轮未推送或远程发布，使用新增手机功能需同步更新随包Receiver42。

## 19. 2026-10-23–24 · 间距收尾、AOD细化及整体审查（源码完成，交付验收中）

最新用户指令覆盖第18节的AOD显示细节：实际常显文本全部英文，参数居中、使用 `|` 分隔；多项显示可换行，不为塞进一行缩小可读字号。常显设置保留本地化菜单，增加小型设置预览，两个同排入口采用同样式，将“预览常显”改为“进入常显”。

| 任务 | 类型与依赖 | 当前验收状态 |
| --- | --- | --- |
| 首页分隔线与模块间距 | [SUBAGENT]，独立完成 | `f295d4f` / `61add73`；420/480宽三处12px、360/390宽8px，浏览器截图及滚动几何通过；未包含于旧1.12.0包 |
| 设置保存反馈 | [SUBAGENT]，独立执行 | 普通保存、无会话高级设置、活动会话确认三条路径均已补齐；最新生产静态前端默认59项布尔全真、确认专项12项全真、保存专项19项全真及调用次数断言通过，等待冻结复审 |
| AOD参数与设置预览 | [SUBAGENT]，独立执行 | 新增实际音频格式、输出引擎、欠载次数可选项，默认关闭；现有四项偏好保留。复用当前Metrics/session数据，无新增采样周期；14套98项JVM通过，清理本轮弃用资源后lint及Debug构建通过，等待冻结复审 |
| 有限恢复及PCM24模式一致性 | [SERIAL]，单执行代理完成耦合模块 | 已修复SetMode后恢复窗口为空导致无限重试、认证前同步PCM24 profile mode；保留质量/速率/位深及手动断开语义。P5-C定向27项和最终Rust串行151项通过 |
| 整体审查与修订 | [SUBAGENT]，全部实现冻结后验收 | 按2026-10-24最新指令改为独立GPT 6.1 sol high，不再使用Astra；覆盖UI/UX、简短文案、连接链路/并发/释放及制品信任边界；发现须修订再复审 |
| 版本、签名与本地交付 | [SERIAL]，依赖上述验收 | 沿用原密钥与精确制品登记门禁，不覆盖历史有效包，不推送或远程发布 |

执行代理保持GPT 6 luna high、无历史派发、无嵌套、最多三个并行。规划完成度独立报告 `artifacts/project-review-20261023/ui-plan-completion.md` 区分源码已实现、离线已验证和实机未验证；P5及此前明确跳过的实机内容不凭浏览器/JVM勾为完成。

### 2026-10-24 源码与验证收尾

两端UI及版本冻结为 `82e82ec`（PC1.12.1 / Receiver0.19.1-v3-flow-candidate/code43）。设置保存三路径的最终默认/确认/保存专项均通过；默认59项布尔及24组布局、确认12项布尔、保存19项布尔及命令调用数检查，零页面错误。修复测试误用控件名称后重跑通过；夹具显示版本随后单独改为1.12.1，不称再次重跑行为。

Android最终组件代码的14套98项JVM测试零失败，原始XML已保留；全量stdout未保存。随后只删除本轮明确弃用资源，lint/assembleDebug再次成功，最终0 Error/59 Warning/5 Information。AOD预览是静态时间与未知遥测占位，复用真实绘制组件，不启动Activity/采样计时；新增数据来自当前PlaybackMetrics/session，未知不伪填。

旧模式切换测试在HEAD与修改版均偶发失败，确证为fixture先等待低频Streaming状态才发送第二块音频，导致接收3840字节而sink仅消费1920、丢弃480帧，继而触发远端断言和额外重连。改为10ms连续capture节拍并保留消耗时间，原3会话/11520字节/3 STOP/首帧/暂停断言不变，定向连续3次通过。最终P5-C组27/27；最终完整Rust为151 passed/0 failed/1 ignored/2 filtered（CI两项实机测试跳过），日志 `p5c-recovery/full-rust-ci-elevated.log`。

本轮Clippy发现五项既有问题，已做单次CAS、尾部冗余return、等价合并条件和两个函数局部参数数目标注的最小修正，保留冻结会话参数。独立复核拦截并恢复了一处中间误删的码率显示保护守卫，错误版本未提交。最终fmt及Clippy `-D warnings`通过。首次完整suite的3项DPAPI模拟存储权限失败有原始日志保留，提升权限后同套离线测试通过，未修改安全断言；初次Clippy日志仅保留头部，五项诊断依据工具观察与源码定位，不声称原始日志完整。

原签名Receiver43已生成：`artifacts/ui-aod-refine-20261024-receiver43/receiver.apk`，13,706,952 bytes，SHA-256 `f912db8dc39935b7d0c3c23dbcf8d54d8053f484e5cb8524b9bba989022f3161`；source `82e82ec1c2f46060ea3aef4a04a2b8b4e538ddf1`、sourceModified=false，输入快照 `e803dc8f3ec1b759c5facb37b2514c053fe9f088fe07f381684be9aef942c604`。原证书摘要保持 `f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573`，五字段精确登记与指定本轮目录的实际制品读取1/1通过。

Java离线协议/PCM检查、版本一致性、源码快照和组件门禁检查通过。未运行C# Core.Tests：主机有dotnet宿主但无.NET SDK，且本轮未改C#；不声称完整GitHub流水线在本机全部执行。全部实机、Windows权限与DPI、三链路物理音频、AOD布局/TalkBack、性能与长稳仍NOT_TESTED，遵守用户跳过旧实机核查的要求。旧包保留，新本地包与独立审计尚待下节结果，不推送或远程发布。
