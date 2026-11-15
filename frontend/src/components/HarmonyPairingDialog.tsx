import { strings } from "../strings";
import { ConfirmDialog } from "./ConfirmDialog";

// Shows only the six-digit code; the tablet shows the same code and asks again.
export function HarmonyPairingDialog({ code, onAnswer }: { code: string; onAnswer: (accept: boolean) => void }) {
  return <ConfirmDialog title={strings.harmonyPairing.title} body={code} confirmLabel={strings.harmonyPairing.confirm} busy={false} onConfirm={() => onAnswer(true)} onCancel={() => onAnswer(false)} />;
}
