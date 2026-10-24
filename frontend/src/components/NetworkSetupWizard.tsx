import type { Ref } from "react";
import { strings } from "../strings";
import type { Device, PrivateIpv4Interface, ReceiverIpv4Interface, ReceiverStatus, ReceiverUsbOperationResult, ReceiverPackageCandidate } from "../appTypes";
import { needsReceiverUpdate, receiverErrorMessage } from "../appRuntime";
import { ErrorDetails } from "./Modal";

type Step = 1 | 2 | 3;

export function NetworkSetupWizard({
  step, stepRef, setStep, busy, serial, setSerial, onReceiverSerialChange, usbDevices, unauthorizedUsb, onRefreshDevices,
  receiverAction, receiverError, receiverStatus, receiverNeedsUpgrade, receiverOperationResult, updateCandidate,
  candidateLoading, updateCandidateError, canRequestUsbInstall, audioSessionActive, checkReceiver, serialRef,
  error, alertRef, interfaces, localAddress, setLocalAddress, receiverAddress, setReceiverAddress, receiverInterfaces,
  compatibleReceiverInterfaces, hasValidBootstrapPairs, localError, loadNetworkInfo, canStart,
  onInstallOrUpgrade, onClearErrors,
}: {
  step: Step; stepRef: Ref<HTMLDivElement>; setStep: (step: Step) => void; busy: boolean; serial: string;
  setSerial: (serial: string) => void; onReceiverSerialChange: (serial: string) => void; usbDevices: Device[];
  unauthorizedUsb: boolean; onRefreshDevices: () => void; receiverAction: "checking" | null; receiverError: string;
  receiverStatus: ReceiverStatus | null; receiverNeedsUpgrade: boolean; receiverOperationResult: ReceiverUsbOperationResult | null;
  updateCandidate: ReceiverPackageCandidate | null; candidateLoading: boolean; updateCandidateError: string;
  canRequestUsbInstall: boolean; audioSessionActive: boolean; checkReceiver: (serial: string) => Promise<ReceiverStatus | null>;
  serialRef: { current: string }; error: string; alertRef: Ref<HTMLParagraphElement>; interfaces: PrivateIpv4Interface[];
  localAddress: string; setLocalAddress: (address: string) => void; receiverAddress: string;
  setReceiverAddress: (address: string) => void; receiverInterfaces: ReceiverIpv4Interface[];
  compatibleReceiverInterfaces: ReceiverIpv4Interface[]; hasValidBootstrapPairs: boolean; localError: string;
  loadNetworkInfo: () => Promise<void>; canStart: boolean; onInstallOrUpgrade: (kind: "install" | "upgrade") => void;
  onClearErrors: () => void;
}) {
  const setUpStep = (next: Step) => { setStep(next); onClearErrors(); };
  return <section className="setup-wizard" aria-label={strings.network.setupUsb}>
    <ol className="setup-step-list" aria-label={strings.network.setupSteps}>
      {([strings.network.setupStepUsb, strings.network.setupStepReceiver, strings.network.setupStepBinding] as const).map((label, index) => {
        const currentStep = (index + 1) as Step;
        return <li key={label}><button type="button" className="setup-step" aria-current={step === currentStep ? "step" : undefined} disabled={busy || currentStep > step} onClick={() => setUpStep(currentStep)}><span>{currentStep}</span>{label}</button></li>;
      })}
    </ol>
    {step === 1 && <div ref={stepRef} className="setup-wizard-step">
      <p>{strings.network.connectAndAuthorizeUsb}</p>
      <label className="network-field">{strings.network.authorizedOnlineUsb}
        <select required value={serial} disabled={busy} onChange={event => { setSerial(event.target.value); onReceiverSerialChange(event.target.value); onClearErrors(); }}>
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
    {step === 2 && <div ref={stepRef} className="setup-wizard-step">
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
            {canRequestUsbInstall && updateCandidate && <button className="primary-button" type="button" disabled={busy} onClick={() => onInstallOrUpgrade("install")}>{strings.network.installReceiver(updateCandidate.versionCode)}</button>}
            {!audioSessionActive && receiverNeedsUpgrade && updateCandidate && !receiverError && <button className="dialog-secondary" type="button" disabled={busy} onClick={() => onInstallOrUpgrade("install")}>{strings.network.optionalMaintenanceUpgrade(updateCandidate.versionCode)}</button>}
          </div>
        </div>
        {receiverError && <ErrorDetails summary={needsReceiverUpdate(receiverError) ? receiverErrorMessage(receiverError) : strings.app.operationNotCompleted} details={receiverError} summaryRef={alertRef} />}
        {error && <ErrorDetails summary={needsReceiverUpdate(error) ? receiverErrorMessage(error) : strings.app.operationNotCompleted} details={error} summaryRef={alertRef} />}
        {canRequestUsbInstall && <p className="network-note">{strings.network.verifiedPackageInstallNote}</p>}
      </section> : <p className="network-recovery" role="status">{strings.network.goBackChooseUsb}</p>}
      <div className="setup-wizard-actions"><button className="dialog-secondary" type="button" disabled={busy} onClick={() => setUpStep(1)}>{strings.network.back}</button><button className="primary-button" type="button" disabled={busy || receiverStatus?.installed !== true || !!receiverError || !!error} onClick={async () => { const checkedSerial = serial; const checked = await checkReceiver(checkedSerial); if (checked?.installed === true && serialRef.current === checkedSerial) setUpStep(3); }}>{strings.network.next}</button></div>
    </div>}
    {step === 3 && <div ref={stepRef} className="setup-wizard-step">
      <p>{strings.network.chooseMatchingInterfaces}</p>
      <label className="network-field">{strings.network.computerInterface}
        <select required value={localAddress} disabled={busy} onChange={event => { setLocalAddress(event.target.value); onClearErrors(); }}>
          <option value="">{strings.network.chooseIpv4Interface}</option>
          {interfaces.map(value => <option key={`${value.index}-${value.address}`} value={value.address}>{value.name} · {value.address}/{value.prefixLength}</option>)}
        </select>
      </label>
      <label className="network-field">{strings.network.phoneCurrentIpv4}
        <select required value={receiverAddress} disabled={busy || receiverStatus?.installed !== true || !localAddress} onChange={event => { setReceiverAddress(event.target.value); onClearErrors(); }}>
          <option value="">{strings.network.choosePhoneInterface}</option>
          {compatibleReceiverInterfaces.map(value => <option key={`${value.interfaceIndex}-${value.address}`} value={value.address}>{value.interfaceName} · {value.address}/{value.prefixLength}</option>)}
        </select>
      </label>
      {interfaces.length > 0 && receiverInterfaces.length > 0 && !hasValidBootstrapPairs && <div className="network-recovery" role="status"><span>{strings.network.noSameSubnet}</span></div>}
      {(interfaces.length === 0 || receiverInterfaces.length === 0) && <p className="network-note">{strings.network.requiresComputerAndPhoneIp}</p>}
      <button type="button" className="text-button network-refresh" disabled={busy || !serial} onClick={() => { void Promise.all([loadNetworkInfo(), checkReceiver(serial)]); }}>{strings.network.recheckNetworkInterfaces}</button>
      {error ? <ErrorDetails id="network-flow-error" className="network-error" summary={needsReceiverUpdate(error) ? receiverErrorMessage(error) : strings.app.connectionNotCompleted} details={error} summaryRef={alertRef} />
        : localError && <p id="network-flow-error" ref={alertRef} className="inline-error network-error" role="alert" tabIndex={-1}>{receiverErrorMessage(localError)}</p>}
      <div className="setup-wizard-actions"><button className="dialog-secondary" type="button" disabled={busy} onClick={() => setUpStep(2)}>{strings.network.back}</button><button className="primary-button" type="submit" disabled={busy || !canStart}>{strings.network.bindAndConnect}</button></div>
    </div>}
  </section>;
}
