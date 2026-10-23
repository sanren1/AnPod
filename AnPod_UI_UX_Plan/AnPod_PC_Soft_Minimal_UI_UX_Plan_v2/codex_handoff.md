# Codex 开发交接指南（与 plan.md 配套）

> 本指南不是代替主规划；具体布局/行为约束以 `plan.md` 第 4.5–4.14 节为准。**确认的效果图已附在 `figures/`**，其中演示电量、Hi-Res 数值、原生标题栏按钮并非当前 AnPod 的真实功能。

## 首轮必须先做的阅读与检查

1. 读取 `plan.md` 第 0–4.14 节和仓库 `AnPod_UI_UX_Plan/plan.md`。查看 `figures/figure-01-soft-minimal-four-views.png` 及四张局部 PNG。
2. 在本地 `git rev-parse HEAD`，记录当前 dev SHA 与原规划 `ffea50f...` 的差异；变化时逐项比对 `frontend/src/App.tsx`、`frontend/src/components/*.tsx`、`appRuntime.ts`、`appTypes.ts`、`index.css`、`tests/AnPod.Frontend.Check.cjs`。**本文件撰写时的远端 SHA 不等于执行时保证的 SHA**。
3. 冻结主屏、设置、无线/USB/首次设置/配对/安装/诊断的旧行为和禁用条件，先运行能够运行的测试。记录 FAIL / PASS / NOT_RUN，禁止伪造测试结论。
4. 严格控制范围：**仅改 PC React UI、样式与 UI 测试，不触碰 Rust 音频连接栈及 Android Receiver**；若必须改变业务流程先汇报并另立 PR。

## 推荐向 Codex 逐轮提交的任务

### 提示词 1：P0 + UI-1（基线与设计基础）

```text
以当前 dev 源码为准，按 plan.md 第 4.5、4.12、4.13 节实现 P0/P1。
先核对所有 GUI snapshot fixture、窗口矩阵和现有 Playwright 选择器，
建立 Soft Minimal 语义色和间距 tokens 与组件 demo。
不要更改 App.tsx 操作分支、NetworkReceiverDialog 监听逻辑、后端命令。
在 360×500 与 420×590 下截 Light/Dark 图，输出变化的文件、测试命令、原始结果、已知差异。
```

### 提示词 2：UI-2 + UI-3（主界面）

```text
按 plan.md 第 4.6/4.9/4.10/4.11 节逐区域改造主界面。
保留现存 runMainAction、setOption、setWiredMode、chooseDevice、
取消/确认逻辑和所有原业务回调；建立受控 SegmentedControl。
以 current Snapshot 事实为准，不展示假的手机电量、波形、延迟与 Hi-Res 传输指标。
提交 11 状态 fixture 和真实主操作标签逐行对照，以及 Light/Dark、360×500、420×590 截图。
```

### 提示词 3：UI-4（设置界面）

```text
按 plan.md 第 4.7 节实现设置面板，保留 SettingsDialog 与当前原生 dialog。
逐个复用 appearanceTheme、minimizeToTray、largeText、audioQuality、
wirelessProtocol、audioBackend 对应现有 handler 和禁用条件。
高级项默认折叠，实际音频格式与用户请求档位分开；不要新增 export_logs 等命令。
提供简体中文/英文 + Light/Dark + 窄窗口截图和设置失败/禁用回归证据。
```

### 提示词 4：UI-5 + UI-6（弹窗）

```text
按 plan.md 第 4.8 节先统一 Modal 外壳，再逐个改设备、无线、首次设置、
USB Direct、配对、手机应用管理、确认弹窗的内容视觉。
不能迁移/删除现有 generation、网络监听队列、AOA 切换、安装包验证等逻辑。
逐类测试 Esc、真正的遮罩点击、表单 Enter、防重复提交、取消、焦点返回、
多候选、未授权、错误及 360×500 的底部动作可达性。
```

### 提示词 5：UI-7 + P5（诊断、验收与交付）

```text
统一诊断/帮助/Toast 的 Soft Minimal 呈现，保留所有已有技术字段、
错误细节、复制、导出诊断、打开日志与隐私保护行为。
执行现有自动化专项、颜色与焦点检查、尺寸/语言/DPI/reduced-motion 矩阵；
归档截图和未通过原因。
真机测试必须独立列出 USB ADB / USB Direct / 无线 UDP，
若没有实际设备就标记 NOT_TESTED，禁止写实机通过。
```

## 每轮结果输出模板

```text
本轮基线 SHA：
本轮涉及文件：
旧行为/状态条件保护点：
完成的 UI 结构和视觉改动：
截图位置（明暗主题 + 窄窗）：
测试命令与 PASS/FAIL/NOT_RUN：
失败点与复现方式：
风险、回滚步骤、下一轮建议：
```

## 成品验收门槛

- 不用假数据填充 UI；动画不改变后端已确认状态。
- 主屏设备/方式/唯一主动作/播放模式顺序清晰；设置常用优先、高级可达；复杂弹窗有正确关闭/取消语义。
- 当前通过的 UI 自动化、签名/Native 安全边界不退化；任何真机路径无实测不能宣称通过。
