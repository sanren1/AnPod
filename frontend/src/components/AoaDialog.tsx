import { useEffect, useRef, useState } from "react";
import { RefreshCw, Usb } from "lucide-react";
import { strings } from "../strings";
import { bindingReferenceLabel, invokeApp } from "../appRuntime";
import type { AoaTarget } from "../appTypes";
import { ErrorDetails, Modal } from "./Modal";

export function AoaDialog({ disabled, onStart, onCancelStart, onClose }: {
  disabled: boolean;
  onStart: (targetId: string, bindingReference: string) => Promise<void>;
  onCancelStart: () => Promise<void>;
  onClose: () => void;
}) {
  const [targets, setTargets] = useState<AoaTarget[]>([]);
  const [switchTargets, setSwitchTargets] = useState<AoaTarget[]>([]);
  const [bindings, setBindings] = useState<string[]>([]);
  const [targetId, setTargetId] = useState("");
  const [switchTargetId, setSwitchTargetId] = useState("");
  const [bindingReference, setBindingReference] = useState("");
  const [loading, setLoading] = useState(true);
  const [switchListLoading, setSwitchListLoading] = useState(true);
  const [switchSectionOpen, setSwitchSectionOpen] = useState(true);
  const [switchConfirmed, setSwitchConfirmed] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [switchComplete, setSwitchComplete] = useState(false);
  const [switchHasSucceeded, setSwitchHasSucceeded] = useState(false);
  const [hasAoaTargetsAfterSwitch, setHasAoaTargetsAfterSwitch] = useState(false);
  const [switchListError, setSwitchListError] = useState("");
  const [starting, setStarting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [errorSummary, setErrorSummary] = useState("");
  const [errorDetails, setErrorDetails] = useState("");
  const switchSectionTouched = useRef(false);
  const switchSummaryRef = useRef<HTMLElement | null>(null);

  const refresh = async () => {
    setLoading(true);
    setErrorSummary("");
    setErrorDetails("");
    setSwitchListError("");
    setSwitchComplete(false);
    setHasAoaTargetsAfterSwitch(false);
    setSwitchListLoading(true);
    setTargetId("");
    setBindingReference("");
    try {
      const [nextTargets, nextBindings] = await Promise.all([
        invokeApp<AoaTarget[]>("list_aoa_targets"),
        invokeApp<string[]>("receiver_binding_references"),
      ]);
      setTargets(nextTargets);
      setBindings(nextBindings);
      if (!switchSectionTouched.current) setSwitchSectionOpen(nextTargets.length === 0 && !switchHasSucceeded);
    } catch (error) {
      setTargets([]);
      setBindings([]);
      setErrorSummary(strings.aoaDialog.loadFailed);
      setErrorDetails(String(error));
    } finally {
      setLoading(false);
    }
    try { setSwitchTargets(await invokeApp<AoaTarget[]>("list_aoa_switch_targets")); }
    catch (error) { setSwitchTargets([]); setSwitchListError(String(error)); }
    finally { setSwitchListLoading(false); }
  };

  useEffect(() => { void refresh(); }, []);

  const switchMode = async () => {
    if (disabled || loading || starting || switching || !switchTargetId || !switchConfirmed) return;
    setSwitching(true);
    setSwitchComplete(false);
    setHasAoaTargetsAfterSwitch(false);
    setErrorSummary("");
    setErrorDetails("");
    setSwitchListError("");
    try {
      const result = await invokeApp<{ success: boolean; message: string; requiresRefresh: boolean }>("switch_aoa_mode", { targetId: switchTargetId, releaseAdbServer: switchConfirmed });
      if (!result.success) throw new Error(result.message);
      try {
        const nextTargets = await invokeApp<AoaTarget[]>("list_aoa_targets");
        setTargets(nextTargets);
        setHasAoaTargetsAfterSwitch(nextTargets.length > 0);
      }
      catch (error) {
        setErrorSummary(strings.aoaDialog.switchRefreshFailed);
        setErrorDetails(String(error));
      }
      if (result.requiresRefresh) {
        try { setSwitchTargets(await invokeApp<AoaTarget[]>("list_aoa_switch_targets")); }
        catch (error) { setSwitchTargets([]); setSwitchListError(String(error)); }
      }
      setSwitchTargetId("");
      setSwitchConfirmed(false);
      setSwitchComplete(true);
      setSwitchHasSucceeded(true);
      switchSummaryRef.current?.focus();
      setSwitchSectionOpen(false);
    } catch (error) {
      const details = String(error);
      setErrorSummary(/0x80070005|E_ACCESSDENIED|Access is denied/i.test(details) ? strings.aoaDialog.switchAccessDenied : strings.aoaDialog.switchFailed);
      setErrorDetails(details);
    } finally { setSwitching(false); }
  };

  const start = async () => {
    if (disabled || loading || starting || !targetId || !bindingReference) return;
    setStarting(true);
    setErrorSummary("");
    setErrorDetails("");
    try { await onStart(targetId, bindingReference); }
    catch (error) {
      setErrorSummary(strings.aoaDialog.startFailed);
      setErrorDetails(String(error));
    }
    finally { setStarting(false); }
  };

  const cancelStart = async () => {
    if (!starting || cancelling) return;
    setCancelling(true);
    setErrorSummary("");
    setErrorDetails("");
    try {
      await onCancelStart();
      onClose();
    } catch (error) {
      setErrorSummary(strings.aoaDialog.cancelFailed);
      setErrorDetails(String(error));
    } finally { setCancelling(false); }
  };

  const busy = disabled || loading || switchListLoading || starting || cancelling || switching;
  return <Modal title={strings.aoaDialog.title} onClose={() => { if (switching) return; if (starting) void cancelStart(); else onClose(); }} className="aoa-dialog">
    <p className="dialog-description">{targets.length ? strings.aoaDialog.readyDescription : strings.aoaDialog.description}</p>
    {errorSummary && <ErrorDetails className="aoa-error" summary={errorSummary} details={errorDetails} />}
    {starting && <p className="dialog-description" role="status" aria-live="polite">{cancelling ? strings.aoaDialog.cancelling : strings.aoaDialog.starting}</p>}
    {switching && <p className="dialog-description" role="status" aria-live="polite">{strings.aoaDialog.switching}</p>}
    {switchComplete && <p className="dialog-description" role="status" aria-live="polite">{hasAoaTargetsAfterSwitch ? strings.aoaDialog.switchReady : strings.aoaDialog.switchComplete}</p>}
    <details className="aoa-switch-section" open={switchSectionOpen} onToggle={event => setSwitchSectionOpen(event.currentTarget.open)}>
      <summary ref={switchSummaryRef} onClick={() => { switchSectionTouched.current = true; }}>{strings.aoaDialog.switchTitle}</summary>
      <p>{strings.aoaDialog.switchSupport}</p>
      {switchListError && <ErrorDetails className="aoa-error" summary={strings.aoaDialog.switchLoadFailed} details={switchListError} />}
      <label className="aoa-field">
        <span>{strings.aoaDialog.switchTarget}</span>
        <select value={switchTargetId} onChange={event => { setSwitchTargetId(event.target.value); setSwitchConfirmed(false); setSwitchComplete(false); }} disabled={busy}>
          <option value="">{switchListLoading ? strings.aoaDialog.loading : strings.aoaDialog.chooseSwitchTarget}</option>
          {switchTargets.map(target => <option key={target.id} value={target.id}>{target.label}</option>)}
        </select>
      </label>
      {!switchListLoading && switchTargets.length === 0 && !switchListError && <p className="empty-note">{strings.aoaDialog.noSwitchTargets}</p>}
      <label className="aoa-switch-confirm">
        <input type="checkbox" checked={switchConfirmed} onChange={event => setSwitchConfirmed(event.target.checked)} disabled={busy || !switchTargetId} />
        <span>{strings.aoaDialog.switchConfirmation}</span>
      </label>
      <div className="dialog-actions aoa-switch-actions">
        <button className="primary-button" type="button" onClick={() => void switchMode()} disabled={busy || !switchTargetId || !switchConfirmed}>
          <Usb size={16} aria-hidden="true" />{switching ? strings.aoaDialog.switching : strings.aoaDialog.switchAction}
        </button>
      </div>
    </details>
    <hr className="aoa-divider" />
    <h3 className="aoa-section-title">{strings.aoaDialog.connectTitle}</h3>
    <label className="aoa-field">
      <span>{strings.aoaDialog.target}</span>
      <select value={targetId} onChange={event => setTargetId(event.target.value)} disabled={busy}>
        <option value="">{loading ? strings.aoaDialog.loading : strings.aoaDialog.chooseTarget}</option>
        {targets.map(target => <option key={target.id} value={target.id}>{target.label}</option>)}
      </select>
    </label>
    {!loading && targets.length === 0 && !errorSummary && <p className="empty-note">{strings.aoaDialog.noTargets}</p>}
    <label className="aoa-field">
      <span>{strings.aoaDialog.pairedReceiver}</span>
      <select value={bindingReference} onChange={event => setBindingReference(event.target.value)} disabled={busy}>
        <option value="">{loading ? strings.aoaDialog.loading : strings.aoaDialog.chooseBinding}</option>
        {bindings.map(reference => <option key={reference} value={reference}>{bindingReferenceLabel(reference)}</option>)}
      </select>
    </label>
    {!loading && bindings.length === 0 && !errorSummary && <p className="empty-note">{strings.aoaDialog.noBindings}</p>}
    <div className="dialog-actions">
      {starting ? <button className="dialog-secondary" type="button" onClick={() => void cancelStart()} disabled={cancelling}>
        {cancelling ? strings.aoaDialog.cancelling : strings.aoaDialog.cancelConnection}
      </button> : <>
        <button className="dialog-secondary" type="button" onClick={() => void refresh()} disabled={busy}>
          <RefreshCw size={15} aria-hidden="true" />{strings.aoaDialog.refresh}
        </button>
        <button className="primary-button" type="button" onClick={() => void start()} disabled={busy || !targetId || !bindingReference}>
          <Usb size={16} aria-hidden="true" />{strings.aoaDialog.start}
        </button>
      </>}
    </div>
  </Modal>;
}
