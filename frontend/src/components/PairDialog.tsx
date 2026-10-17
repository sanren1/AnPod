import { useEffect, useRef, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import { LoaderCircle, Search } from "lucide-react";
import { strings } from "../strings";
import type { Snapshot, Credentials, MdnsResults } from "../appTypes";
import { invokeApp } from "../appRuntime";
import { Modal } from "./Modal";
import { ErrorDetails } from "./Modal";

export function PairDialog({ transport, initialEndpoint, disabled, onClose, onDone }: { transport: "usb" | "wifi"; initialEndpoint: string; disabled: boolean; onClose: () => void; onDone: () => void }) {
  const [tab, setTab] = useState<"qr" | "code" | "address">("qr");
  const [credentials, setCredentials] = useState<Credentials | null>(null);
  const [endpoint, setEndpoint] = useState(initialEndpoint);
  const [code, setCode] = useState("");
  const [message, setMessage] = useState<string>(strings.app.generatingSecurePairingCode);
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
      else if (found.length > 1) setMessage(strings.pairDialog.foundPhones(found.length));
      else setMessage(strings.pairDialog.noPhoneFound);
    } catch (reason) { if (generation === taskGeneration.current) setError(String(reason)); }
    finally { if (generation === taskGeneration.current) setBusy(false); }
  };
  return <Modal title={transport === "wifi" ? strings.pairDialog.wirelessTitle : strings.pairDialog.usbTitle} onClose={onClose} className="pair-dialog">
    {transport === "wifi" ? <>
      <div className="tabs" role="tablist" aria-label={strings.pairDialog.addMethod}><button role="tab" aria-selected={tab === "qr"} disabled={disabled || busy} onClick={() => setTab("qr")}>{strings.pairDialog.qrTab}</button><button role="tab" aria-selected={tab === "code"} disabled={disabled || busy} onClick={() => setTab("code")}>{strings.pairDialog.codeTab}</button><button role="tab" aria-selected={tab === "address"} disabled={disabled || busy} onClick={() => setTab("address")}>{strings.pairDialog.addressTab}</button></div>
      {tab === "qr" && <div className="qr-pane">
        {credentials ? <div className="qr-frame"><QRCodeSVG value={credentials.payload} size={174} level="Q" includeMargin /></div> : <div className="qr-frame qr-wait"><LoaderCircle className="spin" /></div>}
        <p className="qr-help">{strings.pairDialog.qrInstructions}</p>
        <p className="pair-progress" role="status" aria-live="polite">{message}</p>
        {!qrStarted && <button className="primary-button" disabled={disabled || !credentials || busy} onClick={() => {
          if (!credentials) return;
          const generation = currentTask();
          setQrStarted(true); setBusy(true);
          void invokeApp<{ success: boolean; message: string }>("pair_qr", { instanceName: credentials.instanceName, password: credentials.password })
            .then(async result => { if (generation !== taskGeneration.current) return; setMessage(result.message); if (result.success) await onDone(); else { setError(result.message); setQrStarted(false); setCredentials(null); void invokeApp<Credentials>("qr_credentials").then(value => { if (generation === taskGeneration.current) setCredentials(value); }); } })
            .catch(reason => { if (generation === taskGeneration.current) { setError(String(reason)); setQrStarted(false); } }).finally(() => { if (generation === taskGeneration.current) setBusy(false); });
        }}>{busy ? strings.pairDialog.waitingForScan : strings.pairDialog.startQrPairing}</button>}
        {qrStarted && <button className="dialog-secondary" onClick={() => { ++taskGeneration.current; void invokeApp("cancel_qr_pairing"); setQrStarted(false); setBusy(false); setMessage(strings.pairDialog.pairingCancelled); }}>{strings.pairDialog.cancelPairing}</button>}
      </div>}
      {tab === "code" && <form className="form-pane" onSubmit={event => { event.preventDefault(); void finish("pair_wifi", { endpoint, code }); }}>
        <label>{strings.pairDialog.pairingAddress}<input disabled={disabled || busy} value={endpoint} onChange={event => setEndpoint(event.target.value)} autoComplete="off" placeholder={strings.pairDialog.pairingAddressPlaceholder} /></label>
        <label>{strings.pairDialog.sixDigitCode}<input disabled={disabled || busy} value={code} onChange={event => setCode(event.target.value.replace(/\D/g, "").slice(0, 6))} inputMode="numeric" autoComplete="one-time-code" aria-describedby="pair-code-hint" placeholder={strings.pairDialog.codePlaceholder} /></label>
        <small id="pair-code-hint">{strings.pairDialog.codeHint}</small>
        <button className="primary-button" disabled={disabled || busy || code.length !== 6}>{busy ? strings.pairDialog.pairing : strings.pairDialog.pairAndAdd}</button>
      </form>}
      {tab === "address" && <form className="form-pane" onSubmit={event => { event.preventDefault(); void finish("connect_wifi", { endpoint }); }}>
        <label>{strings.pairDialog.wirelessDebugAddress}<input disabled={disabled || busy} value={endpoint} onChange={event => setEndpoint(event.target.value)} autoComplete="off" placeholder={strings.pairDialog.connectionAddressPlaceholder} /></label>
        <small>{strings.pairDialog.connectionPortHint}</small>
        <button className="text-button discover-button" type="button" onClick={() => void discover()} disabled={disabled || busy}><Search size={15} />{strings.pairDialog.searchLanDevices}</button>
        <button className="primary-button" disabled={disabled || busy || !endpoint.trim()}>{busy ? strings.pairDialog.connecting : strings.pairDialog.connectAddress}</button>
      </form>}
    </> : <div className="help-pane"><p>{strings.pairDialog.usbStep1}</p><p>{strings.pairDialog.usbStep2}</p><p>{strings.pairDialog.usbStep3}</p><button className="primary-button" onClick={async () => { await onDone(); }}>{strings.pairDialog.refreshDeviceList}</button></div>}
    {disabled && <p className="dialog-footnote" role="status">{strings.pairDialog.processingConnection}</p>}
    {error && <ErrorDetails summary={strings.app.pairingNotCompleted} details={error} />}
  </Modal>;
}
