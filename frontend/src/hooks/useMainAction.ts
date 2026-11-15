import { useRef } from "react";
import type { RefObject } from "react";
import { invokeApp } from "../appRuntime";
import type { Device, Snapshot, Panel } from "../appTypes";
import type { useNetworkWizard } from "./useNetworkWizard";
import type { useReceiverInstall } from "./useReceiverInstall";

/** Main button and direct connect/disconnect actions, including the cancellable connection start. */
export function useMainAction({ active, apply, cancelNetworkStart, cancelOperationLockRef, cleanupPending, dismissNotice, mainListenerGenerationRef, mainListenerPending, networkStartPending, notify, offerListenerChoice, openAoaDialog, openNetworkDialog, pairedPhoneCount, receiverOperationBusy, selected, setBackgroundStartMode, setCancelPending, setMainListenerPending, setPanel, setWorking, snapshot, startMainScreenWaiting, stopOfferListener, wiredMode, wirelessUdpMode, working }: {
  active: boolean;
  apply: (value: Snapshot) => void;
  cancelNetworkStart: ReturnType<typeof useNetworkWizard>["cancelNetworkStart"];
  cancelOperationLockRef: RefObject<boolean>;
  cleanupPending: boolean;
  dismissNotice: () => void;
  mainListenerGenerationRef: ReturnType<typeof useNetworkWizard>["mainListenerGenerationRef"];
  mainListenerPending: ReturnType<typeof useNetworkWizard>["mainListenerPending"];
  networkStartPending: ReturnType<typeof useNetworkWizard>["networkStartPending"];
  notify: (message: string) => void;
  offerListenerChoice: ReturnType<typeof useNetworkWizard>["offerListenerChoice"];
  openAoaDialog: (disconnectCurrent?: boolean) => Promise<void>;
  openNetworkDialog: ReturnType<typeof useNetworkWizard>["openNetworkDialog"];
  pairedPhoneCount: number | null;
  receiverOperationBusy: ReturnType<typeof useReceiverInstall>["receiverOperationBusy"];
  selected: Device | undefined;
  setBackgroundStartMode: ReturnType<typeof useNetworkWizard>["setBackgroundStartMode"];
  setCancelPending: (value: boolean) => void;
  setMainListenerPending: ReturnType<typeof useNetworkWizard>["setMainListenerPending"];
  setPanel: (panel: Panel | null) => void;
  setWorking: (value: boolean) => void;
  snapshot: Snapshot | null;
  startMainScreenWaiting: ReturnType<typeof useNetworkWizard>["startMainScreenWaiting"];
  stopOfferListener: ReturnType<typeof useNetworkWizard>["stopOfferListener"];
  wiredMode: Snapshot["settings"]["wiredMode"];
  wirelessUdpMode: boolean;
  working: boolean;
}) {
  const cancellableConnectionStart = useRef(false);
  const connectionStartGeneration = useRef(0);
  const cancelPendingConnection = async () => {
    if (cancelOperationLockRef.current) return;
    cancelOperationLockRef.current = true;
    setCancelPending(true);
    try {
      const cancelled = await invokeApp<Snapshot>("cancel_operation");
      connectionStartGeneration.current++;
      cancellableConnectionStart.current = false;
      setWorking(false);
      apply(cancelled);
    } catch (error) { throw error; }
    finally { cancelOperationLockRef.current = false; setCancelPending(false); }
  };
  const setOutput = async (output: "computer" | "phone") => {
    if (!snapshot) return;
    if (working || snapshot.busy || cleanupPending || receiverOperationBusy) return;
    if (output === "phone" && snapshot.state === "managementOnly") return;
    if (output === "phone" && !active && wirelessUdpMode) {
      openNetworkDialog(); return;
    }
    if (output === "phone" && !active && !selected) {
      setPanel(snapshot.connectionTransport === "wireless" ? "pair" : "help");
      return;
    }
    if (output === "phone" && !active && selected) {
      const generation = ++connectionStartGeneration.current;
      cancellableConnectionStart.current = true;
      setWorking(true);
      try {
        const result = await invokeApp<{ success: boolean; message: string; snapshot: Snapshot }>("connect_device", { serial: selected.serial });
        if (connectionStartGeneration.current !== generation) return;
        apply(result.snapshot);
        if (!result.success) notify(result.message);
      } catch (error) { if (connectionStartGeneration.current === generation) notify(String(error)); }
      finally {
        if (connectionStartGeneration.current === generation) {
          cancellableConnectionStart.current = false;
          setWorking(false);
        }
      }
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
      const listenerStopped = offerListenerChoice.current ? await stopOfferListener() : true;
      const result = await invokeApp<{ success: boolean; message: string; snapshot: Snapshot }>("disconnect");
      apply(result.snapshot);
      if (result.success && listenerStopped) dismissNotice(); else if (!result.success) notify(result.message);
    } catch (error) { notify(String(error)); }
    finally { setWorking(false); }
  };

  const needsFirstSetup = pairedPhoneCount === 0 && !active && !cleanupPending && !selected?.connectable;
  const runMainAction = () => {
    if (!snapshot) return;
    if (networkStartPending) {
      void cancelNetworkStart();
    } else if (cancellableConnectionStart.current || snapshot.state === "connecting" || snapshot.state === "reconnecting") {
      void cancelPendingConnection().catch(error => notify(String(error)));
    } else if (snapshot.wirelessConnectionOffers.active || mainListenerPending) {
      mainListenerGenerationRef.current++;
      setMainListenerPending(false);
      void stopOfferListener();
    } else if (cleanupPending || (active && (snapshot.state === "managementOnly" || snapshot.output === "phone"))) {
      void disconnect();
    } else if (needsFirstSetup) {
      setBackgroundStartMode(false);
      setPanel("setup");
    } else if (!active && snapshot.connectionTransport === "usb" && wiredMode === "aoa") {
      void openAoaDialog();
    } else if (!active && wirelessUdpMode) {
      void startMainScreenWaiting();
    } else if (active || selected?.connectable) {
      void setOutput("phone");
    } else {
      setPanel(snapshot.connectionTransport === "usb" ? "help" : "pair");
    }
  };

  const connectOnly = async () => {
    if (!snapshot) return;
    if (wirelessUdpMode) {
      openNetworkDialog(); return;
    }
    if (!selected?.connectable) return;
    const generation = ++connectionStartGeneration.current;
    cancellableConnectionStart.current = true;
    setWorking(true);
    try {
      const result = await invokeApp<{ success: boolean; message: string; snapshot: Snapshot }>("connect_device_only", { serial: selected.serial });
      if (connectionStartGeneration.current !== generation) return;
      apply(result.snapshot);
      if (!result.success) notify(result.message);
    } catch (error) { if (connectionStartGeneration.current === generation) notify(String(error)); }
    finally {
      if (connectionStartGeneration.current === generation) {
        cancellableConnectionStart.current = false;
        setWorking(false);
      }
    }
  };

  return { cancelPendingConnection, cancellableConnectionStart, connectOnly, connectionStartGeneration, needsFirstSetup, runMainAction, setOutput };
}
