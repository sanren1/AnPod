import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Activity, AudioLines, ChevronDown, CircleHelp, Settings2, Smartphone, Usb, Wifi } from "lucide-react";
import { strings } from "./strings";
import type { Device, Snapshot, PrivateIpv4Interface, ReceiverStatus, NetworkCandidate, ReceiverUsbOperation, ReceiverUsbOperationResult, ReceiverInstallProgress, HostDiscoverySnapshot, NetworkStartRequest, OperationResult } from "./appTypes";
import { isDesktop, invokeApp, byTransport, hasActiveSession, bindingLabel, actualConnectionLabel, readableStatus, needsReceiverUpdate, windowsRateMismatchHz, endpointFitsInterface, CAP_AOA_SESSION_RESTART } from "./appRuntime";
import { NetworkReceiverDialog } from "./components/NetworkReceiverDialog";
import { DeviceDialog } from "./components/DeviceDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { PairDialog } from "./components/PairDialog";
import { HelpDialog } from "./components/HelpDialog";
import { DiagnosticsDialog } from "./components/DiagnosticsDialog";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { AoaDialog } from "./components/AoaDialog";
import { ErrorDetails } from "./components/Modal";

function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [panel, setPanel] = useState<"device" | "settings" | "pair" | "network" | "receiver" | "setup" | "aoa" | "help" | "diagnostics" | null>(null);
  const [confirm, setConfirm] = useState<{ kind: "transport" | "wirelessProtocol" | "audioBackend" | "playback" | "wiredMode" | "device" | "network" | "receiverUsb" | "aoa"; value: string } | null>(null);
  const [pendingNetworkStart, setPendingNetworkStart] = useState<NetworkStartRequest | null>(null);
  const [pendingReceiverOperation, setPendingReceiverOperation] = useState<ReceiverUsbOperation | null>(null);
  const [receiverOperationBusy, setReceiverOperationBusy] = useState(false);
  const [receiverManagerSerial, setReceiverManagerSerial] = useState("");
  const [receiverOperationResult, setReceiverOperationResult] = useState<ReceiverUsbOperationResult | null>(null);
  const [receiverOperationProgress, setReceiverOperationProgress] = useState<ReceiverInstallProgress | null>(null);
  const receiverOperationLock = useRef(false);
  const receiverConfirmationStarted = useRef(false);
  const appMounted = useRef(true);
  const receiverOperationUnlisten = useRef<(() => void) | null>(null);
  const aoaConfigurationActive = useRef<number | null>(null);
  const aoaConfigurationOpening = useRef(false);
  const aoaConfigurationClosing = useRef(false);
  const panelRef = useRef(panel);
  panelRef.current = panel;
  const [networkDialogError, setNetworkDialogError] = useState("");
  const [backgroundStartMode, setBackgroundStartMode] = useState(false);
  const [networkRecoveryMode, setNetworkRecoveryMode] = useState(false);
  const [backgroundChoicePrefill, setBackgroundChoicePrefill] = useState({ localAddress: "", bindingReference: "" });
  const [networkChoiceNeedsConfirmation, setNetworkChoiceNeedsConfirmation] = useState({ interfaceAddress: false, bindingReference: false });
  const [localSelected, setLocalSelected] = useState<string | null>(null);
  const [largeText, setLargeText] = useState(false);
  const [notice, setNotice] = useState("");
  const [noticeDetail, setNoticeDetail] = useState("");
  const [working, setWorking] = useState(false);
  const [cancelPending, setCancelPending] = useState(false);
  const cancelOperationLock = useRef(false);

  useEffect(() => {
    appMounted.current = true;
    return () => {
      appMounted.current = false;
      receiverOperationUnlisten.current?.();
      receiverOperationUnlisten.current = null;
      const configurationId = aoaConfigurationActive.current;
      aoaConfigurationActive.current = null;
      if (configurationId !== null) void invokeApp("end_usb_direct_configuration", { configurationId });
    };
  }, []);
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
      const pairing = await listen<string>("anpod://pairing-progress", event => { setNotice(event.payload); setNoticeDetail(""); });
      retain(pairing);
    };
    void attach().catch(error => { if (!disposed) { setNotice(strings.app.operationNotCompleted); setNoticeDetail(String(error)); } });
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
  const aoaSession = active && snapshot?.sessionTransport === "aoa";
  const lanSession = active && snapshot?.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "udp";
  const wiredMode = snapshot?.settings.wiredMode ?? "adb";
  const wirelessUdpMode = !!snapshot && snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp";
  const udpBackendFixed = !!snapshot && ((snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp")
    || (active && snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "udp")
    || aoaSession);
  const canDisconnect = !!snapshot?.canDisconnect;
  const cleanupPending = canDisconnect && snapshot?.state === "error";
  const receiverUpdateNeeded = snapshot?.state === "error" && needsReceiverUpdate(`${snapshot.message} ${snapshot.statusDetail}`)
    || snapshot?.state === "managementOnly" && snapshot.sessionCapabilities?.audioCompatible === false;
  const controlsDisabled = working || snapshot?.busy === true || cleanupPending || receiverOperationBusy;
  const aoaPlaybackRestartUnavailable = aoaSession && !controlsDisabled && snapshot.sessionCapabilities !== null
    && (snapshot.sessionCapabilities.capabilityBits & CAP_AOA_SESSION_RESTART) === 0;
  const activeDevice = snapshot?.sessionTransport === "aoa" ? undefined
    : snapshot?.devices.find(device => device.serial === snapshot.sessionSerial);
  const cardDevice = active ? activeDevice : cleanupPending ? undefined : selected;
  const deviceChoices = useMemo(() => {
    if (snapshot?.sessionTransport === "aoa" || !snapshot?.sessionSerial || devices.some(device => device.serial === snapshot.sessionSerial)) return devices;
    const current = snapshot.devices.find(device => device.serial === snapshot.sessionSerial) ?? {
      serial: snapshot.sessionSerial, transport: snapshot.sessionTransport === "lan" ? "unknown" : snapshot.sessionTransport ?? "unknown", state: "device", manufacturer: "", model: "",
      androidVersion: snapshot.sessionAndroidVersion ?? "", apiLevel: "", displayName: snapshot.deviceName ?? strings.app.connectedPhone,
      statusText: strings.app.staleDeviceScan, connectable: false,
    };
    return [current, ...devices];
  }, [snapshot, devices]);
  const announce = (message: string, detail = "") => {
    setNotice(message);
    setNoticeDetail(detail);
    window.setTimeout(() => { setNotice(""); setNoticeDetail(""); }, 4200);
  };
  const notify = (message: string) => announce(needsReceiverUpdate(message) ? strings.app.receiverUpdateRequired
    : message.includes("FORMAT_RATE_MISMATCH:") ? strings.app.hiresRateMismatch(windowsRateMismatchHz(message)) : strings.app.operationNotCompleted, message);
  const openReceiverUpgrade = () => {
    setBackgroundStartMode(false);
    setReceiverManagerSerial(snapshot?.sessionSerial ?? selected?.serial ?? "");
    setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver");
  };
  const openNetworkDialog = (backgroundStart = false, choices = { localAddress: "", bindingReference: "" }, recovery = true) => {
    setBackgroundStartMode(backgroundStart);
    setBackgroundChoicePrefill(choices);
    setNetworkRecoveryMode(recovery);
    setNetworkDialogError("");
    setPanel("network");
  };
  const openConnectionFlow = () => openNetworkDialog(true, { localAddress: "", bindingReference: "" }, !!snapshot?.wirelessConnectionOffers.active);
  const openAoaDialog = async (disconnectCurrent = false) => {
    if (aoaConfigurationOpening.current || aoaConfigurationClosing.current || controlsDisabled) return;
    if (active && !disconnectCurrent) { setConfirm({ kind: "aoa", value: "" }); return; }
    aoaConfigurationOpening.current = true;
    setWorking(true);
    try {
      const configuration = await invokeApp<{ snapshot: Snapshot; configurationId: number }>("begin_usb_direct_configuration", { disconnectCurrent });
      aoaConfigurationActive.current = configuration.configurationId;
      if (!appMounted.current) {
        aoaConfigurationActive.current = null;
        await invokeApp("end_usb_direct_configuration", { configurationId: configuration.configurationId });
        return;
      }
      apply(configuration.snapshot);
      setPanel("aoa");
    } catch (error) { notify(String(error)); }
    finally { aoaConfigurationOpening.current = false; setWorking(false); }
  };
  const closeAoaDialog = async () => {
    if (aoaConfigurationClosing.current) return;
    aoaConfigurationClosing.current = true;
    const configurationId = aoaConfigurationActive.current;
    try {
      if (configurationId !== null) {
        const next = await invokeApp<Snapshot>("end_usb_direct_configuration", { configurationId });
        if (aoaConfigurationActive.current === configurationId) {
          aoaConfigurationActive.current = null;
          apply(next);
        }
      }
      if (panelRef.current === "aoa") setPanel(null);
    } catch (error) {
      notify(String(error));
    } finally {
      aoaConfigurationClosing.current = false;
    }
  };
  const startAoa = async (targetId: string, bindingReference: string) => {
    const configurationId = aoaConfigurationActive.current;
    if (!snapshot || active || configurationId === null) return;
    setWorking(true);
    try {
      apply(await invokeApp<Snapshot>("start_aoa_audio", { targetId, bindingReference }));
      if (aoaConfigurationActive.current === configurationId && !aoaConfigurationClosing.current) {
        apply(await invokeApp<Snapshot>("end_usb_direct_configuration", { configurationId }));
        if (aoaConfigurationActive.current === configurationId) {
          aoaConfigurationActive.current = null;
          setPanel(null);
        }
      }
    } finally { setWorking(false); }
  };
  const cancelAoaStart = async () => apply(await invokeApp<Snapshot>("cancel_operation"));

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
      || (active && snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "udp")
      || aoaSession)) return;
    if (kind === "playback" && active && !snapshot.canChangePlaybackMode) return;
    const current = kind === "transport" ? snapshot.connectionTransport
      : kind === "wirelessProtocol" ? snapshot.settings.wirelessProtocol
        : kind === "audioBackend" ? snapshot.settings.audioBackend
          : snapshot.settings.playbackMode === "LowLatency" ? "lowLatency" : "balanced";
    if (value === current) return;
    if (active && kind === "playback" && (lanSession || snapshot.canChangePlaybackMode)) {
      setWorking(true);
      try { apply(await invokeApp<Snapshot>("reconfigure_session", { kind, value })); }
      catch (error) {
        let message = String(error);
        try { apply(await invokeApp<Snapshot>("get_snapshot")); }
        catch (refreshError) { message += strings.app.refreshFailureSuffix(String(refreshError)); }
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
      if (refreshFailure) announce(strings.app.settingsSavedRefreshFailed, refreshFailure); else setNotice("");
    } catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const setWiredMode = (mode: "adb" | "aoa") => {
    if (!snapshot || mode === wiredMode || controlsDisabled) return;
    if (active) { setConfirm({ kind: "wiredMode", value: mode }); return; }
    setWorking(true);
    void invokeApp<Snapshot>("update_setting", { key: "wiredMode", value: mode })
      .then(apply).catch(error => notify(String(error))).finally(() => setWorking(false));
  };

  const performReceiverUsbOperation = async (operation: ReceiverUsbOperation) => {
    const operationId = crypto.randomUUID();
    let unlisten: (() => void) | null = null;
    let commandStarted = false;
    setWorking(true); setReceiverOperationBusy(true); setNetworkDialogError(""); setReceiverOperationResult(null);
    setReceiverOperationProgress({ operationId, serial: operation.serial, stage: "checking" });
    try {
      const stopListening = await listen<ReceiverInstallProgress>("anpod://receiver-install-progress", event => {
        const payload = event.payload;
        if (payload.operationId !== operationId || payload.serial !== operation.serial) return;
        if (payload.stage === "completed") return;
        setReceiverOperationProgress(current => {
          if (current?.operationId !== operationId || current.serial !== operation.serial
            || current.stage === "completed" || current.stage === "failed") return current;
          const order = { checking: 0, transferring: 1, verifying: 2, completed: 3, failed: 3 };
          return order[payload.stage] >= order[current.stage] ? payload : current;
        });
      });
      let listening = true;
      unlisten = () => {
        if (!listening) return;
        listening = false;
        stopListening();
        if (receiverOperationUnlisten.current === unlisten) receiverOperationUnlisten.current = null;
      };
      receiverOperationUnlisten.current = unlisten;
      if (!appMounted.current) { unlisten(); return; }
      const command = operation.kind === "upgrade" ? "receiver_upgrade_usb" : "receiver_install";
      const args = operation.kind === "upgrade"
        ? { serial: operation.serial, bindingReference: operation.bindingReference, operationId }
        : { serial: operation.serial, operationId };
      commandStarted = true;
      const status = await invokeApp<ReceiverStatus>(command, args);
      if (!appMounted.current) return;
      if (!status.installed || status.versionCode !== operation.targetVersionCode) {
        throw new Error(strings.app.usbVersionVerificationFailed(operation.targetVersionCode));
      }
      setReceiverOperationProgress({ operationId, serial: operation.serial, stage: "completed" });
      setReceiverOperationResult({ serial: operation.serial, kind: operation.kind, targetVersionCode: operation.targetVersionCode, status });
      setReceiverManagerSerial(operation.serial);
      if (panelRef.current === operation.returnPanel) setPanel(operation.returnPanel);
      try {
        const next = await invokeApp<Snapshot>("refresh_devices");
        if (appMounted.current) apply(next);
      }
      catch (error) { if (appMounted.current) setNetworkDialogError(strings.app.receiverVersionVerifiedRefreshFailed(status.versionCode, String(error))); }
      if (appMounted.current) announce(strings.app.receiverVersionVerifiedReconnect(status.versionCode));
    } catch (error) {
      if (appMounted.current) {
        if (commandStarted) setReceiverOperationProgress({ operationId, serial: operation.serial, stage: "failed" });
        else setReceiverOperationProgress(null);
        setNetworkDialogError(String(error));
        setReceiverManagerSerial(operation.serial);
        if (panelRef.current === operation.returnPanel) setPanel(operation.returnPanel);
      }
    } finally {
      unlisten?.();
      receiverOperationLock.current = false;
      receiverConfirmationStarted.current = false;
      if (appMounted.current) { setReceiverOperationBusy(false); setWorking(false); }
    }
  };

  const requestReceiverUsbOperation = (operation: ReceiverUsbOperation) => {
    if (receiverOperationLock.current) return;
    setReceiverOperationResult(null); setNetworkDialogError("");
    setReceiverOperationProgress(null);
    if (active && snapshot) {
      const targetMatches = snapshot.sessionTransport === "lan"
        ? operation.bindingReference === snapshot.sessionReceiverBinding
        : operation.serial === snapshot.sessionSerial;
      if (!targetMatches) {
        setNetworkDialogError(strings.app.differentCurrentSessionTarget);
        return;
      }
      receiverOperationLock.current = true;
      receiverConfirmationStarted.current = false;
      setPendingReceiverOperation(operation);
      setPanel(null);
      setConfirm({ kind: "receiverUsb", value: "" });
      return;
    }
    receiverOperationLock.current = true;
    void performReceiverUsbOperation(operation);
  };

  const applyConfirmed = async () => {
    if (!confirm) return;
    if (confirm.kind === "receiverUsb") {
      if (receiverConfirmationStarted.current) return;
      receiverConfirmationStarted.current = true;
    }
    setWorking(true);
    try {
      if (confirm.kind === "network") {
        if (!pendingNetworkStart) throw new Error(strings.app.staleNetworkParameters);
        const stopped = await invokeApp<OperationResult>("disconnect");
        apply(stopped.snapshot);
        if (!stopped.success) throw new Error(stopped.message);
        apply(await invokeApp<Snapshot>("start_network_audio", { ...pendingNetworkStart }));
        if (pendingNetworkStart.backgroundAuthorizationRequired && pendingNetworkStart.bindingReference)
          lastOfferListenerChoice.current = { localAddress: pendingNetworkStart.localAddress, bindingReference: pendingNetworkStart.bindingReference };
        setPanel(null);
      } else if (confirm.kind === "receiverUsb") {
        if (!pendingReceiverOperation) throw new Error(strings.app.staleUsbOperation);
        const operation = pendingReceiverOperation;
        setConfirm(null); setPendingReceiverOperation(null);
        setPanel(operation.returnPanel);
        await performReceiverUsbOperation(operation);
      } else if (confirm.kind === "aoa") {
        const configuration = await invokeApp<{ snapshot: Snapshot; configurationId: number }>("begin_usb_direct_configuration", { disconnectCurrent: true });
        aoaConfigurationActive.current = configuration.configurationId;
        if (!appMounted.current) {
          aoaConfigurationActive.current = null;
          await invokeApp("end_usb_direct_configuration", { configurationId: configuration.configurationId });
          return;
        }
        apply(configuration.snapshot);
        setPanel("aoa");
      } else if (confirm.kind === "wiredMode") {
        const stopped = await invokeApp<OperationResult>("disconnect");
        apply(stopped.snapshot);
        if (!stopped.success) throw new Error(stopped.message);
        apply(await invokeApp<Snapshot>("update_setting", { key: "wiredMode", value: confirm.value }));
      } else if (confirm.kind === "device") {
        const result = await invokeApp<OperationResult>("switch_device", { serial: confirm.value });
        apply(result.snapshot);
        if (!result.success) notify(result.message);
      } else {
        const next = await invokeApp<Snapshot>("reconfigure_session", { kind: confirm.kind, value: confirm.value });
        apply(next);
        if (!hasActiveSession(next) && next.connectionTransport === "wireless" && next.settings.wirelessProtocol === "udp"
          && (confirm.kind === "transport" || confirm.kind === "wirelessProtocol" || confirm.kind === "audioBackend")) {
          openNetworkDialog();
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
        if (confirm.kind === "transport" || confirm.kind === "wirelessProtocol" || confirm.kind === "audioBackend" || confirm.kind === "playback" || confirm.kind === "wiredMode" || confirm.kind === "aoa") {
          try { apply(await invokeApp<Snapshot>("get_snapshot")); }
          catch (refreshError) { message += strings.app.refreshFailureSuffix(String(refreshError)); }
        }
        notify(message);
      }
      setConfirm(null);
      setPendingNetworkStart(null);
      if (confirm.kind === "receiverUsb") setPendingReceiverOperation(null);
    }
    finally {
      if (confirm.kind === "receiverUsb") {
        receiverOperationLock.current = false;
        receiverConfirmationStarted.current = false;
      }
      setWorking(false);
    }
  };

  const startNetwork = async (request: NetworkStartRequest) => {
    setNetworkDialogError("");
    if (cleanupPending) { setNetworkDialogError(strings.app.cleanupBeforeNetworkStart); return; }
    if (active) {
      setPendingNetworkStart(request);
      setConfirm({ kind: "network", value: "" });
      return;
    }
    setWorking(true);
    try {
      apply(await invokeApp<Snapshot>("start_network_audio", { ...request }));
      if (request.backgroundAuthorizationRequired && request.bindingReference)
        lastOfferListenerChoice.current = { localAddress: request.localAddress, bindingReference: request.bindingReference };
      setPanel(null);
    } catch (error) { setNetworkDialogError(String(error)); }
    finally { setWorking(false); }
  };

  const resolveWirelessChoice = async (localAddress: string, bindingReference?: string) => {
    let selectedAddress = localAddress;
    let selectedBinding = bindingReference;
    const addressWasSpecified = !!selectedAddress;
    const bindingWasSpecified = !!selectedBinding;
    const [availableInterfaces, availableBindings] = await Promise.all([
      invokeApp<PrivateIpv4Interface[]>("network_interfaces"),
      invokeApp<string[]>("receiver_binding_references"),
    ]);
    const availableAddress = (address: string | null | undefined) => !!address && availableInterfaces.some(value => value.address === address);
    const availableBinding = (binding: string | null | undefined) => !!binding && availableBindings.includes(binding);
    const snapshotAddress = snapshot?.wirelessConnectionOffers.interfaceAddress;
    const snapshotBinding = snapshot?.wirelessConnectionOffers.automaticBinding;
    const rememberedChoice = lastOfferListenerChoice.current;
    const addressChoiceInvalidated = !selectedAddress && !localAddress && (!!snapshotAddress || !!rememberedChoice?.localAddress)
      && !availableAddress(snapshotAddress) && !availableAddress(rememberedChoice?.localAddress);
    const bindingChoiceInvalidated = !selectedBinding && !bindingReference && (!!snapshotBinding || !!rememberedChoice?.bindingReference)
      && !availableBinding(snapshotBinding) && !availableBinding(rememberedChoice?.bindingReference);
    if (!availableAddress(selectedAddress)) selectedAddress = "";
    if (!availableBinding(selectedBinding)) selectedBinding = "";
    if (!selectedAddress && !addressWasSpecified && availableAddress(snapshotAddress)) selectedAddress = snapshotAddress!;
    if (!selectedBinding && !bindingWasSpecified && availableBinding(snapshotBinding)) selectedBinding = snapshotBinding!;
    if (!selectedAddress && !addressWasSpecified && availableAddress(rememberedChoice?.localAddress)) selectedAddress = rememberedChoice!.localAddress;
    if (!selectedBinding && !bindingWasSpecified && availableBinding(rememberedChoice?.bindingReference)) selectedBinding = rememberedChoice!.bindingReference;
    if (!selectedAddress && !addressWasSpecified && !addressChoiceInvalidated && availableInterfaces.length === 1) selectedAddress = availableInterfaces[0].address;
    if (!selectedBinding && !bindingWasSpecified && !bindingChoiceInvalidated && availableBindings.length === 1) selectedBinding = availableBindings[0];
    return { selectedAddress, selectedBinding, addressChoiceInvalidated, bindingChoiceInvalidated, availableInterfaces, availableBindings };
  };

  const startBackgroundConnection = async (bindingReference?: string) => {
    if (!snapshot || working || snapshot.busy || active) return;
    setWorking(true); setNetworkDialogError("");
    try {
      const choice = await resolveWirelessChoice("", bindingReference);
      setNetworkChoiceNeedsConfirmation({ interfaceAddress: choice.addressChoiceInvalidated, bindingReference: choice.bindingChoiceInvalidated });
      const choices = { localAddress: choice.selectedAddress, bindingReference: choice.selectedBinding ?? "" };
      if (!choices.bindingReference && choice.availableBindings.length === 0) {
        setBackgroundStartMode(false);
        setPanel("setup");
        return;
      }
      if (!choices.localAddress || !choices.bindingReference) {
        openNetworkDialog(true, choices, true);
        return;
      }
      const local = choice.availableInterfaces.find(value => value.address === choices.localAddress);
      const candidates = (await invokeApp<NetworkCandidate[]>("discover_network_receivers", { localAddress: choices.localAddress }))
        .filter(candidate => endpointFitsInterface(candidate.endpoint, local));
      if (candidates.length !== 1) {
        openNetworkDialog(true, choices, true);
        return;
      }
      try {
        apply(await invokeApp<Snapshot>("start_network_audio", {
          localAddress: choices.localAddress, peer: candidates[0].endpoint,
          bindingReference: choices.bindingReference, backgroundAuthorizationRequired: true,
        }));
        lastOfferListenerChoice.current = { localAddress: choices.localAddress, bindingReference: choices.bindingReference };
        setPanel(null);
      } catch (error) {
        openNetworkDialog(true, choices, true);
        setNetworkDialogError(String(error));
      }
    } catch (error) {
      openNetworkDialog(true, { localAddress: "", bindingReference: "" }, true);
      setNetworkDialogError(String(error));
    } finally { setWorking(false); }
  };

  const setOfferListener = async (localAddress: string, allowed: boolean, bindingReference?: string) => {
    if (!snapshot || (allowed && (active || working || snapshot.busy))) return;
    if (allowed) setWorking(true);
    setNetworkDialogError("");
    try {
      let selectedAddress = localAddress;
      let selectedBinding = bindingReference;
      if (allowed) {
        const choice = await resolveWirelessChoice(selectedAddress, selectedBinding);
        selectedAddress = choice.selectedAddress;
        selectedBinding = choice.selectedBinding;
        if (!selectedBinding && choice.availableBindings.length === 0) { setPanel("setup"); return; }
        if (!selectedAddress || !selectedBinding) {
          setNetworkChoiceNeedsConfirmation({ interfaceAddress: choice.addressChoiceInvalidated, bindingReference: choice.bindingChoiceInvalidated });
          openNetworkDialog(false); return;
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
      openNetworkDialog(); return;
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
      if (cancelOperationLock.current) return;
      cancelOperationLock.current = true; setCancelPending(true);
      void invokeApp<Snapshot>("cancel_operation").then(apply).catch(error => notify(String(error))).finally(() => {
        cancelOperationLock.current = false; setCancelPending(false);
      });
    } else if (snapshot.wirelessConnectionOffers.active) {
      void setOfferListener(snapshot.wirelessConnectionOffers.interfaceAddress ?? "", false);
    } else if (cleanupPending || (active && (snapshot.state === "managementOnly" || snapshot.output === "phone"))) {
      void disconnect();
    } else if (!active && snapshot.connectionTransport === "usb" && wiredMode === "aoa") {
      void openAoaDialog();
    } else if (receiverUpdateNeeded && !active) {
      openReceiverUpgrade();
    } else if (!active && wirelessUdpMode) {
      openConnectionFlow();
    } else if (active || selected?.connectable) {
      void setOutput("phone");
    } else {
      setPanel(snapshot.connectionTransport === "usb" ? "help" : "pair");
    }
  };

  const connectOnly = async () => {
    if (!snapshot) return;
    if (snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp") {
      openNetworkDialog(); return;
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

  if (!snapshot) return <main className="loading-shell"><AudioLines size={25} /><span>{strings.app.loading}</span></main>;
  const empty = devices.length === 0;
  const cancellingConnection = snapshot.state === "connecting" || snapshot.state === "reconnecting";
  const statusTitle = readableStatus(snapshot, cleanupPending);
  const statusDetail = snapshot.state === "streaming"
    ? snapshot.output !== "phone" ? strings.app.outputComputer
      : snapshot.playbackDiagnostics?.playbackAdvancing === true
      ? strings.app.playbackProgress
      : strings.app.receivingAudio
    : lanSession
    ? snapshot.state === "managementOnly" ? strings.app.receiverManagementOnly
      : snapshot.state === "phoneReady" ? snapshot.output === "computer" ? strings.app.receiverComputer : strings.app.receiverAwaitingPlayback
      : snapshot.state === "paused" ? strings.app.receiverPaused
      : snapshot.state === "reconnecting" ? strings.app.directNetworkRecovering
          : snapshot.state === "error" ? snapshot.statusDetail || snapshot.message || strings.app.retryDisconnectOrDiagnostics
            : snapshot.output === "computer" ? strings.app.networkSessionComputer : strings.app.networkSessionPhone
    : snapshot.statusDetail;
  const mainActionCanStop = snapshot.wirelessConnectionOffers.active || cleanupPending
    || (active && (snapshot.output === "phone" || snapshot.state === "managementOnly"));
  const mainActionDisabled = cancellingConnection ? cancelPending : !mainActionCanStop
    && (receiverOperationBusy || working || snapshot.busy);
  const mainActionLabel = cancellingConnection ? strings.app.connectCancel
    : snapshot.wirelessConnectionOffers.active ? strings.app.stopReceiving
      : cleanupPending ? strings.app.retryDisconnect
        : !active && snapshot.connectionTransport === "usb" && wiredMode === "aoa" ? working || snapshot.busy ? strings.app.processing : strings.app.connectUsbDirect
        : receiverUpdateNeeded && !active ? strings.app.upgradePhone
        : active && snapshot.state === "managementOnly" ? strings.app.disconnectManagement
          : active && snapshot.output === "phone" ? strings.app.stop
              : !active && wirelessUdpMode ? working || snapshot.busy ? strings.app.processing : strings.app.connectPhone
              : active || selected?.connectable ? working || snapshot.busy ? strings.app.processing : strings.app.playToPhone
              : snapshot.connectionTransport === "usb" ? strings.app.connectUsbPhone : strings.app.addWirelessPhone;
  const failureDetail = [snapshot.message, snapshot.statusDetail].filter(Boolean).join("；") || statusDetail;
  const failureSummary = cleanupPending ? strings.app.cleanupRetryHint : receiverUpdateNeeded ? strings.app.receiverUpdateRequired
    : failureDetail.includes("FORMAT_RATE_MISMATCH:") ? strings.app.hiresRateMismatch(windowsRateMismatchHz(failureDetail)) : strings.app.operationNotCompleted;

  return <div className={`app-shell ${largeText ? "large-text" : ""}`}>
    <header className="topbar">
      <div className="brand"><img src="/icon.png" alt="" /><div><strong>AnPod</strong><span>{strings.app.tagline}</span></div></div>
      <button className="icon-button" aria-label={strings.app.openSettings} title={strings.app.settings} onClick={() => setPanel("settings")}><Settings2 size={18} /></button>
    </header>

    <main className="main-scroll">
      <section className="device-section" aria-label={strings.app.targetPhone}>
        <div className="section-title"><h1>{strings.app.targetPhone}</h1></div>
        <div className="device-card">
          <button className="device-select" onClick={() => snapshot.connectionTransport === "usb" && wiredMode === "aoa" && !active ? void openAoaDialog() : wirelessUdpMode ? openConnectionFlow() : setPanel("device")} disabled={controlsDisabled} aria-haspopup="dialog" aria-expanded={panel === "device" || panel === "network" || panel === "setup" || panel === "aoa"} aria-label={`${strings.app.chooseTarget}，${active ? `${strings.app.currentConnection}${lanSession ? bindingLabel(snapshot.sessionReceiverBinding) : activeDevice?.displayName ?? snapshot.deviceName ?? strings.app.phone}` : cleanupPending ? strings.app.awaitingCleanup : wirelessUdpMode ? snapshot.wirelessConnectionOffers.automaticBinding ? strings.app.savedWirelessPhone : strings.app.choosePairedPhone : snapshot.connectionTransport === "usb" && wiredMode === "aoa" ? strings.app.connectUsbDirect : cardDevice ? `${strings.app.currentSelection}${cardDevice.displayName}` : strings.app.openDeviceList}`}>
            <span className="device-icon"><Smartphone size={18} aria-hidden="true" /></span>
            <span className="device-copy">
              <strong>{active ? lanSession ? strings.app.connectedPhone : activeDevice?.displayName ?? snapshot.deviceName ?? strings.app.connectedPhone : cleanupPending ? snapshot.deviceName ?? strings.app.awaitingCleanup : wirelessUdpMode ? snapshot.wirelessConnectionOffers.automaticBinding ? strings.app.savedWirelessPhone : strings.app.choosePairedPhone : snapshot.connectionTransport === "usb" && wiredMode === "aoa" ? strings.app.chooseUsbDirectPhone : selected?.displayName ?? (empty ? strings.app.noPhone : strings.app.choosePhone)}</strong>
              <span>{active ? aoaSession ? actualConnectionLabel(snapshot) : strings.app.connectedPhone : cleanupPending ? strings.app.connectionUnavailable : wirelessUdpMode || snapshot.connectionTransport === "wireless" || cardDevice?.transport === "wifi" ? strings.app.wireless : strings.app.usb}</span>
              {cleanupPending && <span className="device-live-state" role="status">{strings.app.retryCleanup}</span>}
            </span>
            <ChevronDown size={18} aria-hidden="true" />
          </button>
          <div className="device-actions">
            {active && snapshot.output === "phone" && snapshot.state !== "managementOnly" && <button className="text-button" type="button" disabled={controlsDisabled} onClick={() => void setOutput("computer")}>{strings.app.switchToComputer}</button>}
          </div>
        </div>
      </section>

      <section className="connection-section" aria-label={strings.app.connectionMode}>
        <div className="section-title"><h2>{strings.app.connectionMode}</h2></div>
        <div className="transport-switch" role="group" aria-label={strings.app.connectionMode}>
          <button aria-pressed={snapshot.connectionTransport === "usb"} disabled={controlsDisabled || snapshot.wirelessConnectionOffers.active} onClick={() => void setOption("transport", "usb")}><Usb size={16} aria-hidden="true" />{strings.app.usb}</button>
          <button aria-pressed={snapshot.connectionTransport === "wireless"} disabled={controlsDisabled || snapshot.wirelessConnectionOffers.active} onClick={() => void setOption("transport", "wireless")}><Wifi size={16} aria-hidden="true" />{strings.app.wireless}</button>
        </div>
        {snapshot.connectionTransport === "usb" && <div className="wired-mode-section">
          <div className="mode-row"><h3>{strings.app.wiredMode}</h3><div className="transport-switch" role="group" aria-label={strings.app.wiredMode}>
            <button aria-pressed={wiredMode === "adb"} disabled={controlsDisabled} onClick={() => setWiredMode("adb")}>{strings.app.wiredModeAdb}</button>
            <button aria-pressed={wiredMode === "aoa"} disabled={controlsDisabled} onClick={() => setWiredMode("aoa")}>{strings.app.wiredModeDirect}</button>
          </div></div>
          <p className="connection-hint">{strings.app.wiredModeHint}</p>
        </div>}
      </section>

      <div className={`status-line state-${snapshot.state}`}>
        <span className="status-dot" aria-hidden="true" />
        <span role="status" aria-live="polite">{statusTitle}</span>
      </div>
      <button className="primary-button main-action" type="button" disabled={mainActionDisabled} onClick={runMainAction}>
        {mainActionLabel}
      </button>
      {!active && wirelessUdpMode && !snapshot.wirelessConnectionOffers.active && <p className="status-description">{strings.app.backgroundStartIdleHint}</p>}
      {snapshot.state === "error" || cleanupPending || snapshot.wirelessConnectionOffers.active ? snapshot.state === "error" || cleanupPending ? <ErrorDetails
        className="status-description actionable-error"
        summary={failureSummary}
        details={failureDetail}
      /> : <p className="status-description" role="status">{strings.app.waitingForRequest}</p> : null}
      {receiverUpdateNeeded && (active || cleanupPending) && snapshot.sessionTransport !== "aoa" && <button className="primary-button" type="button" disabled={working || snapshot.busy || receiverOperationBusy} onClick={openReceiverUpgrade}>{strings.app.upgradePhone}</button>}

      <section className="mode-section" aria-label={strings.app.playbackMode}>
        <div className="mode-row"><h2>{strings.app.playbackMode}</h2><div className="mode-switch" role="group" aria-label={strings.app.playbackMode}>
          <button aria-pressed={snapshot.settings.playbackMode === "Balanced"} disabled={controlsDisabled || (active ? !snapshot.canChangePlaybackMode : !snapshot.canChangeModes)} onClick={() => void setOption("playback", "balanced")}>{strings.app.balanced}</button>
          <button aria-pressed={snapshot.settings.playbackMode === "LowLatency"} disabled={controlsDisabled || (active ? !snapshot.canChangePlaybackMode : !snapshot.canChangeModes)} onClick={() => void setOption("playback", "lowLatency")}>{strings.app.lowLatency}</button>
        </div></div>
        <p className="mode-description" role="status" aria-live="polite">{snapshot.settings.playbackMode === "Balanced" ? strings.app.balancedDescription : strings.app.lowLatencyDescription}</p>
        {aoaPlaybackRestartUnavailable && <p className="dialog-footnote">{strings.app.aoaPlaybackModeUpdate}</p>}
      </section>
    </main>

    <footer className="footer"><button onClick={() => setPanel("help")}><CircleHelp size={15} />{strings.app.help}</button><span>v{snapshot.version}</span><button onClick={() => setPanel("diagnostics")}><Activity size={15} />{strings.app.diagnostics}</button></footer>
    {notice && <div className="toast" role="status"><span>{notice}</span>{noticeDetail && <details><summary>{strings.app.showTechnicalDetails}</summary><pre>{noticeDetail}</pre></details>}</div>}

    {panel === "device" && <DeviceDialog devices={deviceChoices} selected={localSelected} activeSerial={snapshot.sessionTransport === "aoa" ? null : snapshot.sessionSerial} activeBinding={lanSession ? snapshot.sessionReceiverBinding : null} busy={working || snapshot.busy} onSelect={chooseDevice} onRefresh={refresh} onConnectOnly={() => void connectOnly()} onAddPhone={snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "adb" ? () => setPanel("pair") : null} onFirstSetup={!active || snapshot.sessionConnectionTransport === "wireless" ? () => setPanel("setup") : null} onClose={() => setPanel(null)} />}
    {panel === "settings" && <SettingsDialog snapshot={snapshot} largeText={largeText} onLargeText={setLargeText} udpBackendFixed={udpBackendFixed} aoaSession={aoaSession} busy={controlsDisabled} onSetOption={(kind, value) => void setOption(kind, value)} onOpenReceiverManager={() => { setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver"); }} onSave={async (key, value) => {
      try { const next = await invokeApp<Snapshot>("update_setting", { key, value }); apply(next); if (key === "preferredTransport") apply(await invokeApp<Snapshot>("refresh_devices")); }
      catch (error) { notify(String(error)); }
    }} onClose={() => setPanel(null)} />}
    {(panel === "network" || panel === "receiver" || panel === "setup") && <NetworkReceiverDialog key={`${panel}:${backgroundStartMode}:${networkRecoveryMode}`} snapshot={snapshot} backgroundStartMode={backgroundStartMode && panel === "network"} backgroundChoicePrefill={backgroundChoicePrefill} networkRecoveryMode={networkRecoveryMode} onNetworkRecoveryChange={setNetworkRecoveryMode} onConnect={startBackgroundConnection} preferredBindingReference={lastOfferListenerChoice.current?.bindingReference ?? snapshot.wirelessConnectionOffers.automaticBinding ?? ""} setupWizard={panel === "setup"} receiverManagerOnly={panel === "receiver"} initialReceiverSerial={receiverManagerSerial} requiredChoices={networkChoiceNeedsConfirmation} usbDevices={snapshot.devices.filter(device => device.transport === "usb" && device.connectable)} usbSeenDevices={snapshot.devices.filter(device => device.transport === "usb")} disabled={working || snapshot.busy} receiverOperationBusy={receiverOperationBusy} receiverOperationResult={receiverOperationResult} receiverOperationProgress={receiverOperationProgress} audioSessionActive={active} udpMode={snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp"} offers={snapshot.wirelessConnectionOffers} error={networkDialogError} onError={setNetworkDialogError} onStart={startNetwork} onSetOfferListener={setOfferListener} onAcceptOffer={acceptOffer} onReceiverSerialChange={serial => { if (serial !== receiverManagerSerial) setReceiverOperationResult(null); setReceiverManagerSerial(serial); }} onRefreshDevices={refresh} onReceiverOperation={requestReceiverUsbOperation} onAddPhone={() => setPanel("pair")} onOpenReceiverManager={() => { setBackgroundStartMode(false); setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver"); }} onClose={() => { setBackgroundStartMode(false); if (!receiverOperationBusy) { setReceiverOperationResult(null); setReceiverOperationProgress(null); } setPanel(null); }} />}
    {panel === "pair" && <PairDialog transport="wifi" initialEndpoint={snapshot.settings.lastWirelessEndpoint ?? ""} disabled={working || snapshot.busy} onClose={() => setPanel(null)} onDone={async () => { apply(await invokeApp<Snapshot>("refresh_devices")); setPanel(wirelessUdpMode ? "network" : "device"); }} />}
    {panel === "help" && <HelpDialog transport={snapshot.connectionTransport} wirelessProtocol={snapshot.settings.wirelessProtocol} onClose={() => setPanel(null)} />}
    {panel === "diagnostics" && <DiagnosticsDialog snapshot={snapshot} onClose={() => setPanel(null)} />}
    {panel === "aoa" && <AoaDialog disabled={false} lastUsbDirectTargetId={snapshot.settings.lastUsbDirectTargetId} lastUsbDirectBindingReference={snapshot.settings.lastUsbDirectBindingReference} onStart={startAoa} onCancelStart={cancelAoaStart} onClose={() => void closeAoaDialog()} />}
    {confirm && (() => {
      // Every interrupting switch uses one confirmation: a single consequence line and one primary
      // action, instead of a separate dialog per kind.
      const installing = confirm.kind === "receiverUsb";
      return <ConfirmDialog
        title={installing ? strings.app.confirmInstallTitle : strings.app.confirmInterruptTitle}
        body={installing ? strings.app.confirmInstallBody : strings.app.confirmInterruptBody}
        confirmLabel={strings.app.confirmInterruptAction}
        busy={working} onCancel={() => { if (confirm.kind === "receiverUsb") { setPanel(pendingReceiverOperation?.returnPanel ?? "receiver"); receiverOperationLock.current = false; receiverConfirmationStarted.current = false; } setConfirm(null); setPendingNetworkStart(null); setPendingReceiverOperation(null); }} onConfirm={() => void applyConfirmed()} />;
    })()}
  </div>;
}


export default App;
