import { useEffect, useRef, type Ref, type ReactNode } from "react";
import { X } from "lucide-react";
import { zhCN } from "../strings.zh-CN";

export function Modal({ title, children, onClose, className = "" }: { title: string; children: ReactNode; onClose: () => void; className?: string }) {
  const ref = useRef<HTMLDialogElement>(null);
  const onCloseRef = useRef(onClose);
  useEffect(() => { onCloseRef.current = onClose; });
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (!dialog.open) dialog.showModal();
    const key = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); onCloseRef.current(); } };
    dialog.addEventListener("keydown", key);
    return () => { dialog.removeEventListener("keydown", key); if (dialog.open) dialog.close(); };
  }, []);
  return <dialog ref={ref} aria-label={title} className={`modal ${className}`} onCancel={event => { event.preventDefault(); onClose(); }} onClick={event => { if (event.target === ref.current) onClose(); }}>
    <div className="modal-head"><h2>{title}</h2><button className="icon-button" onClick={onClose} aria-label={zhCN.modal.close}><X size={18} /></button></div>{children}
  </dialog>;
}


export function ErrorDetails({ className = "", id, summary, details, summaryRef }: { className?: string; id?: string; summary: string; details: string; summaryRef?: Ref<HTMLParagraphElement> }) {
  return <div id={id} className={`inline-error error-details ${className}`} role="alert">
    <p ref={summaryRef} tabIndex={-1}>{summary}</p>
    <details><summary>{zhCN.app.showTechnicalDetails}</summary><pre>{details}</pre></details>
  </div>;
}
