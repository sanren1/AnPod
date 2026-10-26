import { useState } from "react";
import { Activity, AudioLines, CircleHelp, ChevronDown, Settings2, Smartphone } from "lucide-react";
import { FluidSegmentedControl, type SegmentOption } from "../components/ui/FluidSegmentedControl";
import "./ui-gallery.css";

const states = [
  ["noDevice", "未连接", "等待手机连接", "muted"],
  ["unauthorized", "需要授权", "请在手机上确认 USB 调试授权", "warning"],
  ["ready", "已就绪", "设备已就绪，尚未建立会话", "ready"],
  ["connecting", "正在连接", "可取消的连接操作", "pending"],
  ["connected", "已连接", "连接不代表正在传输音频", "ready"],
  ["phoneReady", "等待播放", "尚未确认收到音频", "ready"],
  ["streaming", "音频流状态", "示例不代表 PC 播放进度已确认", "ready"],
  ["paused", "已暂停", "暂停状态不视为错误", "warning"],
  ["reconnecting", "正在恢复", "有限恢复中，可见进度", "pending"],
  ["managementOnly", "管理连接", "可管理设备，未确认音频传输", "muted"],
  ["error", "操作未完成", "检查连接后重试或查看诊断", "error"],
] as const;

type Theme = "light" | "dark";
type AppearanceChoice = "system" | Theme;

function PreviewWindow({ theme, view }: { theme: Theme; view: "main" | "settings" }) {
  const english = new URLSearchParams(location.search).get("lang") === "en";
  const t = (zh: string, en: string) => english ? en : zh;
  const [appearanceChoice, setAppearanceChoice] = useState<AppearanceChoice>(theme);
  const appearance = appearanceChoice === "system" ? theme : appearanceChoice;
  const [wirelessMethod, setWirelessMethod] = useState<"udp" | "adb">("udp");
  const themeOptions: SegmentOption<AppearanceChoice>[] = [
    { value: "system", label: t("跟随系统", "System") },
    { value: "light", label: t("浅色", "Light") },
    { value: "dark", label: t("深色", "Dark") },
  ];

  return <article className="gallery-window" data-theme={appearance} aria-label={`${view} ${appearance}`}>
    <header className="gallery-titlebar">
      <span className="gallery-brand"><span className="gallery-brand-icon"><AudioLines size={17} /></span><strong>AnPod</strong></span>
      {view === "settings" && <button className="gallery-icon-button" aria-label={t("返回", "Back")}><span aria-hidden="true">←</span></button>}
      {view === "main" && <button className="gallery-icon-button" aria-label={t("设置", "Settings")}><Settings2 size={17} /></button>}
      <div className="gallery-window-controls" aria-hidden="true"><span>−</span><span>□</span><span>×</span></div>
    </header>

    {view === "main" ? <main className="gallery-main">
      <section className="device-section" aria-label={t("目标手机", "Target phone")}>
        <div className="section-title"><h1>{t("目标手机", "Target phone")}</h1></div>
        <div className="device-card">
          <button className="device-select" type="button" aria-haspopup="dialog" aria-label={t("选择手机和连接方式", "Choose phone and connection type")}>
            <span className="device-icon"><Smartphone size={18} aria-hidden="true" /></span>
            <span className="device-copy"><strong>{t("Pixel 8", "Pixel 8")}</strong><span>{t("USB Direct · 已连接", "USB Direct · Connected")}</span></span>
            <ChevronDown size={18} aria-hidden="true" />
          </button>
          <div className="device-actions"><button className="text-button" type="button">{t("断开连接", "Disconnect")}</button></div>
        </div>
        <button className="audio-settings-summary" type="button">
          <span>{t("均衡", "Balanced")} · {t("标准", "Standard")}</span>
          <span>{t("音频设置", "Audio settings")}</span>
        </button>
      </section>
      <section className="device-status-actions status-hero" aria-label={t("连接状态", "Connection status")}>
        <div className="status-line state-streaming"><span className="status-dot" /><span>{t("手机正在播放", "Playing on phone")}</span></div>
        <button className="primary-button main-action" type="button">{t("暂停", "Pause")}</button>
      </section>
      <footer className="gallery-footer"><span><CircleHelp size={14} />{t("帮助", "Help")}</span><span>v1.10.14</span><span><Activity size={14} />{t("诊断", "Diagnostics")}</span></footer>
    </main> : <main className="gallery-settings">
      <div className="gallery-settings-title"><h2>{t("设置", "Settings")}</h2><p>{t("管理连接、音频与界面偏好", "Manage connection, audio and appearance")}</p></div>
      <section className="gallery-settings-group">
        <h3>{t("界面", "Appearance")}</h3>
        <div className="gallery-setting-row"><strong>{t("主题", "Theme")}</strong><FluidSegmentedControl label={t("主题", "Theme")} value={appearanceChoice} options={themeOptions} columns={3} onValueChange={setAppearanceChoice} /></div>
        <label className="gallery-toggle-row"><span>{t("放大文字", "Large text")}</span><input type="checkbox" disabled /></label>
        <label className="gallery-toggle-row"><span>{t("最小化到托盘", "Minimize to tray")}</span><input type="checkbox" checked readOnly /></label>
      </section>
      <section className="gallery-settings-group">
        <h3>{t("音频", "Audio")}</h3>
        <strong className="gallery-setting-label">{t("音质档位", "Quality preference")}</strong>
        <FluidSegmentedControl label={t("音质档位", "Quality preference")} value="standard" options={[
          { value: "standard", label: t("标准", "Standard") },
          { value: "hifi", label: t("高保真", "Hi-Fi"), disabled: true },
          { value: "hires", label: "Hi-Res", disabled: true },
        ]} onValueChange={() => undefined} />
        <p className="gallery-note">{t("高级档当前不可用；所选档位不代表实际格式。", "Advanced modes are unavailable; preference does not confirm the actual format.")}</p>
        <div className="gallery-format-row"><span>{t("实际传输格式", "Actual transport format")}</span><strong>{t("未确认", "Not confirmed")}</strong></div>
      </section>
      <section className="gallery-settings-group">
        <h3>{t("连接", "Connection")}</h3>
        <div className="gallery-setting-row"><strong>{t("无线方式", "Wireless method")}</strong><FluidSegmentedControl label={t("无线方式", "Wireless method")} value={wirelessMethod} options={[
          { value: "udp", label: "UDP" },
          { value: "adb", label: "ADB" },
        ]} onValueChange={setWirelessMethod} /></div>
        <details className="gallery-disclosure"><summary>{t("高级连接设置", "Advanced connection options")}<small>{t("按需展开", "Expand when needed")}</small></summary><p>{t("部分选项只在满足设备条件时可用。", "Some options depend on device capability.")}</p></details>
      </section>
      <section className="gallery-settings-group gallery-about">
        <h3>{t("关于与诊断", "About & diagnostics")}</h3>
        <button type="button">{t("查看诊断", "View diagnostics")} <span aria-hidden="true">›</span></button>
      </section>
    </main>}
  </article>;
}

