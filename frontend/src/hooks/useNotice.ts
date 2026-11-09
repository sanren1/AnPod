import { useCallback, useRef, useState } from "react";
import { needsReceiverUpdate, windowsRateMismatchHz } from "../appRuntime";
import { strings } from "../strings";

/** Toast state. A newer notice cancels the older timer; errors stay until dismissed. */
export function useNotice() {
  const [notice, setNotice] = useState("");
  const [noticeDetail, setNoticeDetail] = useState("");
  const timer = useRef<number | undefined>(undefined);

  // Stable identity: listeners attached once at startup keep using it.
  const announce = useCallback((message: string, detail = "", persist = false) => {
    window.clearTimeout(timer.current);
    setNotice(message);
    setNoticeDetail(detail);
    if (!persist) timer.current = window.setTimeout(() => { setNotice(""); setNoticeDetail(""); }, 4200);
  }, []);
  const dismissNotice = () => { window.clearTimeout(timer.current); setNotice(""); setNoticeDetail(""); };
  const notify = (message: string) => announce(needsReceiverUpdate(message) ? strings.app.receiverUpdateRequired
    : message.includes("FORMAT_RATE_MISMATCH:") ? strings.app.hiresRateMismatch(windowsRateMismatchHz(message)) : strings.app.operationNotCompleted, message, true);

  return { notice, noticeDetail, announce, dismissNotice, notify };
}
