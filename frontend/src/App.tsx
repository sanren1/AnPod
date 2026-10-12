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
  state: "noDevice" | "unauthorized" | "ready" | "connecting" | "connected" | "streaming" | "phoneReady" | "paused" | "reconnecting" | "error";
  message: string; deviceName: string | null; devices: Device[]; selectedSerial: string | null; sessionSerial: string | null;
  sessionReceiverBinding: string | null; sessionTransport: Device["transport"] | "lan" | null; sessionAndroidVersion: string | null;
  transport: "usb" | "wifi"; output: "computer" | "phone"; settings: Settings;
  busy: boolean; canDisconnect: boolean; canChangeModes: boolean; statusDetail: string;
  audioBitrateBps: number | null; version: string;
};
type Credentials = { instanceName: string; password: string; payload: string };
type MdnsResults = { available: boolean; services: { name: string; endpoint: string; kind: "pairing" | "connect" }[] };
type PrivateIpv4Interface = { index: number; name: string; address: string; prefixLength: number; gateways: string[] };
type ReceiverIpv4Interface = { address: string; prefixLength: number; interfaceIndex: number; interfaceName: string };
type ReceiverStatus = {
  installed: boolean; versionName: string | null; versionCode: number | null; serviceActive: boolean;
  foreground: boolean; healthy: boolean; audioActive: boolean; error: string | null; message: string;
};
type NetworkCandidate = { endpoint: string };
type NetworkStartRequest = {
  localAddress: string; peer: string; bootstrapSerial?: string; receiverAddress?: string;
  receiverPrefixLength?: number; bindingReference?: string;
};
type OperationResult = { success: boolean; message: string; snapshot: Snapshot };

const isDesktop = "__TAURI_INTERNALS__" in window;
const invokeApp = <T,>(command: string, args?: Record<string, unknown>) => {
  if (!isDesktop) return Promise.reject(new Error("请从 AnPod 桌面程序打开此界面。"));
  return invoke<T>(command, args);
};
const byTransport = (snapshot: Snapshot) => snapshot.devices.filter(device => device.transport === snapshot.transport || device.transport === "unknown");
const hasActiveSession = (snapshot: Snapshot | null) => !!snapshot?.sessionSerial || !!snapshot?.sessionReceiverBinding || snapshot?.sessionTransport === "lan";
const connectCommandForOutput = (output: Snapshot["output"]) => output === "phone" ? "connect_device" : "connect_device_only";
const formatBitrate = (bps: number | null) => bps === null ? "—" : bps >= 1_000_000 ? `${(bps / 1_000_000).toFixed(2)} Mbps` : bps >= 1_000 ? `${(bps / 1_000).toFixed(1)} kbps` : `${bps} bps`;
const bitrateHelp = "只统计实际成功发送的 PCM payload，不含 UDP/TCP/ADB 等网络开销。暂停或电脑输出时显示后端实际值（可能为 0）；不是延迟、链路带宽或接收端吞吐。";
const transportLabel = (transport?: Snapshot["sessionTransport"]) => transport === "lan" ? "直接 UDP · 加密认证"
  : transport === "usb" ? "USB · ADB" : transport === "wifi" ? "Wi-Fi · ADB TCP" : "连接方式未知";
const bindingLabel = (reference: string | null | undefined) => reference ? `已配对接收端 · ${reference.slice(0, 8)}` : "已配对接收端";
const isIpv4Endpoint = (value: string) => {
  const match = /^(\d{1,3}(?:\.\d{1,3}){3}):(\d{1,5})$/.exec(value.trim());
  return !!match && match[1].split(".").every(part => Number(part) <= 255) && Number(match[2]) > 0 && Number(match[2]) <= 65535;
};

