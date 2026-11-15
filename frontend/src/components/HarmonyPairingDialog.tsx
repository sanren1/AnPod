import { strings } from "../strings";
import { ConfirmDialog } from "./ConfirmDialog";

// Shows only the six-digit code; the tablet shows the same code and asks again.
export function HarmonyPairingDialog({ code, busy, onAnswer }: { code: string; busy: boolean; onAnswer: (accept: boolean) => void }) {
  return <ConfirmDialog title={strings.harmonyPairing.title} body={code} confirmLabel={strings.harmonyPairing.confirm} busyLabel={strings.harmonyPairing.confirm} busy={busy} onConfirm={() => onAnswer(true)} onCancel={() => onAnswer(false)} />;
}
