import { strings } from "../strings";
import { bindingChoiceLabels } from "../appRuntime";

export function BindingReferenceList({ bindings }: { bindings: string[] }) {
  if (bindings.length < 2) return null;
  const labels = bindingChoiceLabels(bindings);
  return <details className="binding-reference-list">
    <summary>{strings.app.pairedRecordReferences}</summary>
    <ol>{bindings.map(value => <li key={value}><span>{labels.get(value)}</span><code>{value}</code></li>)}</ol>
  </details>;
}
