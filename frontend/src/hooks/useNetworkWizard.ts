import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { listen } from "@tauri-apps/api/event";
import { strings } from "../strings";
import { isDesktop, invokeApp, hasActiveSession, endpointFitsInterface } from "../appRuntime";
import type { Snapshot, PrivateIpv4Interface, NetworkCandidate, HostDiscoverySnapshot, NetworkStartRequest, OperationResult, Panel, ConfirmRequest } from "../appTypes";

/** Wireless (LAN) connection flow: network dialog state, starting/cancelling a network start, and the
 * phone connection-offer listener. */
export function useNetworkWizard({ active, apply, cancelOperationLockRef, cleanupPending, notify, panel, panelRef, setCancelPending, setConfirm, setNetworkDialogError, setPanel, setSnapshot, setWorking, snapshot, snapshotRefreshGenerationRef, wirelessUdpMode, working }: {
  active: boolean;
  apply: (value: Snapshot) => void;
  cancelOperationLockRef: RefObject<boolean>;
  cleanupPending: boolean;
  notify: (message: string) => void;
  panel: Panel | null;
  panelRef: RefObject<Panel | null>;
  setCancelPending: (value: boolean) => void;
  setConfirm: (value: ConfirmRequest | null) => void;
  setNetworkDialogError: (value: string) => void;
  setPanel: (panel: Panel | null) => void;
  setSnapshot: (value: Snapshot) => void;
  setWorking: (value: boolean) => void;
  snapshot: Snapshot | null;
  snapshotRefreshGenerationRef: RefObject<number>;
  wirelessUdpMode: boolean;
  working: boolean;
}) {
  const [pendingNetworkStart, setPendingNetworkStart] = useState<NetworkStartRequest | null>(null);
  const [backgroundStartMode, setBackgroundStartMode] = useState(false);
  const [networkRecoveryMode, setNetworkRecoveryMode] = useState(false);
  const [backgroundChoicePrefill, setBackgroundChoicePrefill] = useState({ localAddress: "", bindingReference: "" });
  const [networkChoiceNeedsConfirmation, setNetworkChoiceNeedsConfirmation] = useState({ interfaceAddress: false, bindingReference: false });
  const [mainListenerPending, setMainListenerPending] = useState(false);
  const [networkStartPending, setNetworkStartPending] = useState(false);
  const networkStartPendingRef = useRef(false);
  const networkStartGeneration = useRef(0);
  const networkStartPanel = useRef(panel);
  const lastOfferListenerChoice = useRef<{ localAddress: string; bindingReference: string } | null>(null);
  const mainListenerGenerationRef = useRef(0);
  const offerListenerGeneration = useRef(0);
  const offerListenerQueue = useRef<Promise<void>>(Promise.resolve());
  const offerListenerChoice = useRef<{ localAddress: string; bindingReference: string } | null>(null);
  const retainOfferListenerOnClose = useRef(false);
  const seenOfferIds = useRef(new Set<string>());
  useEffect(() => {
    if (!isDesktop || (panel !== "network" && !mainListenerPending && !snapshot?.wirelessConnectionOffers.active)) return;
    let disposed = false;
    const refreshOffers = async () => {
      const generation = ++snapshotRefreshGenerationRef.current;
      try {
        const next = await invokeApp<Snapshot>("get_snapshot");
        if (!disposed && generation === snapshotRefreshGenerationRef.current) setSnapshot(next);
      } catch (error) {
        if (!disposed && generation === snapshotRefreshGenerationRef.current) setNetworkDialogError(String(error));
      }
    };
    void refreshOffers();
    // The offer listener announces new and expired offers; no polling needed.
    const unlisten = listen("anpod://offers-changed", () => void refreshOffers());
    return () => { disposed = true; void unlisten.then(stop => stop()); snapshotRefreshGenerationRef.current++; };
  }, [panel, mainListenerPending, snapshot?.wirelessConnectionOffers.active, setSnapshot, setNetworkDialogError, snapshotRefreshGenerationRef]);
  const openNetworkDialog = (backgroundStart = false, choices = { localAddress: "", bindingReference: "" }, recovery = true) => {
    retainOfferListenerOnClose.current = false;
    setBackgroundStartMode(backgroundStart);
    setBackgroundChoicePrefill(choices);
    setNetworkRecoveryMode(recovery);
    setNetworkDialogError("");
    setPanel("network");
  };
  const openConnectionFlow = () => openNetworkDialog(true, { localAddress: "", bindingReference: "" }, !!snapshot?.wirelessConnectionOffers.active);
  useEffect(() => {
    const offers = snapshot?.wirelessConnectionOffers;
    if (!offers?.active || !offers.offers.length) return;
    if (panel === "network") {
      offers.offers.forEach(offer => seenOfferIds.current.add(offer.id));
      return;
    }
    if (panel !== null) return;
    const unseen = offers.offers.some(offer => !seenOfferIds.current.has(offer.id));
    if (!unseen) return;
    offers.offers.forEach(offer => seenOfferIds.current.add(offer.id));
    openNetworkDialog(false, { localAddress: offers.interfaceAddress ?? "", bindingReference: offers.automaticBinding ?? "" }, true);
  }, [snapshot?.wirelessConnectionOffers, panel]);
  useEffect(() => {
    if (panel === null) return;
    mainListenerGenerationRef.current++;
  }, [panel]);
  useEffect(() => {
    if (networkStartPending && panel !== networkStartPanel.current) void cancelNetworkStart();
  }, [panel, networkStartPending]);
  const startNetworkOperation = async (operation: () => Promise<Snapshot>) => {
    const generation = ++networkStartGeneration.current;
    networkStartPendingRef.current = true;
    networkStartPanel.current = panelRef.current;
    setNetworkStartPending(true);
    setWorking(true);
    try {
      const next = await operation();
      if (generation !== networkStartGeneration.current) return null;
      apply(next);
      return next;
    } catch (error) {
      if (generation !== networkStartGeneration.current) return null;
      throw error;
    } finally {
      if (generation === networkStartGeneration.current) {
        networkStartPendingRef.current = false;
        setNetworkStartPending(false);
        setWorking(false);
      }
    }
  };

  async function startNetworkAudio(request: NetworkStartRequest) {
    const generation = networkStartGeneration.current + 1;
    try {
      return await startNetworkOperation(
        () => invokeApp<Snapshot>("start_network_audio", { ...request }),
      );
    } catch (error) {
      if (!String(error).includes("receiver_requires_connection_offer:")) throw error;
      if (generation !== networkStartGeneration.current) return null;
      const latest = await invokeApp<Snapshot>("get_snapshot");
      if (generation !== networkStartGeneration.current) return null;
      if (panelRef.current !== networkStartPanel.current) return null;
      apply(latest);
      if (latest.state === "error" && (latest.canDisconnect || latest.message === "网络启动失败，电脑声音恢复失败"))
        throw new Error(strings.app.cleanupBeforeNetworkStart);
      if (latest.busy || !request.bindingReference || request.bootstrapSerial) throw error;
      openNetworkDialog(true, { localAddress: request.localAddress, bindingReference: request.bindingReference }, true);
      const listenerGeneration = offerListenerGeneration.current + 1;
      setMainListenerPending(true);
      try { await setOfferListener(request.localAddress, true, request.bindingReference); }
      finally { setMainListenerPending(false); }
      if (generation !== networkStartGeneration.current || listenerGeneration !== offerListenerGeneration.current) return null;
      if (panelRef.current !== "network") return null;
      return null;
    }
  };

  const cancelNetworkStart = async () => {
    if (!networkStartPendingRef.current || cancelOperationLockRef.current) return;
    networkStartGeneration.current++;
    networkStartPendingRef.current = false;
    cancelOperationLockRef.current = true;
    setNetworkStartPending(false);
    setCancelPending(true);
    try {
      const cancelled = await invokeApp<Snapshot>("cancel_operation");
      if (hasActiveSession(cancelled)) {
        const result = await invokeApp<OperationResult>("disconnect");
        apply(result.snapshot);
        if (!result.success) notify(result.message);
      } else apply(cancelled);
    }
    catch (error) { notify(String(error)); }
    finally { cancelOperationLockRef.current = false; setCancelPending(false); setWorking(false); }
  };
  const startNetwork = async (request: NetworkStartRequest) => {
    setNetworkDialogError("");
    if (cleanupPending) { setNetworkDialogError(strings.app.cleanupBeforeNetworkStart); return; }
    if (active) {
      setPendingNetworkStart(request);
      setConfirm({ kind: "network", value: "" });
      return;
    }
    try {
      const started = await startNetworkAudio(request);
      if (!started) return;
      if (request.bindingReference)
        lastOfferListenerChoice.current = { localAddress: request.localAddress, bindingReference: request.bindingReference };
      setPanel(null);
    } catch (error) {
      if (panelRef.current === "network" || panelRef.current === "setup") setNetworkDialogError(String(error));
      else if (panelRef.current === null && networkStartPanel.current === null) {
        openNetworkDialog(true, { localAddress: request.localAddress, bindingReference: request.bindingReference ?? "" }, true);
        setNetworkDialogError(String(error));
      }
    }
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
        const started = await startNetworkAudio({
          localAddress: choices.localAddress, peer: candidates[0].endpoint,
          // A phone that answered discovery is awake; do not demand the standby capability (that is only for the explicit background flow).
          bindingReference: choices.bindingReference, backgroundAuthorizationRequired: false,
        });
        if (!started) return;
        lastOfferListenerChoice.current = { localAddress: choices.localAddress, bindingReference: choices.bindingReference };
        setPanel(null);
      } catch (error) {
        if (panelRef.current !== "network" && panelRef.current !== "setup") return;
        openNetworkDialog(true, choices, true);
        setNetworkDialogError(String(error));
      }
    } catch (error) {
      if (panelRef.current !== "network" && panelRef.current !== "setup") return;
      openNetworkDialog(true, { localAddress: "", bindingReference: "" }, true);
      setNetworkDialogError(String(error));
    } finally { setWorking(false); }
  };

  const queueOfferListener = <T,>(operation: () => Promise<T>) => {
    const result = offerListenerQueue.current.then(operation, operation);
    offerListenerQueue.current = result.then(() => undefined, () => undefined);
    return result;
  };
  const stopOfferListener = () => {
    retainOfferListenerOnClose.current = false;
    offerListenerGeneration.current++;
    offerListenerChoice.current = null;
    return queueOfferListener(async () => {
      try {
        await invokeApp<HostDiscoverySnapshot>("set_wireless_connection_offers", { localAddress: "", allowed: false });
        const generation = ++snapshotRefreshGenerationRef.current;
        const next = await invokeApp<Snapshot>("get_snapshot");
        if (generation === snapshotRefreshGenerationRef.current) apply(next);
        return true;
      } catch (error) {
        if (panel === "network") setNetworkDialogError(String(error)); else notify(String(error));
        return false;
      }
    });
  };
  const setOfferListener = async (localAddress: string, allowed: boolean, bindingReference?: string, silent = false): Promise<boolean> => {
    if (!snapshot) return false;
    if (!allowed) return panel === null || retainOfferListenerOnClose.current || active ? true : stopOfferListener();
    if (active || working || snapshot.busy) return false;
    const generation = ++offerListenerGeneration.current;
    setNetworkDialogError("");
    try {
      const choice = await resolveWirelessChoice(localAddress, bindingReference);
      if (generation !== offerListenerGeneration.current) return false;
      const selectedAddress = choice.selectedAddress;
      const selectedBinding = choice.selectedBinding;
      if (!selectedBinding && choice.availableBindings.length === 0) {
        if (silent) setNetworkDialogError(strings.network.firstUseBindOverUsb); else setPanel("setup");
        return false;
      }
      if (!selectedAddress || !selectedBinding) {
        setNetworkChoiceNeedsConfirmation({ interfaceAddress: choice.addressChoiceInvalidated, bindingReference: choice.bindingChoiceInvalidated });
        if (silent) setNetworkDialogError(strings.network.chooseComputerInterface); else openNetworkDialog(false);
        return false;
      }
      return await queueOfferListener(async () => {
        if (generation !== offerListenerGeneration.current) return false;
        const current = offerListenerChoice.current;
        if (current && (current.localAddress !== selectedAddress || current.bindingReference !== selectedBinding)) {
          await invokeApp<HostDiscoverySnapshot>("set_wireless_connection_offers", { localAddress: "", allowed: false });
          offerListenerChoice.current = null;
        }
        if (generation !== offerListenerGeneration.current) return false;
        if (snapshot.wirelessConnectionOffers.active
          && snapshot.wirelessConnectionOffers.interfaceAddress === selectedAddress
          && snapshot.wirelessConnectionOffers.automaticBinding === selectedBinding) {
          offerListenerChoice.current = { localAddress: selectedAddress, bindingReference: selectedBinding };
          return true;
        }
        for (let attempts = 0; attempts < 6; attempts++) {
          if (generation !== offerListenerGeneration.current) return false;
          setWorking(true);
          try {
            await invokeApp<HostDiscoverySnapshot>("set_wireless_connection_offers", {
              localAddress: selectedAddress, allowed: true, bindingReference: selectedBinding,
            });
            if (generation !== offerListenerGeneration.current) return false;
            offerListenerChoice.current = { localAddress: selectedAddress, bindingReference: selectedBinding };
            lastOfferListenerChoice.current = offerListenerChoice.current;
            setNetworkChoiceNeedsConfirmation({ interfaceAddress: false, bindingReference: false });
            const refreshGeneration = ++snapshotRefreshGenerationRef.current;
            const next = await invokeApp<Snapshot>("get_snapshot");
            if (refreshGeneration === snapshotRefreshGenerationRef.current) apply(next);
            return true;
          } catch (error) {
            if (generation !== offerListenerGeneration.current) return false;
            if (attempts === 5) {
              if (panel === "network") setNetworkDialogError(String(error)); else if (!silent) notify(String(error));
              return false;
            }
          } finally { setWorking(false); }
          await new Promise<void>(resolve => window.setTimeout(resolve, 2000));
        }
        return false;
      });
    } catch (error) {
      if (panel === "network") setNetworkDialogError(String(error)); else if (!silent) notify(String(error));
      return false;
    }
  };
  useEffect(() => {
    const choice = offerListenerChoice.current;
    if (!choice || !snapshot || active || snapshot.busy || !wirelessUdpMode || snapshot.wirelessConnectionOffers.active) return;
    setMainListenerPending(true);
    void setOfferListener(choice.localAddress, true, choice.bindingReference, true).finally(() => setMainListenerPending(false));
  }, [snapshot?.wirelessConnectionOffers.active, active, snapshot?.busy, wirelessUdpMode]);

  const startMainScreenWaiting = async () => {
    if (!snapshot || active || working || snapshot.busy) return;
    const generation = ++mainListenerGenerationRef.current;
    try {
      const choice = await resolveWirelessChoice("", undefined);
      if (generation !== mainListenerGenerationRef.current) return;
      if (!choice.selectedBinding && choice.availableBindings.length === 0) { setPanel("setup"); return; }
      if (!choice.selectedAddress || !choice.selectedBinding) {
        setNetworkChoiceNeedsConfirmation({ interfaceAddress: choice.addressChoiceInvalidated, bindingReference: choice.bindingChoiceInvalidated });
        openNetworkDialog(false, { localAddress: choice.selectedAddress, bindingReference: choice.selectedBinding ?? "" }, true);
        return;
      }
      setWorking(true);
      let candidates: NetworkCandidate[];
      try {
        const local = choice.availableInterfaces.find(value => value.address === choice.selectedAddress);
        candidates = (await invokeApp<NetworkCandidate[]>("discover_network_receivers", { localAddress: choice.selectedAddress }))
          .filter(candidate => endpointFitsInterface(candidate.endpoint, local));
      } catch (error) {
        if (generation !== mainListenerGenerationRef.current) return;
        openNetworkDialog(true, { localAddress: choice.selectedAddress, bindingReference: choice.selectedBinding }, true);
        setNetworkDialogError(String(error));
        return;
      } finally { setWorking(false); }
      if (generation !== mainListenerGenerationRef.current) return;
      if (candidates.length === 1) {
        lastOfferListenerChoice.current = { localAddress: choice.selectedAddress, bindingReference: choice.selectedBinding };
        void startNetwork({ localAddress: choice.selectedAddress, peer: candidates[0].endpoint, bindingReference: choice.selectedBinding, backgroundAuthorizationRequired: false });
        return;
      }
      if (candidates.length > 1) {
        openNetworkDialog(true, { localAddress: choice.selectedAddress, bindingReference: choice.selectedBinding }, true);
        return;
      }
      setMainListenerPending(true);
      void setOfferListener(choice.selectedAddress, true, choice.selectedBinding).finally(() => setMainListenerPending(false));
    } catch (error) {
      if (generation === mainListenerGenerationRef.current) notify(String(error));
    }
  };

  const acceptOffer = async (offerId: string, bindingReference: string) => {
    if (!snapshot || active || working || snapshot.busy || !bindingReference) return;
    setNetworkDialogError("");
    try {
      const next = await startNetworkOperation(
        () => invokeApp<Snapshot>("accept_wireless_connection_offer", { offerId, bindingReference }),
      );
      if (!next) return;
      setPanel(null);
    } catch (error) { setNetworkDialogError(String(error)); }
  };

  return { acceptOffer, backgroundChoicePrefill, backgroundStartMode, cancelNetworkStart, lastOfferListenerChoice, mainListenerGenerationRef, mainListenerPending, networkChoiceNeedsConfirmation, networkRecoveryMode, networkStartPending, networkStartPendingRef, offerListenerChoice, offerListenerGeneration, openConnectionFlow, openNetworkDialog, pendingNetworkStart, retainOfferListenerOnClose, setBackgroundStartMode, setMainListenerPending, setNetworkRecoveryMode, setOfferListener, setPendingNetworkStart, startBackgroundConnection, startMainScreenWaiting, startNetwork, startNetworkAudio, stopOfferListener };
}
