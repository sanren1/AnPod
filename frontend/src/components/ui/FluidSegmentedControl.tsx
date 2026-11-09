import type { CSSProperties, ReactNode } from "react";
import "../../design/components.css";

export type SegmentOption<T extends string> = {
  value: T;
  label: ReactNode;
  disabled?: boolean;
};

export function FluidSegmentedControl<T extends string>({
  label,
  value,
  options,
  onValueChange,
  className,
}: {
  label: string;
  value: T;
  options: SegmentOption<T>[];
  onValueChange: (value: T) => void;
  className?: string;
}) {
  const selectedIndex = options.findIndex(option => option.value === value);

  return <div
    className={`fluid-segmented-control${className ? ` ${className}` : ""}`}
    role="group"
    aria-label={label}
    style={{ "--segment-count": options.length, "--segment-index": selectedIndex } as CSSProperties}
  >
    {selectedIndex >= 0 && <span className="fluid-segmented-control__selection" aria-hidden="true" />}
    {options.map(option => <button
      key={option.value}
      type="button"
      aria-pressed={option.value === value}
      disabled={option.disabled}
      onClick={() => onValueChange(option.value)}
    >
      <span>{option.label}</span>
    </button>)}
  </div>;
}
