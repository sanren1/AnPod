import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { QRCodeSVG } from "qrcode.react";
import {
  Activity, AudioLines, Check, ChevronDown, CircleHelp, Copy,
  ExternalLink, Info, LoaderCircle, Search, Settings2,
  Smartphone, Usb, Wifi, X,
} from "lucide-react";
import type { ReactNode } from "react";
import { zhCN } from "./strings.zh-CN";

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
  audioBitrateBps: number | null; networkRecoverySecondsRemaining?: number | null; version: string;
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
type BindingResetPreview = { serial: string; androidUser: string; bindingReference: string; phoneState: string; phoneCommitment: string | null; pcState: string; pcCommitment: string | null; receiverBusy: boolean };
type BindingResetResult = Omit<BindingResetPreview, "receiverBusy"> & { outcome: string; reason: string; nextAction: string };
type ReceiverUsbOperation = { kind: "install" | "upgrade"; serial: string; bindingReference?: string; targetVersionCode: number; returnPanel: "network" | "receiver" | "setup" };
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
  if (!isDesktop) return Promise.reject(new Error(zhCN.app.desktopOnly));
  return invoke<T>(command, args);
};
const byTransport = (snapshot: Snapshot) => {
  const deviceTransport = snapshot.connectionTransport === "usb" ? "usb" : snapshot.settings.wirelessProtocol === "adb" ? "wifi" : null;
  return deviceTransport ? snapshot.devices.filter(device => device.transport === deviceTransport || device.transport === "unknown") : [];
};
const hasActiveSession = (snapshot: Snapshot | null) => !!snapshot && (!!snapshot.sessionSerial || !!snapshot.sessionReceiverBinding || snapshot.sessionConnectionTransport !== null);
const connectionLabel = (transport: Snapshot["connectionTransport"] | null, protocol: Snapshot["sessionWirelessProtocol"] | null) =>
  transport === "usb" ? zhCN.connection.usb : transport === "wireless" ? protocol === "udp" ? zhCN.connection.udp : zhCN.connection.adb : zhCN.connection.unknown;
const intendedConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.connectionTransport,
  snapshot.connectionTransport === "wireless" ? snapshot.settings.wirelessProtocol : null);
const actualConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.sessionConnectionTransport, snapshot.sessionWirelessProtocol);
const bindingLabel = (reference: string | null | undefined) => reference ? zhCN.connection.pairedReceiverWithReference(reference) : zhCN.connection.pairedReceiver;
const resetPhoneStateLabel = (state: string) => ({ empty: zhCN.network.resetPhoneEmpty, pending: zhCN.network.resetPhonePending, saved: zhCN.network.resetPhoneSaved, failed: zhCN.network.resetPhoneFailed, unknown: zhCN.network.resetPhoneUnknown } as Record<string, string>)[state] ?? state;
const resetPcStateLabel = (state: string) => ({ missing: zhCN.network.resetPcMissing, saved: zhCN.network.resetPcSaved, unknown: zhCN.network.resetPcUnknown } as Record<string, string>)[state] ?? state;
const resetOutcomeLabel = (outcome: string) => ({ reset: zhCN.network.resetOutcomeReset, alreadyEmpty: zhCN.network.resetOutcomeAlreadyEmpty, phoneStateUnknown: zhCN.network.resetOutcomePhoneUnknown, phoneRetained: zhCN.network.resetOutcomePhoneRetained, phoneClearedPcUnknown: zhCN.network.resetOutcomePhoneClearedPcUnknown, phoneClearedPcRetained: zhCN.network.resetOutcomePhoneClearedPcRetained, phoneClearedPcChanged: zhCN.network.resetOutcomePhoneClearedPcChanged, pcClearedPhoneEmpty: zhCN.network.resetOutcomePcClearedPhoneEmpty, pcClearedPhoneUnknown: zhCN.network.resetOutcomePcClearedPhoneUnknown, pcClearedPhoneChanged: zhCN.network.resetOutcomePcClearedPhoneChanged, targetChanged: zhCN.network.resetOutcomeTargetChanged } as Record<string, string>)[outcome] ?? zhCN.network.resetOutcomeReview;
const commitmentSummary = (value: string | null) => zhCN.network.commitmentSummary(value);
const playbackModeLabel = (mode: Snapshot["settings"]["playbackMode"]) => mode === "Balanced" ? zhCN.app.balanced : zhCN.app.lowLatency;
const readableStatus = (snapshot: Snapshot, cleanupPending: boolean) => {
  if (cleanupPending) return zhCN.app.disconnectedPending;
  if (snapshot.wirelessConnectionOffers.active) return zhCN.app.waitingPhone;
  if (snapshot.state === "reconnecting" && snapshot.networkRecoverySecondsRemaining != null)
    return zhCN.app.reconnecting(snapshot.networkRecoverySecondsRemaining);
  if (snapshot.state === "streaming" && snapshot.output === "computer") return zhCN.app.connectedComputer;
  if (snapshot.state === "streaming" && snapshot.playbackDiagnostics?.playbackAdvancing !== true)
    return zhCN.app.receivingAudio;
  return ({
    noDevice: snapshot.devices.length ? zhCN.app.waitingAuthorization : snapshot.connectionTransport === "wireless" ? zhCN.app.choosePairedPhone : zhCN.app.noPhone,
    unauthorized: zhCN.app.waitingAuthorization, ready: zhCN.app.ready, connecting: zhCN.app.connecting, connected: zhCN.app.connectedComputer,
    streaming: zhCN.app.playingOnPhone, phoneReady: zhCN.app.phoneReady, paused: zhCN.app.paused,
    reconnecting: zhCN.app.reconnectingShort, managementOnly: zhCN.app.managementOnly, error: zhCN.app.error,
  } satisfies Record<Snapshot["state"], string>)[snapshot.state];
};
const CAP_PLAYBACK_PROGRESS = 1 << 3;
const needsReceiverUpdate = (message: string) => message.includes("RECEIVER_UPDATE_REQUIRED:");
const receiverErrorMessage = (message: string) => needsReceiverUpdate(message) ? zhCN.app.receiverUpdateRequired : message;
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
  const [panel, setPanel] = useState<"device" | "settings" | "pair" | "network" | "receiver" | "setup" | "help" | "diagnostics" | null>(null);
  const [confirm, setConfirm] = useState<{ kind: "transport" | "wirelessProtocol" | "audioBackend" | "playback" | "device" | "network" | "receiverUsb"; value: string } | null>(null);
  const [pendingNetworkStart, setPendingNetworkStart] = useState<NetworkStartRequest | null>(null);
  const [pendingReceiverOperation, setPendingReceiverOperation] = useState<ReceiverUsbOperation | null>(null);
  const [receiverOperationBusy, setReceiverOperationBusy] = useState(false);
  const [receiverManagerSerial, setReceiverManagerSerial] = useState("");
  const [receiverOperationResult, setReceiverOperationResult] = useState<ReceiverUsbOperationResult | null>(null);
  const panelRef = useRef(panel);
  panelRef.current = panel;
  const [networkDialogError, setNetworkDialogError] = useState("");
  const [networkChoiceNeedsConfirmation, setNetworkChoiceNeedsConfirmation] = useState({ interfaceAddress: false, bindingReference: false });
  const [localSelected, setLocalSelected] = useState<string | null>(null);
  const [largeText, setLargeText] = useState(false);
  const [notice, setNotice] = useState("");
  const [working, setWorking] = useState(false);
  const snapshotRefreshGeneration = useRef(0);
  const lastOfferListenerChoice = useRef<{ localAddress: string; bindingReference: string } | null>(null);

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
  const cleanupPending = canDisconnect && snapshot?.state === "error";
  const receiverUpdateNeeded = snapshot?.state === "error" && needsReceiverUpdate(`${snapshot.message} ${snapshot.statusDetail}`)
    || snapshot?.state === "managementOnly" && snapshot.sessionCapabilities?.audioCompatible === false;
  const controlsDisabled = working || snapshot?.busy === true || cleanupPending || receiverOperationBusy;
  const activeDevice = snapshot?.devices.find(device => device.serial === snapshot.sessionSerial);
  const cardDevice = active ? activeDevice : cleanupPending ? undefined : selected;
  const deviceChoices = useMemo(() => {
    if (!snapshot?.sessionSerial || devices.some(device => device.serial === snapshot.sessionSerial)) return devices;
    const current = snapshot.devices.find(device => device.serial === snapshot.sessionSerial) ?? {
      serial: snapshot.sessionSerial, transport: snapshot.sessionTransport === "lan" ? "unknown" : snapshot.sessionTransport ?? "unknown", state: "device", manufacturer: "", model: "",
      androidVersion: snapshot.sessionAndroidVersion ?? "", apiLevel: "", displayName: snapshot.deviceName ?? zhCN.app.connectedPhone,
      statusText: zhCN.app.staleDeviceScan, connectable: false,
    };
    return [current, ...devices];
  }, [snapshot, devices]);
  const notify = (message: string) => { setNotice(receiverErrorMessage(message)); window.setTimeout(() => setNotice(""), 4200); };
  const openReceiverUpgrade = () => {
    setReceiverManagerSerial(snapshot?.sessionSerial ?? selected?.serial ?? "");
    setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver");
  };

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
    if (active && kind === "playback" && lanSession) {
      setWorking(true);
      try { apply(await invokeApp<Snapshot>("reconfigure_session", { kind, value })); }
      catch (error) {
        let message = String(error);
        try { apply(await invokeApp<Snapshot>("get_snapshot")); }
        catch (refreshError) { message += zhCN.app.refreshFailureSuffix(String(refreshError)); }
        notify(message);
      }
      finally { setWorking(false); }
      return;
    }
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
      if (refreshFailure) notify(zhCN.app.savedButRefreshFailed(refreshFailure)); else setNotice("");
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
        throw new Error(zhCN.app.usbVersionVerificationFailed(operation.targetVersionCode));
      }
      setReceiverOperationResult({ serial: operation.serial, kind: operation.kind, targetVersionCode: operation.targetVersionCode, status });
      setReceiverManagerSerial(operation.serial);
      if (panelRef.current === operation.returnPanel) setPanel(operation.returnPanel);
      try { apply(await invokeApp<Snapshot>("refresh_devices")); }
      catch (error) { setNetworkDialogError(zhCN.app.receiverVersionVerifiedRefreshFailed(status.versionCode, String(error))); }
      notify(zhCN.app.receiverVersionVerifiedReconnect(status.versionCode));
    } catch (error) {
      setNetworkDialogError(String(error));
      setReceiverManagerSerial(operation.serial);
      if (panelRef.current === operation.returnPanel) setPanel(operation.returnPanel);
    } finally { setReceiverOperationBusy(false); setWorking(false); }
  };

  const requestReceiverUsbOperation = (operation: ReceiverUsbOperation) => {
    setReceiverOperationResult(null); setNetworkDialogError("");
    if (active && snapshot) {
      const targetMatches = snapshot.sessionTransport === "lan"
        ? operation.bindingReference === snapshot.sessionReceiverBinding
        : operation.serial === snapshot.sessionSerial;
      if (!targetMatches) {
        setNetworkDialogError(zhCN.app.differentCurrentSessionTarget);
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
        if (!pendingNetworkStart) throw new Error(zhCN.app.staleNetworkParameters);
        const stopped = await invokeApp<OperationResult>("disconnect");
        apply(stopped.snapshot);
        if (!stopped.success) throw new Error(stopped.message);
        apply(await invokeApp<Snapshot>("start_network_audio", { ...pendingNetworkStart }));
        setPanel(null);
      } else if (confirm.kind === "receiverUsb") {
        if (!pendingReceiverOperation) throw new Error(zhCN.app.staleUsbOperation);
        const operation = pendingReceiverOperation;
        setConfirm(null); setPendingReceiverOperation(null);
        setPanel(operation.returnPanel);
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
          catch (refreshError) { message += zhCN.app.refreshFailureSuffix(String(refreshError)); }
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
    if (cleanupPending) { setNetworkDialogError(zhCN.app.cleanupBeforeNetworkStart); return; }
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
    if (!snapshot || (allowed && (active || working || snapshot.busy))) return;
    if (allowed) setWorking(true);
    setNetworkDialogError("");
    try {
      let selectedAddress = localAddress;
      let selectedBinding = bindingReference;
      if (allowed) {
        const addressWasSpecified = !!selectedAddress;
        const bindingWasSpecified = !!selectedBinding;
        const [availableInterfaces, availableBindings] = await Promise.all([
          invokeApp<PrivateIpv4Interface[]>("network_interfaces"),
          invokeApp<string[]>("receiver_binding_references"),
        ]);
        const availableAddress = (address: string | null | undefined) => !!address && availableInterfaces.some(value => value.address === address);
        const availableBinding = (binding: string | null | undefined) => !!binding && availableBindings.includes(binding);
        const snapshotAddress = snapshot.wirelessConnectionOffers.interfaceAddress;
        const snapshotBinding = snapshot.wirelessConnectionOffers.automaticBinding;
        const rememberedChoice = lastOfferListenerChoice.current;
        const addressChoiceInvalidated = !selectedAddress && !localAddress && (!!snapshotAddress || !!rememberedChoice?.localAddress)
          && !availableAddress(snapshotAddress) && !availableAddress(rememberedChoice?.localAddress);
        const bindingChoiceInvalidated = !selectedBinding && !bindingReference && (!!snapshotBinding || !!rememberedChoice?.bindingReference)
          && !availableBinding(snapshotBinding) && !availableBinding(rememberedChoice?.bindingReference);
        if (!availableAddress(selectedAddress)) selectedAddress = "";
        if (!availableBinding(selectedBinding)) selectedBinding = "";
        if (!selectedAddress && !addressWasSpecified && availableAddress(snapshotAddress)) selectedAddress = snapshotAddress!;
        if (!selectedBinding && !bindingWasSpecified && availableBinding(snapshotBinding)) selectedBinding = snapshotBinding!;
        if (!selectedAddress && !addressWasSpecified && availableAddress(lastOfferListenerChoice.current?.localAddress))
          selectedAddress = lastOfferListenerChoice.current!.localAddress;
        if (!selectedBinding && !bindingWasSpecified && availableBinding(lastOfferListenerChoice.current?.bindingReference))
          selectedBinding = lastOfferListenerChoice.current!.bindingReference;
        if (!selectedAddress && !addressWasSpecified && !addressChoiceInvalidated && availableInterfaces.length === 1) selectedAddress = availableInterfaces[0].address;
        if (!selectedBinding && !bindingWasSpecified && !bindingChoiceInvalidated && availableBindings.length === 1) selectedBinding = availableBindings[0];
        if (!selectedBinding && availableBindings.length === 0) { setPanel("setup"); return; }
        if (!selectedAddress || !selectedBinding) {
          setNetworkChoiceNeedsConfirmation({ interfaceAddress: addressChoiceInvalidated, bindingReference: bindingChoiceInvalidated });
          setPanel("network"); return;
        }
      }
      await invokeApp<HostDiscoverySnapshot>("set_wireless_connection_offers", {
        localAddress: selectedAddress, allowed, bindingReference: selectedBinding,
      });
      if (allowed) {
        lastOfferListenerChoice.current = { localAddress: selectedAddress, bindingReference: selectedBinding! };
        setNetworkChoiceNeedsConfirmation({ interfaceAddress: false, bindingReference: false });
      }
      const generation = ++snapshotRefreshGeneration.current;
      const next = await invokeApp<Snapshot>("get_snapshot");
      if (generation === snapshotRefreshGeneration.current) apply(next);
    } catch (error) {
      if (panel === "network") setNetworkDialogError(String(error)); else notify(String(error));
    }
    finally { if (allowed) setWorking(false); }
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
        const result = await invokeApp<{ success: boolean; message: string; snapshot: Snapshot }>("connect_device", { serial: selected.serial });
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
    if (snapshot.state === "connecting" || snapshot.state === "reconnecting") {
      void invokeApp<Snapshot>("cancel_operation").then(apply).catch(error => notify(String(error)));
    } else if (snapshot.wirelessConnectionOffers.active) {
      void setOfferListener(snapshot.wirelessConnectionOffers.interfaceAddress ?? "", false);
    } else if (cleanupPending || (active && (snapshot.state === "managementOnly" || snapshot.output === "phone"))) {
      void disconnect();
    } else if (receiverUpdateNeeded && !active) {
      openReceiverUpgrade();
    } else if (!active && wirelessUdpMode) {
      void setOfferListener("", true);
    } else if (active || selected?.connectable) {
      void setOutput("phone");
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

  if (!snapshot) return <main className="loading-shell"><AudioLines size={25} /><span>{zhCN.app.loading}</span></main>;
  const empty = devices.length === 0;
  const cancellingConnection = snapshot.state === "connecting" || snapshot.state === "reconnecting";
  const statusTitle = readableStatus(snapshot, cleanupPending);
  const statusDetail = snapshot.state === "streaming"
    ? snapshot.output !== "phone" ? zhCN.app.outputComputer
      : snapshot.playbackDiagnostics?.playbackAdvancing === true
      ? zhCN.app.playbackProgress
      : zhCN.app.receivingAudio
    : lanSession
    ? snapshot.state === "managementOnly" ? zhCN.app.receiverManagementOnly
      : snapshot.state === "phoneReady" ? snapshot.output === "computer" ? zhCN.app.receiverComputer : zhCN.app.receiverAwaitingPlayback
      : snapshot.state === "paused" ? zhCN.app.receiverPaused
      : snapshot.state === "reconnecting" ? zhCN.app.directNetworkRecovering
          : snapshot.state === "error" ? snapshot.statusDetail || snapshot.message || zhCN.app.retryDisconnectOrDiagnostics
            : snapshot.output === "computer" ? zhCN.app.networkSessionComputer : zhCN.app.networkSessionPhone
    : snapshot.statusDetail;
  const mainActionCanStop = snapshot.wirelessConnectionOffers.active || cleanupPending
    || (active && (snapshot.output === "phone" || snapshot.state === "managementOnly"));
  const mainActionDisabled = !cancellingConnection && !mainActionCanStop
    && (receiverOperationBusy || working || snapshot.busy);
  const mainActionLabel = cancellingConnection ? zhCN.app.connectCancel
    : snapshot.wirelessConnectionOffers.active ? zhCN.app.stopReceiving
      : cleanupPending ? zhCN.app.retryDisconnect
        : receiverUpdateNeeded && !active ? zhCN.app.upgradePhone
        : active && snapshot.state === "managementOnly" ? zhCN.app.disconnectManagement
          : active && snapshot.output === "phone" ? zhCN.app.stop
        : !active && wirelessUdpMode ? working || snapshot.busy ? zhCN.app.processing : zhCN.app.waitForPhone
              : active || selected?.connectable ? working || snapshot.busy ? zhCN.app.processing : zhCN.app.playToPhone
              : snapshot.connectionTransport === "usb" ? zhCN.app.connectUsbPhone : zhCN.app.addWirelessPhone;
  const failureDetail = [snapshot.message, snapshot.statusDetail].filter(Boolean).join("；") || statusDetail;

  return <div className={`app-shell ${largeText ? "large-text" : ""}`}>
    <header className="topbar">
      <div className="brand"><img src="/icon.png" alt="" /><div><strong>AnPod</strong><span>{zhCN.app.tagline}</span></div></div>
      <button className="icon-button" aria-label={zhCN.app.openSettings} title={zhCN.app.settings} onClick={() => setPanel("settings")}><Settings2 size={18} /></button>
    </header>

    <main className="main-scroll">
      <section className="device-section" aria-label={zhCN.app.targetPhone}>
        <div className="section-title"><h1>{zhCN.app.targetPhone}</h1></div>
        <div className="device-card">
          <button className="device-select" onClick={() => setPanel(wirelessUdpMode && !active ? "network" : "device")} disabled={controlsDisabled} aria-haspopup="dialog" aria-expanded={panel === "device" || panel === "network" || panel === "setup"} aria-label={`${zhCN.app.chooseTarget}，${active ? `${zhCN.app.currentConnection}${lanSession ? bindingLabel(snapshot.sessionReceiverBinding) : activeDevice?.displayName ?? snapshot.deviceName ?? zhCN.app.phone}` : cleanupPending ? zhCN.app.awaitingCleanup : wirelessUdpMode ? snapshot.wirelessConnectionOffers.automaticBinding ? bindingLabel(snapshot.wirelessConnectionOffers.automaticBinding) : zhCN.app.choosePairedReceiver : cardDevice ? `${zhCN.app.currentSelection}${cardDevice.displayName}` : zhCN.app.openDeviceList}`}>
            <span className="device-icon"><Smartphone size={18} aria-hidden="true" /></span>
            <span className="device-copy">
              <strong>{active ? lanSession ? bindingLabel(snapshot.sessionReceiverBinding) : activeDevice?.displayName ?? snapshot.deviceName ?? zhCN.app.connectedPhone : cleanupPending ? snapshot.deviceName ?? zhCN.app.awaitingCleanup : wirelessUdpMode ? snapshot.wirelessConnectionOffers.automaticBinding ? bindingLabel(snapshot.wirelessConnectionOffers.automaticBinding) : zhCN.app.chooseReceiverPhone : selected?.displayName ?? (empty ? zhCN.app.noPhone : zhCN.app.choosePhone)}</strong>
              <span>{active ? `${actualConnectionLabel(snapshot)}${snapshot.sessionAndroidVersion ? ` · Android ${snapshot.sessionAndroidVersion}` : ""}` : cleanupPending ? zhCN.app.connectionUnavailable : wirelessUdpMode ? zhCN.connection.udp : cardDevice ? `${cardDevice.transport === "wifi" ? zhCN.connection.adb : zhCN.connection.usb}${cardDevice.androidVersion ? ` · Android ${cardDevice.androidVersion}` : ""}` : snapshot.connectionTransport === "wireless" ? zhCN.app.wirelessAdbDevice : zhCN.app.usbDevice}</span>
              {cleanupPending && <span className="device-live-state" role="status">{zhCN.app.retryCleanup}</span>}
            </span>
            <ChevronDown size={18} aria-hidden="true" />
          </button>
          <div className="device-actions">
            {!active && !wirelessUdpMode && selected?.connectable && <button className="text-button" type="button" disabled={controlsDisabled} onClick={() => void connectOnly()}>{zhCN.app.connectOnly}</button>}
            {active && snapshot.output === "phone" && snapshot.state !== "managementOnly" && <button className="text-button" type="button" disabled={controlsDisabled} onClick={() => void setOutput("computer")}>{zhCN.app.switchToComputer}</button>}
            {snapshot.connectionTransport === "wireless" && <button className="text-button" type="button" disabled={controlsDisabled} onClick={() => setPanel(wirelessUdpMode ? "network" : "pair")}>{zhCN.app.addPhone}</button>}
          </div>
        </div>
      </section>

      <section className="connection-section" aria-label={zhCN.app.connectionMode}>
        <div className="section-title"><h2>{zhCN.app.connectionMode}</h2></div>
        <div className="transport-switch" role="group" aria-label={zhCN.app.connectionMode}>
          <button aria-pressed={snapshot.connectionTransport === "usb"} disabled={controlsDisabled || snapshot.wirelessConnectionOffers.active} onClick={() => void setOption("transport", "usb")}><Usb size={16} aria-hidden="true" />{zhCN.app.usb}</button>
          <button aria-pressed={snapshot.connectionTransport === "wireless"} disabled={controlsDisabled || snapshot.wirelessConnectionOffers.active} onClick={() => void setOption("transport", "wireless")}><Wifi size={16} aria-hidden="true" />{zhCN.app.wireless}</button>
        </div>
      </section>

      <div className={`status-line state-${snapshot.state}`}>
        <span className="status-dot" aria-hidden="true" />
        <span role="status" aria-live="polite">{statusTitle}</span>
      </div>
      <button className="primary-button main-action" type="button" disabled={mainActionDisabled} onClick={runMainAction}>
        {mainActionLabel}
      </button>
      {!active && wirelessUdpMode && !snapshot.wirelessConnectionOffers.active && <p className="status-description">{zhCN.app.wirelessUdpIdleHint}</p>}
      {(!active || snapshot.sessionConnectionTransport === "wireless") && <button className="text-button setup-wizard-entry" type="button" disabled={working || receiverOperationBusy} onClick={() => setPanel("setup")}>{zhCN.app.firstSetup}</button>}
      {snapshot.state === "error" || cleanupPending || snapshot.wirelessConnectionOffers.active ? <p className={`status-description${snapshot.state === "error" || cleanupPending ? " actionable-error" : ""}`} role={snapshot.state === "error" || cleanupPending ? "alert" : "status"} aria-live={snapshot.state === "error" || cleanupPending ? undefined : "polite"}>{cleanupPending || snapshot.state === "error" ? receiverErrorMessage(failureDetail) : zhCN.app.waitingForRequest}</p> : null}
      {receiverUpdateNeeded && (active || cleanupPending) && <button className="primary-button" type="button" disabled={working || snapshot.busy || receiverOperationBusy} onClick={openReceiverUpgrade}>{zhCN.app.upgradePhone}</button>}

      <section className="mode-section" aria-label={zhCN.app.playbackMode}>
        <div className="mode-row"><h2>{zhCN.app.playbackMode}</h2><div className="mode-switch" role="group" aria-label={zhCN.app.playbackMode}>
          <button aria-pressed={snapshot.settings.playbackMode === "Balanced"} disabled={controlsDisabled} onClick={() => void setOption("playback", "balanced")}>{zhCN.app.balanced}</button>
          <button aria-pressed={snapshot.settings.playbackMode === "LowLatency"} disabled={controlsDisabled} onClick={() => void setOption("playback", "lowLatency")}>{zhCN.app.lowLatency}</button>
        </div></div>
        <p className="mode-description" role="status" aria-live="polite">{snapshot.settings.playbackMode === "Balanced" ? zhCN.app.balancedDescription : zhCN.app.lowLatencyDescription}</p>
      </section>
    </main>

    <footer className="footer"><button onClick={() => setPanel("help")}><CircleHelp size={15} />{zhCN.app.help}</button><span>v{snapshot.version}</span><button onClick={() => setPanel("diagnostics")}><Activity size={15} />{zhCN.app.diagnostics}</button></footer>
    {notice && <div className="toast" role="status">{notice}</div>}

    {panel === "device" && <DeviceDialog devices={deviceChoices} selected={localSelected} activeSerial={snapshot.sessionSerial} activeBinding={lanSession ? snapshot.sessionReceiverBinding : null} busy={working || snapshot.busy} onSelect={chooseDevice} onRefresh={refresh} onClose={() => setPanel(null)} />}
    {panel === "settings" && <SettingsDialog snapshot={snapshot} largeText={largeText} onLargeText={setLargeText} udpBackendFixed={udpBackendFixed} busy={controlsDisabled} onSetOption={(kind, value) => void setOption(kind, value)} onOpenReceiverManager={() => { setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver"); }} onSave={async (key, value) => {
      try { const next = await invokeApp<Snapshot>("update_setting", { key, value }); apply(next); if (key === "preferredTransport") apply(await invokeApp<Snapshot>("refresh_devices")); }
      catch (error) { notify(String(error)); }
    }} onClose={() => setPanel(null)} />}
    {(panel === "network" || panel === "receiver" || panel === "setup") && <NetworkReceiverDialog key={panel} snapshot={snapshot} setupWizard={panel === "setup"} receiverManagerOnly={panel === "receiver"} initialReceiverSerial={receiverManagerSerial} requiredChoices={networkChoiceNeedsConfirmation} usbDevices={snapshot.devices.filter(device => device.transport === "usb" && device.connectable)} usbSeenDevices={snapshot.devices.filter(device => device.transport === "usb")} disabled={working || snapshot.busy} receiverOperationBusy={receiverOperationBusy} receiverOperationResult={receiverOperationResult} audioSessionActive={active} udpMode={snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp"} offers={snapshot.wirelessConnectionOffers} error={networkDialogError} onError={setNetworkDialogError} onStart={startNetwork} onSetOfferListener={setOfferListener} onAcceptOffer={acceptOffer} onReceiverSerialChange={serial => { if (serial !== receiverManagerSerial) setReceiverOperationResult(null); setReceiverManagerSerial(serial); }} onRefreshDevices={refresh} onReceiverOperation={requestReceiverUsbOperation} onOpenReceiverManager={() => { setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver"); }} onClose={() => setPanel(null)} />}
    {panel === "pair" && <PairDialog transport="wifi" initialEndpoint={snapshot.settings.lastWirelessEndpoint ?? ""} disabled={working || snapshot.busy} onClose={() => setPanel(null)} onDone={async () => { apply(await invokeApp<Snapshot>("refresh_devices")); setPanel("device"); }} />}
    {panel === "help" && <HelpDialog transport={snapshot.connectionTransport} wirelessProtocol={snapshot.settings.wirelessProtocol} onClose={() => setPanel(null)} />}
    {panel === "diagnostics" && <DiagnosticsDialog snapshot={snapshot} onClose={() => setPanel(null)} />}
    {confirm && <ConfirmDialog
      title={confirm.kind === "receiverUsb" ? pendingReceiverOperation?.kind === "upgrade" ? zhCN.app.switchReceiverUpgradeTitle : zhCN.app.switchReceiverInstallTitle : confirm.kind === "network" ? zhCN.app.switchNetworkTitle : confirm.kind === "device" ? zhCN.app.switchDeviceTitle : confirm.kind === "transport" ? zhCN.app.switchTransportTitle : confirm.kind === "wirelessProtocol" ? zhCN.app.switchWirelessProtocolTitle : confirm.kind === "audioBackend" ? zhCN.app.switchAudioBackendTitle : zhCN.app.switchPlaybackTitle}
      body={confirm.kind === "receiverUsb" ? pendingReceiverOperation ? zhCN.app.receiverInstallOrCheck(pendingReceiverOperation.targetVersionCode) : zhCN.app.receiverOperation
        : confirm.kind === "network" ? zhCN.app.networkConfirmation
        : confirm.kind === "device" && lanSession ? zhCN.app.switchLanDevice(snapshot.devices.find(device => device.serial === confirm.value)?.displayName ?? zhCN.app.selectedPhone)
          : confirm.kind === "transport" || confirm.kind === "wirelessProtocol" ? zhCN.app.switchConnectionSettings
            : confirm.kind === "audioBackend" ? zhCN.app.switchBackend
            : confirm.kind === "device" ? zhCN.app.switchPhoneContinue(snapshot.devices.find(device => device.serial === confirm.value)?.displayName ?? zhCN.app.selectedPhone, snapshot.output === "phone")
              : zhCN.app.switchPlayback}
      confirmLabel={confirm.kind === "receiverUsb" ? zhCN.app.stopAndContinue : confirm.kind === "network" ? zhCN.app.disconnectAndStart : confirm.kind === "device" ? zhCN.app.interruptAndSwitch : confirm.kind === "transport" || confirm.kind === "wirelessProtocol" || confirm.kind === "audioBackend" ? zhCN.app.confirmSwitch : zhCN.app.reconfigure} busy={working} onCancel={() => { if (confirm.kind === "receiverUsb") setPanel(pendingReceiverOperation?.returnPanel ?? "receiver"); setConfirm(null); setPendingNetworkStart(null); setPendingReceiverOperation(null); }} onConfirm={() => void applyConfirmed()} />}
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
    <div className="modal-head"><h2>{title}</h2><button className="icon-button" onClick={onClose} aria-label={zhCN.modal.close}><X size={18} /></button></div>{children}
  </dialog>;
}

function NetworkReceiverDialog({ snapshot, setupWizard, usbDevices, usbSeenDevices, receiverManagerOnly, initialReceiverSerial, requiredChoices, disabled, receiverOperationBusy, receiverOperationResult, audioSessionActive, udpMode, offers, error, onError, onStart, onSetOfferListener, onAcceptOffer, onReceiverSerialChange, onRefreshDevices, onReceiverOperation, onOpenReceiverManager, onClose }: {
  snapshot: Snapshot;
  setupWizard: boolean;
  receiverManagerOnly: boolean;
  initialReceiverSerial: string;
  requiredChoices: { interfaceAddress: boolean; bindingReference: boolean };
  usbDevices: Device[]; usbSeenDevices: Device[]; disabled: boolean; receiverOperationBusy: boolean; receiverOperationResult: ReceiverUsbOperationResult | null; audioSessionActive: boolean; udpMode: boolean; offers: HostDiscoverySnapshot; error: string; onError: (error: string) => void;
  onStart: (request: NetworkStartRequest) => void; onSetOfferListener: (localAddress: string, allowed: boolean, bindingReference?: string) => void;
  onAcceptOffer: (offerId: string, bindingReference: string) => void; onReceiverSerialChange: (serial: string) => void;
  onRefreshDevices: () => void; onReceiverOperation: (operation: ReceiverUsbOperation) => void; onOpenReceiverManager: () => void; onClose: () => void;
}) {
  const [method, setMethod] = useState<"bootstrap" | "paired">("bootstrap");
  const [setupStep, setSetupStep] = useState<1 | 2 | 3>(1);
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
  const [bindingResetPreview, setBindingResetPreview] = useState<BindingResetPreview | null>(null);
  const [bindingResetResult, setBindingResetResult] = useState<BindingResetResult | null>(null);
  const [bindingResetError, setBindingResetError] = useState("");
  const [bindingResetLoading, setBindingResetLoading] = useState(false);
  const [bindingResetBusy, setBindingResetBusy] = useState(false);
  const alertRef = useRef<HTMLParagraphElement>(null);
  const setupStepRef = useRef<HTMLDivElement>(null);
  const receiverGeneration = useRef(0);
  const serialRef = useRef(serial);
  serialRef.current = serial;
  const discoveryGeneration = useRef(0);
  const networkInfoGeneration = useRef(0);
  const bindingResetGeneration = useRef(0);
  const methodManuallySelected = useRef(receiverManagerOnly || setupWizard);
  const peerManuallyEdited = useRef(false);
  const busy = disabled || loading || receiverAction !== null || receiverOperationBusy || candidateLoading || bindingResetLoading || bindingResetBusy;
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
  useLayoutEffect(() => {
    if (setupWizard) setupStepRef.current?.querySelector<HTMLElement>("select, button:not(:disabled), input")?.focus();
  }, [setupStep, setupWizard]);

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
        : !requiredChoices.interfaceAddress && interfacesResult.value.some(value => value.address === offers.interfaceAddress) ? offers.interfaceAddress ?? ""
          : !requiredChoices.interfaceAddress && interfacesResult.value.length === 1 ? interfacesResult.value[0].address : "");
      if (interfacesResult.value.length === 0) errors.push(zhCN.network.unavailableComputerInterface);
    } else errors.push(String(interfacesResult.reason));
    if (bindingsResult.status === "fulfilled") {
      const values = bindingsResult.value;
      setBindings(values);
      setBindingReference(current => values.includes(current) ? current
        : receiverManagerOnly && snapshot.sessionTransport === "lan" && snapshot.sessionReceiverBinding && values.includes(snapshot.sessionReceiverBinding)
          ? snapshot.sessionReceiverBinding
          : !receiverManagerOnly && !requiredChoices.bindingReference && values.length === 1 ? values[0] : "");
      if (!methodManuallySelected.current) setMethod(values.length > 0 ? "paired" : "bootstrap");
    } else errors.push(String(bindingsResult.reason));
    setLocalError(errors.join("；"));
    if (networkInfoGeneration.current === generation) setLoading(false);
  };
  const invalidateBindingReset = () => {
    bindingResetGeneration.current++;
    setBindingResetPreview(null); setBindingResetResult(null); setBindingResetError("");
    setBindingResetLoading(false); setBindingResetBusy(false);
  };
  const previewBindingReset = async (deviceSerial: string, recheck = false) => {
    const generation = ++bindingResetGeneration.current;
    const isCurrent = () => generation === bindingResetGeneration.current && serialRef.current === deviceSerial;
    setBindingResetPreview(null); setBindingResetResult(null); setBindingResetError("");
    setBindingResetLoading(true);
    const independentRefresh = recheck
      ? Promise.all([loadNetworkInfo(), Promise.resolve(onRefreshDevices())])
      : null;
    try {
      if (recheck) {
        const status = await checkReceiver(deviceSerial);
        if (!isCurrent()) return;
        if (!status) throw new Error(zhCN.network.receiverStatusUnavailable);
      }
      if (!isCurrent()) return;
      const preview = await invokeApp<BindingResetPreview>("receiver_binding_reset_preview", { serial: deviceSerial });
      if (!isCurrent()) return;
      if (independentRefresh) {
        await independentRefresh;
        if (!isCurrent()) return;
      }
      setBindingResetPreview(preview);
    } catch (reason) {
      if (isCurrent()) setBindingResetError(String(reason));
    } finally {
      if (isCurrent()) setBindingResetLoading(false);
    }
  };
  const confirmBindingReset = async () => {
    const preview = bindingResetPreview;
    if (!preview || preview.serial !== serialRef.current || preview.receiverBusy || preview.phoneState === "pending" || preview.phoneState === "failed") return;
    const generation = ++bindingResetGeneration.current;
    setBindingResetPreview(null); setBindingResetError(""); setBindingResetBusy(true);
    try {
      const result = await invokeApp<BindingResetResult>("receiver_reset_binding", { preview });
      if (generation !== bindingResetGeneration.current || serialRef.current !== preview.serial) return;
      setBindingResetResult(result);
      await Promise.all([loadNetworkInfo(), Promise.resolve(onRefreshDevices())]);
    } catch (reason) {
      if (generation === bindingResetGeneration.current && serialRef.current === preview.serial) setBindingResetError(String(reason));
    } finally {
      if (generation === bindingResetGeneration.current && serialRef.current === preview.serial) setBindingResetBusy(false);
    }
  };
  useEffect(() => () => { bindingResetGeneration.current++; }, []);
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
    invalidateBindingReset();
    const next = usbDevices.some(device => device.serial === serial) ? serial
      : receiverManagerOnly && usbDevices.some(device => device.serial === initialReceiverSerial) ? initialReceiverSerial
        : receiverManagerOnly && usbDevices.length === 1 ? usbDevices[0].serial
        : !receiverManagerOnly && usbDevices.length === 1 ? usbDevices[0].serial : "";
    setSerial(next);
    onReceiverSerialChange(next);
  }, [receiverDeviceKey, receiverManagerOnly, initialReceiverSerial]);
  const readReceiverStatusAndInterfaces = async (deviceSerial: string, generation: number): Promise<ReceiverStatus | null> => {
    const status = await invokeApp<ReceiverStatus>("receiver_status", { serial: deviceSerial });
    if (receiverGeneration.current !== generation || serialRef.current !== deviceSerial) return null;
    setReceiverStatus(status);
    setReceiverError("");
    if (receiverManagerOnly) {
      setReceiverInterfaces([]); setReceiverAddress(""); setLocalError("");
      return status;
    }
    if (!status.installed) {
      setReceiverInterfaces([]); setReceiverAddress(""); setLocalError("");
      return status;
    }
    const values = await invokeApp<ReceiverIpv4Interface[]>("receiver_network_interfaces", { serial: deviceSerial });
    if (receiverGeneration.current !== generation || serialRef.current !== deviceSerial) return null;
    setReceiverInterfaces(values);
    setReceiverAddress(current => values.some(value => value.address === current) ? current : "");
    setLocalError(values.length === 0 ? zhCN.network.receiverHasNoIpv4 : "");
    return status;
  };
  const checkReceiver = async (deviceSerial: string): Promise<ReceiverStatus | null> => {
    const generation = ++receiverGeneration.current;
    onError("");
    setReceiverStatus(null); setReceiverError(""); setReceiverInterfaces([]); setReceiverAddress(""); setLocalError("");
    setReceiverAction("checking");
    try { return await readReceiverStatusAndInterfaces(deviceSerial, generation); }
    catch (reason) {
      if (receiverGeneration.current === generation && serialRef.current === deviceSerial) setReceiverError(String(reason));
      return null;
    }
    finally { if (receiverGeneration.current === generation && serialRef.current === deviceSerial) setReceiverAction(null); }
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
      if (!interfaces.some(value => value.address === localAddress)) setLocalAddress(!requiredChoices.interfaceAddress && interfaces.length === 1 ? interfaces[0].address : "");
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
  }, [interfaces, localAddress, method, receiverAddress, receiverStatus?.installed, requiredChoices.interfaceAddress, validBootstrapPairs]);

  const discoverCandidates = async (automatic = false) => {
    const local = interfaces.find(value => value.address === localAddress);
    if (!local) return;
    const generation = ++discoveryGeneration.current;
    setLoading(true); setCandidates([]);
    if (!automatic) setLocalError("");
    setAddressNotice(zhCN.network.searchingCandidates);
    try {
      const values = await invokeApp<NetworkCandidate[]>("discover_network_receivers", { localAddress });
      if (discoveryGeneration.current !== generation) return;
      const usable = values.filter(value => endpointFitsInterface(value.endpoint, local));
      setCandidates(usable);
      if (!peerManuallyEdited.current) setPeer(usable.length === 1 ? usable[0].endpoint : "");
      setAddressNotice(usable.length === 1 ? zhCN.network.oneUnverifiedCandidate
        : usable.length > 1 ? zhCN.network.manyUnverifiedCandidates
          : zhCN.network.noCandidateSuggestion);
      if (usable.length === 0 && !automatic) setLocalError(zhCN.network.noReceiverDiscovered);
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
    if (!localAddress) { setLocalError(zhCN.network.chooseComputerInterface); return; }
    if (method === "bootstrap") {
      if (!receiverStatus?.installed) { setLocalError(zhCN.network.checkInstallReceiverFirst); return; }
      const receiver = receiverInterfaces.find(value => value.address === receiverAddress);
      if (!serial || !receiver) { setLocalError(zhCN.network.chooseAuthorizedUsbWithAddress); return; }
      onStart({ localAddress, peer: "", bootstrapSerial: serial, receiverAddress: receiver.address, receiverPrefixLength: receiver.prefixLength });
      return;
    }
    if (!bindingReference) { setLocalError(zhCN.network.choosePairedBinding); return; }
    const local = interfaces.find(value => value.address === localAddress);
    if (!isIpv4Endpoint(peer)) { setLocalError(zhCN.network.chooseCandidateOrIpv4); return; }
    if (!endpointFitsInterface(peer, local)) { setLocalError(zhCN.network.endpointOutsideSubnet); return; }
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
  const currentVersionLabel = receiverStatus?.installed === false ? zhCN.network.notInstalled
    : currentVersionCode === null ? zhCN.network.awaitingUsbVerification : String(currentVersionCode);
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
      targetVersionCode: updateCandidate.versionCode, returnPanel: receiverManagerOnly ? "receiver" : setupWizard ? "setup" : "network",
    });
  };
  const pairedServiceStopped = method === "paired" && !!serial && receiverStatus?.installed === true && !receiverStatus.serviceActive;
  const mainSummary = receiverManagerOnly ? selectedReceiverDevice?.displayName ?? zhCN.network.chooseAuthorizedUsb : method === "paired"
    ? bindingReference ? bindingLabel(bindingReference) : zhCN.network.choosePairedReceiver
    : selectedReceiverDevice?.displayName ?? zhCN.network.chooseUsbPhone;
  const addressSummary = receiverManagerOnly ? zhCN.network.managerAddressSummary
    : localAddress
    ? `${interfaces.find(value => value.address === localAddress)?.name ?? zhCN.network.computerInterface} · ${localAddress}${method === "paired" && peer ? ` → ${peer}` : method === "bootstrap" && receiverAddress ? ` ↔ ${receiverAddress}` : ""}`
    : zhCN.network.interfacePending;
  const advancedRequired = method === "paired"
    ? bindings.length !== 1 || !bindingReference || interfaces.length !== 1 && !localAddress
    : usbDevices.length !== 1 || !serial || receiverStatus?.installed !== true || !selectedBootstrapPair;

  const unauthorizedUsb = usbSeenDevices.some(device => device.state === "unauthorized" || device.statusText.includes("未授权"));
  const setUpStep = (step: 1 | 2 | 3) => { setSetupStep(step); setLocalError(""); onError(""); };

  return <Modal title={setupWizard ? zhCN.network.dialogSetup : receiverManagerOnly ? zhCN.network.dialogReceiverManager : zhCN.network.dialogWirelessReceiver} onClose={onClose} className={`network-dialog${setupWizard ? " setup-wizard-dialog" : ""}`}>
    <form className="network-pane" onSubmit={submit}>
      {setupWizard && <section className="setup-wizard" aria-label={zhCN.network.setupUsb}>
        <ol className="setup-step-list" aria-label={zhCN.network.setupSteps}>
          {([zhCN.network.setupStepUsb, zhCN.network.setupStepReceiver, zhCN.network.setupStepBinding] as const).map((label, index) => {
            const step = (index + 1) as 1 | 2 | 3;
            return <li key={label}><button type="button" className="setup-step" aria-current={setupStep === step ? "step" : undefined} disabled={busy || step > setupStep} onClick={() => setUpStep(step)}><span>{step}</span>{label}</button></li>;
          })}
        </ol>
        {setupStep === 1 && <div ref={setupStepRef} className="setup-wizard-step">
          <p>{zhCN.network.connectAndAuthorizeUsb}</p>
          <label className="network-field">{zhCN.network.authorizedOnlineUsb}
            <select required value={serial} disabled={busy} onChange={event => { setSerial(event.target.value); onReceiverSerialChange(event.target.value); setLocalError(""); onError(""); }}>
              <option value="">{zhCN.network.chooseUsbPlaceholder}</option>
              {usbDevices.map(device => <option key={device.serial} value={device.serial}>{device.displayName}</option>)}
            </select>
          </label>
          {usbDevices.length === 0 && <div className="network-recovery" role="status">
            <span>{unauthorizedUsb ? zhCN.network.usbDetectedUnauthorized : zhCN.network.noAuthorizedUsb}</span>
            <button type="button" className="text-button" disabled={busy} onClick={onRefreshDevices}>{zhCN.network.refreshUsbDevices}</button>
          </div>}
          <div className="setup-wizard-actions"><button className="primary-button" type="button" disabled={busy || !serial} onClick={() => setUpStep(2)}>{zhCN.network.next}</button></div>
        </div>}
        {setupStep === 2 && <div ref={setupStepRef} className="setup-wizard-step">
          <p>{zhCN.network.checkReceiverOnPhone}</p>
          {serial ? <section className="receiver-manager" aria-label={zhCN.network.receiverInstallCheck}>
            <div className="receiver-status-row">
              <div className="receiver-status-copy" role="status" aria-live="polite">
                <strong>{receiverAction === "checking" ? zhCN.network.checkingReceiver : receiverError ? zhCN.network.receiverCheckFailed : !receiverStatus ? zhCN.network.waitingReceiverCheck : !receiverStatus.installed ? zhCN.network.receiverNotInstalled : receiverStatus.serviceActive ? receiverStatus.healthy ? zhCN.network.receiverHealthy : zhCN.network.receiverUnhealthy : zhCN.network.receiverStopped}</strong>
                {receiverStatus?.installed && <small>{[receiverStatus.versionName, receiverStatus.versionCode === null ? "" : zhCN.network.versionCode(receiverStatus.versionCode)].filter(Boolean).join(" · ")}</small>}
                {receiverStatus && !receiverStatus.healthy && <small>{receiverStatus.error || receiverStatus.message}</small>}
                {receiverNeedsUpgrade && <small>{zhCN.network.oldCompatibleCanContinue}</small>}
                {receiverOperationResult?.serial === serial && <small>{zhCN.network.recheckedVersion(receiverOperationResult.status.versionCode)}</small>}
                {updateCandidate && <small>{zhCN.network.availableVersion(updateCandidate.versionName, updateCandidate.versionCode, (updateCandidate.sizeBytes / (1024 * 1024)).toFixed(1))}</small>}
                {!updateCandidate && <small>{candidateLoading ? zhCN.network.checkingBundledVersion : updateCandidateError || zhCN.network.bundledPackageUnavailable}</small>}
                {updateCandidateError && updateCandidate && <small className="inline-error">{updateCandidateError}</small>}
              </div>
              <div className="receiver-status-actions setup-receiver-actions">
                <button className="dialog-secondary" type="button" disabled={busy} onClick={() => void checkReceiver(serial)}>{zhCN.network.recheck}</button>
                {canRequestUsbInstall && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{zhCN.network.installReceiver(updateCandidate.versionCode)}</button>}
                {setupWizard && !audioSessionActive && receiverNeedsUpgrade && updateCandidate && !receiverError && <button className="dialog-secondary" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{zhCN.network.optionalMaintenanceUpgrade(updateCandidate.versionCode)}</button>}
                {!setupWizard && canRequestUsbMaintenance && <button className="dialog-secondary" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{zhCN.network.optionalMaintenanceUpgrade(updateCandidate.versionCode)}</button>}
                {!setupWizard && canRequestUsbUpgrade && <button className="dialog-secondary" type="button" disabled={busy} onClick={() => requestUsbOperation("upgrade")}>{zhCN.network.optionalUpgrade(updateCandidate.versionCode)}</button>}
              </div>
            </div>
            {receiverError && <p ref={alertRef} className="inline-error receiver-error" role="alert" tabIndex={-1}>{receiverErrorMessage(receiverError)}</p>}
            {error && <p ref={alertRef} className="inline-error receiver-error" role="alert" tabIndex={-1}>{error}</p>}
            {canRequestUsbInstall && <p className="network-note">{zhCN.network.verifiedPackageInstallNote}</p>}
          </section> : <p className="network-recovery" role="status">{zhCN.network.goBackChooseUsb}</p>}
          <div className="setup-wizard-actions"><button className="dialog-secondary" type="button" disabled={busy} onClick={() => setUpStep(1)}>{zhCN.network.back}</button><button className="primary-button" type="button" disabled={busy || receiverStatus?.installed !== true || !!receiverError || !!error} onClick={async () => { const checkedSerial = serial; const checked = await checkReceiver(checkedSerial); if (checked?.installed === true && serialRef.current === checkedSerial) setUpStep(3); }}>{zhCN.network.next}</button></div>
        </div>}
        {setupStep === 3 && <div ref={setupStepRef} className="setup-wizard-step">
          <p>{zhCN.network.chooseMatchingInterfaces}</p>
          <label className="network-field">{zhCN.network.computerInterface}
            <select required value={localAddress} disabled={busy} onChange={event => { setLocalAddress(event.target.value); setLocalError(""); onError(""); }}>
              <option value="">{zhCN.network.chooseIpv4Interface}</option>
              {interfaces.map(value => <option key={`${value.index}-${value.address}`} value={value.address}>{value.name} · {value.address}/{value.prefixLength}</option>)}
            </select>
          </label>
          <label className="network-field">{zhCN.network.phoneCurrentIpv4}
            <select required value={receiverAddress} disabled={busy || receiverStatus?.installed !== true || !localAddress} onChange={event => { setReceiverAddress(event.target.value); setLocalError(""); onError(""); }}>
              <option value="">{zhCN.network.choosePhoneInterface}</option>
              {compatibleReceiverInterfaces.map(value => <option key={`${value.interfaceIndex}-${value.address}`} value={value.address}>{value.interfaceName} · {value.address}/{value.prefixLength}</option>)}
            </select>
          </label>
          {interfaces.length > 0 && receiverInterfaces.length > 0 && validBootstrapPairs.length === 0 && <div className="network-recovery" role="status"><span>{zhCN.network.noSameSubnet}</span></div>}
          {(interfaces.length === 0 || receiverInterfaces.length === 0) && <p className="network-note">{zhCN.network.requiresComputerAndPhoneIp}</p>}
          <button type="button" className="text-button network-refresh" disabled={busy || !serial} onClick={() => { void Promise.all([loadNetworkInfo(), checkReceiver(serial)]); }}>{zhCN.network.recheckNetworkInterfaces}</button>
          {shownError && <p id="network-flow-error" ref={alertRef} className="inline-error network-error" role="alert" tabIndex={-1}>{receiverErrorMessage(shownError)}</p>}
          <div className="setup-wizard-actions"><button className="dialog-secondary" type="button" disabled={busy} onClick={() => setUpStep(2)}>{zhCN.network.back}</button><button className="primary-button" type="submit" disabled={busy || !canStart}>{zhCN.network.bindAndConnect}</button></div>
        </div>}
      </section>}
      {!setupWizard && <>
      <div className="network-primary">
        <div className="network-summary"><strong>{mainSummary}</strong><span>{addressSummary}</span></div>
        {receiverManagerOnly && <section className="receiver-manager receiver-usb-card" aria-label={zhCN.network.receiverUsbManagementLabel}>
          <div className="receiver-status-row">
            <div className="receiver-status-copy" role="status" aria-live="polite">
              <strong>{candidateLoading ? zhCN.network.checkingLocalPackage : updateCandidate ? zhCN.network.checkAndUpgradeUsb : zhCN.network.localPackageUnavailable}</strong>
              {updateCandidate && <small>{zhCN.network.managerVersionSummary(updateCandidate.versionName, currentVersionLabel, updateCandidate.versionCode, (updateCandidate.sizeBytes / (1024 * 1024)).toFixed(1))}</small>}
              {!updateCandidate && <small>{updateCandidateError || zhCN.network.connectAuthorizedUsbToCheck}</small>}
              {receiverAlreadyCurrent && <small>{zhCN.network.currentVersionIsTarget}</small>}
              {receiverNeedsUpgrade && <small>{snapshot.sessionTransport === "lan"
                ? bindingReference ? zhCN.network.selectedBindingAndUsbIdentity : zhCN.network.chooseSessionBinding
                : zhCN.network.oldReceiverMaintenance}</small>}
              {receiverOperationResult && <small>{zhCN.network.usbOperationVersionVerified(receiverOperationResult.serial, receiverOperationResult.status.versionCode)}</small>}
              {usbDevices.length === 0 && <small>{zhCN.network.connectAndAuthorizeToRefresh}</small>}
              {updateCandidateError && updateCandidate && <small className="inline-error">{updateCandidateError}</small>}
            </div>
            <div className="receiver-status-actions">
              {canRequestUsbInstall && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{zhCN.network.installReceiver(updateCandidate.versionCode)}</button>}
              {canRequestUsbMaintenance && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{zhCN.network.usbMaintenanceInstall(updateCandidate.versionCode)}</button>}
              {canRequestUsbUpgrade && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("upgrade")}>{zhCN.network.upgradeReceiver(updateCandidate.versionCode)}</button>}
              <button className="dialog-secondary" type="button" disabled={busy} onClick={onRefreshDevices}>{zhCN.network.refreshUsbDeviceList}</button>
            </div>
          </div>
          {receiverNeedsUpgrade && bindings.length > 0 && <label className="network-field">{zhCN.network.optionalSavedBinding}
              <select value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); setLocalError(""); onError(""); }}>
              <option value="">{zhCN.network.noBindingUsbMaintenance}</option>
              {bindings.map(value => <option key={value} value={value}>{bindingLabel(value)}</option>)}
            </select>
          </label>}
          {receiverNeedsUpgrade && !bindingReference && <small>{zhCN.network.noBindingMaintenanceNote}</small>}
          <section className="binding-reset" aria-label={zhCN.network.resetUsbPairing}>
            <div className="binding-reset-heading"><strong>{zhCN.network.resetPairing}</strong><small>{zhCN.network.resetOnlyCurrentUsb}</small></div>
            {!bindingResetPreview && !bindingResetResult && <button className="dialog-secondary" type="button" disabled={busy || !serial} onClick={() => void previewBindingReset(serial)}>
              {bindingResetLoading ? zhCN.network.readingResetPreview : zhCN.network.resetPairingUsbRequired}
            </button>}
            {bindingResetLoading && <p className="network-loading" role="status"><LoaderCircle className="spin" size={15} />{zhCN.network.readOnlyUsbBindingCheck}</p>}
            {bindingResetError && <div className="binding-reset-error" role="alert"><p>{receiverErrorMessage(bindingResetError)}</p><button className="dialog-secondary" type="button" disabled={busy || !serial} onClick={() => void previewBindingReset(serial, true)}>{zhCN.network.recheck}</button></div>}
            {bindingResetPreview && <div className="binding-reset-preview">
              <strong>{zhCN.network.readOnlyPreview} · {bindingResetPreview.serial}</strong>
              <dl>
                <div><dt>{zhCN.network.androidUser}</dt><dd>{bindingResetPreview.androidUser}</dd></div>
                <div><dt>{zhCN.network.bindingReference}</dt><dd>{bindingResetPreview.bindingReference}</dd></div>
                <div><dt>{zhCN.network.phoneRecord}</dt><dd>{resetPhoneStateLabel(bindingResetPreview.phoneState)} · {commitmentSummary(bindingResetPreview.phoneCommitment)}</dd></div>
                <div><dt>{zhCN.network.pcRecord}</dt><dd>{resetPcStateLabel(bindingResetPreview.pcState)} · {commitmentSummary(bindingResetPreview.pcCommitment)}</dd></div>
              </dl>
              {bindingResetPreview.phoneState === "saved" && bindingResetPreview.pcState === "saved" && bindingResetPreview.phoneCommitment !== bindingResetPreview.pcCommitment
                ? <p className="binding-reset-warning" role="alert">{zhCN.network.differentBindingSummaries}</p>
                : <p className="binding-reset-warning">{zhCN.network.resetConfirmationDetail}</p>}
              {bindingResetPreview.receiverBusy && <p className="binding-reset-warning" role="alert">{zhCN.network.receiverHasActiveSession}</p>}
              {(bindingResetPreview.phoneState === "pending" || bindingResetPreview.phoneState === "failed" || bindingResetPreview.phoneState === "unknown" || bindingResetPreview.pcState === "unknown") && <p className="binding-reset-warning" role="alert">{zhCN.network.resetUnknownState}</p>}
              <div className="binding-reset-actions">
                <button className="dialog-secondary" type="button" disabled={busy} onClick={() => void previewBindingReset(serial, true)}>{zhCN.network.recheck}</button>
                <button className="dialog-secondary" type="button" disabled={busy} onClick={invalidateBindingReset}>{zhCN.network.cancel}</button>
                <button className="primary-button binding-reset-confirm" type="button" disabled={busy || bindingResetPreview.receiverBusy || !["empty", "saved"].includes(bindingResetPreview.phoneState) || !["missing", "saved"].includes(bindingResetPreview.pcState)} onClick={() => void confirmBindingReset()}>{zhCN.network.confirmResetTarget}</button>
              </div>
            </div>}
            {bindingResetBusy && <p className="network-loading" role="status"><LoaderCircle className="spin" size={15} />{zhCN.network.resettingAndRefreshing}</p>}
            {bindingResetResult && <div className={`binding-reset-result ${bindingResetResult.outcome === "reset" || bindingResetResult.outcome === "alreadyEmpty" ? "is-complete" : "is-partial"}`} role={bindingResetResult.outcome === "reset" || bindingResetResult.outcome === "alreadyEmpty" ? "status" : "alert"}>
              <strong>{resetOutcomeLabel(bindingResetResult.outcome)}</strong>
              <dl>
                <div><dt>{zhCN.network.phoneRecord}</dt><dd>{resetPhoneStateLabel(bindingResetResult.phoneState)} · {commitmentSummary(bindingResetResult.phoneCommitment)}</dd></div>
                <div><dt>{zhCN.network.pcRecord}</dt><dd>{resetPcStateLabel(bindingResetResult.pcState)} · {commitmentSummary(bindingResetResult.pcCommitment)}</dd></div>
              </dl>
              <p>{bindingResetResult.reason}</p><p>{bindingResetResult.nextAction}</p>
              <button className="dialog-secondary" type="button" disabled={busy || !serial} onClick={() => void previewBindingReset(serial, true)}>{zhCN.network.recheck}</button>
            </div>}
          </section>
        </section>}
        {pairedServiceStopped && <div className="network-recovery" role="status">
          <span>{zhCN.network.receiverServiceStopped}</span>
          <button type="button" className="text-button" disabled={busy} onClick={() => { methodManuallySelected.current = true; setMethod("bootstrap"); setAdvancedOpen(true); }}>{zhCN.network.startSelectedPhoneOverUsb}</button>
        </div>}
        {shownError && <p id="network-flow-error" ref={alertRef} className="inline-error network-error" role="alert" tabIndex={-1}>{receiverErrorMessage(shownError)}</p>}
        {busy && <p className="network-loading" role="status" aria-live="polite"><LoaderCircle className="spin" size={15} />{receiverOperationBusy ? zhCN.network.usbOperationVerifying : receiverAction === "checking" ? zhCN.network.checkingReceiver : candidateLoading ? zhCN.network.checkingBundledVersion : zhCN.network.readingNetworkInfo}</p>}
        {!receiverManagerOnly && udpMode && <section className="receiver-manager offer-manager" aria-label={zhCN.network.offerListenerLabel}>
          <div className="receiver-status-row">
            <div className="receiver-status-copy" role="status" aria-live="polite">
              <strong>{offers.active ? zhCN.network.listeningAt(offers.interfaceAddress ?? zhCN.network.networkInterface) : zhCN.network.allowBoundPhone}</strong>
              <small>{offers.active
                ? offers.offers.length > 1 ? zhCN.network.multipleRequests : offers.automaticBinding ? zhCN.network.autoAuthenticateBinding(bindingLabel(offers.automaticBinding)) : zhCN.network.autoAuthDisabled
                : bindings.length === 0 ? zhCN.network.firstUseBindOverUsb : zhCN.network.phoneCanStartOneTap}</small>
            </div>
            {!offers.active && <button className="dialog-secondary" type="button" disabled={busy || audioSessionActive || !localAddress || !bindingReference} onClick={() => void onSetOfferListener(localAddress, true, bindingReference || undefined)}>{zhCN.network.startListening}</button>}
          </div>
          {offers.active && (!offers.automaticBinding || offers.offers.length > 1) && <>
            {offers.offers.length > 0
              ? <div className="candidate-list" aria-label={zhCN.network.unverifiedPhoneRequests}>
                  {offers.offers.map(offer => <button type="button" key={offer.id} disabled={busy || audioSessionActive} aria-pressed={selectedOfferId === offer.id} onClick={() => setSelectedOfferId(offer.id)}>
                    <span>{offer.endpoint}</span><small>{zhCN.network.unverifiedPhoneRequest(selectedOfferId === offer.id)}</small>
                  </button>)}
                </div>
              : <p className="network-hint">{zhCN.network.waitPhoneReceive}</p>}
            <p className="network-hint">{bindings.length === 0 ? zhCN.network.noSavedBinding
              : bindingReference ? zhCN.network.useBindingWhenAccepted(bindingLabel(bindingReference))
                : zhCN.network.multipleSavedBindings}</p>
            {offers.offers.length > 1 && !selectedOfferId && <p className="network-hint">{zhCN.network.chooseRequestToAccept}</p>}
            <button className="primary-button" type="button" disabled={busy || audioSessionActive || !selectedOfferId || !bindingReference || !offers.offers.some(offer => offer.id === selectedOfferId)} onClick={() => onAcceptOffer(selectedOfferId, bindingReference)}>{zhCN.network.acceptSelectedPhone}</button>
            {audioSessionActive && <p className="network-hint">{zhCN.network.cannotAcceptDuringAudio}</p>}
          </>}
        </section>}
      </div>
      <details className="network-advanced" open={receiverManagerOnly ? advancedOpen : advancedOpen || advancedRequired} onToggle={event => { if (receiverManagerOnly || !advancedRequired) setAdvancedOpen(event.currentTarget.open); }}>
        <summary>{receiverManagerOnly ? zhCN.network.receiverStatusAndInstall : zhCN.network.connectionOptions}</summary>
        <div className="network-advanced-body">
          {!receiverManagerOnly && <p className="network-intro">{zhCN.network.addressHintNeedsAuth}</p>}
          {!receiverManagerOnly && <div className="network-methods" role="group" aria-label={zhCN.network.receiverConnectionMode}>
            <button type="button" aria-pressed={method === "bootstrap"} disabled={busy} onClick={() => { methodManuallySelected.current = true; setMethod("bootstrap"); onError(""); setLocalError(""); }}>{zhCN.network.usbFirstBinding}</button>
            <button type="button" aria-pressed={method === "paired"} disabled={busy || bindings.length === 0} onClick={() => { methodManuallySelected.current = true; setMethod("paired"); onError(""); setLocalError(""); }}>{zhCN.network.connectPairedPhone}</button>
          </div>}
          {!receiverManagerOnly && <label className="network-field">{zhCN.network.computerInterface}
            <select required value={localAddress} disabled={busy || offers.active} onChange={event => { setLocalAddress(event.target.value); setCandidates([]); if (!peerManuallyEdited.current) setPeer(""); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !localAddress || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
              <option value="">{zhCN.network.chooseIpv4Interface}</option>
              {interfaces.map(value => <option key={`${value.index}-${value.address}`} value={value.address}>{value.name} · {value.address}/{value.prefixLength}</option>)}
            </select>
          </label>}
          {method === "bootstrap" ? <>
            <label className="network-field">{zhCN.network.authorizedOnlineUsb}
              <select required value={serial} disabled={busy} onChange={event => { invalidateBindingReset(); setSerial(event.target.value); onReceiverSerialChange(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !serial || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
                <option value="">{zhCN.network.chooseUsbPlaceholder}</option>
                {usbDevices.map(device => <option key={device.serial} value={device.serial}>{device.displayName}</option>)}
              </select>
            </label>
            {receiverManagerOnly && usbDevices.length === 0 && <div className="network-recovery" role="status"><span>{zhCN.network.connectUsbBeforeCheck}</span><button type="button" className="text-button" disabled={busy} onClick={onRefreshDevices}>{zhCN.network.refreshDevices}</button></div>}
            {serial && <section className="receiver-manager" aria-label={zhCN.network.receiverManagement}>
              <div className="receiver-status-row">
                <div className="receiver-status-copy" role="status" aria-live="polite">
                  <strong>{receiverOperationBusy ? zhCN.network.installingAndVerifying : receiverAction === "checking" ? zhCN.network.checkingReceiver : receiverError ? zhCN.network.receiverCheckFailed : !receiverStatus ? zhCN.network.waitingReceiverCheck : !receiverStatus.installed ? zhCN.network.receiverNotInstalled : receiverStatus.serviceActive ? receiverStatus.healthy ? zhCN.network.receiverHealthy : zhCN.network.receiverUnhealthy : zhCN.network.receiverStopped}</strong>
                  {receiverStatus?.installed && <small>{[receiverStatus.versionName, receiverStatus.versionCode === null ? "" : zhCN.network.versionCode(receiverStatus.versionCode), receiverStatus.foreground ? zhCN.network.foregroundService : ""].filter(Boolean).join(" · ")}</small>}
                  {receiverStatus?.audioActive && <small>{zhCN.network.audioActiveNotPlaying}</small>}
                  {receiverStatus && !receiverStatus.healthy && <small>{receiverStatus.error || receiverStatus.message}</small>}
                  {!receiverManagerOnly && receiverNeedsUpgrade && <small>{zhCN.network.oldCompatibleUsbUpgrade}</small>}
                </div>
                <div className="receiver-status-actions">
                  <button className="dialog-secondary" type="button" disabled={busy} onClick={() => void checkReceiver(serial)}>{receiverAction === "checking" ? zhCN.network.checking : zhCN.network.checkStatus}</button>
                  {!receiverManagerOnly && canRequestUsbInstall && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{zhCN.network.installReceiver(updateCandidate.versionCode)}</button>}
                  {!receiverManagerOnly && receiverNeedsUpgrade && <button className="primary-button" type="button" disabled={busy} onClick={onOpenReceiverManager}>{zhCN.network.usbUpgradeOptions}</button>}
                </div>
              </div>
              {receiverError && <p ref={alertRef} className="inline-error receiver-error" role="alert" tabIndex={-1}>{receiverErrorMessage(receiverError)}</p>}
              {!receiverManagerOnly && receiverNeedsInstall && <p className="network-note">{zhCN.network.installPackageFirstVerify}</p>}
              {!receiverManagerOnly && receiverNeedsUpgrade && <button type="button" className="text-button" disabled={busy} onClick={onOpenReceiverManager}>{zhCN.network.openUsbUpgradeManager}</button>}
            </section>}
            {!receiverManagerOnly && <label className="network-field">{zhCN.network.phoneCurrentIpv4}
              <select required value={receiverAddress} disabled={busy || !serial || receiverStatus?.installed !== true} onChange={event => { setReceiverAddress(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !receiverAddress || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
                <option value="">{zhCN.network.choosePhoneInterface}</option>
                {compatibleReceiverInterfaces.map(value => <option key={`${value.interfaceIndex}-${value.address}`} value={value.address}>{value.interfaceName} · {value.address}/{value.prefixLength}</option>)}
              </select>
            </label>}
            {!receiverManagerOnly && <p className="network-note">{zhCN.network.onlySameSubnetPhoneAddresses}</p>}
          </> : <>
            <label className="network-field">{zhCN.network.pairedReceiver}
              <select required value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !bindingReference || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
                <option value="">{zhCN.network.chooseBinding}</option>
                {bindings.map(value => <option key={value} value={value}>{bindingLabel(value)}</option>)}
              </select>
            </label>
            <label className="network-field">{zhCN.network.receiverIpv4Endpoint}
              <input required value={peer} disabled={busy} onChange={event => { peerManuallyEdited.current = true; setPeer(event.target.value); setLocalError(""); onError(""); }} placeholder="192.0.2.20:45678" autoComplete="off" inputMode="decimal" aria-invalid={!!shownError && !isIpv4Endpoint(peer) || peerOutsideSubnet || undefined} aria-describedby={`network-peer-hint${peerOutsideSubnet ? " network-peer-network-warning" : ""}${shownError ? " network-flow-error" : ""}`} />
            </label>
            <small id="network-peer-hint" className="network-hint">{zhCN.network.addressSuggestionNeedsConnection}</small>
            {peerOutsideSubnet && <small id="network-peer-network-warning" className="network-validation-warning">{zhCN.network.invalidSubnetConnectDisabled}</small>}
            <p className="network-hint" role="status" aria-live="polite">{addressNotice}</p>
            <button className="text-button network-refresh" type="button" onClick={() => void discoverCandidates()} disabled={busy || !localAddress}><Search size={15} />{zhCN.network.searchAddressAgain}</button>
            {candidates.length > 0 && <div className="candidate-list" aria-label={zhCN.network.unverifiedDiscoveryCandidates}>{candidates.map(candidate => <button type="button" key={candidate.endpoint} disabled={busy} aria-pressed={peer === candidate.endpoint} onClick={() => { peerManuallyEdited.current = true; setPeer(candidate.endpoint); setLocalError(""); onError(""); }}><span>{candidate.endpoint}</span><small>{zhCN.network.addressOnlyUnverified}</small></button>)}</div>}
          </>}
          {!receiverManagerOnly && <button className="text-button network-refresh" type="button" onClick={() => void loadNetworkInfo()} disabled={busy}><Search size={15} />{zhCN.network.refreshInterfacesAndBindings}</button>}
          {!receiverManagerOnly && <button className="primary-button network-connect" disabled={busy || !canStart}>{disabled ? zhCN.network.processing : method === "bootstrap" ? zhCN.network.firstBindAndConnect : zhCN.network.connectReceiverDirectly}</button>}
        </div>
      </details>
      </>}
      {!receiverManagerOnly && needsReceiverUpdate([shownError, receiverError, bindingResetError].join(" ")) && <button className="primary-button" type="button" disabled={busy} onClick={onOpenReceiverManager}>{zhCN.app.upgradePhone}</button>}
    </form>
  </Modal>;
}

function DeviceDialog({ devices, selected, activeSerial, activeBinding, busy, onSelect, onRefresh, onClose }: { devices: Device[]; selected: string | null; activeSerial: string | null; activeBinding: string | null; busy: boolean; onSelect: (device: Device) => void; onRefresh: () => void; onClose: () => void }) {
  return <Modal title={zhCN.deviceDialog.title} onClose={onClose} className="device-dialog">
    <div className="dialog-tools"><span>{zhCN.deviceDialog.onlyCurrentTransport}</span><button className="text-button" onClick={onRefresh} disabled={busy}><Search size={15} />{zhCN.deviceDialog.refresh}</button></div>
    {activeBinding && <div className="network-active-card"><strong>{bindingLabel(activeBinding)}</strong><span>{zhCN.deviceDialog.activeDirectUdp}</span></div>}
    <div className="device-list">{devices.map(device => { const connected = activeSerial === device.serial; const chosen = (activeSerial ?? selected) === device.serial; return <button className="device-option" key={device.serial} onClick={() => onSelect(device)} disabled={busy || (!connected && !device.connectable)} aria-pressed={chosen}>
      <Smartphone size={18} aria-hidden="true" /><span><strong>{device.displayName}</strong><small>{zhCN.deviceDialog.deviceDetails(connected, device.statusText, device.transport, device.androidVersion)}</small></span>{chosen && <Check size={17} aria-hidden="true" />}
    </button>; })}</div>
    {devices.length === 0 && <div className="empty-note"><Info size={17} /><span>{activeBinding ? zhCN.deviceDialog.emptyForBinding : zhCN.deviceDialog.empty}</span></div>}
    <button className="dialog-secondary" onClick={onClose}>{zhCN.deviceDialog.finish}</button>
  </Modal>;
}

function SettingsDialog({ snapshot, largeText, onLargeText, udpBackendFixed, busy, onSetOption, onOpenReceiverManager, onSave, onClose }: { snapshot: Snapshot; largeText: boolean; onLargeText: (value: boolean) => void; udpBackendFixed: boolean; busy: boolean; onSetOption: (kind: "wirelessProtocol" | "audioBackend", value: string) => void; onOpenReceiverManager: () => void; onSave: (key: string, value: unknown) => void; onClose: () => void }) {
  const settings = snapshot.settings;
  const update = (key: keyof Settings, value: boolean | string) => { void onSave(key, value); };
  return <Modal title={zhCN.app.settingsTitle} onClose={onClose} className="settings-dialog">
    <h3>{zhCN.app.settingsConnection}</h3>
    <SettingToggle disabled={snapshot.busy} title={zhCN.app.autoConnect} description={zhCN.app.autoConnectDescription} checked={settings.autoConnectLastDevice} onChange={value => update("autoConnectLastDevice", value)} />
    <SettingToggle disabled={snapshot.busy} title={zhCN.app.wirelessRecovery} description={zhCN.app.wirelessRecoveryDescription} checked={settings.autoReconnectWifi} onChange={value => update("autoReconnectWifi", value)} />
    <h3>{zhCN.app.advancedConnection}</h3>
    <label className="select-setting"><span>{zhCN.app.wirelessProtocol}</span><select aria-label={zhCN.app.wirelessProtocol} disabled={busy} value={settings.wirelessProtocol} onChange={event => onSetOption("wirelessProtocol", event.target.value)}><option value="udp">{zhCN.app.udpRecommended}</option><option value="adb">{zhCN.app.wirelessAdbCompatible}</option></select></label>
    <label className="select-setting"><span>{zhCN.app.receiverBackend}</span><select aria-label={zhCN.app.receiverBackend} disabled={busy || udpBackendFixed} value={settings.audioBackend} onChange={event => onSetOption("audioBackend", event.target.value)}><option value="legacy">{zhCN.app.legacyCompatible}</option><option value="receiver">Receiver</option></select></label>
    {settings.wirelessProtocol === "adb" && <p className="dialog-footnote">{zhCN.app.wirelessAdbHelp}</p>}
    {udpBackendFixed && <p className="dialog-footnote">{zhCN.app.udpUsesReceiver}</p>}
    <button className="text-button backend-manager-button" type="button" disabled={busy} onClick={onOpenReceiverManager}>{zhCN.app.receiverManager}</button>
    <h3>{zhCN.app.windowAppearance}</h3>
    <SettingToggle disabled={snapshot.busy} title={zhCN.app.minimizeToTray} description={zhCN.app.minimizeToTrayDescription} checked={settings.minimizeToTray} onChange={value => update("minimizeToTray", value)} />
    <SettingToggle title={zhCN.app.enlargeText} description={zhCN.app.enlargeTextDescription} checked={largeText} onChange={onLargeText} />
    <label className="select-setting"><span>{zhCN.app.theme}</span><select disabled={snapshot.busy} value={settings.appearanceTheme} onChange={event => update("appearanceTheme", event.target.value)}><option value="System">{zhCN.app.followSystem}</option><option value="Light">{zhCN.app.light}</option><option value="Dark">{zhCN.app.dark}</option></select></label>
    {snapshot.busy && <p className="dialog-footnote">{zhCN.app.settingsBusy}</p>}
    <p className="dialog-footnote">{zhCN.app.settingsPrivacy}</p>
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
  const [message, setMessage] = useState<string>(zhCN.app.generatingSecurePairingCode);
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
      else if (found.length > 1) setMessage(zhCN.pairDialog.foundPhones(found.length));
      else setMessage(zhCN.pairDialog.noPhoneFound);
    } catch (reason) { if (generation === taskGeneration.current) setError(String(reason)); }
    finally { if (generation === taskGeneration.current) setBusy(false); }
  };
  return <Modal title={transport === "wifi" ? zhCN.pairDialog.wirelessTitle : zhCN.pairDialog.usbTitle} onClose={onClose} className="pair-dialog">
    {transport === "wifi" ? <>
      <div className="tabs" role="tablist" aria-label={zhCN.pairDialog.addMethod}><button role="tab" aria-selected={tab === "qr"} disabled={disabled || busy} onClick={() => setTab("qr")}>{zhCN.pairDialog.qrTab}</button><button role="tab" aria-selected={tab === "code"} disabled={disabled || busy} onClick={() => setTab("code")}>{zhCN.pairDialog.codeTab}</button><button role="tab" aria-selected={tab === "address"} disabled={disabled || busy} onClick={() => setTab("address")}>{zhCN.pairDialog.addressTab}</button></div>
      {tab === "qr" && <div className="qr-pane">
        {credentials ? <div className="qr-frame"><QRCodeSVG value={credentials.payload} size={174} level="Q" includeMargin /></div> : <div className="qr-frame qr-wait"><LoaderCircle className="spin" /></div>}
        <p className="qr-help">{zhCN.pairDialog.qrInstructions}</p>
        <p className="pair-progress" role="status" aria-live="polite">{message}</p>
        {!qrStarted && <button className="primary-button" disabled={disabled || !credentials || busy} onClick={() => {
          if (!credentials) return;
          const generation = currentTask();
          setQrStarted(true); setBusy(true);
          void invokeApp<{ success: boolean; message: string }>("pair_qr", { instanceName: credentials.instanceName, password: credentials.password })
            .then(async result => { if (generation !== taskGeneration.current) return; setMessage(result.message); if (result.success) await onDone(); else { setError(result.message); setQrStarted(false); setCredentials(null); void invokeApp<Credentials>("qr_credentials").then(value => { if (generation === taskGeneration.current) setCredentials(value); }); } })
            .catch(reason => { if (generation === taskGeneration.current) { setError(String(reason)); setQrStarted(false); } }).finally(() => { if (generation === taskGeneration.current) setBusy(false); });
        }}>{busy ? zhCN.pairDialog.waitingForScan : zhCN.pairDialog.startQrPairing}</button>}
        {qrStarted && <button className="dialog-secondary" onClick={() => { ++taskGeneration.current; void invokeApp("cancel_qr_pairing"); setQrStarted(false); setBusy(false); setMessage(zhCN.pairDialog.pairingCancelled); }}>{zhCN.pairDialog.cancelPairing}</button>}
      </div>}
      {tab === "code" && <form className="form-pane" onSubmit={event => { event.preventDefault(); void finish("pair_wifi", { endpoint, code }); }}>
        <label>{zhCN.pairDialog.pairingAddress}<input disabled={disabled || busy} value={endpoint} onChange={event => setEndpoint(event.target.value)} autoComplete="off" placeholder={zhCN.pairDialog.pairingAddressPlaceholder} /></label>
        <label>{zhCN.pairDialog.sixDigitCode}<input disabled={disabled || busy} value={code} onChange={event => setCode(event.target.value.replace(/\D/g, "").slice(0, 6))} inputMode="numeric" autoComplete="one-time-code" aria-describedby="pair-code-hint" placeholder={zhCN.pairDialog.codePlaceholder} /></label>
        <small id="pair-code-hint">{zhCN.pairDialog.codeHint}</small>
        <button className="primary-button" disabled={disabled || busy || code.length !== 6}>{busy ? zhCN.pairDialog.pairing : zhCN.pairDialog.pairAndAdd}</button>
      </form>}
      {tab === "address" && <form className="form-pane" onSubmit={event => { event.preventDefault(); void finish("connect_wifi", { endpoint }); }}>
        <label>{zhCN.pairDialog.wirelessDebugAddress}<input disabled={disabled || busy} value={endpoint} onChange={event => setEndpoint(event.target.value)} autoComplete="off" placeholder={zhCN.pairDialog.connectionAddressPlaceholder} /></label>
        <small>{zhCN.pairDialog.connectionPortHint}</small>
        <button className="text-button discover-button" type="button" onClick={() => void discover()} disabled={disabled || busy}><Search size={15} />{zhCN.pairDialog.searchLanDevices}</button>
        <button className="primary-button" disabled={disabled || busy || !endpoint.trim()}>{busy ? zhCN.pairDialog.connecting : zhCN.pairDialog.connectAddress}</button>
      </form>}
    </> : <div className="help-pane"><p>{zhCN.pairDialog.usbStep1}</p><p>{zhCN.pairDialog.usbStep2}</p><p>{zhCN.pairDialog.usbStep3}</p><button className="primary-button" onClick={async () => { await onDone(); }}>{zhCN.pairDialog.refreshDeviceList}</button></div>}
    {disabled && <p className="dialog-footnote" role="status">{zhCN.pairDialog.processingConnection}</p>}
    {error && <p className="inline-error" role="alert">{error}</p>}
  </Modal>;
}

function HelpDialog({ transport, wirelessProtocol, onClose }: { transport: Snapshot["connectionTransport"]; wirelessProtocol: Snapshot["sessionWirelessProtocol"]; onClose: () => void }) {
  return <Modal title={zhCN.helpDialog.title} onClose={onClose} className="help-dialog">
    {transport === "usb" ? <><h3>{zhCN.helpDialog.usbTitle}</h3><ol><li>{zhCN.helpDialog.usbStep1}</li><li>{zhCN.helpDialog.usbStep2}</li></ol></>
      : wirelessProtocol === "udp" ? <><h3>{zhCN.helpDialog.udpTitle}</h3><ol><li>{zhCN.helpDialog.udpStep1}</li><li>{zhCN.helpDialog.udpStep2}</li><li>{zhCN.helpDialog.udpStep3}</li></ol></>
        : <><h3>{zhCN.helpDialog.adbTitle}</h3><ol><li>{zhCN.helpDialog.adbStep1}</li><li>{zhCN.helpDialog.adbStep2}</li></ol></>}
    <p>{transport === "usb" ? zhCN.helpDialog.usbPlaybackNote
      : wirelessProtocol === "udp" ? zhCN.helpDialog.receiverRestartNote
        : zhCN.helpDialog.adbPlaybackNote}</p><button className="dialog-secondary" onClick={onClose}>{zhCN.helpDialog.finish}</button>
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
  const number = (value: number | null) => value === null ? zhCN.diagnostics.unavailable : value.toLocaleString("zh-CN");
  const frames = (value: number | null, meaning: string) => value === null ? zhCN.diagnostics.unavailable : zhCN.diagnostics.frameValue(number(value), meaning);
  const playbackContext = !active ? zhCN.diagnostics.noPhoneSession
    : snapshot.output === "computer" ? zhCN.diagnostics.computerSnapshot
      : snapshot.state === "paused" ? zhCN.diagnostics.pausedSnapshot
        : diagnostic?.underrunIncrement !== null && diagnostic?.underrunIncrement !== undefined && diagnostic.underrunIncrement > 0
          ? zhCN.diagnostics.underrunDelta(number(diagnostic.underrunIncrement))
        : diagnostic?.playbackAdvancing === true ? zhCN.diagnostics.progressConfirmed
          : diagnostic?.playbackAdvancing === false ? zhCN.diagnostics.progressNotConfirmed
            : unsupportedUdpDiagnostics ? zhCN.diagnostics.unsupportedProgress
              : zhCN.diagnostics.missingProgress;
  const unavailable = !active ? zhCN.diagnostics.noSessionSnapshot
    : unsupportedUdpDiagnostics ? zhCN.diagnostics.unsupportedDiagnostics
      : zhCN.diagnostics.missingDiagnostics;
  const groups: { title: string; note?: string; metrics: [string, string][] }[] = diagnostic ? [
    { title: zhCN.diagnostics.actualAudioFormat, metrics: [
      [zhCN.diagnostics.pcSampleRate, diagnostic.sampleRate === null ? zhCN.diagnostics.unavailable : `${number(diagnostic.sampleRate)} Hz`],
      [zhCN.diagnostics.pcChannels, number(diagnostic.channels)],
      [zhCN.diagnostics.receiverSampleRate, diagnostic.receiverSampleRate === null ? zhCN.diagnostics.unavailable : `${number(diagnostic.receiverSampleRate)} Hz`],
      [zhCN.diagnostics.receiverChannels, number(diagnostic.receiverChannels)],
    ] },
    { title: zhCN.diagnostics.queues, note: zhCN.diagnostics.queueNote, metrics: [
      [zhCN.diagnostics.pcQueue, frames(diagnostic.pcQueuedFrames, zhCN.diagnostics.currentQueueMeaning)],
      [zhCN.diagnostics.pcDropped, frames(diagnostic.pcDroppedFrames, zhCN.diagnostics.sessionTotalMeaning)],
      [zhCN.diagnostics.udpJitterTarget, frames(diagnostic.udpJitterTargetFrames, zhCN.diagnostics.targetQueueMeaning)],
      [zhCN.diagnostics.udpQueue, frames(diagnostic.udpQueuedFrames, zhCN.diagnostics.currentQueueMeaning)],
      [zhCN.diagnostics.udpExpired, frames(diagnostic.udpExpiredFrames, zhCN.diagnostics.sessionTotalMeaning)],
      [zhCN.diagnostics.tcpDropped, frames(diagnostic.tcpDroppedBacklogFrames, zhCN.diagnostics.sessionTotalMeaning)],
    ] },
    { title: zhCN.diagnostics.receiverPlayback, note: zhCN.diagnostics.playbackNote, metrics: [
      [zhCN.diagnostics.audioTrackPending, frames(diagnostic.trackPendingFrames, zhCN.diagnostics.currentQueryMeaning)],
      [zhCN.diagnostics.underrunTotal, diagnostic.underrunCount === null ? zhCN.diagnostics.unavailable : `${number(diagnostic.underrunCount)} · ${zhCN.diagnostics.audioTrackCumulativeMeaning}`],
      [zhCN.diagnostics.underrunIncrement, diagnostic.underrunIncrement === null ? zhCN.diagnostics.unavailable : `${number(diagnostic.underrunIncrement)} · ${zhCN.diagnostics.sinceLastQueryMeaning}`],
      [zhCN.diagnostics.advancing, diagnostic.playbackAdvancing === null ? zhCN.diagnostics.unavailable : diagnostic.playbackAdvancing ? zhCN.diagnostics.yes : zhCN.diagnostics.no],
    ] },
    { title: zhCN.diagnostics.audioTrackBuffer, note: zhCN.diagnostics.bufferNote, metrics: [
      [zhCN.diagnostics.requested, frames(diagnostic.requestedBufferFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.accepted, frames(diagnostic.acceptedBufferFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.effective, frames(diagnostic.effectiveBufferFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.capacity, frames(diagnostic.capacityFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.startThreshold, frames(diagnostic.startThresholdFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.preRoll, frames(diagnostic.preRollFrames, zhCN.diagnostics.audioTrackMeaning)],
    ] },
  ] : [];
  const statusText = [
    `${zhCN.diagnostics.state}：${readableStatus(snapshot, snapshot.canDisconnect && snapshot.state === "error")}`,
    `${zhCN.diagnostics.serviceMessage}：${snapshot.message}`, `${zhCN.diagnostics.actualConnection}：${active ? actualConnectionLabel(snapshot) : zhCN.diagnostics.notConnected}`,
    `${zhCN.diagnostics.chosenConnection}：${intendedConnectionLabel(snapshot)}`, `${zhCN.diagnostics.receiverBackend}：${snapshot.settings.audioBackend}`,
    `${zhCN.diagnostics.device}：${snapshot.sessionReceiverBinding ? bindingLabel(snapshot.sessionReceiverBinding) : snapshot.deviceName ?? zhCN.diagnostics.none}`,
    `${zhCN.diagnostics.output}：${snapshot.output === "phone" ? zhCN.diagnostics.phone : zhCN.diagnostics.computer}`, `${zhCN.diagnostics.visibleDevices}：${snapshot.devices.length}`,
    `${zhCN.diagnostics.detail}：${snapshot.statusDetail}`,
    `${zhCN.diagnostics.capabilities}：${capabilities ? zhCN.diagnostics.protocolCompatibility(capabilities.controlMajor, capabilities.managementCompatible, capabilities.audioMajor, capabilities.audioCompatible, capabilities.capabilityBits.toString(16), capabilities.versionCode) : zhCN.diagnostics.unknownCapabilities}`,
    `${zhCN.diagnostics.version}：${snapshot.version}`,
  ].join("\n");
  const copyText = [
    `${zhCN.diagnostics.state}：${readableStatus(snapshot, snapshot.canDisconnect && snapshot.state === "error")}`,
    `${zhCN.diagnostics.actualConnection}：${active ? actualConnectionLabel(snapshot) : zhCN.diagnostics.notConnected}`,
    `${zhCN.diagnostics.playbackMode}：${playbackModeLabel(snapshot.settings.playbackMode)}`,
    `${zhCN.diagnostics.output}：${snapshot.output === "phone" ? zhCN.diagnostics.phone : zhCN.diagnostics.computer}`,
    `${zhCN.diagnostics.version}：${snapshot.version}`,
    `${zhCN.diagnostics.capabilitySummary}：${capabilities ? zhCN.diagnostics.protocolCompatibility(capabilities.controlMajor, capabilities.managementCompatible, capabilities.audioMajor, capabilities.audioCompatible, capabilities.capabilityBits.toString(16), capabilities.versionCode) : zhCN.diagnostics.unknownCapabilities}`,
    `${zhCN.diagnostics.playbackDiagnostics}：${playbackContext}`,
    ...groups.flatMap(group => [group.title, ...group.metrics.map(([label, value]) => `${label}：${value}`), ...(group.note ? [group.note] : [])]),
  ].join("\n");
  const metricGroup = (group: typeof groups[number]) => <section className="diagnostic-group" key={group.title}>
    <h3>{group.title}</h3>
    <dl>{group.metrics.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
    {group.note && <p>{group.note}</p>}
  </section>;
  return <Modal title={zhCN.diagnostics.dialogTitle} onClose={onClose} className="diagnostics-dialog">
    <section className="diagnostic-overview" aria-label={zhCN.diagnostics.currentState}>
      <dl><div><dt>{zhCN.diagnostics.state}</dt><dd>{readableStatus(snapshot, snapshot.canDisconnect && snapshot.state === "error")}</dd></div>
        <div><dt>{zhCN.diagnostics.actualConnection}</dt><dd>{active ? actualConnectionLabel(snapshot) : zhCN.diagnostics.notConnected}</dd></div>
        <div><dt>{zhCN.diagnostics.playbackMode}</dt><dd>{playbackModeLabel(snapshot.settings.playbackMode)}</dd></div></dl>
      <p className="diagnostic-context" role="status">{playbackContext}</p>
    </section>
    <details className="diagnostic-details">
      <summary>{zhCN.diagnostics.technicalDetails}</summary>
      <p>{zhCN.diagnostics.runState}</p><pre>{statusText}</pre>
      <section className="diagnostic-playback" aria-label={zhCN.diagnostics.playbackDiagnostics}>
        <h3>{zhCN.diagnostics.playbackDiagnostics}</h3>
        {diagnostic ? groups.map(metricGroup) : <p className="diagnostic-empty">{unavailable}</p>}
      </section>
      <div className="dialog-actions"><button className="dialog-secondary" onClick={() => void navigator.clipboard.writeText(copyText)}><Copy size={15} />{zhCN.diagnostics.copyDiagnostics}</button><button className="dialog-secondary" onClick={() => void invokeApp("open_logs")}>{zhCN.diagnostics.openLogs}<ExternalLink size={14} /></button></div>
    </details>
  </Modal>;
}

function ConfirmDialog({ title, body, confirmLabel, busy, onConfirm, onCancel }: { title: string; body: string; confirmLabel: string; busy: boolean; onConfirm: () => void; onCancel: () => void }) {
  return <Modal title={title} onClose={() => { if (!busy) onCancel(); }} className="confirm-dialog"><p>{body}</p><div className="dialog-actions"><button className="dialog-secondary" onClick={onCancel} disabled={busy}>{zhCN.confirmDialog.cancel}</button><button className="primary-button" onClick={onConfirm} disabled={busy}>{busy ? zhCN.confirmDialog.switching : confirmLabel}</button></div></Modal>;
}

export default App;
