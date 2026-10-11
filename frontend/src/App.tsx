import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { QRCodeSVG } from "qrcode.react";
import {
  Activity, AudioLines, Check, ChevronDown, CircleHelp, Computer, Copy,
  ExternalLink, Info, LoaderCircle, Search, Settings2,
  Smartphone, Usb, Wifi, X,
} from "lucide-react";
import type { ReactNode } from "react";

type Device = {
  serial: string; transport: "usb" | "wifi" | "unknown"; state: string;
  manufacturer: string; model: string; androidVersion: string; apiLevel: string;
  displayName: string; statusText: string; connectable: boolean;
};
type Settings = {
  schemaVersion: number; autoConnectLastDevice: boolean; lastDeviceId: string | null;
  minimizeToTray: boolean; launchAtLogin: boolean; preferredTransport: "Usb" | "Wifi";
  autoReconnectWifi: boolean; lastWirelessEndpoint: string | null;
  playbackMode: "Balanced" | "LowLatency"; appearanceTheme: "System" | "Light" | "Dark";
};
type Snapshot = {
  state: "noDevice" | "unauthorized" | "ready" | "connecting" | "connected" | "streaming" | "reconnecting" | "error";
  message: string; deviceName: string | null; devices: Device[]; selectedSerial: string | null; sessionSerial: string | null;
  sessionTransport: Device["transport"] | null; sessionAndroidVersion: string | null;
  transport: "usb" | "wifi"; output: "computer" | "phone"; settings: Settings;
  busy: boolean; canDisconnect: boolean; canChangeModes: boolean; statusDetail: string; version: string;
};
type Credentials = { instanceName: string; password: string; payload: string };
type MdnsResults = { available: boolean; services: { name: string; endpoint: string; kind: "pairing" | "connect" }[] };

const isDesktop = "__TAURI_INTERNALS__" in window;
const invokeApp = <T,>(command: string, args?: Record<string, unknown>) => {
  if (!isDesktop) return Promise.reject(new Error("请从 AnPod 桌面程序打开此界面。"));
  return invoke<T>(command, args);
};
const byTransport = (snapshot: Snapshot) => snapshot.devices.filter(device => device.transport === snapshot.transport || device.transport === "unknown");

