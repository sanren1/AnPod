import { strings } from "../strings";
import { bindingChoiceLabels } from "../appRuntime";

export function NetworkConnectionEntry({
  audioSessionActive, busy, bindings, bindingReference, setBindingReference, onConnect, onRecover, onClose,
}: {
  audioSessionActive: boolean; busy: boolean; bindings: string[]; bindingReference: string;
  setBindingReference: (value: string) => void; onConnect: (bindingReference?: string) => void;
  onRecover: () => void; onClose: () => void;
}) {
  const choiceLabels = bindingChoiceLabels(bindings);
  return <section className="connection-entry">
    {audioSessionActive ? <>
      <p>{strings.network.phoneAlreadyConnected}</p>
      <button className="dialog-secondary" type="button" onClick={onClose}>{strings.network.done}</button>
    </> : <>
      <p>{bindings.length === 0 ? strings.network.firstUseConnectHint : strings.network.connectSavedPhoneHint}</p>
      {bindings.length > 1 && <label className="network-field">{strings.network.savedPhoneChoice}
        <select value={bindingReference} onChange={event => setBindingReference(event.target.value)}>
          <option value="">{strings.network.chooseSavedPhone}</option>
          {bindings.map(value => <option key={value} value={value}>{choiceLabels.get(value)}</option>)}
        </select>
      </label>}
      <button className="primary-button" type="button" disabled={busy || (bindings.length > 1 && !bindingReference)} onClick={() => onConnect(bindingReference || undefined)}>
        {bindings.length === 0 ? strings.network.firstSetupAction : strings.app.connectPhone}
      </button>
      <button className="text-button" type="button" disabled={busy} onClick={onRecover}>{strings.network.cannotFindPhone}</button>
    </>}
  </section>;
}
