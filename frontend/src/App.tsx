import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Activity, AudioLines, ChevronDown, CircleHelp, Settings2, Smartphone, Usb, Wifi, X } from "lucide-react";
import { strings } from "./strings";
import type { Device, Snapshot, OperationResult, Panel, ConfirmRequest } from "./appTypes";
import { isDesktop, invokeApp, byTransport, hasActiveSession, canSetAudioQuality, bindingLabel, rememberBindingModels, actualConnectionLabel, readableStatus, needsReceiverUpdate, windowsRateMismatchHz, CAP_AOA_SESSION_RESTART } from "./appRuntime";
import { DeviceDialog } from "./components/DeviceDialog";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { ErrorDetails } from "./components/Modal";
import { FluidSegmentedControl } from "./components/ui/FluidSegmentedControl";
import { useNotice } from "./hooks/useNotice";
import { usePcUpdate } from "./hooks/usePcUpdate";
import { useReceiverVersions } from "./hooks/useReceiverVersions";
import { useMainAction } from "./hooks/useMainAction";
import { useNetworkWizard } from "./hooks/useNetworkWizard";
import { useReceiverInstall } from "./hooks/useReceiverInstall";

// Dialogs opened on demand load as separate chunks to keep startup parsing small.
const NetworkReceiverDialog = lazy(() => import("./components/NetworkReceiverDialog").then(module => ({ default: module.NetworkReceiverDialog })));
const SettingsDialog = lazy(() => import("./components/SettingsDialog").then(module => ({ default: module.SettingsDialog })));
const PairDialog = lazy(() => import("./components/PairDialog").then(module => ({ default: module.PairDialog })));
const HelpDialog = lazy(() => import("./components/HelpDialog").then(module => ({ default: module.HelpDialog })));
const DiagnosticsDialog = lazy(() => import("./components/DiagnosticsDialog").then(module => ({ default: module.DiagnosticsDialog })));
const AoaDialog = lazy(() => import("./components/AoaDialog").then(module => ({ default: module.AoaDialog })));

