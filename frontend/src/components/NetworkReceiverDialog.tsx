import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { LoaderCircle, Search } from "lucide-react";
import { strings } from "../strings";
import type { Device, Snapshot, PrivateIpv4Interface, ReceiverIpv4Interface, ReceiverStatus, NetworkCandidate, ReceiverPackageCandidate, BindingResetPreview, BindingResetResult, ReceiverUsbOperation, ReceiverUsbOperationResult, ReceiverInstallProgress, HostDiscoverySnapshot, NetworkStartRequest, BackgroundTrustTarget, BackgroundTrustStatus } from "../appTypes";
import { invokeApp, hasActiveSession, bindingLabel, bindingReferenceLabel, resetPhoneStateLabel, resetPcStateLabel, resetOutcomeLabel, commitmentSummary, backgroundTrustKindLabel, backgroundTrustPersistenceLabel, backgroundTrustRemaining, needsReceiverUpdate, receiverErrorMessage, isIpv4Endpoint, sameIpv4Subnet, bootstrapNetworkPairs, endpointFitsInterface } from "../appRuntime";
import { Modal } from "./Modal";
import { ErrorDetails } from "./Modal";
import { BindingReferenceList } from "./BindingReferenceList";

export function NetworkReceiverDialog({ snapshot, backgroundStartMode, backgroundChoicePrefill, networkRecoveryMode, onNetworkRecoveryChange, onConnect, preferredBindingReference, setupWizard, usbDevices, usbSeenDevices, receiverManagerOnly, initialReceiverSerial, requiredChoices, disabled, receiverOperationBusy, receiverOperationResult, receiverOperationProgress, audioSessionActive, udpMode, offers, error, onError, onStart, onSetOfferListener, onAcceptOffer, onReceiverSerialChange, onRefreshDevices, onReceiverOperation, onAddPhone, onOpenReceiverManager, onClose }: {
  snapshot: Snapshot;
  backgroundStartMode: boolean; backgroundChoicePrefill: { localAddress: string; bindingReference: string };
  networkRecoveryMode: boolean; onNetworkRecoveryChange: (value: boolean) => void; onConnect: (bindingReference?: string) => void; preferredBindingReference: string;
  setupWizard: boolean;
  receiverManagerOnly: boolean;
  initialReceiverSerial: string;
  requiredChoices: { interfaceAddress: boolean; bindingReference: boolean };
  usbDevices: Device[]; usbSeenDevices: Device[]; disabled: boolean; receiverOperationBusy: boolean; receiverOperationResult: ReceiverUsbOperationResult | null; receiverOperationProgress: ReceiverInstallProgress | null; audioSessionActive: boolean; udpMode: boolean; offers: HostDiscoverySnapshot; error: string; onError: (error: string) => void;
  onStart: (request: NetworkStartRequest) => void; onSetOfferListener: (localAddress: string, allowed: boolean, bindingReference?: string) => void;
  onAcceptOffer: (offerId: string, bindingReference: string) => void; onReceiverSerialChange: (serial: string) => void;
  onRefreshDevices: () => void; onReceiverOperation: (operation: ReceiverUsbOperation) => void; onAddPhone: () => void; onOpenReceiverManager: () => void; onClose: () => void;
}) {
  const [method, setMethod] = useState<"bootstrap" | "paired">(backgroundStartMode ? "paired" : "bootstrap");
  const [setupStep, setSetupStep] = useState<1 | 2 | 3>(1);
  const [interfaces, setInterfaces] = useState<PrivateIpv4Interface[]>([]);
  const [receiverInterfaces, setReceiverInterfaces] = useState<ReceiverIpv4Interface[]>([]);
  const [bindings, setBindings] = useState<string[]>([]);
  const [candidates, setCandidates] = useState<NetworkCandidate[]>([]);
  const [localAddress, setLocalAddress] = useState("");
  const [serial, setSerial] = useState("");
  const [receiverAddress, setReceiverAddress] = useState("");
  const [bindingReference, setBindingReference] = useState(preferredBindingReference);
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
  const [backgroundTrust, setBackgroundTrust] = useState<{ value: BackgroundTrustStatus; checkedAt: number; targetKey: string } | null>(null);
  const [backgroundTrustError, setBackgroundTrustError] = useState("");
  const [backgroundTrustNotice, setBackgroundTrustNotice] = useState("");
  const [backgroundTrustBusy, setBackgroundTrustBusy] = useState(false);
  const [confirmBackgroundTrustRevoke, setConfirmBackgroundTrustRevoke] = useState(false);
  const alertRef = useRef<HTMLParagraphElement>(null);
  const setupStepRef = useRef<HTMLDivElement>(null);
  const receiverGeneration = useRef(0);
  const receiverCheckInFlight = useRef<{ serial: string; generation: number; promise: Promise<ReceiverStatus | null> } | null>(null);
  const serialRef = useRef(serial);
  serialRef.current = serial;
  const discoveryGeneration = useRef(0);
  const networkInfoGeneration = useRef(0);
  const bindingResetGeneration = useRef(0);
  const backgroundTrustGeneration = useRef(0);
  const backgroundTrustTargetKeyRef = useRef("");
  const pendingBackgroundRevokeTargetRef = useRef<{ key: string; target: BackgroundTrustTarget } | null>(null);
  const snapshotRef = useRef(snapshot);
  const backgroundChoiceInvalidated = useRef({ interfaceAddress: false, bindingReference: false });
  snapshotRef.current = snapshot;
  const methodManuallySelected = useRef(receiverManagerOnly || setupWizard);
  const peerManuallyEdited = useRef(false);
  const peerSelectionSource = useRef<"candidate" | "manual" | null>(null);
  const busy = disabled || loading || receiverAction !== null || receiverOperationBusy || candidateLoading || bindingResetLoading || bindingResetBusy || backgroundTrustBusy;
  const selectedOffer = offers.offers.find(offer => offer.id === selectedOfferId) ?? (offers.offers.length === 1 ? offers.offers[0] : null);
  const receiverProgressStage = receiverOperationProgress?.serial === serial ? receiverOperationProgress.stage : null;
  const receiverProgressLabel = receiverProgressStage === "checking" ? strings.network.receiverProgressChecking
    : receiverProgressStage === "transferring" ? strings.network.receiverProgressTransferring
      : receiverProgressStage === "verifying" ? strings.network.receiverProgressVerifying
        : receiverProgressStage === "completed" ? strings.network.receiverProgressCompleted
          : receiverProgressStage === "failed" ? strings.network.receiverProgressFailed : "";
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
      setLocalAddress(current => {
        if (interfacesResult.value.some(value => value.address === current)) return current;
        if (backgroundStartMode && current) { backgroundChoiceInvalidated.current.interfaceAddress = true; return ""; }
        if (backgroundStartMode && backgroundChoiceInvalidated.current.interfaceAddress) return "";
        if (backgroundStartMode && backgroundChoicePrefill.localAddress) {
          if (interfacesResult.value.some(value => value.address === backgroundChoicePrefill.localAddress)) return backgroundChoicePrefill.localAddress;
          backgroundChoiceInvalidated.current.interfaceAddress = true; return "";
        }
        return !requiredChoices.interfaceAddress && interfacesResult.value.some(value => value.address === offers.interfaceAddress) ? offers.interfaceAddress ?? ""
          : !requiredChoices.interfaceAddress && interfacesResult.value.length === 1 ? interfacesResult.value[0].address : "";
      });
      if (interfacesResult.value.length === 0) errors.push(strings.network.unavailableComputerInterface);
    } else errors.push(String(interfacesResult.reason));
    if (bindingsResult.status === "fulfilled") {
      const values = bindingsResult.value;
      setBindings(values);
      setBindingReference(current => {
        if (values.includes(current)) return current;
        if (backgroundStartMode && current) { backgroundChoiceInvalidated.current.bindingReference = true; return ""; }
        if (backgroundStartMode && backgroundChoiceInvalidated.current.bindingReference) return "";
        if (backgroundStartMode && backgroundChoicePrefill.bindingReference) {
          if (values.includes(backgroundChoicePrefill.bindingReference)) return backgroundChoicePrefill.bindingReference;
          backgroundChoiceInvalidated.current.bindingReference = true; return "";
        }
        return receiverManagerOnly && snapshot.sessionTransport === "lan" && snapshot.sessionReceiverBinding && values.includes(snapshot.sessionReceiverBinding)
          ? snapshot.sessionReceiverBinding
          : !receiverManagerOnly && !requiredChoices.bindingReference && values.length === 1 ? values[0] : "";
      });
      if (!methodManuallySelected.current) setMethod(backgroundStartMode ? "paired" : values.length > 0 ? "paired" : "bootstrap");
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
        if (!status) throw new Error(strings.network.receiverStatusUnavailable);
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
  const readReceiverInterfaces = async (deviceSerial: string, generation: number) => {
    const values = await invokeApp<ReceiverIpv4Interface[]>("receiver_network_interfaces", { serial: deviceSerial });
    if (receiverGeneration.current !== generation || serialRef.current !== deviceSerial) return;
    setReceiverInterfaces(values);
    setReceiverAddress(current => values.some(value => value.address === current) ? current : "");
    setLocalError(values.length === 0 ? strings.network.receiverHasNoIpv4 : "");
  };
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
    await readReceiverInterfaces(deviceSerial, generation);
    return status;
  };
  const checkReceiver = (deviceSerial: string, clearFlowError = true): Promise<ReceiverStatus | null> => {
    if (receiverCheckInFlight.current) {
      const inFlight = receiverCheckInFlight.current;
      if (inFlight.serial === deviceSerial && inFlight.generation === receiverGeneration.current) return inFlight.promise;
      return inFlight.promise.then(() => serialRef.current === deviceSerial ? checkReceiver(deviceSerial, clearFlowError) : null);
    }
    const generation = ++receiverGeneration.current;
    if (clearFlowError) onError("");
    setReceiverStatus(null); setReceiverError(""); setReceiverInterfaces([]); setReceiverAddress(""); setLocalError("");
    setReceiverAction("checking");
    const promise = (async () => {
      try { return await readReceiverStatusAndInterfaces(deviceSerial, generation); }
      catch (reason) {
        if (receiverGeneration.current === generation && serialRef.current === deviceSerial) setReceiverError(String(reason));
        return null;
      }
      finally {
        if (receiverCheckInFlight.current?.generation === generation) receiverCheckInFlight.current = null;
        if (receiverGeneration.current === generation && serialRef.current === deviceSerial) setReceiverAction(null);
      }
    })();
    receiverCheckInFlight.current = { serial: deviceSerial, generation, promise };
    return promise;
  };
  useEffect(() => {
    if (receiverOperationBusy) { setReceiverAction(null); return; }
    if (receiverOperationResult?.serial === serial) {
      const generation = ++receiverGeneration.current;
      setReceiverStatus(receiverOperationResult.status);
      setReceiverError("");
      if (receiverManagerOnly || !receiverOperationResult.status.installed) {
        setReceiverInterfaces([]); setReceiverAddress(""); setLocalError(""); setReceiverAction(null);
        return;
      }
      setReceiverAction("checking");
      void readReceiverInterfaces(serial, generation).catch(reason => {
        if (receiverGeneration.current === generation && serialRef.current === serial) setReceiverError(String(reason));
      }).finally(() => {
        if (receiverGeneration.current === generation && serialRef.current === serial) setReceiverAction(null);
      });
      return;
    }
    setReceiverInterfaces([]); setReceiverAddress("");
    setReceiverStatus(null); setReceiverError(""); setLocalError("");
    if (serial) void checkReceiver(serial, false);
    else setReceiverAction(null);
    return () => { receiverGeneration.current++; };
  }, [receiverManagerOnly, receiverOperationBusy, receiverOperationResult, serial]);

  useEffect(() => {
    if (receiverManagerOnly) return;
    if (method === "paired") {
      if (!interfaces.some(value => value.address === localAddress)) setLocalAddress(backgroundStartMode && backgroundChoiceInvalidated.current.interfaceAddress
        ? "" : !requiredChoices.interfaceAddress && interfaces.length === 1 ? interfaces[0].address : "");
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
    if (backgroundStartMode && peerSelectionSource.current === "candidate") {
      peerSelectionSource.current = null; peerManuallyEdited.current = false; setPeer("");
    }
    if (!automatic) setLocalError("");
    setAddressNotice(strings.network.searchingCandidates);
    try {
      const values = await invokeApp<NetworkCandidate[]>("discover_network_receivers", { localAddress });
      if (discoveryGeneration.current !== generation) return;
      const usable = values.filter(value => endpointFitsInterface(value.endpoint, local));
      setCandidates(usable);
      if (!peerManuallyEdited.current && usable.length === 1) {
        peerSelectionSource.current = "candidate";
        setPeer(usable[0].endpoint);
      }
      setAddressNotice(usable.length === 1 ? strings.network.oneUnverifiedCandidate
        : usable.length > 1 ? strings.network.manyUnverifiedCandidates
          : strings.network.noCandidateSuggestion);
      if (usable.length === 0 && !automatic) setLocalError(strings.network.noReceiverDiscovered);
    } catch (reason) { if (discoveryGeneration.current === generation) { setAddressNotice(""); onError(String(reason)); } }
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
    if (!localAddress) { setLocalError(strings.network.chooseComputerInterface); return; }
    if (backgroundStartMode) {
      if (!bindingReference) { setLocalError(strings.network.choosePairedBinding); return; }
      const local = interfaces.find(value => value.address === localAddress);
      if (!isIpv4Endpoint(peer)) { setLocalError(strings.network.chooseCandidateOrIpv4); return; }
      if (!endpointFitsInterface(peer, local)) { setLocalError(strings.network.endpointOutsideSubnet); return; }
      onStart({ localAddress, peer: peer.trim(), bindingReference, backgroundAuthorizationRequired: true });
      return;
    }
    if (method === "bootstrap") {
      if (!receiverStatus?.installed) { setLocalError(strings.network.checkInstallReceiverFirst); return; }
      const receiver = receiverInterfaces.find(value => value.address === receiverAddress);
      if (!serial || !receiver) { setLocalError(strings.network.chooseAuthorizedUsbWithAddress); return; }
      onStart({ localAddress, peer: "", bootstrapSerial: serial, receiverAddress: receiver.address, receiverPrefixLength: receiver.prefixLength });
      return;
    }
    if (!bindingReference) { setLocalError(strings.network.choosePairedBinding); return; }
    const local = interfaces.find(value => value.address === localAddress);
    if (!isIpv4Endpoint(peer)) { setLocalError(strings.network.chooseCandidateOrIpv4); return; }
    if (!endpointFitsInterface(peer, local)) { setLocalError(strings.network.endpointOutsideSubnet); return; }
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
  const currentVersionLabel = receiverStatus?.installed === false ? strings.network.notInstalled
    : currentVersionCode === null ? strings.network.awaitingUsbVerification : String(currentVersionCode);
  const packageVersion = updateCandidate?.versionCode ?? null;
  const receiverNeedsUpgrade = !!receiverStatus?.installed && currentVersionCode !== null && packageVersion !== null && currentVersionCode < packageVersion;
  const receiverAlreadyCurrent = currentVersionCode !== null && packageVersion !== null && currentVersionCode >= packageVersion;
  const receiverNeedsInstall = !!serial && receiverStatus?.installed === false;
  const canRequestUsbInstall = receiverNeedsInstall && !!updateCandidate && !receiverError;
  const canRequestUsbMaintenance = receiverNeedsUpgrade && snapshot.sessionTransport !== "lan" && !bindingReference && !!updateCandidate && !receiverError;
  const canRequestUsbUpgrade = receiverNeedsUpgrade && bindings.length > 0 && !!updateCandidate && !!bindingReference && !receiverError;
  const canStart = backgroundStartMode ? method === "paired" && !!bindingReference && endpointFitsInterface(peer, selectedLocalInterface)
    : method === "bootstrap"
    ? !!serial && receiverStatus?.installed === true && !!selectedBootstrapPair
    : !!bindingReference && endpointFitsInterface(peer, selectedLocalInterface);
  const backgroundTrustTarget: BackgroundTrustTarget | null = audioSessionActive
    && snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "udp"
    && snapshot.sessionReceiverBinding && typeof snapshot.sessionGeneration === "number" && Number.isSafeInteger(snapshot.sessionGeneration)
      ? { target: "activeSession", bindingReference: snapshot.sessionReceiverBinding, sessionGeneration: snapshot.sessionGeneration }
      : !audioSessionActive && bindingReference && localAddress && endpointFitsInterface(peer, selectedLocalInterface)
        ? { target: "receiver", bindingReference, localAddress, peer: peer.trim() }
        : null;
  const backgroundTrustTargetKey = backgroundTrustTarget ? JSON.stringify(backgroundTrustTarget) : "";
  useLayoutEffect(() => {
    if (backgroundTrustTargetKey === backgroundTrustTargetKeyRef.current) return;
    const previousKey = backgroundTrustTargetKeyRef.current;
    const pendingRevoke = pendingBackgroundRevokeTargetRef.current;
    const preservingRevokedSession = !!pendingRevoke && pendingRevoke.key === previousKey
      && pendingRevoke.target.target === "activeSession" && !hasActiveSession(snapshotRef.current)
      && (!backgroundTrustTarget || backgroundTrustTarget.bindingReference === pendingRevoke.target.bindingReference);
    backgroundTrustTargetKeyRef.current = backgroundTrustTargetKey;
    if (!preservingRevokedSession) {
      backgroundTrustGeneration.current++;
      setBackgroundTrust(null); setBackgroundTrustError(""); setBackgroundTrustNotice(""); setConfirmBackgroundTrustRevoke(false); setBackgroundTrustBusy(false);
    }
  }, [backgroundTrustTargetKey]);
  useLayoutEffect(() => {
    if (backgroundTrust && backgroundTrust.value.bindingReference !== bindingReference) {
      setBackgroundTrust(null); setBackgroundTrustNotice(""); setBackgroundTrustError("");
    }
  }, [bindingReference]);
  const operateBackgroundTrust = async (revoke: boolean) => {
    const target = backgroundTrustTarget;
    const targetKey = backgroundTrustTargetKey;
    if (!target || !targetKey || backgroundTrustBusy) return;
    const generation = ++backgroundTrustGeneration.current;
    if (revoke) pendingBackgroundRevokeTargetRef.current = { key: targetKey, target };
    setBackgroundTrustBusy(true); setBackgroundTrust(null); setBackgroundTrustError(""); setBackgroundTrustNotice("");
    setConfirmBackgroundTrustRevoke(false);
    const current = () => generation === backgroundTrustGeneration.current && targetKey === backgroundTrustTargetKeyRef.current;
    const matchesRequest = (value: BackgroundTrustStatus) => value.target === target.target
      && value.bindingReference === target.bindingReference
      && value.sessionGeneration === (target.target === "activeSession" ? target.sessionGeneration : null);
    try {
      const value = await invokeApp<BackgroundTrustStatus>(revoke ? "revoke_background_trust" : "query_background_trust", { target });
      if (!matchesRequest(value)) throw new Error(strings.network.backgroundTrustTargetMismatch);
      const expectedSessionEnd = revoke && target.target === "activeSession"
        && pendingBackgroundRevokeTargetRef.current?.key === targetKey
        && !hasActiveSession(snapshotRef.current)
        && (snapshotRef.current.sessionGeneration === null || snapshotRef.current.sessionGeneration !== target.sessionGeneration)
        && (!backgroundTrustTarget || backgroundTrustTarget.bindingReference === target.bindingReference);
      if (!current() && !expectedSessionEnd) return;
      setBackgroundTrust({ value, checkedAt: Date.now(), targetKey });
      setBackgroundTrustError("");
      if (revoke) setBackgroundTrustNotice(strings.network.backgroundTrustRevokeAck);
    } catch (reason) {
      const expectedSessionEnd = revoke && target.target === "activeSession"
        && pendingBackgroundRevokeTargetRef.current?.key === targetKey
        && !hasActiveSession(snapshotRef.current)
        && (!backgroundTrustTarget || backgroundTrustTarget.bindingReference === target.bindingReference);
      if (current() || expectedSessionEnd) {
        setBackgroundTrust(null);
        setBackgroundTrustError(`${strings.network.backgroundTrustUnknown} ${String(reason)}`);
        setBackgroundTrustNotice("");
      }
    } finally {
      if (revoke && pendingBackgroundRevokeTargetRef.current?.key === targetKey) pendingBackgroundRevokeTargetRef.current = null;
      if (current() || !hasActiveSession(snapshotRef.current)) setBackgroundTrustBusy(false);
    }
  };
  const requestUsbOperation = (kind: ReceiverUsbOperation["kind"]) => {
    if (!serial || !updateCandidate) return;
    onReceiverOperation({
      kind, serial, ...(kind === "upgrade" ? { bindingReference } : {}),
      targetVersionCode: updateCandidate.versionCode, returnPanel: receiverManagerOnly ? "receiver" : setupWizard ? "setup" : "network",
    });
  };
  const pairedServiceStopped = method === "paired" && !!serial && receiverStatus?.installed === true && !receiverStatus.serviceActive;
  const mainSummary = receiverManagerOnly ? selectedReceiverDevice?.displayName ?? strings.network.chooseAuthorizedUsb : method === "paired"
    ? bindingReference ? strings.network.savedPhoneNumber(bindings.indexOf(bindingReference) + 1) : strings.network.choosePairedReceiver
    : selectedReceiverDevice?.displayName ?? strings.network.chooseUsbPhone;

  const unauthorizedUsb = usbSeenDevices.some(device => device.state === "unauthorized" || device.statusText.includes("未授权"));
  const setUpStep = (step: 1 | 2 | 3) => { setSetupStep(step); setLocalError(""); onError(""); };

  return <Modal title={setupWizard ? strings.network.dialogSetup : receiverManagerOnly ? strings.network.dialogReceiverManager : strings.network.dialogWirelessReceiver} onClose={onClose} className={`network-dialog${setupWizard ? " setup-wizard-dialog" : ""}`}>
    <form className="network-pane" onSubmit={submit}>
      {setupWizard && <section className="setup-wizard" aria-label={strings.network.setupUsb}>
        <ol className="setup-step-list" aria-label={strings.network.setupSteps}>
          {([strings.network.setupStepUsb, strings.network.setupStepReceiver, strings.network.setupStepBinding] as const).map((label, index) => {
            const step = (index + 1) as 1 | 2 | 3;
            return <li key={label}><button type="button" className="setup-step" aria-current={setupStep === step ? "step" : undefined} disabled={busy || step > setupStep} onClick={() => setUpStep(step)}><span>{step}</span>{label}</button></li>;
          })}
        </ol>
        {setupStep === 1 && <div ref={setupStepRef} className="setup-wizard-step">
          <p>{strings.network.connectAndAuthorizeUsb}</p>
          <label className="network-field">{strings.network.authorizedOnlineUsb}
            <select required value={serial} disabled={busy} onChange={event => { setSerial(event.target.value); onReceiverSerialChange(event.target.value); setLocalError(""); onError(""); }}>
              <option value="">{strings.network.chooseUsbPlaceholder}</option>
              {usbDevices.map(device => <option key={device.serial} value={device.serial}>{device.displayName}</option>)}
            </select>
          </label>
          {usbDevices.length === 0 && <div className="network-recovery" role="status">
            <span>{unauthorizedUsb ? strings.network.usbDetectedUnauthorized : strings.network.noAuthorizedUsb}</span>
            <button type="button" className="text-button" disabled={busy} onClick={onRefreshDevices}>{strings.network.refreshUsbDevices}</button>
          </div>}
          <div className="setup-wizard-actions"><button className="primary-button" type="button" disabled={busy || !serial} onClick={() => setUpStep(2)}>{strings.network.next}</button></div>
        </div>}
        {setupStep === 2 && <div ref={setupStepRef} className="setup-wizard-step">
          <p>{strings.network.checkReceiverOnPhone}</p>
          {serial ? <section className="receiver-manager" aria-label={strings.network.receiverInstallCheck}>
            <div className="receiver-status-row">
              <div className="receiver-status-copy" role="status" aria-live="polite">
                <strong>{receiverAction === "checking" ? strings.network.checkingReceiver : receiverError ? strings.network.receiverCheckFailed : !receiverStatus ? strings.network.waitingReceiverCheck : !receiverStatus.installed ? strings.network.receiverNotInstalled : receiverStatus.serviceActive ? receiverStatus.healthy ? strings.network.receiverHealthy : strings.network.receiverUnhealthy : strings.network.receiverStopped}</strong>
                {receiverStatus?.installed && <small>{[receiverStatus.versionName, receiverStatus.versionCode === null ? "" : strings.network.versionCode(receiverStatus.versionCode)].filter(Boolean).join(" · ")}</small>}
                {receiverStatus?.installed && !receiverStatus.healthy && <ErrorDetails summary={strings.app.operationNotCompleted} details={receiverStatus.error || receiverStatus.message} />}
                {receiverNeedsUpgrade && <small>{strings.network.oldCompatibleCanContinue}</small>}
                {receiverOperationResult?.serial === serial && <small>{strings.network.recheckedVersion(receiverOperationResult.status.versionCode)}</small>}
                {updateCandidate && <small>{strings.network.availableVersion(updateCandidate.versionName, updateCandidate.versionCode, (updateCandidate.sizeBytes / (1024 * 1024)).toFixed(1))}</small>}
                {!updateCandidate && <small>{candidateLoading ? strings.network.checkingBundledVersion : updateCandidateError ? strings.app.operationNotCompleted : strings.network.bundledPackageUnavailable}</small>}
                {updateCandidateError && <ErrorDetails summary={strings.app.operationNotCompleted} details={updateCandidateError} />}
              </div>
              <div className="receiver-status-actions setup-receiver-actions">
                <button className="dialog-secondary" type="button" disabled={busy} onClick={() => void checkReceiver(serial)}>{strings.network.recheck}</button>
                {canRequestUsbInstall && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{strings.network.installReceiver(updateCandidate.versionCode)}</button>}
                {setupWizard && !audioSessionActive && receiverNeedsUpgrade && updateCandidate && !receiverError && <button className="dialog-secondary" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{strings.network.optionalMaintenanceUpgrade(updateCandidate.versionCode)}</button>}
                {!setupWizard && canRequestUsbMaintenance && <button className="dialog-secondary" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{strings.network.optionalMaintenanceUpgrade(updateCandidate.versionCode)}</button>}
                {!setupWizard && canRequestUsbUpgrade && <button className="dialog-secondary" type="button" disabled={busy} onClick={() => requestUsbOperation("upgrade")}>{strings.network.optionalUpgrade(updateCandidate.versionCode)}</button>}
              </div>
            </div>
            {receiverError && <ErrorDetails summary={needsReceiverUpdate(receiverError) ? receiverErrorMessage(receiverError) : strings.app.operationNotCompleted} details={receiverError} summaryRef={alertRef} />}
            {error && <ErrorDetails summary={needsReceiverUpdate(error) ? receiverErrorMessage(error) : strings.app.operationNotCompleted} details={error} summaryRef={alertRef} />}
            {canRequestUsbInstall && <p className="network-note">{strings.network.verifiedPackageInstallNote}</p>}
          </section> : <p className="network-recovery" role="status">{strings.network.goBackChooseUsb}</p>}
          <div className="setup-wizard-actions"><button className="dialog-secondary" type="button" disabled={busy} onClick={() => setUpStep(1)}>{strings.network.back}</button><button className="primary-button" type="button" disabled={busy || receiverStatus?.installed !== true || !!receiverError || !!error} onClick={async () => { const checkedSerial = serial; const checked = await checkReceiver(checkedSerial); if (checked?.installed === true && serialRef.current === checkedSerial) setUpStep(3); }}>{strings.network.next}</button></div>
        </div>}
        {setupStep === 3 && <div ref={setupStepRef} className="setup-wizard-step">
          <p>{strings.network.chooseMatchingInterfaces}</p>
          <label className="network-field">{strings.network.computerInterface}
            <select required value={localAddress} disabled={busy} onChange={event => { setLocalAddress(event.target.value); setLocalError(""); onError(""); }}>
              <option value="">{strings.network.chooseIpv4Interface}</option>
              {interfaces.map(value => <option key={`${value.index}-${value.address}`} value={value.address}>{value.name} · {value.address}/{value.prefixLength}</option>)}
            </select>
          </label>
          <label className="network-field">{strings.network.phoneCurrentIpv4}
            <select required value={receiverAddress} disabled={busy || receiverStatus?.installed !== true || !localAddress} onChange={event => { setReceiverAddress(event.target.value); setLocalError(""); onError(""); }}>
              <option value="">{strings.network.choosePhoneInterface}</option>
              {compatibleReceiverInterfaces.map(value => <option key={`${value.interfaceIndex}-${value.address}`} value={value.address}>{value.interfaceName} · {value.address}/{value.prefixLength}</option>)}
            </select>
          </label>
          {interfaces.length > 0 && receiverInterfaces.length > 0 && validBootstrapPairs.length === 0 && <div className="network-recovery" role="status"><span>{strings.network.noSameSubnet}</span></div>}
          {(interfaces.length === 0 || receiverInterfaces.length === 0) && <p className="network-note">{strings.network.requiresComputerAndPhoneIp}</p>}
          <button type="button" className="text-button network-refresh" disabled={busy || !serial} onClick={() => { void Promise.all([loadNetworkInfo(), checkReceiver(serial)]); }}>{strings.network.recheckNetworkInterfaces}</button>
          {error ? <ErrorDetails id="network-flow-error" className="network-error" summary={needsReceiverUpdate(error) ? receiverErrorMessage(error) : strings.app.connectionNotCompleted} details={error} summaryRef={alertRef} />
            : localError && <p id="network-flow-error" ref={alertRef} className="inline-error network-error" role="alert" tabIndex={-1}>{receiverErrorMessage(localError)}</p>}
          <div className="setup-wizard-actions"><button className="dialog-secondary" type="button" disabled={busy} onClick={() => setUpStep(2)}>{strings.network.back}</button><button className="primary-button" type="submit" disabled={busy || !canStart}>{strings.network.bindAndConnect}</button></div>
        </div>}
      </section>}
      {!setupWizard && !receiverManagerOnly && !networkRecoveryMode && <section className="connection-entry">
        {audioSessionActive ? <>
          <p>{strings.network.phoneAlreadyConnected}</p>
          <button className="dialog-secondary" type="button" onClick={onClose}>{strings.network.done}</button>
        </> : <>
          <p>{bindings.length === 0 ? strings.network.firstUseConnectHint : strings.network.connectSavedPhoneHint}</p>
          {bindings.length > 1 && <label className="network-field">{strings.network.savedPhoneChoice}
            <select value={bindingReference} onChange={event => setBindingReference(event.target.value)}>
              <option value="">{strings.network.chooseSavedPhone}</option>
              {bindings.map((value, index) => <option key={value} value={value}>{strings.network.savedPhoneNumber(index + 1)}</option>)}
            </select>
          </label>}
          <button className="primary-button" type="button" disabled={busy || (bindings.length > 1 && !bindingReference)} onClick={() => onConnect(bindingReference || undefined)}>
            {bindings.length === 0 ? strings.network.firstSetupAction : strings.app.connectPhone}
          </button>
          <button className="text-button" type="button" disabled={busy} onClick={() => onNetworkRecoveryChange(true)}>{strings.network.cannotFindPhone}</button>
        </>}
      </section>}
      {!setupWizard && !receiverManagerOnly && networkRecoveryMode && <button className="text-button network-recovery-back" type="button" onClick={() => onNetworkRecoveryChange(false)}>{strings.network.backToSimpleConnection}</button>}
      {!setupWizard && (receiverManagerOnly || networkRecoveryMode) && <>
      <div className="network-primary">
        {!receiverManagerOnly && <div className="network-summary"><strong>{mainSummary}</strong></div>}
        {receiverManagerOnly && <section className="receiver-manager receiver-usb-card" aria-label={strings.network.receiverUsbManagementLabel}>
          <label className="network-field">{strings.network.authorizedOnlineUsb}
            <select required value={serial} disabled={busy} onChange={event => { invalidateBindingReset(); setSerial(event.target.value); onReceiverSerialChange(event.target.value); setLocalError(""); onError(""); }}>
              <option value="">{strings.network.chooseUsbPlaceholder}</option>
              {usbDevices.map(device => <option key={device.serial} value={device.serial}>{device.displayName}</option>)}
            </select>
          </label>
          <div className="receiver-status-row">
            <div className="receiver-status-copy" role="status" aria-live="polite">
              <strong>{candidateLoading || receiverAction === "checking" ? strings.network.checkingReceiver : receiverError ? strings.network.receiverCheckFailed : !serial ? strings.network.chooseAuthorizedUsb : !receiverStatus ? strings.network.waitingReceiverCheck : !receiverStatus.installed ? strings.network.receiverNotInstalled : receiverNeedsUpgrade ? strings.network.phoneAppUpdateAvailable : receiverAlreadyCurrent ? strings.network.phoneAppCurrent : receiverStatus.serviceActive ? receiverStatus.healthy ? strings.network.receiverHealthy : strings.network.receiverUnhealthy : strings.network.receiverStopped}</strong>
              {receiverStatus?.installed && !receiverNeedsUpgrade && !receiverAlreadyCurrent && <small>{strings.network.receiverInstalled}</small>}
              {!candidateLoading && !updateCandidate && <small>{updateCandidateError ? strings.app.operationNotCompleted : strings.network.localPackageUnavailable}</small>}
              {usbDevices.length === 0 && <small>{unauthorizedUsb ? strings.network.usbDetectedUnauthorized : strings.network.noAuthorizedUsb}</small>}
              {updateCandidateError && <ErrorDetails summary={strings.app.operationNotCompleted} details={updateCandidateError} />}
            </div>
            <div className="receiver-status-actions">
              <button className="dialog-secondary" type="button" disabled={busy || !serial} onClick={() => void checkReceiver(serial)}>{receiverAction === "checking" ? strings.network.checkingReceiver : strings.network.checkStatus}</button>
              {canRequestUsbInstall && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{strings.network.installReceiver(updateCandidate.versionCode)}</button>}
              {canRequestUsbMaintenance && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{strings.network.usbMaintenanceInstall(updateCandidate.versionCode)}</button>}
              {canRequestUsbUpgrade && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("upgrade")}>{strings.network.upgradeReceiver(updateCandidate.versionCode)}</button>}
              <button className="dialog-secondary" type="button" disabled={busy} onClick={onRefreshDevices}>{strings.network.refreshUsbDeviceList}</button>
            </div>
          </div>
              {(updateCandidate || receiverOperationResult || receiverNeedsUpgrade) && <details className="receiver-version-details">
            <summary>{strings.network.versionDetails}</summary>
            {updateCandidate && <small>{strings.network.managerVersionSummary(updateCandidate.versionName, currentVersionLabel, updateCandidate.versionCode, (updateCandidate.sizeBytes / (1024 * 1024)).toFixed(1))}</small>}
            {receiverOperationResult && <small>{strings.network.usbOperationVersionVerified(receiverOperationResult.serial, receiverOperationResult.status.versionCode)}</small>}
          </details>}
          {receiverNeedsUpgrade && snapshot.sessionTransport === "lan" && bindings.length > 0 && <>
            <small>{bindingReference ? strings.network.selectedBindingAndUsbIdentity : strings.network.chooseSessionBinding}</small>
            <label className="network-field receiver-maintenance-binding">{strings.network.requiredSessionBinding}
              <select value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); setLocalError(""); onError(""); }}>
                <option value="">{strings.network.chooseSessionBindingOption}</option>
                {bindings.map((value, index) => <option key={value} value={value}>{strings.network.savedPhoneNumber(index + 1)}</option>)}
              </select>
            </label>
            {bindings.length > 1 && <BindingReferenceList bindings={bindings} />}
          </>}
          {receiverNeedsUpgrade && snapshot.sessionTransport !== "lan" && bindings.length > 0 && <details className="receiver-maintenance-details">
            <summary>{strings.network.upgradeTargetDetails}</summary>
            <label className="network-field receiver-maintenance-binding">{strings.network.optionalSavedBinding}
              <select value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); setLocalError(""); onError(""); }}>
                <option value="">{strings.network.noBindingUsbMaintenance}</option>
                {bindings.map((value, index) => <option key={value} value={value}>{strings.network.savedPhoneNumber(index + 1)}</option>)}
              </select>
            </label>
            {bindings.length > 1 && <BindingReferenceList bindings={bindings} />}
            {!bindingReference && <small>{strings.network.noBindingMaintenanceNote}</small>}
          </details>}
          {receiverError && <ErrorDetails className="receiver-error" summary={strings.app.operationNotCompleted} details={receiverError} />}
          <details className="pairing-repair-disclosure">
          <summary>{strings.network.pairingRepair}</summary>
          <section className="binding-reset" aria-label={strings.network.resetUsbPairing}>
            <div className="binding-reset-heading"><strong>{strings.network.resetPairing}</strong><small>{strings.network.resetOnlyCurrentUsb}</small></div>
            {!bindingResetPreview && !bindingResetResult && <button className="dialog-secondary" type="button" disabled={busy || !serial} onClick={() => void previewBindingReset(serial)}>
              {bindingResetLoading ? strings.network.readingResetPreview : strings.network.resetPairingUsbRequired}
            </button>}
            {bindingResetLoading && <p className="network-loading" role="status"><LoaderCircle className="spin" size={15} />{strings.network.readOnlyUsbBindingCheck}</p>}
            {bindingResetError && <div className="binding-reset-error"><ErrorDetails summary={strings.app.operationNotCompleted} details={bindingResetError} /><button className="dialog-secondary" type="button" disabled={busy || !serial} onClick={() => void previewBindingReset(serial, true)}>{strings.network.recheck}</button></div>}
            {bindingResetPreview && <div className="binding-reset-preview">
              <strong>{strings.network.readOnlyPreview} · {bindingResetPreview.serial}</strong>
              <dl>
                <div><dt>{strings.network.androidUser}</dt><dd>{bindingResetPreview.androidUser}</dd></div>
                <div><dt>{strings.network.bindingReference}</dt><dd>{bindingResetPreview.bindingReference}</dd></div>
                <div><dt>{strings.network.phoneRecord}</dt><dd>{resetPhoneStateLabel(bindingResetPreview.phoneState)} · {commitmentSummary(bindingResetPreview.phoneCommitment)}</dd></div>
                <div><dt>{strings.network.pcRecord}</dt><dd>{resetPcStateLabel(bindingResetPreview.pcState)} · {commitmentSummary(bindingResetPreview.pcCommitment)}</dd></div>
              </dl>
              {bindingResetPreview.phoneState === "saved" && bindingResetPreview.pcState === "saved" && bindingResetPreview.phoneCommitment !== bindingResetPreview.pcCommitment
                ? <p className="binding-reset-warning" role="alert">{strings.network.differentBindingSummaries}</p>
                : <p className="binding-reset-warning">{strings.network.resetConfirmationDetail}</p>}
              {bindingResetPreview.receiverBusy && <p className="binding-reset-warning" role="alert">{strings.network.receiverHasActiveSession}</p>}
              {(bindingResetPreview.phoneState === "pending" || bindingResetPreview.phoneState === "failed" || bindingResetPreview.phoneState === "unknown" || bindingResetPreview.pcState === "unknown") && <p className="binding-reset-warning" role="alert">{strings.network.resetUnknownState}</p>}
              <div className="binding-reset-actions">
                <button className="dialog-secondary" type="button" disabled={busy} onClick={() => void previewBindingReset(serial, true)}>{strings.network.recheck}</button>
                <button className="dialog-secondary" type="button" disabled={busy} onClick={invalidateBindingReset}>{strings.network.cancel}</button>
                <button className="primary-button binding-reset-confirm" type="button" disabled={busy || bindingResetPreview.receiverBusy || !["empty", "saved"].includes(bindingResetPreview.phoneState) || !["missing", "saved"].includes(bindingResetPreview.pcState)} onClick={() => void confirmBindingReset()}>{strings.network.confirmResetTarget}</button>
              </div>
            </div>}
            {bindingResetBusy && <p className="network-loading" role="status"><LoaderCircle className="spin" size={15} />{strings.network.resettingAndRefreshing}</p>}
            {bindingResetResult && <div className={`binding-reset-result ${bindingResetResult.outcome === "reset" || bindingResetResult.outcome === "alreadyEmpty" ? "is-complete" : "is-partial"}`} role={bindingResetResult.outcome === "reset" || bindingResetResult.outcome === "alreadyEmpty" ? "status" : "alert"}>
              <strong>{resetOutcomeLabel(bindingResetResult.outcome)}</strong>
              <dl>
                <div><dt>{strings.network.phoneRecord}</dt><dd>{resetPhoneStateLabel(bindingResetResult.phoneState)} · {commitmentSummary(bindingResetResult.phoneCommitment)}</dd></div>
                <div><dt>{strings.network.pcRecord}</dt><dd>{resetPcStateLabel(bindingResetResult.pcState)} · {commitmentSummary(bindingResetResult.pcCommitment)}</dd></div>
              </dl>
              <p>{bindingResetResult.reason}</p><p>{bindingResetResult.nextAction}</p>
              <button className="dialog-secondary" type="button" disabled={busy || !serial} onClick={() => void previewBindingReset(serial, true)}>{strings.network.recheck}</button>
            </div>}
          </section>
          </details>
        </section>}
        {pairedServiceStopped && <div className="network-recovery" role="status">
          <span>{strings.network.receiverServiceStopped}</span>
          <button type="button" className="text-button" disabled={busy} onClick={() => { methodManuallySelected.current = true; setMethod("bootstrap"); }}>{strings.network.startSelectedPhoneOverUsb}</button>
        </div>}
        {error ? <ErrorDetails id="network-flow-error" className="network-error" summary={needsReceiverUpdate(error) ? receiverErrorMessage(error) : strings.app.connectionNotCompleted} details={error} summaryRef={alertRef} />
          : localError && <p id="network-flow-error" ref={alertRef} className="inline-error network-error" role="alert" tabIndex={-1}>{receiverErrorMessage(localError)}</p>}
        {busy && !receiverOperationBusy && <p className="network-loading" role="status" aria-live="polite"><LoaderCircle className="spin" size={15} />{receiverAction === "checking" ? strings.network.checkingReceiver : candidateLoading ? strings.network.checkingBundledVersion : strings.network.readingNetworkInfo}</p>}
        {!receiverManagerOnly && udpMode && <p className="network-hint connect-choice-hint">{strings.network.connectChoiceHint}</p>}
        {!receiverManagerOnly && udpMode && <section className="receiver-manager offer-manager" aria-label={strings.network.offerListenerLabel}>
          <div className="receiver-status-row">
            <div className="receiver-status-copy" role="status" aria-live="polite">
              <strong>{offers.active ? strings.network.listeningAt(offers.interfaceAddress ?? strings.network.networkInterface) : strings.network.allowBoundPhone}</strong>
              <small>{offers.active
                ? offers.offers.length > 1 ? strings.network.multipleRequests : offers.offers.length === 1 ? strings.network.confirmRequestOnPc : strings.network.waitPhoneReceive
                : bindings.length === 0 ? strings.network.firstUseBindOverUsb : strings.network.phoneCanStartOneTap}</small>
            </div>
            {!offers.active && <button className="dialog-secondary" type="button" disabled={busy || audioSessionActive || !localAddress || !bindingReference} onClick={() => void onSetOfferListener(localAddress, true, bindingReference || undefined)}>{strings.network.startListening}</button>}
          </div>
          {offers.active && <>
            {offers.offers.length > 0
              ? <div className="candidate-list" aria-label={strings.network.unverifiedPhoneRequests}>
                  {offers.offers.map(offer => <button type="button" key={offer.id} disabled={busy || audioSessionActive} aria-pressed={selectedOffer?.id === offer.id} onClick={() => setSelectedOfferId(offer.id)}>
                    <span>{offer.endpoint}</span><small>{strings.network.unverifiedPhoneRequest(selectedOffer?.id === offer.id)}</small>
                  </button>)}
                </div>
              : <p className="network-hint">{strings.network.waitPhoneReceive}</p>}
            <p className="network-hint">{bindings.length === 0 ? strings.network.noSavedBinding
              : bindingReference ? strings.network.useBindingWhenAccepted(bindingLabel(bindingReference))
                : strings.network.multipleSavedBindings}</p>
            {offers.offers.length > 1 && !selectedOffer && <p className="network-hint">{strings.network.chooseRequestToAccept}</p>}
            <button className="primary-button" type="button" disabled={busy || audioSessionActive || !selectedOffer || !bindingReference} onClick={() => selectedOffer && onAcceptOffer(selectedOffer.id, bindingReference)}>{strings.network.acceptSelectedPhone}</button>
            {audioSessionActive && <p className="network-hint">{strings.network.cannotAcceptDuringAudio}</p>}
          </>}
        </section>}
      </div>
      <div className="network-options">
          {!setupWizard && udpMode && <details className="background-trust-disclosure">
            <summary>{strings.network.backgroundTrustTitle}</summary>
            <section className="background-trust" aria-label={strings.network.backgroundTrustTitle}>
            {backgroundTrustTarget ? <>
              <small>{backgroundTrustTarget.target === "activeSession"
                ? strings.network.backgroundTrustScopeActive(backgroundTrustTarget.sessionGeneration)
                : strings.network.backgroundTrustScopeReceiver(backgroundTrustTarget.localAddress, backgroundTrustTarget.peer)}</small>
              <small>{bindingLabel(backgroundTrustTarget.bindingReference)}</small>
              <div className="background-trust-actions">
                <button className="dialog-secondary" type="button" disabled={busy} onClick={() => void operateBackgroundTrust(false)}>{strings.network.backgroundTrustQuery}</button>
                <button className="dialog-secondary" type="button" disabled={busy} onClick={() => setConfirmBackgroundTrustRevoke(true)}>{strings.network.backgroundTrustRevoke}</button>
              </div>
              {backgroundTrust && <div className="background-trust-result" role="status" aria-live="polite">
                <span>{backgroundTrustKindLabel(backgroundTrust.value.kind)} · {backgroundTrustPersistenceLabel(backgroundTrust.value.persistence)}</span>
                <small>{backgroundTrustRemaining(backgroundTrust.value.remainingMs)}</small>
                <small>{strings.network.backgroundTrustVerifiedAt(new Date(backgroundTrust.checkedAt).toLocaleTimeString())}</small>
                {backgroundTrust.value.kind === "none" && backgroundTrust.value.target === "activeSession" && !audioSessionActive && <small>{strings.network.backgroundTrustRecent}</small>}
                {backgroundTrustNotice && <small>{backgroundTrustNotice}</small>}
              </div>}
              {confirmBackgroundTrustRevoke && <div className="background-trust-confirm" role="group" aria-label={strings.network.backgroundTrustConfirmTitle}>
                <p>{strings.network.backgroundTrustConfirmBody(
                  backgroundTrustTarget.target === "activeSession"
                    ? strings.network.backgroundTrustScopeActive(backgroundTrustTarget.sessionGeneration)
                    : strings.network.backgroundTrustScopeReceiver(backgroundTrustTarget.localAddress, backgroundTrustTarget.peer),
                  bindingReferenceLabel(backgroundTrustTarget.bindingReference))}</p>
                <div className="background-trust-actions">
                  <button className="dialog-secondary" type="button" disabled={busy} onClick={() => setConfirmBackgroundTrustRevoke(false)}>{strings.network.backgroundTrustCancel}</button>
                  <button className="primary-button" type="button" disabled={busy} onClick={() => void operateBackgroundTrust(true)}>{strings.network.backgroundTrustConfirm}</button>
                </div>
              </div>}
            </> : backgroundTrust?.value.target === "activeSession" && !audioSessionActive
              ? <div className="background-trust-result" role="status" aria-live="polite">
                  <strong>{backgroundTrustKindLabel(backgroundTrust.value.kind)} · {backgroundTrustPersistenceLabel(backgroundTrust.value.persistence)}</strong>
                  <small>{strings.network.backgroundTrustScopeActive(backgroundTrust.value.sessionGeneration ?? 0)} · {backgroundTrust.value.bindingReference}</small>
                  <small>{backgroundTrustRemaining(backgroundTrust.value.remainingMs)}</small>
                  <small>{strings.network.backgroundTrustVerifiedAt(new Date(backgroundTrust.checkedAt).toLocaleTimeString())}</small>
                  <small>{strings.network.backgroundTrustRecent}</small>
                  {backgroundTrustNotice && <small>{backgroundTrustNotice}</small>}
                </div>
              : <small>{strings.network.backgroundTrustNoTarget}</small>}
            {backgroundTrustBusy && <small role="status">{strings.network.checking}</small>}
            {backgroundTrustError && <ErrorDetails summary={strings.network.backgroundTrustUnknown} details={backgroundTrustError} />}
            </section>
          </details>}
          {!receiverManagerOnly && !backgroundStartMode && <div className="network-methods" role="group" aria-label={strings.network.receiverConnectionMode}>
              <button type="button" aria-pressed={method === "bootstrap"} disabled={busy || backgroundStartMode} onClick={() => { methodManuallySelected.current = true; setMethod("bootstrap"); onError(""); setLocalError(""); }}>{strings.network.usbFirstBinding}</button>
            <button type="button" aria-pressed={method === "paired"} disabled={busy || bindings.length === 0} onClick={() => { methodManuallySelected.current = true; setMethod("paired"); onError(""); setLocalError(""); }}>{strings.network.connectPairedPhone}</button>
          </div>}
          {!receiverManagerOnly && snapshot.connectionTransport === "wireless" && snapshot.settings.wirelessProtocol === "adb" && <div className="network-entry-actions">
            <button className="text-button" type="button" disabled={busy} onClick={onAddPhone}>{strings.app.addPhone}</button>
          </div>}
          {!receiverManagerOnly && <label className="network-field">{strings.network.computerInterface}
            <select required value={localAddress} disabled={busy || offers.active} onChange={event => { setLocalAddress(event.target.value); setCandidates([]); if (backgroundStartMode || !peerManuallyEdited.current) { peerSelectionSource.current = null; peerManuallyEdited.current = false; setPeer(""); } backgroundChoiceInvalidated.current.interfaceAddress = false; setLocalError(""); onError(""); }} aria-invalid={!!shownError && !localAddress || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
              <option value="">{strings.network.chooseIpv4Interface}</option>
              {interfaces.map(value => <option key={`${value.index}-${value.address}`} value={value.address}>{value.name} · {value.address}/{value.prefixLength}</option>)}
            </select>
          </label>}
          {method === "bootstrap" ? <>
            {!receiverManagerOnly && <label className="network-field">{strings.network.authorizedOnlineUsb}
              <select required value={serial} disabled={busy} onChange={event => { invalidateBindingReset(); setSerial(event.target.value); onReceiverSerialChange(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !serial || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
                <option value="">{strings.network.chooseUsbPlaceholder}</option>
                {usbDevices.map(device => <option key={device.serial} value={device.serial}>{device.displayName}</option>)}
              </select>
            </label>}
            {!receiverManagerOnly && serial && <section className="receiver-manager" aria-label={strings.network.receiverManagement}>
              <div className="receiver-status-row">
                <div className="receiver-status-copy" role="status" aria-live="polite">
                  <strong>{receiverOperationBusy ? strings.network.installingAndVerifying : receiverAction === "checking" ? strings.network.checkingReceiver : receiverError ? strings.network.receiverCheckFailed : !receiverStatus ? strings.network.waitingReceiverCheck : !receiverStatus.installed ? strings.network.receiverNotInstalled : receiverStatus.serviceActive ? receiverStatus.healthy ? strings.network.receiverHealthy : strings.network.receiverUnhealthy : strings.network.receiverStopped}</strong>
                  {receiverStatus?.installed && <small>{[receiverStatus.versionName, receiverStatus.versionCode === null ? "" : strings.network.versionCode(receiverStatus.versionCode), receiverStatus.foreground ? strings.network.foregroundService : ""].filter(Boolean).join(" · ")}</small>}
                  {receiverStatus?.audioActive && <small>{strings.network.audioActiveNotPlaying}</small>}
                  {receiverStatus && !receiverStatus.healthy && <ErrorDetails summary={strings.app.operationNotCompleted} details={receiverStatus.error || receiverStatus.message} />}
                  {!receiverManagerOnly && receiverNeedsUpgrade && <small>{strings.network.oldCompatibleUsbUpgrade}</small>}
                </div>
                <div className="receiver-status-actions">
                  <button className="dialog-secondary" type="button" disabled={busy} onClick={() => void checkReceiver(serial)}>{receiverAction === "checking" ? strings.network.checking : strings.network.checkStatus}</button>
                  {!receiverManagerOnly && canRequestUsbInstall && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{strings.network.installReceiver(updateCandidate.versionCode)}</button>}
                  {!receiverManagerOnly && receiverNeedsUpgrade && <button className="primary-button" type="button" disabled={busy} onClick={onOpenReceiverManager}>{strings.network.usbUpgradeOptions}</button>}
                </div>
              </div>
              {receiverError && <ErrorDetails className="receiver-error" summary={needsReceiverUpdate(receiverError) ? receiverErrorMessage(receiverError) : strings.app.operationNotCompleted} details={receiverError} summaryRef={alertRef} />}
              {!receiverManagerOnly && receiverNeedsInstall && <p className="network-note">{strings.network.installPackageFirstVerify}</p>}
              {!receiverManagerOnly && receiverNeedsUpgrade && <button type="button" className="text-button" disabled={busy} onClick={onOpenReceiverManager}>{strings.network.openUsbUpgradeManager}</button>}
            </section>}
            {!receiverManagerOnly && <label className="network-field">{strings.network.phoneCurrentIpv4}
              <select required value={receiverAddress} disabled={busy || !serial || receiverStatus?.installed !== true} onChange={event => { setReceiverAddress(event.target.value); setLocalError(""); onError(""); }} aria-invalid={!!shownError && !receiverAddress || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
                <option value="">{strings.network.choosePhoneInterface}</option>
                {compatibleReceiverInterfaces.map(value => <option key={`${value.interfaceIndex}-${value.address}`} value={value.address}>{value.interfaceName} · {value.address}/{value.prefixLength}</option>)}
              </select>
            </label>}
            {!receiverManagerOnly && <p className="network-note">{strings.network.onlySameSubnetPhoneAddresses}</p>}
          </> : <>
            <label className="network-field">{strings.network.pairedReceiver}
              <select required value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); backgroundChoiceInvalidated.current.bindingReference = false; setLocalError(""); onError(""); }} aria-invalid={!!shownError && !bindingReference || undefined} aria-describedby={shownError ? "network-flow-error" : undefined}>
                <option value="">{strings.network.chooseBinding}</option>
                {bindings.map((value, index) => <option key={value} value={value}>{strings.network.savedPhoneNumber(index + 1)}</option>)}
              </select>
            </label>
            <BindingReferenceList bindings={bindings} />
            <label className="network-field">{strings.network.receiverIpv4Endpoint}
              <input required value={peer} disabled={busy} onChange={event => { peerManuallyEdited.current = true; peerSelectionSource.current = "manual"; setPeer(event.target.value); setLocalError(""); onError(""); }} placeholder="192.0.2.20:45678" autoComplete="off" inputMode="decimal" aria-invalid={!!shownError && !isIpv4Endpoint(peer) || peerOutsideSubnet || undefined} aria-describedby={`network-peer-hint${peerOutsideSubnet ? " network-peer-network-warning" : ""}${shownError ? " network-flow-error" : ""}`} />
            </label>
            <small id="network-peer-hint" className="network-hint">{strings.network.addressSuggestionNeedsConnection}</small>
            {snapshot.settings.lastReceiverEndpoint && <small className="network-hint">{strings.network.lastReceiverHint(snapshot.settings.lastReceiverEndpoint)}</small>}
            {(snapshot.presence ?? []).map(entry => <small className="network-hint" key={entry.bindingReference}>{strings.network.presenceLine(entry.online, entry.lastAddress, entry.lastSeenUnix)}</small>)}
            {peerOutsideSubnet && <small id="network-peer-network-warning" className="network-validation-warning">{strings.network.invalidSubnetConnectDisabled}</small>}
            <p className="network-hint" role="status" aria-live="polite">{addressNotice}</p>
            <button className="text-button network-refresh" type="button" onClick={() => void discoverCandidates()} disabled={busy || !localAddress}><Search size={15} />{strings.network.searchAddressAgain}</button>
            {candidates.length > 0 && <div className="candidate-list" aria-label={strings.network.unverifiedDiscoveryCandidates}>{candidates.map(candidate => <button type="button" key={candidate.endpoint} disabled={busy} aria-pressed={peer === candidate.endpoint} onClick={() => { peerManuallyEdited.current = true; peerSelectionSource.current = "candidate"; setPeer(candidate.endpoint); setLocalError(""); onError(""); }}><span>{candidate.endpoint}</span><small>{candidate.bindingReference ? strings.network.verifiedCandidate : strings.network.unverifiedCandidate}</small></button>)}</div>}
          </>}
          {!receiverManagerOnly && <button className="text-button network-refresh" type="button" onClick={() => void loadNetworkInfo()} disabled={busy}><Search size={15} />{strings.network.refreshInterfacesAndBindings}</button>}
          {!receiverManagerOnly && <button className="primary-button network-connect" disabled={busy || !canStart}>{disabled ? strings.network.processing : backgroundStartMode ? strings.app.backgroundStartConnect : method === "bootstrap" ? strings.network.firstBindAndConnect : strings.network.connectReceiverDirectly}</button>}
          </div>
      </>}
      {receiverProgressStage && <div className={`receiver-operation-progress${receiverProgressStage === "failed" ? " is-failed" : ""}`} role={receiverProgressStage === "failed" ? "alert" : "status"} aria-live={receiverProgressStage === "failed" ? "assertive" : "polite"}>
        <span>{receiverProgressLabel}</span>
        {(receiverProgressStage === "checking" || receiverProgressStage === "transferring" || receiverProgressStage === "verifying") && <progress aria-label={receiverProgressLabel} />}
      </div>}
      {!receiverManagerOnly && needsReceiverUpdate([shownError, receiverError, bindingResetError].join(" ")) && <button className="primary-button" type="button" disabled={busy} onClick={onOpenReceiverManager}>{strings.app.upgradePhone}</button>}
    </form>
  </Modal>;
}
