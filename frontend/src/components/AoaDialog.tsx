import { useEffect, useRef, useState } from "react";
import { RefreshCw, Usb } from "lucide-react";
import { strings } from "../strings";
import { bindingReferenceLabel, invokeApp } from "../appRuntime";
import type { AoaTarget } from "../appTypes";
import { ErrorDetails, Modal } from "./Modal";

export function AoaDialog({ disabled, lastUsbDirectTargetId, lastUsbDirectBindingReference, onStart, onCancelStart, onClose }: {
  disabled: boolean;
  lastUsbDirectTargetId: string | null;
  lastUsbDirectBindingReference: string | null;
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
  const [hasAoaTargetsAfterSwitch, setHasAoaTargetsAfterSwitch] = useState(false);
  const [switchListError, setSwitchListError] = useState("");
  const [starting, setStarting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [errorSummary, setErrorSummary] = useState("");
  const [errorDetails, setErrorDetails] = useState("");
  const [discoveryError, setDiscoveryError] = useState("");
  const switchSectionTouched = useRef(false);
  const switchSummaryRef = useRef<HTMLElement | null>(null);
  const targetSelectionTouched = useRef(false);
  const bindingSelectionTouched = useRef(false);
  const switchTargetTouched = useRef(false);
  const targetIdRef = useRef("");
  const switchTargetIdRef = useRef("");
  const bindingReferenceRef = useRef("");
  const refreshInFlight = useRef(false);
  const discoveryErrorRef = useRef("");
  const pollPaused = useRef(false);
  const mounted = useRef(false);
  const switchHasSucceededRef = useRef(false);

  const selectTarget = (value: string) => { targetIdRef.current = value; setTargetId(value); };
  const selectSwitchTarget = (value: string) => { switchTargetIdRef.current = value; setSwitchTargetId(value); };
  const selectBinding = (value: string) => { bindingReferenceRef.current = value; setBindingReference(value); };

  const reportDiscoveryError = (value: string) => {
    if (value !== discoveryErrorRef.current) {
      discoveryErrorRef.current = value;
      setDiscoveryError(value);
    }
  };

  const refresh = async (initial = false, manual = false) => {
    if (refreshInFlight.current) return;
    refreshInFlight.current = true;
    if (initial) { setLoading(true); setSwitchListLoading(true); }
    if (manual) {
      setErrorSummary("");
      setErrorDetails("");
      setSwitchListError("");
      setSwitchComplete(false);
      setHasAoaTargetsAfterSwitch(false);
    }
    const [targetsResult, bindingsResult, switchTargetsResult] = await Promise.allSettled([
      invokeApp<AoaTarget[]>("list_aoa_targets"),
      invokeApp<string[]>("receiver_binding_references"),
      invokeApp<AoaTarget[]>("list_aoa_switch_targets"),
    ]);
    if (mounted.current) {
      if (targetsResult.status === "fulfilled") {
        const nextTargets = targetsResult.value;
        setTargets(nextTargets);
        const currentTarget = targetIdRef.current;
        if (currentTarget && !nextTargets.some(target => target.id === currentTarget)) {
          selectTarget("");
          targetSelectionTouched.current = false;
          selectBinding("");
          bindingSelectionTouched.current = true;
        } else if (!currentTarget && !targetSelectionTouched.current) {
          const rememberedPairIsPresent = lastUsbDirectTargetId !== null
            && lastUsbDirectBindingReference !== null
            && nextTargets.some(target => target.id === lastUsbDirectTargetId)
            && bindingsResult.status === "fulfilled"
            && bindingsResult.value.includes(lastUsbDirectBindingReference);
          if (rememberedPairIsPresent) {
            selectTarget(lastUsbDirectTargetId);
            selectBinding(lastUsbDirectBindingReference);
            targetSelectionTouched.current = true;
            bindingSelectionTouched.current = true;
          } else if (nextTargets.length === 1) {
            selectTarget(nextTargets[0].id);
            targetSelectionTouched.current = true;
          }
        }
        if (initial && !switchSectionTouched.current)
          setSwitchSectionOpen(nextTargets.length === 0 && !switchHasSucceededRef.current);
      } else if (initial || manual) {
        setErrorSummary(strings.aoaDialog.loadFailed);
        setErrorDetails(String(targetsResult.reason));
      }
      if (bindingsResult.status === "fulfilled") {
        const nextBindings = bindingsResult.value;
        setBindings(nextBindings);
        const currentBinding = bindingReferenceRef.current;
        if (currentBinding && !nextBindings.includes(currentBinding)) {
          selectBinding("");
          bindingSelectionTouched.current = true;
        } else if (!bindingSelectionTouched.current
          && lastUsbDirectTargetId !== null
          && targetIdRef.current === lastUsbDirectTargetId
          && lastUsbDirectBindingReference !== null
          && nextBindings.includes(lastUsbDirectBindingReference)) {
          selectBinding(lastUsbDirectBindingReference);
          bindingSelectionTouched.current = true;
        }
      } else if (initial || manual) {
        setErrorSummary(strings.aoaDialog.loadFailed);
        setErrorDetails(String(bindingsResult.reason));
      }
      if (switchTargetsResult.status === "fulfilled") {
        const nextSwitchTargets = switchTargetsResult.value;
        setSwitchTargets(nextSwitchTargets);
        const currentSwitchTarget = switchTargetIdRef.current;
        if (currentSwitchTarget && !nextSwitchTargets.some(target => target.id === currentSwitchTarget)) {
          selectSwitchTarget("");
          switchTargetTouched.current = false;
          setSwitchConfirmed(false);
        } else if (!currentSwitchTarget && !switchTargetTouched.current && nextSwitchTargets.length === 1) {
          selectSwitchTarget(nextSwitchTargets[0].id);
          switchTargetTouched.current = true;
        }
        if (initial || manual) setSwitchListError("");
      } else if (initial || manual) {
        setSwitchListError(String(switchTargetsResult.reason));
      }
      const discoveryFailures = [
        targetsResult.status === "rejected" ? String(targetsResult.reason) : "",
        bindingsResult.status === "rejected" ? String(bindingsResult.reason) : "",
        switchTargetsResult.status === "rejected" ? String(switchTargetsResult.reason) : "",
      ].filter(Boolean).join("；");
      reportDiscoveryError(discoveryFailures);
      if (initial) { setLoading(false); setSwitchListLoading(false); }
    }
    refreshInFlight.current = false;
  };

  useEffect(() => {
    mounted.current = true;
    void refresh(true);
    const timer = window.setInterval(() => { if (!pollPaused.current) void refresh(); }, 1800);
    return () => { mounted.current = false; window.clearInterval(timer); };
  }, []);

  pollPaused.current = starting || cancelling || switching;

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
      switchHasSucceededRef.current = true;
      selectSwitchTarget("");
      switchTargetTouched.current = true;
      setSwitchConfirmed(false);
      setSwitchComplete(true);
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
    {discoveryError && <ErrorDetails className="aoa-error" summary={strings.aoaDialog.discoveryFailed} details={discoveryError} />}
    {starting && <p className="dialog-description" role="status" aria-live="polite">{cancelling ? strings.aoaDialog.cancelling : strings.aoaDialog.starting}</p>}
    {switching && <p className="dialog-description" role="status" aria-live="polite">{strings.aoaDialog.switching}</p>}
    {switchComplete && <p className="dialog-description" role="status" aria-live="polite">{hasAoaTargetsAfterSwitch ? strings.aoaDialog.switchReady : strings.aoaDialog.switchComplete}</p>}
    <details className="aoa-switch-section" open={switchSectionOpen} onToggle={event => setSwitchSectionOpen(event.currentTarget.open)}>
      <summary ref={switchSummaryRef} onClick={() => { switchSectionTouched.current = true; }}>{strings.aoaDialog.switchTitle}</summary>
      <p>{strings.aoaDialog.switchSupport}</p>
      {switchListError && <ErrorDetails className="aoa-error" summary={strings.aoaDialog.switchLoadFailed} details={switchListError} />}
      <label className="aoa-field">
        <span>{strings.aoaDialog.switchTarget}</span>
        <select value={switchTargetId} onChange={event => {
          const value = event.target.value;
          if (value !== switchTargetIdRef.current) {
            selectSwitchTarget(value);
            switchTargetTouched.current = true;
            setSwitchConfirmed(false);
            setSwitchComplete(false);
          }
        }} disabled={busy}>
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
      <select value={targetId} onChange={event => {
        const value = event.target.value;
        if (value !== targetIdRef.current) {
          selectTarget(value);
          targetSelectionTouched.current = true;
          selectBinding("");
          bindingSelectionTouched.current = true;
        }
      }} disabled={busy}>
        <option value="">{loading ? strings.aoaDialog.loading : strings.aoaDialog.chooseTarget}</option>
        {targets.map(target => <option key={target.id} value={target.id}>{target.label}</option>)}
      </select>
    </label>
    {!loading && targets.length === 0 && !errorSummary && <p className="empty-note">{strings.aoaDialog.noTargets}</p>}
    <label className="aoa-field">
      <span>{strings.aoaDialog.pairedReceiver}</span>
      <select value={bindingReference} onChange={event => {
        selectBinding(event.target.value);
        bindingSelectionTouched.current = true;
      }} disabled={busy}>
        <option value="">{loading ? strings.aoaDialog.loading : strings.aoaDialog.chooseBinding}</option>
        {bindings.map(reference => <option key={reference} value={reference}>{bindingReferenceLabel(reference)}</option>)}
      </select>
    </label>
    {!loading && bindings.length === 0 && !errorSummary && <p className="empty-note">{strings.aoaDialog.noBindings}</p>}
    <div className="dialog-actions">
      {starting ? <button className="dialog-secondary" type="button" onClick={() => void cancelStart()} disabled={cancelling}>
        {cancelling ? strings.aoaDialog.cancelling : strings.aoaDialog.cancelConnection}
      </button> : <>
        <button className="dialog-secondary" type="button" onClick={() => void refresh(false, true)} disabled={busy}>
          <RefreshCw size={15} aria-hidden="true" />{strings.aoaDialog.refresh}
        </button>
        <button className="primary-button" type="button" onClick={() => void start()} disabled={busy || !targetId || !bindingReference}>
          <Usb size={16} aria-hidden="true" />{strings.aoaDialog.start}
        </button>
      </>}
    </div>
  </Modal>;
}