function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [panel, setPanel] = useState<Panel | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const pendingSettingsConfirmation = useRef<{ kind: "wirelessProtocol" | "audioBackend"; value: string; resolve: () => void; reject: (error: unknown) => void } | null>(null);
  const helpButtonRef = useRef<HTMLButtonElement>(null);
  const diagnosticsButtonRef = useRef<HTMLButtonElement>(null);
  const appMounted = useRef(true);
  const aoaConfigurationActive = useRef<number | null>(null);
  const aoaConfigurationOpening = useRef(false);
  const aoaConfigurationClosing = useRef(false);
  const panelRef = useRef(panel);
  panelRef.current = panel;
  const [networkDialogError, setNetworkDialogError] = useState("");
  const [localSelected, setLocalSelected] = useState<string | null>(null);
  const [pairedPhoneCount, setPairedPhoneCount] = useState<number | null>(null);
  const { notice, noticeDetail, announce, dismissNotice, notify } = useNotice();
  const { outdatedReceiverSerial, rememberInstalledVersion } = useReceiverVersions(snapshot);
  const { pcUpdate, checkPcUpdate, downloadPcUpdate, installPcUpdate, openPcUpdateDownload, openPcUpdateRelease, loadPcUpdateStatus } = usePcUpdate();
  const [working, setWorking] = useState(false);
  const [cancelPending, setCancelPending] = useState(false);
  const cancelOperationLockRef = useRef(false);

  useEffect(() => {
    appMounted.current = true;
    return () => {
      appMounted.current = false;
      const configurationId = aoaConfigurationActive.current;
      aoaConfigurationActive.current = null;
      if (configurationId !== null) void invokeApp("end_usb_direct_configuration", { configurationId });
    };
  }, []);
  const snapshotRefreshGenerationRef = useRef(0);

  const apply = (value: Snapshot) => {
    snapshotRefreshGenerationRef.current++;
    setSnapshot(value);
    setLocalSelected(value.selectedSerial);
  };

  const settlePendingSettingsConfirmation = (kind: string, value: string, result: { error?: unknown }) => {
    const pending = pendingSettingsConfirmation.current;
    if (!pending || pending.kind !== kind || pending.value !== value) return;
    pendingSettingsConfirmation.current = null;
    if ("error" in result) pending.reject(result.error);
    else pending.resolve();
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
      const pairing = await listen<string>("anpod://pairing-progress", event => announce(event.payload, "", true));
      retain(pairing);
    };
    void attach().catch(error => { if (!disposed) announce(strings.app.operationNotCompleted, String(error), true); });
    return () => { disposed = true; cleanups.splice(0).forEach(cleanup => cleanup()); };
  }, [announce]);

  useEffect(() => {
    if (!isDesktop || panel) return;
    void invokeApp<Record<string, { model?: string | null }>>("receiver_binding_profiles").then(profiles => {
      rememberBindingModels(profiles);
      setPairedPhoneCount(Object.keys(profiles).length);
    }).catch(() => {});
  }, [panel]);

  useEffect(() => {
    if (!snapshot) return;
    if (snapshot.settings.appearanceTheme !== "System") {
      document.documentElement.dataset.theme = snapshot.settings.appearanceTheme.toLowerCase();
      return;
    }
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const update = () => document.documentElement.dataset.theme = media.matches ? "dark" : "light";
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [snapshot?.settings.appearanceTheme]);

  const { pendingReceiverOperation, performReceiverUsbOperation, receiverConfirmationStarted, receiverManagerSerial, receiverOperationBusy, receiverOperationLock, receiverOperationProgress, receiverOperationResult, requestReceiverUsbOperation, setPendingReceiverOperation, setReceiverManagerSerial, setReceiverOperationProgress, setReceiverOperationResult } = useReceiverInstall({ active: hasActiveSession(snapshot), announce, appMounted, apply, panelRef, rememberInstalledVersion, setConfirm, setNetworkDialogError, setPanel, setWorking, snapshot });
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
  const controlsDisabled = working || cancelPending || snapshot?.busy === true || cleanupPending || receiverOperationBusy || pcUpdate.phase === "installing";
  const aoaPlaybackRestartUnavailable = aoaSession && !controlsDisabled && snapshot.sessionCapabilities !== null
    && (snapshot.sessionCapabilities.capabilityBits & CAP_AOA_SESSION_RESTART) === 0;
  const activeDevice = snapshot?.sessionTransport === "aoa" ? undefined
    : snapshot?.devices.find(device => device.serial === snapshot.sessionSerial);
  const cardDevice = active ? activeDevice : cleanupPending ? undefined : selected;
  const { acceptOffer, backgroundChoicePrefill, backgroundStartMode, cancelNetworkStart, lastOfferListenerChoice, mainListenerGenerationRef, mainListenerPending, networkChoiceNeedsConfirmation, networkRecoveryMode, networkStartPending, networkStartPendingRef, offerListenerChoice, offerListenerGeneration, openConnectionFlow, openNetworkDialog, pendingNetworkStart, retainOfferListenerOnClose, setBackgroundStartMode, setMainListenerPending, setNetworkRecoveryMode, setOfferListener, setPendingNetworkStart, startBackgroundConnection, startMainScreenWaiting, startNetwork, startNetworkAudio, stopOfferListener } = useNetworkWizard({ active, apply, cancelOperationLockRef, cleanupPending, notify, panel, panelRef, setCancelPending, setConfirm, setNetworkDialogError, setPanel, setSnapshot, setWorking, snapshot, snapshotRefreshGenerationRef, wirelessUdpMode, working });
  const deviceChoices = useMemo(() => {
    if (snapshot?.sessionTransport === "aoa" || !snapshot?.sessionSerial || devices.some(device => device.serial === snapshot.sessionSerial)) return devices;
    const current = snapshot.devices.find(device => device.serial === snapshot.sessionSerial) ?? {
      serial: snapshot.sessionSerial, transport: snapshot.sessionTransport === "lan" ? "unknown" : snapshot.sessionTransport ?? "unknown", state: "device", manufacturer: "", model: "",
      androidVersion: snapshot.sessionAndroidVersion ?? "", apiLevel: "", displayName: snapshot.deviceName ?? strings.app.connectedPhone,
      statusText: strings.app.staleDeviceScan, connectable: false,
    };
    return [current, ...devices];
  }, [snapshot, devices]);
  const openReceiverUpgrade = () => {
    setBackgroundStartMode(false);
    setReceiverManagerSerial(snapshot?.sessionSerial ?? outdatedReceiverSerial ?? selected?.serial ?? "");
    setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver");
  };
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
  const { cancelPendingConnection, cancellableConnectionStart, connectOnly, connectionStartGeneration, needsFirstSetup, runMainAction, setOutput } = useMainAction({ active, apply, cancelNetworkStart, cancelOperationLockRef, cleanupPending, dismissNotice, mainListenerGenerationRef, mainListenerPending, networkStartPending, notify, offerListenerChoice, openAoaDialog, openNetworkDialog, pairedPhoneCount, receiverOperationBusy, selected, setBackgroundStartMode, setCancelPending, setMainListenerPending, setPanel, setWorking, snapshot, startMainScreenWaiting, stopOfferListener, wiredMode, wirelessUdpMode, working });
  const startAoa = async (targetId: string, bindingReference: string) => {
    const configurationId = aoaConfigurationActive.current;
    if (!snapshot || active || configurationId === null) return;
    const generation = ++connectionStartGeneration.current;
    cancellableConnectionStart.current = true;
    setWorking(true);
    try {
      const started = await invokeApp<Snapshot>("start_aoa_audio", { targetId, bindingReference });
      if (connectionStartGeneration.current !== generation) return;
      cancellableConnectionStart.current = false;
      apply(started);
      if (aoaConfigurationActive.current === configurationId && !aoaConfigurationClosing.current) {
        const ended = await invokeApp<Snapshot>("end_usb_direct_configuration", { configurationId });
        if (connectionStartGeneration.current !== generation) return;
        apply(ended);
        if (aoaConfigurationActive.current === configurationId) {
          aoaConfigurationActive.current = null;
          setPanel(null);
        }
      }
    } finally {
      if (connectionStartGeneration.current === generation) {
        cancellableConnectionStart.current = false;
        setWorking(false);
      }
    }
  };
  const cancelAoaStart = cancelPendingConnection;

  const refresh = async () => {
    setWorking(true);
    try { apply(await invokeApp<Snapshot>("refresh_devices")); }
    catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const setOption = async (kind: "transport" | "wirelessProtocol" | "audioBackend" | "playback", value: string, reportFailureToSettings = false) => {
    if (!snapshot) return;
    if (working || snapshot.busy || cleanupPending || receiverOperationBusy) return;
    if (kind === "transport" || kind === "wirelessProtocol") {
      mainListenerGenerationRef.current++;
      if (mainListenerPending || snapshot.wirelessConnectionOffers.active) {
        setMainListenerPending(false);
        void stopOfferListener();
      } else offerListenerGeneration.current++;
    }
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
      try {
        apply(await invokeApp<Snapshot>("apply_audio_settings", {
          audioQuality: snapshot.settings.audioQuality,
          playbackMode: value === "lowLatency" ? "LowLatency" : "Balanced",
        }));
      }
      catch (error) {
        let message = String(error);
        try { apply(await invokeApp<Snapshot>("get_snapshot")); }
        catch (refreshError) { message += strings.app.refreshFailureSuffix(String(refreshError)); }
        notify(message);
      }
      finally { setWorking(false); }
      return;
    }
    if (active) {
      setConfirm({ kind, value });
      if (reportFailureToSettings && (kind === "wirelessProtocol" || kind === "audioBackend"))
        return new Promise<void>((resolve, reject) => { pendingSettingsConfirmation.current = { kind, value, resolve, reject }; });
      return;
    }
    if (kind === "playback") {
      setWorking(true);
      void invokeApp<Snapshot>("apply_audio_settings", {
        audioQuality: snapshot.settings.audioQuality,
        playbackMode: value === "lowLatency" ? "LowLatency" : "Balanced",
      }).then(apply).catch(error => notify(String(error))).finally(() => setWorking(false));
      return;
    }
    setWorking(true);
    try {
      let next = await invokeApp<Snapshot>("update_setting", { key: kind, value });
      apply(next);
      let refreshFailure = "";
      if (kind === "transport" || kind === "wirelessProtocol") {
        try { next = await invokeApp<Snapshot>("refresh_devices"); apply(next); }
        catch (error) { refreshFailure = String(error); }
      }
      if (refreshFailure) announce(strings.app.settingsSavedRefreshFailed, refreshFailure); else dismissNotice();
    } catch (error) { notify(String(error)); if (reportFailureToSettings) throw error; }
    finally { setWorking(false); }
  };

  const setWiredMode = (mode: "adb" | "aoa") => {
    if (!snapshot || mode === wiredMode || controlsDisabled) return;
    if (active) { setConfirm({ kind: "wiredMode", value: mode }); return; }
    setWorking(true);
    void invokeApp<Snapshot>("update_setting", { key: "wiredMode", value: mode })
      .then(apply).catch(error => notify(String(error))).finally(() => setWorking(false));
  };

  const saveSetting = async (key: keyof Snapshot["settings"], value: unknown) => {
    if (!snapshot || snapshot.settings[key] === value) return;
    if (key === "audioQuality") {
      try {
        apply(await invokeApp<Snapshot>("apply_audio_settings", { audioQuality: value, playbackMode: snapshot.settings.playbackMode }));
      } catch (error) {
        try { apply(await invokeApp<Snapshot>("get_snapshot")); }
        catch (refreshError) { notify(`${String(error)}${strings.app.refreshFailureSuffix(String(refreshError))}`); throw error; }
        notify(String(error));
        throw error;
      }
      return;
    }
    try {
      const next = await invokeApp<Snapshot>("update_setting", { key, value });
      apply(next);
      if (key === "preferredTransport") apply(await invokeApp<Snapshot>("refresh_devices"));
    } catch (error) {
      notify(String(error));
      throw error;
    }
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
        const started = await startNetworkAudio(pendingNetworkStart);
        if (!started) { setConfirm(null); setPendingNetworkStart(null); return; }
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
      } else if (confirm.kind === "playback") {
        if (!snapshot) throw new Error(strings.app.desktopOnly);
        const playbackMode = confirm.value === "lowLatency" ? "LowLatency" : "Balanced";
        apply(await invokeApp<Snapshot>("apply_audio_settings", { audioQuality: snapshot.settings.audioQuality, playbackMode }));
      } else {
        const next = await invokeApp<Snapshot>("reconfigure_session", { kind: confirm.kind, value: confirm.value });
        apply(next);
        if (!hasActiveSession(next) && next.connectionTransport === "wireless" && next.settings.wirelessProtocol === "udp"
          && (confirm.kind === "transport" || confirm.kind === "wirelessProtocol" || confirm.kind === "audioBackend")) {
          openNetworkDialog();
        }
      }
      settlePendingSettingsConfirmation(confirm.kind, confirm.value, {});
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
      settlePendingSettingsConfirmation(confirm.kind, confirm.value, { error });
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
  const cancellingConnection = cancellableConnectionStart.current || snapshot.state === "connecting" || snapshot.state === "reconnecting";
  const statusTitle = networkStartPending && !cleanupPending ? strings.app.connecting : readableStatus(snapshot, cleanupPending);
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
  const mainActionCanStop = networkStartPending || snapshot.wirelessConnectionOffers.active || mainListenerPending || cleanupPending
    || (active && (snapshot.output === "phone" || snapshot.state === "managementOnly"));
  const mainActionDisabled = cancelPending || (!networkStartPending && !cancellingConnection && !mainActionCanStop
    && (receiverOperationBusy || working || snapshot.busy));
  const mainActionBusyLabel = working || snapshot.busy ? strings.app.processing : strings.app.connectAndPlay;
  const mainActionLabel = networkStartPending || cancellingConnection ? strings.app.connectCancel
    : snapshot.wirelessConnectionOffers.active || mainListenerPending ? strings.app.stopReceiving
      : cleanupPending ? strings.app.retryDisconnect
        : active && (snapshot.state === "managementOnly" || snapshot.output === "phone") ? strings.app.disconnect
          : needsFirstSetup ? strings.app.firstSetup
            : active ? strings.app.connectAndPlay
              : mainActionBusyLabel;
  const failureDetail = [snapshot.message, snapshot.statusDetail].filter(Boolean).join("；") || statusDetail;
  const failureSummary = cleanupPending ? strings.app.cleanupRetryHint : receiverUpdateNeeded ? strings.app.receiverUpdateRequired
    : failureDetail.includes("FORMAT_RATE_MISMATCH:") ? strings.app.hiresRateMismatch(windowsRateMismatchHz(failureDetail)) : strings.app.operationNotCompleted;

  return <div className="app-shell">
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
              <strong>{active ? lanSession ? bindingLabel(snapshot.sessionReceiverBinding) : activeDevice?.displayName ?? snapshot.deviceName ?? strings.app.connectedPhone : cleanupPending ? snapshot.deviceName ?? strings.app.awaitingCleanup : wirelessUdpMode ? snapshot.wirelessConnectionOffers.automaticBinding ? strings.app.savedWirelessPhone : strings.app.choosePairedPhone : snapshot.connectionTransport === "usb" && wiredMode === "aoa" ? strings.app.chooseUsbDirectPhone : selected?.displayName ?? (empty ? strings.app.noPhone : strings.app.choosePhone)}</strong>
              <span>{active ? actualConnectionLabel(snapshot) : cleanupPending ? strings.app.connectionUnavailable : wirelessUdpMode || snapshot.connectionTransport === "wireless" || cardDevice?.transport === "wifi" ? strings.app.wireless : strings.app.usb}</span>
              {cleanupPending && <span className="device-live-state" role="status">{strings.app.retryCleanup}</span>}
            </span>
            <ChevronDown size={18} aria-hidden="true" />
          </button>
        </div>

      <section className="connection-section" aria-label={strings.app.connectionMode}>
        <div className="section-title"><h2>{strings.app.connectionMode}</h2></div>
        <FluidSegmentedControl
          className="transport-switch"
          label={strings.app.connectionMode}
          value={snapshot.connectionTransport}
          options={[
            { value: "usb", label: <><Usb size={16} aria-hidden="true" />{strings.app.usb}</>, disabled: controlsDisabled || snapshot.wirelessConnectionOffers.active },
            { value: "wireless", label: <><Wifi size={16} aria-hidden="true" />{strings.app.wireless}</>, disabled: controlsDisabled || snapshot.wirelessConnectionOffers.active },
          ]}
          onValueChange={value => void setOption("transport", value)}
        />
        {snapshot.wirelessConnectionOffers.active && <p className="connection-hint">{strings.app.stopWaitingToChange}</p>}
        <div className="wired-mode-section">
          <div className="mode-row"><h3>{strings.app.wiredMode}</h3>{snapshot.connectionTransport === "usb" ? <FluidSegmentedControl
            className="transport-switch"
            label={strings.app.wiredMode}
            value={wiredMode}
            options={[
              { value: "adb", label: strings.app.wiredModeAdb, disabled: controlsDisabled },
              { value: "aoa", label: strings.app.wiredModeDirect, disabled: controlsDisabled },
            ]}
            onValueChange={setWiredMode}
          /> : <FluidSegmentedControl
            className="transport-switch"
            label={strings.app.wiredMode}
            value={snapshot.settings.wirelessProtocol}
            options={[
              { value: "udp", label: strings.app.wirelessUdp, disabled: controlsDisabled || snapshot.wirelessConnectionOffers.active },
              { value: "adb", label: strings.app.wirelessAdb, disabled: controlsDisabled || snapshot.wirelessConnectionOffers.active },
            ]}
            onValueChange={value => void setOption("wirelessProtocol", value)}
          />}</div>
        </div>
      </section>

      </section>

      <section className="mode-section home-choice-section" aria-labelledby="home-playback-mode-title">
          <h2 id="home-playback-mode-title">{strings.app.playbackMode}</h2>
          <FluidSegmentedControl
            className="mode-switch"
            label={strings.app.playbackMode}
            value={snapshot.settings.playbackMode}
            options={[
              { value: "Balanced", label: strings.app.balanced, disabled: controlsDisabled || (active ? !snapshot.canChangePlaybackMode : !snapshot.canChangeModes) },
              { value: "LowLatency", label: strings.app.lowLatency, disabled: controlsDisabled || (active ? !snapshot.canChangePlaybackMode : !snapshot.canChangeModes) },
            ]}
            onValueChange={value => void setOption("playback", value === "Balanced" ? "balanced" : "lowLatency")}
          />
          {aoaPlaybackRestartUnavailable && <p className="dialog-footnote">{strings.app.aoaPlaybackModeUpdate}</p>}
      </section>
      <section className="mode-section home-choice-section home-quality-section" aria-labelledby="home-audio-quality-title">
        <h2 id="home-audio-quality-title">{strings.app.audioQualityTitle}</h2>
        <FluidSegmentedControl
          className="quality-switch"
          label={strings.app.audioQualityTitle}
          value={snapshot.settings.audioQuality}
          options={(["standard", "hifi", "hires"] as const).map(quality => ({
            value: quality,
            label: strings.app.audioQualityShortLabels[quality],
            disabled: controlsDisabled || !canSetAudioQuality(snapshot, quality),
          }))}
          onValueChange={quality => {
            if (quality === snapshot.settings.audioQuality) return;
            setWorking(true);
            void saveSetting("audioQuality", quality).catch(() => {}).finally(() => setWorking(false));
          }}
        />
        {!controlsDisabled && !canSetAudioQuality(snapshot, "hires") && <p className="connection-hint">{!snapshot.canChangeModes ? strings.app.changeAfterDisconnect : strings.app.standardQualityOnly}</p>}
      </section>
    </main>
    <section className="action-dock device-status-actions status-hero" aria-label={strings.app.connectionStatus}>
      {(receiverUpdateNeeded || outdatedReceiverSerial) && snapshot.sessionTransport !== "aoa" && <button className="update-banner" type="button" disabled={working || snapshot.busy || receiverOperationBusy} onClick={openReceiverUpgrade}>{strings.app.upgradePhone}</button>}
      <div className={`status-line state-${snapshot.state}`}>
        <span className="status-dot" aria-hidden="true" />
        <span role="status" aria-live="polite">{statusTitle}</span>
      </div>
      <button className="primary-button main-action" type="button" disabled={mainActionDisabled} onClick={runMainAction}>
        {mainActionLabel}
      </button>
      {snapshot.state === "error" || cleanupPending || snapshot.wirelessConnectionOffers.active ? snapshot.state === "error" || cleanupPending ? <ErrorDetails
        className="status-description actionable-error"
        summary={failureSummary}
        details={failureDetail}
      /> : <p className="status-description" role="status">{strings.app.waitingForRequest}</p> : null}
      {active && snapshot.output === "phone" && snapshot.state !== "managementOnly" && <button className="text-button" type="button" disabled={controlsDisabled} onClick={() => void setOutput("computer")}>{strings.app.switchToComputer}</button>}
    </section>

    <footer className="footer"><button ref={helpButtonRef} onClick={() => setPanel("help")}><CircleHelp size={15} />{strings.app.help}</button><span>v{snapshot.version}</span><button ref={diagnosticsButtonRef} onClick={() => setPanel("diagnostics")}><Activity size={15} />{strings.app.diagnostics}</button></footer>
    {notice && <div className="toast" role="status"><span>{notice}</span>{noticeDetail && <details><summary>{strings.app.showTechnicalDetails}</summary><pre>{noticeDetail}</pre></details>}<button className="icon-button toast-close" type="button" onClick={dismissNotice} aria-label={strings.modal.close}><X size={16} /></button></div>}

    {panel === "device" && <DeviceDialog devices={deviceChoices} selected={localSelected} activeSerial={snapshot.sessionTransport === "aoa" ? null : snapshot.sessionSerial} activeBinding={lanSession ? snapshot.sessionReceiverBinding : null} busy={working || snapshot.busy} onSelect={chooseDevice} onRefresh={refresh} onConnectOnly={() => void connectOnly()} onAddPhone={snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "adb" ? () => setPanel("pair") : null} onClose={() => setPanel(null)} />}
    <Suspense fallback={null}>
    {panel === "settings" && <SettingsDialog snapshot={snapshot} udpBackendFixed={udpBackendFixed} aoaSession={aoaSession} busy={controlsDisabled} updateState={pcUpdate} onCheckUpdate={() => void checkPcUpdate()} onDownloadUpdate={() => void downloadPcUpdate()} onInstallUpdate={() => void installPcUpdate(controlsDisabled || active)} onOpenUpdateDownload={() => void openPcUpdateDownload()} onOpenUpdateRelease={() => void openPcUpdateRelease()} onLoadUpdateStatus={() => void loadPcUpdateStatus()} onSetOption={(kind, value) => setOption(kind, value, true)} onOpenReceiverManager={() => { setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver"); }} onSave={saveSetting} onClose={() => setPanel(null)} />}
    {(panel === "network" || panel === "receiver" || panel === "setup") && <NetworkReceiverDialog key={`${panel}:${backgroundStartMode}:${networkRecoveryMode}`} snapshot={snapshot} backgroundStartMode={backgroundStartMode && panel === "network"} backgroundChoicePrefill={backgroundChoicePrefill} networkRecoveryMode={networkRecoveryMode} onNetworkRecoveryChange={setNetworkRecoveryMode} onConnect={startBackgroundConnection} preferredBindingReference={lastOfferListenerChoice.current?.bindingReference ?? snapshot.wirelessConnectionOffers.automaticBinding ?? ""} setupWizard={panel === "setup"} receiverManagerOnly={panel === "receiver"} initialReceiverSerial={receiverManagerSerial} requiredChoices={networkChoiceNeedsConfirmation} usbDevices={snapshot.devices.filter(device => device.transport === "usb" && device.connectable)} usbSeenDevices={snapshot.devices.filter(device => device.transport === "usb")} disabled={working || cancelPending || snapshot.busy} receiverOperationBusy={receiverOperationBusy} receiverOperationResult={receiverOperationResult} receiverOperationProgress={receiverOperationProgress} audioSessionActive={active} udpMode={snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "udp"} offers={snapshot.wirelessConnectionOffers} error={networkDialogError} onError={setNetworkDialogError} onStart={startNetwork} onSetOfferListener={setOfferListener} onAcceptOffer={acceptOffer} onReceiverSerialChange={serial => { if (serial !== receiverManagerSerial) setReceiverOperationResult(null); setReceiverManagerSerial(serial); }} onRefreshDevices={refresh} onReceiverOperation={requestReceiverUsbOperation} onAddPhone={() => setPanel("pair")} onOpenReceiverManager={() => { setBackgroundStartMode(false); setNetworkDialogError(""); setReceiverOperationResult(null); setPanel("receiver"); }} onClose={() => { if (networkStartPendingRef.current) void cancelNetworkStart(); if (panel === "network") retainOfferListenerOnClose.current = true; setBackgroundStartMode(false); if (!receiverOperationBusy) { setReceiverOperationResult(null); setReceiverOperationProgress(null); } setPanel(null); }} />}
    {panel === "pair" && <PairDialog transport="wifi" initialEndpoint={snapshot.settings.lastWirelessEndpoint ?? ""} disabled={working || snapshot.busy} onClose={() => setPanel(null)} onDone={async () => { apply(await invokeApp<Snapshot>("refresh_devices")); setPanel(wirelessUdpMode ? "network" : "device"); }} />}
    {panel === "help" && <HelpDialog transport={snapshot.connectionTransport} wirelessProtocol={snapshot.settings.wirelessProtocol} onClose={() => setPanel(null)} returnFocusTarget={() => helpButtonRef.current} />}
    {panel === "diagnostics" && <DiagnosticsDialog snapshot={snapshot} onClose={() => setPanel(null)} returnFocusTarget={() => diagnosticsButtonRef.current} />}
    {panel === "aoa" && <AoaDialog disabled={false} lastUsbDirectTargetId={snapshot.settings.lastUsbDirectTargetId} lastUsbDirectBindingReference={snapshot.settings.lastUsbDirectBindingReference} onStart={startAoa} onCancelStart={cancelAoaStart} onClose={() => void closeAoaDialog()} />}
    </Suspense>
    {confirm && (() => {
      // Every interrupting switch uses one confirmation: a single consequence line and one primary
      // action, instead of a separate dialog per kind.
      const installing = confirm.kind === "receiverUsb";
      return <ConfirmDialog
        title={installing ? strings.app.confirmInstallTitle : strings.app.confirmInterruptTitle}
        body={installing ? strings.app.confirmInstallBody : strings.app.confirmInterruptBody}
        confirmLabel={confirm.kind === "network" && networkStartPending ? strings.app.connectCancel : strings.app.confirmInterruptAction}
        busy={working && !networkStartPending} onCancel={() => { if (networkStartPending) void cancelNetworkStart(); if (confirm.kind === "receiverUsb") { setPanel(pendingReceiverOperation?.returnPanel ?? "receiver"); receiverOperationLock.current = false; receiverConfirmationStarted.current = false; } settlePendingSettingsConfirmation(confirm.kind, confirm.value, {}); setConfirm(null); setPendingNetworkStart(null); setPendingReceiverOperation(null); }} onConfirm={() => networkStartPending ? void cancelNetworkStart() : void applyConfirmed()} />;
    })()}
  </div>;
}


export default App;
