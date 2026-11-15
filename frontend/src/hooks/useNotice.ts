import { useCallback, useRef, useState } from "react";
import { errorDetail, noticeSummary } from "../appRuntime";

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
  const notify = (message: string) => announce(noticeSummary(message), errorDetail(message), true);

  return { notice, noticeDetail, announce, dismissNotice, notify };
}
