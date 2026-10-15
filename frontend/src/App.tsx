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
  wirelessProtocol: "udp" | "adb"; audioBackend: "receiver" | "legacy";
  autoReconnectWifi: boolean; lastWirelessEndpoint: string | null;
  playbackMode: "Balanced" | "LowLatency"; appearanceTheme: "System" | "Light" | "Dark";
};
type Snapshot = {
  state: "noDevice" | "unauthorized" | "ready" | "connecting" | "connected" | "streaming" | "phoneReady" | "paused" | "reconnecting" | "managementOnly" | "error";
  message: string; deviceName: string | null; devices: Device[]; selectedSerial: string | null; sessionSerial: string | null;
  sessionReceiverBinding: string | null; sessionTransport: Device["transport"] | "lan" | null;
  sessionConnectionTransport: "usb" | "wireless" | null; sessionWirelessProtocol: "udp" | "adb" | null;
  sessionAndroidVersion: string | null; connectionTransport: "usb" | "wireless"; transport: "usb" | "wifi";
  sessionCapabilities: ReceiverCapabilities | null; wirelessConnectionOffers: HostDiscoverySnapshot;
  playbackDiagnostics: PlaybackDiagnostics | null;
  output: "computer" | "phone"; settings: Settings;
  busy: boolean; canDisconnect: boolean; canChangeModes: boolean; statusDetail: string;
  audioBitrateBps: number | null; version: string;
};
type Credentials = { instanceName: string; password: string; payload: string };
type MdnsResults = { available: boolean; services: { name: string; endpoint: string; kind: "pairing" | "connect" }[] };
type PrivateIpv4Interface = { index: number; name: string; address: string; prefixLength: number; gateways: string[] };
type ReceiverIpv4Interface = { address: string; prefixLength: number; interfaceIndex: number; interfaceName: string };
type NetworkInterfacePair = { local: PrivateIpv4Interface; receiver: ReceiverIpv4Interface };
type ReceiverStatus = {
  installed: boolean; versionName: string | null; versionCode: number | null; serviceActive: boolean;
  foreground: boolean; healthy: boolean; audioActive: boolean; error: string | null; message: string;
};
type NetworkCandidate = { endpoint: string };
type ReceiverCapabilities = {
  controlMajor: number; audioMajor: number; managementCompatible: boolean;
  audioCompatible: boolean; compatible: boolean; capabilityBits: number; versionCode: number;
};
type PlaybackDiagnostics = {
  sampleRate: number | null; channels: number | null;
  receiverSampleRate: number | null; receiverChannels: number | null;
  pcDroppedFrames: number | null; pcQueuedFrames: number | null;
  udpJitterTargetFrames: number | null; udpExpiredFrames: number | null; udpQueuedFrames: number | null;
  tcpDroppedBacklogFrames: number | null; trackPendingFrames: number | null;
  underrunCount: number | null; underrunIncrement: number | null; playbackAdvancing: boolean | null;
  requestedBufferFrames: number | null; acceptedBufferFrames: number | null; effectiveBufferFrames: number | null;
  capacityFrames: number | null; startThresholdFrames: number | null; preRollFrames: number | null;
};
type ReceiverPackageCandidate = { versionCode: number; versionName: string; sizeBytes: number; currentVersionCode: number | null; bindingReference: string | null };
type ReceiverUsbOperation = { kind: "install" | "upgrade"; serial: string; bindingReference?: string; targetVersionCode: number; returnPanel: "network" | "receiver" };
type ReceiverUsbOperationResult = { serial: string; kind: ReceiverUsbOperation["kind"]; targetVersionCode: number; status: ReceiverStatus };
type ConnectionOffer = { id: string; endpoint: string };
type HostDiscoverySnapshot = {
  active: boolean; automaticBinding?: string | null; interfaceAddress: string | null; interfacePrefixLength: number | null; offers: ConnectionOffer[];
};
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
const byTransport = (snapshot: Snapshot) => {
  const deviceTransport = snapshot.connectionTransport === "usb" ? "usb" : snapshot.settings.wirelessProtocol === "adb" ? "wifi" : null;
  return deviceTransport ? snapshot.devices.filter(device => device.transport === deviceTransport || device.transport === "unknown") : [];
};
const hasActiveSession = (snapshot: Snapshot | null) => !!snapshot && (!!snapshot.sessionSerial || !!snapshot.sessionReceiverBinding || snapshot.sessionConnectionTransport !== null);
const connectionLabel = (transport: Snapshot["connectionTransport"] | null, protocol: Snapshot["sessionWirelessProtocol"] | null) =>
  transport === "usb" ? "USB" : transport === "wireless" ? protocol === "udp" ? "无线 · UDP" : "无线 · ADB" : "连接方式未知";
const intendedConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.connectionTransport,
  snapshot.connectionTransport === "wireless" ? snapshot.settings.wirelessProtocol : null);
const actualConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.sessionConnectionTransport, snapshot.sessionWirelessProtocol);
const bindingLabel = (reference: string | null | undefined) => reference ? `已配对接收端 · ${reference.slice(0, 8)}` : "已配对接收端";
const CAP_PLAYBACK_PROGRESS = 1 << 3;
const isIpv4Endpoint = (value: string) => {
  const match = /^(\d{1,3}(?:\.\d{1,3}){3}):(\d{1,5})$/.exec(value.trim());
  return !!match && match[1].split(".").every(part => Number(part) <= 255) && Number(match[2]) > 0 && Number(match[2]) <= 65535;
};
const ipv4Number = (address: string) => {
  const parts = address.split(".");
  if (parts.length !== 4 || parts.some(part => !/^\d{1,3}$/.test(part) || Number(part) > 255)) return null;
  return parts.reduce((value, part) => ((value << 8) | Number(part)) >>> 0, 0);
};
const sameIpv4Subnet = (first: string, firstPrefix: number, second: string, secondPrefix: number) => {
  const a = ipv4Number(first); const b = ipv4Number(second);
  if (a === null || b === null || firstPrefix < 0 || firstPrefix > 32 || secondPrefix < 0 || secondPrefix > 32) return false;
  const prefix = Math.max(firstPrefix, secondPrefix);
  const mask = prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0;
  return (a & mask) >>> 0 === (b & mask) >>> 0;
};
const bootstrapNetworkPairs = (locals: PrivateIpv4Interface[], receivers: ReceiverIpv4Interface[]): NetworkInterfacePair[] =>
  locals.flatMap(local => receivers.filter(receiver => sameIpv4Subnet(local.address, local.prefixLength, receiver.address, receiver.prefixLength))
    .map(receiver => ({ local, receiver })));
const endpointFitsInterface = (endpoint: string, local: PrivateIpv4Interface | undefined) => {
  const address = endpoint.match(/^(\d{1,3}(?:\.\d{1,3}){3}):\d{1,5}$/)?.[1];
  return !!local && isIpv4Endpoint(endpoint) && !!address && sameIpv4Subnet(local.address, local.prefixLength, address, local.prefixLength);
};

