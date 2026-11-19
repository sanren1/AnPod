import { strings } from "../strings";
import { bindingChoiceLabels } from "../appRuntime";

export function BindingReferenceList({ bindings, labels = bindingChoiceLabels(bindings) }: { bindings: string[]; labels?: Map<string, string> }) {
  if (bindings.length < 2) return null;
  return <details className="binding-reference-list">
    <summary>{strings.app.pairedRecordReferences}</summary>
    <ol>{bindings.map(value => <li key={value}><span>{labels.get(value)}</span><code>{value}</code></li>)}</ol>
  </details>;
}
