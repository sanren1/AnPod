import { describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { strings } from "../strings";
import { ConfirmDialog } from "./ConfirmDialog";
import { HarmonyPairingDialog } from "./HarmonyPairingDialog";

type Props = { children?: ReactNode; onClick?: () => void };
const flatten = (node: ReactNode): ReactElement<Props>[] => {
  if (!node || typeof node !== "object") return [];
  if (Array.isArray(node)) return node.flatMap(flatten);
  const element = node as ReactElement<Props>;
  return [element, ...flatten(element.props.children)];
};
const text = (node: ReactNode): string =>
  typeof node === "string" || typeof node === "number" ? String(node)
    : Array.isArray(node) ? node.map(text).join("")
      : node && typeof node === "object" ? text((node as ReactElement<Props>).props.children) : "";

describe("HarmonyPairingDialog", () => {
  it("pairing dialog shows code and answers", () => {
    const onAnswer = vi.fn();
    const dialog = HarmonyPairingDialog({ code: "017647", busy: false, onAnswer });
    const modal = ConfirmDialog(dialog.props as Parameters<typeof ConfirmDialog>[0]) as ReactElement<{ title: string; children: ReactNode }>;
    expect(modal.props.title).toBe(strings.harmonyPairing.title);
    const nodes = flatten(modal.props.children);
    expect(nodes.filter(node => node.type === "p").map(node => text(node))).toEqual(["017647"]);
    const [cancel, confirm] = nodes.filter(node => node.type === "button");
    expect(text(cancel)).toBe(strings.confirmDialog.cancel);
    expect(text(confirm)).toBe(strings.harmonyPairing.confirm);
    confirm.props.onClick?.();
    cancel.props.onClick?.();
    expect(onAnswer.mock.calls).toEqual([[true], [false]]);
  });

  it("disables both answers while one is being sent", () => {
    const dialog = HarmonyPairingDialog({ code: "017647", busy: true, onAnswer: vi.fn() });
    const modal = ConfirmDialog(dialog.props as Parameters<typeof ConfirmDialog>[0]) as ReactElement<{ children: ReactNode }>;
    const buttons = flatten(modal.props.children).filter(node => node.type === "button") as ReactElement<Props & { disabled?: boolean }>[];
    expect(buttons.map(button => button.props.disabled)).toEqual([true, true]);
    expect(text(buttons[1])).toBe(strings.harmonyPairing.confirm);
  });
});