function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [panel, setPanel] = useState<"device" | "settings" | "pair" | "network" | "receiver" | "help" | "diagnostics" | null>(null);
  const [confirm, setConfirm] = useState<{ kind: "transport" | "wirelessProtocol" | "audioBackend" | "playback" | "device" | "network" | "receiverUsb"; value: string } | null>(null);
  const [pendingNetworkStart, setPendingNetworkStart] = useState<NetworkStartRequest | null>(null);
  const [pendingReceiverOperation, setPendingReceiverOperation] = useState<ReceiverUsbOperation | null>(null);
  const [receiverOperationBusy, setReceiverOperationBusy] = useState(false);
  const [receiverManagerSerial, setReceiverManagerSerial] = useState("");
  const [receiverOperationResult, setReceiverOperationResult] = useState<ReceiverUsbOperationResult | null>(null);
  const [networkDialogError, setNetworkDialogError] = useState("");
  const [localSelected, setLocalSelected] = useState<string | null>(null);
  const [largeText, setLargeText] = useState(false);
  const [notice, setNotice] = useState("");
  const [working, setWorking] = useState(false);
  const snapshotRefreshGeneration = useRef(0);

  const apply = (value: Snapshot) => {
    snapshotRefreshGeneration.current++;
    setSnapshot(value);
    setLocalSelected(value.selectedSerial);
  };
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
    if (panel !== "network") return;
    let disposed = false;
    const refreshOffers = async () => {
      const generation = ++snapshotRefreshGeneration.current;
      try {
        const next = await invokeApp<Snapshot>("get_snapshot");
        if (!disposed && generation === snapshotRefreshGeneration.current) setSnapshot(next);
      } catch (error) {
        if (!disposed && generation === snapshotRefreshGeneration.current) setNetworkDialogError(String(error));
      }
    };
    void refreshOffers();
    const timer = window.setInterval(() => void refreshOffers(), 1000);
    return () => { disposed = true; window.clearInterval(timer); snapshotRefreshGeneration.current++; };
  }, [panel]);

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
  const lanSession = active && snapshot?.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "udp";
  const wirelessUdpMode = !!snapshot && snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp";
  const udpBackendFixed = !!snapshot && ((snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp")
    || (active && snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "udp"));
  const canDisconnect = !!snapshot?.canDisconnect;
  const cleanupPending = canDisconnect && !active && snapshot?.state === "error";
  const controlsDisabled = working || snapshot?.busy === true || cleanupPending || receiverOperationBusy;
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

  const setOption = async (kind: "transport" | "wirelessProtocol" | "audioBackend" | "playback", value: string) => {
    if (!snapshot) return;
    if (working || snapshot.busy || cleanupPending || receiverOperationBusy) return;
    if (kind === "audioBackend" && ((snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp")
      || (active && snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "udp"))) return;
    const current = kind === "transport" ? snapshot.connectionTransport
      : kind === "wirelessProtocol" ? snapshot.settings.wirelessProtocol
        : kind === "audioBackend" ? snapshot.settings.audioBackend
          : snapshot.settings.playbackMode === "LowLatency" ? "lowLatency" : "balanced";
    if (value === current) return;
    if (active) { setConfirm({ kind, value }); return; }
    setWorking(true);
    try {
      const key = kind === "playback" ? "playbackMode" : kind;
      const savedValue = kind === "playback" ? value === "lowLatency" ? "LowLatency" : "Balanced" : value;
      let next = await invokeApp<Snapshot>("update_setting", { key, value: savedValue });
      apply(next);
      let refreshFailure = "";
      if (kind === "transport" || kind === "wirelessProtocol") {
        try { next = await invokeApp<Snapshot>("refresh_devices"); apply(next); }
        catch (error) { refreshFailure = String(error); }
      }
      if (refreshFailure) notify(`选择已保存，但设备列表刷新失败：${refreshFailure}`); else setNotice("");
      const wirelessProtocol = kind === "wirelessProtocol" ? value : next.settings.wirelessProtocol;
      const connectionTransport = kind === "transport" ? value : next.connectionTransport;
      if (connectionTransport === "wireless" && wirelessProtocol === "udp" && (kind === "transport" || kind === "wirelessProtocol")) {
        setNetworkDialogError(""); setPanel("network");
      }
    } catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const performReceiverUsbOperation = async (operation: ReceiverUsbOperation) => {
    setWorking(true); setReceiverOperationBusy(true); setNetworkDialogError(""); setReceiverOperationResult(null);
    try {
      const command = operation.kind === "upgrade" ? "receiver_upgrade_usb" : "receiver_install";
      const args = operation.kind === "upgrade"
        ? { serial: operation.serial, bindingReference: operation.bindingReference }
        : { serial: operation.serial };
      await invokeApp<ReceiverStatus>(command, args);
      const status = await invokeApp<ReceiverStatus>("receiver_status", { serial: operation.serial });
      if (!status.installed || status.versionCode !== operation.targetVersionCode) {
        throw new Error(`已执行 USB 操作，但重新检查未确认目标版本码 ${operation.targetVersionCode}；请检查设备状态后重试。`);
      }
      setReceiverOperationResult({ serial: operation.serial, kind: operation.kind, targetVersionCode: operation.targetVersionCode, status });
      setReceiverManagerSerial(operation.serial);
      setPanel(operation.returnPanel);
      try { apply(await invokeApp<Snapshot>("refresh_devices")); }
      catch (error) { setNetworkDialogError(`Receiver 版本码 ${status.versionCode} 已核验；设备列表刷新失败：${String(error)}`); }
      notify(`Receiver 版本码 ${status.versionCode} 已核验；请重新认证连接。`);
    } catch (error) {
      setNetworkDialogError(String(error));
      setReceiverManagerSerial(operation.serial);
      setPanel(operation.returnPanel);
    } finally { setReceiverOperationBusy(false); setWorking(false); }
  };

  const requestReceiverUsbOperation = (operation: ReceiverUsbOperation) => {
    setReceiverOperationResult(null); setNetworkDialogError("");
    if (active && snapshot) {
      const targetMatches = snapshot.sessionTransport === "lan"
        ? operation.bindingReference === snapshot.sessionReceiverBinding
        : operation.serial === snapshot.sessionSerial;
      if (!targetMatches) {
        setNetworkDialogError("当前会话使用另一台设备；请先断开或选择当前会话对应的 USB 设备与绑定。");
        return;
      }
      setPendingReceiverOperation(operation);
      setPanel(null);
      setConfirm({ kind: "receiverUsb", value: "" });
      return;
    }
    void performReceiverUsbOperation(operation);
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
      } else if (confirm.kind === "receiverUsb") {
        if (!pendingReceiverOperation) throw new Error("USB 操作信息已失效；请重新检查 Receiver 状态。");
        const operation = pendingReceiverOperation;
        setConfirm(null); setPendingReceiverOperation(null);
        await performReceiverUsbOperation(operation);
      } else if (confirm.kind === "device") {
        const result = await invokeApp<OperationResult>("switch_device", { serial: confirm.value });
        apply(result.snapshot);
        if (!result.success) notify(result.message);
      } else {
        const next = await invokeApp<Snapshot>("reconfigure_session", { kind: confirm.kind, value: confirm.value });
        apply(next);
        if (!hasActiveSession(next) && next.connectionTransport === "wireless" && next.settings.wirelessProtocol === "udp"
          && (confirm.kind === "transport" || confirm.kind === "wirelessProtocol" || confirm.kind === "audioBackend")) {
          setNetworkDialogError(""); setPanel("network");
        }
      }
      setConfirm(null);
      setPendingNetworkStart(null);
    } catch (error) {
      if (confirm.kind === "network" || confirm.kind === "receiverUsb") {
        setNetworkDialogError(String(error));
        if (confirm.kind === "receiverUsb") setPanel(pendingReceiverOperation?.returnPanel ?? "receiver");
      }
      else {
        let message = String(error);
        if (confirm.kind === "transport" || confirm.kind === "wirelessProtocol" || confirm.kind === "audioBackend" || confirm.kind === "playback") {
          try { apply(await invokeApp<Snapshot>("get_snapshot")); }
          catch (refreshError) { message += `；状态刷新失败：${String(refreshError)}`; }
        }
        notify(message);
      }
      setConfirm(null);
      setPendingNetworkStart(null);
      if (confirm.kind === "receiverUsb") setPendingReceiverOperation(null);
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

  const setOfferListener = async (localAddress: string, allowed: boolean, bindingReference?: string) => {
    if (!snapshot || active || working || snapshot.busy) return;
    setWorking(true);
    setNetworkDialogError("");
    try {
      let selectedAddress = localAddress;
      let selectedBinding = bindingReference;
      if (allowed && (!selectedAddress || !selectedBinding)) {
        const [availableInterfaces, availableBindings] = await Promise.all([
          invokeApp<PrivateIpv4Interface[]>("network_interfaces"),
          invokeApp<string[]>("receiver_binding_references"),
        ]);
        if (!selectedAddress && availableInterfaces.length === 1) selectedAddress = availableInterfaces[0].address;
        if (!selectedBinding && availableBindings.length === 1) selectedBinding = availableBindings[0];
        if (!selectedAddress || !selectedBinding) { setPanel("network"); return; }
      }
      await invokeApp<HostDiscoverySnapshot>("set_wireless_connection_offers", {
        localAddress: selectedAddress, allowed, bindingReference: selectedBinding,
      });
      const generation = ++snapshotRefreshGeneration.current;
      const next = await invokeApp<Snapshot>("get_snapshot");
      if (generation === snapshotRefreshGeneration.current) apply(next);
    } catch (error) {
      if (panel === "network") setNetworkDialogError(String(error)); else notify(String(error));
    }
    finally { setWorking(false); }
  };

  const acceptOffer = async (offerId: string, bindingReference: string) => {
    if (!snapshot || active || working || snapshot.busy || !bindingReference) return;
    setWorking(true);
    setNetworkDialogError("");
    try {
      const next = await invokeApp<Snapshot>("accept_wireless_connection_offer", { offerId, bindingReference });
      apply(next);
      setPanel(null);
    } catch (error) { setNetworkDialogError(String(error)); }
    finally { setWorking(false); }
  };

  const setOutput = async (output: "computer" | "phone") => {
    if (!snapshot) return;
    if (working || snapshot.busy || cleanupPending || receiverOperationBusy) return;
    if (output === "phone" && snapshot.state === "managementOnly") return;
    if (output === "phone" && !active && snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp") {
      setNetworkDialogError(""); setPanel("network"); return;
    }
    if (output === "phone" && !active && !selected) {
      setPanel(snapshot.connectionTransport === "wireless" ? "pair" : "help");
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

  const runMainAction = () => {
    if (!snapshot) return;
    if (snapshot.wirelessConnectionOffers.active) {
      void setOfferListener(snapshot.wirelessConnectionOffers.interfaceAddress ?? "", false);
    } else if (snapshot.state === "connecting" || snapshot.state === "reconnecting") {
      void invokeApp<Snapshot>("cancel_operation").then(apply).catch(error => notify(String(error)));
    } else if (canDisconnect) {
      void disconnect();
    } else if (wirelessUdpMode) {
      void setOfferListener("", true);
    } else if (selected?.connectable) {
      void connectOnly();
    } else {
      setPanel(snapshot.connectionTransport === "usb" ? "help" : "pair");
    }
  };

  const connectOnly = async () => {
    if (!snapshot) return;
    if (snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp") {
      setNetworkDialogError(""); setPanel("network"); return;
    }
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
    setLocalSelected(device.serial);
    void invokeApp<Snapshot>("select_device", { serial: device.serial }).then(apply).catch(error => notify(String(error)));
  };

  if (!snapshot) return <main className="loading-shell"><AudioLines size={25} /><span>正在启动 AnPod…</span></main>;
  const empty = devices.length === 0;
  const stateLabels: Record<Snapshot["state"], string> = {
    noDevice: empty ? "尚未连接手机" : "等待手机授权", unauthorized: "等待手机授权",
    ready: "准备就绪", connecting: "正在连接", connected: "已连接 · 电脑播放",
    streaming: "正在手机播放", phoneReady: lanSession ? "已认证 · 等待手机播放确认" : "手机接收已就绪",
    paused: "手机播放已暂停", reconnecting: "正在重新连接", managementOnly: "Receiver 管理连接已认证", error: "连接失败",
  };
  const cancellingConnection = snapshot.state === "connecting" || snapshot.state === "reconnecting";
  const statusTitle = snapshot.state === "error" ? stateLabels.error : snapshot.wirelessConnectionOffers.active ? "等待手机连接" : stateLabels[snapshot.state];
  const statusDetail = lanSession
    ? snapshot.state === "managementOnly" ? "Receiver 管理连接已认证，但音频协议不兼容；声音继续由电脑输出。"
      : snapshot.state === "phoneReady" ? snapshot.output === "computer" ? "Receiver 已认证；声音由电脑输出。" : "Receiver 已认证；等待接收端确认手机正在播放。"
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
        <span role="status" aria-live="polite">{statusTitle}</span>
      </div>
      <button className="primary-button main-action" type="button" disabled={receiverOperationBusy || (!cancellingConnection && (working || snapshot.busy)) || !(snapshot.wirelessConnectionOffers.active || canDisconnect || wirelessUdpMode || selected?.connectable)} onClick={runMainAction}>
        {cancellingConnection ? "取消连接" : working || snapshot.busy ? "正在处理…" : snapshot.wirelessConnectionOffers.active ? "停止接收"
            : canDisconnect ? cleanupPending ? "重试断开" : "断开连接"
              : wirelessUdpMode ? "开始接收" : selected?.connectable ? "连接手机"
                : snapshot.connectionTransport === "usb" ? "连接 USB 手机" : "添加无线手机"}
      </button>

      <section className="playback-section" aria-label="声音输出">
        <div className="section-title"><h1>声音输出</h1></div>
        <div className="output-grid" role="group" aria-label="选择声音输出">
          <button className={`output-card ${snapshot.output === "computer" ? "selected" : ""}`} aria-pressed={snapshot.output === "computer"} onClick={() => void setOutput("computer")} disabled={controlsDisabled || (!active && snapshot.output === "computer")}>
            <span className="output-heading"><Computer size={18} aria-hidden="true" /><span className="output-name">电脑</span></span>
          </button>
          <button className={`output-card ${snapshot.output === "phone" ? "selected" : ""}`} aria-pressed={snapshot.output === "phone"} onClick={() => void setOutput("phone")} disabled={controlsDisabled || snapshot.state === "managementOnly"}>
            <span className="output-heading"><Smartphone size={18} aria-hidden="true" /><span className="output-name">手机</span></span>
          </button>
        </div>
        {snapshot.state === "error" && <p className="status-description" role="alert">{snapshot.statusDetail || snapshot.message || statusDetail}</p>}
      </section>

      <section className="connection-section" aria-label="连接手机">
        <div className="section-title"><h2>连接方式</h2></div>
        <div className="transport-switch" role="group" aria-label="连接方式">
          <button aria-pressed={snapshot.connectionTransport === "usb"} disabled={controlsDisabled || snapshot.wirelessConnectionOffers.active} onClick={() => void setOption("transport", "usb")}><Usb size={16} />USB</button>
          <button aria-pressed={snapshot.connectionTransport === "wireless"} disabled={controlsDisabled || snapshot.wirelessConnectionOffers.active} onClick={() => void setOption("transport", "wireless")}><Wifi size={16} />无线</button>
        </div>
        {snapshot.connectionTransport === "wireless" && <>
          <div className="protocol-switch" role="group" aria-label="无线协议">
            <button aria-pressed={snapshot.settings.wirelessProtocol === "udp"} disabled={controlsDisabled || snapshot.wirelessConnectionOffers.active} onClick={() => void setOption("wirelessProtocol", "udp")}>UDP <small>推荐</small></button>
            <button aria-pressed={snapshot.settings.wirelessProtocol === "adb"} disabled={controlsDisabled || snapshot.wirelessConnectionOffers.active} onClick={() => void setOption("wirelessProtocol", "adb")}>ADB <small>兼容</small></button>
          </div>
          {snapshot.settings.wirelessProtocol === "adb" && <p className="connection-hint">手机需开启无线调试并完成配对。</p>}
        </>}
        <div className="section-title device-heading"><h2>目标</h2>{snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "adb" && <button className="text-button" disabled={working || snapshot.busy || cleanupPending} onClick={() => setPanel("pair")}>添加手机</button>}</div>
        <div className="device-card">
          <button className="device-select" onClick={() => setPanel(wirelessUdpMode && !active ? "network" : "device")} disabled={controlsDisabled} aria-haspopup="dialog" aria-expanded={panel === "device" || panel === "network"} aria-label={`选择目标，${active ? `当前连接${lanSession ? bindingLabel(snapshot.sessionReceiverBinding) : activeDevice?.displayName ?? snapshot.deviceName ?? "手机"}` : cleanupPending ? "连接清理待完成" : wirelessUdpMode ? "选择已配对接收端" : cardDevice ? `当前选择${cardDevice.displayName}` : "打开设备列表"}`}>
            <span className="device-icon"><Smartphone size={18} aria-hidden="true" /></span>
            <span className="device-copy">
              <strong>{active ? lanSession ? bindingLabel(snapshot.sessionReceiverBinding) : activeDevice?.displayName ?? snapshot.deviceName ?? "已连接手机" : cleanupPending ? snapshot.deviceName ?? "连接清理待完成" : wirelessUdpMode ? snapshot.wirelessConnectionOffers.automaticBinding ? bindingLabel(snapshot.wirelessConnectionOffers.automaticBinding) : "选择接收手机" : selected?.displayName ?? (empty ? "未检测到手机" : "选择手机")}</strong>
              <span>{active ? `${actualConnectionLabel(snapshot)}${snapshot.sessionAndroidVersion ? ` · Android ${snapshot.sessionAndroidVersion}` : ""}` : cleanupPending ? "当前连接信息不可用" : wirelessUdpMode ? "无线 · UDP" : cardDevice ? `${cardDevice.transport === "wifi" ? "无线 · ADB" : "USB"}${cardDevice.androidVersion ? ` · Android ${cardDevice.androidVersion}` : ""}` : snapshot.connectionTransport === "wireless" ? "无线调试设备" : "USB 设备"}</span>
              {cleanupPending && <span className="device-live-state" role="status">连接清理待重试</span>}
            </span>
            <ChevronDown size={18} aria-hidden="true" />
          </button>
        </div>
      </section>

      <section className="mode-section" aria-label="播放模式">
        <div className="mode-row"><h2>播放模式</h2><div className="mode-switch" role="group" aria-label="播放模式">
          <button aria-pressed={snapshot.settings.playbackMode === "Balanced"} disabled={controlsDisabled} onClick={() => void setOption("playback", "balanced")}>均衡</button>
          <button aria-pressed={snapshot.settings.playbackMode === "LowLatency"} disabled={controlsDisabled} onClick={() => void setOption("playback", "lowLatency")}>低延迟</button>
        </div></div>
      </section>
    </main>

    <footer className="footer"><button onClick={() => setPanel("help")}><CircleHelp size={15} />连接帮助</button><span>v{snapshot.version}</span><button onClick={() => setPanel("diagnostics")}><Activity size={15} />诊断</button></footer>
    {notice && <div className="toast" role="status">{notice}</div>}

    {panel === "device" && <DeviceDialog devices={deviceChoices} selected={localSelected} activeSerial={snapshot.sessionSerial} activeBinding={lanSession ? snapshot.sessionReceiverBinding : null} busy={working || snapshot.busy} onSelect={chooseDevice} onRefresh={refresh} onClose={() => setPanel(null)} />}
    {panel === "settings" && <SettingsDialog snapshot={snapshot} largeText={largeText} onLargeText={setLargeText} udpBackendFixed={udpBackendFixed} busy={controlsDisabled} onOpenReceiverManager={() => { setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver"); }} onSave={async (key, value) => {
      try { const next = await invokeApp<Snapshot>("update_setting", { key, value }); apply(next); if (key === "preferredTransport") apply(await invokeApp<Snapshot>("refresh_devices")); }
      catch (error) { notify(String(error)); }
    }} onClose={() => setPanel(null)} />}
    {(panel === "network" || panel === "receiver") && <NetworkReceiverDialog key={panel} snapshot={snapshot} receiverManagerOnly={panel === "receiver"} initialReceiverSerial={receiverManagerSerial} usbDevices={snapshot.devices.filter(device => device.transport === "usb" && device.connectable)} disabled={working || snapshot.busy} receiverOperationBusy={receiverOperationBusy} receiverOperationResult={receiverOperationResult} audioSessionActive={active} udpMode={snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp"} offers={snapshot.wirelessConnectionOffers} error={networkDialogError} onError={setNetworkDialogError} onStart={startNetwork} onSetOfferListener={setOfferListener} onAcceptOffer={acceptOffer} onReceiverSerialChange={serial => { if (serial !== receiverManagerSerial) setReceiverOperationResult(null); setReceiverManagerSerial(serial); }} onRefreshDevices={refresh} onReceiverOperation={requestReceiverUsbOperation} onOpenReceiverManager={() => { setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver"); }} onClose={() => setPanel(null)} />}
    {panel === "pair" && <PairDialog transport="wifi" initialEndpoint={snapshot.settings.lastWirelessEndpoint ?? ""} disabled={working || snapshot.busy} onClose={() => setPanel(null)} onDone={async () => { apply(await invokeApp<Snapshot>("refresh_devices")); setPanel("device"); }} />}
    {panel === "help" && <HelpDialog transport={snapshot.connectionTransport} wirelessProtocol={snapshot.settings.wirelessProtocol} onClose={() => setPanel(null)} />}
    {panel === "diagnostics" && <DiagnosticsDialog snapshot={snapshot} onClose={() => setPanel(null)} />}
    {confirm && <ConfirmDialog
      title={confirm.kind === "receiverUsb" ? pendingReceiverOperation?.kind === "upgrade" ? "停止播放并升级 Receiver？" : "停止播放并安装 Receiver？" : confirm.kind === "network" ? "断开当前会话并连接？" : confirm.kind === "device" ? "切换连接手机？" : confirm.kind === "transport" ? "切换连接方式？" : confirm.kind === "wirelessProtocol" ? "切换无线协议？" : confirm.kind === "audioBackend" ? "切换接收后端？" : "切换播放模式？"}
      body={confirm.kind === "receiverUsb" ? `将停止当前播放并通过所选 USB 设备${pendingReceiverOperation ? `安装或核验目标版本码 ${pendingReceiverOperation.targetVersionCode}` : "执行 Receiver 操作"}。仅在设备身份、绑定、签名和版本预检通过后才会继续；完成后需要重新认证连接。`
        : confirm.kind === "network" ? "将先断开当前会话，再启动所选网络接收。声音会中断；只有 Receiver 完成双向认证后才会显示连接就绪。"
        : confirm.kind === "device" && lanSession ? `先断开当前直接网络会话，再连接 ${snapshot.devices.find(device => device.serial === confirm.value)?.displayName ?? "所选手机"}。声音会中断，原网络会话不会自动恢复。`
          : confirm.kind === "transport" || confirm.kind === "wirelessProtocol" ? "将短暂中断声音，再按所选连接方式重新配置；缺少接口、绑定或设备时会回到电脑输出并提示下一步。"
            : confirm.kind === "audioBackend" ? "将短暂中断声音并重新配置接收后端；未能重新连接时会回到电脑输出。"
            : confirm.kind === "device" ? `切换到 ${snapshot.devices.find(device => device.serial === confirm.value)?.displayName ?? "所选手机"} 会短暂中断当前连接；${snapshot.output === "phone" ? "完成后会继续从手机输出。" : "完成后声音仍从电脑输出。"}`
              : "切换播放模式时会短暂中断声音，AnPod 会重新配置当前连接。"}
      confirmLabel={confirm.kind === "receiverUsb" ? "停止播放并继续" : confirm.kind === "network" ? "断开并启动" : confirm.kind === "device" ? "中断并切换" : confirm.kind === "transport" || confirm.kind === "wirelessProtocol" || confirm.kind === "audioBackend" ? "确认切换" : "重新配置"} busy={working} onCancel={() => { if (confirm.kind === "receiverUsb") setPanel(pendingReceiverOperation?.returnPanel ?? "receiver"); setConfirm(null); setPendingNetworkStart(null); setPendingReceiverOperation(null); }} onConfirm={() => void applyConfirmed()} />}
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

function NetworkReceiverDialog({ snapshot, usbDevices, receiverManagerOnly, initialReceiverSerial, disabled, receiverOperationBusy, receiverOperationResult, audioSessionActive, udpMode, offers, error, onError, onStart, onSetOfferListener, onAcceptOffer, onReceiverSerialChange, onRefreshDevices, onReceiverOperation, onOpenReceiverManager, onClose }: {
  snapshot: Snapshot;
  receiverManagerOnly: boolean;
  initialReceiverSerial: string;
  usbDevices: Device[]; disabled: boolean; receiverOperationBusy: boolean; receiverOperationResult: ReceiverUsbOperationResult | null; audioSessionActive: boolean; udpMode: boolean; offers: HostDiscoverySnapshot; error: string; onError: (error: string) => void;
  onStart: (request: NetworkStartRequest) => void; onSetOfferListener: (localAddress: string, allowed: boolean, bindingReference?: string) => void;
  onAcceptOffer: (offerId: string, bindingReference: string) => void; onReceiverSerialChange: (serial: string) => void;
  onRefreshDevices: () => void; onReceiverOperation: (operation: ReceiverUsbOperation) => void; onOpenReceiverManager: () => void; onClose: () => void;
}) {
  const [method, setMethod] = useState<"bootstrap" | "paired">("bootstrap");
  const [advancedOpen, setAdvancedOpen] = useState(receiverManagerOnly);
  const [interfaces, setInterfaces] = useState<PrivateIpv4Interface[]>([]);
  const [receiverInterfaces, setReceiverInterfaces] = useState<ReceiverIpv4Interface[]>([]);
  const [bindings, setBindings] = useState<string[]>([]);
  const [candidates, setCandidates] = useState<NetworkCandidate[]>([]);
  const [localAddress, setLocalAddress] = useState("");
  const [serial, setSerial] = useState("");
  const [receiverAddress, setReceiverAddress] = useState("");
  const [bindingReference, setBindingReference] = useState("");
  const [selectedOfferId, setSelectedOfferId] = useState("");
  const [peer, setPeer] = useState("");
  const [loading, setLoading] = useState(false);
  const [receiverAction, setReceiverAction] = useState<"checking" | null>(null);
  const [receiverStatus, setReceiverStatus] = useState<ReceiverStatus | null>(null);
  const [receiverError, setReceiverError] = useState("");
  const [updateCandidate, setUpdateCandidate] = useState<ReceiverPackageCandidate | null>(null);
  const [updateCandidateError, setUpdateCandidateError] = useState("");
  const [candidateLoading, setCandidateLoading] = useState(false);
  const [localError, setLocalError] = useState("");
  const [addressNotice, setAddressNotice] = useState("");
  const alertRef = useRef<HTMLParagraphElement>(null);
  const receiverGeneration = useRef(0);
  const discoveryGeneration = useRef(0);
  const networkInfoGeneration = useRef(0);
  const methodManuallySelected = useRef(receiverManagerOnly);
  const peerManuallyEdited = useRef(false);
  const busy = disabled || loading || receiverAction !== null || receiverOperationBusy || candidateLoading;
  const receiverDeviceKey = usbDevices.map(device => device.serial).join("\u0000");
  const selectedReceiverDevice = usbDevices.find(device => device.serial === serial);
  const validBootstrapPairs = useMemo(() => bootstrapNetworkPairs(interfaces, receiverInterfaces), [interfaces, receiverInterfaces]);
  const compatibleReceiverInterfaces = useMemo(() => localAddress
    ? receiverInterfaces.filter(receiver => {
      const local = interfaces.find(value => value.address === localAddress);
      return !!local && sameIpv4Subnet(local.address, local.prefixLength, receiver.address, receiver.prefixLength);
    }) : [], [interfaces, localAddress, receiverInterfaces]);
  const selectedBootstrapPair = validBootstrapPairs.find(pair => pair.local.address === localAddress && pair.receiver.address === receiverAddress);

  useEffect(() => {
    if (error || localError || receiverError) alertRef.current?.focus();
  }, [error, localError, receiverError]);

  useEffect(() => {
    setSelectedOfferId(current => offers.offers.some(offer => offer.id === current)
      ? current : offers.offers.length === 1 ? offers.offers[0].id : "");
  }, [offers.offers]);

  const loadNetworkInfo = async () => {
    const generation = ++networkInfoGeneration.current;
    setLoading(true); setLocalError("");
    const [interfacesResult, bindingsResult] = await Promise.allSettled([
      receiverManagerOnly ? Promise.resolve([] as PrivateIpv4Interface[]) : invokeApp<PrivateIpv4Interface[]>("network_interfaces"),
      invokeApp<string[]>("receiver_binding_references"),
    ]);
    if (networkInfoGeneration.current !== generation) return;
    const errors: string[] = [];
    if (receiverManagerOnly) {
      setInterfaces([]); setLocalAddress("");
    } else if (interfacesResult.status === "fulfilled") {
      setInterfaces(interfacesResult.value);
      setLocalAddress(current => interfacesResult.value.some(value => value.address === current) ? current
        : interfacesResult.value.some(value => value.address === offers.interfaceAddress) ? offers.interfaceAddress ?? ""
          : interfacesResult.value.length === 1 ? interfacesResult.value[0].address : "");
      if (interfacesResult.value.length === 0) errors.push("没有可用的电脑 IPv4 网络接口。请启用 Wi-Fi 或有线网络后刷新。");
    } else errors.push(String(interfacesResult.reason));
    if (bindingsResult.status === "fulfilled") {
      const values = bindingsResult.value;
      setBindings(values);
      setBindingReference(current => values.includes(current) ? current
        : receiverManagerOnly && snapshot.sessionTransport === "lan" && snapshot.sessionReceiverBinding && values.includes(snapshot.sessionReceiverBinding)
          ? snapshot.sessionReceiverBinding
          : !receiverManagerOnly && values.length === 1 ? values[0] : "");
      if (!methodManuallySelected.current) setMethod(values.length > 0 ? "paired" : "bootstrap");
    } else errors.push(String(bindingsResult.reason));
    setLocalError(errors.join("；"));
    if (networkInfoGeneration.current === generation) setLoading(false);
  };
  useEffect(() => {
    void loadNetworkInfo();
    return () => { networkInfoGeneration.current++; };
  }, [receiverManagerOnly]);
  useEffect(() => {
    let disposed = false;
    setUpdateCandidate(null); setUpdateCandidateError("");
    setCandidateLoading(true);
    void invokeApp<ReceiverPackageCandidate | null>("receiver_update_candidate")
      .then(value => {
        if (disposed) return;
        setUpdateCandidate(value);
        if (value?.bindingReference && value.bindingReference === snapshot.sessionReceiverBinding && bindings.includes(value.bindingReference))
          setBindingReference(value.bindingReference);
      })
      .catch(reason => { if (!disposed) setUpdateCandidateError(String(reason)); })
      .finally(() => { if (!disposed) setCandidateLoading(false); });
    return () => { disposed = true; };
  }, [receiverManagerOnly, snapshot.sessionReceiverBinding, snapshot.sessionCapabilities?.managementCompatible, snapshot.sessionCapabilities?.versionCode, snapshot.sessionCapabilities?.capabilityBits]);
  useEffect(() => {
    const next = usbDevices.some(device => device.serial === serial) ? serial
      : receiverManagerOnly && usbDevices.some(device => device.serial === initialReceiverSerial) ? initialReceiverSerial
        : receiverManagerOnly && usbDevices.length === 1 ? usbDevices[0].serial
        : !receiverManagerOnly && usbDevices.length === 1 ? usbDevices[0].serial : "";
    setSerial(next);
    onReceiverSerialChange(next);
  }, [receiverDeviceKey, receiverManagerOnly, initialReceiverSerial]);
  const readReceiverStatusAndInterfaces = async (deviceSerial: string, generation: number) => {
    const status = await invokeApp<ReceiverStatus>("receiver_status", { serial: deviceSerial });
    if (receiverGeneration.current !== generation) return;
    setReceiverStatus(status);
    setReceiverError("");
    if (receiverManagerOnly) {
      setReceiverInterfaces([]); setReceiverAddress(""); setLocalError("");
      return;
    }
    if (!status.installed) {
      setReceiverInterfaces([]); setReceiverAddress(""); setLocalError("");
      return;
    }
    const values = await invokeApp<ReceiverIpv4Interface[]>("receiver_network_interfaces", { serial: deviceSerial });
    if (receiverGeneration.current !== generation) return;
    setReceiverInterfaces(values);
    setReceiverAddress(current => values.some(value => value.address === current) ? current : "");
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
  useEffect(() => {
    setReceiverInterfaces([]); setReceiverAddress("");
    setReceiverStatus(null); setReceiverError(""); setLocalError("");
    if (serial) void checkReceiver(serial);
    else setReceiverAction(null);
    return () => { receiverGeneration.current++; };
  }, [serial]);
  useEffect(() => {
    if (receiverOperationResult?.serial === serial) setReceiverStatus(receiverOperationResult.status);
  }, [receiverOperationResult, serial]);

  useEffect(() => {
    if (receiverManagerOnly) return;
    if (method === "paired") {
      if (!interfaces.some(value => value.address === localAddress)) setLocalAddress(interfaces.length === 1 ? interfaces[0].address : "");
      return;
    }
    if (receiverStatus?.installed !== true) return;
    if (interfaces.some(value => value.address === localAddress)) {
      const choices = validBootstrapPairs.filter(pair => pair.local.address === localAddress);
      if (choices.some(pair => pair.receiver.address === receiverAddress)) return;
      setReceiverAddress(choices.length === 1 ? choices[0].receiver.address : "");
      return;
    }
    if (validBootstrapPairs.length === 1) {
      setLocalAddress(validBootstrapPairs[0].local.address);
      setReceiverAddress(validBootstrapPairs[0].receiver.address);
    } else {
      setLocalAddress(""); setReceiverAddress("");
    }
  }, [interfaces, localAddress, method, receiverAddress, receiverStatus?.installed, validBootstrapPairs]);

  const discoverCandidates = async (automatic = false) => {
    const local = interfaces.find(value => value.address === localAddress);
    if (!local) return;
    const generation = ++discoveryGeneration.current;
    setLoading(true); setCandidates([]);
    if (!automatic) setLocalError("");
    setAddressNotice("正在搜索候选地址…");
    try {
      const values = await invokeApp<NetworkCandidate[]>("discover_network_receivers", { localAddress });
      if (discoveryGeneration.current !== generation) return;
      const usable = values.filter(value => endpointFitsInterface(value.endpoint, local));
      setCandidates(usable);
      if (!peerManuallyEdited.current) setPeer(usable.length === 1 ? usable[0].endpoint : "");
      setAddressNotice(usable.length === 1 ? "找到一个未验证的地址建议；点击连接后仍会进行双向认证。"
        : usable.length > 1 ? "找到多个未验证的地址建议，请在高级选项中明确选择。"
          : "未发现地址建议；可在高级选项中重新搜索或手动输入。");
      if (usable.length === 0 && !automatic) setLocalError("没有发现接收端。确认两端位于可互访网络后，可重试搜索或手动输入地址。");
    } catch (reason) { if (discoveryGeneration.current === generation) { setAddressNotice(""); setLocalError(String(reason)); } }
    finally { if (discoveryGeneration.current === generation) setLoading(false); }
  };
  useEffect(() => {
    if (method !== "paired" || !localAddress) return;
    setCandidates([]);
    if (!peerManuallyEdited.current) setPeer("");
    void discoverCandidates(true);
    return () => {
      discoveryGeneration.current++;
      setLoading(false);
    };
  }, [interfaces, localAddress, method]);
  const submit = (event: React.FormEvent) => {
    event.preventDefault(); setLocalError(""); onError("");
    if (receiverManagerOnly) return;
    if (!localAddress) { setLocalError("请明确选择电脑使用的网络接口。"); return; }
    if (method === "bootstrap") {
      if (!receiverStatus?.installed) { setLocalError("请先检查并安装/更新已核验的 Receiver，再选择手机网络接口。"); return; }
      const receiver = receiverInterfaces.find(value => value.address === receiverAddress);
      if (!serial || !receiver) { setLocalError("请选择已授权的 USB 手机及其当前 IPv4 地址。"); return; }
      onStart({ localAddress, peer: "", bootstrapSerial: serial, receiverAddress: receiver.address, receiverPrefixLength: receiver.prefixLength });
      return;
    }
    if (!bindingReference) { setLocalError("请选择已配对的接收端绑定。"); return; }
    const local = interfaces.find(value => value.address === localAddress);
    if (!isIpv4Endpoint(peer)) { setLocalError("请选择一个候选地址，或在高级选项中输入有效的 IPv4 地址和端口。"); return; }
    if (!endpointFitsInterface(peer, local)) { setLocalError("接收端地址不在所选电脑接口的本地网段内；请检查网络接口或地址。"); return; }
    onStart({ localAddress, peer: peer.trim(), bindingReference });
  };
  const shownError = error || localError;
  const selectedLocalInterface = interfaces.find(value => value.address === localAddress);
  const peerOutsideSubnet = method === "paired" && isIpv4Endpoint(peer) && !endpointFitsInterface(peer, selectedLocalInterface);
  const currentVersionCode = receiverStatus
    ? receiverStatus.installed
      ? receiverStatus.versionCode ?? (bindingReference && bindingReference === updateCandidate?.bindingReference ? updateCandidate.currentVersionCode : null)
      : null
    : bindingReference && bindingReference === updateCandidate?.bindingReference ? updateCandidate.currentVersionCode : null;
  const currentVersionLabel = receiverStatus?.installed === false ? "未安装"
    : currentVersionCode === null ? "待 USB 核验" : String(currentVersionCode);
  const packageVersion = updateCandidate?.versionCode ?? null;
  const receiverNeedsUpgrade = !!receiverStatus?.installed && currentVersionCode !== null && packageVersion !== null && currentVersionCode < packageVersion;
  const receiverAlreadyCurrent = currentVersionCode !== null && packageVersion !== null && currentVersionCode >= packageVersion;
  const receiverNeedsInstall = !!serial && receiverStatus?.installed === false;
  const canRequestUsbInstall = receiverNeedsInstall && !!updateCandidate && !receiverError;
  const canRequestUsbMaintenance = receiverNeedsUpgrade && snapshot.sessionTransport !== "lan" && !bindingReference && !!updateCandidate && !receiverError;
  const canRequestUsbUpgrade = receiverNeedsUpgrade && bindings.length > 0 && !!updateCandidate && !!bindingReference && !receiverError;
  const canStart = method === "bootstrap"
    ? !!serial && receiverStatus?.installed === true && !!selectedBootstrapPair
    : !!bindingReference && endpointFitsInterface(peer, selectedLocalInterface);
  const requestUsbOperation = (kind: ReceiverUsbOperation["kind"]) => {
    if (!serial || !updateCandidate) return;
    onReceiverOperation({
      kind, serial, ...(kind === "upgrade" ? { bindingReference } : {}),
      targetVersionCode: updateCandidate.versionCode, returnPanel: receiverManagerOnly ? "receiver" : "network",
    });
  };
  const pairedServiceStopped = method === "paired" && !!serial && receiverStatus?.installed === true && !receiverStatus.serviceActive;
  const mainSummary = receiverManagerOnly ? selectedReceiverDevice?.displayName ?? "选择已授权且在线的 USB 手机" : method === "paired"
    ? bindingReference ? bindingLabel(bindingReference) : "选择已配对接收端"
    : selectedReceiverDevice?.displayName ?? "选择 USB 手机";
  const addressSummary = receiverManagerOnly ? "仅查看 Receiver 状态，不要求电脑网络接口"
    : localAddress
    ? `${interfaces.find(value => value.address === localAddress)?.name ?? "电脑接口"} · ${localAddress}${method === "paired" && peer ? ` → ${peer}` : method === "bootstrap" && receiverAddress ? ` ↔ ${receiverAddress}` : ""}`
    : "网络接口待选择";
  const advancedRequired = method === "paired"
    ? bindings.length !== 1 || !bindingReference || interfaces.length !== 1 && !localAddress
    : usbDevices.length !== 1 || !serial || receiverStatus?.installed !== true || !selectedBootstrapPair;

  return <Modal title={receiverManagerOnly ? "检查或安装 Receiver" : "无线 Receiver"} onClose={onClose} className="network-dialog">
    <form className="network-pane" onSubmit={submit}>
      <div className="network-primary">
        <div className="network-summary"><strong>{mainSummary}</strong><span>{addressSummary}</span></div>
        {receiverManagerOnly && <section className="receiver-manager receiver-usb-card" aria-label="USB 检查或升级 Receiver">
          <div className="receiver-status-row">
            <div className="receiver-status-copy" role="status" aria-live="polite">
              <strong>{candidateLoading ? "正在检查本机 Receiver 安装包…" : updateCandidate ? "通过 USB 检查并升级 Receiver" : "本机 Receiver 安装包不可用"}</strong>
              {updateCandidate && <small>{updateCandidate.versionName} · 当前版本码 {currentVersionLabel} → 目标版本码 {updateCandidate.versionCode} · {(updateCandidate.sizeBytes / (1024 * 1024)).toFixed(1)} MiB</small>}
              {!updateCandidate && <small>{updateCandidateError || "连接已授权的 USB 手机后可检查版本。"}</small>}
              {receiverAlreadyCurrent && <small>当前版本已达到目标版本，无需升级。</small>}
              {receiverNeedsUpgrade && <small>{snapshot.sessionTransport === "lan"
                ? bindingReference ? "当前网络会话已选定绑定；升级还会核验 USB 设备身份。" : "当前存在网络会话，请选择与该会话对应的保存绑定后再升级。"
                : "Receiver 版本较旧；可用 USB 维护安装，或指定保存的绑定来验证升级目标。"}</small>}
              {receiverOperationResult && <small>USB 操作后已重新读取并核验 {receiverOperationResult.serial} 的版本码 {receiverOperationResult.status.versionCode}。</small>}
              {usbDevices.length === 0 && <small>连接 USB 手机、允许 USB 调试后刷新设备列表。</small>}
              {updateCandidateError && updateCandidate && <small className="inline-error">{updateCandidateError}</small>}
            </div>
            <div className="receiver-status-actions">
              {canRequestUsbInstall && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>安装 Receiver · {updateCandidate.versionCode}</button>}
              {canRequestUsbMaintenance && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>USB 维护安装 · {updateCandidate.versionCode}</button>}
              {canRequestUsbUpgrade && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("upgrade")}>升级 Receiver · {updateCandidate.versionCode}</button>}
              <button className="dialog-secondary" type="button" disabled={busy} onClick={onRefreshDevices}>刷新 USB 设备</button>
            </div>
          </div>
          {receiverNeedsUpgrade && bindings.length > 0 && <label className="network-field">可选：使用已保存绑定验证升级目标
            <select value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); setLocalError(""); onError(""); }}>
              <option value="">不指定绑定（USB 维护安装）</option>
              {bindings.map(value => <option key={value} value={value}>{bindingLabel(value)}</option>)}
            </select>
          </label>}
          {receiverNeedsUpgrade && !bindingReference && <small>未指定绑定；后台会先核验设备身份和版本，再执行 USB 维护安装。</small>}
        </section>}
        {pairedServiceStopped && <div className="network-recovery" role="status">
          <span>当前 USB 手机的 Receiver 服务未启动；可通过 USB 启动所选手机。</span>
          <button type="button" className="text-button" disabled={busy} onClick={() => { methodManuallySelected.current = true; setMethod("bootstrap"); setAdvancedOpen(true); }}>USB 启动所选手机</button>
        </div>}
        {shownError && <p id="network-flow-error" ref={alertRef} className="inline-error network-error" role="alert" tabIndex={-1}>{shownError}</p>}
        {busy && <p className="network-loading" role="status" aria-live="polite"><LoaderCircle className="spin" size={15} />{receiverOperationBusy ? "正在执行 USB 操作并核验结果…" : receiverAction === "checking" ? "正在检查 Receiver…" : candidateLoading ? "正在检查随包版本…" : "正在读取网络信息…"}</p>}
        {!receiverManagerOnly && udpMode && <section className="receiver-manager offer-manager" aria-label="允许手机搜索电脑">
          <div className="receiver-status-row">
            <div className="receiver-status-copy" role="status" aria-live="polite">
              <strong>{offers.active ? `手机连接监听中 · ${offers.interfaceAddress ?? "网络接口"}` : "允许已绑定手机连接"}</strong>
              <small>{offers.active
                ? offers.offers.length > 1 ? "收到多个请求，请选择目标后连接。" : offers.automaticBinding ? `手机点击“开始接收”后自动认证 · ${bindingLabel(offers.automaticBinding)}` : "自动认证绑定未启用；收到请求后仍需手动选择并接受。"
                : bindings.length === 0 ? "首次使用请通过 USB 完成绑定。" : "电脑监听后，手机可一键开始接收。"}</small>
            </div>
            {!offers.active && <button className="dialog-secondary" type="button" disabled={busy || audioSessionActive || !localAddress || !bindingReference} onClick={() => void onSetOfferListener(localAddress, true, bindingReference || undefined)}>开始监听</button>}
          </div>
          {offers.active && (!offers.automaticBinding || offers.offers.length > 1) && <>
            {offers.offers.length > 0
              ? <div className="candidate-list" aria-label="未认证的手机连接请求">
                  {offers.offers.map(offer => <button type="button" key={offer.id} disabled={busy || audioSessionActive} aria-pressed={selectedOfferId === offer.id} onClick={() => setSelectedOfferId(offer.id)}>
                    <span>{offer.endpoint}</span><small>{selectedOfferId === offer.id ? "已选择 · " : ""}未认证请求 · 收到后 30 秒内有效</small>
                  </button>)}
                </div>
              : <p className="network-hint">等待手机开始接收。</p>}
            <p className="network-hint">{bindings.length === 0 ? "尚无已保存绑定；请先完成手机配对。"
              : bindingReference ? `接受时将使用：${bindingLabel(bindingReference)}`
                : "有多个已保存绑定；请在高级选项中明确选择一个。"}</p>
            {offers.offers.length > 1 && !selectedOfferId && <p className="network-hint">请选择要接受的手机请求。</p>}
            <button className="primary-button" type="button" disabled={busy || audioSessionActive || !selectedOfferId || !bindingReference || !offers.offers.some(offer => offer.id === selectedOfferId)} onClick={() => onAcceptOffer(selectedOfferId, bindingReference)}>接受并连接所选手机</button>
            {audioSessionActive && <p className="network-hint">音频会话运行时，无法接受连接请求。</p>}
          </>}
        </section>}
      </div>
      <details className="network-advanced" open={receiverManagerOnly ? advancedOpen : advancedOpen || advancedRequired} onToggle={event => { if (receiverManagerOnly || !advancedRequired) setAdvancedOpen(event.currentTarget.open); }}>
        <summary>{receiverManagerOnly ? "Receiver 状态与安装" : "连接选项"}</summary>
        <div className="network-advanced-body">
          {!receiverManagerOnly && <p className="network-intro">候选地址仅作提示；连接仍使用已保存的绑定进行双向认证。</p>}
          {!receiverManagerOnly && <div className="network-methods" role="group" aria-label="Receiver 连接方式">
            <button type="button" aria-pressed={method === "bootstrap"} disabled={busy} onClick={() => { methodManuallySelected.current = true; setMethod("bootstrap"); onError(""); setLocalError(""); }}>USB 首次绑定</button>
            <button type="button" aria-pressed={method === "paired"} disabled={busy || bindings.length === 0} onClick={() => { methodManuallySelected.current = true; setMethod("paired"); onError(""); setLocalError(""); }}>连接已配对手机</button>
          </div>}
          {!receiverManagerOnly && <label className="network-field">电脑网络接口
            <select required value={localAddress} disabled={busy || offers.active} onChange={event => { setLocalAddress(event.target.value); setCandidates([]); if (!peerManuallyEdited.current) setPeer(""); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !localAddress || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
              <option value="">请选择 IPv4 接口</option>
              {interfaces.map(value => <option key={`${value.index}-${value.address}`} value={value.address}>{value.name} · {value.address}/{value.prefixLength}</option>)}
            </select>
          </label>}
          {method === "bootstrap" ? <>
            <label className="network-field">已授权且在线的 USB 手机
              <select required value={serial} disabled={busy} onChange={event => { setSerial(event.target.value); onReceiverSerialChange(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !serial || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
                <option value="">请选择 USB 手机</option>
                {usbDevices.map(device => <option key={device.serial} value={device.serial}>{device.displayName}</option>)}
              </select>
            </label>
            {receiverManagerOnly && usbDevices.length === 0 && <div className="network-recovery" role="status"><span>连接 USB 手机并允许 USB 调试后，才能检查或升级。</span><button type="button" className="text-button" disabled={busy} onClick={onRefreshDevices}>刷新设备</button></div>}
            {serial && <section className="receiver-manager" aria-label="Receiver 安装管理">
              <div className="receiver-status-row">
                <div className="receiver-status-copy" role="status" aria-live="polite">
                  <strong>{receiverOperationBusy ? "正在安装并核验 Receiver…" : receiverAction === "checking" ? "正在检查 Receiver…" : receiverError ? "Receiver 检查失败" : !receiverStatus ? "等待 Receiver 检查" : !receiverStatus.installed ? "Receiver 未安装" : receiverStatus.serviceActive ? receiverStatus.healthy ? "Receiver 已安装 · 服务正常" : "Receiver 已安装 · 服务异常" : "Receiver 已安装 · 服务未启动"}</strong>
                  {receiverStatus?.installed && <small>{[receiverStatus.versionName, receiverStatus.versionCode === null ? "" : `版本码 ${receiverStatus.versionCode}`, receiverStatus.foreground ? "前台服务" : ""].filter(Boolean).join(" · ")}</small>}
                  {receiverStatus?.audioActive && <small>Receiver 报告音频活动；这不代表手机正在播放。</small>}
                  {receiverStatus && !receiverStatus.healthy && <small>{receiverStatus.error || receiverStatus.message}</small>}
                  {!receiverManagerOnly && receiverNeedsUpgrade && <small>此版本较旧；兼容的 Receiver 仍可连接，也可选用 USB 管理升级。</small>}
                </div>
                <div className="receiver-status-actions">
                  <button className="dialog-secondary" type="button" disabled={busy} onClick={() => void checkReceiver(serial)}>{receiverAction === "checking" ? "检查中…" : "检查状态"}</button>
                  {!receiverManagerOnly && canRequestUsbInstall && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>安装 Receiver · {updateCandidate.versionCode}</button>}
                  {!receiverManagerOnly && receiverNeedsUpgrade && <button className="primary-button" type="button" disabled={busy} onClick={onOpenReceiverManager}>USB 升级选项</button>}
                </div>
              </div>
              {receiverError && <p ref={alertRef} className="inline-error receiver-error" role="alert" tabIndex={-1}>{receiverError}</p>}
              {!receiverManagerOnly && receiverNeedsInstall && <p className="network-note">使用 AnPod 随包 Receiver；设备身份、签名和版本会先核验。</p>}
              {!receiverManagerOnly && receiverNeedsUpgrade && <button type="button" className="text-button" disabled={busy} onClick={onOpenReceiverManager}>打开 USB 升级管理</button>}
            </section>}
            {!receiverManagerOnly && <label className="network-field">手机当前 IPv4 地址
              <select required value={receiverAddress} disabled={busy || !serial || receiverStatus?.installed !== true} onChange={event => { setReceiverAddress(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !receiverAddress || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
                <option value="">请选择手机网络接口</option>
                {compatibleReceiverInterfaces.map(value => <option key={`${value.interfaceIndex}-${value.address}`} value={value.address}>{value.interfaceName} · {value.address}/{value.prefixLength}</option>)}
              </select>
            </label>}
            {!receiverManagerOnly && <p className="network-note">只显示与所选电脑接口处于同一网段的手机地址。</p>}
          </> : <>
            <label className="network-field">已配对接收端
              <select required value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !bindingReference || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
                <option value="">请选择配对绑定</option>
                {bindings.map(value => <option key={value} value={value}>{bindingLabel(value)}</option>)}
              </select>
            </label>
            <label className="network-field">接收端 IPv4 地址和控制端口
              <input required value={peer} disabled={busy} onChange={event => { peerManuallyEdited.current = true; setPeer(event.target.value); setLocalError(""); onError(""); }} placeholder="192.0.2.20:45678" autoComplete="off" inputMode="decimal" aria-invalid={!!shownError && !isIpv4Endpoint(peer) || peerOutsideSubnet || undefined} aria-describedby={`network-peer-hint${peerOutsideSubnet ? " network-peer-network-warning" : ""}${shownError ? " network-flow-error" : ""}`} />
            </label>
            <small id="network-peer-hint" className="network-hint">地址建议未经验证；选择或填写后仍需点击连接。</small>
            {peerOutsideSubnet && <small id="network-peer-network-warning" className="network-validation-warning">此地址不在所选电脑接口的网段内，连接按钮不可用。</small>}
            <p className="network-hint" role="status" aria-live="polite">{addressNotice}</p>
            <button className="text-button network-refresh" type="button" onClick={() => void discoverCandidates()} disabled={busy || !localAddress}><Search size={15} />重新搜索地址</button>
            {candidates.length > 0 && <div className="candidate-list" aria-label="未验证的发现候选">{candidates.map(candidate => <button type="button" key={candidate.endpoint} disabled={busy} aria-pressed={peer === candidate.endpoint} onClick={() => { peerManuallyEdited.current = true; setPeer(candidate.endpoint); setLocalError(""); onError(""); }}><span>{candidate.endpoint}</span><small>未经验证 · 仅填入地址</small></button>)}</div>}
          </>}
          {!receiverManagerOnly && <button className="text-button network-refresh" type="button" onClick={() => void loadNetworkInfo()} disabled={busy}><Search size={15} />刷新接口和配对信息</button>}
          {!receiverManagerOnly && <button className="primary-button network-connect" disabled={busy || !canStart}>{disabled ? "正在处理…" : method === "bootstrap" ? "首次绑定并连接" : "直接连接 Receiver"}</button>}
        </div>
      </details>
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

function SettingsDialog({ snapshot, largeText, onLargeText, udpBackendFixed, busy, onOpenReceiverManager, onSave, onClose }: { snapshot: Snapshot; largeText: boolean; onLargeText: (value: boolean) => void; udpBackendFixed: boolean; busy: boolean; onOpenReceiverManager: () => void; onSave: (key: string, value: unknown) => void; onClose: () => void }) {
  const settings = snapshot.settings;
  const update = (key: keyof Settings, value: boolean | string) => { void onSave(key, value); };
  return <Modal title="设置" onClose={onClose} className="settings-dialog">
    <h3>连接</h3>
    <SettingToggle disabled={snapshot.busy} title="自动连接上次使用的手机" description="设备重新出现时尝试连接" checked={settings.autoConnectLastDevice} onChange={value => update("autoConnectLastDevice", value)} />
    <SettingToggle disabled={snapshot.busy} title="无线连接恢复" description="无线掉线后进行有限次数的重连" checked={settings.autoReconnectWifi} onChange={value => update("autoReconnectWifi", value)} />
    <h3>接收端</h3>
    <label className="select-setting"><span>接收后端</span><select aria-label="接收后端" disabled={busy || udpBackendFixed} value={settings.audioBackend} onChange={event => update("audioBackend", event.target.value)}><option value="legacy">Legacy（兼容）</option><option value="receiver">Receiver</option></select></label>
    {udpBackendFixed && <p className="dialog-footnote">无线 UDP 使用 Receiver。</p>}
    <button className="text-button backend-manager-button" type="button" disabled={busy} onClick={onOpenReceiverManager}>检查或安装 Receiver</button>
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
  return <Modal title={transport === "wifi" ? "无线 ADB 兼容连接" : "连接 USB 手机"} onClose={onClose} className="pair-dialog">
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

function HelpDialog({ transport, wirelessProtocol, onClose }: { transport: Snapshot["connectionTransport"]; wirelessProtocol: Snapshot["sessionWirelessProtocol"]; onClose: () => void }) {
  return <Modal title="连接帮助" onClose={onClose} className="help-dialog">
    {transport === "usb" ? <><h3>USB 连接</h3><ol><li>在手机开发者选项中开启 USB 调试，用支持数据的线连接，并允许此电脑进行 USB 调试。</li><li>接收后端可在主界面的二级菜单中选择 Legacy 或已安装 Receiver。</li></ol></>
      : wirelessProtocol === "udp" ? <><h3>无线 UDP 直连</h3><ol><li>使用已配对接收端；首次绑定时需通过已授权 USB 手机检查 Receiver。</li><li>明确选择电脑网络接口和接收端地址后，再点击连接。发现结果只是地址建议，不代表认证连接。</li><li>仅在你明确点击“安装/更新 Receiver”后，程序才会请求安装。</li></ol></>
        : <><h3>无线 ADB 兼容连接</h3><ol><li>先在手机的无线调试页面启用并配对设备，再从添加手机入口选择扫码、配对码或手动连接地址。</li><li>配对/连接成功后，再从设备卡片明确连接；不会因搜索到设备就开始播放。</li></ol></>}
    <p>{transport === "usb" ? "USB 操作需要在手机上允许此电脑进行调试。使用 Receiver 时，连接就绪不代表手机已开始播放；播放状态会在 Receiver 确认后显示。"
      : wirelessProtocol === "udp" ? "Receiver 安装或通知权限可能需要 Android 系统确认。若 Receiver 被强行停止或手机重启，需要通过 USB 重新启动；连接就绪不代表手机已开始播放。"
        : "无线调试配对需要在手机系统中明确授权。连接就绪不代表手机已开始播放；播放状态由当前音频后端报告。"}</p><button className="dialog-secondary" onClick={onClose}>完成</button>
  </Modal>;
}

function DiagnosticsDialog({ snapshot, onClose }: { snapshot: Snapshot; onClose: () => void }) {
  const capabilities = snapshot.sessionCapabilities;
  const diagnostic = snapshot.playbackDiagnostics;
  const active = hasActiveSession(snapshot);
  const udpSession = active && snapshot.sessionTransport === "lan"
    && snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "udp";
  const unsupportedUdpDiagnostics = udpSession
    && (capabilities === null || (capabilities.capabilityBits & CAP_PLAYBACK_PROGRESS) === 0);
  const playbackContext = !active ? "当前无手机会话。"
    : snapshot.output === "computer" ? "声音当前输出到电脑；以下仅为最近快照，不代表手机此刻正在播放。"
      : snapshot.state === "paused" ? "手机播放已暂停；以下仅为最近快照。"
        : snapshot.state === "streaming" && diagnostic?.playbackAdvancing === true ? "最近快照报告手机播放正在推进。"
          : "以下为最近取得的诊断快照，不代表手机此刻正在播放。";
  const unavailable = !active ? "当前没有手机会话，尚无播放诊断快照。"
    : unsupportedUdpDiagnostics ? "此 Receiver 未提供播放诊断能力（可能为旧版本或尚未协商）；连接与播放不受影响。"
      : "当前没有可用的播放诊断快照；数据尚未取得或 Receiver 未提供。";
  const number = (value: number | null) => value === null ? "未提供" : value.toLocaleString("zh-CN");
  const frames = (value: number | null, meaning: string) => value === null ? "未提供" : `${number(value)} 帧 · ${meaning}`;
  const groups: { title: string; note?: string; metrics: [string, string][] }[] = diagnostic ? [
    { title: "实际音频格式", metrics: [
      ["PC 采样率", diagnostic.sampleRate === null ? "未提供" : `${number(diagnostic.sampleRate)} Hz`],
      ["PC 声道数", number(diagnostic.channels)],
      ["Receiver 采样率", diagnostic.receiverSampleRate === null ? "未提供" : `${number(diagnostic.receiverSampleRate)} Hz`],
      ["Receiver 声道数", number(diagnostic.receiverChannels)],
    ] },
    { title: "发送与接收队列", note: "队列和目标以当前帧数显示；丢弃、过期与积压为本次会话累计。", metrics: [
      ["PC 当前队列", frames(diagnostic.pcQueuedFrames, "当前队列")],
      ["PC 丢弃", frames(diagnostic.pcDroppedFrames, "会话累计")],
      ["UDP 抖动目标", frames(diagnostic.udpJitterTargetFrames, "目标队列")],
      ["UDP 当前队列", frames(diagnostic.udpQueuedFrames, "当前队列")],
      ["UDP 过期", frames(diagnostic.udpExpiredFrames, "会话累计")],
      ["TCP 丢弃积压", frames(diagnostic.tcpDroppedBacklogFrames, "会话累计")],
    ] },
    { title: "Receiver 播放状态", note: "待播帧为本次查询；欠载增量是与上次查询的差值，首次查询或计数重置时未提供。", metrics: [
      ["AudioTrack 待播", frames(diagnostic.trackPendingFrames, "本次查询")],
      ["欠载总数", diagnostic.underrunCount === null ? "未提供" : `${number(diagnostic.underrunCount)} · AudioTrack 累计`],
      ["欠载增量", diagnostic.underrunIncrement === null ? "未提供" : `${number(diagnostic.underrunIncrement)} · 自上次查询`],
      ["播放推进", diagnostic.playbackAdvancing === null ? "未提供" : diagnostic.playbackAdvancing ? "是" : "否"],
    ] },
    { title: "AudioTrack 缓冲", note: "均为 AudioTrack 报告的帧数；不等同端到端延迟。", metrics: [
      ["请求", frames(diagnostic.requestedBufferFrames, "AudioTrack")],
      ["系统接受", frames(diagnostic.acceptedBufferFrames, "AudioTrack")],
      ["当前有效", frames(diagnostic.effectiveBufferFrames, "AudioTrack")],
      ["容量", frames(diagnostic.capacityFrames, "AudioTrack")],
      ["启动阈值", frames(diagnostic.startThresholdFrames, "AudioTrack")],
      ["预填充", frames(diagnostic.preRollFrames, "AudioTrack")],
    ] },
  ] : [];
  const statusText = [
    `状态：${snapshot.message}`, `本次连接：${active ? actualConnectionLabel(snapshot) : "未连接"}`,
    `当前选择：${intendedConnectionLabel(snapshot)}`, `接收后端选择：${snapshot.settings.audioBackend}`,
    `设备：${snapshot.sessionReceiverBinding ? bindingLabel(snapshot.sessionReceiverBinding) : snapshot.deviceName ?? "无"}`,
    `输出：${snapshot.output === "phone" ? "手机" : "电脑"}`, `可见设备：${snapshot.devices.length}`,
    `详细状态：${snapshot.statusDetail}`,
    `认证能力：${capabilities ? `管理 ${capabilities.controlMajor}（兼容 ${capabilities.managementCompatible}），音频 ${capabilities.audioMajor}（兼容 ${capabilities.audioCompatible}），位标志 0x${capabilities.capabilityBits.toString(16)}，版本码 ${capabilities.versionCode}` : "无"}`,
    `版本：${snapshot.version}`,
  ].join("\n");
  const text = [statusText, `播放诊断：${playbackContext}`,
    ...groups.flatMap(group => [group.title, ...group.metrics.map(([label, value]) => `${label}：${value}`), ...(group.note ? [group.note] : [])]),
  ].join("\n");
  const metricGroup = (group: typeof groups[number]) => <section className="diagnostic-group" key={group.title}>
    <h3>{group.title}</h3>
    <dl>{group.metrics.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
    {group.note && <p>{group.note}</p>}
  </section>;
  return <Modal title="诊断" onClose={onClose} className="diagnostics-dialog">
    <p>本次运行状态</p><pre>{statusText}</pre>
    <section className="diagnostic-playback" aria-label="播放诊断">
      <h3>播放诊断</h3><p className="diagnostic-context" role="status">{playbackContext}</p>
      {diagnostic ? groups.map(metricGroup) : <p className="diagnostic-empty">{unavailable}</p>}
    </section>
    <div className="dialog-actions"><button className="dialog-secondary" onClick={() => void navigator.clipboard.writeText(text)}><Copy size={15} />复制诊断</button><button className="dialog-secondary" onClick={() => void invokeApp("open_logs")}>查看日志文件夹<ExternalLink size={14} /></button></div>
  </Modal>;
}

function ConfirmDialog({ title, body, confirmLabel, busy, onConfirm, onCancel }: { title: string; body: string; confirmLabel: string; busy: boolean; onConfirm: () => void; onCancel: () => void }) {
  return <Modal title={title} onClose={() => { if (!busy) onCancel(); }} className="confirm-dialog"><p>{body}</p><div className="dialog-actions"><button className="dialog-secondary" onClick={onCancel} disabled={busy}>取消</button><button className="primary-button" onClick={onConfirm} disabled={busy}>{busy ? "正在切换…" : confirmLabel}</button></div></Modal>;
}

export default App;
