import { useEffect, useRef, type Ref, type ReactNode } from "react";
import { X } from "lucide-react";
import { strings } from "../strings";

export function Modal({ title, children, onClose, className = "", returnFocusTarget, closeDisabled = false }: { title: string; children: ReactNode; onClose: () => void; className?: string; returnFocusTarget?: () => HTMLElement | null; closeDisabled?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null);
  const onCloseRef = useRef(onClose);
  const closeDisabledRef = useRef(closeDisabled);
  useEffect(() => { onCloseRef.current = onClose; });
  useEffect(() => { closeDisabledRef.current = closeDisabled; });
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (!dialog.open) dialog.showModal();
    const key = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); if (!closeDisabledRef.current) onCloseRef.current(); } };
    dialog.addEventListener("keydown", key);
    return () => {
      dialog.removeEventListener("keydown", key);
      if (dialog.open) dialog.close();
      const replacementModal = [...document.querySelectorAll("dialog.modal")].some(candidate => candidate !== dialog);
      const focusTarget = returnFocusTarget?.() ?? opener;
      if (!replacementModal && focusTarget?.isConnected) focusTarget.focus({ preventScroll: true });
    };
  }, []);
  return <dialog ref={ref} aria-label={title} className={`modal ${className}`} onCancel={event => { event.preventDefault(); if (!closeDisabled) onClose(); }} onClick={event => { const dialog = ref.current; if (closeDisabled || !dialog || event.target !== dialog) return; const rect = dialog.getBoundingClientRect(); if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) onClose(); }}>
    <div className="modal-head"><h2>{title}</h2><button className="icon-button" disabled={closeDisabled} onClick={onClose} aria-label={strings.modal.close}><X size={18} /></button></div>{children}
  </dialog>;
}


export function ErrorDetails({ className = "", id, summary, details, summaryRef }: { className?: string; id?: string; summary: string; details: string; summaryRef?: Ref<HTMLParagraphElement> }) {
  return <div id={id} className={`inline-error error-details ${className}`} role="alert">
    <p ref={summaryRef} tabIndex={-1}>{summary}</p>
    <details><summary>{strings.app.showTechnicalDetails}</summary><pre>{details}</pre></details>
  </div>;
}