export function UiGallery() {
  const english = new URLSearchParams(location.search).get("lang") === "en";
  return <main className="ui-gallery">
    <header className="ui-gallery-header">
      <div><h1>AnPod · Soft Minimal</h1><p>{english ? "Static visual samples. They do not represent a live device or playback state." : "静态视觉样例，不代表真实设备或运行状态。"}</p></div>
      <a href={english ? "/__ui-gallery" : "/__ui-gallery?lang=en"}>{english ? "中文" : "English"}</a>
    </header>
    <section className="gallery-views" aria-label={english ? "Main and settings theme views" : "主界面与设置主题对照"}>
      <div className="gallery-view"><h2>{english ? "Main · Light" : "主界面 · 浅色"}</h2><PreviewWindow theme="light" view="main" /></div>
      <div className="gallery-view"><h2>{english ? "Main · Dark" : "主界面 · 深色"}</h2><PreviewWindow theme="dark" view="main" /></div>
      <div className="gallery-view"><h2>{english ? "Settings · Light" : "设置 · 浅色"}</h2><PreviewWindow theme="light" view="settings" /></div>
      <div className="gallery-view"><h2>{english ? "Settings · Dark" : "设置 · 深色"}</h2><PreviewWindow theme="dark" view="settings" /></div>
    </section>
    <section className="gallery-state-section">
      <div className="gallery-state-heading"><h2>{english ? "Status language samples" : "状态文案样例"}</h2><p>{english ? "Illustrative only; no device state is read or inferred." : "仅展示文案与色调，不读取或推断设备状态。"}</p></div>
      <div className="gallery-state-grid">{states.map(([key, title, detail, tone]) => <article key={key} className={`gallery-state-sample tone-${tone}`}>
        <span className={`gallery-state-dot ${tone}`} aria-hidden="true" />
        <div><strong>{english ? stateEnglish[key] : title}</strong><p>{english ? stateDetailEnglish[key] : detail}</p></div>
      </article>)}</div>
      <p className="gallery-disclaimer">{english ? "Actual display must follow Snapshot/readableStatus and validated diagnostics; unknown format stays unknown." : "实际状态须由 Snapshot、readableStatus 与经验证的诊断字段提供；未知格式继续显示未确认。"}</p>
    </section>
  </main>;
}

const stateEnglish: Record<string, string> = {
  noDevice: "No device", unauthorized: "Authorization needed", ready: "Ready", connecting: "Connecting", connected: "Connected", phoneReady: "Waiting to play", streaming: "Audio stream", paused: "Paused", reconnecting: "Recovering", managementOnly: "Management only", error: "Action incomplete",
};
const stateDetailEnglish: Record<string, string> = {
  noDevice: "Waiting for a phone", unauthorized: "Confirm USB debugging on the phone", ready: "Device ready; no session yet", connecting: "Cancelable connection operation", connected: "Connection does not confirm audio transfer", phoneReady: "Audio receipt is not confirmed", streaming: "Example does not confirm PC playback progress", paused: "Paused is not an error", reconnecting: "Bounded recovery with visible progress", managementOnly: "Device management; no audio transfer confirmed", error: "Check the connection or view diagnostics",
};
