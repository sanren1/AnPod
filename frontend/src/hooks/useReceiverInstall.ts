import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { listen } from "@tauri-apps/api/event";
import { strings } from "../strings";
import { invokeApp } from "../appRuntime";
import type { Snapshot, ReceiverStatus, ReceiverUsbOperation, ReceiverUsbOperationResult, ReceiverInstallProgress, Panel, ConfirmRequest } from "../appTypes";

/** USB Receiver install/upgrade: confirmation gate, progress events and the verified result. */
export function useReceiverInstall({ active, announce, appMounted, apply, panelRef, rememberInstalledVersion, setConfirm, setNetworkDialogError, setPanel, setWorking, snapshot }: {
  active: boolean;
  announce: (message: string, detail?: string, persist?: boolean) => void;
  appMounted: RefObject<boolean>;
  apply: (value: Snapshot) => void;
  panelRef: RefObject<Panel | null>;
  rememberInstalledVersion: (serial: string, version: number | null) => void;
  setConfirm: (value: ConfirmRequest | null) => void;
  setNetworkDialogError: (value: string) => void;
  setPanel: (panel: Panel | null) => void;
  setWorking: (value: boolean) => void;
  snapshot: Snapshot | null;
}) {
  const [pendingReceiverOperation, setPendingReceiverOperation] = useState<ReceiverUsbOperation | null>(null);
  const [receiverOperationBusy, setReceiverOperationBusy] = useState(false);
  const [receiverManagerSerial, setReceiverManagerSerial] = useState("");
  const [receiverOperationResult, setReceiverOperationResult] = useState<ReceiverUsbOperationResult | null>(null);
  const [receiverOperationProgress, setReceiverOperationProgress] = useState<ReceiverInstallProgress | null>(null);
  const receiverOperationLock = useRef(false);
  const receiverConfirmationStarted = useRef(false);
  const receiverOperationUnlisten = useRef<(() => void) | null>(null);
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
      rememberInstalledVersion(operation.serial, status.installed ? status.versionCode : null);
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
  useEffect(() => () => {
    receiverOperationUnlisten.current?.();
    receiverOperationUnlisten.current = null;
  }, []);

  return { pendingReceiverOperation, performReceiverUsbOperation, receiverConfirmationStarted, receiverManagerSerial, receiverOperationBusy, receiverOperationLock, receiverOperationProgress, receiverOperationResult, requestReceiverUsbOperation, setPendingReceiverOperation, setReceiverManagerSerial, setReceiverOperationProgress, setReceiverOperationResult };
}