function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [panel, setPanel] = useState<"device" | "settings" | "pair" | "help" | "diagnostics" | null>(null);
  const [confirm, setConfirm] = useState<{ kind: "transport" | "playback" | "device"; value: string } | null>(null);
  const [localSelected, setLocalSelected] = useState<string | null>(null);
  const [largeText, setLargeText] = useState(false);
  const [notice, setNotice] = useState("");
  const [working, setWorking] = useState(false);

  const apply = (value: Snapshot) => { setSnapshot(value); setLocalSelected(value.selectedSerial); };
  useEffect(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const retain = (cleanup: () => void) => { if (disposed) cleanup(); else cleanups.push(cleanup); };
    const attach = async () => {
      if (!isDesktop) return;
      retain(await listen<Snapshot>("anpod://snapshot", event => apply(event.payload)));
      if (disposed) return;
      const initial = await invokeApp<Snapshot>("get_snapshot");
      if (disposed) return;
      apply(initial);
      const refreshed = await invokeApp<Snapshot>("refresh_devices");
      if (disposed) return;
      apply(refreshed);
      const show = await listen("anpod://show-window", () => { void invokeApp("show_main_window"); });
      retain(show);
      const pairing = await listen<string>("anpod://pairing-progress", event => setNotice(event.payload));
      retain(pairing);
    };
    void attach().catch(error => { if (!disposed) setNotice(String(error)); });
    return () => { disposed = true; cleanups.splice(0).forEach(cleanup => cleanup()); };
  }, []);

  useEffect(() => {
    if (!snapshot || snapshot.settings.appearanceTheme !== "System") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const update = () => document.documentElement.dataset.theme = media.matches ? "dark" : "light";
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [snapshot?.settings.appearanceTheme]);

  const devices = useMemo(() => snapshot ? byTransport(snapshot) : [], [snapshot]);
  const selected = devices.find(device => device.serial === localSelected) ?? devices.find(device => device.connectable);
  const active = !!snapshot?.sessionSerial;
  const canDisconnect = !!snapshot?.canDisconnect;
  const cleanupPending = canDisconnect && !active;
  const activeDevice = snapshot?.devices.find(device => device.serial === snapshot.sessionSerial);
  const cardDevice = active ? activeDevice : cleanupPending ? undefined : selected;
  const deviceChoices = useMemo(() => {
    if (!snapshot?.sessionSerial || devices.some(device => device.serial === snapshot.sessionSerial)) return devices;
    const current = snapshot.devices.find(device => device.serial === snapshot.sessionSerial) ?? {
      serial: snapshot.sessionSerial, transport: snapshot.sessionTransport ?? "unknown", state: "device", manufacturer: "", model: "",
      androidVersion: snapshot.sessionAndroidVersion ?? "", apiLevel: "", displayName: snapshot.deviceName ?? "已连接手机",
      statusText: "当前连接；设备扫描暂不可用", connectable: false,
    };
    return [current, ...devices];
  }, [snapshot, devices]);
  const transportLabel = (transport?: Device["transport"] | null) => transport === "usb" ? "USB · ADB" : transport === "wifi" ? "Wi-Fi · ADB TCP" : "连接方式未知";
  const notify = (message: string) => { setNotice(message); window.setTimeout(() => setNotice(""), 4200); };

  const refresh = async () => {
    setWorking(true);
    try { apply(await invokeApp<Snapshot>("refresh_devices")); }
    catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const setOption = async (kind: "transport" | "playback", value: string) => {
    if (!snapshot) return;
    if (snapshot.busy || cleanupPending) return;
    if (active) { setConfirm({ kind, value }); return; }
    setWorking(true);
    try {
      if (kind === "transport") await invokeApp("update_setting", { key: "preferredTransport", value: value === "wifi" ? "Wifi" : "Usb" });
      else await invokeApp("update_setting", { key: "playbackMode", value: value === "lowLatency" ? "LowLatency" : "Balanced" });
      apply(await invokeApp<Snapshot>("refresh_devices"));
      setNotice("");
    } catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const applyConfirmed = async () => {
    if (!confirm) return;
    setWorking(true);
    try {
      if (confirm.kind === "device") {
        const result = await invokeApp<{ success: boolean; message: string; snapshot: Snapshot }>("switch_device", { serial: confirm.value });
        apply(result.snapshot);
        if (!result.success) notify(result.message);
      } else {
        const next = await invokeApp<Snapshot>("reconfigure_session", { kind: confirm.kind, value: confirm.value });
        apply(next);
      }
      setConfirm(null);
    } catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const setOutput = async (output: "computer" | "phone") => {
    if (!snapshot) return;
    if (snapshot.busy || cleanupPending) return;
    if (output === "phone" && !active && !selected) {
      if (snapshot.transport === "wifi") setPanel("pair"); else setPanel("help");
      return;
    }
    if (output === "phone" && !active && selected) {
      setWorking(true);
      try {
        const result = await invokeApp<{ success: boolean; message: string; snapshot: Snapshot }>("connect_device", { serial: localSelected ?? selected.serial });
        apply(result.snapshot);
        if (!result.success) notify(result.message);
      } catch (error) { notify(String(error)); }
      finally { setWorking(false); }
      return;
    }
    setWorking(true);
    try { apply(await invokeApp<Snapshot>("set_output", { output })); }
    catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const disconnect = async () => {
    setWorking(true);
    try {
      const result = await invokeApp<{ success: boolean; message: string; snapshot: Snapshot }>("disconnect");
      apply(result.snapshot);
      if (result.success) setNotice(""); else notify(result.message);
    } catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const connectOnly = async () => {
    if (!selected?.connectable) return;
    setWorking(true);
    try {
      const result = await invokeApp<{ success: boolean; message: string; snapshot: Snapshot }>("connect_device_only", { serial: selected.serial });
      apply(result.snapshot);
      if (!result.success) notify(result.message);
    } catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const chooseDevice = (device: Device) => {
    setPanel(null);
    if (canDisconnect) {
      if (device.serial === snapshot.sessionSerial) return;
      setConfirm({ kind: "device", value: device.serial });
      return;
    }
    if (device.serial === localSelected) return;
    void invokeApp<Snapshot>("select_device", { serial: device.serial }).then(apply).catch(error => notify(String(error)));
  };

  if (!snapshot) return <main className="loading-shell"><AudioLines size={25} /><span>正在启动 AnPod…</span></main>;
  const empty = devices.length === 0;
  const stateLabels: Record<Snapshot["state"], string> = {
    noDevice: empty ? "尚未连接手机" : "等待手机授权", unauthorized: "等待手机授权",
    ready: "准备就绪", connecting: "正在连接", connected: "已连接 · 电脑播放",
    streaming: "正在手机播放", reconnecting: "正在重新连接", error: "连接失败",
  };

  return <div className={`app-shell ${largeText ? "large-text" : ""}`}>
    <header className="topbar">
      <div className="brand"><img src="/icon.png" alt="" /><div><strong>AnPod</strong><span>电脑声音，手机播放</span></div></div>
      <button className="icon-button" aria-label="打开设置" title="设置" onClick={() => setPanel("settings")}><Settings2 size={18} /></button>
    </header>

    <main className="main-scroll">
      <div className={`status-line state-${snapshot.state}`} role="status" aria-live="polite">
        <span className="status-dot" aria-hidden="true" />
        <span>{stateLabels[snapshot.state]}</span>
        {snapshot.deviceName && <span className="status-device">{snapshot.deviceName}</span>}
        <span className="transport-tag">{snapshot.transport.toUpperCase()}</span>
      </div>

      <section className="playback-section" aria-label="声音输出">
        <div className="section-title"><h1>声音输出</h1><button className="text-button" onClick={() => void refresh()} disabled={working || snapshot.busy} aria-label="刷新设备"><LoaderCircle className={working || snapshot.busy ? "spin" : ""} size={15} />刷新</button></div>
        <div className="output-grid" role="group" aria-label="选择声音输出">
          <button className={`output-card ${snapshot.output === "computer" ? "selected" : ""}`} aria-pressed={snapshot.output === "computer"} onClick={() => void setOutput("computer")} disabled={working || snapshot.busy || cleanupPending || (!active && snapshot.output === "computer")}>
            <span className="output-heading"><Computer size={18} aria-hidden="true" /><span className="output-name">电脑</span></span>
          </button>
          <button className={`output-card ${snapshot.output === "phone" ? "selected" : ""}`} aria-pressed={snapshot.output === "phone"} onClick={() => void setOutput("phone")} disabled={working || snapshot.busy || cleanupPending}>
            <span className="output-heading"><Smartphone size={18} aria-hidden="true" /><span className="output-name">手机</span></span>
          </button>
        </div>
        <p className="status-description">{snapshot.statusDetail}</p>
        <div className="session-row">
        <span>{snapshot.busy ? "正在处理手机连接" : cleanupPending ? "连接清理待完成" : active ? "手机连接保持中" : empty ? "声音继续从电脑播放" : "连接手机后可从手机输出声音"}</span>
          {(snapshot.state === "connecting" || snapshot.state === "reconnecting") && <button className="text-button cancel-button" onClick={() => void invokeApp<Snapshot>("cancel_operation").then(apply).catch(error => notify(String(error)))}>取消连接</button>}
        </div>
      </section>

      <section className="connection-section" aria-label="连接手机">
        <div className="transport-switch" role="group" aria-label="连接方式">
          <button aria-pressed={snapshot.transport === "usb"} disabled={working || snapshot.busy || cleanupPending} onClick={() => void setOption("transport", "usb")}><Usb size={16} />USB</button>
          <button aria-pressed={snapshot.transport === "wifi"} disabled={working || snapshot.busy || cleanupPending} onClick={() => void setOption("transport", "wifi")}><Wifi size={16} />Wi-Fi</button>
        </div>
        <div className="section-title device-heading"><h2>连接手机</h2><button className="text-button" disabled={working || snapshot.busy} onClick={() => setPanel(snapshot.transport === "wifi" ? "pair" : "help")}>{snapshot.transport === "wifi" ? "添加手机" : "连接帮助"}</button></div>
        <div className="device-card">
          <button className="device-select" onClick={() => setPanel("device")} disabled={working || snapshot.busy} aria-haspopup="dialog" aria-expanded={panel === "device"} aria-label={`选择手机，${active ? `当前连接${activeDevice?.displayName ?? snapshot.deviceName ?? "手机"}` : cardDevice ? `当前选择${cardDevice.displayName}` : "打开设备列表"}`}>
            <span className="device-icon"><Smartphone size={18} aria-hidden="true" /></span>
            <span className="device-copy">
              <strong>{active ? activeDevice?.displayName ?? snapshot.deviceName ?? "已连接手机" : cleanupPending ? snapshot.deviceName ?? "连接清理待完成" : selected?.displayName ?? (empty ? "未检测到手机" : "选择手机")}</strong>
              <span>{active ? `${transportLabel(snapshot.sessionTransport)}${snapshot.sessionAndroidVersion ? ` · Android ${snapshot.sessionAndroidVersion}` : ""}` : cleanupPending ? "当前连接信息不可用" : cardDevice ? `${transportLabel(cardDevice.transport)}${cardDevice.androidVersion ? ` · Android ${cardDevice.androidVersion}` : ""}` : snapshot.transport === "wifi" ? "通过扫码、配对码或手动地址添加" : "连接 USB 手机后刷新设备列表"}</span>
              {(active || cleanupPending) && <span className="device-live-state" role="status">{cleanupPending ? "连接清理待重试" : stateLabels[snapshot.state]}</span>}
            </span>
            <ChevronDown size={18} aria-hidden="true" />
          </button>
          <button className="primary-button device-action" disabled={working || snapshot.busy || (!canDisconnect && !selected?.connectable)} onClick={() => canDisconnect ? void disconnect() : void connectOnly()}>
            {working || snapshot.busy ? <><LoaderCircle className="spin" size={15} />处理中</> : canDisconnect ? "断开连接" : "连接"}
          </button>
        </div>
      </section>

      <section className="mode-section" aria-label="播放模式">
        <div className="mode-row"><h2>播放模式</h2><div className="mode-switch" role="group" aria-label="播放模式">
          <button aria-pressed={snapshot.settings.playbackMode === "Balanced"} disabled={working || snapshot.busy || cleanupPending} onClick={() => void setOption("playback", "balanced")}>均衡</button>
          <button aria-pressed={snapshot.settings.playbackMode === "LowLatency"} disabled={working || snapshot.busy || cleanupPending} onClick={() => void setOption("playback", "lowLatency")}>低延迟</button>
        </div></div>
        <p>{snapshot.settings.playbackMode === "Balanced" ? "优先播放稳定，适合长时间收听。" : "缩短缓冲，适合互动和游戏声音。"}</p>
      </section>
    </main>

    <footer className="footer"><button onClick={() => setPanel("help")}><CircleHelp size={15} />连接帮助</button><span>v{snapshot.version}</span><button onClick={() => setPanel("diagnostics")}><Activity size={15} />诊断</button></footer>
    {notice && <div className="toast" role="status">{notice}</div>}

    {panel === "device" && <DeviceDialog devices={deviceChoices} selected={localSelected} activeSerial={snapshot.sessionSerial} busy={working || snapshot.busy} onSelect={chooseDevice} onRefresh={refresh} onClose={() => setPanel(null)} />}
    {panel === "settings" && <SettingsDialog snapshot={snapshot} largeText={largeText} onLargeText={setLargeText} onSave={async (key, value) => {
      try { const next = await invokeApp<Snapshot>("update_setting", { key, value }); apply(next); if (key === "preferredTransport") apply(await invokeApp<Snapshot>("refresh_devices")); }
      catch (error) { notify(String(error)); }
    }} onClose={() => setPanel(null)} />}
    {panel === "pair" && <PairDialog transport={snapshot.transport} initialEndpoint={snapshot.settings.lastWirelessEndpoint ?? ""} disabled={snapshot.busy} onClose={() => setPanel(null)} onDone={async () => { apply(await invokeApp<Snapshot>("refresh_devices")); setPanel("device"); }} />}
    {panel === "help" && <HelpDialog transport={snapshot.transport} onClose={() => setPanel(null)} />}
    {panel === "diagnostics" && <DiagnosticsDialog snapshot={snapshot} onClose={() => setPanel(null)} />}
    {confirm && <ConfirmDialog
      title={confirm.kind === "device" ? "切换连接手机？" : confirm.kind === "transport" ? "切换连接方式？" : "切换播放模式？"}
      body={confirm.kind === "device" ? `切换到 ${snapshot.devices.find(device => device.serial === confirm.value)?.displayName ?? "所选手机"} 会短暂中断当前连接；${snapshot.output === "phone" ? "完成后会继续从手机输出。" : "完成后声音仍从电脑输出。"}` : "切换时会短暂中断声音，AnPod 会清理当前连接后重新连接同一部手机。"}
      confirmLabel={confirm.kind === "device" ? "中断并切换" : "断开并重新连接"} busy={working} onCancel={() => setConfirm(null)} onConfirm={() => void applyConfirmed()} />}
  </div>;
}

function Modal({ title, children, onClose, className = "" }: { title: string; children: ReactNode; onClose: () => void; className?: string }) {
  const ref = useRef<HTMLDialogElement>(null);
  const onCloseRef = useRef(onClose);
  useEffect(() => { onCloseRef.current = onClose; });
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (!dialog.open) dialog.showModal();
    const key = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); onCloseRef.current(); } };
    dialog.addEventListener("keydown", key);
    return () => { dialog.removeEventListener("keydown", key); if (dialog.open) dialog.close(); };
  }, []);
  return <dialog ref={ref} aria-label={title} className={`modal ${className}`} onCancel={event => { event.preventDefault(); onClose(); }} onClick={event => { if (event.target === ref.current) onClose(); }}>
    <div className="modal-head"><h2>{title}</h2><button className="icon-button" onClick={onClose} aria-label="关闭"><X size={18} /></button></div>{children}
  </dialog>;
}

function DeviceDialog({ devices, selected, activeSerial, busy, onSelect, onRefresh, onClose }: { devices: Device[]; selected: string | null; activeSerial: string | null; busy: boolean; onSelect: (device: Device) => void; onRefresh: () => void; onClose: () => void }) {
  return <Modal title="选择手机" onClose={onClose} className="device-dialog">
    <div className="dialog-tools"><span>只显示当前连接方式的设备</span><button className="text-button" onClick={onRefresh} disabled={busy}><Search size={15} />刷新</button></div>
    <div className="device-list">{devices.map(device => { const connected = activeSerial === device.serial; const chosen = (activeSerial ?? selected) === device.serial; return <button className="device-option" key={device.serial} onClick={() => onSelect(device)} disabled={busy || (!connected && !device.connectable)} aria-pressed={chosen}>
      <Smartphone size={18} aria-hidden="true" /><span><strong>{device.displayName}</strong><small>{connected ? "当前连接 · " : ""}{device.statusText}{device.transport !== "unknown" ? ` · ${device.transport === "wifi" ? "Wi-Fi · ADB TCP" : "USB · ADB"}` : " · 连接方式未知"}{device.androidVersion ? ` · Android ${device.androidVersion}` : ""}</small></span>{chosen && <Check size={17} aria-hidden="true" />}
    </button>; })}</div>
    {devices.length === 0 && <div className="empty-note"><Info size={17} /><span>尚未发现设备。{" "} {" “添加手机”可通过扫码、配对码或 IP:端口地址添加。"}</span></div>}
    <button className="dialog-secondary" onClick={onClose}>完成</button>
  </Modal>;
}

function SettingsDialog({ snapshot, largeText, onLargeText, onSave, onClose }: { snapshot: Snapshot; largeText: boolean; onLargeText: (value: boolean) => void; onSave: (key: string, value: unknown) => void; onClose: () => void }) {
  const settings = snapshot.settings;
  const update = (key: keyof Settings, value: boolean | string) => { void onSave(key, value); };
  return <Modal title="设置" onClose={onClose} className="settings-dialog">
    <h3>连接</h3>
    <SettingToggle disabled={snapshot.busy} title="自动连接上次使用的手机" description="设备重新出现时尝试连接" checked={settings.autoConnectLastDevice} onChange={value => update("autoConnectLastDevice", value)} />
    <SettingToggle disabled={snapshot.busy} title="无线连接恢复" description="无线掉线后进行有限次数的重连" checked={settings.autoReconnectWifi} onChange={value => update("autoReconnectWifi", value)} />
    <h3>窗口和外观</h3>
    <SettingToggle disabled={snapshot.busy} title="关闭时最小化到托盘" description="从托盘菜单可以重新打开 AnPod" checked={settings.minimizeToTray} onChange={value => update("minimizeToTray", value)} />
    <SettingToggle title="放大文字" description="增加界面文字尺寸" checked={largeText} onChange={onLargeText} />
    <label className="select-setting"><span>主题</span><select disabled={snapshot.busy} value={settings.appearanceTheme} onChange={event => update("appearanceTheme", event.target.value)}><option value="System">跟随系统</option><option value="Light">浅色</option><option value="Dark">深色</option></select></label>
    {snapshot.busy && <p className="dialog-footnote">正在连接，暂时无法更改设置。</p>}
    <p className="dialog-footnote">配置保存在本机，不包含配对码或握手令牌。</p>
  </Modal>;
}

function SettingToggle({ title, description, checked, disabled = false, onChange }: { title: string; description: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void }) {
  return <label className="setting-toggle"><span><strong>{title}</strong><small>{description}</small></span><input type="checkbox" checked={checked} disabled={disabled} onChange={event => onChange(event.target.checked)} /></label>;
}

function PairDialog({ transport, initialEndpoint, disabled, onClose, onDone }: { transport: "usb" | "wifi"; initialEndpoint: string; disabled: boolean; onClose: () => void; onDone: () => void }) {
  const [tab, setTab] = useState<"qr" | "code" | "address">("qr");
  const [credentials, setCredentials] = useState<Credentials | null>(null);
  const [endpoint, setEndpoint] = useState(initialEndpoint);
  const [code, setCode] = useState("");
  const [message, setMessage] = useState("正在生成安全配对码…");
  const [busy, setBusy] = useState(false);
  const [qrStarted, setQrStarted] = useState(false);
  const [error, setError] = useState("");
  const taskGeneration = useRef(0);
  useEffect(() => {
    ++taskGeneration.current;
    return () => { ++taskGeneration.current; };
  }, [transport, tab]);
  useEffect(() => {
    if (transport !== "wifi" || tab !== "qr") return;
    const generation = taskGeneration.current;
    void invokeApp<Credentials>("qr_credentials").then(value => { if (taskGeneration.current === generation) setCredentials(value); }).catch(reason => { if (taskGeneration.current === generation) setError(String(reason)); });
    return () => { void invokeApp("cancel_qr_pairing"); };
  }, [transport, tab]);
  const currentTask = () => taskGeneration.current;
  const finish = async (command: string, args: Record<string, unknown>) => {
    const generation = currentTask();
    setBusy(true); setError("");
    try {
      const result = await invokeApp<{ success: boolean; message: string; snapshot?: Snapshot }>(command, args);
      if (generation !== taskGeneration.current) return;
      setMessage(result.message);
      if (result.success) await onDone();
      else setError(result.message);
    } catch (reason) { if (generation === taskGeneration.current) setError(String(reason)); }
    finally { if (generation === taskGeneration.current) { setBusy(false); setCode(""); } }
  };
  const discover = async () => {
    const generation = currentTask();
    setBusy(true); setError("");
    try {
      const result = await invokeApp<MdnsResults>("discover_wifi");
      if (generation !== taskGeneration.current) return;
      const found = result.services.filter(service => service.kind === "connect");
      if (found.length === 1) setEndpoint(found[0].endpoint);
      else if (found.length > 1) setMessage(`发现 ${found.length} 台手机，请确认后选择连接地址。`);
      else setMessage("没有发现手机；请手动输入无线调试的连接地址。配对端口和连接端口不同。");
    } catch (reason) { if (generation === taskGeneration.current) setError(String(reason)); }
    finally { if (generation === taskGeneration.current) setBusy(false); }
  };
  return <Modal title={transport === "wifi" ? "添加无线手机" : "连接 USB 手机"} onClose={onClose} className="pair-dialog">
    {transport === "wifi" ? <>
      <div className="tabs" role="tablist" aria-label="添加方式"><button role="tab" aria-selected={tab === "qr"} disabled={disabled || busy} onClick={() => setTab("qr")}>扫码配对</button><button role="tab" aria-selected={tab === "code"} disabled={disabled || busy} onClick={() => setTab("code")}>配对码</button><button role="tab" aria-selected={tab === "address"} disabled={disabled || busy} onClick={() => setTab("address")}>手动地址</button></div>
      {tab === "qr" && <div className="qr-pane">
        {credentials ? <div className="qr-frame"><QRCodeSVG value={credentials.payload} size={174} level="Q" includeMargin /></div> : <div className="qr-frame qr-wait"><LoaderCircle className="spin" /></div>}
        <p className="qr-help">在手机「无线调试」中选择“使用二维码配对设备”并扫描。配对码仅保存在当前窗口。</p>
        <p className="pair-progress" role="status" aria-live="polite">{message}</p>
        {!qrStarted && <button className="primary-button" disabled={disabled || !credentials || busy} onClick={() => {
          if (!credentials) return;
          const generation = currentTask();
          setQrStarted(true); setBusy(true);
          void invokeApp<{ success: boolean; message: string }>("pair_qr", { instanceName: credentials.instanceName, password: credentials.password })
            .then(async result => { if (generation !== taskGeneration.current) return; setMessage(result.message); if (result.success) await onDone(); else { setError(result.message); setQrStarted(false); setCredentials(null); void invokeApp<Credentials>("qr_credentials").then(value => { if (generation === taskGeneration.current) setCredentials(value); }); } })
            .catch(reason => { if (generation === taskGeneration.current) { setError(String(reason)); setQrStarted(false); } }).finally(() => { if (generation === taskGeneration.current) setBusy(false); });
        }}>{busy ? "等待手机扫描…" : "开始扫码配对"}</button>}
        {qrStarted && <button className="dialog-secondary" onClick={() => { ++taskGeneration.current; void invokeApp("cancel_qr_pairing"); setQrStarted(false); setBusy(false); setMessage("配对已取消。"); }}>取消配对</button>}
      </div>}
      {tab === "code" && <form className="form-pane" onSubmit={event => { event.preventDefault(); void finish("pair_wifi", { endpoint, code }); }}>
        <label>手机配对地址<input disabled={disabled || busy} value={endpoint} onChange={event => setEndpoint(event.target.value)} autoComplete="off" placeholder="192.0.2.20:37001" /></label>
        <label>6 位配对码<input disabled={disabled || busy} value={code} onChange={event => setCode(event.target.value.replace(/\D/g, "").slice(0, 6))} inputMode="numeric" autoComplete="one-time-code" aria-describedby="pair-code-hint" placeholder="123456" /></label>
        <small id="pair-code-hint">配对码来自手机的“使用配对码配对设备”页面。</small>
        <button className="primary-button" disabled={disabled || busy || code.length !== 6}>{busy ? "正在配对…" : "配对并添加"}</button>
      </form>}
      {tab === "address" && <form className="form-pane" onSubmit={event => { event.preventDefault(); void finish("connect_wifi", { endpoint }); }}>
        <label>无线调试连接地址<input disabled={disabled || busy} value={endpoint} onChange={event => setEndpoint(event.target.value)} autoComplete="off" placeholder="192.0.2.20:5555" /></label>
        <small>请使用手机无线调试主页面显示的连接端口，不要使用配对端口。</small>
        <button className="text-button discover-button" type="button" onClick={() => void discover()} disabled={disabled || busy}><Search size={15} />搜索局域网设备</button>
        <button className="primary-button" disabled={disabled || busy || !endpoint.trim()}>{busy ? "正在连接…" : "连接地址"}</button>
      </form>}
    </> : <div className="help-pane"><p>1. 在手机开发者选项中开启 USB 调试。</p><p>2. 使用支持数据传输的 USB 线连接电脑。</p><p>3. 解锁手机并允许此电脑进行 USB 调试。</p><button className="primary-button" onClick={async () => { await onDone(); }}>刷新设备列表</button></div>}
    {disabled && <p className="dialog-footnote" role="status">手机连接处理中，完成后可继续添加。</p>}
    {error && <p className="inline-error" role="alert">{error}</p>}
  </Modal>;
}

function HelpDialog({ transport, onClose }: { transport: "usb" | "wifi"; onClose: () => void }) {
  return <Modal title="连接帮助" onClose={onClose} className="help-dialog">
    {transport === "usb" ? <><h3>USB 调试</h3><ol><li>在手机开发者选项中开启 USB 调试。</li><li>用支持数据的线连接手机和电脑。</li><li>解锁手机，在授权提示中允许此电脑。</li></ol></> : <><h3>无线调试</h3><ol><li>让电脑和手机连接同一可互访网络。</li><li>在手机无线调试中扫描二维码，或输入配对端口和 6 位码。</li><li>连接地址使用无线调试主页面的连接端口。</li></ol></>}
    <p>手机端无需安装可见 App。配对仅授予 ADB 权限，不会自动开始播放。</p><button className="dialog-secondary" onClick={onClose}>完成</button>
  </Modal>;
}

function DiagnosticsDialog({ snapshot, onClose }: { snapshot: Snapshot; onClose: () => void }) {
  const text = [`状态：${snapshot.message}`, `连接方式：${snapshot.transport.toUpperCase()}`, `设备：${snapshot.deviceName ?? "无"}`, `输出：${snapshot.output === "phone" ? "手机" : "电脑"}`, `可见设备：${snapshot.devices.length}`, `版本：${snapshot.version}`].join("\n");
  return <Modal title="诊断" onClose={onClose} className="diagnostics-dialog">
    <p>本次运行状态</p><pre>{text}</pre><div className="dialog-actions"><button className="dialog-secondary" onClick={() => void navigator.clipboard.writeText(text)}><Copy size={15} />复制诊断</button><button className="dialog-secondary" onClick={() => void invokeApp("open_logs")}>查看日志文件夹<ExternalLink size={14} /></button></div>
  </Modal>;
}

function ConfirmDialog({ title, body, confirmLabel, busy, onConfirm, onCancel }: { title: string; body: string; confirmLabel: string; busy: boolean; onConfirm: () => void; onCancel: () => void }) {
  return <Modal title={title} onClose={onCancel} className="confirm-dialog"><p>{body}</p><div className="dialog-actions"><button className="dialog-secondary" onClick={onCancel} disabled={busy}>取消</button><button className="primary-button" onClick={onConfirm} disabled={busy}>{busy ? "正在切换…" : confirmLabel}</button></div></Modal>;
}

export default App;
