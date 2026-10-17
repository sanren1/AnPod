import { zhCN } from "../strings.zh-CN";

export function BindingReferenceList({ bindings }: { bindings: string[] }) {
  if (bindings.length < 2) return null;
  return <details className="binding-reference-list">
    <summary>{zhCN.app.pairedRecordReferences}</summary>
    <ol>{bindings.map((value, index) => <li key={value}><span>{zhCN.app.pairedDeviceNumber(index + 1)}</span><code>{value}</code></li>)}</ol>
  </details>;
}
