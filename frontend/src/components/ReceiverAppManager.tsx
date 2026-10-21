import { LoaderCircle } from "lucide-react";
import { strings } from "../strings";
import type { BindingResetPreview, BindingResetResult, Device, ReceiverPackageCandidate, ReceiverStatus, ReceiverUsbOperation, ReceiverUsbOperationResult, Snapshot } from "../appTypes";
import { commitmentSummary, resetOutcomeLabel, resetPcStateLabel, resetPhoneStateLabel } from "../appRuntime";
import { BindingReferenceList } from "./BindingReferenceList";
import { ErrorDetails } from "./Modal";

export function ReceiverAppManager({
  snapshot, busy, candidateLoading, receiverAction, receiverError, serial, receiverStatus, receiverNeedsUpgrade,
  setSerial, onReceiverSerialChange, receiverAlreadyCurrent, updateCandidateError, usbDevices, unauthorizedUsb, updateCandidate, checkReceiver,
  canRequestUsbInstall, canRequestUsbMaintenance, canRequestUsbUpgrade, requestUsbOperation, onRefreshDevices,
  receiverOperationResult, currentVersionLabel, bindings, bindingReference, setBindingReference, onClearErrors,
  bindingResetPreview, bindingResetResult, bindingResetError, bindingResetLoading, bindingResetBusy,
  previewBindingReset, invalidateBindingReset, confirmBindingReset,
}: {
  snapshot: Snapshot; busy: boolean; candidateLoading: boolean; receiverAction: "checking" | null; receiverError: string;
  serial: string; setSerial: (serial: string) => void; onReceiverSerialChange: (serial: string) => void;
  receiverStatus: ReceiverStatus | null; receiverNeedsUpgrade: boolean; receiverAlreadyCurrent: boolean;
  updateCandidateError: string; usbDevices: Device[]; unauthorizedUsb: boolean; updateCandidate: ReceiverPackageCandidate | null;
  checkReceiver: (serial: string) => Promise<ReceiverStatus | null>; canRequestUsbInstall: boolean;
  canRequestUsbMaintenance: boolean; canRequestUsbUpgrade: boolean;
  requestUsbOperation: (kind: ReceiverUsbOperation["kind"]) => void; onRefreshDevices: () => void;
  receiverOperationResult: ReceiverUsbOperationResult | null; currentVersionLabel: string; bindings: string[];
  bindingReference: string; setBindingReference: (value: string) => void; onClearErrors: () => void;
  bindingResetPreview: BindingResetPreview | null; bindingResetResult: BindingResetResult | null; bindingResetError: string;
  bindingResetLoading: boolean; bindingResetBusy: boolean;
  previewBindingReset: (serial: string, recheck?: boolean) => Promise<void>;
  invalidateBindingReset: () => void; confirmBindingReset: () => Promise<void>;
}) {
  return <section className="receiver-manager receiver-usb-card" aria-label={strings.network.receiverUsbManagementLabel}>
    <label className="network-field">{strings.network.authorizedOnlineUsb}
      <select required value={serial} disabled={busy} onChange={event => { invalidateBindingReset(); setSerial(event.target.value); onReceiverSerialChange(event.target.value); onClearErrors(); }}>
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
        {canRequestUsbInstall && updateCandidate && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{strings.network.installReceiver(updateCandidate.versionCode)}</button>}
        {canRequestUsbMaintenance && updateCandidate && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("install")}>{strings.network.usbMaintenanceInstall(updateCandidate.versionCode)}</button>}
        {canRequestUsbUpgrade && updateCandidate && <button className="primary-button" type="button" disabled={busy} onClick={() => requestUsbOperation("upgrade")}>{strings.network.upgradeReceiver(updateCandidate.versionCode)}</button>}
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
        <select value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); onClearErrors(); }}>
          <option value="">{strings.network.chooseSessionBindingOption}</option>
          {bindings.map((value, index) => <option key={value} value={value}>{strings.network.savedPhoneNumber(index + 1)}</option>)}
        </select>
      </label>
      {bindings.length > 1 && <BindingReferenceList bindings={bindings} />}
    </>}
    {receiverNeedsUpgrade && snapshot.sessionTransport !== "lan" && bindings.length > 0 && <details className="receiver-maintenance-details">
      <summary>{strings.network.upgradeTargetDetails}</summary>
      <label className="network-field receiver-maintenance-binding">{strings.network.optionalSavedBinding}
        <select value={bindingReference} disabled={busy} onChange={event => { setBindingReference(event.target.value); onClearErrors(); }}>
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
  </section>;
}
