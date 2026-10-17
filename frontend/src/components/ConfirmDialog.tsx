import { strings } from "../strings";
import { Modal } from "./Modal";

export function ConfirmDialog({ title, body, confirmLabel, busy, onConfirm, onCancel }: { title: string; body: string; confirmLabel: string; busy: boolean; onConfirm: () => void; onCancel: () => void }) {
  return <Modal title={title} onClose={() => { if (!busy) onCancel(); }} className="confirm-dialog"><p>{body}</p><div className="dialog-actions"><button className="dialog-secondary" onClick={onCancel} disabled={busy}>{strings.confirmDialog.cancel}</button><button className="primary-button" onClick={onConfirm} disabled={busy}>{busy ? strings.confirmDialog.switching : confirmLabel}</button></div></Modal>;
}