function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [panel, setPanel] = useState<"device" | "settings" | "pair" | "network" | "help" | "diagnostics" | null>(null);
  const [confirm, setConfirm] = useState<{ kind: "transport" | "playback" | "device" | "network"; value: string } | null>(null);
  const [pendingNetworkStart, setPendingNetworkStart] = useState<NetworkStartRequest | null>(null);
  const [networkDialogError, setNetworkDialogError] = useState("");
  const [localSelected, setLocalSelected] = useState<string | null>(null);
  const [explicitWifiDeviceSerial, setExplicitWifiDeviceSerial] = useState<string | null>(null);
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
  const active = hasActiveSession(snapshot);
  const lanSession = active && snapshot?.sessionTransport === "lan";
  const canDisconnect = !!snapshot?.canDisconnect;
  const cleanupPending = canDisconnect && !active && snapshot?.state === "error";
  const activeDevice = snapshot?.devices.find(device => device.serial === snapshot.sessionSerial);
  const cardDevice = active ? activeDevice : cleanupPending ? undefined : selected;
  const deviceChoices = useMemo(() => {
    if (!snapshot?.sessionSerial || devices.some(device => device.serial === snapshot.sessionSerial)) return devices;
    const current = snapshot.devices.find(device => device.serial === snapshot.sessionSerial) ?? {
      serial: snapshot.sessionSerial, transport: snapshot.sessionTransport === "lan" ? "unknown" : snapshot.sessionTransport ?? "unknown", state: "device", manufacturer: "", model: "",
      androidVersion: snapshot.sessionAndroidVersion ?? "", apiLevel: "", displayName: snapshot.deviceName ?? "已连接手机",
      statusText: "当前连接；设备扫描暂不可用", connectable: false,
    };
    return [current, ...devices];
  }, [snapshot, devices]);
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
    if ((kind === "transport" && value === snapshot.transport)
      || (kind === "playback" && value === (snapshot.settings.playbackMode === "LowLatency" ? "lowLatency" : "balanced"))) return;
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
      if (confirm.kind === "network") {
        if (!pendingNetworkStart) throw new Error("网络连接参数已失效；请重新选择接收端。");
        const stopped = await invokeApp<OperationResult>("disconnect");
        apply(stopped.snapshot);
        if (!stopped.success) throw new Error(stopped.message);
        apply(await invokeApp<Snapshot>("start_network_audio", { ...pendingNetworkStart }));
        setPanel(null);
      } else if (confirm.kind === "device" && lanSession) {
        const output = snapshot?.output ?? "computer";
        const stopped = await invokeApp<OperationResult>("disconnect");
        apply(stopped.snapshot);
        if (!stopped.success) throw new Error(stopped.message);
        const command = connectCommandForOutput(output);
        const result = await invokeApp<OperationResult>(command, { serial: confirm.value });
        apply(result.snapshot);
        if (!result.success) notify(result.message);
      } else if (confirm.kind === "transport" && lanSession) {
        const stopped = await invokeApp<OperationResult>("disconnect");
        apply(stopped.snapshot);
        if (!stopped.success) throw new Error(stopped.message);
        await invokeApp("update_setting", { key: "preferredTransport", value: confirm.value === "wifi" ? "Wifi" : "Usb" });
        apply(await invokeApp<Snapshot>("refresh_devices"));
        setNotice("直接网络会话已断开；连接方式已更新。需要时请重新选择手机。");
      } else if (confirm.kind === "device") {
        const result = await invokeApp<OperationResult>("switch_device", { serial: confirm.value });
        apply(result.snapshot);
        if (!result.success) notify(result.message);
      } else {
        const next = await invokeApp<Snapshot>("reconfigure_session", { kind: confirm.kind, value: confirm.value });
        apply(next);
      }
      setConfirm(null);
      setPendingNetworkStart(null);
    } catch (error) {
      if (confirm.kind === "network") setNetworkDialogError(String(error)); else notify(String(error));
      setConfirm(null);
      setPendingNetworkStart(null);
    }
    finally { setWorking(false); }
  };

  const startNetwork = async (request: NetworkStartRequest) => {
    setNetworkDialogError("");
    if (cleanupPending) { setNetworkDialogError("上一条连接仍有清理工作待完成；请先重试断开，再启动网络连接。"); return; }
    if (active) {
      setPendingNetworkStart(request);
      setConfirm({ kind: "network", value: "" });
      return;
    }
    setWorking(true);
    try {
      apply(await invokeApp<Snapshot>("start_network_audio", { ...request }));
      setPanel(null);
    } catch (error) { setNetworkDialogError(String(error)); }
    finally { setWorking(false); }
  };

  const setOutput = async (output: "computer" | "phone") => {
    if (!snapshot) return;
    if (snapshot.busy || cleanupPending) return;
    const selectedWifiAdb = selected?.transport === "wifi" && explicitWifiDeviceSerial === selected.serial;
    if (output === "phone" && !active && snapshot.transport === "wifi" && !selectedWifiAdb) {
      setNetworkDialogError(""); setPanel("network"); return;
    }
    if (output === "phone" && !active && !selected) {
      setPanel("help");
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
    setExplicitWifiDeviceSerial(device.transport === "wifi" ? device.serial : null);
    if (canDisconnect) {
      if (device.serial === snapshot.sessionSerial) return;
      setConfirm({ kind: "device", value: device.serial });
      return;
    }
    if (device.serial === localSelected) return;
    setLocalSelected(device.serial);
    void invokeApp<Snapshot>("select_device", { serial: device.serial }).then(apply).catch(error => notify(String(error)));
  };

  if (!snapshot) return <main className="loading-shell"><AudioLines size={25} /><span>正在启动 AnPod…</span></main>;
  const empty = devices.length === 0;
  const stateLabels: Record<Snapshot["state"], string> = {
    noDevice: empty ? "尚未连接手机" : "等待手机授权", unauthorized: "等待手机授权",
    ready: "准备就绪", connecting: "正在连接", connected: "已连接 · 电脑播放",
    streaming: "正在手机播放", phoneReady: lanSession ? "已认证 · 等待手机播放确认" : "手机接收已就绪",
    paused: "手机播放已暂停", reconnecting: "正在重新连接", error: "连接失败",
  };
  const currentDeviceLabel = lanSession ? bindingLabel(snapshot.sessionReceiverBinding) : snapshot.deviceName;
  const statusDetail = lanSession
    ? snapshot.state === "phoneReady" ? snapshot.output === "computer" ? "Receiver 已认证；声音由电脑输出。" : "Receiver 已认证；等待接收端确认手机正在播放。"
      : snapshot.state === "streaming" ? "Receiver 已确认音频正在手机端播放。"
        : snapshot.state === "paused" ? "Receiver 会话已暂停；手机当前没有播放声音。"
        : snapshot.state === "reconnecting" ? "直接网络连接正在恢复；请保持两端位于可互访网络。"
          : snapshot.state === "error" ? "直接网络连接出现错误；请查看诊断或重新连接。"
            : snapshot.output === "computer" ? "网络会话保持；声音由电脑输出。" : "电脑声音通过已认证的直接网络会话发送。"
    : snapshot.statusDetail;

  return <div className={`app-shell ${largeText ? "large-text" : ""}`}>
    <header className="topbar">
      <div className="brand"><img src="/icon.png" alt="" /><div><strong>AnPod</strong><span>电脑声音，手机播放</span></div></div>
      <button className="icon-button" aria-label="打开设置" title="设置" onClick={() => setPanel("settings")}><Settings2 size={18} /></button>
    </header>

    <main className="main-scroll">
      <div className={`status-line state-${snapshot.state}`}>
        <span className="status-dot" aria-hidden="true" />
        <span role="status" aria-live="polite">{stateLabels[snapshot.state]}</span>
        {currentDeviceLabel && <span className="status-device">{currentDeviceLabel}</span>}
        <span className="audio-bitrate" title={bitrateHelp}>
          <span>音频码率</span><span className="audio-bitrate-value">{formatBitrate(snapshot.audioBitrateBps)}</span>
        </span>
        <span className="transport-tag">{snapshot.sessionTransport === "lan" ? "UDP" : (snapshot.sessionTransport ?? snapshot.transport).toUpperCase()}</span>
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
        <p className="status-description">{statusDetail}</p>
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
        <div className="section-title device-heading"><h2>连接手机</h2><button className="text-button" disabled={working || snapshot.busy} onClick={() => { setNetworkDialogError(""); setPanel(snapshot.transport === "wifi" ? "network" : "help"); }}>{snapshot.transport === "wifi" ? "添加手机" : "连接帮助"}</button></div>
        <div className="device-card">
          <button className="device-select" onClick={() => setPanel("device")} disabled={working || snapshot.busy} aria-haspopup="dialog" aria-expanded={panel === "device"} aria-label={`选择手机，${active ? `当前连接${lanSession ? bindingLabel(snapshot.sessionReceiverBinding) : activeDevice?.displayName ?? snapshot.deviceName ?? "手机"}` : cardDevice ? `当前选择${cardDevice.displayName}` : "打开设备列表"}`}>
            <span className="device-icon"><Smartphone size={18} aria-hidden="true" /></span>
            <span className="device-copy">
              <strong>{active ? lanSession ? bindingLabel(snapshot.sessionReceiverBinding) : activeDevice?.displayName ?? snapshot.deviceName ?? "已连接手机" : cleanupPending ? snapshot.deviceName ?? "连接清理待完成" : selected?.displayName ?? (empty ? "未检测到手机" : "选择手机")}</strong>
              <span>{active ? `${transportLabel(snapshot.sessionTransport)}${snapshot.sessionAndroidVersion ? ` · Android ${snapshot.sessionAndroidVersion}` : ""}` : cleanupPending ? "当前连接信息不可用" : cardDevice ? `${transportLabel(cardDevice.transport)}${cardDevice.androidVersion ? ` · Android ${cardDevice.androidVersion}` : ""}` : snapshot.transport === "wifi" ? "直接网络接收；ADB 无线调试为过渡/诊断方式" : "连接 USB 手机后刷新设备列表"}</span>
              {(active || cleanupPending) && <span className="device-live-state" role="status">{cleanupPending ? "连接清理待重试" : stateLabels[snapshot.state]}</span>}
            </span>
            <ChevronDown size={18} aria-hidden="true" />
          </button>
          <button className="primary-button device-action" disabled={working || snapshot.busy || (!canDisconnect && !selected?.connectable)} onClick={() => canDisconnect ? void disconnect() : void connectOnly()}>
            {working || snapshot.busy ? <><LoaderCircle className="spin" size={15} />处理中</> : canDisconnect ? "断开连接" : snapshot.transport === "wifi" ? "ADB 诊断连接" : "连接"}
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

    {panel === "device" && <DeviceDialog devices={deviceChoices} selected={localSelected} activeSerial={snapshot.sessionSerial} activeBinding={lanSession ? snapshot.sessionReceiverBinding : null} busy={working || snapshot.busy} onSelect={chooseDevice} onRefresh={refresh} onClose={() => setPanel(null)} />}
    {panel === "settings" && <SettingsDialog snapshot={snapshot} largeText={largeText} onLargeText={setLargeText} onSave={async (key, value) => {
      try { const next = await invokeApp<Snapshot>("update_setting", { key, value }); apply(next); if (key === "preferredTransport") apply(await invokeApp<Snapshot>("refresh_devices")); }
      catch (error) { notify(String(error)); }
    }} onClose={() => setPanel(null)} />}
    {panel === "network" && <NetworkReceiverDialog usbDevices={snapshot.devices.filter(device => device.transport === "usb" && device.connectable)} disabled={working || snapshot.busy} audioSessionActive={active} error={networkDialogError} onError={setNetworkDialogError} onStart={startNetwork} onDisconnect={() => void disconnect()} onLegacy={() => setPanel("pair")} onClose={() => setPanel(null)} />}
    {panel === "pair" && <PairDialog transport="wifi" initialEndpoint={snapshot.settings.lastWirelessEndpoint ?? ""} disabled={snapshot.busy} onClose={() => setPanel("network")} onDone={async () => { apply(await invokeApp<Snapshot>("refresh_devices")); setPanel("device"); }} />}
    {panel === "help" && <HelpDialog transport={snapshot.transport} onClose={() => setPanel(null)} />}
    {panel === "diagnostics" && <DiagnosticsDialog snapshot={snapshot} onClose={() => setPanel(null)} />}
    {confirm && <ConfirmDialog
      title={confirm.kind === "network" ? "断开当前会话并连接？" : confirm.kind === "device" ? "切换连接手机？" : confirm.kind === "transport" ? "切换连接方式？" : "切换播放模式？"}
      body={confirm.kind === "network" ? "将先断开当前会话，再启动所选网络接收。声音会中断；只有 Receiver 完成双向认证后才会显示连接就绪。"
        : confirm.kind === "device" && lanSession ? `先断开当前直接网络会话，再连接 ${snapshot.devices.find(device => device.serial === confirm.value)?.displayName ?? "所选手机"}。声音会中断，原网络会话不会自动恢复。`
          : confirm.kind === "transport" && lanSession ? "先断开当前直接网络会话，再切换连接方式。不会自动重连同一部手机。"
            : confirm.kind === "device" ? `切换到 ${snapshot.devices.find(device => device.serial === confirm.value)?.displayName ?? "所选手机"} 会短暂中断当前连接；${snapshot.output === "phone" ? "完成后会继续从手机输出。" : "完成后声音仍从电脑输出。"}`
              : "切换播放模式时会短暂中断声音，AnPod 会重新配置当前连接。"}
      confirmLabel={confirm.kind === "network" ? "断开并启动" : confirm.kind === "device" ? "中断并切换" : confirm.kind === "transport" && lanSession ? "断开并切换" : confirm.kind === "transport" ? "断开并重新连接" : "重新配置"} busy={working} onCancel={() => { setConfirm(null); setPendingNetworkStart(null); }} onConfirm={() => void applyConfirmed()} />}
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

function NetworkReceiverDialog({ usbDevices, disabled, audioSessionActive, error, onError, onStart, onDisconnect, onLegacy, onClose }: {
  usbDevices: Device[]; disabled: boolean; audioSessionActive: boolean; error: string; onError: (error: string) => void;
  onStart: (request: NetworkStartRequest) => void; onDisconnect: () => void; onLegacy: () => void; onClose: () => void;
}) {
  const [method, setMethod] = useState<"bootstrap" | "paired">("bootstrap");
  const [interfaces, setInterfaces] = useState<PrivateIpv4Interface[]>([]);
  const [receiverInterfaces, setReceiverInterfaces] = useState<ReceiverIpv4Interface[]>([]);
  const [bindings, setBindings] = useState<string[]>([]);
  const [candidates, setCandidates] = useState<NetworkCandidate[]>([]);
  const [localAddress, setLocalAddress] = useState("");
  const [serial, setSerial] = useState("");
  const [receiverAddress, setReceiverAddress] = useState("");
  const [bindingReference, setBindingReference] = useState("");
  const [peer, setPeer] = useState("");
  const [loading, setLoading] = useState(false);
  const [receiverAction, setReceiverAction] = useState<"checking" | "installing" | null>(null);
  const [receiverStatus, setReceiverStatus] = useState<ReceiverStatus | null>(null);
  const [receiverError, setReceiverError] = useState("");
  const [localError, setLocalError] = useState("");
  const alertRef = useRef<HTMLParagraphElement>(null);
  const receiverGeneration = useRef(0);
  const busy = disabled || loading || receiverAction !== null;

  useEffect(() => {
    if (error || localError || receiverError) alertRef.current?.focus();
  }, [error, localError, receiverError]);

  const loadNetworkInfo = async () => {
    setLoading(true); setLocalError("");
    const [interfacesResult, bindingsResult] = await Promise.allSettled([
      invokeApp<PrivateIpv4Interface[]>("network_interfaces"),
      invokeApp<string[]>("receiver_binding_references"),
    ]);
    const errors: string[] = [];
    if (interfacesResult.status === "fulfilled") {
      setInterfaces(interfacesResult.value);
      if (!interfacesResult.value.some(value => value.address === localAddress)) setLocalAddress("");
      if (interfacesResult.value.length === 0) errors.push("没有可用的电脑 IPv4 网络接口。请启用 Wi-Fi 或有线网络后刷新。");
    } else errors.push(String(interfacesResult.reason));
    if (bindingsResult.status === "fulfilled") {
      setBindings(bindingsResult.value);
      if (!bindingsResult.value.includes(bindingReference)) setBindingReference("");
    } else errors.push(String(bindingsResult.reason));
    setLocalError(errors.join("；"));
    setLoading(false);
  };
  useEffect(() => { void loadNetworkInfo(); }, []);
  const readReceiverStatusAndInterfaces = async (deviceSerial: string, generation: number) => {
    const status = await invokeApp<ReceiverStatus>("receiver_status", { serial: deviceSerial });
    if (receiverGeneration.current !== generation) return;
    setReceiverStatus(status);
    setReceiverError("");
    if (!status.installed) {
      setReceiverInterfaces([]); setReceiverAddress(""); setLocalError("");
      return;
    }
    const values = await invokeApp<ReceiverIpv4Interface[]>("receiver_network_interfaces", { serial: deviceSerial });
    if (receiverGeneration.current !== generation) return;
    setReceiverInterfaces(values);
    if (!values.some(value => value.address === receiverAddress)) setReceiverAddress("");
    setLocalError(values.length === 0 ? "Receiver 已安装，但手机没有报告可用的 IPv4 地址；请检查手机网络并重试检查状态。" : "");
  };
  const checkReceiver = async (deviceSerial: string) => {
    const generation = ++receiverGeneration.current;
    setReceiverStatus(null); setReceiverError(""); setReceiverInterfaces([]); setReceiverAddress(""); setLocalError("");
    setReceiverAction("checking");
    try { await readReceiverStatusAndInterfaces(deviceSerial, generation); }
    catch (reason) { if (receiverGeneration.current === generation) setReceiverError(String(reason)); }
    finally { if (receiverGeneration.current === generation) setReceiverAction(null); }
  };
  const installReceiver = async () => {
    if (!serial || busy) return;
    if (audioSessionActive) { setReceiverError("请先断开音频会话，再安装或更新 Receiver。"); return; }
    const deviceSerial = serial;
    const generation = ++receiverGeneration.current;
    setReceiverAction("installing"); setReceiverError(""); setReceiverStatus(null); setReceiverInterfaces([]); setReceiverAddress(""); setLocalError("");
    try {
      await invokeApp<ReceiverStatus>("receiver_install", { serial: deviceSerial });
      if (receiverGeneration.current === generation) await readReceiverStatusAndInterfaces(deviceSerial, generation);
    } catch (reason) { if (receiverGeneration.current === generation) setReceiverError(String(reason)); }
    finally { if (receiverGeneration.current === generation) setReceiverAction(null); }
  };
  useEffect(() => {
    setReceiverInterfaces([]); setReceiverAddress("");
    setReceiverStatus(null); setReceiverError(""); setLocalError("");
    if (serial) void checkReceiver(serial);
    else setReceiverAction(null);
    return () => { receiverGeneration.current++; };
  }, [serial]);

  const discover = async () => {
    if (!localAddress) return;
    setLoading(true); setLocalError(""); setCandidates([]);
    try {
      const values = await invokeApp<NetworkCandidate[]>("discover_network_receivers", { localAddress });
      setCandidates(values);
      if (values.length === 0) setLocalError("没有发现接收端。确认两端位于可互访网络后，可手动输入接收端地址。");
    } catch (reason) { setLocalError(String(reason)); }
    finally { setLoading(false); }
  };
  const submit = (event: React.FormEvent) => {
    event.preventDefault(); setLocalError(""); onError("");
    if (!localAddress) { setLocalError("请明确选择电脑使用的网络接口。"); return; }
    if (method === "bootstrap") {
      if (!receiverStatus?.installed) { setLocalError("请先检查并安装/更新已核验的 Receiver，再选择手机网络接口。"); return; }
      const receiver = receiverInterfaces.find(value => value.address === receiverAddress);
      if (!serial || !receiver) { setLocalError("请选择已授权的 USB 手机及其当前 IPv4 地址。"); return; }
      onStart({ localAddress, peer: "", bootstrapSerial: serial, receiverAddress: receiver.address, receiverPrefixLength: receiver.prefixLength });
      return;
    }
    if (!bindingReference) { setLocalError("请选择已配对的接收端绑定。"); return; }
    if (!isIpv4Endpoint(peer)) { setLocalError("请输入有效的 IPv4 地址和端口，例如 192.0.2.20:45678。"); return; }
    onStart({ localAddress, peer: peer.trim(), bindingReference });
  };
  const shownError = error || localError;
  const canStart = method === "bootstrap"
    ? !!serial && receiverStatus?.installed === true && receiverInterfaces.some(value => value.address === receiverAddress)
    : !!bindingReference && isIpv4Endpoint(peer);

  return <Modal title="直接连接 Receiver" onClose={onClose} className="network-dialog">
    <form className="network-pane" onSubmit={submit}>
      <p className="network-intro">先安装已通过版本和签名核验的 Receiver。选择电脑网络接口，再通过 USB 首次绑定，或使用已配对接收端；发现结果只是地址提示，连接仍会进行双向认证。</p>
      <label className="network-field">电脑网络接口
        <select required value={localAddress} disabled={busy} onChange={event => { setLocalAddress(event.target.value); setCandidates([]); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !localAddress || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
          <option value="">请选择 IPv4 接口</option>
          {interfaces.map(value => <option key={`${value.index}-${value.address}`} value={value.address}>{value.name} · {value.address}/{value.prefixLength}</option>)}
        </select>
      </label>
      <button className="text-button network-refresh" type="button" onClick={() => void loadNetworkInfo()} disabled={busy}><Search size={15} />刷新接口和配对信息</button>
      <div className="network-methods" role="group" aria-label="Receiver 连接方式">
        <button type="button" aria-pressed={method === "bootstrap"} disabled={busy} onClick={() => { setMethod("bootstrap"); onError(""); setLocalError(""); }}>USB 首次绑定</button>
        <button type="button" aria-pressed={method === "paired"} disabled={busy} onClick={() => { setMethod("paired"); onError(""); setLocalError(""); }}>已配对接收端</button>
      </div>
      {method === "bootstrap" ? <>
        <label className="network-field">已授权的 USB 手机
          <select required value={serial} disabled={busy} onChange={event => { setSerial(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !serial || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
            <option value="">请选择 USB 设备</option>
            {usbDevices.map(device => <option key={device.serial} value={device.serial}>{device.displayName}</option>)}
          </select>
        </label>
        <section className="receiver-manager" aria-label="Receiver 安装管理">
          <div className="receiver-status-row">
            <div className="receiver-status-copy" role="status" aria-live="polite">
              <strong>{receiverAction === "checking" ? "正在检查 Receiver…" : receiverAction === "installing" ? "正在安装/更新 Receiver…" : !serial ? "请选择 USB 手机以检查 Receiver" : receiverError ? "Receiver 检查失败" : !receiverStatus ? "等待 Receiver 检查" : !receiverStatus.installed ? "Receiver 未安装" : receiverStatus.serviceActive ? receiverStatus.healthy ? "Receiver 已安装 · 服务正常" : "Receiver 已安装 · 服务异常" : "Receiver 已安装 · 服务未启动"}</strong>
              {receiverStatus?.installed && <small>{[receiverStatus.versionName, receiverStatus.versionCode === null ? "" : `版本码 ${receiverStatus.versionCode}`, receiverStatus.foreground ? "前台服务" : ""].filter(Boolean).join(" · ")}</small>}
              {receiverStatus?.audioActive && <small>Receiver 报告音频活动；安装状态、服务状态和手机实际播放分别显示。</small>}
              {receiverStatus && <small>{receiverStatus.message}</small>}
            </div>
            <div className="receiver-status-actions">
              <button className="dialog-secondary" type="button" disabled={busy || !serial} onClick={() => void checkReceiver(serial)}>{receiverAction === "checking" ? "检查中…" : "检查状态"}</button>
              <button className="primary-button" type="button" disabled={busy || !serial || audioSessionActive} onClick={() => void installReceiver()}>{receiverAction === "installing" ? "安装中…" : "安装/更新 Receiver"}</button>
            </div>
          </div>
          {receiverError && <p ref={alertRef} className="inline-error receiver-error" role="alert" tabIndex={-1}>{receiverError}</p>}
          {audioSessionActive && <div className="receiver-session-warning"><span>当前有音频会话；请先断开后才能安装/更新 Receiver。</span><button type="button" className="text-button" disabled={busy} onClick={onDisconnect}>断开音频会话</button></div>}
          <p className="network-note">将使用本便携包附带的 Receiver APK。手机系统可能要求确认安装或通知权限；签名不匹配时 Android 会拒绝覆盖并保留现有应用数据。</p>
        </section>
        <label className="network-field">手机当前 IPv4 地址
          <select required value={receiverAddress} disabled={busy || !serial} onChange={event => { setReceiverAddress(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !receiverAddress || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
            <option value="">请选择手机网络接口</option>
            {receiverInterfaces.map(value => <option key={`${value.interfaceIndex}-${value.address}`} value={value.address}>{value.interfaceName} · {value.address}/{value.prefixLength}</option>)}
          </select>
        </label>
        {serial && receiverStatus?.installed && <button className="text-button network-refresh" type="button" disabled={busy} onClick={() => void checkReceiver(serial)}><Search size={15} />检查状态并刷新手机接口</button>}
        <p className="network-note">USB 查询所选手机的实际网络地址，并用于首次绑定或重新启动 Receiver；不会猜测热点地址。Receiver 安装成功不等于服务已启动或手机正在播放。</p>
      </> : <>
        <label className="network-field">已配对接收端
          <select required value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !bindingReference || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
            <option value="">请选择配对绑定</option>
            {bindings.map(value => <option key={value} value={value}>{bindingLabel(value)}</option>)}
          </select>
        </label>
        {bindings.length === 0 && <p className="network-note">没有已保存的配对绑定。请先通过 USB 首次授权。</p>}
        <label className="network-field">接收端 IPv4 地址和控制端口
          <input required value={peer} disabled={busy} onChange={event => { setPeer(event.target.value); setLocalError(""); onError(""); }} placeholder="192.0.2.20:45678" autoComplete="off" inputMode="decimal" aria-invalid={!!shownError && !isIpv4Endpoint(peer) || undefined} aria-describedby={`network-peer-hint${shownError ? " network-flow-error" : ""}`} />
        </label>
        <small id="network-peer-hint" className="network-hint">也可以先发现候选地址。候选未经验证，不会自动绑定或配对。</small>
        <button className="text-button network-refresh" type="button" onClick={() => void discover()} disabled={busy || !localAddress}><Search size={15} />发现接收端候选</button>
        {candidates.length > 0 && <div className="candidate-list" aria-label="未验证的发现候选">{candidates.map(candidate => <button type="button" key={candidate.endpoint} disabled={busy} aria-pressed={peer === candidate.endpoint} onClick={() => { setPeer(candidate.endpoint); setLocalError(""); onError(""); }}><span>{candidate.endpoint}</span><small>未经验证 · 仅填入地址</small></button>)}</div>}
      </>}
      {shownError && <p id="network-flow-error" ref={alertRef} className="inline-error network-error" role="alert" tabIndex={-1}>{shownError}</p>}
      {busy && <p className="network-loading" role="status" aria-live="polite"><LoaderCircle className="spin" size={15} />正在读取网络信息…</p>}
      <div className="network-actions"><button type="button" className="dialog-secondary" onClick={onLegacy} disabled={busy}>无线调试（过渡/诊断）</button><button className="primary-button" disabled={busy || !canStart}>{disabled ? "正在处理…" : method === "bootstrap" ? "通过 USB 绑定并连接" : "连接已配对 Receiver"}</button></div>
    </form>
  </Modal>;
}

function DeviceDialog({ devices, selected, activeSerial, activeBinding, busy, onSelect, onRefresh, onClose }: { devices: Device[]; selected: string | null; activeSerial: string | null; activeBinding: string | null; busy: boolean; onSelect: (device: Device) => void; onRefresh: () => void; onClose: () => void }) {
  return <Modal title="选择手机" onClose={onClose} className="device-dialog">
    <div className="dialog-tools"><span>只显示当前连接方式的设备</span><button className="text-button" onClick={onRefresh} disabled={busy}><Search size={15} />刷新</button></div>
    {activeBinding && <div className="network-active-card"><strong>{bindingLabel(activeBinding)}</strong><span>当前连接 · 直接 UDP · 加密认证</span></div>}
    <div className="device-list">{devices.map(device => { const connected = activeSerial === device.serial; const chosen = (activeSerial ?? selected) === device.serial; return <button className="device-option" key={device.serial} onClick={() => onSelect(device)} disabled={busy || (!connected && !device.connectable)} aria-pressed={chosen}>
      <Smartphone size={18} aria-hidden="true" /><span><strong>{device.displayName}</strong><small>{connected ? "当前连接 · " : ""}{device.statusText}{device.transport !== "unknown" ? ` · ${device.transport === "wifi" ? "Wi-Fi · ADB TCP" : "USB · ADB"}` : " · 连接方式未知"}{device.androidVersion ? ` · Android ${device.androidVersion}` : ""}</small></span>{chosen && <Check size={17} aria-hidden="true" />}
    </button>; })}</div>
    {devices.length === 0 && <div className="empty-note"><Info size={17} /><span>{activeBinding ? "当前是直接网络 Receiver 会话。可添加另一台接收端；USB/ADB 设备扫描为空时不影响当前连接。" : "尚未发现设备。Wi-Fi 下可添加直接网络 Receiver；无线调试扫码、配对码或地址连接位于过渡/诊断入口。"}</span></div>}
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
  return <Modal title={transport === "wifi" ? "无线调试（过渡/诊断）" : "连接 USB 手机"} onClose={onClose} className="pair-dialog">
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
    {transport === "usb" ? <><h3>USB 与 Receiver</h3><ol><li>在手机开发者选项中开启 USB 调试，用支持数据的线连接，并允许此电脑进行 USB 调试。</li><li>在直接连接弹窗中选中 USB 手机后，可检查 Receiver 状态，并由你明确点击“安装/更新 Receiver”；系统可能要求确认安装或通知权限。</li><li>连接时明确选择电脑与手机正在使用的网络接口，两端需要位于可互访网络。</li></ol></> : <><h3>直接网络连接</h3><ol><li>主入口使用已安装并通过版本、签名核验的 Receiver，在可互访网络上进行加密认证。</li><li>首次绑定或 Receiver 需重新启动时，需要已授权 USB 连接；可在弹窗检查状态并明确点击“安装/更新 Receiver”。系统可能要求安装或通知权限确认。</li><li>无线调试扫码、配对码和地址连接保留为过渡/诊断方式。</li></ol></>}
    <p>请在手机上允许所需的 USB 调试和 Receiver 通知权限。如果 Receiver 被强行停止或手机重启，需要重新通过 USB 启动。连接就绪不代表手机已开始播放；播放状态会在 Receiver 确认后显示。</p><button className="dialog-secondary" onClick={onClose}>完成</button>
  </Modal>;
}

function DiagnosticsDialog({ snapshot, onClose }: { snapshot: Snapshot; onClose: () => void }) {
  const text = [`状态：${snapshot.message}`, `连接方式：${transportLabel(snapshot.sessionTransport ?? snapshot.transport)}`, `设备：${snapshot.sessionReceiverBinding ? bindingLabel(snapshot.sessionReceiverBinding) : snapshot.deviceName ?? "无"}`, `输出：${snapshot.output === "phone" ? "手机" : "电脑"}`, `可见设备：${snapshot.devices.length}`, `详细状态：${snapshot.statusDetail}`, `版本：${snapshot.version}`].join("\n");
  return <Modal title="诊断" onClose={onClose} className="diagnostics-dialog">
    <p>本次运行状态</p><pre>{text}</pre><div className="dialog-actions"><button className="dialog-secondary" onClick={() => void navigator.clipboard.writeText(text)}><Copy size={15} />复制诊断</button><button className="dialog-secondary" onClick={() => void invokeApp("open_logs")}>查看日志文件夹<ExternalLink size={14} /></button></div>
  </Modal>;
}

function ConfirmDialog({ title, body, confirmLabel, busy, onConfirm, onCancel }: { title: string; body: string; confirmLabel: string; busy: boolean; onConfirm: () => void; onCancel: () => void }) {
  return <Modal title={title} onClose={onCancel} className="confirm-dialog"><p>{body}</p><div className="dialog-actions"><button className="dialog-secondary" onClick={onCancel} disabled={busy}>取消</button><button className="primary-button" onClick={onConfirm} disabled={busy}>{busy ? "正在切换…" : confirmLabel}</button></div></Modal>;
}

export default App;
